use std::io;
use std::path::Path;

/// Replace `dst` with `src` atomically when the platform supports it.
///
/// Both paths are expected to live on the same volume. Callers create a temp
/// file next to the final destination and then promote it via this helper.
pub fn replace_file(src: &Path, dst: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        replace_file_windows(src, dst)
    }

    #[cfg(not(windows))]
    {
        std::fs::rename(src, dst)
    }
}

#[cfg(windows)]
fn replace_file_windows(src: &Path, dst: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    use windows::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let src = crate::fs::long_path::maybe_extended(src);
    let dst = crate::fs::long_path::maybe_extended(dst);

    let src_wide: Vec<u16> = src
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let dst_wide: Vec<u16> = dst
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        MoveFileExW(
            windows::core::PCWSTR::from_raw(src_wide.as_ptr()),
            windows::core::PCWSTR::from_raw(dst_wide.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    }
    .map_err(|e| io::Error::other(e.to_string()))
}
