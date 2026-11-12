//! GPU-часть превью: текстура кадра исходника (в неё рисует mpv) и размытый фон по шаблону.
//!
//! Слои поверх фона рисует UI (egui) по тем же формулам из `sc-core`
//! ([`sc_core::main_video_crop`], [`sc_core::background_crop`]), что и рендер.

pub mod gl;

use sc_core::{Background, Template, background_crop};

pub use gl::{Blur, BlurParams, Target};

/// Во сколько раз фон меньше холста: blur в ¼ — 0.4 мс против 7 мс на полном, на глаз не отличить (шаг 0.5).
pub const BLUR_DIVISOR: u32 = 4;
/// Сигма размытия фона в пикселях холста 1080×1920 (масштабируется под размер холста).
pub const BLUR_SIGMA: f32 = 40.0;

pub struct Compositor {
    video: Option<Target>,
    blur: Blur,
    /// Размер исходника (для формул кропа) — может отличаться от размера текстуры.
    src_size: (u32, u32),
    has_background: bool,
}

impl Compositor {
    /// # Safety
    /// GL-контекст должен быть текущим.
    pub unsafe fn new(gl: &glow::Context) -> Result<Self, String> {
        Ok(Self { video: None, blur: unsafe { Blur::new(gl)? }, src_size: (0, 0), has_background: false })
    }

    /// Цель для mpv: (fbo, ширина, высота), если кадр уже известен.
    pub fn video_target(&self) -> Option<(u32, i32, i32)> {
        self.video.as_ref().map(|t| (t.fbo.0.get(), t.w, t.h))
    }

    pub fn src_size(&self) -> Option<(u32, u32)> {
        (self.src_size.0 > 0 && self.src_size.1 > 0).then_some(self.src_size)
    }

    /// Есть ли актуальный размытый фон (после первого кадра).
    pub fn has_background(&self) -> bool {
        self.has_background
    }

    /// Готовит текстуру под размер кадра `w`×`h`. Возвращает новую текстуру, если пересоздана.
    ///
    /// # Safety
    /// GL-контекст должен быть текущим.
    pub unsafe fn ensure_video(&mut self, gl: &glow::Context, w: u32, h: u32) -> Result<Option<glow::Texture>, String> {
        if w == 0 || h == 0 || self.src_size == (w, h) && self.video.is_some() {
            return Ok(None);
        }
        unsafe {
            if let Some(t) = self.video.take() {
                t.delete(gl);
            }
            let t = Target::new(gl, w as i32, h as i32, true)?;
            let tex = t.tex;
            self.video = Some(t);
            self.src_size = (w, h);
            self.has_background = false;
            Ok(Some(tex))
        }
    }

    /// Вызывать после того, как mpv нарисовал новый кадр: мипмапы и фон.
    ///
    /// # Safety
    /// GL-контекст должен быть текущим.
    pub unsafe fn frame_rendered(&mut self, gl: &glow::Context, template: &Template) -> Result<Option<glow::Texture>, String> {
        let Some(video) = &self.video else { return Ok(None) };
        unsafe {
            video.update_mipmaps(gl);
            self.refresh_background(gl, template)
        }
    }

    /// Пересчитать фон (например, шаблон поменялся на паузе). Возвращает текстуру фона, если пересоздана.
    ///
    /// # Safety
    /// GL-контекст должен быть текущим.
    pub unsafe fn refresh_background(&mut self, gl: &glow::Context, template: &Template) -> Result<Option<glow::Texture>, String> {
        let Some(video) = &self.video else { return Ok(None) };
        let Background::Blur { darken, scale } = template.background else {
            self.has_background = false;
            return Ok(None);
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
        let (tex, recreated) = unsafe { self.blur.run(gl, video, &p)? };
        self.has_background = true;
        Ok(recreated.then_some(tex))
    }

    /// Освободить GL-ресурсы.
    ///
    /// # Safety
    /// GL-контекст должен быть текущим.
    pub unsafe fn delete(&mut self, gl: &glow::Context) {
        unsafe {
            if let Some(t) = self.video.take() {
                t.delete(gl);
            }
            self.blur.delete(gl);
        }
    }
}
