//! Обложка-кадр (SPEC §5): выбранный кадр с надписями стоит первым в шортсе, чтобы YouTube взял его превью.
//!
//! Обложка — свой полный шаблон шортса ([`Cover::template`]): его правят тем же редактором «Оформления»,
//! что и шаблон шортса. Источник кадра ([`CoverSource`]) — кадр исходника или картинка библиотеки;
//! картинка встаёт на место кадра видео (в слои видео и размытый фон), так что любой шаблон работает с обоими.
//!
//! Стили обложек — шаблоны обложек: встроенные ([`builtin_style`]) — надписи поверх шаблона шортса,
//! свои — сохранённые целиком шаблоны (каталог `cover-styles` рядом со своими шаблонами).

use std::ops::RangeInclusive;

use serde::{Deserialize, Serialize};

use crate::asset::AssetId;
use crate::template::{Layer, Template};

/// Длительность обложки по умолчанию, кадров вывода. Сколько нужно YouTube — уточняется экспериментом
/// (DECISIONS, этап 6).
pub const COVER_FRAMES_DEFAULT: u32 = 6;
/// Допустимая длительность обложки, кадров.
pub const COVER_FRAMES: RangeInclusive<u32> = 1..=120;

/// Встроенные стили обложек: (id, слои поверх шаблона шортса в TOML). Подпись — ключ `cover-style-<id>`.
const BUILTIN_STYLES: [(&str, &str); 3] = [
    ("plain", ""),
    ("big-title", include_str!("../templates/cover-big-title.toml")),
    ("top-title", include_str!("../templates/cover-top-title.toml")),
];

/// Стиль обложки новой обложки по умолчанию.
pub const DEFAULT_COVER_STYLE: &str = "big-title";

/// Откуда кадр обложки.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CoverSource {
    /// Кадр исходника шортса в момент `t` (секунды исходника, не обязательно внутри шортса).
    Frame { t: f64 },
    /// Картинка из библиотеки вместо кадра видео.
    Image { asset: AssetId },
}

/// Обложка шортса.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cover {
    pub source: CoverSource,
    /// Как выглядит кадр обложки — полный шаблон (копия шаблона шортса со своими надписями).
    pub template: Template,
    /// Длительность, кадров вывода.
    #[serde(default = "default_frames")]
    pub frames: u32,
    /// «Звук сразу»: обложка закрывает первые кадры шортса, звук идёт с начала (ничего не сдвигается);
    /// иначе обложка добавляется перед шортсом, на ней — тишина.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub sound_now: bool,
}

fn default_frames() -> u32 {
    COVER_FRAMES_DEFAULT
}

impl Cover {
    /// Длительность в секундах при частоте кадров вывода `fps`.
    pub fn duration(&self, fps: f64) -> f64 {
        cover_duration(self.frames, fps)
    }

    /// Сдвиг остального шортса (0 — «звук сразу»).
    pub fn shift(&self, fps: f64) -> f64 {
        if self.sound_now { 0.0 } else { self.duration(fps) }
    }
}

/// Секунды `frames` кадров при `fps` (неизвестная частота — 30).
pub fn cover_duration(frames: u32, fps: f64) -> f64 {
    let fps = if fps.is_finite() && fps > 0.0 { fps } else { 30.0 };
    frames.clamp(*COVER_FRAMES.start(), *COVER_FRAMES.end()) as f64 / fps
}

/// Откуда взят стиль обложки.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum CoverStyleRef {
    /// Встроенный: надписи поверх шаблона шортса.
    Builtin(String),
    /// Свой — имя файла в каталоге стилей обложек без `.toml`.
    User(String),
}

#[derive(Deserialize)]
struct StyleLayers {
    #[serde(default)]
    layers: Vec<Layer>,
}

/// Id встроенных стилей по порядку показа.
pub fn builtin_style_ids() -> impl Iterator<Item = &'static str> {
    BUILTIN_STYLES.iter().map(|(id, _)| *id)
}

/// Встроенный стиль `id`, применённый к шаблону шортса `base`: его слои — поверх.
/// Ресурсы проверяются тестом, поэтому разбор не падает.
pub fn builtin_style(id: &str, base: &Template) -> Option<Template> {
    let (_, src) = BUILTIN_STYLES.iter().find(|(b, _)| *b == id)?;
    let extra: StyleLayers = toml::from_str(src).expect("встроенный стиль обложки разбирается");
    let mut t = base.clone();
    // Субтитров на обложке нет (слов ещё не сказано) — слой не нужен и мешает править.
    t.layers.retain(|l| !matches!(l, Layer::Subtitles { .. }));
    t.layers.extend(extra.layers);
    Some(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_styles_parse_and_validate() {
        let base = Template::minimal();
        for id in builtin_style_ids() {
            let t = builtin_style(id, &base).unwrap();
            t.validate().unwrap_or_else(|e| panic!("{id}: {e}"));
            assert!(t.layers.len() >= base.layers.len());
        }
        assert!(builtin_style("nope", &base).is_none());
    }

    #[test]
    fn duration_in_output_frames() {
        assert!((cover_duration(6, 60.0) - 0.1).abs() < 1e-9);
        assert!((cover_duration(3, 0.0) - 0.1).abs() < 1e-9);
        assert!((cover_duration(0, 30.0) - 1.0 / 30.0).abs() < 1e-9);
        let c = Cover { source: CoverSource::Frame { t: 1.0 }, template: Template::minimal(), frames: 30, sound_now: true };
        assert_eq!(c.shift(30.0), 0.0);
        assert!((Cover { sound_now: false, ..c }.shift(30.0) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn toml_roundtrip() {
        let c = Cover {
            source: CoverSource::Image { asset: AssetId::new() },
            template: Template::minimal(),
            frames: 4,
            sound_now: false,
        };
        let s = toml::to_string(&c).unwrap();
        assert_eq!(toml::from_str::<Cover>(&s).unwrap(), c);
    }
}
