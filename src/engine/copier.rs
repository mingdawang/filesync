use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{bail, Result};
use flume::Sender;

use crate::engine::events::SyncEvent;
use crate::fs::long_path::maybe_extended;
use crate::fs::volume::VolumeCapabilities;
use crate::log::LogLevel;

const BUFFER_SIZE: usize = 256 * 1024; // 256 KB
const MAX_RETRIES: u32 = 3;
const RETRY_DELAY_MS: u64 = 500;

/// 无缓冲 IO 默认阈值（字节）。当调用方无法提供配置时使用。
const DEFAULT_UNBUFFERED_THRESHOLD: u64 = 128 * 1024 * 1024; // 128 MB

/// 复制单个文件到目标路径（使用默认阈值，供 delta 内部回退路径调用）。
///
/// 调用者（executor）负责 delta 决策；此函数只负责实际字节传输。
pub fn copy_file(
    src: &Path,
    dst: &Path,
    worker_id: usize,
    size: u64,
    tx: &Sender<SyncEvent>,
    stop: &Arc<AtomicBool>,
) -> Result<()> {
    copy_file_with_caps(src, dst, worker_id, size, tx, stop, None, false, DEFAULT_UNBUFFERED_THRESHOLD)
}

/// 带 VolumeCapabilities 和引擎配置的复制入口。
///
/// 策略（由外到内）：
/// - 无缓冲 IO：文件 >= unbuffered_threshold 且目标卷为本地卷
/// - CopyFileEx：Windows 内核路径，含进度回调
/// - 缓冲分块复制：回退，含重试
///
/// 所有路径均使用「先写临时文件，再原子 rename」保障写入安全。
/// delta 决策由调用方（executor）在调用本函数之前处理。
pub fn copy_file_with_caps(
    src: &Path,
    dst: &Path,
    worker_id: usize,
    size: u64,
    tx: &Sender<SyncEvent>,
    stop: &Arc<AtomicBool>,
    caps: Option<&VolumeCapabilities>,
    verify: bool,
    unbuffered_threshold: u64,
) -> Result<()> {
    do_copy(src, dst, worker_id, size, tx, stop, caps, unbuffered_threshold)?;
    if verify {
        do_verify(src, dst)?;
    }
    Ok(())
}

fn do_copy(
    src: &Path,
    dst: &Path,
    worker_id: usize,
    size: u64,
    tx: &Sender<SyncEvent>,
    stop: &Arc<AtomicBool>,
    caps: Option<&VolumeCapabilities>,
    unbuffered_threshold: u64,
) -> Result<()> {
    // 确保目标父目录存在
    if let Some(parent) = dst.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            crate::log::app_log(
                &format!("failed to create destination directory {}: {}", parent.display(), e),
                LogLevel::Error,
            );
            return Err(e.into());
        }
    }

    // 决策：无缓冲 IO（大文件 + 本地卷）
    let use_unbuffered = caps
        .map(|c| c.supports_unbuffered_io())
        .unwrap_or(false)
        && size >= unbuffered_threshold;

    // 决策：CopyFileEx（Windows 本地优化内核路径，用于非无缓冲情形）
    #[cfg(windows)]
    {
        if !use_unbuffered {
            match copy_file_ex(src, dst, worker_id, size, tx, stop) {
                Ok(()) => return Ok(()),
                Err(_) if stop.load(Ordering::Relaxed) => bail!("已停止"),
                Err(e) => {
                    crate::log::app_log(
                        &format!("CopyFileEx failed for {}: {}", src.display(), e),
                        LogLevel::Error,
                    );
                    // 回退到缓冲 IO
                }
            }
        }
    }

    // 回退：标准缓冲 IO（含重试）
    let tmp = make_tmp_path(dst);
    let mut last_err = String::new();
    for attempt in 0..MAX_RETRIES {
        if stop.load(Ordering::Relaxed) {
            bail!("已停止");
        }
        if attempt > 0 {
            std::thread::sleep(std::time::Duration::from_millis(RETRY_DELAY_MS));
        }

        match do_copy_buffered(src, &tmp, dst, worker_id, tx, stop) {
            Ok(()) => return Ok(()),
            Err(e) => {
                cleanup_temp(&tmp);
                if stop.load(Ordering::Relaxed) {
                    bail!("已停止");
                }
                // 权限/文件不存在等不可重试错误，立即放弃
                if let Some(io_err) = e.root_cause().downcast_ref::<std::io::Error>() {
                    if matches!(
                        io_err.kind(),
                        std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::NotFound
                    ) {
                        bail!("{}", e);
                    }
                }
                last_err = e.to_string();
            }
        }
    }

    let msg = format!("buffered copy failed after {} retries for {}: {}", MAX_RETRIES, src.display(), last_err);
    crate::log::app_log(&msg, LogLevel::Error);
    bail!("重试 {} 次后失败: {}", MAX_RETRIES, last_err)
}

fn do_verify(src: &Path, dst: &Path) -> Result<()> {
    let sh = crate::engine::hash::hash_file(src)
        .ok_or_else(|| anyhow::anyhow!("无法计算源文件哈希"))?;
    let dh = crate::engine::hash::hash_file(dst)
        .ok_or_else(|| anyhow::anyhow!("无法计算目标文件哈希"))?;
    if sh != dh {
        bail!("校验失败：复制后内容与源不一致");
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────
// Windows CopyFileEx
// ─────────────────────────────────────────────────────────────────

#[cfg(windows)]
fn copy_file_ex(
    src: &Path,
    dst: &Path,
    worker_id: usize,
    _size: u64,
    tx: &Sender<SyncEvent>,
    stop: &Arc<AtomicBool>,
) -> Result<()> {
    use windows::core::HSTRING;
    use windows::Win32::Foundation::BOOL;
    use windows::Win32::Storage::FileSystem::CopyFileExW;

    let tmp = make_tmp_path(dst);

    // 进度回调上下文
    struct CbCtx<'a> {
        worker_id: usize,
        tx: &'a Sender<SyncEvent>,
        stop: &'a Arc<AtomicBool>,
    }

    unsafe extern "system" fn progress_cb(
        _total_size: i64,
        total_transferred: i64,
        _stream_size: i64,
        _stream_transferred: i64,
        _stream_no: u32,
        _cb_reason: windows::Win32::Storage::FileSystem::LPPROGRESS_ROUTINE_CALLBACK_REASON,
        _src_handle: windows::Win32::Foundation::HANDLE,
        _dst_handle: windows::Win32::Foundation::HANDLE,
        data: *const std::ffi::c_void,
    ) -> u32 {
        if data.is_null() {
            return 0; // PROGRESS_CONTINUE
        }
        let ctx = &*(data as *const CbCtx);
        if ctx.stop.load(Ordering::Relaxed) {
            return 1; // PROGRESS_CANCEL
        }
        let _ = ctx.tx.try_send(SyncEvent::FileProgress {
            worker_id: ctx.worker_id,
            bytes_done: total_transferred as u64,
        });
        0 // PROGRESS_CONTINUE
    }

    let ctx = CbCtx { worker_id, tx, stop };

    let src_h = HSTRING::from(maybe_extended(src).to_string_lossy().as_ref());
    let tmp_h = HSTRING::from(maybe_extended(&tmp).to_string_lossy().as_ref());

    let mut cancel = BOOL(0);
    let result = unsafe {
        CopyFileExW(
            &src_h,
            &tmp_h,
            Some(progress_cb),
            Some(&ctx as *const _ as *const _),
            Some(&mut cancel),
            0,
        )
    };

    if cancel.as_bool() || stop.load(Ordering::Relaxed) {
        let _ = std::fs::remove_file(&tmp);
        bail!("已停止");
    }

    match result {
        Ok(()) => {
            std::fs::rename(&tmp, dst)?;
            Ok(())
        }
        Err(e) => {
            cleanup_temp(&tmp);
            bail!("CopyFileEx 失败: {}", e)
        }
    }
}

// ─────────────────────────────────────────────────────────────────
// 标准缓冲 IO 复制
// ─────────────────────────────────────────────────────────────────

fn do_copy_buffered(
    src: &Path,
    tmp: &Path,
    dst: &Path,
    worker_id: usize,
    tx: &Sender<SyncEvent>,
    stop: &Arc<AtomicBool>,
) -> Result<()> {
    let mut src_file = std::fs::File::open(maybe_extended(src))?;
    let mut tmp_file = std::fs::File::create(maybe_extended(tmp))?;

    let mut buf = vec![0u8; BUFFER_SIZE];
    let mut bytes_done: u64 = 0;

    loop {
        if stop.load(Ordering::Relaxed) {
            bail!("已停止");
        }

        let n = src_file.read(&mut buf)?;
        if n == 0 {
            break;
        }

        tmp_file.write_all(&buf[..n])?;
        bytes_done += n as u64;

        let _ = tx.try_send(SyncEvent::FileProgress { worker_id, bytes_done });
    }

    tmp_file.flush()?;
    drop(tmp_file);

    std::fs::rename(tmp, dst)?;
    Ok(())
}

// ─────────────────────────────────────────────────────────────────
// 辅助
// ─────────────────────────────────────────────────────────────────

fn cleanup_temp(tmp: &Path) {
    if let Err(e) = std::fs::remove_file(tmp) {
        crate::log::app_log(
            &format!("failed to clean up temp file {}: {}", tmp.display(), e),
            LogLevel::Error,
        );
    }
}

fn make_tmp_path(dst: &Path) -> std::path::PathBuf {
    let id = uuid::Uuid::new_v4().simple().to_string();
    let stem = dst
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("file");
    let ext = dst.extension().and_then(|e| e.to_str()).unwrap_or("");
    let tmp_name = if ext.is_empty() {
        format!(".{}.{}.tmp", stem, id)
    } else {
        format!(".{}.{}.{}.tmp", stem, id, ext)
    };
    dst.parent().unwrap_or(Path::new(".")).join(tmp_name)
}
