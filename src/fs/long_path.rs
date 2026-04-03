/// Windows 长路径支持（> 260 字符）
///
/// 通过给绝对路径添加 `\\?\` 前缀，绕过 MAX_PATH 限制。
/// - 本地路径：`C:\...`  →  `\\?\C:\...`
/// - UNC 路径：`\\server\share\...`  →  `\\?\UNC\server\share\...`
/// - 已有前缀：原样返回
///
/// 只在 Windows 平台生效；非 Windows 原样返回。
pub fn extended_path(path: &std::path::Path) -> std::path::PathBuf {
    #[cfg(windows)]
    {
        let s = path.to_string_lossy();

        // 已是扩展路径，直接返回
        if s.starts_with(r"\\?\") {
            return path.to_path_buf();
        }

        // UNC 路径：\\server\share → \\?\UNC\server\share
        if s.starts_with(r"\\") {
            let unc_part = &s[2..]; // 去掉前导 \\
            return std::path::PathBuf::from(format!(r"\\?\UNC\{}", unc_part));
        }

        // 普通绝对路径
        if path.is_absolute() {
            return std::path::PathBuf::from(format!(r"\\?\{}", s));
        }

        path.to_path_buf()
    }
    #[cfg(not(windows))]
    {
        path.to_path_buf()
    }
}

/// 仅当路径长度超过安全阈值（248 字符）时才转换为扩展路径。
/// 对短路径不做处理，避免与某些不支持 `\\?\` 前缀的 API 冲突。
pub fn maybe_extended(path: &std::path::Path) -> std::path::PathBuf {
    if path.to_string_lossy().len() > 248 {
        extended_path(path)
    } else {
        path.to_path_buf()
    }
}
