//! Баннер пачки (SPEC §4): картинка, GIF или видео с альфой поверх шортса, по месту `BannerSlot` шаблона,
//! и необязательный текст рядом (промокод, ссылка).
//!
//! Превью и рендер получают баннер уже внутри шаблона ([`Template::with_banner`]): место под баннер
//! заменяется слоем `Image` (неподвижная картинка) или `Clip` (анимация по кругу), текст — слоем `Text`
//! в конце списка. Номера остальных слоёв не меняются — растры, субтитры и редактор их не замечают.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::asset::{Asset, AssetId};
use crate::geometry::{Color, Rect};
use crate::template::{AccentMode, Align, Layer, Outline, Template, VAlign};

/// Где баннер, если в шаблоне нет места под него (то же, что у нового слоя «Место под баннер»).
pub const DEFAULT_BANNER_RECT: Rect = Rect::new(0.1, 0.62, 0.8, 0.1);

/// Баннер пачки.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Banner {
    /// Ассет библиотеки (картинка, GIF, видео с альфой).
    pub asset: AssetId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<BannerText>,
}

/// С какой стороны от баннера текст.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BannerSide {
    #[default]
    Below,
    Above,
    Left,
    Right,
}

impl BannerSide {
    pub const ALL: [BannerSide; 4] = [BannerSide::Below, BannerSide::Above, BannerSide::Left, BannerSide::Right];

    pub fn key(self) -> &'static str {
        match self {
            BannerSide::Below => "banner-side-below",
            BannerSide::Above => "banner-side-above",
            BannerSide::Left => "banner-side-left",
            BannerSide::Right => "banner-side-right",
        }
    }
}

/// Текст рядом с баннером; двигается вместе с ним.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BannerText {
    /// Акцент — `*так*`, как у текстовых слоёв.
    pub content: String,
    #[serde(default)]
    pub side: BannerSide,
    pub font: String,
    /// Кегль в долях ширины холста.
    pub size: f32,
    pub color: Color,
    #[serde(default = "default_accent")]
    pub accent_color: Color,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outline: Option<Outline>,
}

fn default_accent() -> Color {
    Color::rgb(0xff, 0xd6, 0x0a)
}

impl Default for BannerText {
    fn default() -> Self {
        Self {
            content: String::new(),
            side: BannerSide::Below,
            font: "Montserrat ExtraBold".into(),
            size: 0.04,
            color: Color::WHITE,
            accent_color: default_accent(),
            outline: Some(Outline { width: 0.004, color: Color::BLACK }),
        }
    }
}

/// Отступ текста от баннера, доля ширины холста.
const TEXT_GAP: f32 = 0.012;
/// Высота строки относительно кегля.
const LINE: f32 = 1.35;
/// Поле у края холста для текста сбоку, доля ширины.
const SIDE_MARGIN: f32 = 0.03;

impl BannerText {
    /// Прямоугольник текста рядом с баннером в `slot` (холст с соотношением сторон `aspect` = ширина / высота).
    /// Не вылезает за холст; места нет совсем — `None`.
    pub fn rect(&self, slot: Rect, aspect: f32) -> Option<Rect> {
        // Длины в долях ширины → в доли высоты.
        let gap_y = TEXT_GAP * aspect;
        let line_h = self.size * LINE * aspect;
        let r = match self.side {
            BannerSide::Below => Rect::new(slot.x, slot.bottom() + gap_y, slot.w, line_h),
            BannerSide::Above => Rect::new(slot.x, slot.y - gap_y - line_h, slot.w, line_h),
            BannerSide::Left => {
                let x0 = SIDE_MARGIN;
                Rect::new(x0, slot.y, slot.x - TEXT_GAP - x0, slot.h)
            }
            BannerSide::Right => {
                let x0 = slot.right() + TEXT_GAP;
                Rect::new(x0, slot.y, 1.0 - SIDE_MARGIN - x0, slot.h)
            }
        };
        clamp_rect(r)
    }

    fn layer(&self, rect: Rect) -> Layer {
        let align = match self.side {
            BannerSide::Below | BannerSide::Above => Align::Center,
            BannerSide::Left => Align::Right,
            BannerSide::Right => Align::Left,
        };
        Layer::Text {
            rect,
            content: self.content.clone(),
            font: self.font.clone(),
            size: self.size,
            color: self.color,
            accent_color: self.accent_color,
            accent_mode: AccentMode::Color,
            outline: self.outline,
            shadow: None,
            align,
            valign: VAlign::Center,
        }
    }
}

/// Часть прямоугольника внутри холста; слишком мало — `None`.
fn clamp_rect(r: Rect) -> Option<Rect> {
    let (x0, y0) = (r.x.max(0.0), r.y.max(0.0));
    let (x1, y1) = (r.right().min(1.0), r.bottom().min(1.0));
    (x1 - x0 > 0.01 && y1 - y0 > 0.005).then(|| Rect::new(x0, y0, x1 - x0, y1 - y0))
}

/// Баннер, готовый к показу: файл ассета и как его рисовать.
#[derive(Debug, Clone, PartialEq)]
pub struct BannerArt {
    pub path: PathBuf,
    /// Размер кадра (для вписывания анимации; у картинки растеризатор знает его сам).
    pub size: (u32, u32),
    /// Анимация (GIF, видео) — по кругу; иначе неподвижная картинка.
    pub animated: bool,
    /// Декодер ffmpeg, который сохраняет прозрачность (VP8/VP9 с альфой — только libvpx).
    pub decoder: Option<String>,
    pub text: Option<BannerText>,
}

impl BannerArt {
    /// Баннер `banner` из ассета `asset` с файлом `path` в библиотеке.
    pub fn new(banner: &Banner, asset: &Asset, path: PathBuf) -> Self {
        Self {
            path,
            size: (asset.meta.width, asset.meta.height),
            // Неподвижные картинки рисует растеризатор (PNG, JPEG, WebP); остальное (GIF из одного кадра, BMP…) —
            // через ffmpeg, как анимацию из одного кадра.
            animated: asset.meta.is_animated() || !matches!(asset.meta.codec.as_str(), "png" | "mjpeg" | "webp"),
            decoder: alpha_decoder(&asset.meta.codec, asset.meta.alpha).map(str::to_owned),
            text: banner.text.clone().filter(|t| !t.content.trim().is_empty()),
        }
    }
}

/// Декодер с поддержкой альфы: встроенные декодеры ffmpeg VP8/VP9 прозрачность теряют.
pub fn alpha_decoder(codec: &str, alpha: bool) -> Option<&'static str> {
    match codec {
        "vp9" if alpha => Some("libvpx-vp9"),
        "vp8" if alpha => Some("libvpx"),
        _ => None,
    }
}

impl Template {
    /// Номер слоя-места под баннер (первого).
    pub fn banner_slot(&self) -> Option<usize> {
        self.layers.iter().position(|l| matches!(l, Layer::BannerSlot { .. }))
    }

    /// Шаблон с баннером: первое место под баннер становится картинкой/анимацией (нет места — баннер
    /// добавляется в конец в [`DEFAULT_BANNER_RECT`]), текст рядом — слоем в конце. `None` — без изменений.
    pub fn with_banner(&self, banner: Option<&BannerArt>) -> Template {
        let mut t = self.clone();
        let Some(b) = banner else { return t };
        let layer = |rect: Rect| {
            if b.animated {
                Layer::Clip { rect, path: b.path.clone(), width: b.size.0, height: b.size.1, decoder: b.decoder.clone() }
            } else {
                Layer::Image { rect, path: b.path.clone(), opacity: 1.0 }
            }
        };
        let slot = match t.banner_slot() {
            Some(i) => {
                let rect = t.layers[i].rect();
                t.layers[i] = layer(rect);
                rect
            }
            None => {
                t.layers.push(layer(DEFAULT_BANNER_RECT));
                DEFAULT_BANNER_RECT
            }
        };
        if let Some(text) = &b.text {
            let aspect = t.canvas.width as f32 / t.canvas.height as f32;
            // Текст — рядом с самой картинкой, а не с рамкой места.
            let fitted = fit_rect(slot, b.size, aspect);
            if let Some(r) = text.rect(fitted, aspect) {
                t.layers.push(text.layer(r));
            }
        }
        t
    }
}

/// Картинка `size` (пиксели), вписанная в `rect` (доли холста с соотношением сторон `aspect`) по центру.
pub fn fit_rect(rect: Rect, size: (u32, u32), aspect: f32) -> Rect {
    if size.0 == 0 || size.1 == 0 {
        return rect;
    }
    // Соотношение сторон картинки и рамки в одних единицах (доли высоты → доли ширины).
    let img = size.0 as f32 / size.1 as f32;
    let frame = rect.w * aspect / rect.h;
    if img > frame {
        let h = rect.h * frame / img;
        Rect::new(rect.x, rect.y + (rect.h - h) / 2.0, rect.w, h)
    } else {
        let w = rect.w * img / frame;
        Rect::new(rect.x + (rect.w - w) / 2.0, rect.y, w, rect.h)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asset::AssetMeta;

    fn art(animated: bool, text: Option<BannerText>) -> BannerArt {
        BannerArt { path: "/lib/b.png".into(), size: (800, 200), animated, decoder: None, text }
    }

    fn close(a: Rect, b: Rect) -> bool {
        [a.x - b.x, a.y - b.y, a.w - b.w, a.h - b.h].iter().all(|d| d.abs() < 1e-4)
    }

    #[test]
    fn banner_replaces_first_slot_and_keeps_indices() {
        let mut t = Template::minimal();
        let slot = Rect::new(0.1, 0.7, 0.8, 0.1);
        t.layers.push(Layer::BannerSlot { rect: slot });
        t.layers.push(Layer::BannerSlot { rect: Rect::new(0.0, 0.0, 0.5, 0.1) });
        let n = t.layers.len();

        assert_eq!(t.with_banner(None), t);
        let b = t.with_banner(Some(&art(false, None)));
        assert_eq!(b.layers.len(), n);
        assert_eq!(b.layers[1], Layer::Image { rect: slot, path: "/lib/b.png".into(), opacity: 1.0 });
        assert!(matches!(b.layers[2], Layer::BannerSlot { .. }));
        b.validate().unwrap();

        let c = t.with_banner(Some(&art(true, None)));
        assert!(matches!(&c.layers[1], Layer::Clip { width: 800, height: 200, .. }));
        c.validate().unwrap();
    }

    #[test]
    fn no_slot_goes_to_default_place_and_text_follows_banner() {
        let t = Template::minimal();
        let text = BannerText { content: "Промокод *SHORTS*".into(), ..Default::default() };
        let b = t.with_banner(Some(&art(false, Some(text.clone()))));
        assert_eq!(b.layers.len(), t.layers.len() + 2);
        assert_eq!(b.layers[1].rect(), DEFAULT_BANNER_RECT);
        let Layer::Text { rect, content, align, .. } = &b.layers[2] else { panic!("{:?}", b.layers[2]) };
        assert_eq!((content.as_str(), *align), ("Промокод *SHORTS*", Align::Center));
        // 800×200 в рамке 0.8 × 0.1 холста 9:16 → по высоте 0.8·(9/16)/4 = 0.1125 > 0.1: упирается в высоту
        let fitted = fit_rect(DEFAULT_BANNER_RECT, (800, 200), 9.0 / 16.0);
        assert!(close(fitted, Rect::new(0.1 + (0.8 - 0.1 * 4.0 * 16.0 / 9.0) / 2.0, 0.62, 0.1 * 4.0 * 16.0 / 9.0, 0.1)), "{fitted:?}");
        assert!((rect.y - (fitted.bottom() + TEXT_GAP * 9.0 / 16.0)).abs() < 1e-5);
        b.validate().unwrap();
    }

    #[test]
    fn text_rect_sides_stay_on_canvas() {
        let a = 9.0 / 16.0;
        let t = |side| BannerText { content: "x".into(), side, ..Default::default() };
        let slot = Rect::new(0.3, 0.01, 0.4, 0.1);
        // сверху места почти нет — обрезается до края, а совсем нет — None
        assert!(t(BannerSide::Above).rect(slot, a).is_none());
        let r = t(BannerSide::Above).rect(Rect::new(0.3, 0.02, 0.4, 0.1), a).unwrap();
        assert!(r.y == 0.0 && r.is_valid_fraction(), "{r:?}");
        let l = t(BannerSide::Left).rect(slot, a).unwrap();
        assert!(close(l, Rect::new(SIDE_MARGIN, 0.01, 0.3 - TEXT_GAP - SIDE_MARGIN, 0.1)), "{l:?}");
        let r = t(BannerSide::Right).rect(slot, a).unwrap();
        assert!((r.right() - (1.0 - SIDE_MARGIN)).abs() < 1e-5 && r.is_valid_fraction());
        assert!(t(BannerSide::Right).rect(Rect::new(0.5, 0.5, 0.5, 0.1), a).is_none());
    }

    #[test]
    fn alpha_decoders() {
        assert_eq!(alpha_decoder("vp9", true), Some("libvpx-vp9"));
        assert_eq!(alpha_decoder("vp9", false), None);
        assert_eq!(alpha_decoder("prores", true), None);
        let asset = Asset {
            id: AssetId::new(),
            name: "b".into(),
            kind: crate::asset::AssetKind::Banner,
            file: "b.webm".into(),
            hash: "h".into(),
            bytes: 1,
            added: 0,
            tags: vec![],
            meta: AssetMeta { width: 600, height: 200, duration: Some(2.0), alpha: true, codec: "vp9".into(), ..Default::default() },
            hide_banner: false,
        };
        let banner = Banner { asset: asset.id, text: Some(BannerText { content: "  ".into(), ..Default::default() }) };
        let a = BannerArt::new(&banner, &asset, "/l/b.webm".into());
        assert!(a.animated && a.decoder.as_deref() == Some("libvpx-vp9") && a.text.is_none());
        let still = |codec: &str| {
            let meta = AssetMeta { width: 10, height: 10, codec: codec.into(), ..Default::default() };
            BannerArt::new(&banner, &Asset { meta, ..asset.clone() }, "/l/x".into()).animated
        };
        assert!(!still("png") && !still("mjpeg") && !still("webp"));
        assert!(still("gif") && still("bmp"), "растеризатор их не читает — через ffmpeg");
    }
}
