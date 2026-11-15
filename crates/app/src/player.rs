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
use sc_compositor::{ArtSlot, Compositor};
use sc_raster::LayerArt;
use sc_core::{Background, Canvas, Layer, SubWord, SubtitleMode, SubtitleTrack, Template};
use sc_mpv::{Event, Format, GetProcAddress, Mpv, Prop};

use crate::raster_worker::{RasterWorker, Rastered};
use crate::subtitle_worker::{SubKey, SubtitleWorker};

/// Сколько готовых растров субтитров держать (текущие и упреждение).
const SUB_CACHE: usize = 48;
/// Сколько следующих состояний субтитров рисовать заранее.
const SUB_AHEAD: usize = 4;

/// Слова субтитров для превью: шортса или образец (когда речи нет), `fixed_time` — показать момент образца,
/// а не время плеера.
#[derive(Clone)]
pub struct SubtitleInput {
    pub words: Arc<Vec<SubWord>>,
    pub fixed_time: Option<f64>,
}

impl SubtitleInput {
    fn same(&self, o: &SubtitleInput) -> bool {
        Arc::ptr_eq(&self.words, &o.words) && self.fixed_time == o.fixed_time
    }
}

/// Субтитры превью: дорожки по раскладке слоёв, готовые растры, что сейчас лежит в композиторе.
#[derive(Default)]
struct Subs {
    input: Option<SubtitleInput>,
    /// Раскладки слов текущего `input`: (слов в строке, строк) → дорожка.
    tracks: Vec<((u32, u32), Arc<SubtitleTrack>)>,
    cache: Vec<(SubKey, Arc<LayerArt>)>,
    /// Подмены в композиторе: индекс слоя → что нарисовано.
    gpu: Vec<(usize, SubKey)>,
    requested: Vec<SubKey>,
}

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
    /// Шаблон, по которому собирается холст.
    template: Option<Template>,
    /// Растры считаются в фоне; `ready` — последний готовый результат.
    raster: RasterWorker,
    ready: Option<Rastered>,
    /// Последний отправленный на растеризацию шаблон.
    requested: Option<Template>,
    /// Для каких слоёв (ключ `raster_key`) нарисованы растры, лежащие в композиторе, по слоям шаблона;
    /// ключ может быть устаревшим — тогда композитор растягивает старый растр, пока не готов новый.
    gpu_layers: Vec<Option<Layer>>,
    gpu_bg: Option<Background>,
    gpu_canvas: Option<Canvas>,
    /// Поколение шрифтов, при котором нарисованы растры в композиторе.
    fonts: u64,
    /// Размер холста на экране в пикселях (из прошлого кадра UI).
    canvas_size: (u32, u32),
    canvas_tex: Option<egui::TextureId>,
    /// Холст надо пересобрать: новый кадр, шаблон или размер.
    dirty: bool,
    subs: Subs,
    sub_worker: SubtitleWorker,
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
            raster: RasterWorker::new(cc.egui_ctx.clone()),
            ready: None,
            requested: None,
            gpu_layers: Vec::new(),
            gpu_bg: None,
            gpu_canvas: None,
            fonts: sc_raster::fonts::generation(),
            canvas_size: (0, 0),
            canvas_tex: None,
            dirty: true,
            subs: Subs::default(),
            sub_worker: SubtitleWorker::new(cc.egui_ctx.clone()),
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

    /// Слова субтитров (шортса или образец); `None` — субтитров не показывать.
    pub fn set_subtitles(&mut self, input: Option<SubtitleInput>) {
        let same = match (&self.subs.input, &input) {
            (Some(a), Some(b)) => a.same(b),
            (None, None) => true,
            _ => false,
        };
        if !same {
            self.subs.input = input;
            self.subs.tracks.clear();
            self.dirty = true;
        }
    }

    /// Дорожка текущих слов в раскладке слоя.
    fn sub_track(&mut self, words_per_line: u32, lines: u32) -> Option<Arc<SubtitleTrack>> {
        let input = self.subs.input.as_ref()?;
        if let Some((_, t)) = self.subs.tracks.iter().find(|(k, _)| *k == (words_per_line, lines)) {
            return Some(t.clone());
        }
        let t = Arc::new(SubtitleTrack::new(input.words.as_ref().clone(), words_per_line, lines));
        self.subs.tracks.push(((words_per_line, lines), t.clone()));
        Some(t)
    }

    /// Субтитры в момент воспроизведения: нужный растр каждого слоя субтитров — в композитор (если готов;
    /// пока нет — остаётся прошлый), нужное и следующие состояния — в фоновый поток.
    fn pump_subtitles(&mut self, template: &Template) {
        for (key, art) in self.sub_worker.poll() {
            if !self.subs.cache.iter().any(|(k, _)| k.same(&key)) {
                self.subs.cache.push((key, art));
            }
        }
        let fonts = sc_raster::fonts::generation();
        let t = self.subs.input.as_ref().map(|i| i.fixed_time.unwrap_or(self.state.time));
        let mut wanted: Vec<SubKey> = Vec::new();
        let mut layers: Vec<usize> = Vec::new();
        for (i, layer) in template.layers.iter().enumerate() {
            let &Layer::Subtitles { words_per_line, lines, mode, .. } = layer else { continue };
            layers.push(i);
            let (Some(t), Some(track)) = (t, self.sub_track(words_per_line, lines)) else {
                self.set_sub_overlay(i, None);
                continue;
            };
            let key_at = |at: f64| {
                track.at(at).map(|(phrase, spoken)| SubKey {
                    layer: layer.clone(),
                    canvas: template.canvas,
                    track: track.clone(),
                    phrase,
                    spoken: if mode == SubtitleMode::Static { 0 } else { spoken },
                    fonts,
                })
            };
            let Some(now) = key_at(t) else {
                self.set_sub_overlay(i, None);
                continue;
            };
            let ready = self.subs.cache.iter().find(|(k, _)| k.same(&now)).map(|(_, a)| a.clone());
            let shown = self.subs.gpu.iter().any(|(j, k)| *j == i && k.same(&now));
            match ready {
                Some(art) if !shown => self.set_sub_overlay(i, Some((now.clone(), &art))),
                Some(_) => {}
                None => wanted.push(now.clone()),
            }
            // Упреждение — только при воспроизведении настоящих слов.
            if self.subs.input.as_ref().is_some_and(|i| i.fixed_time.is_none()) {
                for at in track.changes_after(t).take(SUB_AHEAD).collect::<Vec<_>>() {
                    if let Some(k) = key_at(at + 1e-3)
                        && !k.same(&now)
                        && !wanted.iter().any(|w| w.same(&k))
                        && !self.subs.cache.iter().any(|(c, _)| c.same(&k))
                    {
                        wanted.push(k);
                    }
                }
            }
        }
        // Подмены слоёв, которые больше не субтитры (удалены, сдвинуты в списке).
        let stale: Vec<usize> = self.subs.gpu.iter().map(|(j, _)| *j).filter(|j| !layers.contains(j)).collect();
        for j in stale {
            self.set_sub_overlay(j, None);
        }
        // Кэш: самые старые уходят первыми, показанные сейчас — остаются.
        while self.subs.cache.len() > SUB_CACHE {
            let gpu = &self.subs.gpu;
            match self.subs.cache.iter().position(|(k, _)| !gpu.iter().any(|(_, g)| g.same(k))) {
                Some(p) => {
                    self.subs.cache.remove(p);
                }
                None => break,
            }
        }
        let changed = wanted.len() != self.subs.requested.len() || wanted.iter().zip(&self.subs.requested).any(|(a, b)| !a.same(b));
        if changed {
            if !wanted.is_empty() {
                self.sub_worker.request(wanted.clone());
            }
            self.subs.requested = wanted;
        }
    }

    fn set_sub_overlay(&mut self, i: usize, art: Option<(SubKey, &LayerArt)>) {
        let had = self.subs.gpu.iter().any(|(j, _)| *j == i);
        if art.is_none() && !had {
            return;
        }
        self.subs.gpu.retain(|(j, _)| *j != i);
        let res = match art {
            Some((key, art)) => {
                let r = unsafe { self.comp.set_overlay(&self.gl, i, Some((key.layer.rect(), art))) };
                self.subs.gpu.push((i, key));
                r
            }
            None => unsafe { self.comp.set_overlay(&self.gl, i, None) },
        };
        if let Err(e) = res {
            log::error!("растр субтитров: {e}");
        }
        self.dirty = true;
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
        let fonts = sc_raster::fonts::generation();
        if self.fonts != fonts {
            // Подключён шрифт: текст мог быть нарисован запасным — растры слоёв считаются заново.
            self.fonts = fonts;
            self.ready = None;
            self.requested = None;
            self.gpu_layers.clear();
            self.sync_art(template);
            self.dirty = true;
        }
        if let Some(r) = self.raster.poll().filter(|r| r.fonts == fonts) {
            self.ready = Some(r);
            self.sync_art(template);
            self.dirty = true;
        }
        self.pump_bg();
        self.pump_subtitles(template);

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
        self.sync_art(template);
        if let Err(e) = unsafe { self.comp.refresh_background(&self.gl, template) } {
            log::error!("фон превью: {e}");
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

    /// Растры в композиторе — под слои `template`: готовые остаются, свежие из фонового потока загружаются,
    /// для остальных растягивается прошлый растр того же слоя; чего не хватает — запрашивается в фоне.
    fn sync_art(&mut self, template: &Template) {
        let canvas = template.canvas;
        if self.gpu_canvas != Some(canvas) {
            // Растры другого размера холста не годятся даже растянутыми.
            self.gpu_layers.clear();
            self.gpu_bg = None;
            self.gpu_canvas = Some(canvas);
        }
        let ready = self.ready.as_ref();
        if self.gpu_bg.as_ref() != Some(&template.background)
            && let Some(bitmap) = ready.and_then(|r| r.background(canvas, &template.background))
        {
            if let Err(e) = unsafe { self.comp.set_background_art(&self.gl, bitmap) } {
                log::error!("растр фона: {e}");
            }
            self.gpu_bg = Some(template.background.clone());
        }

        let keys: Vec<Layer> = template.layers.iter().map(sc_raster::raster_key).collect();
        let old = &self.gpu_layers;
        let mut used = vec![false; old.len()];
        let mut slots = Vec::with_capacity(keys.len());
        let mut gpu: Vec<Option<Layer>> = Vec::with_capacity(keys.len());
        for k in &keys {
            if let Some(j) = (0..old.len()).find(|&j| !used[j] && old[j].as_ref() == Some(k)) {
                used[j] = true;
                slots.push(ArtSlot::Keep(j));
                gpu.push(Some(k.clone()));
            } else if let Some(art) = ready.and_then(|r| r.layer(canvas, k)) {
                slots.push(ArtSlot::Upload(k.rect(), art));
                gpu.push(Some(k.clone()));
            } else {
                slots.push(ArtSlot::Empty);
                gpu.push(None);
            }
        }
        // Слой, который сейчас двигают: старый растр на том же месте в списке, растянутый на новый прямоугольник.
        for (i, slot) in slots.iter_mut().enumerate() {
            if gpu[i].is_none()
                && let Some(Some(prev)) = old.get(i)
                && !used[i]
                && std::mem::discriminant(prev) == std::mem::discriminant(&keys[i])
            {
                used[i] = true;
                *slot = ArtSlot::Keep(i);
                gpu[i] = Some(prev.clone());
            }
        }
        if let Err(e) = unsafe { self.comp.set_layers(&self.gl, &slots) } {
            log::error!("растры шаблона: {e}");
        }
        let complete = gpu.iter().zip(&keys).all(|(g, k)| g.as_ref() == Some(k)) && self.gpu_bg.as_ref() == Some(&template.background);
        self.gpu_layers = gpu;
        if !complete && self.requested.as_ref() != Some(template) {
            self.raster.request(template.clone());
            self.requested = Some(template.clone());
        }
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
