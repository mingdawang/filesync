/// Delta Sync — rsync 算法的 Rust 实现
///
/// 算法流程：
///   1. 读取目标文件，按 BLOCK_SIZE 分块，计算每块 Adler-32（弱校验）+ BLAKE3（强校验）
///   2. 构建 弱校验 → 块序号 的哈希表
///   3. 在源文件上滑动窗口：
///      a. 命中弱校验 → 再验证强校验
///      b. 命中强校验 → 记录"从目标第 N 块复制"指令
///      c. 否则 → 记录"字面字节"
///   4. 按指令流重建目标文件（先写临时文件，再原子 rename）
///
/// 节省量 = 所有"复制"指令覆盖的字节数

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{Context, Result};
use flume::Sender;

use crate::engine::events::SyncEvent;

/// 每块大小：2 KB（可调；较小块命中率高但开销大）
pub const BLOCK_SIZE: usize = 2048;

/// 对源文件执行 Delta Sync，将结果写入 dst，通过 tx 上报进度。
///
/// 返回 `(delta_applied: bool, saved_bytes: u64)`。
/// - `delta_applied`：是否实际使用了 delta（目标不存在时直接完整复制）
/// - `saved_bytes`：相比完整复制节省的字节数（0 表示未节省）
pub fn delta_sync(
    src: &Path,
    dst: &Path,
    worker_id: usize,
    tx: &Sender<SyncEvent>,
    stop: &Arc<AtomicBool>,
) -> Result<(bool, u64)> {
    // 1. 目标不存在 → 完整复制（非 delta）
    if !dst.exists() {
        plain_copy(src, dst, worker_id, tx, stop)?;
        return Ok((false, 0));
    }

    // 2. 构建目标文件的块校验表
    let block_table = build_block_table(dst)?;
    if block_table.is_empty() {
        plain_copy(src, dst, worker_id, tx, stop)?;
        return Ok((false, 0));
    }

    // 3. 计算指令序列
    let instructions = compute_instructions(src, &block_table, stop)?;

    // 4. 按指令重建文件
    let saved = apply_instructions(src, dst, &instructions, worker_id, tx, stop)?;

    Ok((true, saved))
}

// ─────────────────────────────────────────────────────────────────
// 块校验表
// ─────────────────────────────────────────────────────────────────

/// `(adler32, blake3_first16)` → block_index
type BlockTable = HashMap<(u32, [u8; 16]), usize>;

fn build_block_table(dst: &Path) -> Result<BlockTable> {
    let mut file = std::fs::File::open(dst)
        .with_context(|| format!("打开目标文件失败: {}", dst.display()))?;

    let mut table = BlockTable::new();
    let mut buf = vec![0u8; BLOCK_SIZE];
    let mut block_idx = 0usize;

    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        let data = &buf[..n];

        let weak = adler32_checksum(data);
        let strong = blake3_prefix(data);

        table.entry((weak, strong)).or_insert(block_idx);
        block_idx += 1;
    }

    Ok(table)
}

// ─────────────────────────────────────────────────────────────────
// 指令计算（滑动窗口）
// ─────────────────────────────────────────────────────────────────

#[derive(Debug)]
enum Instruction {
    /// 从目标文件第 N 块（offset = N * BLOCK_SIZE）复制 `len` 字节
    CopyFromDst { block_idx: usize, len: usize },
    /// 字面字节
    Literal(Vec<u8>),
}

fn compute_instructions(
    src: &Path,
    block_table: &BlockTable,
    stop: &Arc<AtomicBool>,
) -> Result<Vec<Instruction>> {
    let src_data = std::fs::read(src)
        .with_context(|| format!("读取源文件失败: {}", src.display()))?;

    let mut instructions = Vec::new();
    let mut pos = 0usize;
    let mut literal_buf: Vec<u8> = Vec::new();

    while pos + BLOCK_SIZE <= src_data.len() {
        if stop.load(Ordering::Relaxed) {
            anyhow::bail!("已停止");
        }

        let window = &src_data[pos..pos + BLOCK_SIZE];
        let weak = adler32_checksum(window);
        let strong = blake3_prefix(window);

        if let Some(&block_idx) = block_table.get(&(weak, strong)) {
            // 命中 → 先 flush literal，再发 Copy 指令
            if !literal_buf.is_empty() {
                instructions.push(Instruction::Literal(std::mem::take(&mut literal_buf)));
            }
            instructions.push(Instruction::CopyFromDst {
                block_idx,
                len: BLOCK_SIZE,
            });
            pos += BLOCK_SIZE;
        } else {
            // 未命中 → 按字节推进，累积 literal
            literal_buf.push(src_data[pos]);
            pos += 1;
        }
    }

    // 尾部剩余字节全部作为 literal
    if pos < src_data.len() {
        literal_buf.extend_from_slice(&src_data[pos..]);
    }
    if !literal_buf.is_empty() {
        instructions.push(Instruction::Literal(literal_buf));
    }

    Ok(instructions)
}

// ─────────────────────────────────────────────────────────────────
// 指令应用
// ─────────────────────────────────────────────────────────────────

fn apply_instructions(
    _src: &Path,
    dst: &Path,
    instructions: &[Instruction],
    worker_id: usize,
    tx: &Sender<SyncEvent>,
    stop: &Arc<AtomicBool>,
) -> Result<u64> {
    let parent = dst.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;

    let stem = dst.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
    let ext = dst.extension().and_then(|e| e.to_str()).unwrap_or("");
    let tmp_name = if ext.is_empty() {
        format!(".{}.delta.tmp", stem)
    } else {
        format!(".{}.{}.delta.tmp", stem, ext)
    };
    let tmp_path = parent.join(&tmp_name);

    let mut dst_file = std::fs::File::open(dst).ok();
    let mut tmp_file = std::fs::File::create(&tmp_path)?;
    let mut bytes_done: u64 = 0;
    let mut saved: u64 = 0;

    for instr in instructions {
        if stop.load(Ordering::Relaxed) {
            drop(tmp_file);
            let _ = std::fs::remove_file(&tmp_path);
            anyhow::bail!("已停止");
        }

        match instr {
            Instruction::CopyFromDst { block_idx, len } => {
                if let Some(ref mut df) = dst_file {
                    let offset = (*block_idx * BLOCK_SIZE) as u64;
                    df.seek(SeekFrom::Start(offset))?;
                    let mut buf = vec![0u8; *len];
                    let n = df.read(&mut buf)?;
                    tmp_file.write_all(&buf[..n])?;
                    bytes_done += n as u64;
                    saved += n as u64;
                }
            }
            Instruction::Literal(data) => {
                tmp_file.write_all(data)?;
                bytes_done += data.len() as u64;
            }
        }

        let _ = tx.try_send(SyncEvent::FileProgress { worker_id, bytes_done });
    }

    tmp_file.flush()?;
    drop(tmp_file);
    drop(dst_file);

    std::fs::rename(&tmp_path, dst)?;

    Ok(saved)
}

// ─────────────────────────────────────────────────────────────────
// 辅助函数：完整复制（用于 delta 不适用时的回退）
// ─────────────────────────────────────────────────────────────────

fn plain_copy(
    src: &Path,
    dst: &Path,
    worker_id: usize,
    tx: &Sender<SyncEvent>,
    stop: &Arc<AtomicBool>,
) -> Result<()> {
    use crate::engine::copier;
    let size = src.metadata().map(|m| m.len()).unwrap_or(0);
    copier::copy_file(src, dst, worker_id, size, tx, stop)
}

// ─────────────────────────────────────────────────────────────────
// 校验函数
// ─────────────────────────────────────────────────────────────────

fn adler32_checksum(data: &[u8]) -> u32 {
    let mut a: u32 = 1;
    let mut b: u32 = 0;
    for &byte in data {
        a = a.wrapping_add(byte as u32) % 65521;
        b = b.wrapping_add(a) % 65521;
    }
    (b << 16) | a
}

fn blake3_prefix(data: &[u8]) -> [u8; 16] {
    let hash = blake3::hash(data);
    let bytes = hash.as_bytes();
    let mut prefix = [0u8; 16];
    prefix.copy_from_slice(&bytes[..16]);
    prefix
}
