//! `settings.toml`: чтение с дефолтами для отсутствующих полей, атомарная запись.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::batch::{ConflictPolicy, OutputSettings};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Language {
    #[default]
    #[serde(rename = "ru")]
    Ru,
    #[serde(rename = "en")]
    En,
}

impl Language {
    pub const ALL: [Language; 2] = [Language::Ru, Language::En];

    pub fn code(self) -> &'static str {
        match self {
            Language::Ru => "ru",
            Language::En => "en",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub language: Language,
    /// Параллельных задач рендера (DECISIONS: 2).
    pub render_jobs: u32,
    /// Перекрытия шортсов (DECISIONS: выключены).
    pub allow_overlaps: bool,
    /// Вывод по умолчанию для новой пачки.
    pub output: OutputSettings,
    /// Папка, из которой в прошлый раз добавляли видео.
    pub last_import_dir: Option<PathBuf>,
    pub analysis: AnalysisSettings,
    /// Длительность новой обложки, кадров (SPEC §5).
    pub cover_frames: u32,
}

/// Распознавание речи и нарезка.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AnalysisSettings {
    /// Свой файл модели whisper; `None` — стандартная large-v3-turbo в каталоге моделей.
    pub whisper_model: Option<PathBuf>,
    /// Язык речи для whisper (`ru`, `en`… или `auto`).
    pub language: String,
    /// Словарь терминов (имена, названия игр и сервисов) — подсказка whisper.
    pub terms: Vec<String>,
}

impl Default for AnalysisSettings {
    fn default() -> Self {
        Self { whisper_model: None, language: "ru".into(), terms: Vec::new() }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            language: Language::Ru,
            render_jobs: 2,
            allow_overlaps: false,
            output: OutputSettings { conflict_policy: ConflictPolicy::Continue, ..Default::default() },
            last_import_dir: None,
            analysis: AnalysisSettings::default(),
            cover_frames: crate::cover::COVER_FRAMES_DEFAULT,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("ошибка чтения/записи {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("ошибка в {path}: {source}")]
    Parse { path: PathBuf, source: toml::de::Error },
}

impl Settings {
    /// Читает настройки; отсутствующий файл — дефолты, отсутствующие поля — дефолты.
    pub fn load(path: &Path) -> Result<Self, SettingsError> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(source) => return Err(SettingsError::Io { path: path.into(), source }),
        };
        let mut s: Settings = toml::from_str(&text)
            .map_err(|source| SettingsError::Parse { path: path.into(), source })?;
        s.render_jobs = s.render_jobs.clamp(1, 8);
        Ok(s)
    }

    /// Как [`Settings::load`], но битый файл не мешает запуску: он переименовывается
    /// в `settings.toml.bad`, а возвращаются дефолты.
    pub fn load_or_recover(path: &Path) -> Self {
        match Self::load(path) {
            Ok(s) => s,
            Err(e) => {
                log::warn!("{e}; используются настройки по умолчанию");
                if matches!(e, SettingsError::Parse { .. }) {
                    let bad = path.with_extension("toml.bad");
                    if let Err(e) = std::fs::rename(path, &bad) {
                        log::warn!("не удалось переименовать {}: {e}", path.display());
                    }
                }
                Self::default()
            }
        }
    }

    /// Пишет во временный файл рядом и переименовывает — файл не бывает недописанным.
    pub fn save(&self, path: &Path) -> Result<(), SettingsError> {
        let io_err = |source| SettingsError::Io { path: path.into(), source };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(io_err)?;
        }
        let text = toml::to_string_pretty(self).expect("настройки всегда сериализуются в TOML");
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text).map_err(io_err)?;
        std::fs::rename(&tmp, path).map_err(io_err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::batch::Codec;

    #[test]
    fn missing_file_gives_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        let s = Settings::load(&tmp.path().join("nope.toml")).unwrap();
        assert_eq!(s, Settings::default());
        assert_eq!(s.language, Language::Ru);
        assert_eq!(s.render_jobs, 2);
    }

    #[test]
    fn save_and_load_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("cfg/settings.toml");
        let mut s = Settings { language: Language::En, ..Default::default() };
        s.output.codec = Codec::Hevc;
        s.output.dir = Some("/tmp/шортсы".into());
        s.save(&path).unwrap();
        assert!(!path.with_extension("toml.tmp").exists());
        assert_eq!(Settings::load(&path).unwrap(), s);
    }

    #[test]
    fn partial_file_fills_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("settings.toml");
        std::fs::write(&path, "language = \"en\"\nrender_jobs = 99\n[output]\nbase_name = \"hrenn\"\n")
            .unwrap();
        let s = Settings::load(&path).unwrap();
        assert_eq!(s.language, Language::En);
        assert_eq!(s.render_jobs, 8);
        assert_eq!(s.output.base_name, "hrenn");
        assert_eq!(s.output.video_bitrate_kbps, 12_000);
        assert_eq!(s.analysis.language, "ru");
    }

    #[test]
    fn analysis_terms_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("settings.toml");
        std::fs::write(&path, "[analysis]\nterms = [\"Lumen VPN\", \"Hyprland\"]\n").unwrap();
        let s = Settings::load(&path).unwrap();
        assert_eq!(s.analysis.terms, ["Lumen VPN", "Hyprland"]);
        assert_eq!(s.analysis.language, "ru");
        assert_eq!(s.analysis.whisper_model, None);
    }

    #[test]
    fn broken_file_is_moved_aside() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("settings.toml");
        std::fs::write(&path, "language = [").unwrap();
        assert!(matches!(Settings::load(&path), Err(SettingsError::Parse { .. })));
        assert_eq!(Settings::load_or_recover(&path), Settings::default());
        assert!(!path.exists());
        assert!(tmp.path().join("settings.toml.bad").exists());
    }
}
