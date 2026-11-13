//! Модели ИИ: где лежат, откуда качать, проверка целостности.
//!
//! Загрузка — в `.<имя>.part` рядом, с докачкой после обрыва; файл появляется под настоящим
//! именем только после проверки размера и SHA-256.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use sha2::{Digest, Sha256};

use crate::AnalysisError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelSpec {
    pub file: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
    pub size: u64,
}

/// Whisper large-v3-turbo F16 (DECISIONS: основная модель распознавания).
pub const WHISPER_TURBO: ModelSpec = ModelSpec {
    file: "ggml-large-v3-turbo.bin",
    url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo.bin",
    sha256: "1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69",
    size: 1_624_555_275,
};

/// Silero VAD для встроенного VAD whisper.cpp.
pub const SILERO_VAD: ModelSpec = ModelSpec {
    file: "ggml-silero-v6.2.0.bin",
    url: "https://huggingface.co/ggml-org/whisper-vad/resolve/main/ggml-silero-v6.2.0.bin",
    sha256: "2aa269b785eeb53a82983a20501ddf7c1d9c48e33ab63a41391ac6c9f7fb6987",
    size: 885_098,
};

/// Эмбеддинги предложений для границ тем (шаг 2.2): Qwen3-Embedding-0.6B Q8_0, многоязычная, ~0.7 ГБ VRAM.
pub const QWEN3_EMBED: ModelSpec = ModelSpec {
    file: "Qwen3-Embedding-0.6B-Q8_0.gguf",
    url: "https://huggingface.co/Qwen/Qwen3-Embedding-0.6B-GGUF/resolve/main/Qwen3-Embedding-0.6B-Q8_0.gguf",
    sha256: "06507c7b42688469c4e7298b0a1e16deff06caf291cf0a5b278c308249c3e439",
    size: 639_150_592,
};

/// LLM нарезки (DECISIONS, шаг 0.3): Qwen3.5-9B UD-Q4_K_XL от Unsloth, ~5.7 ГБ VRAM с контекстом 12288.
pub const QWEN35_9B: ModelSpec = ModelSpec {
    file: "Qwen3.5-9B-UD-Q4_K_XL.gguf",
    url: "https://huggingface.co/unsloth/Qwen3.5-9B-GGUF/resolve/main/Qwen3.5-9B-UD-Q4_K_XL.gguf",
    sha256: "6f5d30666c2d8ae16a306e616d95341dcf3cc46810df84d7e6f5a7d1e4c1b293",
    size: 5_966_095_584,
};

impl ModelSpec {
    pub fn path(&self, dir: &Path) -> PathBuf {
        dir.join(self.file)
    }

    /// Файл на месте и нужного размера (хэш проверяется только при загрузке).
    pub fn present(&self, dir: &Path) -> bool {
        std::fs::metadata(self.path(dir)).is_ok_and(|m| m.len() == self.size)
    }

    fn part_path(&self, dir: &Path) -> PathBuf {
        dir.join(format!(".{}.part", self.file))
    }

    /// Качает модель в `dir`. Прогресс — (получено, всего) байт.
    pub fn download(
        &self,
        dir: &Path,
        cancel: &AtomicBool,
        mut progress: impl FnMut(u64, u64),
    ) -> Result<PathBuf, AnalysisError> {
        std::fs::create_dir_all(dir)?;
        let part = self.part_path(dir);
        let have = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
        let mut req = ureq::get(self.url);
        if have > 0 && have < self.size {
            req = req.header("Range", format!("bytes={have}-"));
        }
        let mut resp = req.call().map_err(|e| AnalysisError::Download(e.to_string()))?;
        // 206 — сервер согласился докачать; иначе начинаем заново.
        let resumed = resp.status() == 206;
        let mut file = if resumed {
            OpenOptions::new().append(true).open(&part)?
        } else {
            File::create(&part)?
        };
        let mut done = if resumed { have } else { 0 };
        let mut body = resp.body_mut().with_config().limit(u64::MAX).reader();
        let mut buf = vec![0u8; 1 << 16];
        let mut last = 0;
        progress(done, self.size);
        loop {
            if cancel.load(Ordering::Relaxed) {
                // .part остаётся — следующая попытка докачает.
                return Err(AnalysisError::Cancelled);
            }
            let n = body.read(&mut buf).map_err(|e| AnalysisError::Download(e.to_string()))?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n])?;
            done += n as u64;
            if done - last >= 1 << 20 {
                last = done;
                progress(done, self.size);
            }
        }
        file.sync_all()?;
        drop(file);
        progress(done, self.size);

        if done != self.size || sha256_file(&part)? != self.sha256 {
            let _ = std::fs::remove_file(&part);
            return Err(AnalysisError::Checksum(self.file));
        }
        let path = self.path(dir);
        std::fs::rename(&part, &path)?;
        Ok(path)
    }
}

fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut f = File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn present_checks_size() {
        let tmp = tempfile::tempdir().unwrap();
        let spec = ModelSpec { file: "m.bin", url: "", sha256: "", size: 3 };
        assert!(!spec.present(tmp.path()));
        std::fs::write(spec.path(tmp.path()), b"ab").unwrap();
        assert!(!spec.present(tmp.path()));
        std::fs::write(spec.path(tmp.path()), b"abc").unwrap();
        assert!(spec.present(tmp.path()));
    }

    /// Сеть: `cargo test -p sc-analysis -- --ignored`. Докачка с середины и проверка хэша.
    #[test]
    #[ignore]
    fn download_resumes_vad() {
        let tmp = tempfile::tempdir().unwrap();
        let cancel = AtomicBool::new(false);
        let full = SILERO_VAD.download(tmp.path(), &cancel, |_, _| {}).unwrap();
        assert!(SILERO_VAD.present(tmp.path()));
        // Обрыв на середине: оставляем половину в .part — следующая загрузка докачивает.
        let bytes = std::fs::read(&full).unwrap();
        std::fs::remove_file(&full).unwrap();
        std::fs::write(SILERO_VAD.part_path(tmp.path()), &bytes[..bytes.len() / 2]).unwrap();
        let mut first = None;
        SILERO_VAD.download(tmp.path(), &cancel, |done, _| { first.get_or_insert(done); }).unwrap();
        assert_eq!(first, Some(bytes.len() as u64 / 2));
        assert_eq!(std::fs::read(&full).unwrap(), bytes);
        assert!(!SILERO_VAD.part_path(tmp.path()).exists());
    }

    #[test]
    fn sha256_hex() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("x");
        std::fs::write(&p, b"abc").unwrap();
        assert_eq!(sha256_file(&p).unwrap(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }
}
