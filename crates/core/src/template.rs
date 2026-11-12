//! Шаблон оформления шортса. Все координаты — в долях холста (см. DECISIONS).

use serde::{Deserialize, Serialize};

use crate::geometry::{Color, Rect};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Template {
    pub name: String,
    #[serde(default)]
    pub canvas: Canvas,
    pub background: Background,
    /// Слои снизу вверх (z-order).
    #[serde(default)]
    pub layers: Vec<Layer>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Canvas {
    pub width: u32,
    pub height: u32,
}

impl Default for Canvas {
    fn default() -> Self {
        Self { width: 1080, height: 1920 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Background {
    /// Размытое основное видео на весь холст.
    Blur {
        /// Затемнение 0..1 (0 — без затемнения).
        darken: f32,
        /// Увеличение поверх «заполнить холст» (1 — ровно заполнить).
        scale: f32,
    },
    Color { color: Color },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Layer {
    /// Кадр основного видео: `crop` — в долях исходника, `dest` — в долях холста.
    MainVideo {
        crop: Rect,
        dest: Rect,
        /// Радиус скругления в долях ширины холста.
        #[serde(default)]
        corner_radius: f32,
    },
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum TemplateError {
    #[error("размер холста {0}×{1} должен быть чётным и не меньше 16")]
    BadCanvas(u32, u32),
    #[error("слой {layer}: прямоугольник «{field}» выходит за 0..1 или пустой")]
    BadRect { layer: usize, field: &'static str },
    #[error("слой {0}: недопустимый радиус скругления")]
    BadRadius(usize),
    #[error("фон: значение вне допустимого диапазона")]
    BadBackground,
    #[error("в шаблоне нет ни одного слоя основного видео")]
    NoMainVideo,
}

impl Template {
    /// Минимальный шаблон этапа 1: размытый фон и исходник 16:9 по центру во всю ширину.
    pub fn minimal() -> Self {
        let canvas = Canvas::default();
        let h = (canvas.width as f32 * 9.0 / 16.0) / canvas.height as f32;
        Self {
            name: "minimal".into(),
            canvas,
            background: Background::Blur { darken: 0.3, scale: 1.0 },
            layers: vec![Layer::MainVideo {
                crop: Rect::FULL,
                dest: Rect::new(0.0, (1.0 - h) / 2.0, 1.0, h),
                corner_radius: 0.0,
            }],
        }
    }

    pub fn validate(&self) -> Result<(), TemplateError> {
        let Canvas { width, height } = self.canvas;
        if width < 16 || height < 16 || width % 2 != 0 || height % 2 != 0 {
            return Err(TemplateError::BadCanvas(width, height));
        }
        match self.background {
            Background::Blur { darken, scale } => {
                if !(0.0..=1.0).contains(&darken) || !(1.0..=4.0).contains(&scale) {
                    return Err(TemplateError::BadBackground);
                }
            }
            Background::Color { .. } => {}
        }
        let mut has_main = false;
        for (i, layer) in self.layers.iter().enumerate() {
            match layer {
                Layer::MainVideo { crop, dest, corner_radius } => {
                    has_main = true;
                    if !crop.is_valid_fraction() {
                        return Err(TemplateError::BadRect { layer: i, field: "crop" });
                    }
                    if !dest.is_valid_fraction() {
                        return Err(TemplateError::BadRect { layer: i, field: "dest" });
                    }
                    if !(0.0..=0.5).contains(corner_radius) {
                        return Err(TemplateError::BadRadius(i));
                    }
                }
            }
        }
        if !has_main {
            return Err(TemplateError::NoMainVideo);
        }
        Ok(())
    }

    pub fn from_toml(s: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(s)
    }

    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).expect("шаблон всегда сериализуется в TOML")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_is_valid() {
        Template::minimal().validate().unwrap();
    }

    #[test]
    fn toml_roundtrip() {
        let t = Template::minimal();
        let s = t.to_toml();
        assert!(s.contains("type = \"blur\""), "{s}");
        assert!(s.contains("type = \"main_video\""), "{s}");
        assert_eq!(Template::from_toml(&s).unwrap(), t);
    }

    #[test]
    fn handwritten_toml() {
        let t = Template::from_toml(
            r##"
            name = "цвет"
            background = { type = "color", color = "#101820" }

            [[layers]]
            type = "main_video"
            crop = { x = 0.25, y = 0.0, w = 0.5, h = 1.0 }
            dest = { x = 0.0, y = 0.0, w = 1.0, h = 1.0 }
            "##,
        )
        .unwrap();
        assert_eq!(t.canvas, Canvas::default());
        assert_eq!(t.background, Background::Color { color: Color::rgb(0x10, 0x18, 0x20) });
        t.validate().unwrap();
    }

    #[test]
    fn validation_errors() {
        let mut t = Template::minimal();
        t.canvas.width = 1081;
        assert_eq!(t.validate(), Err(TemplateError::BadCanvas(1081, 1920)));

        let mut t = Template::minimal();
        let Layer::MainVideo { dest, .. } = &mut t.layers[0];
        dest.y = 0.9;
        assert_eq!(t.validate(), Err(TemplateError::BadRect { layer: 0, field: "dest" }));

        let mut t = Template::minimal();
        t.layers.clear();
        assert_eq!(t.validate(), Err(TemplateError::NoMainVideo));
    }
}
