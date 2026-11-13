//! Плеер превью: libmpv рисует кадр исходника (и видео-фона, если он в шаблоне) в текстуры композитора,
//! композитор собирает холст по шаблону в одну текстуру, egui её показывает.
//!
//! С mpv — только асинхронно (см. `sc-mpv`): состояние ниже обновляется событиями в [`Player::pump`],
//! команды уходят без ожидания. `pump` — в начале кадра UI, `end_frame` — в конце.
//! Размер холста на экране UI сообщает через [`Player::request_canvas`]; холст пересобирается в следующем `pump`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use eframe::egui;
use eframe::glow;
use raw_window_handle::{HasDisplayHandle, RawDisplayHandle};
use sc_compositor::Compositor;
use sc_core::{Background, Template};
use sc_mpv::{Event, Format, GetProcAddress, Mpv, Prop};
use sc_raster::Art;

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

/// Второй экземпляр mpv — видео-фон шаблона (по кругу, без звука, играет вместе с основным).
struct BgVideo {
    mpv: Mpv,
    path: PathBuf,
    size: (u32, u32),
    paused: bool,
}

pub struct Player {
    mpv: Mpv,
    comp: Compositor,
    gl: Arc<glow::Context>,
    get_proc: GetProcAddress,
    wl_display: *mut std::ffi::c_void,
    ctx: egui::Context,
    pub state: State,
    bg: Option<BgVideo>,
    loaded: Option<PathBuf>,
    /// Петля воспроизведения [начало, конец) — выбранный шортс.
    range: Option<(f64, f64)>,
    /// Шаблон, по которому растрированы слои (растры пересчитываются при его изменении).
    template: Option<Template>,
    /// Размер холста на экране в пикселях (из прошлого кадра UI).
    canvas_size: (u32, u32),
    canvas_tex: Option<egui::TextureId>,
    /// Холст надо пересобрать: новый кадр, шаблон или размер.
    dirty: bool,
}

/// Опции mpv для обоих экземпляров.
const MPV_OPTIONS: [(&str, &str); 13] = [
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

impl Player {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Result<Self, String> {
        let gl = cc.gl.clone().ok_or("нет GL-контекста (нужен рендерер glow)")?;
        let get_proc = cc.get_proc_address.clone().ok_or("нет get_proc_address")?;
        let wl_display = match cc.display_handle().map(|h| h.as_raw()) {
            Ok(RawDisplayHandle::Wayland(h)) => h.display.as_ptr(),
            _ => std::ptr::null_mut(),
        };
        let ctx = cc.egui_ctx.clone();
        let wake = ctx.clone();
        let mpv = Mpv::new(&MPV_OPTIONS, get_proc.clone(), wl_display, Arc::new(move || wake.request_repaint()))
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
            get_proc,
            wl_display,
            ctx,
            state: State { paused: true, ..Default::default() },
            bg: None,
            loaded: None,
            range: None,
            template: None,
            canvas_size: (0, 0),
            canvas_tex: None,
            dirty: true,
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

    /// Собранный холст, если исходник открыт и его кадр уже есть.
    pub fn canvas_texture(&self) -> Option<egui::TextureId> {
        self.canvas_tex.filter(|_| self.loaded.is_some() && self.comp.src_size().is_some())
    }

    /// Размер холста на экране в физических пикселях; холст пересоберётся в следующем `pump`.
    pub fn request_canvas(&mut self, w: u32, h: u32) {
        if self.canvas_size != (w, h) {
            self.canvas_size = (w, h);
            self.dirty = true;
            self.ctx.request_repaint();
        }
    }

    /// События mpv и GL-работа (кадры → текстуры → фон → холст). В начале каждого кадра UI.
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
        if self.template.as_ref() != Some(template) {
            self.apply_template(template);
        }
        self.pump_bg();

        let gl = self.gl.clone();
        let (w, h) = self.state.size;
        if let Err(e) = unsafe { self.comp.ensure_video(&gl, w, h) } {
            log::error!("текстура кадра {w}×{h}: {e}");
        }
        if let Some((fbo, fw, fh)) = self.comp.video_target()
            && self.mpv.wants_frame()
        {
            self.mpv.render(fbo, fw, fh);
            if let Err(e) = unsafe { self.comp.video_rendered(&gl, template) } {
                log::error!("фон превью: {e}");
            }
            self.dirty = true;
        }
        if let Some(bg) = &self.bg
            && let Some((fbo, fw, fh)) = self.comp.bg_video_target()
            && bg.mpv.wants_frame()
        {
            bg.mpv.render(fbo, fw, fh);
            unsafe { self.comp.bg_video_rendered(&gl) };
            self.dirty = true;
        }

        let (cw, ch) = self.canvas_size;
        if self.dirty && cw > 0 && ch > 0 && self.comp.has_video() {
            match unsafe { self.comp.compose(&gl, template, cw, ch) } {
                Ok((tex, recreated)) => {
                    if recreated || self.canvas_tex.is_none() {
                        self.canvas_tex = Some(frame.register_native_glow_texture(tex));
                    }
                    self.dirty = false;
                }
                Err(e) => log::error!("холст превью {cw}×{ch}: {e}"),
            }
        }
        unsafe {
            use glow::HasContext;
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        }
    }

    /// Шаблон изменился: растры слоёв, размытый фон, видео-фон.
    fn apply_template(&mut self, template: &Template) {
        // Ошибка одного растра (нет файла картинки) не мешает остальным: превью показывает, что может.
        let canvas = template.canvas;
        let background = sc_raster::background(&template.background, canvas).unwrap_or_else(|e| {
            log::error!("фон шаблона: {e}");
            None
        });
        let layers = template
            .layers
            .iter()
            .map(|l| {
                sc_raster::layer(l, canvas).unwrap_or_else(|e| {
                    log::error!("слой шаблона: {e}");
                    Default::default()
                })
            })
            .collect();
        let art = Art { background, layers };
        unsafe {
            if let Err(e) = self.comp.set_art(&self.gl, &art) {
                log::error!("растры шаблона: {e}");
            }
            if let Err(e) = self.comp.refresh_background(&self.gl, template) {
                log::error!("фон превью: {e}");
            }
        }

        let bg_path = match &template.background {
            Background::Video { path, .. } => Some(path.clone()),
            _ => None,
        };
        if self.bg.as_ref().map(|b| &b.path) != bg_path.as_ref() {
            self.bg = None;
            if let Some(path) = bg_path {
                self.bg = self.open_bg(path);
            }
        }
        self.template = Some(template.clone());
        self.dirty = true;
    }

    fn open_bg(&self, path: PathBuf) -> Option<BgVideo> {
        let wake = self.ctx.clone();
        let mut options = MPV_OPTIONS.to_vec();
        options.extend([("loop-file", "inf"), ("aid", "no"), ("hr-seek", "no")]);
        let mpv = Mpv::new(&options, self.get_proc.clone(), self.wl_display, Arc::new(move || wake.request_repaint()))
            .map_err(|e| log::error!("mpv видео-фона: {e}"))
            .ok()?;
        mpv.observe("dwidth", Format::Int);
        mpv.observe("dheight", Format::Int);
        mpv.command(&["loadfile", &path.to_string_lossy()]);
        Some(BgVideo { mpv, path, size: (0, 0), paused: true })
    }

    /// События видео-фона, его текстура и пауза вместе с основным плеером.
    fn pump_bg(&mut self) {
        if let Some(bg) = &mut self.bg {
            while let Some(ev) = bg.mpv.poll_event() {
                match ev {
                    Event::Property { name, value: Some(Prop::Int(v)) } if name == "dwidth" => bg.size.0 = v as u32,
                    Event::Property { name, value: Some(Prop::Int(v)) } if name == "dheight" => bg.size.1 = v as u32,
                    Event::EndFile { error: Some(e) } => log::error!("mpv не открыл видео-фон {:?}: {e}", bg.path),
                    _ => {}
                }
            }
            if bg.paused != self.state.paused {
                bg.mpv.set("pause", if self.state.paused { "yes" } else { "no" });
                bg.paused = self.state.paused;
            }
        }
        let size = self.bg.as_ref().map(|b| b.size);
        if let Err(e) = unsafe { self.comp.ensure_bg_video(&self.gl, size) } {
            log::error!("текстура видео-фона: {e}");
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
        if let Some(bg) = &self.bg {
            bg.mpv.report_swap();
        }
    }

    /// Освободить mpv и GL-ресурсы. Вызывать при текущем GL-контексте (`on_exit`).
    pub fn destroy(mut self, gl: &glow::Context) {
        // render context mpv освобождается первым — в Drop, пока контекст текущий.
        drop(self.bg.take());
        drop(self.mpv);
        unsafe { self.comp.delete(gl) };
    }
}
