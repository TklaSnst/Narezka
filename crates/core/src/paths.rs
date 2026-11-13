//! Каталоги приложения по XDG (ARCHITECTURE §7).

use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    /// `~/.config/shortscutter` — `settings.toml`, пресеты качества.
    pub config: PathBuf,
    /// `~/.local/share/shortscutter` — шаблоны, библиотека, история, шрифты.
    pub data: PathBuf,
    /// `~/.cache/shortscutter` — модели, кэш анализа, прокси, временные файлы.
    pub cache: PathBuf,
}

impl Paths {
    /// Каталоги текущего пользователя; `None`, если не удалось определить домашний каталог.
    pub fn from_env() -> Option<Self> {
        let dirs = directories::ProjectDirs::from("", "", "shortscutter")?;
        Some(Self {
            config: dirs.config_dir().to_path_buf(),
            data: dirs.data_dir().to_path_buf(),
            cache: dirs.cache_dir().to_path_buf(),
        })
    }

    /// Все каталоги внутри одного корня — для тестов и портативного запуска.
    pub fn with_root(root: &Path) -> Self {
        Self {
            config: root.join("config"),
            data: root.join("data"),
            cache: root.join("cache"),
        }
    }

    pub fn settings_file(&self) -> PathBuf {
        self.config.join("settings.toml")
    }
    /// Свои промпты нарезки — перекрывают встроенные одноимённые файлы.
    pub fn prompts_dir(&self) -> PathBuf {
        self.config.join("prompts")
    }
    pub fn templates_dir(&self) -> PathBuf {
        self.data.join("templates")
    }
    pub fn library_dir(&self) -> PathBuf {
        self.data.join("library")
    }
    pub fn history_file(&self) -> PathBuf {
        self.data.join("history.toml")
    }
    pub fn fonts_dir(&self) -> PathBuf {
        self.data.join("fonts")
    }
    pub fn models_dir(&self) -> PathBuf {
        self.cache.join("models")
    }
    pub fn analysis_dir(&self) -> PathBuf {
        self.cache.join("analysis")
    }
    pub fn proxy_dir(&self) -> PathBuf {
        self.cache.join("proxy")
    }
    pub fn tmp_dir(&self) -> PathBuf {
        self.cache.join("tmp")
    }

    /// Создаёт все каталоги, если их нет.
    pub fn ensure(&self) -> io::Result<()> {
        for dir in [
            self.config.clone(),
            self.templates_dir(),
            self.library_dir(),
            self.fonts_dir(),
            self.models_dir(),
            self.analysis_dir(),
            self.proxy_dir(),
            self.tmp_dir(),
        ] {
            std::fs::create_dir_all(dir)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_under_root() {
        let tmp = tempfile::tempdir().unwrap();
        let p = Paths::with_root(tmp.path());
        p.ensure().unwrap();
        assert!(p.tmp_dir().is_dir());
        assert!(p.templates_dir().is_dir());
        assert_eq!(p.settings_file(), tmp.path().join("config/settings.toml"));
    }

    #[test]
    fn from_env_uses_app_name() {
        if let Some(p) = Paths::from_env() {
            assert!(p.config.ends_with("shortscutter"));
            assert!(p.cache.ends_with("shortscutter"));
        }
    }
}
