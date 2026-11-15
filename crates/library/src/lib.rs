//! Библиотека ассетов (`~/.local/share/shortscutter/library`) и история использования (`history.toml`).
//!
//! Импорт копирует файл в каталог библиотеки под именем `<id>.<расширение>` — ассет не ломается, если
//! исходный файл переместили или удалили. Описание всех ассетов — `library.toml` рядом, миниатюры — `thumbs/<id>.png`.
//! Импорт идёт в два шага: `stage` (копия, ffprobe, миниатюра — долго, в фоновом потоке) и `Library::add`
//! (в UI-потоке, запись индекса).

use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use sc_core::{Asset, AssetId, AssetKind, AssetMeta, History, suggest_kind};
use serde::{Deserialize, Serialize};

/// Расширения шрифтов: их не разбирает ffprobe, имя шрифта узнаёт растеризатор.
pub const FONT_EXTS: &[&str] = &["ttf", "otf", "ttc", "otc"];
pub const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "webp", "gif", "bmp"];
pub const VIDEO_EXTS: &[&str] = &["mp4", "mkv", "webm", "mov", "m4v"];

/// Сторона миниатюры, px.
pub const THUMB_SIZE: u32 = 320;

const INDEX: &str = "library.toml";

#[derive(Debug, thiserror::Error)]
pub enum LibraryError {
    #[error("файл уже есть в библиотеке")]
    Duplicate(String),
    #[error("ассет не найден")]
    NotFound,
    #[error("пустое имя")]
    EmptyName,
    #[error("не удалось прочитать файл: {0}")]
    Media(#[from] sc_media::MediaError),
    #[error("{0}")]
    Io(#[from] io::Error),
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Index {
    #[serde(default, rename = "asset")]
    assets: Vec<Asset>,
}

/// Ассет, файл которого уже скопирован в библиотеку, но ещё не записан в индекс.
#[derive(Debug)]
pub struct Staged {
    pub asset: Asset,
    dir: PathBuf,
}

impl Staged {
    pub fn path(&self) -> PathBuf {
        self.dir.join(&self.asset.file)
    }

    /// Убрать скопированное (повтор или ошибка после копирования).
    pub fn discard(self) {
        let _ = std::fs::remove_file(self.path());
        let _ = std::fs::remove_file(thumb_path(&self.dir, self.asset.id));
    }
}

pub fn thumb_path(dir: &Path, id: AssetId) -> PathBuf {
    dir.join("thumbs").join(format!("{id}.png"))
}

pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn ext_of(path: &Path) -> String {
    path.extension().and_then(|e| e.to_str()).map_or_else(String::new, str::to_lowercase)
}

pub fn is_font(path: &Path) -> bool {
    FONT_EXTS.contains(&ext_of(path).as_str())
}

/// Копия файла в библиотеку, сведения о нём и миниатюра. Долго (копия видео, ffprobe, ffmpeg) — не в UI-потоке.
/// `known(hash)` — уже есть в библиотеке: тогда ничего не копируется, `Err(Duplicate(hash))`.
/// У шрифта `meta.font` пусто — его заполняет вызывающий (растеризатор знает имя).
pub fn stage(dir: &Path, src: &Path, known: impl Fn(&str) -> bool) -> Result<Staged, LibraryError> {
    let hash = sc_media::quick_hash(src)?;
    if known(&hash) {
        return Err(LibraryError::Duplicate(hash));
    }
    let id = AssetId::new();
    let ext = ext_of(src);
    let file = if ext.is_empty() { id.to_string() } else { format!("{id}.{ext}") };
    std::fs::create_dir_all(dir.join("thumbs"))?;
    let dest = dir.join(&file);
    let tmp = dir.join(format!(".{file}.tmp"));
    let bytes = std::fs::copy(src, &tmp).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })?;
    std::fs::rename(&tmp, &dest)?;

    let font = is_font(src);
    let name = src.file_stem().map_or_else(|| file.clone(), |s| s.to_string_lossy().into_owned());
    let mut staged = Staged {
        asset: Asset {
            id,
            name,
            kind: AssetKind::Image,
            file,
            hash,
            bytes,
            added: now(),
            tags: Vec::new(),
            meta: AssetMeta::default(),
            hide_banner: false,
        },
        dir: dir.to_owned(),
    };
    if !font {
        match sc_media::probe_asset(&dest) {
            Ok(meta) => staged.asset.meta = meta,
            Err(e) => {
                staged.discard();
                return Err(e.into());
            }
        }
        if let Err(e) = sc_media::thumbnail(&dest, &staged.asset.meta, &thumb_path(dir, id), THUMB_SIZE) {
            log::warn!("миниатюра {}: {e}", src.display());
        }
    }
    staged.asset.kind = suggest_kind(&staged.asset.meta, font);
    Ok(staged)
}

/// Итог `Library::add`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Added {
    New(AssetId),
    /// Такой файл уже был — новая копия удалена.
    Duplicate(AssetId),
}

/// Ассеты библиотеки (индекс в памяти, каждая правка сразу пишется на диск).
#[derive(Debug)]
pub struct Library {
    dir: PathBuf,
    assets: Vec<Asset>,
}

impl Library {
    /// Читает `library.toml`; испорченный файл откладывается в `library.toml.bad`, библиотека — пустая.
    pub fn open(dir: impl Into<PathBuf>) -> Self {
        let dir = dir.into();
        let path = dir.join(INDEX);
        let assets = match std::fs::read_to_string(&path) {
            Ok(s) => match toml::from_str::<Index>(&s) {
                Ok(i) => i.assets,
                Err(e) => {
                    log::error!("{}: {e}; библиотека начата заново", path.display());
                    let _ = std::fs::rename(&path, path.with_extension("toml.bad"));
                    Vec::new()
                }
            },
            Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(e) => {
                log::error!("{}: {e}", path.display());
                Vec::new()
            }
        };
        Self { dir, assets }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// В порядке импорта.
    pub fn assets(&self) -> &[Asset] {
        &self.assets
    }

    pub fn get(&self, id: AssetId) -> Option<&Asset> {
        self.assets.iter().find(|a| a.id == id)
    }

    pub fn by_hash(&self, hash: &str) -> Option<&Asset> {
        self.assets.iter().find(|a| a.hash == hash)
    }

    /// Файл ассета в библиотеке.
    pub fn path(&self, a: &Asset) -> PathBuf {
        self.dir.join(&a.file)
    }

    pub fn thumb(&self, id: AssetId) -> PathBuf {
        thumb_path(&self.dir, id)
    }

    /// Файл ассета пропал из каталога библиотеки.
    pub fn is_missing(&self, a: &Asset) -> bool {
        !self.path(a).exists()
    }

    pub fn add(&mut self, staged: Staged) -> Result<Added, LibraryError> {
        if let Some(old) = self.by_hash(&staged.asset.hash) {
            let id = old.id;
            staged.discard();
            return Ok(Added::Duplicate(id));
        }
        let id = staged.asset.id;
        self.assets.push(staged.asset);
        self.save()?;
        Ok(Added::New(id))
    }

    /// Новые сверху; `kind` — только этого типа; запрос — `Asset::matches`.
    pub fn search(&self, query: &str, kind: Option<AssetKind>) -> Vec<&Asset> {
        let mut out: Vec<&Asset> =
            self.assets.iter().filter(|a| kind.is_none_or(|k| a.kind == k) && a.matches(query)).collect();
        out.sort_by_key(|a| std::cmp::Reverse(a.added));
        out
    }

    /// Все теги с числом ассетов, по алфавиту.
    pub fn tags(&self) -> Vec<(String, usize)> {
        let mut map = std::collections::BTreeMap::<String, usize>::new();
        for t in self.assets.iter().flat_map(|a| &a.tags) {
            *map.entry(t.clone()).or_default() += 1;
        }
        map.into_iter().collect()
    }

    /// Правка описания ассета (имя, тип, теги, настройки) с записью индекса.
    pub fn update(&mut self, id: AssetId, f: impl FnOnce(&mut Asset)) -> Result<(), LibraryError> {
        let a = self.assets.iter_mut().find(|a| a.id == id).ok_or(LibraryError::NotFound)?;
        f(a);
        self.save()
    }

    pub fn rename(&mut self, id: AssetId, name: &str) -> Result<(), LibraryError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(LibraryError::EmptyName);
        }
        self.update(id, |a| a.name = name.to_owned())
    }

    /// Удаляет запись, файл и миниатюру.
    pub fn delete(&mut self, id: AssetId) -> Result<Asset, LibraryError> {
        let i = self.assets.iter().position(|a| a.id == id).ok_or(LibraryError::NotFound)?;
        let a = self.assets.remove(i);
        self.save()?;
        for p in [self.path(&a), self.thumb(id)] {
            if let Err(e) = std::fs::remove_file(&p)
                && e.kind() != io::ErrorKind::NotFound
            {
                log::warn!("{}: {e}", p.display());
            }
        }
        Ok(a)
    }

    /// Через временный файл и rename — индекс не бывает недописанным.
    fn save(&self) -> Result<(), LibraryError> {
        std::fs::create_dir_all(&self.dir)?;
        let index = Index { assets: self.assets.clone() };
        let text = toml::to_string_pretty(&index).expect("индекс сериализуется в TOML");
        let path = self.dir.join(INDEX);
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }
}

/// `history.toml`; нет или испорчен — пустая история (испорченный откладывается в `.bad`).
pub fn load_history(path: &Path) -> History {
    match std::fs::read_to_string(path) {
        Ok(s) => toml::from_str(&s).unwrap_or_else(|e| {
            log::error!("{}: {e}; история начата заново", path.display());
            let _ = std::fs::rename(path, path.with_extension("toml.bad"));
            History::default()
        }),
        Err(_) => History::default(),
    }
}

pub fn save_history(path: &Path, h: &History) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, toml::to_string_pretty(h).expect("история сериализуется в TOML"))?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sc_core::Usage;

    fn has_ffmpeg() -> bool {
        std::process::Command::new("ffmpeg").arg("-version").output().is_ok_and(|o| o.status.success())
    }

    fn ffmpeg(args: &[&str]) {
        let ok = std::process::Command::new("ffmpeg").args(["-v", "error", "-y"]).args(args).status().unwrap().success();
        assert!(ok, "ffmpeg {args:?}");
    }

    #[test]
    fn open_missing_and_broken_index() {
        let dir = tempfile::tempdir().unwrap();
        assert!(Library::open(dir.path().join("нет")).assets().is_empty());
        std::fs::write(dir.path().join(INDEX), "asset = 1").unwrap();
        assert!(Library::open(dir.path()).assets().is_empty());
        assert!(dir.path().join("library.toml.bad").exists());
    }

    #[test]
    fn font_import_without_ffprobe() {
        let src = tempfile::tempdir().unwrap();
        let font = src.path().join("Мой Шрифт.TTF");
        std::fs::write(&font, b"not really a font").unwrap();
        let dir = tempfile::tempdir().unwrap();
        let lib_dir = dir.path().join("library");

        let mut s = stage(&lib_dir, &font, |_| false).unwrap();
        assert_eq!(s.asset.kind, AssetKind::Font);
        assert_eq!(s.asset.name, "Мой Шрифт");
        assert!(s.asset.file.ends_with(".ttf"));
        assert_eq!(s.asset.bytes, 17);
        s.asset.meta.font = Some("Мой Шрифт Bold".into());
        let mut lib = Library::open(&lib_dir);
        let Added::New(id) = lib.add(s).unwrap() else { panic!() };
        assert!(lib.path(lib.get(id).unwrap()).exists());

        // Тот же файл ещё раз: отсекается ещё до копии…
        let hash = lib.get(id).unwrap().hash.clone();
        assert!(matches!(stage(&lib_dir, &font, |h| h == hash), Err(LibraryError::Duplicate(_))));
        // …а если проверка не успела — `add` удаляет новую копию.
        let again = stage(&lib_dir, &font, |_| false).unwrap();
        let copy = again.path();
        assert_eq!(lib.add(again).unwrap(), Added::Duplicate(id));
        assert!(!copy.exists());

        // Правки переживают перечитывание индекса.
        lib.rename(id, "  Заголовки ").unwrap();
        assert!(matches!(lib.rename(id, " "), Err(LibraryError::EmptyName)));
        lib.update(id, |a| a.tags = sc_core::parse_tags("шрифт, Терминал")).unwrap();
        let lib2 = Library::open(&lib_dir);
        let a = lib2.get(id).unwrap();
        assert_eq!((a.name.as_str(), a.meta.font.as_deref()), ("Заголовки", Some("Мой Шрифт Bold")));
        assert_eq!(lib2.tags(), [("терминал".to_owned(), 1), ("шрифт".to_owned(), 1)]);
        assert_eq!(lib2.search("#терм", None).len(), 1);
        assert!(lib2.search("", Some(AssetKind::Banner)).is_empty());

        let path = lib.path(lib.get(id).unwrap());
        lib.delete(id).unwrap();
        assert!(!path.exists());
        assert!(Library::open(&lib_dir).assets().is_empty());
        assert!(matches!(lib.delete(id), Err(LibraryError::NotFound)));
    }

    #[test]
    fn media_import() {
        if !has_ffmpeg() {
            eprintln!("ffmpeg нет — пропуск");
            return;
        }
        let src = tempfile::tempdir().unwrap();
        let p = |n: &str| src.path().join(n).to_string_lossy().into_owned();
        // Баннер с прозрачностью, горизонтальная вставка со звуком, VP9 с альфой, ProRes 4444, GIF, битый файл.
        ffmpeg(&["-f", "lavfi", "-i", "color=c=red@0.5:s=600x200,format=rgba", "-frames:v", "1", &p("banner.png")]);
        ffmpeg(&[
            "-f", "lavfi", "-i", "testsrc2=s=320x180:r=30:d=2", "-f", "lavfi", "-i", "sine=d=2", "-c:v", "libx264", "-pix_fmt",
            "yuv420p", "-c:a", "aac", "-shortest", &p("insert.mp4"),
        ]);
        ffmpeg(&[
            "-f", "lavfi", "-i", "color=c=blue@0.3:s=200x400:r=25:d=1,format=yuva420p", "-c:v", "libvpx-vp9", "-auto-alt-ref", "0",
            &p("alpha.webm"),
        ]);
        ffmpeg(&[
            "-f", "lavfi", "-i", "color=c=green@0.3:s=256x256:r=25:d=1,format=yuva444p10le", "-c:v", "prores_ks", "-profile:v", "4444",
            &p("alpha.mov"),
        ]);
        ffmpeg(&["-f", "lavfi", "-i", "testsrc2=s=160x120:r=10:d=1", &p("anim.gif")]);
        std::fs::write(p("broken.png"), b"nope").unwrap();

        let dir = tempfile::tempdir().unwrap();
        let mut lib = Library::open(dir.path());
        let mut add = |name: &str| {
            let s = stage(dir.path(), Path::new(&p(name)), |_| false).unwrap();
            assert!(thumb_path(dir.path(), s.asset.id).exists(), "миниатюра {name}");
            let Added::New(id) = lib.add(s).unwrap() else { panic!() };
            lib.get(id).unwrap().clone()
        };
        let banner = add("banner.png");
        assert_eq!((banner.kind, banner.meta.width, banner.meta.alpha, banner.meta.duration), (AssetKind::Banner, 600, true, None));
        let insert = add("insert.mp4");
        assert_eq!(insert.kind, AssetKind::Insert);
        assert!(insert.meta.has_audio && !insert.meta.alpha);
        assert!((insert.meta.duration.unwrap() - 2.0).abs() < 0.1);
        assert_eq!(insert.meta.orientation(), Some(sc_core::Orientation::Horizontal));
        let webm = add("alpha.webm");
        assert!(webm.meta.alpha, "VP9 с альфой");
        assert_eq!(webm.kind, AssetKind::Banner);
        assert_eq!(webm.meta.orientation(), Some(sc_core::Orientation::Vertical));
        assert!(add("alpha.mov").meta.alpha, "ProRes 4444");
        let gif = add("anim.gif");
        assert!(gif.meta.is_animated() && gif.kind == AssetKind::Banner);

        // Миниатюра VP9 с альфой — прозрачная (если libvpx есть).
        let thumb = std::fs::read(thumb_path(dir.path(), webm.id)).unwrap();
        assert!(thumb.starts_with(b"\x89PNG"));

        let before: Vec<_> = std::fs::read_dir(dir.path()).unwrap().flatten().map(|e| e.file_name()).collect();
        assert!(matches!(stage(dir.path(), Path::new(&p("broken.png")), |_| false), Err(LibraryError::Media(_))));
        let after: Vec<_> = std::fs::read_dir(dir.path()).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(before.len(), after.len(), "битый файл не оставляет копию");
        assert_eq!(lib.search("", None).len(), 5);
    }

    #[test]
    fn history_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.toml");
        assert_eq!(load_history(&path), History::default());
        let mut h = History::default();
        h.record(Usage { asset: AssetId::new(), role: AssetKind::Banner, at: 5, rect: None });
        save_history(&path, &h).unwrap();
        assert_eq!(load_history(&path), h);
        std::fs::write(&path, "use = 3").unwrap();
        assert_eq!(load_history(&path), History::default());
        assert!(dir.path().join("history.toml.bad").exists());
    }
}
