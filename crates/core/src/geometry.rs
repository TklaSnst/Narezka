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
}
