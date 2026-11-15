//! Рекламная вставка (SPEC §4): клип, на время которого основное видео шортса останавливается.
//!
//! Ассет вставки выбирается на пачку ([`BatchInsert`]), место — у каждого шортса (`Short::insert_at`,
//! секунды исходника, обычно пауза между фразами). Вертикальная вставка идёт на весь экран, горизонтальная —
//! в место основного видео шаблона ([`InsertMode`]); что прятать на время вставки, решает [`InsertPlan`] —
//! один и тот же для превью и рендера.

use serde::{Deserialize, Serialize};

use crate::asset::{AssetId, AssetMeta, Orientation};
use crate::template::{Layer, Template};

/// Предел длины шортса у YouTube (с октября 2024 — 3 минуты).
pub const SHORTS_MAX: f64 = 180.0;
/// Вставка не ближе к краю шортса, с.
pub const INSERT_EDGE: f64 = 0.2;
/// Пауза между словами не короче, с, — место, к которому прилипает маркер вставки.
pub const MIN_PAUSE: f64 = 0.15;

/// Как показывать вставку.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InsertMode {
    /// На весь холст (вертикальные вставки).
    Fullscreen,
    /// В место основного видео шаблона (горизонтальные).
    Slot,
}

impl InsertMode {
    pub const ALL: [InsertMode; 2] = [InsertMode::Fullscreen, InsertMode::Slot];

    pub fn key(self) -> &'static str {
        match self {
            InsertMode::Fullscreen => "insert-mode-fullscreen",
            InsertMode::Slot => "insert-mode-slot",
        }
    }

    /// По ориентации клипа: вертикальный — весь экран, остальные — в слот видео.
    pub fn auto(meta: &AssetMeta) -> InsertMode {
        match meta.orientation() {
            Some(Orientation::Vertical) => InsertMode::Fullscreen,
            _ => InsertMode::Slot,
        }
    }
}

/// Вставка пачки.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BatchInsert {
    pub asset: AssetId,
    /// Ручной выбор; `None` — по ориентации ([`InsertMode::auto`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<InsertMode>,
}

impl BatchInsert {
    pub fn mode(&self, meta: &AssetMeta) -> InsertMode {
        self.mode.unwrap_or_else(|| InsertMode::auto(meta))
    }
}

/// Где вставка в порядке слоёв.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertZ {
    /// Сразу после кадра видео слоя `i` (рамка и прочее «над слоем» — поверх вставки).
    AfterVideo(usize),
    /// Перед слоем `i` (`layers.len()` — поверх всего).
    Before(usize),
}

/// Как вписать кадр вставки в прямоугольник: что взять из кадра и куда положить внутри прямоугольника.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InsertFit {
    /// Кроп кадра вставки, пиксели кадра.
    pub crop: (u32, u32, u32, u32),
    /// Куда внутри прямоугольника (от его левого верхнего угла), пиксели холста.
    pub out: (u32, u32, u32, u32),
}

/// План показа вставки в шаблоне — общий для превью и рендера.
#[derive(Debug, Clone, PartialEq)]
pub struct InsertPlan {
    pub mode: InsertMode,
    pub z: InsertZ,
    /// Прямоугольник вставки в пикселях холста (чётный): холст или место видео.
    pub dest: (u32, u32, u32, u32),
    /// Скругление углов, пиксели холста.
    pub radius: f32,
    /// Слои, которых во время вставки не видно (субтитры; баннер, если вставка его прячет;
    /// на весь экран — всё, что выше вставки, кроме баннера).
    pub hidden: Vec<usize>,
}

impl Template {
    /// План вставки для шаблона с баннером (`banner_layers` — слои баннера из [`Template::with_banner_layers`]).
    /// Режим «в слот» без видео в шаблоне — на весь экран.
    pub fn insert_plan(&self, mode: InsertMode, banner_layers: &[usize], hide_banner: bool) -> InsertPlan {
        let canvas = self.canvas;
        let n = self.layers.len();
        let is_banner = |i: usize| banner_layers.contains(&i);
        let mut hidden: Vec<usize> =
            (0..n).filter(|&i| matches!(self.layers[i], Layer::Subtitles { .. }) || hide_banner && is_banner(i)).collect();
        let slot = (mode == InsertMode::Slot)
            .then(|| {
                self.layers
                    .iter()
                    .enumerate()
                    .filter_map(|(i, l)| match l {
                        Layer::MainVideo { dest, corner_radius, .. } => Some((i, *dest, *corner_radius)),
                        _ => None,
                    })
                    .max_by(|a, b| (a.1.w * a.1.h).total_cmp(&(b.1.w * b.1.h)))
            })
            .flatten();
        match slot {
            Some((i, dest, radius)) => {
                let d = dest.to_pixels_even(canvas.width, canvas.height);
                let radius = (radius * canvas.width as f32).min(d.2.min(d.3) as f32 / 2.0);
                InsertPlan { mode: InsertMode::Slot, z: InsertZ::AfterVideo(i), dest: d, radius, hidden }
            }
            None => {
                // Баннер виден — вставка под ним; всё остальное выше вставки прячется.
                let before = if hide_banner { n } else { banner_layers.iter().copied().min().unwrap_or(n) };
                hidden.extend((before..n).filter(|&i| !is_banner(i)));
                hidden.sort_unstable();
                hidden.dedup();
                InsertPlan {
                    mode: InsertMode::Fullscreen,
                    z: InsertZ::Before(before),
                    dest: (0, 0, canvas.width, canvas.height),
                    radius: 0.0,
                    hidden,
                }
            }
        }
    }
}

/// Кадр `src` в прямоугольник `w`×`h`: «cover» (заполнить, обрезав края), если теряется не больше
/// пятой части кадра, иначе целиком с полями. Размеры чётные (yuv420).
pub fn insert_fit(src: (u32, u32), w: u32, h: u32) -> InsertFit {
    let even = |v: f64| (((v / 2.0).round() as u32) * 2).max(2);
    if src.0 == 0 || src.1 == 0 || w == 0 || h == 0 {
        return InsertFit { crop: (0, 0, src.0.max(2), src.1.max(2)), out: (0, 0, w, h) };
    }
    let (sa, da) = (src.0 as f64 / src.1 as f64, w as f64 / h as f64);
    let ratio = if sa > da { sa / da } else { da / sa };
    if ratio <= 1.25 {
        let (cw, ch) = if sa > da { (even(src.1 as f64 * da).min(src.0), src.1) } else { (src.0, even(src.0 as f64 / da).min(src.1)) };
        let (cx, cy) = (((src.0 - cw) / 4) * 2, ((src.1 - ch) / 4) * 2);
        InsertFit { crop: (cx, cy, cw, ch), out: (0, 0, w, h) }
    } else {
        let (ow, oh) = if sa > da { (w, even(w as f64 / sa).min(h)) } else { (even(h as f64 * sa).min(w), h) };
        InsertFit { crop: (0, 0, src.0, src.1), out: (((w - ow) / 4) * 2, ((h - oh) / 4) * 2, ow, oh) }
    }
}

/// Паузы между словами внутри `[start, end)` (секунды исходника): середины промежутков не короче [`MIN_PAUSE`],
/// не ближе [`INSERT_EDGE`] к краям.
pub fn pauses(words: impl IntoIterator<Item = (f64, f64)>, start: f64, end: f64) -> Vec<f64> {
    let mut out = Vec::new();
    let mut prev_end: Option<f64> = None;
    for (s, e) in words {
        if let Some(pe) = prev_end
            && s - pe >= MIN_PAUSE
        {
            let mid = (pe + s) / 2.0;
            if mid > start + INSERT_EDGE && mid < end - INSERT_EDGE {
                out.push(mid);
            }
        }
        prev_end = Some(prev_end.map_or(e, |p: f64| p.max(e)));
    }
    out
}

/// Ближайшая к `t` пауза не дальше `window` секунд; нет такой — `t`.
pub fn snap_to_pause(t: f64, pauses: &[f64], window: f64) -> f64 {
    pauses
        .iter()
        .copied()
        .filter(|p| (p - t).abs() <= window)
        .min_by(|a, b| (a - t).abs().total_cmp(&(b - t).abs()))
        .unwrap_or(t)
}

/// Время вставки внутри шортса `[start, end)` годится: не у самого края.
pub fn insert_inside(at: f64, start: f64, end: f64) -> bool {
    at.is_finite() && at >= start + INSERT_EDGE && at <= end - INSERT_EDGE
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Rect;

    fn meta(w: u32, h: u32) -> AssetMeta {
        AssetMeta { width: w, height: h, duration: Some(10.0), ..Default::default() }
    }

    #[test]
    fn auto_mode_by_orientation() {
        assert_eq!(InsertMode::auto(&meta(1080, 1920)), InsertMode::Fullscreen);
        assert_eq!(InsertMode::auto(&meta(1920, 1080)), InsertMode::Slot);
        assert_eq!(InsertMode::auto(&meta(1000, 1000)), InsertMode::Slot);
        let b = BatchInsert { asset: AssetId::new(), mode: Some(InsertMode::Fullscreen) };
        assert_eq!(b.mode(&meta(1920, 1080)), InsertMode::Fullscreen);
    }

    fn template() -> Template {
        let mut t = Template::minimal();
        t.layers.push(Layer::BannerSlot { rect: Rect::new(0.1, 0.8, 0.8, 0.1) });
        let subs = r##"
            type = "subtitles"
            rect = { x = 0.05, y = 0.6, w = 0.9, h = 0.1 }
            font = "Montserrat ExtraBold"
            size = 0.06
            color = "#ffffff"
        "##;
        t.layers.push(toml::from_str::<Layer>(subs).unwrap());
        t.layers.push(Layer::BannerSlot { rect: Rect::new(0.0, 0.0, 1.0, 0.1) });
        t
    }

    #[test]
    fn slot_plan_hides_subtitles_and_optionally_banner() {
        let t = template();
        let p = t.insert_plan(InsertMode::Slot, &[1], false);
        assert_eq!(p.z, InsertZ::AfterVideo(0));
        assert_eq!(p.dest, (0, 656, 1080, 608));
        assert_eq!(p.hidden, vec![2]);
        let p = t.insert_plan(InsertMode::Slot, &[1], true);
        assert_eq!(p.hidden, vec![1, 2]);
    }

    #[test]
    fn fullscreen_plan_goes_under_visible_banner() {
        let t = template();
        let p = t.insert_plan(InsertMode::Fullscreen, &[1], false);
        assert_eq!(p.z, InsertZ::Before(1));
        assert_eq!(p.dest, (0, 0, 1080, 1920));
        // субтитры и всё выше вставки, кроме баннера
        assert_eq!(p.hidden, vec![2, 3]);
        let p = t.insert_plan(InsertMode::Fullscreen, &[1], true);
        assert_eq!(p.z, InsertZ::Before(4));
        assert_eq!(p.hidden, vec![1, 2]);
        // без видео в шаблоне «в слот» — на весь экран
        let mut t = template();
        t.layers.remove(0);
        assert_eq!(t.insert_plan(InsertMode::Slot, &[], false).mode, InsertMode::Fullscreen);
    }

    #[test]
    fn fit_cover_or_contain() {
        // 16:9 в 16:9 — как есть
        assert_eq!(insert_fit((1920, 1080), 1080, 608), InsertFit { crop: (0, 0, 1918, 1080), out: (0, 0, 1080, 608) });
        // 9:16 на холст 9:16
        assert_eq!(insert_fit((720, 1280), 1080, 1920).out, (0, 0, 1080, 1920));
        // 4:5 на 9:16 — теряется больше пятой части → целиком с полями сверху и снизу
        let f = insert_fit((1080, 1350), 1080, 1920);
        assert_eq!(f.crop, (0, 0, 1080, 1350));
        assert_eq!(f.out, (0, 284, 1080, 1350));
        // 16:9 в 16:10 — cover, срезаются бока
        let f = insert_fit((1920, 1080), 1080, 676);
        assert_eq!(f.out, (0, 0, 1080, 676));
        assert_eq!(f.crop.3, 1080);
        assert!(f.crop.0 > 0 && f.crop.0.is_multiple_of(2) && f.crop.2.is_multiple_of(2));
    }

    #[test]
    fn pauses_and_snap() {
        let words = [(10.0, 10.5), (10.55, 11.0), (11.6, 12.0), (12.05, 12.4), (13.0, 13.5)];
        let p = pauses(words, 10.0, 13.5);
        assert_eq!(p, vec![11.3, 12.7]);
        assert_eq!(snap_to_pause(11.0, &p, 1.0), 11.3);
        assert_eq!(snap_to_pause(12.6, &p, 1.0), 12.7);
        assert_eq!(snap_to_pause(15.0, &p, 1.0), 15.0);
        // у края шортса паузы не считаются
        assert_eq!(pauses(words, 11.2, 13.5), vec![12.7]);
        assert!(insert_inside(11.3, 10.0, 13.5));
        assert!(!insert_inside(10.1, 10.0, 13.5));
    }
}
