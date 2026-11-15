//! Шаблон оформления шортса. Все координаты — в долях холста, длины (радиусы, толщины, кегль) —
//! в долях ширины холста (см. DECISIONS): шаблон одинаково выглядит при любом выходном разрешении.

use std::path::PathBuf;

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
    /// Линейный градиент на весь холст.
    Gradient { from: Color, to: Color, #[serde(default)] angle: f32 },
    /// Картинка «заполнить холст» (лишнее срезается по центру).
    Image {
        path: PathBuf,
        #[serde(default)]
        darken: f32,
    },
    /// Видео «заполнить холст», по кругу, без звука.
    Video {
        path: PathBuf,
        #[serde(default)]
        darken: f32,
    },
}

/// Заливка фигуры.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Paint {
    Solid { color: Color },
    /// `angle` в градусах: 0 — `from` сверху, `to` снизу; 90 — слева направо.
    Linear { from: Color, to: Color, #[serde(default)] angle: f32 },
}

/// Рамка по внутреннему краю слоя (не меняет его размер).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Border {
    pub width: f32,
    pub color: Color,
}

/// Тень под слоем: та же фигура со смещением и размытием.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Shadow {
    pub color: Color,
    /// Радиус размытия (0 — жёсткая тень).
    #[serde(default)]
    pub blur: f32,
    #[serde(default)]
    pub dx: f32,
    #[serde(default)]
    pub dy: f32,
}

/// Обводка текста.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Outline {
    pub width: f32,
    pub color: Color,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Align {
    Left,
    #[default]
    Center,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VAlign {
    Top,
    #[default]
    Center,
    Bottom,
}

/// Как субтитры показывают ход речи.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubtitleMode {
    /// Фраза целиком, без выделения текущего слова.
    Static,
    /// Фраза целиком, текущее слово выделено.
    #[default]
    Highlight,
    /// Сказанные слова — основным цветом, текущее выделено, несказанные — цветом `dim_color`
    /// (прозрачный — скрыты, место под них остаётся).
    Reveal,
}

/// Как выделяется акцент `*…*` в тексте.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccentMode {
    /// Цветом текста.
    #[default]
    Color,
    /// Плашкой цвета акцента под текстом.
    Plate,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Layer {
    /// Кадр основного видео: `crop` — в долях исходника, `dest` — в долях холста.
    MainVideo {
        crop: Rect,
        dest: Rect,
        /// Радиус скругления.
        #[serde(default)]
        corner_radius: f32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        border: Option<Border>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        shadow: Option<Shadow>,
    },
    /// Текст; акцент — `*так*`, подстановки `{part}`, `{title}`.
    Text {
        rect: Rect,
        content: String,
        font: String,
        /// Кегль.
        size: f32,
        color: Color,
        #[serde(default = "default_accent")]
        accent_color: Color,
        #[serde(default)]
        accent_mode: AccentMode,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        outline: Option<Outline>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        shadow: Option<Shadow>,
        #[serde(default)]
        align: Align,
        #[serde(default)]
        valign: VAlign,
    },
    /// Картинка «вписать» в `rect` с сохранением пропорций.
    Image {
        rect: Rect,
        path: PathBuf,
        #[serde(default = "one")]
        opacity: f32,
    },
    /// Капсула: скруглённая плашка с логотипом слева и текстом.
    Capsule {
        rect: Rect,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        logo: Option<PathBuf>,
        text: String,
        font: String,
        text_color: Color,
        fill: Color,
    },
    /// Прямоугольник с заливкой (плашка, затемнение-градиент).
    Shape {
        rect: Rect,
        paint: Paint,
        #[serde(default)]
        corner_radius: f32,
    },
    /// Место под баннер пачки (этап 5).
    BannerSlot { rect: Rect },
    /// Субтитры из транскрипции шортса: фразы по `lines` строк до `words_per_line` слов.
    Subtitles {
        rect: Rect,
        font: String,
        size: f32,
        /// Обычные (сказанные) слова.
        color: Color,
        /// Текущее слово: цвет текста или плашки (`active_mode`).
        #[serde(default = "default_active")]
        active_color: Color,
        #[serde(default)]
        active_mode: AccentMode,
        /// Ключевые слова (из нарезки LLM и отмеченные вручную).
        #[serde(default = "default_accent")]
        keyword_color: Color,
        /// Несказанные слова в режиме `reveal`.
        #[serde(default = "default_dim")]
        dim_color: Color,
        #[serde(default)]
        mode: SubtitleMode,
        #[serde(default = "default_words_per_line")]
        words_per_line: u32,
        #[serde(default = "default_lines")]
        lines: u32,
        #[serde(default)]
        uppercase: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        outline: Option<Outline>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        shadow: Option<Shadow>,
        /// Подложка под каждой строкой.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        plate: Option<Color>,
        #[serde(default)]
        align: Align,
        #[serde(default)]
        valign: VAlign,
    },
}

/// Слов в строке субтитров и строк во фразе — допустимые пределы.
pub const SUBTITLE_WORDS_PER_LINE: std::ops::RangeInclusive<u32> = 1..=10;
pub const SUBTITLE_LINES: std::ops::RangeInclusive<u32> = 1..=4;

fn default_active() -> Color {
    Color::rgb(0xff, 0xd6, 0x0a)
}

fn default_dim() -> Color {
    Color([0xff, 0xff, 0xff, 0x60])
}

fn default_words_per_line() -> u32 {
    3
}

fn default_lines() -> u32 {
    2
}

fn default_accent() -> Color {
    Color::rgb(0xff, 0x3b, 0x30)
}

fn one() -> f32 {
    1.0
}

impl Layer {
    /// Прямоугольник слоя на холсте.
    pub fn rect(&self) -> Rect {
        match self {
            Layer::MainVideo { dest, .. } => *dest,
            Layer::Text { rect, .. }
            | Layer::Image { rect, .. }
            | Layer::Capsule { rect, .. }
            | Layer::Shape { rect, .. }
            | Layer::BannerSlot { rect }
            | Layer::Subtitles { rect, .. } => *rect,
        }
    }
}

/// Где интерфейс YouTube Shorts перекрывает кадр (в долях холста 9:16, приблизительно — по скриншотам
/// приложения на телефоне): сверху — поиск и меню, снизу — канал, подпись и звук, справа — кнопки реакций.
pub const SHORTS_UNSAFE_ZONES: [Rect; 3] = [
    Rect::new(0.0, 0.0, 1.0, 0.12),
    Rect::new(0.0, 0.78, 1.0, 0.22),
    Rect::new(0.86, 0.42, 0.14, 0.36),
];

impl Layer {
    /// Слой с содержимым, которое интерфейс Shorts не должен закрывать (текст, картинки, капсула, баннер),
    /// заходит в [`SHORTS_UNSAFE_ZONES`]. Видео и фигуры-подложки на весь кадр — не в счёт.
    pub fn in_unsafe_zone(&self) -> bool {
        if matches!(self, Layer::MainVideo { .. } | Layer::Shape { .. }) {
            return false;
        }
        let r = self.rect();
        SHORTS_UNSAFE_ZONES.iter().any(|z| r.intersects(z))
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum TemplateError {
    #[error("размер холста {0}×{1} должен быть чётным и не меньше 16")]
    BadCanvas(u32, u32),
    #[error("слой {layer}: прямоугольник «{field}» выходит за 0..1 или пустой")]
    BadRect { layer: usize, field: &'static str },
    #[error("слой {layer}: недопустимое значение «{field}»")]
    BadValue { layer: usize, field: &'static str },
    #[error("фон: значение «{0}» вне допустимого диапазона")]
    BadBackground(&'static str),
    #[error("в шаблоне нет ни одного слоя основного видео")]
    NoMainVideo,
}

/// Участок кадра исходника `src` (ширина, высота в пикселях), который идёт на размытый фон:
/// «cover» на весь холст, приближенный в `scale` раз. Одна формула для превью и рендера.
pub fn background_crop(scale: f32, src: (u32, u32), canvas: Canvas) -> Rect {
    let src_aspect = src.0 as f32 / src.1 as f32;
    let canvas_aspect = canvas.width as f32 / canvas.height as f32;
    Rect::cover(src_aspect, canvas_aspect).zoom_center(scale.max(1.0))
}

/// Какая часть исходника реально попадает в `dest`: `crop` из шаблона, подрезанный по центру
/// под пропорции `dest`, чтобы видео не растягивалось (исходник может быть не 16:9).
pub fn main_video_crop(crop: Rect, src: (u32, u32), dest: Rect, canvas: Canvas) -> Rect {
    let crop_aspect = crop.w * src.0 as f32 / (crop.h * src.1 as f32);
    let dest_aspect = dest.w * canvas.width as f32 / (dest.h * canvas.height as f32);
    crop.sub(Rect::cover(crop_aspect, dest_aspect))
}

/// [`main_video_crop`] в пикселях исходника (чётные) — именно этот участок берут и рендер, и превью.
pub fn main_video_crop_px(crop: Rect, src: (u32, u32), dest: Rect, canvas: Canvas) -> (u32, u32, u32, u32) {
    main_video_crop(crop, src, dest, canvas).to_pixels_even(src.0, src.1)
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
                border: None,
                shadow: None,
            }],
        }
    }

    /// Есть ли в шаблоне слой субтитров.
    pub fn has_subtitles(&self) -> bool {
        self.layers.iter().any(|l| matches!(l, Layer::Subtitles { .. }))
    }

    /// Слои основного видео: (индекс слоя, crop, dest).
    pub fn main_videos(&self) -> impl Iterator<Item = (usize, Rect, Rect)> + '_ {
        self.layers.iter().enumerate().filter_map(|(i, l)| match l {
            Layer::MainVideo { crop, dest, .. } => Some((i, *crop, *dest)),
            _ => None,
        })
    }

    pub fn validate(&self) -> Result<(), TemplateError> {
        let Canvas { width, height } = self.canvas;
        if width < 16 || height < 16 || width % 2 != 0 || height % 2 != 0 {
            return Err(TemplateError::BadCanvas(width, height));
        }
        let unit = |v: f32| (0.0..=1.0).contains(&v);
        let bg = |ok: bool, field| if ok { Ok(()) } else { Err(TemplateError::BadBackground(field)) };
        match &self.background {
            Background::Blur { darken, scale } => {
                bg(unit(*darken), "darken")?;
                bg((1.0..=4.0).contains(scale), "scale")?;
            }
            Background::Color { .. } => {}
            Background::Gradient { angle, .. } => bg(angle.is_finite(), "angle")?,
            Background::Image { path, darken } | Background::Video { path, darken } => {
                bg(!path.as_os_str().is_empty(), "path")?;
                bg(unit(*darken), "darken")?;
            }
        }

        let mut has_main = false;
        for (i, layer) in self.layers.iter().enumerate() {
            let rect = |r: &Rect, field| {
                if r.is_valid_fraction() { Ok(()) } else { Err(TemplateError::BadRect { layer: i, field }) }
            };
            let value = |ok: bool, field| if ok { Ok(()) } else { Err(TemplateError::BadValue { layer: i, field }) };
            let shadow = |s: &Option<Shadow>| match s {
                Some(s) => {
                    value((0.0..=0.2).contains(&s.blur), "shadow.blur")?;
                    value(s.dx.abs() <= 0.2 && s.dy.abs() <= 0.2, "shadow.offset")
                }
                None => Ok(()),
            };
            match layer {
                Layer::MainVideo { crop, dest, corner_radius, border, shadow: sh } => {
                    has_main = true;
                    rect(crop, "crop")?;
                    rect(dest, "dest")?;
                    value((0.0..=0.5).contains(corner_radius), "corner_radius")?;
                    if let Some(b) = border {
                        value(b.width > 0.0 && b.width <= 0.1, "border.width")?;
                    }
                    shadow(sh)?;
                }
                Layer::Text { rect: r, font, size, outline, shadow: sh, .. } => {
                    rect(r, "rect")?;
                    value(!font.is_empty(), "font")?;
                    value((0.005..=0.5).contains(size), "size")?;
                    if let Some(o) = outline {
                        value(o.width > 0.0 && o.width <= 0.05, "outline.width")?;
                    }
                    shadow(sh)?;
                }
                Layer::Image { rect: r, path, opacity } => {
                    rect(r, "rect")?;
                    value(!path.as_os_str().is_empty(), "path")?;
                    value(unit(*opacity), "opacity")?;
                }
                Layer::Capsule { rect: r, font, .. } => {
                    rect(r, "rect")?;
                    value(!font.is_empty(), "font")?;
                }
                Layer::Shape { rect: r, paint, corner_radius } => {
                    rect(r, "rect")?;
                    value((0.0..=0.5).contains(corner_radius), "corner_radius")?;
                    if let Paint::Linear { angle, .. } = paint {
                        value(angle.is_finite(), "paint.angle")?;
                    }
                }
                Layer::BannerSlot { rect: r } => rect(r, "rect")?,
                Layer::Subtitles { rect: r, font, size, words_per_line, lines, outline, shadow: sh, .. } => {
                    rect(r, "rect")?;
                    value(!font.is_empty(), "font")?;
                    value((0.005..=0.5).contains(size), "size")?;
                    value(SUBTITLE_WORDS_PER_LINE.contains(words_per_line), "words_per_line")?;
                    value(SUBTITLE_LINES.contains(lines), "lines")?;
                    if let Some(o) = outline {
                        value(o.width > 0.0 && o.width <= 0.05, "outline.width")?;
                    }
                    shadow(sh)?;
                }
            }
        }
        if !has_main {
            return Err(TemplateError::NoMainVideo);
        }
        Ok(())
    }

    /// Шаблон для конкретного шортса: `{part}` и `{title}` в текстах заменены значениями.
    pub fn with_vars(&self, vars: &TextVars) -> Self {
        let mut t = self.clone();
        for layer in &mut t.layers {
            match layer {
                Layer::Text { content: s, .. } | Layer::Capsule { text: s, .. } => *s = vars.apply(s),
                _ => {}
            }
        }
        t
    }

    pub fn from_toml(s: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(s)
    }

    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).expect("шаблон всегда сериализуется в TOML")
    }
}

/// Значения подстановок в текстах шаблона.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TextVars {
    /// `{part}` — номер шортса среди принятых своего исходника (с 1).
    pub part: u32,
    /// `{title}` — заголовок шортса.
    pub title: String,
}

impl TextVars {
    pub fn apply(&self, s: &str) -> String {
        s.replace("{part}", &self.part.to_string()).replace("{title}", &self.title)
    }
}

/// Линейный градиент: доля `to` (0..1) в точке `(x, y)` прямоугольника `w`×`h` (пиксели от его угла).
/// Угол 0 — сверху вниз, 90 — слева направо; градиент проходит от угла до угла вдоль направления,
/// как в CSS `linear-gradient`. Одна формула для превью и рендера.
pub fn gradient_t(angle_deg: f32, w: f32, h: f32, x: f32, y: f32) -> f32 {
    let a = angle_deg.to_radians();
    // Направление: 0° — вниз (+y), 90° — вправо (+x).
    let (dx, dy) = (a.sin(), a.cos());
    let half = (w * dx.abs() + h * dy.abs()) / 2.0;
    if half <= 0.0 {
        return 0.0;
    }
    let p = (x - w / 2.0) * dx + (y - h / 2.0) * dy;
    (p / (2.0 * half) + 0.5).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_is_valid() {
        Template::minimal().validate().unwrap();
    }

    #[test]
    fn crops_keep_aspect() {
        let canvas = Canvas::default();
        let (_, crop, dest) = Template::minimal().main_videos().next().unwrap();
        // 16:9 в минимальном шаблоне — без подрезки
        let c = main_video_crop(crop, (1920, 1080), dest, canvas);
        assert!((c.w - 1.0).abs() < 1e-3 && (c.h - 1.0).abs() < 1e-3, "{c:?}");
        // 4:3 — срезаются верх и низ
        let c = main_video_crop(crop, (1440, 1080), dest, canvas);
        assert!((c.w - 1.0).abs() < 1e-3 && (c.h - 0.75).abs() < 1e-3, "{c:?}");
        // фон: полоса 9:16 из 16:9, при scale = 2 — вдвое меньше
        let b1 = background_crop(1.0, (1920, 1080), canvas);
        let b2 = background_crop(2.0, (1920, 1080), canvas);
        assert!((b1.h - 1.0).abs() < 1e-5 && (b2.w * 2.0 - b1.w).abs() < 1e-5);
    }

    #[test]
    fn toml_roundtrip() {
        let mut t = Template::minimal();
        t.layers.push(Layer::Shape {
            rect: Rect::new(0.0, 0.6, 1.0, 0.4),
            paint: Paint::Linear { from: Color([0, 0, 0, 0]), to: Color::BLACK, angle: 0.0 },
            corner_radius: 0.0,
        });
        t.layers.push(Layer::Text {
            rect: Rect::new(0.05, 0.1, 0.9, 0.1),
            content: "ТОП *языков*".into(),
            font: "Press Start 2P".into(),
            size: 0.05,
            color: Color::WHITE,
            accent_color: default_accent(),
            accent_mode: AccentMode::Plate,
            outline: Some(Outline { width: 0.004, color: Color::BLACK }),
            shadow: None,
            align: Align::Center,
            valign: VAlign::Bottom,
        });
        let s = t.to_toml();
        assert!(s.contains("type = \"blur\""), "{s}");
        assert!(s.contains("type = \"main_video\""), "{s}");
        assert!(!s.contains("border"), "пустые Option не пишутся: {s}");
        assert_eq!(Template::from_toml(&s).unwrap(), t);
        t.validate().unwrap();
    }

    #[test]
    fn handwritten_toml() {
        let t = Template::from_toml(
            r##"
            name = "цвет"
            background = { type = "gradient", from = "#101820", to = "#000000" }

            [[layers]]
            type = "main_video"
            crop = { x = 0.25, y = 0.0, w = 0.5, h = 1.0 }
            dest = { x = 0.0, y = 0.0, w = 1.0, h = 1.0 }
            border = { width = 0.003, color = "#33ff66" }
            shadow = { color = "#00000080", blur = 0.02, dy = 0.01 }

            [[layers]]
            type = "image"
            rect = { x = 0.4, y = 0.02, w = 0.2, h = 0.1 }
            path = "/tmp/logo.png"
            "##,
        )
        .unwrap();
        assert_eq!(t.canvas, Canvas::default());
        assert_eq!(t.background, Background::Gradient { from: Color::rgb(0x10, 0x18, 0x20), to: Color::BLACK, angle: 0.0 });
        let Layer::MainVideo { shadow: Some(s), .. } = &t.layers[0] else { panic!() };
        assert_eq!((s.blur, s.dx, s.dy), (0.02, 0.0, 0.01));
        assert_eq!(t.layers[1], Layer::Image { rect: Rect::new(0.4, 0.02, 0.2, 0.1), path: "/tmp/logo.png".into(), opacity: 1.0 });
        t.validate().unwrap();
    }

    #[test]
    fn validation_errors() {
        let mut t = Template::minimal();
        t.canvas.width = 1081;
        assert_eq!(t.validate(), Err(TemplateError::BadCanvas(1081, 1920)));

        let mut t = Template::minimal();
        let Layer::MainVideo { dest, .. } = &mut t.layers[0] else { panic!() };
        dest.y = 0.9;
        assert_eq!(t.validate(), Err(TemplateError::BadRect { layer: 0, field: "dest" }));

        let mut t = Template::minimal();
        let Layer::MainVideo { border, .. } = &mut t.layers[0] else { panic!() };
        *border = Some(Border { width: 0.0, color: Color::WHITE });
        assert_eq!(t.validate(), Err(TemplateError::BadValue { layer: 0, field: "border.width" }));

        let mut t = Template::minimal();
        t.background = Background::Image { path: PathBuf::new(), darken: 0.0 };
        assert_eq!(t.validate(), Err(TemplateError::BadBackground("path")));

        let mut t = Template::minimal();
        t.layers = vec![Layer::BannerSlot { rect: Rect::FULL }];
        assert_eq!(t.validate(), Err(TemplateError::NoMainVideo));
    }

    #[test]
    fn vars_substituted_in_text_and_capsule() {
        let mut t = Template::minimal();
        t.layers.push(Layer::Capsule {
            rect: Rect::new(0.1, 0.1, 0.5, 0.05),
            logo: None,
            text: "Часть {part}".into(),
            font: "Sans".into(),
            text_color: Color::WHITE,
            fill: Color::BLACK,
        });
        let text = |content: &str| Layer::Text {
            rect: Rect::new(0.0, 0.0, 1.0, 0.1),
            content: content.into(),
            font: "Sans".into(),
            size: 0.05,
            color: Color::WHITE,
            accent_color: default_accent(),
            accent_mode: AccentMode::Color,
            outline: None,
            shadow: None,
            align: Align::Center,
            valign: VAlign::Center,
        };
        t.layers.push(text("{title} — {part}/{x}"));
        let r = t.with_vars(&TextVars { part: 3, title: "Про *bash*".into() });
        assert!(matches!(&r.layers[1], Layer::Capsule { text, .. } if text == "Часть 3"));
        assert_eq!(r.layers[2], text("Про *bash* — 3/{x}"));
        assert_eq!(r.layers[0], t.layers[0]);
    }

    #[test]
    fn unsafe_zones() {
        let capsule = |y: f32| Layer::Capsule {
            rect: Rect::new(0.25, y, 0.5, 0.05),
            logo: None,
            text: String::new(),
            font: "Sans".into(),
            text_color: Color::WHITE,
            fill: Color::BLACK,
        };
        assert!(capsule(0.05).in_unsafe_zone());
        assert!(!capsule(0.2).in_unsafe_zone());
        assert!(capsule(0.75).in_unsafe_zone(), "задевает нижнюю зону");
        // видео на весь кадр — не предупреждение
        assert!(!Layer::MainVideo { crop: Rect::FULL, dest: Rect::FULL, corner_radius: 0.0, border: None, shadow: None }.in_unsafe_zone());
        // кнопки справа
        assert!(Layer::BannerSlot { rect: Rect::new(0.7, 0.5, 0.2, 0.1) }.in_unsafe_zone());
        assert!(!Layer::BannerSlot { rect: Rect::new(0.05, 0.5, 0.8, 0.1) }.in_unsafe_zone());
    }

    #[test]
    fn subtitles_layer_defaults_and_limits() {
        let mut t = Template::minimal();
        t.layers.push(
            toml::from_str::<Layer>(
                r##"
                type = "subtitles"
                rect = { x = 0.05, y = 0.6, w = 0.8, h = 0.12 }
                font = "Montserrat ExtraBold"
                size = 0.06
                color = "#ffffff"
                mode = "reveal"
                "##,
            )
            .unwrap(),
        );
        assert!(t.has_subtitles());
        let Layer::Subtitles { words_per_line, lines, active_mode, dim_color, .. } = &t.layers[1] else { panic!() };
        assert_eq!((*words_per_line, *lines, *active_mode), (3, 2, AccentMode::Color));
        assert_eq!(dim_color.0[3], 0x60);
        t.validate().unwrap();
        assert_eq!(Template::from_toml(&t.to_toml()).unwrap(), t);
        assert!(!t.layers[1].in_unsafe_zone());
        let Layer::Subtitles { words_per_line, .. } = &mut t.layers[1] else { panic!() };
        *words_per_line = 0;
        assert_eq!(t.validate(), Err(TemplateError::BadValue { layer: 1, field: "words_per_line" }));
    }

    #[test]
    fn gradient_directions() {
        // 0° — сверху вниз
        assert_eq!(gradient_t(0.0, 100.0, 200.0, 50.0, 0.0), 0.0);
        assert_eq!(gradient_t(0.0, 100.0, 200.0, 50.0, 200.0), 1.0);
        assert!((gradient_t(0.0, 100.0, 200.0, 0.0, 100.0) - 0.5).abs() < 1e-6);
        // 90° — слева направо
        assert!(gradient_t(90.0, 100.0, 200.0, 0.0, 100.0) < 1e-6);
        assert!((gradient_t(90.0, 100.0, 200.0, 100.0, 0.0) - 1.0).abs() < 1e-6);
        // 45° — от угла до угла
        assert!(gradient_t(45.0, 100.0, 100.0, 0.0, 0.0) < 1e-6);
        assert!((gradient_t(45.0, 100.0, 100.0, 100.0, 100.0) - 1.0).abs() < 1e-6);
    }
}
