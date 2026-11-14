//! Общий растеризатор превью и рендера: всё, что не зависит от кадра видео, рисуется здесь в RGBA —
//! фон-градиент и фон-картинка, тени и рамки слоёв видео, фигуры, картинки, текст и капсулы.
//! Превью загружает результат в текстуры, рендер пишет в файлы и накладывает `overlay`: одна картинка
//! на обе стороны, поэтому совпадение не зависит от того, как их рисовал бы GL или ffmpeg.
//!
//! Всё рисуется в пикселях холста шаблона; на другое выходное разрешение масштабирует потребитель.

use std::path::{Path, PathBuf};

use image::imageops::FilterType;
use sc_core::{Align, Background, Canvas, Color, Layer, Paint, Rect, Shadow, Template, VAlign, gradient_t, rounded_rect_coverage};

pub mod text;

pub use text::{TextStyle, warm_up};

#[derive(Debug, thiserror::Error)]
pub enum RasterError {
    #[error("не удалось открыть картинку {path}: {source}")]
    Image { path: PathBuf, source: image::ImageError },
}

/// RGBA8 с premultiplied альфой.
#[derive(Debug, Clone, PartialEq)]
pub struct Bitmap {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl Bitmap {
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height, data: vec![0; (width * height * 4) as usize] }
    }

    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * self.width + x) * 4) as usize;
        self.data[i..i + 4].try_into().unwrap()
    }

    fn set(&mut self, x: u32, y: u32, c: [f32; 4]) {
        let i = ((y * self.width + x) * 4) as usize;
        for (d, v) in self.data[i..i + 4].iter_mut().zip(c) {
            *d = (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    }

    /// PAM (`P7`, `RGB_ALPHA`, альфа не premultiplied) — читает любой ffmpeg, без зависимостей.
    pub fn to_pam(&self) -> Vec<u8> {
        let header = format!(
            "P7\nWIDTH {}\nHEIGHT {}\nDEPTH 4\nMAXVAL 255\nTUPLTYPE RGB_ALPHA\nENDHDR\n",
            self.width, self.height
        );
        let mut out = header.into_bytes();
        out.reserve(self.data.len());
        for px in self.data.as_chunks::<4>().0 {
            let a = px[3];
            if a == 0 {
                out.extend([0, 0, 0, 0]);
            } else {
                let un = |c: u8| ((c as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8;
                out.extend([un(px[0]), un(px[1]), un(px[2]), a]);
            }
        }
        out
    }
}

/// Картинка на холсте: левый верхний угол в пикселях холста (может выходить за край — тень).
#[derive(Debug, Clone, PartialEq)]
pub struct Sprite {
    pub x: i32,
    pub y: i32,
    pub bitmap: Bitmap,
}

/// Растры одного слоя: `below` — под содержимым слоя (тень видео), `above` — над ним (рамка видео) или сам слой.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LayerArt {
    pub below: Option<Sprite>,
    pub above: Option<Sprite>,
}

/// Всё растровое в шаблоне: фон (если он не цвет, не размытие и не видео) и растры слоёв по порядку.
#[derive(Debug, Clone, PartialEq)]
pub struct Art {
    pub background: Option<Bitmap>,
    pub layers: Vec<LayerArt>,
}

impl Art {
    pub fn new(template: &Template) -> Result<Self, RasterError> {
        let canvas = template.canvas;
        Ok(Self {
            background: background(&template.background, canvas)?,
            layers: template.layers.iter().map(|l| layer(l, canvas)).collect::<Result<_, _>>()?,
        })
    }
}

/// Пиксели слоя видео на холсте — те же, что у рендера (чётные, требование yuv420).
pub fn video_rect_px(dest: Rect, canvas: Canvas) -> (u32, u32, u32, u32) {
    dest.to_pixels_even(canvas.width, canvas.height)
}

/// Прямоугольник остальных слоёв в пикселях холста (округление, не меньше 1×1).
pub fn rect_px(r: Rect, canvas: Canvas) -> (i32, i32, u32, u32) {
    let (cw, ch) = (canvas.width as f32, canvas.height as f32);
    let x0 = (r.x * cw).round() as i32;
    let y0 = (r.y * ch).round() as i32;
    let x1 = ((r.x + r.w) * cw).round() as i32;
    let y1 = ((r.y + r.h) * ch).round() as i32;
    (x0, y0, (x1 - x0).max(1) as u32, (y1 - y0).max(1) as u32)
}

pub fn background(bg: &Background, canvas: Canvas) -> Result<Option<Bitmap>, RasterError> {
    let (w, h) = (canvas.width, canvas.height);
    match bg {
        Background::Gradient { from, to, angle } => {
            let mut b = Bitmap::new(w, h);
            let (from, to) = (premul(*from), premul(*to));
            for y in 0..h {
                for x in 0..w {
                    let t = gradient_t(*angle, w as f32, h as f32, x as f32 + 0.5, y as f32 + 0.5);
                    let c = lerp(from, to, t);
                    // Фон непрозрачный: полупрозрачные цвета ложатся на чёрный.
                    b.set(x, y, [c[0], c[1], c[2], 1.0]);
                }
            }
            Ok(Some(b))
        }
        Background::Image { path, darken } => {
            let img = open(path)?;
            let (iw, ih) = img.dimensions();
            let c = Rect::cover(iw as f32 / ih as f32, w as f32 / h as f32);
            let (cx, cy, cw, ch) = (
                (c.x * iw as f32).round() as u32,
                (c.y * ih as f32).round() as u32,
                ((c.w * iw as f32).round() as u32).clamp(1, iw),
                ((c.h * ih as f32).round() as u32).clamp(1, ih),
            );
            let cropped = image::imageops::crop_imm(&img, cx, cy, cw.min(iw - cx), ch.min(ih - cy)).to_image();
            let scaled = image::imageops::resize(&cropped, w, h, FilterType::CatmullRom);
            let k = 1.0 - darken;
            let mut b = Bitmap::new(w, h);
            for (x, y, p) in scaled.enumerate_pixels() {
                let c = premul(Color(p.0));
                b.set(x, y, [c[0] * k, c[1] * k, c[2] * k, 1.0]);
            }
            Ok(Some(b))
        }
        Background::Blur { .. } | Background::Color { .. } | Background::Video { .. } => Ok(None),
    }
}

pub fn layer(layer: &Layer, canvas: Canvas) -> Result<LayerArt, RasterError> {
    let cw = canvas.width as f32;
    Ok(match layer {
        Layer::MainVideo { dest, corner_radius, border, shadow, .. } => {
            let (x, y, w, h) = video_rect_px(*dest, canvas);
            let r = corner_radius * cw;
            LayerArt {
                below: shadow.map(|s| shadow_sprite(x as i32, y as i32, w, h, r, &s, cw)),
                above: border.map(|b| {
                    let bw = (b.width * cw).max(0.5);
                    let mut bm = Bitmap::new(w, h);
                    let c = premul(b.color);
                    for py in 0..h {
                        for px in 0..w {
                            let (fx, fy) = (px as f32 + 0.5, py as f32 + 0.5);
                            let outer = rounded_rect_coverage(w as f32, h as f32, r, fx, fy);
                            let inner = rounded_rect_coverage(
                                w as f32 - 2.0 * bw,
                                h as f32 - 2.0 * bw,
                                (r - bw).max(0.0),
                                fx - bw,
                                fy - bw,
                            );
                            let a = (outer - inner).max(0.0);
                            bm.set(px, py, c.map(|v| v * a));
                        }
                    }
                    Sprite { x: x as i32, y: y as i32, bitmap: bm }
                }),
            }
        }
        Layer::Shape { rect, paint, corner_radius } => {
            let (x, y, w, h) = rect_px(*rect, canvas);
            let r = corner_radius * cw;
            let mut bm = Bitmap::new(w, h);
            for py in 0..h {
                for px in 0..w {
                    let (fx, fy) = (px as f32 + 0.5, py as f32 + 0.5);
                    let c = match paint {
                        Paint::Solid { color } => premul(*color),
                        Paint::Linear { from, to, angle } => {
                            lerp(premul(*from), premul(*to), gradient_t(*angle, w as f32, h as f32, fx, fy))
                        }
                    };
                    let a = rounded_rect_coverage(w as f32, h as f32, r, fx, fy);
                    bm.set(px, py, c.map(|v| v * a));
                }
            }
            LayerArt { below: None, above: Some(Sprite { x, y, bitmap: bm }) }
        }
        Layer::Image { rect, path, opacity } => {
            let (x, y, w, h) = rect_px(*rect, canvas);
            let (bm, ox, oy) = fit_image(&open(path)?, w, h, *opacity);
            LayerArt { below: None, above: Some(Sprite { x: x + ox, y: y + oy, bitmap: bm }) }
        }
        Layer::Text { rect, content, font, size, color, accent_color, accent_mode, outline, shadow, align, valign } => {
            let (x, y, w, h) = rect_px(*rect, canvas);
            let style = TextStyle {
                font,
                size: size * cw,
                color: *color,
                accent_color: *accent_color,
                accent_mode: *accent_mode,
                outline: outline.map(|o| ((o.width * cw).max(0.5), o.color)),
                align: *align,
                valign: *valign,
            };
            LayerArt { below: None, above: text_sprite(x, y, w, h, content, &style, shadow.as_ref(), cw) }
        }
        Layer::Capsule { rect, logo, text, font, text_color, fill } => {
            let (x, y, w, h) = rect_px(*rect, canvas);
            let logo = logo.as_deref().map(open).transpose()?;
            LayerArt { below: None, above: Some(Sprite { x, y, bitmap: capsule(w, h, logo.as_ref(), text, font, *text_color, *fill) }) }
        }
        // Место баннера — этап 5.
        Layer::BannerSlot { .. } => LayerArt::default(),
    })
}

/// Картинка, вписанная в `w`×`h` с сохранением пропорций, и её сдвиг до центра прямоугольника.
fn fit_image(img: &image::RgbaImage, w: u32, h: u32, opacity: f32) -> (Bitmap, i32, i32) {
    let (iw, ih) = (img.width() as f32, img.height() as f32);
    let k = (w as f32 / iw).min(h as f32 / ih);
    let (sw, sh) = (((iw * k).round() as u32).clamp(1, w), ((ih * k).round() as u32).clamp(1, h));
    let scaled = image::imageops::resize(img, sw, sh, FilterType::CatmullRom);
    let mut bm = Bitmap::new(sw, sh);
    for (px, py, p) in scaled.enumerate_pixels() {
        bm.set(px, py, premul(Color(p.0)).map(|v| v * opacity));
    }
    (bm, (w - sw) as i32 / 2, (h - sh) as i32 / 2)
}

/// Текстовый слой: прямоугольник `(x, y, w, h)` с полями под обводку, вылезающие глифы и тень.
#[allow(clippy::too_many_arguments)]
fn text_sprite(x: i32, y: i32, w: u32, h: u32, content: &str, style: &TextStyle, shadow: Option<&Shadow>, cw: f32) -> Option<Sprite> {
    if content.trim().is_empty() {
        return None;
    }
    let sigma = shadow.map_or(0.0, |s| s.blur * cw / 2.0);
    let (sdx, sdy) = shadow.map_or((0, 0), |s| ((s.dx * cw).round() as i32, (s.dy * cw).round() as i32));
    let outline = style.outline.map_or(0.0, |o| o.0);
    let m = (style.size * 0.5 + outline + sigma * 3.0).ceil() as u32 + sdx.unsigned_abs().max(sdy.unsigned_abs()) + 1;
    let (bw, bh) = (w + 2 * m, h + 2 * m);
    let mut pm = tiny_skia::Pixmap::new(bw, bh)?;
    text::draw_text(&mut pm, (m as f32, m as f32, w as f32, h as f32), content, style);
    let mut bm = Bitmap { width: bw, height: bh, data: pm.take() };
    if let Some(s) = shadow {
        add_shadow(&mut bm, s.color, sigma, sdx, sdy);
    }
    Some(Sprite { x: x - m as i32, y: y - m as i32, bitmap: bm })
}

/// Подкладывает под картинку её тень: альфа, размытая гауссом `sigma` и сдвинутая на `(dx, dy)`.
fn add_shadow(bm: &mut Bitmap, color: Color, sigma: f32, dx: i32, dy: i32) {
    let (w, h) = (bm.width as usize, bm.height as usize);
    let mut alpha: Vec<f32> = bm.data.as_chunks::<4>().0.iter().map(|p| p[3] as f32 / 255.0).collect();
    if sigma > 0.0 {
        for size in box_sizes(sigma) {
            box_blur(&mut alpha, w, h, size);
        }
    }
    let c = premul(color);
    for y in 0..h {
        for x in 0..w {
            let (sx, sy) = (x as i32 - dx, y as i32 - dy);
            let a = if (0..w as i32).contains(&sx) && (0..h as i32).contains(&sy) { alpha[sy as usize * w + sx as usize] } else { 0.0 };
            let i = (y * w + x) * 4;
            let top = bm.data[i + 3] as f32 / 255.0;
            for (k, ck) in c.iter().enumerate() {
                let v = bm.data[i + k] as f32 / 255.0 + ck * a * (1.0 - top);
                bm.data[i + k] = (v.clamp(0.0, 1.0) * 255.0).round() as u8;
            }
        }
    }
}

/// Капсула: плашка-«таблетка», логотип слева (вписан в круг высоты), текст по центру оставшегося места.
fn capsule(w: u32, h: u32, logo: Option<&image::RgbaImage>, text: &str, font: &str, text_color: Color, fill: Color) -> Bitmap {
    let mut bm = Bitmap::new(w, h);
    let c = premul(fill);
    for py in 0..h {
        for px in 0..w {
            let a = rounded_rect_coverage(w as f32, h as f32, h as f32 / 2.0, px as f32 + 0.5, py as f32 + 0.5);
            bm.set(px, py, c.map(|v| v * a));
        }
    }
    let hf = h as f32;
    let pad = (hf * 0.12).round() as u32;
    let mut text_x0 = hf * 0.45;
    if let Some(img) = logo {
        let side = h.saturating_sub(2 * pad).max(1);
        let (lb, ox, oy) = fit_image(img, side, side, 1.0);
        let (lx, ly) = (pad as i32 + ox, pad as i32 + oy);
        for py in 0..lb.height {
            for px in 0..lb.width {
                let (x, y) = (lx + px as i32, ly + py as i32);
                if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
                    continue;
                }
                let src = lb.pixel(px, py).map(|v| v as f32 / 255.0);
                let dst = bm.pixel(x as u32, y as u32).map(|v| v as f32 / 255.0);
                bm.set(x as u32, y as u32, std::array::from_fn(|k| src[k] + dst[k] * (1.0 - src[3])));
            }
        }
        text_x0 = (pad + side) as f32 + hf * 0.25;
    }
    let text_w = (w as f32 - hf * 0.45 - text_x0).max(1.0);
    let style = TextStyle {
        font,
        size: hf * 0.5,
        color: text_color,
        accent_color: text_color,
        accent_mode: sc_core::AccentMode::Color,
        outline: None,
        align: if logo.is_some() { Align::Left } else { Align::Center },
        valign: VAlign::Center,
    };
    if let Some(mut pm) = tiny_skia::Pixmap::from_vec(std::mem::take(&mut bm.data), tiny_skia::IntSize::from_wh(w, h).unwrap()) {
        text::draw_text(&mut pm, (text_x0, 0.0, text_w, hf), text, &style);
        bm.data = pm.take();
    }
    bm
}

fn open(path: &Path) -> Result<image::RgbaImage, RasterError> {
    image::open(path).map(|i| i.to_rgba8()).map_err(|source| RasterError::Image { path: path.to_owned(), source })
}

fn premul(c: Color) -> [f32; 4] {
    let [r, g, b, a] = c.0.map(|v| v as f32 / 255.0);
    [r * a, g * a, b * a, a]
}

fn lerp(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)
}

/// Тень скруглённого прямоугольника `w`×`h` в точке (x, y): та же фигура, сдвинутая и размытая
/// (три прохода box blur ≈ гаусс с сигмой `blur / 2`).
fn shadow_sprite(x: i32, y: i32, w: u32, h: u32, r: f32, s: &Shadow, cw: f32) -> Sprite {
    let blur = s.blur * cw;
    let sigma = blur / 2.0;
    let m = (sigma * 3.0).ceil() as u32 + 1;
    let (bw, bh) = (w + 2 * m, h + 2 * m);
    let mut alpha = vec![0f32; (bw * bh) as usize];
    for py in 0..h {
        for px in 0..w {
            alpha[((py + m) * bw + px + m) as usize] =
                rounded_rect_coverage(w as f32, h as f32, r, px as f32 + 0.5, py as f32 + 0.5);
        }
    }
    if sigma > 0.0 {
        for size in box_sizes(sigma) {
            box_blur(&mut alpha, bw as usize, bh as usize, size);
        }
    }
    let c = premul(s.color);
    let mut bm = Bitmap::new(bw, bh);
    for py in 0..bh {
        for px in 0..bw {
            let a = alpha[(py * bw + px) as usize];
            bm.set(px, py, c.map(|v| v * a));
        }
    }
    Sprite {
        x: x + (s.dx * cw).round() as i32 - m as i32,
        y: y + (s.dy * cw).round() as i32 - m as i32,
        bitmap: bm,
    }
}

/// Ширины трёх box-фильтров, дающих в сумме гаусс с сигмой `sigma` (нечётные).
fn box_sizes(sigma: f32) -> [usize; 3] {
    let ideal = (12.0 * sigma * sigma / 3.0 + 1.0).sqrt();
    let mut lo = ideal.floor() as usize;
    if lo.is_multiple_of(2) {
        lo = lo.saturating_sub(1);
    }
    let lo = lo.max(1);
    let hi = lo + 2;
    let lf = lo as f32;
    let m = ((12.0 * sigma * sigma - 3.0 * lf * lf - 12.0 * lf - 9.0) / (-4.0 * lf - 4.0)).round();
    std::array::from_fn(|i| if (i as f32) < m { lo } else { hi })
}

/// Box blur ширины `size` по горизонтали и вертикали; за краем — нули.
fn box_blur(buf: &mut [f32], w: usize, h: usize, size: usize) {
    let r = size / 2;
    let norm = 1.0 / size as f32;
    let mut line = vec![0f32; w.max(h)];
    let mut out = vec![0f32; h];
    for y in 0..h {
        let row = &mut buf[y * w..(y + 1) * w];
        line[..w].copy_from_slice(row);
        blur_line(&line[..w], row, r, norm);
    }
    for x in 0..w {
        for y in 0..h {
            line[y] = buf[y * w + x];
        }
        blur_line(&line[..h], &mut out, r, norm);
        for y in 0..h {
            buf[y * w + x] = out[y];
        }
    }
}

fn blur_line(src: &[f32], dst: &mut [f32], r: usize, norm: f32) {
    let n = src.len() as isize;
    let r = r as isize;
    let at = |i: isize| if (0..n).contains(&i) { src[i as usize] } else { 0.0 };
    let mut acc: f32 = (-r..=r).map(at).sum();
    for i in 0..n {
        dst[i as usize] = acc * norm;
        acc += at(i + r + 1) - at(i - r);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sc_core::{Border, Shadow};

    fn canvas() -> Canvas {
        Canvas { width: 100, height: 200 }
    }

    #[test]
    fn gradient_background_and_pam() {
        let bg = Background::Gradient { from: Color::BLACK, to: Color::WHITE, angle: 0.0 };
        let b = background(&bg, canvas()).unwrap().unwrap();
        assert_eq!((b.width, b.height), (100, 200));
        assert!(b.pixel(50, 0)[0] < 3 && b.pixel(50, 199)[0] > 252, "{:?} {:?}", b.pixel(50, 0), b.pixel(50, 199));
        assert_eq!(b.pixel(0, 100), b.pixel(99, 100));
        let pam = b.to_pam();
        assert!(pam.starts_with(b"P7\nWIDTH 100\nHEIGHT 200\nDEPTH 4\n"));
        assert_eq!(pam.len(), pam.iter().position(|&c| c == b'R').unwrap() + "RGB_ALPHA\nENDHDR\n".len() + 100 * 200 * 4);
        assert!(background(&Background::Color { color: Color::BLACK }, canvas()).unwrap().is_none());
    }

    #[test]
    fn video_border_and_shadow() {
        let l = Layer::MainVideo {
            crop: Rect::FULL,
            dest: Rect::new(0.1, 0.25, 0.8, 0.25),
            corner_radius: 0.1,
            border: Some(Border { width: 0.02, color: Color::WHITE }),
            shadow: Some(Shadow { color: Color::BLACK, blur: 0.04, dx: 0.0, dy: 0.05 }),
        };
        let art = layer(&l, canvas()).unwrap();
        let border = art.above.unwrap();
        assert_eq!((border.x, border.y, border.bitmap.width, border.bitmap.height), (10, 50, 80, 50));
        // рамка 2 px: край закрашен, середина пустая, угол срезан скруглением
        assert_eq!(border.bitmap.pixel(40, 0), [255; 4]);
        assert_eq!(border.bitmap.pixel(40, 25), [0; 4]);
        assert_eq!(border.bitmap.pixel(0, 0)[3], 0);
        let shadow = art.below.unwrap();
        // сигма 2 px → поле 7 px; сдвиг вниз на 5 px
        assert_eq!((shadow.x, shadow.y), (3, 48));
        assert_eq!((shadow.bitmap.width, shadow.bitmap.height), (94, 64));
        let mid = shadow.bitmap.pixel(47, 32);
        assert_eq!(mid, [0, 0, 0, 255]);
        // за краем фигуры тень спадает, но не обрывается
        let edge = shadow.bitmap.pixel(47, 7 + 50 + 2)[3];
        assert!(edge > 0 && edge < 128, "{edge}");
        assert_eq!(shadow.bitmap.pixel(0, 0)[3], 0);
    }

    #[test]
    fn shape_gradient_premultiplied() {
        let l = Layer::Shape {
            rect: Rect::new(0.0, 0.5, 1.0, 0.5),
            paint: Paint::Linear { from: Color([0, 0, 0, 0]), to: Color::BLACK, angle: 0.0 },
            corner_radius: 0.0,
        };
        let s = layer(&l, canvas()).unwrap().above.unwrap();
        assert_eq!((s.x, s.y, s.bitmap.width, s.bitmap.height), (0, 100, 100, 100));
        assert!(s.bitmap.pixel(50, 0)[3] < 3);
        assert!(s.bitmap.pixel(50, 99)[3] > 252);
        let mid = s.bitmap.pixel(50, 50);
        assert!((120..=135).contains(&mid[3]) && mid[0] == 0, "{mid:?}");
    }

    #[test]
    fn image_layer_fits_and_missing_file_errors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("logo.png");
        image::RgbaImage::from_pixel(40, 20, image::Rgba([255, 0, 0, 255])).save(&path).unwrap();
        let l = Layer::Image { rect: Rect::new(0.0, 0.0, 1.0, 0.5), path: path.clone(), opacity: 0.5 };
        let s = layer(&l, canvas()).unwrap().above.unwrap();
        // 40×20 в 100×100 — 100×50 по центру по вертикали
        assert_eq!((s.x, s.y, s.bitmap.width, s.bitmap.height), (0, 25, 100, 50));
        assert_eq!(s.bitmap.pixel(50, 25), [128, 0, 0, 128]);

        let bg = Background::Image { path: path.clone(), darken: 0.5 };
        let b = background(&bg, canvas()).unwrap().unwrap();
        assert_eq!(b.pixel(50, 100), [128, 0, 0, 255]);

        let missing = Layer::Image { rect: Rect::FULL, path: dir.path().join("нет.png"), opacity: 1.0 };
        assert!(matches!(layer(&missing, canvas()), Err(RasterError::Image { .. })));
    }

    fn text_layer(content: &str) -> Layer {
        Layer::Text {
            rect: Rect::new(0.1, 0.1, 0.8, 0.2),
            content: content.into(),
            font: "DejaVu Sans Bold".into(),
            size: 0.1,
            color: Color::WHITE,
            accent_color: Color::rgb(255, 0, 0),
            accent_mode: sc_core::AccentMode::Color,
            outline: None,
            shadow: None,
            align: Align::Center,
            valign: VAlign::Center,
        }
    }

    /// Число пикселей спрайта (premultiplied RGBA), подходящих под условие.
    fn count(s: &Sprite, f: impl Fn([u8; 4]) -> bool) -> usize {
        s.bitmap.data.as_chunks::<4>().0.iter().filter(|p| f(**p)).count()
    }

    #[test]
    fn text_accent_color_and_plate() {
        let s = layer(&text_layer("ТОП *ЯЗЫКОВ*"), canvas()).unwrap().above.unwrap();
        let white = count(&s, |p| p[3] == 255 && p[0] == 255 && p[1] == 255);
        let red = count(&s, |p| p[3] == 255 && p[0] == 255 && p[1] == 0);
        assert!(white > 20 && red > 20, "белых {white}, красных {red}");
        // поля под вылезающие глифы: спрайт больше прямоугольника 80×40
        assert!(s.x < 10 && s.y < 20 && s.bitmap.width > 80, "{} {} {}", s.x, s.y, s.bitmap.width);

        let mut l = text_layer("ТОП *ЯЗЫКОВ*");
        let Layer::Text { accent_mode, .. } = &mut l else { panic!() };
        *accent_mode = sc_core::AccentMode::Plate;
        let p = layer(&l, canvas()).unwrap().above.unwrap();
        let plate = count(&p, |p| p[3] == 255 && p[0] == 255 && p[1] == 0);
        // плашка закрашивает намного больше, чем сами буквы
        assert!(plate > red * 2, "плашка {plate}, буквы {red}");
        assert!(count(&p, |p| p[3] == 255 && p[1] == 255) > 20, "текст на плашке белый");
    }

    #[test]
    fn text_outline_shadow_and_empty() {
        let plain = layer(&text_layer("Привет"), canvas()).unwrap().above.unwrap();
        let mut l = text_layer("Привет");
        let Layer::Text { outline, .. } = &mut l else { panic!() };
        *outline = Some(sc_core::Outline { width: 0.03, color: Color::BLACK });
        let o = layer(&l, canvas()).unwrap().above.unwrap();
        let opaque = |s: &Sprite| count(s, |p| p[3] > 128);
        assert!(opaque(&o) > opaque(&plain) * 3 / 2, "{} vs {}", opaque(&o), opaque(&plain));
        assert!(count(&o, |p| p[3] == 255 && p[0] == 0) > 50, "чёрная обводка");

        let Layer::Text { outline, shadow, .. } = &mut l else { panic!() };
        *outline = None;
        *shadow = Some(Shadow { color: Color::BLACK, blur: 0.0, dx: 0.02, dy: 0.02 });
        let sh = layer(&l, canvas()).unwrap().above.unwrap();
        // жёсткая тень сдвинута на 2 px: чёрные непрозрачные пиксели появились, белые остались
        assert!(count(&sh, |p| p[3] == 255 && p[0] == 0) > 20);
        assert_eq!(count(&sh, |p| p[3] == 255 && p[0] == 255), count(&plain, |p| p[3] == 255 && p[0] == 255));

        assert_eq!(layer(&text_layer("  "), canvas()).unwrap(), LayerArt::default());
    }

    #[test]
    fn long_text_shrinks_to_fit() {
        let short = layer(&text_layer("Слово"), canvas()).unwrap().above.unwrap();
        let long = layer(&text_layer("Очень длинный заголовок из многих слов, который не помещается"), canvas()).unwrap().above.unwrap();
        // строки по вертикали: от верхнего до нижнего непрозрачного пикселя на холсте
        let rows = |s: &Sprite| {
            let w = s.bitmap.width as usize;
            let ys: Vec<usize> = s.bitmap.data.as_chunks::<4>().0.iter().enumerate().filter(|(_, p)| p[3] > 0).map(|(i, _)| i / w).collect();
            (s.y + *ys.first().unwrap() as i32, s.y + *ys.last().unwrap() as i32)
        };
        // текст уменьшен и остался в прямоугольнике (y 20..60)
        let (t, b) = rows(&long);
        assert!(t >= 18 && b <= 62, "{t}..{b}");
        let (st, sb) = rows(&short);
        assert!(sb - st < b - t, "одна строка ниже нескольких");
    }

    #[test]
    fn capsule_with_logo() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("logo.png");
        image::RgbaImage::from_pixel(10, 10, image::Rgba([0, 255, 0, 255])).save(&path).unwrap();
        let l = Layer::Capsule {
            rect: Rect::new(0.1, 0.1, 0.8, 0.1),
            logo: Some(path),
            text: "канал".into(),
            font: "DejaVu Sans".into(),
            text_color: Color::WHITE,
            fill: Color::rgb(0, 0, 255),
        };
        let s = layer(&l, canvas()).unwrap().above.unwrap();
        assert_eq!((s.x, s.y, s.bitmap.width, s.bitmap.height), (10, 20, 80, 20));
        assert_eq!(s.bitmap.pixel(0, 0)[3], 0, "углы скруглены");
        assert_eq!(s.bitmap.pixel(10, 10), [0, 255, 0, 255], "логотип слева");
        assert_eq!(s.bitmap.pixel(40, 1), [0, 0, 255, 255], "заливка");
        assert!(count(&s, |p| p == [255; 4]) > 5, "белый текст");
    }

    #[test]
    fn box_blur_keeps_mass() {
        assert_eq!(box_sizes(2.0).iter().sum::<usize>() % 2, 1);
        let mut buf = vec![0f32; 41 * 41];
        buf[20 * 41 + 20] = 1.0;
        for s in box_sizes(3.0) {
            box_blur(&mut buf, 41, 41, s);
        }
        let sum: f32 = buf.iter().sum();
        assert!((sum - 1.0).abs() < 1e-4, "{sum}");
        // дисперсия по x ≈ σ²
        let var: f32 = buf.iter().enumerate().map(|(i, v)| v * ((i % 41) as f32 - 20.0).powi(2)).sum();
        assert!((var - 9.0).abs() < 1.5, "{var}");
    }
}
