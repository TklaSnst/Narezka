//! Транскрипт: слова с таймкодами, сегменты whisper и участки речи (VAD); кэш по хэшу исходника.

use std::io;
use std::ops::Range;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Версия формата кэша; при несовпадении транскрипт распознаётся заново.
pub const FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Word {
    /// Текст слова с примыкающей пунктуацией, без ведущего пробела.
    pub text: String,
    /// Секунды исходника по таймкодам токенов.
    pub start: f64,
    pub end: f64,
    /// Начало слова по DTW (если включено); что точнее — решает пользователь (шаг 0.2).
    #[serde(default)]
    pub dtw: Option<f64>,
    /// Средняя вероятность токенов слова — для подсветки сомнительных мест.
    pub p: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    pub start: f64,
    pub end: f64,
    pub text: String,
    /// Слова сегмента — индексы в [`Transcript::words`].
    pub words: Range<usize>,
}

/// Участок речи по VAD, секунды исходника.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Span {
    pub start: f64,
    pub end: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Transcript {
    pub version: u32,
    /// Имя файла модели whisper.
    pub model: String,
    pub language: String,
    /// Initial prompt (словарь терминов), с которым распознавали.
    pub prompt: String,
    pub words: Vec<Word>,
    pub segments: Vec<Segment>,
    pub speech: Vec<Span>,
}

impl Transcript {
    pub fn cache_file(dir: &Path, hash: &str) -> PathBuf {
        dir.join(format!("{hash}.transcript.json"))
    }

    /// Транскрипт из кэша; `None`, если его нет, он битый или старого формата.
    pub fn load_cached(dir: &Path, hash: &str) -> Option<Self> {
        let path = Self::cache_file(dir, hash);
        let text = std::fs::read_to_string(&path).ok()?;
        match serde_json::from_str::<Transcript>(&text) {
            Ok(t) if t.version == FORMAT_VERSION => Some(t),
            Ok(t) => {
                log::info!("{}: формат {} устарел", path.display(), t.version);
                None
            }
            Err(e) => {
                log::warn!("{}: {e}", path.display());
                None
            }
        }
    }

    /// Атомарная запись (tmp + rename).
    pub fn save_cached(&self, dir: &Path, hash: &str) -> io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let path = Self::cache_file(dir, hash);
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec(self).map_err(io::Error::other)?)?;
        std::fs::rename(&tmp, &path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Transcript {
        Transcript {
            version: FORMAT_VERSION,
            model: "ggml-large-v3-turbo.bin".into(),
            language: "ru".into(),
            prompt: "Lumen VPN, Hyprland".into(),
            words: vec![
                Word { text: "Привет,".into(), start: 1.0, end: 1.4, dtw: Some(1.1), p: 0.9 },
                Word { text: "мир.".into(), start: 1.5, end: 1.9, dtw: None, p: 0.8 },
            ],
            segments: vec![Segment { start: 1.0, end: 1.9, text: "Привет, мир.".into(), words: 0..2 }],
            speech: vec![Span { start: 0.8, end: 2.1 }],
        }
    }

    #[test]
    fn cache_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let t = sample();
        t.save_cached(tmp.path(), "abc").unwrap();
        assert_eq!(Transcript::load_cached(tmp.path(), "abc"), Some(t));
        assert_eq!(Transcript::load_cached(tmp.path(), "nope"), None);
    }

    #[test]
    fn stale_or_broken_cache_is_ignored() {
        let tmp = tempfile::tempdir().unwrap();
        let t = Transcript { version: FORMAT_VERSION + 1, ..sample() };
        t.save_cached(tmp.path(), "old").unwrap();
        assert_eq!(Transcript::load_cached(tmp.path(), "old"), None);
        std::fs::write(Transcript::cache_file(tmp.path(), "bad"), "{").unwrap();
        assert_eq!(Transcript::load_cached(tmp.path(), "bad"), None);
    }
}
