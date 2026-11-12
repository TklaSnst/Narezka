//! Быстрый хэш файла — ключ кэша транскрипции и анализа.
//!
//! Многочасовой ролик целиком не читаем: размер + три куска по 1 МиБ (начало, середина, конец).

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::MediaError;

const CHUNK: u64 = 1 << 20;

/// Возвращает `<размер в hex>-<32 hex-символа blake3>`.
pub fn quick_hash(path: &Path) -> Result<String, MediaError> {
    let mut file = File::open(path)?;
    let size = file.metadata()?.len();

    let mut hasher = blake3::Hasher::new();
    hasher.update(&size.to_le_bytes());
    let mut buf = vec![0u8; CHUNK as usize];
    let offsets: &[u64] = if size <= 3 * CHUNK { &[0] } else { &[0, size / 2 - CHUNK / 2, size - CHUNK] };
    for &off in offsets {
        file.seek(SeekFrom::Start(off))?;
        let mut chunk = (&mut file).take(CHUNK);
        loop {
            let n = chunk.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
    }
    let hex = hasher.finalize().to_hex();
    Ok(format!("{size:x}-{}", &hex[..32]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;

    fn write(dir: &Path, name: &str, data: &[u8]) -> PathBuf {
        let p = dir.join(name);
        File::create(&p).unwrap().write_all(data).unwrap();
        p
    }

    #[test]
    fn stable_and_sensitive() {
        let dir = tempfile::tempdir().unwrap();
        let big: Vec<u8> = (0..5 * CHUNK as usize).map(|i| (i % 251) as u8).collect();
        let a = write(dir.path(), "a", &big);
        let b = write(dir.path(), "b", &big);
        assert_eq!(quick_hash(&a).unwrap(), quick_hash(&b).unwrap());

        // Изменение в середине и в конце файла меняет хэш.
        for pos in [big.len() / 2, big.len() - 1] {
            let mut changed = big.clone();
            changed[pos] ^= 1;
            let c = write(dir.path(), "c", &changed);
            assert_ne!(quick_hash(&a).unwrap(), quick_hash(&c).unwrap(), "pos {pos}");
        }
    }

    #[test]
    fn small_and_empty_files() {
        let dir = tempfile::tempdir().unwrap();
        let e = write(dir.path(), "e", b"");
        let s = write(dir.path(), "s", b"hello");
        let h = quick_hash(&s).unwrap();
        assert!(h.starts_with("5-"));
        assert_ne!(quick_hash(&e).unwrap(), h);
    }
}
