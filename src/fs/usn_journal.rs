/// USN Journal 增量扫描（NTFS / ReFS）
///
/// 使用流程：
///   1. `get_volume_key(path)` 获取文件所在卷根路径（如 "C:\\"）
///   2. `query_journal(volume_root)` 获取当前 journal ID 和最新 USN（新检查点）
///   3. `read_changed_frns(volume_root, last_usn, journal_id)` 拿到
///      自上次同步以来变化的文件 FRN 集合，用于跳过哈希比对

use std::collections::HashSet;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct JournalInfo {
    pub journal_id: u64,
    pub next_usn: i64,
}

/// 获取指定路径所在卷的根路径（如 "C:\\"）。
/// 用作 last_sync_checkpoints 的键。
pub fn get_volume_key(path: &Path) -> Option<String> {
    #[cfg(windows)]
    {
        get_volume_key_windows(path)
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        None
    }
}

/// 查询卷的 USN Journal 信息。
/// `volume_root` 示例：`"C:\\"` 或 `"D:\\"`
pub fn query_journal(volume_root: &str) -> Option<JournalInfo> {
    #[cfg(windows)]
    {
        query_journal_windows(volume_root)
    }
    #[cfg(not(windows))]
    {
        let _ = volume_root;
        None
    }
}

/// 获取文件或目录的 NTFS/ReFS 文件引用号（FRN）。
///
/// FRN 是文件在卷上的唯一 64 位标识符，用于 USN Journal 增量优化：
/// 若文件的 FRN 未出现在变化集中，说明该文件自上次同步后未修改。
/// 非 Windows 平台或 FRN 不可用时返回 `None`。
pub fn get_file_index(path: &Path) -> Option<u64> {
    #[cfg(windows)]
    {
        get_file_index_windows(path)
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        None
    }
}

/// 读取自 `start_usn` 以来发生变化的文件 FRN 集合。
/// 返回 `(changed_frns, next_usn)`，其中 `next_usn` 可用作下次检查点。
pub fn read_changed_frns(
    volume_root: &str,
    start_usn: i64,
    journal_id: u64,
) -> (HashSet<u64>, i64) {
    #[cfg(windows)]
    {
        read_changed_frns_windows(volume_root, start_usn, journal_id)
    }
    #[cfg(not(windows))]
    {
        let _ = (volume_root, start_usn, journal_id);
        (HashSet::new(), start_usn)
    }
}

// ─────────────────────────────────────────────────────────────────
// Windows 实现
// ─────────────────────────────────────────────────────────────────

#[cfg(windows)]
fn get_file_index_windows(path: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, CreateFileW, GetFileInformationByHandle,
        FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };

    // FILE_FLAG_BACKUP_SEMANTICS（0x02000000）：允许打开目录
    // FILE_FLAG_OPEN_REPARSE_POINT（0x00200000）：不跟随符号链接
    const FLAGS: u32 = 0x02000000 | 0x00200000;

    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    let handle = unsafe {
        CreateFileW(
            windows::core::PCWSTR::from_raw(wide.as_ptr()),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(FLAGS),
            None,
        )
    }
    .ok()?;

    if handle == INVALID_HANDLE_VALUE {
        return None;
    }

    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    let ok = unsafe { GetFileInformationByHandle(handle, &mut info).is_ok() };
    unsafe { windows::Win32::Foundation::CloseHandle(handle).ok() };

    if ok {
        Some((info.nFileIndexHigh as u64) << 32 | info.nFileIndexLow as u64)
    } else {
        None
    }
}

#[cfg(windows)]
fn open_volume_handle(volume_root: &str) -> Option<windows::Win32::Foundation::HANDLE> {
    use windows::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_READ, FILE_SHARE_WRITE,
        OPEN_EXISTING,
    };
    use windows::Win32::Foundation::GENERIC_READ;

    // 构造 \\.\X: 格式的卷路径（去掉末尾反斜杠）
    let vol = volume_root.trim_end_matches(['\\', '/']);
    let unc = format!("\\\\.\\{}", vol);
    let wide: Vec<u16> = unc.encode_utf16().chain(std::iter::once(0)).collect();

    let handle = unsafe {
        CreateFileW(
            windows::core::PCWSTR::from_raw(wide.as_ptr()),
            GENERIC_READ.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            None,
        )
    };

    match handle {
        Ok(h) if h != INVALID_HANDLE_VALUE => Some(h),
        _ => None,
    }
}

#[cfg(windows)]
fn get_volume_key_windows(path: &Path) -> Option<String> {
    use windows::Win32::Storage::FileSystem::GetVolumePathNameW;

    let path_str = path.to_string_lossy();
    let wide: Vec<u16> = path_str.encode_utf16().chain(std::iter::once(0)).collect();
    let mut vol_buf = vec![0u16; 260];

    let ok = unsafe {
        GetVolumePathNameW(
            windows::core::PCWSTR::from_raw(wide.as_ptr()),
            &mut vol_buf,
        )
        .is_ok()
    };

    if ok {
        let len = vol_buf.iter().position(|&c| c == 0).unwrap_or(0);
        Some(String::from_utf16_lossy(&vol_buf[..len]))
    } else {
        None
    }
}

#[cfg(windows)]
fn query_journal_windows(volume_root: &str) -> Option<JournalInfo> {
    use windows::Win32::System::Ioctl::{
        FSCTL_QUERY_USN_JOURNAL, USN_JOURNAL_DATA_V0,
    };
    use windows::Win32::System::IO::DeviceIoControl;

    let handle = open_volume_handle(volume_root)?;

    let mut journal_data = USN_JOURNAL_DATA_V0::default();
    let mut bytes_returned: u32 = 0;

    let ok = unsafe {
        DeviceIoControl(
            handle,
            FSCTL_QUERY_USN_JOURNAL,
            None,
            0,
            Some(&mut journal_data as *mut _ as *mut std::ffi::c_void),
            std::mem::size_of::<USN_JOURNAL_DATA_V0>() as u32,
            Some(&mut bytes_returned),
            None,
        )
        .is_ok()
    };

    unsafe { windows::Win32::Foundation::CloseHandle(handle).ok() };

    if ok {
        Some(JournalInfo {
            journal_id: journal_data.UsnJournalID,
            next_usn: journal_data.NextUsn,
        })
    } else {
        None
    }
}

#[cfg(windows)]
fn read_changed_frns_windows(
    volume_root: &str,
    start_usn: i64,
    journal_id: u64,
) -> (HashSet<u64>, i64) {
    use windows::Win32::System::Ioctl::{
        FSCTL_READ_USN_JOURNAL, READ_USN_JOURNAL_DATA_V0, USN_RECORD_V2,
    };
    use windows::Win32::System::IO::DeviceIoControl;

    let handle = match open_volume_handle(volume_root) {
        Some(h) => h,
        None => return (HashSet::new(), start_usn),
    };

    let mut frns = HashSet::new();
    let mut next_usn = start_usn;

    let mut read_data = READ_USN_JOURNAL_DATA_V0 {
        StartUsn: start_usn,
        ReasonMask: 0x0FFF_FFFF, // 所有 reason bits
        ReturnOnlyOnClose: 0,
        Timeout: 0,
        BytesToWaitFor: 0,
        UsnJournalID: journal_id,
    };

    let mut buffer = vec![0u8; 64 * 1024]; // 64 KB 读取缓冲

    loop {
        let mut bytes_returned: u32 = 0;

        let ok = unsafe {
            DeviceIoControl(
                handle,
                FSCTL_READ_USN_JOURNAL,
                Some(&read_data as *const _ as *const std::ffi::c_void),
                std::mem::size_of::<READ_USN_JOURNAL_DATA_V0>() as u32,
                Some(buffer.as_mut_ptr() as *mut std::ffi::c_void),
                buffer.len() as u32,
                Some(&mut bytes_returned),
                None,
            )
            .is_ok()
        };

        if !ok || bytes_returned < 8 {
            break;
        }

        // 前 8 字节是下一个 USN（i64）
        let usn = i64::from_le_bytes(buffer[..8].try_into().unwrap_or([0; 8]));
        next_usn = usn;
        if usn == read_data.StartUsn {
            break; // 没有新记录
        }
        read_data.StartUsn = usn;

        // 解析 USN_RECORD_V2 结构，提取文件引用号（FRN）
        let mut offset: usize = 8;
        while offset + std::mem::size_of::<USN_RECORD_V2>() <= bytes_returned as usize {
            let record = unsafe {
                &*(buffer.as_ptr().add(offset) as *const USN_RECORD_V2)
            };

            let record_len = record.RecordLength as usize;
            if record_len == 0 || offset + record_len > bytes_returned as usize {
                break;
            }

            // FileReferenceNumber 是 i64，以 u64 存储
            frns.insert(record.FileReferenceNumber as u64);

            offset += record_len;
        }
    }

    unsafe { windows::Win32::Foundation::CloseHandle(handle).ok() };

    (frns, next_usn)
}
