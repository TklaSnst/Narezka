//! Встроенные шаблоны (ресурсы `templates/*.toml`, по референсам) и свои шаблоны пользователя —
//! по файлу `<id>.toml` на шаблон в `~/.local/share/shortscutter/templates`.
//!
//! Имя файла (`id`) выбирается один раз при сохранении и потом не меняется: переименование правит только
//! `name` внутри, поэтому ссылка пачки на шаблон переживает переименование.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::template::{Template, TemplateError};

/// Встроенные шаблоны: (id, TOML). `name` в файле совпадает с id, подпись в интерфейсе — ключ `template-<id>`.
const BUILTIN: [(&str, &str); 6] = [
    ("terminal", include_str!("../templates/terminal.toml")),
    ("blur-title", include_str!("../templates/blur-title.toml")),
    ("split", include_str!("../templates/split.toml")),
    ("meme", include_str!("../templates/meme.toml")),
    ("full-crop", include_str!("../templates/full-crop.toml")),
    ("clean-title", include_str!("../templates/clean-title.toml")),
];

/// Шаблон по умолчанию для новой пачки.
pub const DEFAULT_BUILTIN: &str = "terminal";

/// Откуда взят шаблон пачки.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum TemplateRef {
    Builtin(String),
    /// Свой шаблон — имя файла без `.toml`.
    User(String),
}

/// Id встроенных шаблонов по порядку показа.
pub fn builtin_ids() -> impl Iterator<Item = &'static str> {
    BUILTIN.iter().map(|(id, _)| *id)
}

/// Встроенный шаблон по id. Ресурсы проверяются тестом, поэтому разбор не падает.
pub fn builtin(id: &str) -> Option<Template> {
    let (_, src) = BUILTIN.iter().find(|(b, _)| *b == id)?;
    Some(Template::from_toml(src).expect("встроенный шаблон разбирается"))
}

#[derive(Debug, thiserror::Error)]
pub enum LibraryError {
    #[error("пустое имя шаблона")]
    EmptyName,
    #[error("шаблон с именем «{0}» уже есть")]
    NameTaken(String),
    #[error("шаблон не найден")]
    NotFound,
    #[error("шаблон некорректен: {0}")]
    Invalid(#[from] TemplateError),
    #[error("{0}")]
    Io(#[from] io::Error),
}

/// Свой шаблон, прочитанный с диска.
#[derive(Debug, Clone, PartialEq)]
pub struct UserTemplate {
    pub id: String,
    pub template: Template,
}

/// Каталог своих шаблонов.
#[derive(Debug, Clone)]
pub struct UserTemplates {
    dir: PathBuf,
}

impl UserTemplates {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    fn file(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.toml"))
    }

    /// Все читаемые шаблоны по имени; испорченные файлы пропускаются (с записью в лог).
    pub fn list(&self) -> Vec<UserTemplate> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else { return Vec::new() };
        let mut out: Vec<UserTemplate> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "toml"))
            .filter_map(|p| {
                let id = p.file_stem()?.to_str()?.to_owned();
                match read(&p) {
                    Ok(template) => Some(UserTemplate { id, template }),
                    Err(e) => {
                        log::warn!("шаблон {p:?} пропущен: {e}");
                        None
                    }
                }
            })
            .collect();
        out.sort_by_key(|u| u.template.name.to_lowercase());
        out
    }

    pub fn load(&self, id: &str) -> Result<Template, LibraryError> {
        let path = self.file(id);
        if !path.exists() {
            return Err(LibraryError::NotFound);
        }
        read(&path)
    }

    /// Имя свободно (без учёта регистра и пробелов по краям); `except` — id, который не считается.
    fn check_name(&self, name: &str, except: Option<&str>) -> Result<String, LibraryError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(LibraryError::EmptyName);
        }
        let taken = self.list().into_iter().any(|u| Some(u.id.as_str()) != except && u.template.name.trim().to_lowercase() == name.to_lowercase());
        if taken {
            return Err(LibraryError::NameTaken(name.to_owned()));
        }
        Ok(name.to_owned())
    }

    /// Новый свой шаблон с именем `name` (копия `template`); возвращает его id.
    pub fn create(&self, name: &str, template: &Template) -> Result<String, LibraryError> {
        let name = self.check_name(name, None)?;
        let base = slug(&name);
        let id = (1..)
            .map(|n| if n == 1 { base.clone() } else { format!("{base}-{n}") })
            .find(|id| !self.file(id).exists())
            .expect("свободное имя файла найдётся");
        let mut t = template.clone();
        t.name = name;
        self.write(&id, &t)?;
        Ok(id)
    }

    /// Перезаписать существующий шаблон (имя остаётся прежним).
    pub fn save(&self, id: &str, template: &Template) -> Result<(), LibraryError> {
        let name = self.load(id)?.name;
        let mut t = template.clone();
        t.name = name;
        self.write(id, &t)
    }

    pub fn rename(&self, id: &str, name: &str) -> Result<Template, LibraryError> {
        let mut t = self.load(id)?;
        t.name = self.check_name(name, Some(id))?;
        self.write(id, &t)?;
        Ok(t)
    }

    pub fn delete(&self, id: &str) -> Result<(), LibraryError> {
        match std::fs::remove_file(self.file(id)) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Err(LibraryError::NotFound),
            r => Ok(r?),
        }
    }

    /// Запись через временный файл и rename — испорченного шаблона при сбое не остаётся.
    fn write(&self, id: &str, t: &Template) -> Result<(), LibraryError> {
        t.validate()?;
        std::fs::create_dir_all(&self.dir)?;
        let tmp = self.dir.join(format!(".{id}.toml.tmp"));
        std::fs::write(&tmp, t.to_toml())?;
        std::fs::rename(&tmp, self.file(id))?;
        Ok(())
    }
}

fn read(path: &Path) -> Result<Template, LibraryError> {
    let s = std::fs::read_to_string(path)?;
    let t = Template::from_toml(&s).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
    t.validate()?;
    Ok(t)
}

/// Имя файла из имени шаблона: буквы и цифры (в т.ч. кириллица) в нижнем регистре, остальное — `-`.
fn slug(name: &str) -> String {
    let mut s = String::new();
    for c in name.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            s.push(c);
        } else if !s.ends_with('-') {
            s.push('-');
        }
    }
    let s: String = s.trim_matches('-').chars().take(48).collect();
    let s = s.trim_end_matches('-');
    if s.is_empty() { "template".into() } else { s.into() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::template::Layer;

    #[test]
    fn builtins_parse_and_validate() {
        for id in builtin_ids() {
            let t = builtin(id).unwrap();
            t.validate().unwrap_or_else(|e| panic!("{id}: {e}"));
            assert_eq!(t.name, id);
            // Тексты, капсулы и баннер — вне зон интерфейса Shorts.
            for (i, l) in t.layers.iter().enumerate() {
                assert!(!l.in_unsafe_zone(), "{id}: слой {i} в зоне интерфейса Shorts");
            }
        }
        assert!(builtin(DEFAULT_BUILTIN).is_some());
        assert!(builtin("нет такого").is_none());
        let split = builtin("split").unwrap();
        assert_eq!(split.main_videos().count(), 2, "два экрана");
        assert!(builtin("clean-title").unwrap().layers.iter().any(|l| matches!(l, Layer::Capsule { .. })));
    }

    #[test]
    fn slugs() {
        assert_eq!(slug("Мой шаблон #1"), "мой-шаблон-1");
        assert_eq!(slug("  ---  "), "template");
        assert_eq!(slug("A/B..c"), "a-b-c");
    }

    #[test]
    fn create_save_rename_delete() {
        let dir = tempfile::tempdir().unwrap();
        let lib = UserTemplates::new(dir.path().join("templates"));
        assert!(lib.list().is_empty(), "каталога ещё нет");
        let base = builtin("terminal").unwrap();

        let a = lib.create("  Мой  ", &base).unwrap();
        assert_eq!(a, "мой");
        assert_eq!(lib.load(&a).unwrap().name, "Мой");
        assert!(matches!(lib.create("мой", &base), Err(LibraryError::NameTaken(_))));
        assert!(matches!(lib.create(" ", &base), Err(LibraryError::EmptyName)));
        // То же имя файла, другое имя шаблона.
        let b = lib.create("мой!", &base).unwrap();
        assert_eq!(b, "мой-2");

        let mut changed = base.clone();
        changed.layers.pop();
        changed.name = "другое".into();
        lib.save(&a, &changed).unwrap();
        let loaded = lib.load(&a).unwrap();
        assert_eq!(loaded.name, "Мой", "сохранение не меняет имя");
        assert_eq!(loaded.layers, changed.layers);

        assert!(matches!(lib.rename(&a, "мой!"), Err(LibraryError::NameTaken(_))));
        assert_eq!(lib.rename(&a, "Мой").unwrap().name, "Мой", "своё же имя можно");
        assert_eq!(lib.rename(&a, "Ярко").unwrap().name, "Ярко");
        let names: Vec<_> = lib.list().into_iter().map(|u| (u.id, u.template.name)).collect();
        assert_eq!(names, [("мой-2".to_owned(), "мой!".to_owned()), ("мой".to_owned(), "Ярко".to_owned())]);

        lib.delete(&b).unwrap();
        assert!(matches!(lib.delete(&b), Err(LibraryError::NotFound)));
        assert!(matches!(lib.load(&b), Err(LibraryError::NotFound)));

        // Испорченный файл не мешает остальным.
        std::fs::write(dir.path().join("templates/битый.toml"), "name = 1").unwrap();
        assert_eq!(lib.list().len(), 1);

        // Некорректный шаблон не записывается.
        let mut bad = base;
        bad.layers.retain(|l| !matches!(l, Layer::MainVideo { .. }));
        assert!(matches!(lib.create("плохой", &bad), Err(LibraryError::Invalid(_))));
    }
}
