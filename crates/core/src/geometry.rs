//! Прямоугольники в долях (0..1) и цвета `#RRGGBB[AA]`.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Прямоугольник в долях холста или исходника: (0, 0) — левый верхний угол, 1 — вся сторона.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const FULL: Rect = Rect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 };

    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    /// Ненулевой размер и целиком внутри 0..1 (с допуском на погрешность float).
    pub fn is_valid_fraction(&self) -> bool {
        const EPS: f32 = 1e-4;
        let finite = [self.x, self.y, self.w, self.h].iter().all(|v| v.is_finite());
        finite
            && self.w > 0.0
            && self.h > 0.0
            && self.x >= -EPS
            && self.y >= -EPS
            && self.x + self.w <= 1.0 + EPS
            && self.y + self.h <= 1.0 + EPS
    }

    /// Участок источника с соотношением сторон `src_aspect`, который заполняет область с `dst_aspect`
    /// без искажений («cover»): по центру, лишние края обрезаются.
    pub fn cover(src_aspect: f32, dst_aspect: f32) -> Rect {
        if src_aspect > dst_aspect {
            let w = dst_aspect / src_aspect;
            Rect::new((1.0 - w) / 2.0, 0.0, w, 1.0)
        } else {
            let h = src_aspect / dst_aspect;
            Rect::new(0.0, (1.0 - h) / 2.0, 1.0, h)
        }
    }

    /// `inner` задан в долях этого прямоугольника → в тех же координатах, что и он сам.
    pub fn sub(&self, inner: Rect) -> Rect {
        Rect::new(self.x + inner.x * self.w, self.y + inner.y * self.h, inner.w * self.w, inner.h * self.h)
    }

    /// Уменьшенный в `k` раз относительно центра (`k` ≥ 1 — «приблизить»).
    pub fn zoom_center(&self, k: f32) -> Rect {
        let (w, h) = (self.w / k, self.h / k);
        Rect::new(self.x + (self.w - w) / 2.0, self.y + (self.h - h) / 2.0, w, h)
    }

    /// В пикселях для области `width`×`height`, округлено до чётных (требование yuv420).
    pub fn to_pixels_even(&self, width: u32, height: u32) -> (u32, u32, u32, u32) {
        let even = |v: f32| ((v / 2.0).round() as u32) * 2;
        let x = even(self.x * width as f32);
        let y = even(self.y * height as f32);
        let w = even(self.w * width as f32).clamp(2, width - x);
        let h = even(self.h * height as f32).clamp(2, height - y);
        (x, y, w, h)
    }
}

/// Цвет RGBA. В файлах — строка `#RRGGBB` или `#RRGGBBAA`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color(pub [u8; 4]);

impl Color {
    pub const BLACK: Color = Color([0, 0, 0, 255]);
    pub const WHITE: Color = Color([255, 255, 255, 255]);

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self([r, g, b, 255])
    }
}

#[derive(Debug, thiserror::Error)]
#[error("неверный цвет «{0}»: ожидается #RRGGBB или #RRGGBBAA")]
pub struct ColorParseError(String);

impl FromStr for Color {
    type Err = ColorParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = || ColorParseError(s.to_owned());
        let hex = s.strip_prefix('#').ok_or_else(err)?;
        if !(hex.len() == 6 || hex.len() == 8) || !hex.is_ascii() {
            return Err(err());
        }
        let mut c = [0, 0, 0, 255];
        for (i, slot) in c.iter_mut().enumerate().take(hex.len() / 2) {
            *slot = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).map_err(|_| err())?;
        }
        Ok(Color(c))
    }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [r, g, b, a] = self.0;
        if a == 255 {
            write!(f, "#{r:02x}{g:02x}{b:02x}")
        } else {
            write!(f, "#{r:02x}{g:02x}{b:02x}{a:02x}")
        }
    }
}

impl Serialize for Color {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_roundtrip() {
        assert_eq!("#ff8000".parse::<Color>().unwrap(), Color([255, 128, 0, 255]));
        assert_eq!("#ff800080".parse::<Color>().unwrap(), Color([255, 128, 0, 128]));
        assert_eq!(Color([1, 2, 3, 255]).to_string(), "#010203");
        assert_eq!(Color([1, 2, 3, 4]).to_string(), "#01020304");
        for bad in ["ff8000", "#ff80", "#gg0000", "#ффф"] {
            assert!(bad.parse::<Color>().is_err(), "{bad}");
        }
    }

    #[test]
    fn rect_validity_and_pixels() {
        assert!(Rect::FULL.is_valid_fraction());
        assert!(!Rect::new(0.5, 0.0, 0.6, 1.0).is_valid_fraction());
        assert!(!Rect::new(0.0, 0.0, 0.0, 1.0).is_valid_fraction());
        assert!(!Rect::new(f32::NAN, 0.0, 1.0, 1.0).is_valid_fraction());
        // 16:9 по центру холста 1080×1920
        let r = Rect::new(0.0, 0.341_797, 1.0, 0.316_406);
        assert_eq!(r.to_pixels_even(1080, 1920), (0, 656, 1080, 608));
    }

    fn close(a: Rect, b: Rect) -> bool {
        [a.x - b.x, a.y - b.y, a.w - b.w, a.h - b.h].iter().all(|d| d.abs() < 1e-5)
    }

    #[test]
    fn cover_sub_zoom() {
        // 16:9 на 9:16 — узкая полоса по центру
        let c = Rect::cover(16.0 / 9.0, 9.0 / 16.0);
        assert!(close(c, Rect::new(0.341_796_9, 0.0, 0.316_406_2, 1.0)), "{c:?}");
        // 4:3 на 16:9 — срезаются верх и низ
        assert!(close(Rect::cover(4.0 / 3.0, 16.0 / 9.0), Rect::new(0.0, 0.125, 1.0, 0.75)));
        assert!(close(Rect::cover(1.0, 1.0), Rect::FULL));
        let half = Rect::new(0.5, 0.0, 0.5, 1.0);
        assert!(close(half.sub(Rect::new(0.5, 0.5, 0.5, 0.5)), Rect::new(0.75, 0.5, 0.25, 0.5)));
        assert!(close(Rect::FULL.zoom_center(2.0), Rect::new(0.25, 0.25, 0.5, 0.5)));
    }
}
