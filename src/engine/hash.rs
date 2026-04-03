/// 文件内容哈希（BLAKE3），用于 Hash 比较模式

use std::io::Read;
use std::path::Path;

/// 计算文件的 BLAKE3 哈希。失败时返回 None（文件不存在、权限错误等）。
pub fn hash_file(path: &Path) -> Option<[u8; 32]> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; 256 * 1024];

    loop {
        let n = file.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }

    Some(*hasher.finalize().as_bytes())
}
