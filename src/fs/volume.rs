use std::path::Path;

use crate::log::LogLevel;

/// 文件系统类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsType {
    Ntfs,
    ReFS,
    ExFat,
    Fat32,
    Other,
    Unknown,
}

/// 卷能力集——由 `detect_volume` 在运行时填充
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct VolumeCapabilities {
    pub fs_type: FsType,
    /// 是否支持 USN Journal（NTFS/ReFS）
    pub supports_usn_journal: bool,
    /// 是否支持稀疏文件（NTFS/ReFS）
    pub supports_sparse_files: bool,
    /// 是否支持 ReFS Block Clone（同卷零拷贝）
    pub supports_block_clone: bool,
    /// 是否为网络路径
    pub is_remote: bool,
    /// 扇区大小（字节）
    pub sector_size: u32,
}

impl VolumeCapabilities {
    /// 是否可使用无缓冲 IO（本地卷）
    pub fn supports_unbuffered_io(&self) -> bool {
        !self.is_remote
    }
}

/// 探测给定路径所在卷的文件系统特性。
pub fn detect_volume(path: &Path) -> VolumeCapabilities {
    #[cfg(windows)]
    {
        detect_volume_windows(path)
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        VolumeCapabilities {
            fs_type: FsType::Unknown,
            supports_usn_journal: false,
            supports_sparse_files: false,
            supports_block_clone: false,
            is_remote: false,
            sector_size: 512,
        }
    }
}

#[cfg(windows)]
fn detect_volume_windows(path: &Path) -> VolumeCapabilities {
    use windows::core::HSTRING;
    use windows::Win32::Storage::FileSystem::{
        GetDriveTypeW, GetVolumeInformationW,
    };

    // 1. 获取卷根路径
    let volume_root = get_volume_root(path);
    let volume_hstring = HSTRING::from(volume_root.as_str());

    // 2. 查询驱动器类型（DRIVE_REMOTE == 4）
    let drive_type = unsafe { GetDriveTypeW(&volume_hstring) };
    let is_remote = drive_type == 4;

    // 3. 查询文件系统名称和标志位
    let mut fs_name_buf = vec![0u16; 64];
    let mut volume_flags: u32 = 0;
    let mut serial: u32 = 0;
    let mut max_component: u32 = 0;

    let got_info = unsafe {
        GetVolumeInformationW(
            &volume_hstring,
            None,
            Some(&mut serial as *mut u32),
            Some(&mut max_component as *mut u32),
            Some(&mut volume_flags as *mut u32),
            Some(&mut fs_name_buf),
        )
        .is_ok()
    };

    let fs_type = if got_info {
        let name = String::from_utf16_lossy(
            &fs_name_buf
                [..fs_name_buf.iter().position(|&c| c == 0).unwrap_or(fs_name_buf.len())],
        );
        match name.to_uppercase().as_str() {
            "NTFS" => FsType::Ntfs,
            "REFS" => FsType::ReFS,
            "EXFAT" => FsType::ExFat,
            "FAT32" | "FAT" => FsType::Fat32,
            _ => FsType::Other,
        }
    } else {
        crate::log::app_log(
            &format!("GetVolumeInformationW failed for volume: {}", volume_root),
            LogLevel::Error,
        );
        FsType::Unknown
    };

    // FILE_SUPPORTS_SPARSE_FILES = 0x40
    let supports_sparse = volume_flags & 0x40 != 0;
    let supports_usn_journal = matches!(fs_type, FsType::Ntfs | FsType::ReFS);
    let supports_block_clone = matches!(fs_type, FsType::ReFS);

    // 4. 获取扇区大小
    let sector_size = get_sector_size(&volume_root).unwrap_or_else(|e| {
        crate::log::app_log(
            &format!("failed to get sector size for volume {}: {}", volume_root, e),
            LogLevel::Error,
        );
        512
    });

    VolumeCapabilities {
        fs_type,
        supports_usn_journal,
        supports_sparse_files: supports_sparse,
        supports_block_clone,
        is_remote,
        sector_size,
    }
}

#[cfg(windows)]
fn get_volume_root(path: &Path) -> String {
    use windows::Win32::Storage::FileSystem::GetVolumePathNameW;

    let wide: Vec<u16> = path
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0u16))
        .collect();

    let mut buf = vec![0u16; 260];
    let ok = unsafe {
        GetVolumePathNameW(
            windows::core::PCWSTR::from_raw(wide.as_ptr()),
            &mut buf,
        )
        .is_ok()
    };

    if ok {
        String::from_utf16_lossy(
            &buf[..buf.iter().position(|&c| c == 0).unwrap_or(buf.len())],
        )
    } else {
        let s = path.to_string_lossy();
        if s.len() >= 3 {
            s[..3].to_string()
        } else {
            ".\\".to_string()
        }
    }
}

#[cfg(windows)]
fn get_sector_size(volume_root: &str) -> Result<u32, String> {
    use windows::core::HSTRING;
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceW;

    let root = HSTRING::from(volume_root);
    let mut sectors_per_cluster: u32 = 0;
    let mut bytes_per_sector: u32 = 0;
    let mut free_clusters: u32 = 0;
    let mut total_clusters: u32 = 0;

    let ok = unsafe {
        GetDiskFreeSpaceW(
            &root,
            Some(&mut sectors_per_cluster as *mut u32),
            Some(&mut bytes_per_sector as *mut u32),
            Some(&mut free_clusters as *mut u32),
            Some(&mut total_clusters as *mut u32),
        )
        .is_ok()
    };
    if ok && bytes_per_sector > 0 {
        Ok(bytes_per_sector)
    } else {
        Err(format!("GetDiskFreeSpaceW failed or returned zero sector size for {}", volume_root))
    }
}
