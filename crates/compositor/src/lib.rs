//! GPU-часть превью: кадры исходника и видео-фона (в них рисует mpv), размытый фон и сборка
//! всего холста по шаблону в одну текстуру, которую показывает UI.
//!
//! Геометрия — те же функции, что у рендера: кропы из `sc-core` ([`main_video_crop_px`], [`background_crop`]),
//! пиксели слоёв и все растры (тени, рамки, фигуры, фон-картинка) — из `sc-raster`.

pub mod gl;
pub mod quad;

use sc_core::{Background, Layer, Rect, Template, background_crop, main_video_crop_px};
use sc_raster::{Bitmap, LayerArt, Sprite, video_rect_px};

pub use gl::{Blur, BlurParams, Target};
use quad::{Quad, QuadParams};

/// Во сколько раз фон меньше холста: blur в ¼ — 0.4 мс против 7 мс на полном, на глаз не отличить (шаг 0.5).
pub const BLUR_DIVISOR: u32 = 4;
/// Сигма размытия фона в пикселях холста 1080×1920 (масштабируется под размер холста).
pub const BLUR_SIGMA: f32 = 40.0;

/// Растр на GPU: текстура и её место на холсте в пикселях холста.
struct GpuSprite {
    tex: glow::Texture,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
}

/// Растры одного слоя на GPU и прямоугольник, для которого они нарисованы.
struct GpuLayer {
    rect: Rect,
    below: Option<GpuSprite>,
    above: Option<GpuSprite>,
}

#[derive(Default)]
struct GpuArt {
    background: Option<GpuSprite>,
    /// По слоям шаблона; `None` — растра нет (ещё не готов).
    layers: Vec<Option<GpuLayer>>,
    /// Растры, которые рисуются вместо растра слоя с этим индексом (субтитры — по времени).
    overlays: Vec<(usize, GpuLayer)>,
}

/// Что положить на место слоя в [`Compositor::set_layers`].
pub enum ArtSlot<'a> {
    /// Ничего (растр ещё не готов).
    Empty,
    /// Текстуры слоя с этим индексом из прошлого вызова (без повторной загрузки).
    Keep(usize),
    /// Загрузить растры, нарисованные для прямоугольника `Rect`.
    Upload(Rect, &'a LayerArt),
}

pub struct Compositor {
    video: Option<Target>,
    /// Размер исходника (для формул кропа) — может отличаться от размера текстуры.
    src_size: (u32, u32),
    bg_video: Option<Target>,
    blur: Blur,
    blurred: Option<glow::Texture>,
    quad: Quad,
    canvas: Option<Target>,
    art: GpuArt,
}

impl Compositor {
    /// # Safety
    /// GL-контекст должен быть текущим.
    pub unsafe fn new(gl: &glow::Context) -> Result<Self, String> {
        unsafe {
            Ok(Self {
                video: None,
                src_size: (0, 0),
                bg_video: None,
                blur: Blur::new(gl)?,
                blurred: None,
                quad: Quad::new(gl)?,
                canvas: None,
                art: GpuArt::default(),
            })
        }
    }

    /// Цель для mpv: (fbo, ширина, высота), если кадр уже известен.
    pub fn video_target(&self) -> Option<(u32, i32, i32)> {
        self.video.as_ref().map(|t| (t.fbo.0.get(), t.w, t.h))
    }

    /// Цель для mpv видео-фона.
    pub fn bg_video_target(&self) -> Option<(u32, i32, i32)> {
        self.bg_video.as_ref().map(|t| (t.fbo.0.get(), t.w, t.h))
    }

    pub fn src_size(&self) -> Option<(u32, u32)> {
        (self.src_size.0 > 0 && self.src_size.1 > 0).then_some(self.src_size)
    }

    /// Готовит текстуру кадра исходника под размер `w`×`h`.
    ///
    /// # Safety
    /// GL-контекст должен быть текущим.
    pub unsafe fn ensure_video(&mut self, gl: &glow::Context, w: u32, h: u32) -> Result<(), String> {
        if w == 0 || h == 0 || self.src_size == (w, h) && self.video.is_some() {
            return Ok(());
        }
        unsafe {
            if let Some(t) = self.video.take() {
                t.delete(gl);
            }
            self.video = Some(Target::new(gl, w as i32, h as i32, true)?);
        }
        self.src_size = (w, h);
        self.blurred = None;
        Ok(())
    }

    /// Готовит текстуру кадра видео-фона; `None` — фона-видео нет, текстура освобождается.
    ///
    /// # Safety
    /// GL-контекст должен быть текущим.
    pub unsafe fn ensure_bg_video(&mut self, gl: &glow::Context, size: Option<(u32, u32)>) -> Result<(), String> {
        let current = self.bg_video.as_ref().map(|t| (t.w as u32, t.h as u32));
        let size = size.filter(|s| s.0 > 0 && s.1 > 0);
        if current == size {
            return Ok(());
        }
        unsafe {
            if let Some(t) = self.bg_video.take() {
                t.delete(gl);
            }
            if let Some((w, h)) = size {
                self.bg_video = Some(Target::new(gl, w as i32, h as i32, true)?);
            }
        }
        Ok(())
    }

    /// Вызывать после того, как mpv нарисовал кадр исходника: мипмапы и размытый фон.
    ///
    /// # Safety
    /// GL-контекст должен быть текущим.
    pub unsafe fn video_rendered(&mut self, gl: &glow::Context, template: &Template) -> Result<(), String> {
        let Some(video) = &self.video else { return Ok(()) };
        unsafe {
            video.update_mipmaps(gl);
            self.refresh_background(gl, template)
        }
    }

    /// Вызывать после того, как mpv нарисовал кадр видео-фона.
    ///
    /// # Safety
    /// GL-контекст должен быть текущим.
    pub unsafe fn bg_video_rendered(&mut self, gl: &glow::Context) {
        if let Some(t) = &self.bg_video {
            unsafe { t.update_mipmaps(gl) };
        }
    }

    /// Пересчитать размытый фон (новый кадр или шаблон поменялся на паузе).
    ///
    /// # Safety
    /// GL-контекст должен быть текущим.
    pub unsafe fn refresh_background(&mut self, gl: &glow::Context, template: &Template) -> Result<(), String> {
        let Some(video) = &self.video else { return Ok(()) };
        let Background::Blur { darken, scale } = template.background else {
            self.blurred = None;
            return Ok(());
        };
        let canvas = template.canvas;
        let r = background_crop(scale, self.src_size, canvas);
        let k = canvas.width as f32 / 1080.0;
        let p = BlurParams {
            uv_rect: [r.x, r.y, r.w, r.h],
            out_w: (canvas.width / BLUR_DIVISOR).max(1) as i32,
            out_h: (canvas.height / BLUR_DIVISOR).max(1) as i32,
            sigma: BLUR_SIGMA * k / BLUR_DIVISOR as f32,
            dim: 1.0 - darken,
        };
        let (tex, _) = unsafe { self.blur.run(gl, video, &p)? };
        self.blurred = Some(tex);
        Ok(())
    }

    /// Растр фона (градиент, картинка); `None` — фон без растра.
    ///
    /// # Safety
    /// GL-контекст должен быть текущим.
    pub unsafe fn set_background_art(&mut self, gl: &glow::Context, bg: Option<&Bitmap>) -> Result<(), String> {
        use glow::HasContext;
        unsafe {
            if let Some(s) = self.art.background.take() {
                gl.delete_texture(s.tex);
            }
            self.art.background = bg.map(|b| upload(gl, b, 0, 0)).transpose()?;
        }
        Ok(())
    }

    /// Растры слоёв в порядке шаблона. Текстуры прошлых слоёв, не попавшие в `Keep`, освобождаются.
    /// Растр, нарисованный для другого прямоугольника, чем у слоя сейчас, растягивается на новый —
    /// пока свежий растр готовится (перетаскивание мышью).
    ///
    /// # Safety
    /// GL-контекст должен быть текущим.
    pub unsafe fn set_layers(&mut self, gl: &glow::Context, slots: &[ArtSlot]) -> Result<(), String> {
        let mut old: Vec<Option<GpuLayer>> = std::mem::take(&mut self.art.layers);
        let mut result = Ok(());
        for slot in slots {
            let layer = match slot {
                ArtSlot::Empty => None,
                ArtSlot::Keep(i) => old.get_mut(*i).and_then(Option::take),
                ArtSlot::Upload(rect, art) => {
                    let sprite = |s: &Option<Sprite>| -> Result<Option<GpuSprite>, String> {
                        s.as_ref().map(|s| unsafe { upload(gl, &s.bitmap, s.x, s.y) }).transpose()
                    };
                    match sprite(&art.below).and_then(|b| Ok((b, sprite(&art.above)?))) {
                        Ok((below, above)) => Some(GpuLayer { rect: *rect, below, above }),
                        Err(e) => {
                            result = Err(e);
                            None
                        }
                    }
                }
            };
            self.art.layers.push(layer);
        }
        unsafe { delete_layers(gl, old) };
        result
    }

    /// Растр, который рисуется вместо растра слоя `index` (субтитры в текущий момент); `None` — убрать подмену.
    /// [`Compositor::set_layers`] подмены не трогает.
    ///
    /// # Safety
    /// GL-контекст должен быть текущим.
    pub unsafe fn set_overlay(&mut self, gl: &glow::Context, index: usize, art: Option<(Rect, &LayerArt)>) -> Result<(), String> {
        let old: Vec<Option<GpuLayer>> = self
            .art
            .overlays
            .extract_if(.., |(i, _)| *i == index)
            .map(|(_, l)| Some(l))
            .collect();
        unsafe { delete_layers(gl, old) };
        let Some((rect, art)) = art else { return Ok(()) };
        let sprite = |s: &Option<Sprite>| -> Result<Option<GpuSprite>, String> {
            s.as_ref().map(|s| unsafe { upload(gl, &s.bitmap, s.x, s.y) }).transpose()
        };
        let below = sprite(&art.below)?;
        let above = sprite(&art.above)?;
        self.art.overlays.push((index, GpuLayer { rect, below, above }));
        Ok(())
    }

    /// Собрать холст в текстуру `w`×`h` пикселей (размер на экране). Возвращает текстуру и `true`,
    /// если она пересоздана (тогда её надо заново зарегистрировать в egui).
    ///
    /// # Safety
    /// GL-контекст должен быть текущим.
    pub unsafe fn compose(&mut self, gl: &glow::Context, template: &Template, w: u32, h: u32) -> Result<(glow::Texture, bool), String> {
        use glow::HasContext;
        let (w, h) = (w.max(2) as i32, h.max(2) as i32);
        let mut recreated = false;
        unsafe {
            if self.canvas.as_ref().is_none_or(|t| t.w != w || t.h != h) {
                if let Some(t) = self.canvas.take() {
                    t.delete(gl);
                }
                self.canvas = Some(Target::new(gl, w, h, false)?);
                recreated = true;
            }
        }
        let target = self.canvas.as_ref().unwrap();
        let canvas = template.canvas;
        let (cw, ch) = (canvas.width as f32, canvas.height as f32);
        let px = |x: f32, y: f32, sw: f32, sh: f32| [x / cw, y / ch, sw / cw, sh / ch];
        let full = [0.0, 0.0, 1.0, 1.0];
        let q = &self.quad;
        let draw = |tex, dest, uv, radius_px, dim| unsafe {
            q.draw(gl, tex, w, h, &QuadParams { dest, uv, radius_px, dim });
        };
        // `map` — растр нарисован для прямоугольника `from`, а слой сейчас в `to` (в пикселях холста).
        let sprite_mapped = |s: &Option<GpuSprite>, map: Option<(Rect, Rect)>| {
            let Some(s) = s else { return };
            let (mut x, mut y, mut sw, mut sh) = (s.x as f32, s.y as f32, s.w as f32, s.h as f32);
            if let Some((from, to)) = map {
                let (kx, ky) = (to.w / from.w, to.h / from.h);
                x = to.x + (x - from.x) * kx;
                y = to.y + (y - from.y) * ky;
                sw *= kx;
                sh *= ky;
            }
            draw(s.tex, px(x, y, sw, sh), full, 0.0, 1.0);
        };
        let sprite = |s: &Option<GpuSprite>| sprite_mapped(s, None);

        unsafe {
            self.quad.begin(gl, target.fbo, w, h);
            let [r, g, b, a] = match template.background {
                Background::Color { color } => color.0.map(|v| v as f32 / 255.0),
                _ => [0.0, 0.0, 0.0, 1.0],
            };
            gl.clear_color(r * a, g * a, b * a, 1.0);
            gl.clear(glow::COLOR_BUFFER_BIT);
        }
        match &template.background {
            Background::Blur { .. } => {
                if let Some(tex) = self.blurred {
                    draw(tex, full, full, 0.0, 1.0);
                }
            }
            Background::Gradient { .. } | Background::Image { .. } => sprite(&self.art.background),
            Background::Video { darken, .. } => {
                if let Some(t) = &self.bg_video {
                    let c = Rect::cover(t.w as f32 / t.h as f32, cw / ch);
                    draw(t.tex, full, [c.x, c.y, c.w, c.h], 0.0, 1.0 - darken);
                }
            }
            Background::Color { .. } => {}
        }

        let scale = w as f32 / cw;
        for (i, layer) in template.layers.iter().enumerate() {
            let art = match self.art.overlays.iter().find(|(k, _)| *k == i) {
                Some((_, l)) => Some(l),
                None => self.art.layers.get(i).and_then(Option::as_ref),
            };
            let map = art.filter(|a| a.rect != layer.rect() && a.rect.w > 0.0 && a.rect.h > 0.0).map(|a| {
                let to_px = |r: Rect| Rect::new(r.x * cw, r.y * ch, r.w * cw, r.h * ch);
                (to_px(a.rect), to_px(layer.rect()))
            });
            let (below, above) = art.map_or((&None, &None), |a| (&a.below, &a.above));
            sprite_mapped(below, map);
            if let (Layer::MainVideo { crop, dest, corner_radius, .. }, Some(video), Some(src)) =
                (layer, &self.video, self.src_size())
            {
                let (sx, sy, sw, sh) = main_video_crop_px(*crop, src, *dest, canvas);
                let (s0, s1) = (src.0 as f32, src.1 as f32);
                let uv = [sx as f32 / s0, sy as f32 / s1, sw as f32 / s0, sh as f32 / s1];
                let (dx, dy, dw, dh) = video_rect_px(*dest, canvas);
                draw(video.tex, px(dx as f32, dy as f32, dw as f32, dh as f32), uv, corner_radius * cw * scale, 1.0);
            }
            sprite_mapped(above, map);
        }
        unsafe { self.quad.end(gl) };
        Ok((target.tex, recreated))
    }

    /// Есть ли кадр исходника (иначе на холсте нет видео — UI показывает подсказку).
    pub fn has_video(&self) -> bool {
        self.video.is_some()
    }

    unsafe fn delete_art(&mut self, gl: &glow::Context) {
        unsafe {
            let _ = self.set_background_art(gl, None);
            delete_layers(gl, std::mem::take(&mut self.art.layers));
            delete_layers(gl, std::mem::take(&mut self.art.overlays).into_iter().map(|(_, l)| Some(l)).collect());
        }
    }

    /// Освободить GL-ресурсы.
    ///
    /// # Safety
    /// GL-контекст должен быть текущим.
    pub unsafe fn delete(&mut self, gl: &glow::Context) {
        unsafe {
            for t in [self.video.take(), self.bg_video.take(), self.canvas.take()].into_iter().flatten() {
                t.delete(gl);
            }
            self.delete_art(gl);
            self.blur.delete(gl);
            self.quad.delete(gl);
        }
    }
}

/// # Safety
/// GL-контекст должен быть текущим.
unsafe fn delete_layers(gl: &glow::Context, layers: Vec<Option<GpuLayer>>) {
    use glow::HasContext;
    for s in layers.into_iter().flatten().flat_map(|l| [l.below, l.above]).flatten() {
        unsafe { gl.delete_texture(s.tex) };
    }
}

/// # Safety
/// GL-контекст должен быть текущим.
unsafe fn upload(gl: &glow::Context, b: &Bitmap, x: i32, y: i32) -> Result<GpuSprite, String> {
    let tex = unsafe { gl::upload_rgba(gl, b.width, b.height, &b.data)? };
    Ok(GpuSprite { tex, x, y, w: b.width, h: b.height })
}
