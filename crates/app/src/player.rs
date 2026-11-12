//! Плеер превью: libmpv рисует кадр исходника в текстуру композитора, egui её показывает.
//!
//! С mpv — только асинхронно (см. `sc-mpv`): состояние ниже обновляется событиями в [`Player::pump`],
//! команды уходят без ожидания. `pump` — в начале кадра UI, `end_frame` — в конце.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use eframe::egui;
use eframe::glow;
use raw_window_handle::{HasDisplayHandle, RawDisplayHandle};
use sc_compositor::Compositor;
use sc_core::Template;
use sc_mpv::{Event, Format, Mpv, Prop};

/// Последние известные значения свойств mpv.
#[derive(Debug, Clone, Default)]
pub struct State {
    pub time: f64,
    pub duration: f64,
    pub paused: bool,
    /// Размер кадра с учётом поворота и пропорций пикселя; (0, 0) — ещё неизвестен.
    pub size: (u32, u32),
    /// Ошибка открытия последнего файла.
    pub error: Option<String>,
}

pub struct Player {
    mpv: Mpv,
    comp: Compositor,
    gl: Arc<glow::Context>,
    pub state: State,
    video_tex: Option<egui::TextureId>,
    background_tex: Option<egui::TextureId>,
    loaded: Option<PathBuf>,
    /// Петля воспроизведения [начало, конец) — выбранный шортс.
    range: Option<(f64, f64)>,
}

impl Player {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Result<Self, String> {
        let gl = cc.gl.clone().ok_or("нет GL-контекста (нужен рендерер glow)")?;
        let get_proc = cc.get_proc_address.clone().ok_or("нет get_proc_address")?;
        let wl_display = match cc.display_handle().map(|h| h.as_raw()) {
            Ok(RawDisplayHandle::Wayland(h)) => h.display.as_ptr(),
            _ => std::ptr::null_mut(),
        };
        let options = [
            ("vo", "libmpv"),
            ("hwdec", "vaapi"),
            ("hr-seek", "yes"),
            ("hr-seek-framedrop", "no"),
            ("keep-open", "yes"),
            ("idle", "yes"),
            ("pause", "yes"),
            ("terminal", "no"),
            // Без конфигов и lua-скриптов пользователя (osc и т.п. поднялись бы внутри приложения).
            ("config", "no"),
            ("load-scripts", "no"),
            ("osc", "no"),
            ("input-default-bindings", "no"),
            ("input-vo-keyboard", "no"),
        ];
        let ctx = cc.egui_ctx.clone();
        let mpv = Mpv::new(&options, get_proc, wl_display, Arc::new(move || ctx.request_repaint()))
            .map_err(|e| e.to_string())?;
        for (name, f) in [
            ("time-pos", Format::Double),
            ("duration", Format::Double),
            ("pause", Format::Flag),
            ("dwidth", Format::Int),
            ("dheight", Format::Int),
        ] {
            mpv.observe(name, f);
        }
        let comp = unsafe { Compositor::new(&gl)? };
        Ok(Self {
            mpv,
            comp,
            gl,
            state: State { paused: true, ..Default::default() },
            video_tex: None,
            background_tex: None,
            loaded: None,
            range: None,
        })
    }

    /// Открыть файл на паузе с начала. Повторный вызов с тем же путём ничего не делает.
    pub fn load(&mut self, path: &Path) {
        if self.loaded.as_deref() == Some(path) {
            return;
        }
        self.clear_range();
        self.mpv.set("pause", "yes");
        self.mpv.command(&["loadfile", &path.to_string_lossy()]);
        self.loaded = Some(path.to_owned());
        self.state = State { paused: true, ..Default::default() };
    }

    pub fn unload(&mut self) {
        if self.loaded.take().is_some() {
            self.clear_range();
            self.mpv.command(&["stop"]);
            self.state = State { paused: true, ..Default::default() };
        }
    }

    pub fn is_ready(&self) -> bool {
        self.loaded.is_some() && self.state.duration > 0.0
    }

    pub fn set_paused(&mut self, paused: bool) {
        if self.loaded.is_some() && self.state.paused != paused {
            self.mpv.set("pause", if paused { "yes" } else { "no" });
            self.state.paused = paused;
        }
    }

    pub fn toggle_pause(&mut self) {
        // В конце петли/файла с keep-open «играть» означает «сначала».
        if self.state.paused && self.state.duration > 0.0 {
            let end = self.range.map_or(self.state.duration, |r| r.1);
            if self.state.time >= end - 0.05 {
                self.seek(self.range.map_or(0.0, |r| r.0), true);
            }
        }
        self.set_paused(!self.state.paused);
    }

    /// `exact = false` — по ключевым кадрам (для перетаскивания, быстро); `true` — точно до кадра.
    pub fn seek(&mut self, t: f64, exact: bool) {
        if self.loaded.is_none() {
            return;
        }
        let t = t.clamp(0.0, self.state.duration.max(0.0));
        let mode = if exact { "absolute+exact" } else { "absolute+keyframes" };
        self.mpv.command(&["seek", &format!("{t:.3}"), mode]);
        // Сразу показываем цель — событие с настоящим time-pos придёт через 40–90 мс.
        self.state.time = t;
    }

    pub fn frame_step(&mut self, forward: bool) {
        if self.loaded.is_some() {
            self.mpv.command(&[if forward { "frame-step" } else { "frame-back-step" }]);
        }
    }

    /// Играть по кругу отрезок [start, end) (граница `end` исключающая, петля бесшовная).
    pub fn set_range(&mut self, start: f64, end: f64) {
        if self.range == Some((start, end)) {
            return;
        }
        self.mpv.set("ab-loop-a", &format!("{start:.3}"));
        self.mpv.set("ab-loop-b", &format!("{end:.3}"));
        self.range = Some((start, end));
    }

    pub fn clear_range(&mut self) {
        if self.range.take().is_some() {
            self.mpv.set("ab-loop-a", "no");
            self.mpv.set("ab-loop-b", "no");
        }
    }

    pub fn range(&self) -> Option<(f64, f64)> {
        self.range
    }

    pub fn video_texture(&self) -> Option<egui::TextureId> {
        self.video_tex.filter(|_| self.loaded.is_some() && self.comp.src_size().is_some())
    }

    pub fn background_texture(&self) -> Option<egui::TextureId> {
        self.background_tex.filter(|_| self.comp.has_background() && self.video_texture().is_some())
    }

    pub fn src_size(&self) -> Option<(u32, u32)> {
        self.comp.src_size()
    }

    /// События mpv и GL-работа (кадр → текстура → мипмапы → фон). В начале каждого кадра UI.
    pub fn pump(&mut self, frame: &mut eframe::Frame, template: &Template) {
        while let Some(ev) = self.mpv.poll_event() {
            match ev {
                Event::Property { name, value } => self.on_property(&name, value),
                Event::EndFile { error: Some(e) } => {
                    log::error!("mpv не открыл {:?}: {e}", self.loaded);
                    self.state.error = Some(e);
                }
                Event::FileLoaded => self.state.error = None,
                _ => {}
            }
        }

        let (w, h) = self.state.size;
        match unsafe { self.comp.ensure_video(&self.gl, w, h) } {
            Ok(Some(tex)) => self.video_tex = Some(frame.register_native_glow_texture(tex)),
            Ok(None) => {}
            Err(e) => log::error!("текстура кадра {w}×{h}: {e}"),
        }
        let Some((fbo, fw, fh)) = self.comp.video_target() else { return };
        if !self.mpv.wants_frame() {
            return;
        }
        self.mpv.render(fbo, fw, fh);
        let result = unsafe { self.comp.frame_rendered(&self.gl, template) };
        unsafe {
            use glow::HasContext;
            self.gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        }
        match result {
            Ok(Some(tex)) => self.background_tex = Some(frame.register_native_glow_texture(tex)),
            Ok(None) => {}
            Err(e) => log::error!("фон превью: {e}"),
        }
    }

    fn on_property(&mut self, name: &str, value: Option<Prop>) {
        let f = |v: &Option<Prop>| match v {
            Some(Prop::Double(x)) => *x,
            Some(Prop::Int(x)) => *x as f64,
            _ => 0.0,
        };
        match name {
            "time-pos" => {
                if let Some(Prop::Double(t)) = value {
                    self.state.time = t;
                }
            }
            "duration" => self.state.duration = f(&value),
            "pause" => self.state.paused = !matches!(value, Some(Prop::Flag(false))),
            "dwidth" => self.state.size.0 = f(&value) as u32,
            "dheight" => self.state.size.1 = f(&value) as u32,
            _ => {}
        }
    }

    /// В конце каждого кадра UI: mpv ждёт подтверждения показа.
    pub fn end_frame(&self) {
        self.mpv.report_swap();
    }

    /// Освободить mpv и GL-ресурсы. Вызывать при текущем GL-контексте (`on_exit`).
    pub fn destroy(mut self, gl: &glow::Context) {
        // render context mpv освобождается первым — в Drop, пока контекст текущий.
        drop(self.mpv);
        unsafe { self.comp.delete(gl) };
    }
}
