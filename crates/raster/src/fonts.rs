//! Шрифты текстовых слоёв: встроенный комплект (OFL, с кириллицей — файлы и лицензии в `crates/raster/fonts/`),
//! шрифты пользователя (`~/.local/share/shortscutter/fonts`) и системные.
//!
//! Имя шрифта в шаблоне — семейство, начертание можно дописать в конец: «Montserrat ExtraBold»; `sans-serif` и
//! `monospace` — встроенные Montserrat и JetBrains Mono. Встроенные загружаются первыми и при совпадении имён
//! выигрывают у системных, поэтому шаблон на встроенных шрифтах выглядит одинаково на любой системе.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, OnceLock, RwLock};

use cosmic_text::{FontSystem, SwashCache, fontdb};

/// Встроенный комплект.
const BUNDLED: &[&[u8]] = &[
    include_bytes!("../fonts/PressStart2P-Regular.ttf"),
    include_bytes!("../fonts/Montserrat-Regular.ttf"),
    include_bytes!("../fonts/Montserrat-Bold.ttf"),
    include_bytes!("../fonts/Montserrat-ExtraBold.ttf"),
    include_bytes!("../fonts/Montserrat-Black.ttf"),
    include_bytes!("../fonts/JetBrainsMono-Regular.ttf"),
    include_bytes!("../fonts/JetBrainsMono-Bold.ttf"),
    include_bytes!("../fonts/JetBrainsMono-ExtraBold.ttf"),
];

/// Встроенные семейства: пиксельный, жирный гротеск, моноширинный.
pub const PIXEL: &str = "Press Start 2P";
pub const SANS: &str = "Montserrat";
pub const MONO: &str = "JetBrains Mono";

/// Насыщенность ↔ слово в конце имени шрифта.
const WEIGHTS: [(u16, &str); 9] = [
    (100, "Thin"),
    (200, "ExtraLight"),
    (300, "Light"),
    (400, "Regular"),
    (500, "Medium"),
    (600, "SemiBold"),
    (700, "Bold"),
    (800, "ExtraBold"),
    (900, "Black"),
];

pub(crate) struct Fonts {
    pub system: FontSystem,
    pub cache: SwashCache,
}

static USER_DIR: OnceLock<PathBuf> = OnceLock::new();

pub(crate) static FONTS: LazyLock<Mutex<Fonts>> = LazyLock::new(|| {
    let mut db = fontdb::Database::new();
    for data in BUNDLED {
        db.load_font_data(data.to_vec());
    }
    db.load_system_fonts();
    if let Some(dir) = USER_DIR.get() {
        db.load_fonts_dir(dir);
    }
    db.set_sans_serif_family(SANS);
    db.set_monospace_family(MONO);
    db.set_serif_family("DejaVu Serif");
    *LIST.write().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(list(&db)));
    Mutex::new(Fonts { system: FontSystem::new_with_locale_and_db(locale(), db), cache: SwashCache::new() })
});

/// Список семейств для интерфейса — готов после загрузки шрифтов.
static LIST: RwLock<Option<Arc<Vec<FontFamily>>>> = RwLock::new(None);

/// Растёт при каждом подключении шрифта: растры текста, нарисованные раньше, могли взять запасной шрифт.
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Поколение набора шрифтов — кэши растров текста с другим поколением устарели.
pub fn generation() -> u64 {
    GENERATION.load(Ordering::Acquire)
}

/// Каталог шрифтов пользователя; задать до первого обращения к шрифтам.
pub fn set_user_dir(dir: PathBuf) {
    let _ = USER_DIR.set(dir);
}

/// Загружает шрифты (первый раз — сотни миллисекунд); вызывать заранее в фоне.
pub fn warm_up() {
    LazyLock::force(&FONTS);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FontSource {
    Bundled,
    User,
    System,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FontFamily {
    pub name: String,
    pub source: FontSource,
    /// Насыщенности прямых начертаний (100…900), по возрастанию.
    pub weights: Vec<u16>,
}

/// Семейства, если шрифты уже загружены (не ждёт загрузки): встроенные, свои, системные — каждые по алфавиту.
pub fn families() -> Option<Arc<Vec<FontFamily>>> {
    LIST.read().unwrap_or_else(|e| e.into_inner()).clone()
}

fn list(db: &fontdb::Database) -> Vec<FontFamily> {
    let user = USER_DIR.get();
    // Ключ — имя в нижнем регистре; значение — семейство и насыщенности курсивов (на случай семейства из одних курсивов).
    let mut map: BTreeMap<String, (FontFamily, Vec<u16>)> = BTreeMap::new();
    for face in db.faces() {
        let Some((name, _)) = face.families.first() else { continue };
        if name.is_empty() || name.starts_with('.') {
            continue;
        }
        let source = match &face.source {
            fontdb::Source::Binary(_) => FontSource::Bundled,
            fontdb::Source::File(p) | fontdb::Source::SharedFile(p, _) if user.is_some_and(|d| p.starts_with(d)) => FontSource::User,
            _ => FontSource::System,
        };
        let (fam, italic) = map
            .entry(name.to_lowercase())
            .or_insert_with(|| (FontFamily { name: name.clone(), source, weights: Vec::new() }, Vec::new()));
        fam.source = fam.source.min(source);
        let w = round_weight(face.weight.0);
        if face.style == fontdb::Style::Normal { fam.weights.push(w) } else { italic.push(w) }
    }
    let mut out: Vec<FontFamily> = map
        .into_values()
        .map(|(mut f, italic)| {
            if f.weights.is_empty() {
                f.weights = italic;
            }
            f.weights.sort_unstable();
            f.weights.dedup();
            f
        })
        .collect();
    out.sort_by_key(|f| (f.source, f.name.to_lowercase()));
    out
}

fn round_weight(w: u16) -> u16 {
    (w.saturating_add(50) / 100 * 100).clamp(100, 900)
}

/// Слово начертания для насыщенности: 800 → «ExtraBold».
pub fn weight_name(weight: u16) -> &'static str {
    let w = round_weight(weight);
    WEIGHTS.iter().find(|(v, _)| *v == w).map_or("Regular", |(_, n)| n)
}

/// Имя шрифта для шаблона: «Montserrat ExtraBold»; обычное начертание не дописывается.
pub fn font_name(family: &str, weight: u16) -> String {
    match round_weight(weight) {
        400 => family.to_string(),
        w => format!("{family} {}", weight_name(w)),
    }
}

/// `sans-serif`, `serif`, `monospace` и сокращения.
pub fn is_generic(family: &str) -> bool {
    matches!(family.to_ascii_lowercase().as_str(), "sans-serif" | "sans" | "serif" | "monospace" | "mono")
}

/// Семейство и насыщенность по имени шрифта. Слово начертания в конце отделяется, только если семейства
/// с полным именем нет (`is_family`): «JetBrains Mono ExtraBold» → («JetBrains Mono», 800).
pub fn split_font_name(name: &str, is_family: impl Fn(&str) -> bool) -> (&str, u16) {
    if is_generic(name) || is_family(name) {
        return (name, 400);
    }
    if let Some((family, last)) = name.rsplit_once(' ') {
        let last = last.to_ascii_lowercase();
        let alias = match last.as_str() {
            "normal" => Some(400),
            "demibold" => Some(600),
            "ultrabold" => Some(800),
            "heavy" => Some(900),
            _ => None,
        };
        let weight = alias.or_else(|| WEIGHTS.iter().find(|(_, n)| n.eq_ignore_ascii_case(&last)).map(|(w, _)| *w));
        if let Some(w) = weight {
            return (family.trim_end(), w);
        }
    }
    (name, 400)
}

/// Ближайшая к `want` насыщенность из имеющихся.
pub fn nearest_weight(weights: &[u16], want: u16) -> u16 {
    weights.iter().copied().min_by_key(|w| w.abs_diff(want)).unwrap_or(want)
}

#[derive(Debug, thiserror::Error)]
pub enum FontError {
    #[error("не удалось прочитать {path}: {source}")]
    Read { path: PathBuf, source: std::io::Error },
    #[error("не удалось скопировать шрифт в {path}: {source}")]
    Write { path: PathBuf, source: std::io::Error },
    #[error("{0} — не файл шрифта")]
    NotAFont(PathBuf),
    #[error("каталог шрифтов пользователя не задан")]
    NoUserDir,
}

/// Копирует файл шрифта в каталог пользователя и подключает его. Возвращает имя шрифта для шаблона
/// (семейство и начертание первого лица в файле). Тот же файл повторно не копируется.
pub fn install_font(src: &Path) -> Result<String, FontError> {
    let dir = USER_DIR.get().ok_or(FontError::NoUserDir)?;
    let data = std::fs::read(src).map_err(|source| FontError::Read { path: src.into(), source })?;
    let mut probe = fontdb::Database::new();
    probe.load_font_data(data.clone());
    let name = probe
        .faces()
        .find_map(|f| f.families.first().filter(|(n, _)| !n.is_empty()).map(|(n, _)| font_name(n, f.weight.0)))
        .ok_or_else(|| FontError::NotAFont(src.into()))?;

    let file = src.file_name().map_or_else(|| "font.ttf".into(), |n| n.to_string_lossy().into_owned());
    let (stem, ext) = file.rsplit_once('.').map_or((file.as_str(), ""), |(s, e)| (s, e));
    let mut dest = dir.join(&file);
    let mut n = 1;
    loop {
        match std::fs::read(&dest) {
            Ok(old) if old == data => return Ok(name),
            Ok(_) => {
                n += 1;
                dest = dir.join(format!("{stem}-{n}.{ext}"));
            }
            Err(_) => break,
        }
    }
    let write = |e| FontError::Write { path: dest.clone(), source: e };
    std::fs::create_dir_all(dir).map_err(write)?;
    std::fs::write(&dest, &data).map_err(write)?;

    let mut fonts = FONTS.lock().unwrap_or_else(|e| e.into_inner());
    let db = fonts.system.db_mut();
    db.load_font_file(&dest).map_err(write)?;
    *LIST.write().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(list(db)));
    GENERATION.fetch_add(1, Ordering::AcqRel);
    Ok(name)
}

/// Локаль для запасных шрифтов cosmic-text: `ru_RU.UTF-8` → `ru-RU`.
fn locale() -> String {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
        .and_then(|v| v.split(['.', '@']).next().map(|s| s.replace('_', "-")))
        .filter(|s| s != "C" && s != "POSIX")
        .unwrap_or_else(|| "en-US".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        let fam = |n: &str| n == "Foo Black";
        assert_eq!(split_font_name("JetBrains Mono ExtraBold", fam), ("JetBrains Mono", 800));
        assert_eq!(split_font_name("Montserrat", fam), ("Montserrat", 400));
        assert_eq!(split_font_name("Foo Black", fam), ("Foo Black", 400));
        assert_eq!(split_font_name("Foo Black Bold", fam), ("Foo Black", 700));
        assert_eq!(split_font_name("sans-serif", fam), ("sans-serif", 400));
        assert_eq!(split_font_name("sans-serif Bold", fam), ("sans-serif", 700));
        assert_eq!(split_font_name("Press Start 2P", fam), ("Press Start 2P", 400));
        assert_eq!(font_name("Montserrat", 800), "Montserrat ExtraBold");
        assert_eq!(font_name("Montserrat", 400), "Montserrat");
        assert_eq!(font_name("Foo", 350), "Foo");
        assert_eq!(nearest_weight(&[400, 700, 900], 800), 700);
        assert_eq!(nearest_weight(&[400, 700, 900], 850), 900);
    }

    /// Встроенные шрифты на месте, с кириллицей и выигрывают у системных копий с тем же именем.
    #[test]
    fn bundled_fonts() {
        let list = {
            warm_up();
            families().unwrap()
        };
        for (name, weights) in [(PIXEL, &[400][..]), (SANS, &[400, 700, 800, 900]), (MONO, &[400, 700, 800])] {
            let f = list.iter().find(|f| f.name == name).unwrap_or_else(|| panic!("{name}: {list:?}"));
            assert_eq!(f.source, FontSource::Bundled, "{name}");
            // У системной копии того же семейства начертаний может быть больше.
            assert!(weights.iter().all(|w| f.weights.contains(w)), "{name}: {:?}", f.weights);
        }
        assert_eq!(list[0].source, FontSource::Bundled);

        let fonts = FONTS.lock().unwrap();
        let db = fonts.system.db();
        for name in [PIXEL, SANS, MONO] {
            let q = fontdb::Query { families: &[fontdb::Family::Name(name)], ..Default::default() };
            let id = db.query(&q).unwrap();
            assert!(matches!(db.face_source(id).unwrap().0, fontdb::Source::Binary(_)), "{name}: не встроенный");
            let has_cyrillic = db.with_face_data(id, |data, i| {
                let cmap = swash::FontRef::from_index(data, i as usize).unwrap().charmap();
                "ЁжикЪЫЬщ".chars().all(|c| cmap.map(c) != 0)
            });
            assert_eq!(has_cyrillic, Some(true), "{name}");
        }
    }

    #[test]
    fn install() {
        let dir = tempfile::tempdir().unwrap();
        set_user_dir(dir.path().join("fonts"));
        let user = USER_DIR.get().unwrap();
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("fonts/JetBrainsMono-ExtraBold.ttf");
        let generation_before = generation();
        assert_eq!(install_font(&src).unwrap(), "JetBrains Mono ExtraBold");
        assert!(generation() > generation_before);
        assert_eq!(install_font(&src).unwrap(), "JetBrains Mono ExtraBold");
        assert_eq!(std::fs::read_dir(user).unwrap().count(), 1, "тот же файл не копируется второй раз");

        let bad = dir.path().join("JetBrainsMono-ExtraBold.ttf");
        std::fs::write(&bad, b"not a font").unwrap();
        assert!(matches!(install_font(&bad), Err(FontError::NotAFont(_))));
        assert_eq!(std::fs::read_dir(user).unwrap().count(), 1);
    }
}
