//! Спайк 0.5: превью шортса в egui.
//! mpv (render API) рисует кадр в нашу текстуру → egui показывает исходник с перетаскиваемым кропом
//! и холст 9:16 (размытый фон + кроп со скруглением). Режим `--bench` снимает цифры и закрывается.
//!
//! preview-spike <видео> [--bench СЕК] [--hwdec vaapi|no] [--blur-div N] [--sigma PX] [--fbo-max-h PX] [--out файл.json]

mod gl;
mod mpv;

use std::time::Instant;

use eframe::egui::{self, Color32, Pos2, Rect, Vec2};
use eframe::glow::{self, HasContext};
use raw_window_handle::{HasDisplayHandle, RawDisplayHandle};

const CANVAS_W: f32 = 1080.0;
const CANVAS_H: f32 = 1920.0;

#[derive(Clone)]
struct Args {
    file: String,
    bench: Option<f64>,
    hwdec: String,
    blur_div: i32,
    sigma: f32,
    fbo_max_h: i32,
    out: Option<String>,
    /// Доп. опции mpv: --mpv ключ=значение
    extra: Vec<(String, String)>,
}

fn parse_args() -> Args {
    let mut a = Args {
        file: String::new(),
        bench: None,
        hwdec: "vaapi".into(),
        blur_div: 4,
        sigma: 40.0,
        fbo_max_h: 0,
        out: None,
        extra: vec![],
    };
    let mut it = std::env::args().skip(1);
    while let Some(x) = it.next() {
        match x.as_str() {
            "--bench" => a.bench = it.next().and_then(|v| v.parse().ok()),
            "--hwdec" => a.hwdec = it.next().unwrap(),
            "--blur-div" => a.blur_div = it.next().unwrap().parse().unwrap(),
            "--sigma" => a.sigma = it.next().unwrap().parse().unwrap(),
            "--fbo-max-h" => a.fbo_max_h = it.next().unwrap().parse().unwrap(),
            "--out" => a.out = it.next(),
            "--mpv" => {
                let kv = it.next().unwrap();
                let (k, v) = kv.split_once('=').unwrap();
                a.extra.push((k.into(), v.into()));
            }
            _ => a.file = x,
        }
    }
    assert!(!a.file.is_empty(), "не указан файл");
    a
}

/// utime + stime процесса, секунды.
fn cpu_seconds() -> f64 {
    let s = std::fs::read_to_string("/proc/self/stat").unwrap_or_default();
    let rest = s.rsplit_once(')').map(|x| x.1).unwrap_or("");
    let f: Vec<&str> = rest.split_whitespace().collect();
    let tick = |i: usize| f.get(i).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
    // после ')' : state=0 … utime=11, stime=12
    (tick(11) + tick(12)) / 100.0
}

fn pct(v: &mut [f64], p: f64) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[((v.len() - 1) as f64 * p).round() as usize]
}

fn mean(v: &[f64]) -> f64 {
    v.iter().sum::<f64>() / v.len().max(1) as f64
}

/// Последние значения наблюдаемых свойств mpv (приходят событиями, без синхронных запросов).
#[derive(Default)]
struct Props(std::collections::HashMap<String, mpv::Prop>);

impl Props {
    fn f64(&self, n: &str) -> Option<f64> {
        match self.0.get(n)? {
            mpv::Prop::Double(v) => Some(*v),
            mpv::Prop::Int(v) => Some(*v as f64),
            _ => None,
        }
    }
    fn i64(&self, n: &str) -> Option<i64> {
        match self.0.get(n)? {
            mpv::Prop::Int(v) => Some(*v),
            _ => None,
        }
    }
    fn flag(&self, n: &str) -> Option<bool> {
        match self.0.get(n)? {
            mpv::Prop::Flag(v) => Some(*v),
            _ => None,
        }
    }
    fn str(&self, n: &str) -> Option<String> {
        match self.0.get(n)? {
            mpv::Prop::Str(v) => Some(v.clone()),
            _ => None,
        }
    }
}

#[derive(Default)]
struct Window {
    t0: Option<Instant>,
    ui_frames: u64,
    video_frames: u64,
    gpu_mpv: Vec<f64>,
    gpu_blur: Vec<f64>,
    cpu0: f64,
    drops0: i64,
    dec_drops0: i64,
    mistimed0: i64,
}

enum Phase {
    WaitLoad,
    Warmup(Instant),
    Play(Instant),
    Seeks { i: usize, sent: Option<Instant>, target: f64, restarted: Option<Instant>, frame: Option<Instant>, settle: u32 },
    Segment { started: Instant },
    Done,
}

struct Bench {
    secs: f64,
    phase: Phase,
    win: Window,
    report: Vec<String>,
    seek_lat_restart: Vec<f64>,
    seek_lat_frame: Vec<f64>,
    seek_err: Vec<f64>,
    seg_in: f64,
    seg_out: f64,
    seg_last_pos: f64,
    seg_max_pos: f64,
    seg_starts: Vec<f64>,
    seg_ends: Vec<f64>,
    seg_last_frame_at: Option<Instant>,
    seg_gaps: Vec<f64>,
}

struct App {
    args: Args,
    mpv: Option<mpv::Mpv>,
    props: Props,
    video: Option<gl::Target>,
    video_id: Option<egui::TextureId>,
    blur: gl::Blur,
    blur_id: Option<egui::TextureId>,
    stamps: gl::GpuStamps,
    src_size: (i32, i32),
    duration: f64,
    // кроп в нормированных координатах исходника
    crop: Rect,
    crop_aspect: f32,
    sigma: f32,
    blur_div: i32,
    dim: f32,
    seg_in: f64,
    seg_out: f64,
    scrub: f64,
    // статистика для строки состояния
    fps_t: Instant,
    fps_ui: u32,
    fps_video: u32,
    shown: (f32, f32),
    gpu_last: (f64, f64),
    bench: Option<Bench>,
    frame_rendered: bool,
    log: Vec<String>,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, args: Args) -> Self {
        let glc = cc.gl.as_ref().expect("нужен glow").clone();
        let wl_display = match cc.display_handle().map(|h| h.as_raw()) {
            Ok(RawDisplayHandle::Wayland(h)) => h.display.as_ptr(),
            _ => std::ptr::null_mut(),
        };
        let mut log = vec![format!("GL: {} / {}", unsafe { glc.get_parameter_string(glow::VERSION) }, unsafe {
            glc.get_parameter_string(glow::RENDERER)
        })];
        eprintln!("{}", log[0]);
        log.push(format!("Wayland display: {}", !wl_display.is_null()));

        let mut opts: Vec<(&str, &str)> = vec![
            ("vo", "libmpv"),
            ("hwdec", &args.hwdec),
            ("hr-seek", "yes"),
            ("hr-seek-framedrop", "no"),
            ("keep-open", "yes"),
            ("idle", "yes"),
            ("terminal", "yes"),
            ("msg-level", "all=warn,vo=v"),
            ("config", "no"),
            ("load-scripts", "no"),
            ("osc", "no"),
            ("input-default-bindings", "no"),
        ];
        if args.bench.is_some() {
            opts.push(("mute", "yes"));
            opts.push(("pause", "yes"));
        }
        for (k, v) in &args.extra {
            opts.push((k, v));
        }
        let m = mpv::Mpv::new(&opts, cc.get_proc_address.clone().unwrap(), wl_display, &cc.egui_ctx)
            .unwrap_or_else(|e| panic!("mpv: {e}"));
        use mpv::Format as F;
        for (n, f) in [
            ("time-pos", F::Double),
            ("duration", F::Double),
            ("pause", F::Flag),
            ("dwidth", F::Int),
            ("dheight", F::Int),
            ("hwdec-current", F::Str),
            ("container-fps", F::Double),
            ("display-fps", F::Double),
            ("frame-drop-count", F::Int),
            ("decoder-frame-drop-count", F::Int),
            ("mistimed-frame-count", F::Int),
        ] {
            m.observe(n, f);
        }
        m.command(&["loadfile", &args.file]);

        let (blur, stamps) = unsafe { (gl::Blur::new(&glc), gl::GpuStamps::new(&glc)) };
        let bench = args.bench.map(|secs| Bench {
            secs,
            phase: Phase::WaitLoad,
            win: Window::default(),
            report: vec![],
            seek_lat_restart: vec![],
            seek_lat_frame: vec![],
            seek_err: vec![],
            seg_in: 10.0,
            seg_out: 13.0,
            seg_last_pos: 0.0,
            seg_max_pos: 0.0,
            seg_starts: vec![],
            seg_ends: vec![],
            seg_last_frame_at: None,
            seg_gaps: vec![],
        });
        Self {
            sigma: args.sigma,
            blur_div: args.blur_div,
            args,
            mpv: Some(m),
            props: Props::default(),
            video: None,
            video_id: None,
            blur,
            blur_id: None,
            stamps,
            src_size: (0, 0),
            duration: 0.0,
            crop: Rect::from_min_size(Pos2::new(0.25, 0.1), Vec2::new(0.4, 0.4)),
            crop_aspect: 1.0,
            dim: 0.6,
            seg_in: 10.0,
            seg_out: 13.0,
            scrub: 0.0,
            fps_t: Instant::now(),
            fps_ui: 0,
            fps_video: 0,
            shown: (0.0, 0.0),
            gpu_last: (0.0, 0.0),
            bench,
            frame_rendered: false,
            log,
        }
    }

    fn mpv(&self) -> &mpv::Mpv {
        self.mpv.as_ref().unwrap()
    }

    /// События mpv, GL-работа (кадр mpv → текстура → мипмапы → blur), обновление статистики.
    fn pump(&mut self, frame: &mut eframe::Frame) {
        let glc = frame.gl().unwrap().clone();
        let mut restart = false;
        while let Some(ev) = self.mpv().poll_event() {
            match ev.id {
                mpv::EVENT_PLAYBACK_RESTART => restart = true,
                mpv::EVENT_PROPERTY_CHANGE => {
                    if let Some((name, value)) = ev.prop {
                        match value {
                            Some(v) => self.props.0.insert(name, v),
                            None => self.props.0.remove(&name),
                        };
                    }
                }
                _ => {}
            }
        }
        self.duration = self.props.f64("duration").unwrap_or(0.0);
        let w = self.props.i64("dwidth").unwrap_or(0) as i32;
        let h = self.props.i64("dheight").unwrap_or(0) as i32;
        if w > 0 && h > 0 && (w, h) != self.src_size {
            self.src_size = (w, h);
            let (mut fw, mut fh) = (w, h);
            if self.args.fbo_max_h > 0 && fh > self.args.fbo_max_h {
                fw = (w as i64 * self.args.fbo_max_h as i64 / h as i64) as i32;
                fh = self.args.fbo_max_h;
            }
            if let Some(t) = self.video.take() {
                unsafe { t.delete(&glc) };
            }
            let t = unsafe { gl::Target::new(&glc, fw, fh, true) };
            self.video_id = Some(frame.register_native_glow_texture(t.tex));
            self.video = Some(t);
            self.log.push(format!("видео {w}x{h}, текстура {fw}x{fh}"));
        }
        if let Some(b) = &mut self.bench {
            if restart {
                if let Phase::Seeks { restarted, sent: Some(_), .. } = &mut b.phase {
                    *restarted = Some(Instant::now());
                }
            }
        }

        if let Some(gpu) = unsafe { self.stamps.take(&glc) } {
            self.gpu_last = gpu;
            if let Some(b) = &mut self.bench {
                if matches!(b.phase, Phase::Play(_)) {
                    b.win.gpu_mpv.push(gpu.0);
                    b.win.gpu_blur.push(gpu.1);
                }
            }
        }

        self.frame_rendered = false;
        let Some(video) = &self.video else { return };
        if self.mpv().wants_frame() {
            unsafe {
                self.stamps.mark(&glc, 0);
                self.mpv().render(u32::from(video.fbo.0), video.w, video.h, false);
                video.update_mipmaps(&glc);
                self.stamps.mark(&glc, 1);
                let p = self.blur_params();
                let (tex, recreated) = self.blur.run(&glc, video, &p);
                if recreated || self.blur_id.is_none() {
                    self.blur_id = Some(frame.register_native_glow_texture(tex));
                }
                self.stamps.mark(&glc, 2);
                glc.bind_framebuffer(glow::FRAMEBUFFER, None);
            }
            self.frame_rendered = true;
            self.fps_video += 1;
        }
    }

    /// Фон: исходник «cover» на холст 9:16, уменьшенный в blur_div раз.
    fn blur_params(&self) -> gl::BlurParams {
        let (sw, sh) = (self.src_size.0 as f32, self.src_size.1 as f32);
        let canvas_aspect = CANVAS_W / CANVAS_H;
        let w_uv = (sh * canvas_aspect / sw).min(1.0);
        let h_uv = (sw / canvas_aspect / sh).min(1.0);
        let div = self.blur_div.max(1);
        gl::BlurParams {
            uv_rect: [(1.0 - w_uv) / 2.0, (1.0 - h_uv) / 2.0, w_uv, h_uv],
            out_w: CANVAS_W as i32 / div,
            out_h: CANVAS_H as i32 / div,
            sigma: self.sigma / div as f32,
            dim: self.dim,
        }
    }

    fn src_aspect(&self) -> f32 {
        if self.src_size.1 == 0 { 16.0 / 9.0 } else { self.src_size.0 as f32 / self.src_size.1 as f32 }
    }

    fn clamp_crop(&mut self) {
        let w = self.crop.width().clamp(0.05, 1.0);
        let mut h = w * self.src_aspect() / self.crop_aspect;
        let mut w = w;
        if h > 1.0 {
            w *= 1.0 / h;
            h = 1.0;
        }
        let x = self.crop.min.x.clamp(0.0, 1.0 - w);
        let y = self.crop.min.y.clamp(0.0, 1.0 - h);
        self.crop = Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, h));
    }

    fn seek_exact(&self, t: f64) {
        self.mpv().command(&["seek", &format!("{t:.3}"), "absolute+exact"]);
    }

    fn ui_main(&mut self, ui: &mut egui::Ui) {
        let now = Instant::now();
        if now.duration_since(self.fps_t).as_secs_f32() >= 1.0 {
            let dt = now.duration_since(self.fps_t).as_secs_f32();
            self.shown = (self.fps_ui as f32 / dt, self.fps_video as f32 / dt);
            self.fps_ui = 0;
            self.fps_video = 0;
            self.fps_t = now;
        }
        self.fps_ui += 1;
        let pos = self.props.f64("time-pos").unwrap_or(0.0);
        let paused = self.props.flag("pause").unwrap_or(true);

        egui::Panel::top("stats").show(ui, |ui| {
            ui.label(format!(
                "UI {:.0} к/с · видео {:.0} к/с · GPU: mpv {:.2} мс, blur {:.2} мс · hwdec {} · сброшено {} / декодер {}",
                self.shown.0,
                self.shown.1,
                self.gpu_last.0,
                self.gpu_last.1,
                self.props.str("hwdec-current").unwrap_or_default(),
                self.props.i64("frame-drop-count").unwrap_or(0),
                self.props.i64("decoder-frame-drop-count").unwrap_or(0),
            ));
        });

        egui::Panel::bottom("controls").show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.button(if paused { "▶" } else { "⏸" }).clicked() {
                    self.mpv().command(&["cycle", "pause"]);
                }
                if ui.button("◀ кадр").clicked() {
                    self.mpv().command(&["frame-back-step"]);
                }
                if ui.button("кадр ▶").clicked() {
                    self.mpv().command(&["frame-step"]);
                }
                ui.label(format!("{pos:.3} / {:.1} с", self.duration));
                ui.separator();
                ui.add(egui::DragValue::new(&mut self.seg_in).speed(0.05).prefix("in "));
                if ui.button("[ сюда").clicked() {
                    self.seg_in = pos;
                }
                ui.add(egui::DragValue::new(&mut self.seg_out).speed(0.05).prefix("out "));
                if ui.button("сюда ]").clicked() {
                    self.seg_out = pos;
                }
                if ui.button("▶ отрезок").clicked() {
                    let m = self.mpv();
                    m.set("ab-loop-a", &format!("{:.3}", self.seg_in));
                    m.set("ab-loop-b", &format!("{:.3}", self.seg_out));
                    self.seek_exact(self.seg_in);
                    m.set("pause", "no");
                }
                if ui.button("× отрезок").clicked() {
                    self.mpv().set("ab-loop-a", "no");
                    self.mpv().set("ab-loop-b", "no");
                }
            });
            ui.horizontal(|ui| {
                ui.add(egui::Slider::new(&mut self.sigma, 2.0..=120.0).text("сигма blur, px холста"));
                ui.add(egui::Slider::new(&mut self.blur_div, 1..=8).text("делитель разрешения фона"));
                ui.add(egui::Slider::new(&mut self.dim, 0.2..=1.0).text("яркость фона"));
                egui::ComboBox::from_label("кроп")
                    .selected_text(format!("{:.2}", self.crop_aspect))
                    .show_ui(ui, |ui| {
                        for (n, a) in [("1:1", 1.0), ("4:3", 4.0 / 3.0), ("16:9", 16.0 / 9.0), ("9:16", 9.0 / 16.0)] {
                            ui.selectable_value(&mut self.crop_aspect, a, n);
                        }
                    });
            });
            if self.duration > 0.0 {
                ui.spacing_mut().slider_width = ui.available_width();
                if !ui.ctx().dragged_id().is_some() {
                    self.scrub = pos;
                }
                let r = ui.add_sized(
                    [ui.available_width(), 18.0],
                    egui::Slider::new(&mut self.scrub, 0.0..=self.duration).show_value(false),
                );
                if r.dragged() {
                    // во время перетаскивания — по ключевым кадрам (быстро), по отпусканию — точно
                    self.mpv().command(&["seek", &format!("{:.3}", self.scrub), "absolute+keyframes"]);
                }
                if r.drag_stopped() || (r.changed() && !r.dragged()) {
                    self.seek_exact(self.scrub);
                }
            }
            for l in self.log.iter().rev().take(3) {
                ui.small(l);
            }
        });

        egui::CentralPanel::default().show(ui, |ui| {
            let full = ui.available_rect_before_wrap();
            let canvas_h = full.height();
            let canvas_w = canvas_h * CANVAS_W / CANVAS_H;
            let canvas = Rect::from_min_size(Pos2::new(full.max.x - canvas_w, full.min.y), Vec2::new(canvas_w, canvas_h));
            let src_area = Rect::from_min_max(full.min, Pos2::new(canvas.min.x - 12.0, full.max.y));
            let painter = ui.painter_at(full);
            let uv_full = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
            self.clamp_crop();

            // Исходник с прямоугольником кропа.
            if let Some(id) = self.video_id {
                let a = self.src_aspect();
                let mut size = Vec2::new(src_area.width(), src_area.width() / a);
                if size.y > src_area.height() {
                    size = Vec2::new(src_area.height() * a, src_area.height());
                }
                let view = Rect::from_center_size(src_area.center(), size);
                painter.image(id, view, uv_full, Color32::WHITE);
                let to_screen = |p: Pos2| view.min + Vec2::new(p.x * view.width(), p.y * view.height());
                let crop_scr = Rect::from_min_max(to_screen(self.crop.min), to_screen(self.crop.max));
                let resp = ui.interact(crop_scr, ui.id().with("crop"), egui::Sense::drag());
                if resp.dragged() {
                    let d = resp.drag_delta();
                    self.crop = self.crop.translate(Vec2::new(d.x / view.width(), d.y / view.height()));
                }
                if resp.hovered() {
                    let scroll = ui.input(|i| i.smooth_scroll_delta.y);
                    if scroll != 0.0 {
                        let c = self.crop.center();
                        let w = self.crop.width() * (1.0 - scroll * 0.002);
                        self.crop = Rect::from_center_size(c, Vec2::new(w, self.crop.height()));
                    }
                }
                painter.rect_stroke(crop_scr, 0.0, egui::Stroke::new(2.0, Color32::YELLOW), egui::StrokeKind::Outside);
            }

            // Холст 9:16: размытый фон + кроп со скруглением.
            if let Some(bg) = self.blur_id {
                painter.image(bg, canvas, uv_full, Color32::WHITE);
            }
            if let Some(id) = self.video_id {
                let crop_px_aspect = self.crop_aspect;
                let w = canvas.width() * 0.92;
                let h = w / crop_px_aspect;
                let dst = Rect::from_center_size(canvas.center(), Vec2::new(w, h.min(canvas.height())));
                let r = 36.0 * canvas.width() / CANVAS_W;
                painter.add(egui::epaint::RectShape::filled(dst, r, Color32::WHITE).with_texture(id, self.crop));
            }
            painter.rect_stroke(canvas, 0.0, egui::Stroke::new(1.0, Color32::GRAY), egui::StrokeKind::Outside);
        });
    }

    fn bench_step(&mut self, ctx: &egui::Context) {
        let Some(mut b) = self.bench.take() else { return };
        let now = Instant::now();
        let m = self.mpv.as_ref().unwrap();
        match &mut b.phase {
            Phase::WaitLoad => {
                if self.video.is_some() && self.duration > 0.0 {
                    m.set("pause", "no");
                    b.phase = Phase::Warmup(now);
                }
            }
            Phase::Warmup(t) => {
                if now.duration_since(*t).as_secs_f64() > 2.0 {
                    b.win = Window {
                        t0: Some(now),
                        cpu0: cpu_seconds(),
                        drops0: self.props.i64("frame-drop-count").unwrap_or(0),
                        dec_drops0: self.props.i64("decoder-frame-drop-count").unwrap_or(0),
                        mistimed0: self.props.i64("mistimed-frame-count").unwrap_or(0),
                        ..Default::default()
                    };
                    b.phase = Phase::Play(now);
                }
            }
            Phase::Play(t) => {
                b.win.ui_frames += 1;
                if self.frame_rendered {
                    b.win.video_frames += 1;
                }
                let el = now.duration_since(*t).as_secs_f64();
                if el >= b.secs {
                    let w = &mut b.win;
                    let fps = self.props.f64("container-fps").unwrap_or(0.0);
                    b.report.push(format!("\"source\": {:?}", self.args.file));
                    b.report.push(format!("\"source_size\": [{}, {}]", self.src_size.0, self.src_size.1));
                    let v = self.video.as_ref().unwrap();
                    b.report.push(format!("\"texture_size\": [{}, {}]", v.w, v.h));
                    b.report.push(format!("\"hwdec\": {:?}", self.props.str("hwdec-current").unwrap_or_default()));
                    b.report.push(format!("\"blur_div\": {}, \"sigma\": {}", self.blur_div, self.sigma));
                    b.report.push(format!("\"container_fps\": {fps:.3}"));
                    b.report.push(format!("\"display_fps\": {:.2}", self.props.f64("display-fps").unwrap_or(0.0)));
                    b.report.push(format!("\"ui_fps\": {:.2}", w.ui_frames as f64 / el));
                    b.report.push(format!("\"video_fps\": {:.2}", w.video_frames as f64 / el));
                    b.report.push(format!(
                        "\"frame_drops\": {}, \"decoder_drops\": {}, \"mistimed\": {}",
                        self.props.i64("frame-drop-count").unwrap_or(0) - w.drops0,
                        self.props.i64("decoder-frame-drop-count").unwrap_or(0) - w.dec_drops0,
                        self.props.i64("mistimed-frame-count").unwrap_or(0) - w.mistimed0
                    ));
                    b.report.push(format!("\"cpu_cores\": {:.2}", (cpu_seconds() - w.cpu0) / el));
                    b.report.push(format!(
                        "\"gpu_mpv_ms\": {{\"mean\": {:.3}, \"p95\": {:.3}}}, \"gpu_blur_ms\": {{\"mean\": {:.3}, \"p95\": {:.3}}}",
                        mean(&w.gpu_mpv),
                        pct(&mut w.gpu_mpv, 0.95),
                        mean(&w.gpu_blur),
                        pct(&mut w.gpu_blur, 0.95)
                    ));
                    m.set("pause", "yes");
                    b.phase = Phase::Seeks { i: 0, sent: None, target: 0.0, restarted: None, frame: None, settle: 0 };
                }
            }
            Phase::Seeks { i, sent, target, restarted, frame, settle } => {
                const N: usize = 20;
                if sent.is_none() {
                    // псевдослучайные точки с дробными миллисекундами
                    let x = ((*i as f64 + 1.0) * 0.618_034).fract();
                    *target = 1.0 + x * (self.duration - 3.0) + 0.0137;
                    m.command(&["seek", &format!("{:.3}", *target), "absolute+exact"]);
                    *sent = Some(now);
                    *restarted = None;
                    *frame = None;
                } else if let Some(s) = *sent {
                    // кадр может отрисоваться раньше, чем мы прочитаем событие PLAYBACK_RESTART
                    if self.frame_rendered && frame.is_none() && now.duration_since(s).as_millis() > 0 {
                        *frame = Some(now);
                    }
                    if let (Some(r), Some(f), 0) = (*restarted, *frame, *settle) {
                        b.seek_lat_restart.push(r.duration_since(s).as_secs_f64() * 1000.0);
                        b.seek_lat_frame.push(r.max(f).duration_since(s).as_secs_f64() * 1000.0);
                        // time-pos приходит событием — даём ему несколько кадров дойти
                        *settle = 6;
                    } else if *settle > 1 {
                        *settle -= 1;
                    } else if *settle == 1 {
                        *settle = 0;
                        let pos = self.props.f64("time-pos").unwrap_or(f64::NAN);
                        b.seek_err.push((pos - *target) * 1000.0);
                        *i += 1;
                        *sent = None;
                        if *i == N {
                            let mut lr = b.seek_lat_restart.clone();
                            let mut lf = b.seek_lat_frame.clone();
                            let errs = b.seek_err.clone();
                            b.report.push(format!(
                                "\"seek_exact\": {{\"n\": {N}, \"restart_ms_mean\": {:.1}, \"restart_ms_max\": {:.1}, \"frame_ms_mean\": {:.1}, \"frame_ms_max\": {:.1}, \"err_ms_min\": {:.2}, \"err_ms_max\": {:.2}}}",
                                mean(&lr),
                                pct(&mut lr, 1.0),
                                mean(&lf),
                                pct(&mut lf, 1.0),
                                errs.iter().cloned().fold(f64::INFINITY, f64::min),
                                errs.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
                            ));
                            m.set("ab-loop-a", &format!("{:.3}", b.seg_in));
                            m.set("ab-loop-b", &format!("{:.3}", b.seg_out));
                            m.command(&["seek", &format!("{:.3}", b.seg_in), "absolute+exact"]);
                            m.set("pause", "no");
                            b.seg_last_pos = 0.0;
                            b.phase = Phase::Segment { started: now };
                        }
                    }
                }
            }
            Phase::Segment { started } => {
                if self.frame_rendered {
                    let pos = self.props.f64("time-pos").unwrap_or(0.0);
                    if pos + 0.5 < b.seg_last_pos {
                        // петля: прыжок назад
                        b.seg_ends.push(b.seg_last_pos);
                        b.seg_starts.push(pos);
                        if let Some(lf) = b.seg_last_frame_at {
                            b.seg_gaps.push(now.duration_since(lf).as_secs_f64() * 1000.0);
                        }
                    } else if b.seg_starts.is_empty() && b.seg_last_pos == 0.0 {
                        b.seg_starts.push(pos);
                    }
                    b.seg_max_pos = b.seg_max_pos.max(pos);
                    b.seg_last_pos = pos;
                    b.seg_last_frame_at = Some(now);
                }
                if now.duration_since(*started).as_secs_f64() > 8.0 {
                    let f = |v: &[f64]| v.iter().map(|x| format!("{x:.3}")).collect::<Vec<_>>().join(", ");
                    b.report.push(format!(
                        "\"segment\": {{\"in\": {}, \"out\": {}, \"first_frames\": [{}], \"last_frames\": [{}], \"loop_gap_ms\": [{}]}}",
                        b.seg_in,
                        b.seg_out,
                        f(&b.seg_starts),
                        f(&b.seg_ends),
                        b.seg_gaps.iter().map(|x| format!("{x:.0}")).collect::<Vec<_>>().join(", ")
                    ));
                    let json = format!("{{\n  {}\n}}\n", b.report.join(",\n  "));
                    print!("{json}");
                    if let Some(out) = &self.args.out {
                        std::fs::write(out, &json).unwrap();
                    }
                    b.phase = Phase::Done;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
            Phase::Done => {}
        }
        self.bench = Some(b);
        ctx.request_repaint();
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.pump(frame);
        let ctx = ui.ctx().clone();
        self.ui_main(ui);
        self.bench_step(&ctx);
        if let Some(m) = &self.mpv {
            m.report_swap();
        }
    }

    fn on_exit(&mut self, gl: Option<&glow::Context>) {
        // render context освобождается при текущем GL-контексте
        self.mpv.take();
        if let (Some(gl), Some(t)) = (gl, self.video.take()) {
            unsafe { t.delete(gl) };
        }
    }
}

fn main() -> eframe::Result {
    // libmpv требует C-локаль для разбора чисел
    unsafe { libc_setlocale() };
    let args = parse_args();
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Glow,
        viewport: egui::ViewportBuilder::default()
            .with_app_id("shortscutter-preview-spike")
            .with_inner_size([1600.0, 900.0]),
        ..Default::default()
    };
    eframe::run_native("ShortsCutter — спайк превью", options, Box::new(move |cc| Ok(Box::new(App::new(cc, args)))))
}

unsafe extern "C" {
    fn setlocale(category: i32, locale: *const std::ffi::c_char) -> *mut std::ffi::c_char;
}

unsafe fn libc_setlocale() {
    const LC_NUMERIC: i32 = 1;
    unsafe { setlocale(LC_NUMERIC, c"C".as_ptr()) };
}
