//! Ассеты библиотеки (баннеры, вставки, картинки, фоны, шрифты) и история их использования
//! (ARCHITECTURE §3, Asset). Хранение на диске — крейт `sc-library`.

use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::Rect;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AssetId(pub Uuid);

impl AssetId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for AssetId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for AssetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.simple())
    }
}

/// Для чего ассет. Задаётся при импорте по содержимому (`suggest_kind`), пользователь может сменить.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetKind {
    Banner,
    Insert,
    #[default]
    Image,
    Background,
    Font,
}

impl AssetKind {
    pub const ALL: [AssetKind; 5] = [AssetKind::Banner, AssetKind::Insert, AssetKind::Image, AssetKind::Background, AssetKind::Font];

    /// Ключ i18n названия типа.
    pub fn key(self) -> &'static str {
        match self {
            AssetKind::Banner => "asset-kind-banner",
            AssetKind::Insert => "asset-kind-insert",
            AssetKind::Image => "asset-kind-image",
            AssetKind::Background => "asset-kind-background",
            AssetKind::Font => "asset-kind-font",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    Vertical,
    Horizontal,
    Square,
}

impl Orientation {
    pub fn key(self) -> &'static str {
        match self {
            Orientation::Vertical => "orientation-vertical",
            Orientation::Horizontal => "orientation-horizontal",
            Orientation::Square => "orientation-square",
        }
    }
}

/// Сведения о файле ассета (ffprobe; у шрифта — только имя).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct AssetMeta {
    /// Размер кадра с учётом поворота; 0 — нет (шрифт).
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    /// Длительность; `None` — картинка (один кадр).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fps: Option<f64>,
    /// Есть прозрачность (PNG/WebP/GIF с альфой, WebM VP9 с альфой, ProRes 4444 с альфой).
    #[serde(default)]
    pub alpha: bool,
    #[serde(default)]
    pub has_audio: bool,
    /// Кодек видеопотока по ffprobe (`png`, `gif`, `vp9`, `prores`…).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub codec: String,
    /// Шрифт: имя для шаблона (семейство и начертание).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
}

impl AssetMeta {
    /// Почти квадратные (±10 %) — «квадрат»; у шрифта ориентации нет.
    pub fn orientation(&self) -> Option<Orientation> {
        if self.width == 0 || self.height == 0 {
            return None;
        }
        let r = self.width as f64 / self.height as f64;
        Some(if r > 1.1 {
            Orientation::Horizontal
        } else if r < 1.0 / 1.1 {
            Orientation::Vertical
        } else {
            Orientation::Square
        })
    }

    /// Движется (видео, анимированный GIF).
    pub fn is_animated(&self) -> bool {
        self.duration.is_some()
    }
}

/// Тип по содержимому: шрифт — шрифт; видео без альфы — вставка; с альфой или GIF — баннер; картинка — картинка.
pub fn suggest_kind(meta: &AssetMeta, is_font: bool) -> AssetKind {
    if is_font {
        AssetKind::Font
    } else if meta.alpha || meta.codec == "gif" {
        AssetKind::Banner
    } else if meta.is_animated() {
        AssetKind::Insert
    } else {
        AssetKind::Image
    }
}

/// Ассет библиотеки. Файл — копия в каталоге библиотеки (`file` — имя в нём).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Asset {
    pub id: AssetId,
    pub name: String,
    pub kind: AssetKind,
    pub file: String,
    /// `sc_media::quick_hash` исходного файла — повторный импорт того же файла не копирует его.
    pub hash: String,
    /// Размер файла, байты.
    pub bytes: u64,
    /// Время импорта, секунды Unix.
    pub added: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default)]
    pub meta: AssetMeta,
    /// Вставка: прятать баннер, пока она идёт (шаг 5.3; запоминается у ассета).
    #[serde(default)]
    pub hide_banner: bool,
}

impl Asset {
    /// Каждое слово запроса есть в имени или в одном из тегов (без регистра, «ё» = «е»);
    /// слово с `#` — только в тегах, с начала тега.
    pub fn matches(&self, query: &str) -> bool {
        let name = normalize(&self.name);
        let tags: Vec<String> = self.tags.iter().map(|t| normalize(t)).collect();
        query.split_whitespace().map(normalize).all(|term| match term.strip_prefix('#') {
            Some("") => true,
            Some(tag) => tags.iter().any(|t| t.starts_with(tag)),
            None => name.contains(&term) || tags.iter().any(|t| t.contains(&term)),
        })
    }
}

fn normalize(s: &str) -> String {
    s.chars().flat_map(char::to_lowercase).map(|c| if c == 'ё' { 'е' } else { c }).collect()
}

/// Теги из строки ввода: через запятую (и/или `#`), в нижнем регистре, без повторов; пробелы внутри — `-`
/// (как у хэштегов: запрос поиска делится по пробелам, тег в нём — одно слово).
pub fn parse_tags(s: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in s.split([',', '#', ';']) {
        let t = t.split_whitespace().collect::<Vec<_>>().join("-").to_lowercase();
        if !t.is_empty() && !out.contains(&t) {
            out.push(t);
        }
    }
    out
}

/// Одно использование ассета: где и как (для истории и «как в прошлый раз»).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    pub asset: AssetId,
    /// В какой роли (баннер пачки, вставка шортса, картинка шаблона…).
    pub role: AssetKind,
    /// Секунды Unix.
    pub at: u64,
    /// Позиция на холсте (баннер, картинка).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rect: Option<Rect>,
}

/// История использования ассетов (`history.toml`), новые — в конце.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct History {
    #[serde(default, rename = "use", skip_serializing_if = "Vec::is_empty")]
    pub uses: Vec<Usage>,
    /// Настройки последней отрендеренной пачки — для «Сделать как в прошлый раз?».
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_batch: Option<crate::batch::BatchPreset>,
}

impl History {
    /// Старые записи сверх лимита забываются.
    pub const LIMIT: usize = 500;

    pub fn record(&mut self, usage: Usage) {
        self.uses.push(usage);
        if self.uses.len() > Self::LIMIT {
            let extra = self.uses.len() - Self::LIMIT;
            self.uses.drain(..extra);
        }
    }

    pub fn uses_of(&self, id: AssetId) -> impl Iterator<Item = &Usage> {
        self.uses.iter().filter(move |u| u.asset == id)
    }

    /// Сколько раз использован и когда последний раз.
    pub fn summary(&self, id: AssetId) -> Option<(usize, u64)> {
        self.uses_of(id).fold(None, |acc, u| match acc {
            None => Some((1, u.at)),
            Some((n, last)) => Some((n + 1, last.max(u.at))),
        })
    }

    /// Последнее использование в роли (для «как в прошлый раз»).
    pub fn last(&self, role: AssetKind) -> Option<&Usage> {
        self.uses.iter().rev().find(|u| u.role == role)
    }

    /// Ассет удалён из библиотеки — его записи не нужны.
    pub fn forget(&mut self, id: AssetId) {
        self.uses.retain(|u| u.asset != id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(name: &str, tags: &[&str]) -> Asset {
        Asset {
            id: AssetId::new(),
            name: name.into(),
            kind: AssetKind::Banner,
            file: "x.png".into(),
            hash: "h".into(),
            bytes: 1,
            added: 0,
            tags: tags.iter().map(|t| (*t).to_owned()).collect(),
            meta: AssetMeta::default(),
            hide_banner: false,
        }
    }

    #[test]
    fn tags_parse() {
        assert_eq!(parse_tags(" Lumen VPN,  #реклама #Реклама, ;  осень   2026 "), ["lumen-vpn", "реклама", "осень-2026"]);
        assert!(parse_tags(" , # ").is_empty());
    }

    #[test]
    fn search() {
        let a = asset("Баннер Lumen ёлка", &["реклама", "lumen-vpn"]);
        assert!(a.matches(""));
        assert!(a.matches("lumen"));
        assert!(a.matches("ЕЛКА баннер"));
        assert!(a.matches("рекл"));
        assert!(a.matches("#рекл"));
        assert!(a.matches("#"));
        assert!(!a.matches("#баннер"), "# — только теги");
        assert!(!a.matches("#кла"), "# — с начала тега");
        assert!(!a.matches("lumen осень"));
    }

    #[test]
    fn kinds_and_orientation() {
        let still = AssetMeta { width: 1920, height: 1080, codec: "png".into(), ..Default::default() };
        assert_eq!(still.orientation(), Some(Orientation::Horizontal));
        assert_eq!(suggest_kind(&still, false), AssetKind::Image);
        let alpha = AssetMeta { alpha: true, ..still.clone() };
        assert_eq!(suggest_kind(&alpha, false), AssetKind::Banner);
        let clip = AssetMeta { width: 1080, height: 1920, duration: Some(12.0), codec: "h264".into(), ..Default::default() };
        assert_eq!(clip.orientation(), Some(Orientation::Vertical));
        assert_eq!(suggest_kind(&clip, false), AssetKind::Insert);
        let alpha_clip = AssetMeta { alpha: true, ..clip };
        assert_eq!(suggest_kind(&alpha_clip, false), AssetKind::Banner);
        let gif = AssetMeta { width: 500, height: 480, duration: Some(2.0), codec: "gif".into(), ..Default::default() };
        assert_eq!(gif.orientation(), Some(Orientation::Square));
        assert_eq!(suggest_kind(&gif, false), AssetKind::Banner);
        assert_eq!(AssetMeta::default().orientation(), None);
        assert_eq!(suggest_kind(&AssetMeta::default(), true), AssetKind::Font);
    }

    #[test]
    fn history() {
        let (a, b) = (AssetId::new(), AssetId::new());
        let mut h = History::default();
        assert_eq!(h.summary(a), None);
        h.record(Usage { asset: a, role: AssetKind::Banner, at: 10, rect: Some(Rect::new(0.1, 0.2, 0.3, 0.1)) });
        h.record(Usage { asset: b, role: AssetKind::Insert, at: 20, rect: None });
        h.record(Usage { asset: a, role: AssetKind::Banner, at: 30, rect: None });
        assert_eq!(h.summary(a), Some((2, 30)));
        assert_eq!(h.last(AssetKind::Insert).unwrap().asset, b);
        assert_eq!(h.last(AssetKind::Banner).unwrap().at, 30);

        let text = toml::to_string_pretty(&h).unwrap();
        assert!(text.contains("[[use]]"), "{text}");
        assert_eq!(toml::from_str::<History>(&text).unwrap(), h);
        // С настройками прошлой пачки (шаблон с массивами слоёв после массива записей).
        let banner = crate::Banner { asset: a, text: Some(crate::BannerText { content: "код".into(), ..Default::default() }) };
        let batch = crate::Batch { banner: Some(banner), ..Default::default() };
        h.last_batch = Some(batch.preset(42));
        let text = toml::to_string_pretty(&h).unwrap();
        assert_eq!(toml::from_str::<History>(&text).unwrap(), h);
        let mut fresh = crate::Batch::default();
        fresh.apply_preset(h.last_batch.as_ref().unwrap());
        assert_eq!((fresh.banner, fresh.template), (batch.banner, batch.template));
        assert_eq!(toml::from_str::<History>("").unwrap(), History::default());

        h.forget(a);
        assert_eq!(h.uses.len(), 1);
        for i in 0..History::LIMIT + 5 {
            h.record(Usage { asset: b, role: AssetKind::Insert, at: 100 + i as u64, rect: None });
        }
        assert_eq!(h.uses.len(), History::LIMIT);
        assert_eq!(h.uses[0].at, 105, "старые забыты");
    }
}
