//! Экран «Оформление»: шаблон пачки правится мышью прямо в превью (выделение, перемещение, ручки масштаба,
//! кроп видео внутри исходника, привязки и сетка, зоны интерфейса Shorts) и в панели свойств; слои и фон,
//! отмена/повтор. Геометрия — в [`crate::editor`].

use std::ops::RangeInclusive;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, CursorIcon, Key, Pos2, Sense, Stroke, Vec2};
use sc_core::{
    AccentMode, Align, Background, Batch, Border, Canvas, Color, Cover, CoverSource, Layer, Outline, Paint, Rect, SHORTS_UNSAFE_ZONES, Shadow,
    ShortId, ShortStatus, SubtitleMode, Template, TemplateError, TemplateRef, VAlign, SUBTITLE_LINES,
    SUBTITLE_WORDS_PER_LINE, main_video_crop,
};
use sc_i18n::I18n;

use crate::App;
use crate::cut::preview_canvas;
use crate::editor::{self, Guides, Handle, SnapOpts, Targets};
use crate::fmt;
use crate::import::file_name;
use crate::jobs::Msg;
use crate::templates::LibDialog;

/// Порог привязки, пиксели экрана.
const SNAP_PX: f32 = 6.0;
/// Сторона ручки масштаба и зона захвата вокруг неё, пиксели экрана.
const HANDLE_PX: f32 = 8.0;
const HANDLE_GRAB_PX: f32 = 7.0;
/// Клеток сетки по ширине холста (клетки квадратные).
const GRID_CELLS: f32 = 18.0;
/// Правки одного поля подряд (с паузами меньше этой) — один шаг отмены.
const COALESCE: Duration = Duration::from_millis(1500);
const UNDO_MAX: usize = 200;
/// Приближение кропа на единицу прокрутки (точки egui).
const ZOOM_PER_POINT: f32 = 0.003;
/// Ширина панели свойств.
const PROPS_W: f32 = 360.0;

const SELECT: Color32 = Color32::from_rgb(80, 160, 255);
const GUIDE: Color32 = Color32::from_rgb(255, 64, 200);
const CROP_FRAME: Color32 = Color32::from_rgb(255, 210, 60);

const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "webp", "bmp", "gif"];
const VIDEO_EXTS: &[&str] = &["mp4", "mkv", "webm", "mov", "m4v"];
const FONT_EXTS: &[&str] = &["ttf", "otf", "ttc", "otc"];

pub struct DesignState {
    /// Выделенный слой; `None` — фон.
    pub layer: Option<usize>,
    drag: Option<Drag>,
    grid: bool,
    snap: bool,
    zones: bool,
    /// Тянуть мышью кроп видео, а не сам слой (то же — с зажатым Alt).
    crop_mode: bool,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    /// Править локальный шаблон выбранного шортса, а не шаблон пачки.
    pub local: bool,
    /// Править обложку выбранного шортса (если она есть).
    pub cover: bool,
    /// Что правится сейчас (см. [`App::design_sync_target`]).
    pub target: Target,
    /// Какой кадр обложки показан в превью (чтобы перематывать/подменять кадр только при смене).
    pub(crate) cover_shown: Option<CoverSource>,
    /// Открытый диалог библиотеки шаблонов.
    pub(crate) dialog: Option<LibDialog>,
    /// Ключ и время последней правки — для склейки правок в один шаг отмены.
    pub(crate) last_edit: Option<(String, Instant)>,
    guides: Guides,
    /// Ошибка последней правки — ключ i18n.
    error: Option<&'static str>,
}

impl Default for DesignState {
    fn default() -> Self {
        Self {
            layer: None,
            drag: None,
            grid: false,
            snap: true,
            zones: true,
            crop_mode: false,
            undo: Vec::new(),
            redo: Vec::new(),
            local: false,
            cover: false,
            target: Target::Batch,
            cover_shown: None,
            dialog: None,
            last_edit: None,
            guides: Guides::default(),
            error: None,
        }
    }
}

/// Что правит «Оформление».
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// Шаблон пачки.
    Batch,
    /// Локальный шаблон шортса.
    Short(ShortId),
    /// Шаблон обложки шортса.
    Cover(ShortId),
}

impl Target {
    pub fn short(self) -> Option<ShortId> {
        match self {
            Target::Batch => None,
            Target::Short(id) | Target::Cover(id) => Some(id),
        }
    }
}

/// Состояние для отмены: шаблон пачки (с источником в библиотеке), локальный шаблон шортса или обложка шортса.
pub(crate) enum Snapshot {
    Batch(Template, Option<TemplateRef>),
    Short(ShortId, Option<Template>),
    Cover(ShortId, Option<Cover>),
}

/// Шаблон, который правится: пачки, локальный шортса (если правок ещё нет — шаблон пачки) или обложки
/// (если обложки нет — шаблон шортса).
pub(crate) fn edited(batch: &Batch, target: Target) -> &Template {
    match target {
        Target::Batch => &batch.template,
        Target::Short(id) => batch.template_for(id),
        Target::Cover(id) => batch.short(id).and_then(|s| s.cover.as_ref()).map_or_else(|| batch.template_for(id), |c| &c.template),
    }
}

struct Drag {
    layer: usize,
    handle: Handle,
    /// Тянут кроп видео внутри слоя.
    crop: bool,
    start: Rect,
    /// Видимый участок исходника в начале (для кропа) и кроп слоя, как он записан в шаблоне.
    start_crop: Rect,
    orig_crop: Rect,
    /// Точка нажатия в долях холста.
    from: (f32, f32),
}

/// Для чего выбирается файл в диалоге.
#[derive(Debug, Clone, Copy)]
pub enum FileTarget {
    NewImage,
    LayerImage(usize),
    CapsuleLogo(usize),
    BackgroundImage,
    BackgroundVideo,
    /// Свой файл шрифта для текста или капсулы слоя.
    Font(usize),
}

#[derive(Clone, Copy)]
enum NewLayer {
    Video,
    Text,
    Image,
    Capsule,
    Shape,
    Banner,
    Subtitles,
}

fn layer_key(l: &Layer) -> &'static str {
    match l {
        Layer::MainVideo { .. } => "layer-main-video",
        Layer::Text { .. } => "layer-text",
        Layer::Image { .. } => "layer-image",
        Layer::Capsule { .. } => "layer-capsule",
        Layer::Shape { .. } => "layer-shape",
        Layer::BannerSlot { .. } => "layer-banner",
        Layer::Clip { .. } => "layer-image",
        Layer::Subtitles { .. } => "layer-subtitles",
    }
}

fn error_key(e: &TemplateError) -> &'static str {
    match e {
        TemplateError::NoMainVideo => "design-need-video",
        _ => "design-invalid",
    }
}

fn excerpt(s: &str, n: usize) -> String {
    let line = s.lines().next().unwrap_or_default();
    let mut out: String = line.chars().take(n).collect();
    if line.chars().count() > n || s.lines().nth(1).is_some() {
        out.push('…');
    }
    out
}

impl App {
    /// Правка шаблона, который сейчас правится (пачки или локального шортса): применяется, только если шаблон
    /// остаётся корректным; правки с одним `key` подряд склеиваются в один шаг отмены. Первая локальная правка
    /// шортса начинается с копии шаблона пачки.
    fn design_edit(&mut self, key: &str, f: impl FnOnce(&mut Template)) {
        let target = self.design.target;
        let mut t = edited(&self.batch, target).clone();
        f(&mut t);
        if t == *edited(&self.batch, target) {
            return;
        }
        if let Err(e) = t.validate() {
            log::debug!("правка шаблона отклонена: {e}");
            self.design.error = Some(error_key(&e));
            return;
        }
        let now = Instant::now();
        let same = matches!(&self.design.last_edit, Some((k, at)) if k == key && now.duration_since(*at) < COALESCE);
        let next = match target {
            Target::Short(id) => Snapshot::Short(id, Some(t)),
            Target::Cover(id) => {
                let Some(mut c) = self.batch.short(id).and_then(|s| s.cover.clone()) else { return };
                c.template = t;
                Snapshot::Cover(id, Some(c))
            }
            Target::Batch => Snapshot::Batch(t, self.batch.template_ref.clone()),
        };
        self.design_apply(next, !same);
        self.design.last_edit = Some((key.to_owned(), now));
    }

    /// Ставит состояние шаблона (`undo` — запомнить прежнее для отмены) и сбрасывает повтор.
    pub(crate) fn design_apply(&mut self, next: Snapshot, undo: bool) {
        let prev = self.design_restore(next);
        let d = &mut self.design;
        if undo && let Some(prev) = prev {
            d.undo.push(prev);
            if d.undo.len() > UNDO_MAX {
                d.undo.remove(0);
            }
        }
        d.redo.clear();
        d.error = None;
    }

    /// Ставит состояние и возвращает прежнее состояние той же цели (`None` — шортса уже нет).
    fn design_restore(&mut self, snap: Snapshot) -> Option<Snapshot> {
        match snap {
            Snapshot::Batch(t, r) => {
                let t = std::mem::replace(&mut self.batch.template, t);
                let r = std::mem::replace(&mut self.batch.template_ref, r);
                Some(Snapshot::Batch(t, r))
            }
            Snapshot::Short(id, t) => {
                let s = self.batch.short_mut(id)?;
                Some(Snapshot::Short(id, std::mem::replace(&mut s.template, t)))
            }
            Snapshot::Cover(id, c) => {
                let s = self.batch.short_mut(id)?;
                Some(Snapshot::Cover(id, std::mem::replace(&mut s.cover, c)))
            }
        }
    }

    fn design_undo(&mut self, redo: bool) {
        loop {
            let d = &mut self.design;
            let from = if redo { &mut d.redo } else { &mut d.undo };
            let Some(snap) = from.pop() else { return };
            // Шаг про удалённый шортс пропускается.
            let Some(prev) = self.design_restore(snap) else { continue };
            let d = &mut self.design;
            if redo { &mut d.undo } else { &mut d.redo }.push(prev);
            d.last_edit = None;
            d.drag = None;
            d.error = None;
            let n = edited(&self.batch, d.target).layers.len();
            d.layer = d.layer.filter(|&i| i < n);
            return;
        }
    }

    /// Что правит «Оформление»: обложку выбранного шортса (если она есть и включена её правка), его локальный
    /// шаблон (если включены локальные правки) или шаблон пачки. Вызывается в начале кадра.
    pub(crate) fn design_sync_target(&mut self) {
        let short = self.cut.selected.and_then(|id| self.batch.short(id));
        let target = match short {
            Some(s) if self.design.cover && s.cover.is_some() => Target::Cover(s.id),
            Some(s) if self.design.local => Target::Short(s.id),
            _ => Target::Batch,
        };
        if target != self.design.target {
            self.design.target = target;
            self.design.last_edit = None;
            self.design.drag = None;
            self.design.layer = None;
        }
        self.cover_sync_preview();
    }

    /// Шаблон для превью: на «Оформлении» — тот, что правится, иначе — по выбранному шортсу.
    pub(crate) fn preview_template(&self, design: bool) -> &Template {
        match (design, self.cut.selected) {
            (true, _) => edited(&self.batch, self.design.target),
            (false, Some(id)) => self.batch.template_for(id),
            (false, None) => edited(&self.batch, self.design.target),
        }
    }

    fn design_layer(&self) -> Option<usize> {
        self.design.layer.filter(|&i| i < edited(&self.batch, self.design.target).layers.len())
    }

    fn add_layer(&mut self, kind: NewLayer, image: Option<PathBuf>) {
        let at = self.design_layer().map_or(edited(&self.batch, self.design.target).layers.len(), |i| i + 1);
        let layer = match kind {
            NewLayer::Video => {
                let h = 1080.0 * 9.0 / 16.0 / 1920.0;
                Layer::MainVideo {
                    crop: Rect::FULL,
                    dest: Rect::new(0.0, (1.0 - h) / 2.0, 1.0, h),
                    corner_radius: 0.0,
                    border: None,
                    shadow: None,
                }
            }
            NewLayer::Text => Layer::Text {
                rect: Rect::new(0.06, 0.15, 0.88, 0.12),
                content: self.i18n.tr("design-new-text"),
                font: sc_raster::fonts::font_name(sc_raster::fonts::SANS, 800),
                size: 0.07,
                color: Color::WHITE,
                accent_color: Color::rgb(0xff, 0x3b, 0x30),
                accent_mode: AccentMode::Color,
                outline: None,
                shadow: None,
                align: Align::Center,
                valign: VAlign::Center,
            },
            NewLayer::Image => {
                let Some(path) = image else { return };
                Layer::Image { rect: Rect::new(0.35, 0.15, 0.3, 0.15), path, opacity: 1.0 }
            }
            NewLayer::Capsule => Layer::Capsule {
                rect: Rect::new(0.25, 0.14, 0.5, 0.045),
                logo: None,
                text: self.i18n.tr("design-new-capsule"),
                font: sc_raster::fonts::font_name(sc_raster::fonts::SANS, 700),
                text_color: Color::WHITE,
                fill: Color([0x1d, 0x1d, 0x1f, 0xcc]),
            },
            NewLayer::Shape => Layer::Shape {
                rect: Rect::new(0.0, 0.6, 1.0, 0.4),
                paint: Paint::Linear { from: Color([0, 0, 0, 0]), to: Color([0, 0, 0, 0xcc]), angle: 0.0 },
                corner_radius: 0.0,
            },
            NewLayer::Banner => Layer::BannerSlot { rect: Rect::new(0.1, 0.62, 0.8, 0.1) },
            NewLayer::Subtitles => Layer::Subtitles {
                rect: Rect::new(0.06, 0.62, 0.78, 0.12),
                font: sc_raster::fonts::font_name(sc_raster::fonts::SANS, 800),
                size: 0.06,
                color: Color::WHITE,
                active_color: Color::rgb(0xff, 0xd6, 0x0a),
                active_mode: AccentMode::Color,
                keyword_color: Color::rgb(0xff, 0x3b, 0x30),
                dim_color: Color([0xff, 0xff, 0xff, 0x60]),
                mode: SubtitleMode::Highlight,
                words_per_line: 3,
                lines: 2,
                uppercase: true,
                outline: Some(Outline { width: 0.005, color: Color::BLACK }),
                shadow: None,
                plate: None,
                align: Align::Center,
                valign: VAlign::Center,
            },
        };
        self.design.last_edit = None;
        self.design_edit("add", |t| t.layers.insert(at, layer));
        if self.design.error.is_none() {
            self.design.layer = Some(at);
        }
    }

    fn delete_layer(&mut self) {
        let Some(i) = self.design_layer() else { return };
        self.design.last_edit = None;
        self.design_edit("delete", |t| {
            t.layers.remove(i);
        });
        if self.design.error.is_none() {
            self.design.layer = None;
        }
    }

    fn duplicate_layer(&mut self) {
        let Some(i) = self.design_layer() else { return };
        let mut copy = edited(&self.batch, self.design.target).layers[i].clone();
        // Копия чуть ниже и правее, если влезает, — чтобы было видно, что она есть.
        let shift = |r: &mut Rect| {
            if r.right() + 0.02 <= 1.0 && r.bottom() + 0.02 <= 1.0 {
                r.x += 0.02;
                r.y += 0.02;
            }
        };
        set_rect(&mut copy, |r| shift(r));
        self.design.last_edit = None;
        self.design_edit("duplicate", |t| t.layers.insert(i + 1, copy));
        if self.design.error.is_none() {
            self.design.layer = Some(i + 1);
        }
    }

    /// Поднять (`up`) или опустить слой на одну позицию в z-order.
    fn move_layer(&mut self, up: bool) {
        let Some(i) = self.design_layer() else { return };
        let j = if up { i + 1 } else { i.wrapping_sub(1) };
        if j >= edited(&self.batch, self.design.target).layers.len() {
            return;
        }
        self.design.last_edit = None;
        self.design_edit("order", |t| t.layers.swap(i, j));
        self.design.layer = Some(j);
    }

    pub(crate) fn pick_file(&mut self, target: FileTarget) {
        self.dialog_open = true;
        let (title, filter, exts) = match target {
            FileTarget::BackgroundVideo => ("design-pick-video", "design-filter-video", VIDEO_EXTS),
            FileTarget::Font(_) => ("design-pick-font", "design-filter-font", FONT_EXTS),
            _ => ("design-pick-image", "design-filter-image", IMAGE_EXTS),
        };
        let (title, filter) = (self.i18n.tr(title), self.i18n.tr(filter));
        let dialog = self.file_dialog(title);
        self.jobs.spawn(move || {
            let file = dialog.add_filter(filter, exts).pick_file();
            Msg::PickedFile(target, file)
        });
    }

    pub(crate) fn on_picked_file(&mut self, target: FileTarget, file: Option<PathBuf>) {
        self.dialog_open = false;
        let Some(path) = file else { return };
        self.design.last_edit = None;
        match target {
            FileTarget::NewImage => self.add_layer(NewLayer::Image, Some(path)),
            FileTarget::LayerImage(i) => self.design_edit("file", |t| {
                if let Some(Layer::Image { path: p, .. }) = t.layers.get_mut(i) {
                    *p = path;
                }
            }),
            FileTarget::CapsuleLogo(i) => self.design_edit("file", |t| {
                if let Some(Layer::Capsule { logo, .. }) = t.layers.get_mut(i) {
                    *logo = Some(path);
                }
            }),
            FileTarget::BackgroundImage => self.design_edit("file", |t| {
                let darken = bg_darken(&t.background);
                t.background = Background::Image { path, darken };
            }),
            FileTarget::BackgroundVideo => self.design_edit("file", |t| {
                let darken = bg_darken(&t.background);
                t.background = Background::Video { path, darken };
            }),
            FileTarget::Font(_) if target.is_banner_font() => match sc_raster::fonts::install_font(&path) {
                Ok(name) => self.set_banner_font(name),
                Err(e) => {
                    log::error!("шрифт {path:?}: {e}");
                    self.design.error = Some("design-font-failed");
                }
            },
            FileTarget::Font(i) => match sc_raster::fonts::install_font(&path) {
                Ok(name) => self.design_edit("file", |t| {
                    if let Some(Layer::Text { font, .. } | Layer::Capsule { font, .. } | Layer::Subtitles { font, .. }) =
                        t.layers.get_mut(i)
                    {
                        *font = name;
                    }
                }),
                Err(e) => {
                    log::error!("шрифт {path:?}: {e}");
                    self.design.error = Some("design-font-failed");
                }
            },
        }
    }

    fn design_keys(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let (m, pressed) = ctx.input(|i| {
            let keys = [
                Key::Z,
                Key::Y,
                Key::D,
                Key::G,
                Key::C,
                Key::Space,
                Key::Delete,
                Key::Escape,
                Key::ArrowLeft,
                Key::ArrowRight,
                Key::ArrowUp,
                Key::ArrowDown,
            ];
            (i.modifiers, keys.into_iter().filter(|k| i.key_pressed(*k)).collect::<Vec<_>>())
        });
        let canvas = edited(&self.batch, self.design.target).canvas;
        let step = if m.shift { 10.0 } else { 1.0 };
        for key in pressed {
            let nudge = match key {
                Key::ArrowLeft => (-step / canvas.width as f32, 0.0),
                Key::ArrowRight => (step / canvas.width as f32, 0.0),
                Key::ArrowUp => (0.0, -step / canvas.height as f32),
                Key::ArrowDown => (0.0, step / canvas.height as f32),
                _ => (0.0, 0.0),
            };
            match key {
                Key::Z if m.command => self.design_undo(m.shift),
                Key::Y if m.command => self.design_undo(true),
                Key::D if m.command => self.duplicate_layer(),
                Key::G if !m.command => self.design.grid = !self.design.grid,
                Key::C if !m.command => self.design.crop_mode = !self.design.crop_mode,
                Key::Space => {
                    if let Some(p) = &mut self.player {
                        p.toggle_pause();
                    }
                }
                Key::Delete => self.delete_layer(),
                Key::Escape => self.design.layer = None,
                Key::ArrowLeft | Key::ArrowRight | Key::ArrowUp | Key::ArrowDown => {
                    if let Some(i) = self.design_layer() {
                        let key = format!("nudge:{i}");
                        self.design_edit(&key, |t| {
                            set_rect(&mut t.layers[i], |r| {
                                r.x = (r.x + nudge.0).clamp(0.0, 1.0 - r.w);
                                r.y = (r.y + nudge.1).clamp(0.0, 1.0 - r.h);
                            })
                        });
                    }
                }
                _ => {}
            }
        }
    }

    pub(crate) fn design_ui(&mut self, ui: &mut egui::Ui) {
        self.design_keys(&ui.ctx().clone());
        if self.design.layer.is_some() && self.design_layer().is_none() {
            self.design.layer = None;
        }

        egui::Panel::left("design-layers").resizable(true).default_size(230.0).show(ui, |ui| self.layers_ui(ui));
        // Ширина не зависит от выбранного слоя — иначе холст прыгает при выборе.
        egui::Panel::right("design-props").resizable(false).exact_size(PROPS_W).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.props_ui(ui));
        });
        egui::Panel::top("design-tools").show(ui, |ui| self.design_toolbar(ui));
        egui::CentralPanel::default().show(ui, |ui| {
            if self.player.is_none() {
                let err = self.player_error.clone().unwrap_or_default();
                ui.colored_label(ui.visuals().error_fg_color, self.i18n.tr_args("cut-player-failed", &[("error", err.into())]));
                return;
            }
            self.design_canvas(ui);
        });
    }

    fn design_toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            let d = &self.design;
            let (can_undo, can_redo) = (!d.undo.is_empty(), !d.redo.is_empty());
            if ui.add_enabled(can_undo, egui::Button::new("⟲")).on_hover_text(self.i18n.tr("design-undo")).clicked() {
                self.design_undo(false);
            }
            if ui.add_enabled(can_redo, egui::Button::new("⟳")).on_hover_text(self.i18n.tr("design-redo")).clicked() {
                self.design_undo(true);
            }
            ui.separator();
            if let Some(p) = &mut self.player {
                let ready = p.is_ready();
                let play = if p.state.paused { "▶" } else { "⏸" };
                if ui.add_enabled(ready, egui::Button::new(play)).on_hover_text(self.i18n.tr("cut-play")).clicked() {
                    p.toggle_pause();
                }
                ui.monospace(fmt::clock_ms(p.state.time));
            }
            self.preview_short_combo(ui);
            ui.separator();
            let d = &mut self.design;
            ui.toggle_value(&mut d.snap, self.i18n.tr("design-snap")).on_hover_text(self.i18n.tr("design-snap-hint"));
            ui.toggle_value(&mut d.grid, self.i18n.tr("design-grid")).on_hover_text("G");
            ui.toggle_value(&mut d.zones, self.i18n.tr("design-zones")).on_hover_text(self.i18n.tr("design-zones-hint"));
            ui.toggle_value(&mut d.crop_mode, self.i18n.tr("design-crop-mode")).on_hover_text(self.i18n.tr("design-crop-hint"));
            if let Some(key) = d.error {
                ui.separator();
                ui.colored_label(ui.visuals().error_fg_color, self.i18n.tr(key));
            }
        });
    }

    /// Какой шортс показывать в превью (подстановки `{title}`/`{part}` и петля воспроизведения).
    fn preview_short_combo(&mut self, ui: &mut egui::Ui) {
        let Some(source) = self.cut.source else { return };
        let shorts: Vec<(sc_core::ShortId, String)> = self
            .batch
            .shorts_of(source)
            .into_iter()
            .filter(|s| s.status != ShortStatus::Rejected)
            .map(|s| {
                let mark = if s.status == ShortStatus::Accepted { "✔" } else { "?" };
                // ✏ — у шортса свои правки шаблона.
                let own = if s.template.is_some() { " ✏" } else { "" };
                (s.id, format!("{mark}{own} {} {}", fmt::clock_ms(s.start), excerpt(&s.title, 32)))
            })
            .collect();
        let current = self.cut.selected.and_then(|id| shorts.iter().find(|(s, _)| *s == id)).map(|(_, t)| t.clone());
        let mut pick = None;
        egui::ComboBox::from_id_salt("design-short")
            .width(240.0)
            .selected_text(current.unwrap_or_else(|| self.i18n.tr("design-preview-none")))
            .show_ui(ui, |ui| {
                for (id, text) in &shorts {
                    if ui.selectable_label(self.cut.selected == Some(*id), text).clicked() {
                        pick = Some(*id);
                    }
                }
            })
            .response
            .on_hover_text(self.i18n.tr("design-preview-short"));
        if let Some(id) = pick {
            self.select_short(Some(id));
        }
    }

    fn layers_ui(&mut self, ui: &mut egui::Ui) {
        // Пока правится обложка, выбор шаблона пачки/шортса спрятан — у обложки свой стиль.
        if !matches!(self.design.target, Target::Cover(_)) {
            self.template_ui(ui);
            ui.add_space(6.0);
            ui.separator();
        }
        self.cover_ui(ui);
        ui.add_space(6.0);
        ui.separator();
        ui.heading(self.i18n.tr("design-layers"));
        ui.add_space(4.0);
        let n = edited(&self.batch, self.design.target).layers.len();
        let mut pick = self.design.layer;
        // Сверху списка — верхний слой.
        for i in (0..n).rev() {
            let l = &edited(&self.batch, self.design.target).layers[i];
            let kind = self.i18n.tr(layer_key(l));
            let mut text = match l {
                Layer::Text { content, .. } => format!("{kind} · {}", excerpt(content, 18)),
                Layer::Capsule { text, .. } => format!("{kind} · {}", excerpt(text, 18)),
                Layer::Image { path, .. } => format!("{kind} · {}", file_name(path)),
                _ => kind,
            };
            let unsafe_zone = l.in_unsafe_zone();
            if unsafe_zone {
                text = format!("⚠ {text}");
            }
            let r = ui.selectable_label(self.design.layer == Some(i), text);
            let r = if unsafe_zone { r.on_hover_text(self.i18n.tr("design-unsafe-warning")) } else { r };
            if r.clicked() {
                pick = Some(i);
            }
        }
        if ui.selectable_label(self.design.layer.is_none(), self.i18n.tr("design-background")).clicked() {
            pick = None;
        }
        self.design.layer = pick;

        ui.add_space(6.0);
        let sel = self.design_layer().is_some();
        let mut new = None;
        ui.horizontal_wrapped(|ui| {
            ui.menu_button(self.i18n.tr("design-add"), |ui| {
                let kinds = [
                    (NewLayer::Text, "layer-text"),
                    (NewLayer::Subtitles, "layer-subtitles"),
                    (NewLayer::Image, "layer-image"),
                    (NewLayer::Capsule, "layer-capsule"),
                    (NewLayer::Shape, "layer-shape"),
                    (NewLayer::Banner, "layer-banner"),
                    (NewLayer::Video, "layer-main-video"),
                ];
                for (k, key) in kinds {
                    if ui.button(self.i18n.tr(key)).clicked() {
                        new = Some(k);
                        ui.close();
                    }
                }
            });
            if ui.add_enabled(sel, egui::Button::new("⬆")).on_hover_text(self.i18n.tr("design-up")).clicked() {
                self.move_layer(true);
            }
            if ui.add_enabled(sel, egui::Button::new("⬇")).on_hover_text(self.i18n.tr("design-down")).clicked() {
                self.move_layer(false);
            }
            if ui.add_enabled(sel, egui::Button::new("🗐")).on_hover_text(self.i18n.tr("design-duplicate")).clicked() {
                self.duplicate_layer();
            }
            if ui.add_enabled(sel, egui::Button::new("🗑")).on_hover_text(self.i18n.tr("design-delete")).clicked() {
                self.delete_layer();
            }
        });
        match new {
            Some(NewLayer::Image) => self.pick_file(FileTarget::NewImage),
            Some(k) => self.add_layer(k, None),
            None => {}
        }
        ui.add_space(8.0);
        ui.weak(self.i18n.tr("design-hint"));
    }

    fn props_ui(&mut self, ui: &mut egui::Ui) {
        let canvas = edited(&self.batch, self.design.target).canvas;
        let mut pick = None;
        match self.design_layer() {
            None => {
                ui.heading(self.i18n.tr("design-background"));
                let mut bg = edited(&self.batch, self.design.target).background.clone();
                background_props(ui, &self.i18n, &mut bg, &mut pick);
                if bg != edited(&self.batch, self.design.target).background {
                    self.design_edit("bg", |t| t.background = bg);
                }
            }
            Some(i) => {
                let orig = &edited(&self.batch, self.design.target).layers[i];
                ui.heading(self.i18n.tr(layer_key(orig)));
                if orig.in_unsafe_zone() {
                    ui.colored_label(Color32::from_rgb(255, 170, 60), self.i18n.tr("design-unsafe-warning"));
                }
                let slot = matches!(orig, Layer::BannerSlot { .. });
                let mut l = orig.clone();
                layer_props(ui, &self.i18n, &mut l, i, canvas, &mut pick);
                if &l != orig {
                    let key = format!("prop:{i}");
                    self.design_edit(&key, |t| t.layers[i] = l);
                }
                if slot {
                    ui.add_space(10.0);
                    ui.separator();
                    self.banner_ui(ui);
                }
            }
        }
        if let Some(target) = pick {
            self.pick_file(target);
        }
    }

    fn design_canvas(&mut self, ui: &mut egui::Ui) {
        let area = ui.available_rect_before_wrap();
        let empty = self.i18n.tr(if self.cut.source.is_none() { "design-preview-empty" } else { "cut-preview-loading" });
        let screen = preview_canvas(ui, area, self.player.as_mut().unwrap(), edited(&self.batch, self.design.target), &empty);
        let src = self.player.as_ref().map_or((0, 0), |p| p.frame_size());
        let has_src = src.0 > 0 && src.1 > 0;

        let canvas = edited(&self.batch, self.design.target).canvas;
        let (sw, sh) = (screen.width(), screen.height());
        let to_frac = |p: Pos2| ((p.x - screen.min.x) / sw, (p.y - screen.min.y) / sh);
        let to_screen = |r: Rect| egui::Rect::from_min_size(screen.min + Vec2::new(r.x * sw, r.y * sh), Vec2::new(r.w * sw, r.h * sh));
        let handle_at = |r: Rect, p: Pos2| {
            let s = to_screen(r);
            Handle::RESIZE.into_iter().find(|h| {
                let (ax, ay) = h.anchor();
                let c = Pos2::new(s.min.x + ax * s.width(), s.min.y + ay * s.height());
                (c.x - p.x).abs() <= HANDLE_GRAB_PX && (c.y - p.y).abs() <= HANDLE_GRAB_PX
            })
        };

        let resp = ui.interact(screen.expand(HANDLE_GRAB_PX), ui.id().with("design-canvas"), Sense::click_and_drag());
        let m = ui.input(|i| i.modifiers);
        let crop_mode = self.design.crop_mode || m.alt;
        let sel = self.design_layer();
        let is_video = |i: usize, t: &Template| matches!(t.layers[i], Layer::MainVideo { .. });

        // Курсор над ручками и слоями.
        if let Some(p) = resp.hover_pos()
            && self.design.drag.is_none()
        {
            let t = edited(&self.batch, self.design.target);
            let h = sel.and_then(|i| handle_at(t.layers[i].rect(), p));
            let icon = match h {
                Some(Handle::N | Handle::S) => CursorIcon::ResizeVertical,
                Some(Handle::E | Handle::W) => CursorIcon::ResizeHorizontal,
                Some(Handle::NE | Handle::SW) => CursorIcon::ResizeNeSw,
                Some(Handle::NW | Handle::SE) => CursorIcon::ResizeNwSe,
                _ => match editor::hit(&t.layers, to_frac(p), sel) {
                    Some(i) if crop_mode && is_video(i, t) && has_src => CursorIcon::AllScroll,
                    Some(_) => CursorIcon::Move,
                    None => CursorIcon::Default,
                },
            };
            ui.ctx().set_cursor_icon(icon);
        }

        if resp.drag_started()
            && let Some(p) = ui.input(|i| i.pointer.press_origin())
        {
            let t = edited(&self.batch, self.design.target);
            let from = to_frac(p);
            let grabbed = sel.and_then(|i| handle_at(t.layers[i].rect(), p).map(|h| (i, h)));
            let grabbed = grabbed.or_else(|| editor::hit(&t.layers, from, sel).map(|i| (i, Handle::Move)));
            self.design.layer = grabbed.map(|(i, _)| i);
            self.design.drag = grabbed.map(|(i, handle)| {
                let crop = handle == Handle::Move && crop_mode && has_src && is_video(i, t);
                let (start_crop, orig_crop) = match &t.layers[i] {
                    Layer::MainVideo { crop, dest, .. } if has_src => (main_video_crop(*crop, src, *dest, canvas), *crop),
                    _ => (Rect::FULL, Rect::FULL),
                };
                Drag { layer: i, handle, crop, start: t.layers[i].rect(), start_crop, orig_crop, from }
            });
            self.design.last_edit = None;
        }
        if resp.dragged()
            && let Some(d) = &self.design.drag
            && let Some(p) = resp.interact_pointer_pos()
        {
            let (i, now) = (d.layer, to_frac(p));
            let delta = (now.0 - d.from.0, now.1 - d.from.1);
            let key = format!("drag:{i}");
            if d.crop {
                let crop = editor::pan_crop(d.start_crop, (delta.0 / d.start.w, delta.1 / d.start.h));
                // Сдвинуть некуда (видео целиком в слое) — кроп как был, без лишнего шага отмены.
                let crop = if editor::same_rect(crop, d.start_crop) { d.orig_crop } else { crop };
                self.design_edit(&key, |t| {
                    if let Layer::MainVideo { crop: c, .. } = &mut t.layers[i] {
                        *c = crop;
                    }
                });
            } else {
                let targets = Targets::new(&edited(&self.batch, self.design.target).layers, i);
                let opts = SnapOpts {
                    // Ctrl — временно без привязок.
                    threshold: (self.design.snap && !m.command).then_some((SNAP_PX / sw, SNAP_PX / sh)),
                    grid: (self.design.grid && !m.command).then(|| {
                        let step = 1.0 / GRID_CELLS;
                        (step, step * canvas.width as f32 / canvas.height as f32)
                    }),
                };
                let aspect = canvas.width as f32 / canvas.height as f32;
                let (r, guides) = editor::drag(d.start, d.handle, delta, m.shift, aspect, &targets, opts);
                self.design.guides = guides;
                self.design_edit(&key, |t| set_rect(&mut t.layers[i], |x| *x = r));
            }
        }
        if resp.drag_stopped() {
            self.design.drag = None;
            self.design.guides = Guides::default();
        }
        if resp.clicked()
            && let Some(p) = resp.interact_pointer_pos()
        {
            let t = edited(&self.batch, self.design.target);
            if sel.and_then(|i| handle_at(t.layers[i].rect(), p)).is_none() {
                self.design.layer = editor::hit(&t.layers, to_frac(p), None);
            }
        }
        // Колесо над видео в режиме кропа — приблизить/отдалить картинку внутри слоя.
        if crop_mode
            && has_src
            && let Some(p) = resp.hover_pos()
            && let Some(i) = editor::hit(&edited(&self.batch, self.design.target).layers, to_frac(p), sel)
            && let Layer::MainVideo { crop, dest, .. } = &edited(&self.batch, self.design.target).layers[i]
        {
            let scroll = ui.input(|inp| inp.smooth_scroll_delta.y);
            if scroll != 0.0 {
                let (crop, dest) = (*crop, *dest);
                let eff = main_video_crop(crop, src, dest, canvas);
                let f = to_frac(p);
                let at = (((f.0 - dest.x) / dest.w).clamp(0.0, 1.0), ((f.1 - dest.y) / dest.h).clamp(0.0, 1.0));
                let zoomed = editor::zoom_crop(eff, (scroll * ZOOM_PER_POINT).exp(), at);
                let key = format!("zoom:{i}");
                self.design.layer = Some(i);
                self.design_edit(&key, |t| {
                    if let Layer::MainVideo { crop: c, .. } = &mut t.layers[i] {
                        *c = zoomed;
                    }
                });
            }
        }

        // Оверлеи редактора поверх холста.
        let t = edited(&self.batch, self.design.target);
        let sel = self.design_layer();
        let painter = ui.painter().with_clip_rect(area);
        let canvas_painter = ui.painter().with_clip_rect(screen);
        if self.design.grid {
            let step = sw / GRID_CELLS;
            let thin = Stroke::new(1.0, Color32::from_white_alpha(28));
            let mut x = screen.min.x + step;
            while x < screen.max.x - 0.5 {
                canvas_painter.vline(x, screen.y_range(), thin);
                x += step;
            }
            let mut y = screen.min.y + step;
            while y < screen.max.y - 0.5 {
                canvas_painter.hline(screen.x_range(), y, thin);
                y += step;
            }
        }
        if self.design.zones {
            for z in SHORTS_UNSAFE_ZONES {
                let r = to_screen(z);
                canvas_painter.rect_filled(r, 0.0, Color32::from_rgba_unmultiplied(255, 60, 60, 34));
                canvas_painter.rect_stroke(r, 0.0, Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 90, 90, 120)), egui::StrokeKind::Inside);
            }
        }
        // Баннер виден в превью — у его места только тонкая рамка.
        let banner_shown = self.batch.banner.is_some() && !self.banner_missing()
            && !self.cut.selected.and_then(|id| self.batch.short(id)).is_some_and(|s| s.no_banner);
        let first_slot = t.banner_slot();
        for (i, l) in t.layers.iter().enumerate() {
            if let Layer::BannerSlot { rect } = l
                && banner_shown
                && first_slot == Some(i)
            {
                canvas_painter.rect_stroke(to_screen(*rect), 2.0, Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 200, 60, 110)), egui::StrokeKind::Inside);
            } else if let Layer::BannerSlot { rect } = l {
                let r = to_screen(*rect);
                canvas_painter.rect_filled(r, 2.0, Color32::from_rgba_unmultiplied(255, 200, 60, 30));
                canvas_painter.rect_stroke(r, 2.0, Stroke::new(1.0, Color32::from_rgb(255, 200, 60)), egui::StrokeKind::Inside);
                canvas_painter.text(r.center(), egui::Align2::CENTER_CENTER, self.i18n.tr("layer-banner"), egui::FontId::proportional(13.0), Color32::from_rgb(255, 200, 60));
            }
            if Some(i) != sel
                && let Some(p) = resp.hover_pos()
                && editor::hit(&t.layers, to_frac(p), sel) == Some(i)
                && self.design.drag.is_none()
            {
                painter.rect_stroke(to_screen(l.rect()), 0.0, Stroke::new(1.0, Color32::from_white_alpha(140)), egui::StrokeKind::Outside);
            }
        }
        if let Some(i) = sel {
            let l = &t.layers[i];
            // В режиме кропа — где весь кадр исходника относительно слоя.
            if crop_mode
                && has_src
                && let Layer::MainVideo { crop, dest, .. } = l
            {
                let frame = editor::source_frame(*dest, main_video_crop(*crop, src, *dest, canvas));
                painter.rect_stroke(to_screen(frame), 0.0, Stroke::new(1.0, CROP_FRAME), egui::StrokeKind::Outside);
            }
            let r = to_screen(l.rect());
            painter.rect_stroke(r, 0.0, Stroke::new(1.5, SELECT), egui::StrokeKind::Outside);
            for h in Handle::RESIZE {
                let (ax, ay) = h.anchor();
                let c = Pos2::new(r.min.x + ax * r.width(), r.min.y + ay * r.height());
                let hr = egui::Rect::from_center_size(c, Vec2::splat(HANDLE_PX));
                painter.rect_filled(hr, 1.0, Color32::WHITE);
                painter.rect_stroke(hr, 1.0, Stroke::new(1.0, SELECT), egui::StrokeKind::Inside);
            }
        }
        let guide = Stroke::new(1.0, GUIDE);
        for &x in &self.design.guides.xs {
            canvas_painter.vline(screen.min.x + x * sw, screen.y_range(), guide);
        }
        for &y in &self.design.guides.ys {
            canvas_painter.hline(screen.x_range(), screen.min.y + y * sh, guide);
        }
    }
}

fn set_rect(l: &mut Layer, f: impl FnOnce(&mut Rect)) {
    match l {
        Layer::MainVideo { dest: r, .. }
        | Layer::Text { rect: r, .. }
        | Layer::Image { rect: r, .. }
        | Layer::Capsule { rect: r, .. }
        | Layer::Shape { rect: r, .. }
        | Layer::BannerSlot { rect: r }
        | Layer::Clip { rect: r, .. }
        | Layer::Subtitles { rect: r, .. } => f(r),
    }
}

/// Граница видео одним выбором (как у шаблона Terminal): нет / линия / тень. Тонкая настройка — строками ниже.
fn edge_preset(ui: &mut egui::Ui, tr: &I18n, border: &mut Option<Border>, shadow: &mut Option<Shadow>) {
    #[derive(PartialEq, Clone, Copy)]
    enum Edge {
        None,
        Line,
        Shadow,
        Both,
    }
    let now = match (border.is_some(), shadow.is_some()) {
        (false, false) => Edge::None,
        (true, false) => Edge::Line,
        (false, true) => Edge::Shadow,
        (true, true) => Edge::Both,
    };
    let mut v = now;
    row(ui, tr.tr("prop-edge"), |ui| {
        let options = [(Edge::None, "edge-none"), (Edge::Line, "edge-line"), (Edge::Shadow, "edge-shadow")];
        let options: &[(Edge, &str)] = if now == Edge::Both { &[(Edge::None, "edge-none"), (Edge::Line, "edge-line"), (Edge::Shadow, "edge-shadow"), (Edge::Both, "edge-both")] } else { &options };
        align_combo(ui, tr, "edge", &mut v, options);
    });
    if v != now {
        let line = || Border { width: 0.003, color: Color::rgb(0x39, 0xff, 0x7a) };
        let soft = || Shadow { color: Color([0, 0, 0, 0xc0]), blur: 0.03, dx: 0.0, dy: 0.0 };
        (*border, *shadow) = match v {
            Edge::None => (None, None),
            Edge::Line => (border.or_else(|| Some(line())), None),
            Edge::Shadow => (None, shadow.or_else(|| Some(soft()))),
            Edge::Both => (*border, *shadow),
        };
    }
}

fn bg_darken(bg: &Background) -> f32 {
    match bg {
        Background::Blur { darken, .. } | Background::Image { darken, .. } | Background::Video { darken, .. } => *darken,
        _ => 0.3,
    }
}

// ---- Панель свойств: виджеты правят копию слоя, сравнение с исходным решает, была ли правка. ----

fn grid(ui: &mut egui::Ui, id: &str, add: impl FnOnce(&mut egui::Ui)) {
    egui::Grid::new(id).num_columns(2).spacing([10.0, 6.0]).show(ui, add);
}

fn row(ui: &mut egui::Ui, label: String, add: impl FnOnce(&mut egui::Ui)) {
    ui.label(label);
    ui.horizontal(add);
    ui.end_row();
}

/// Длина в долях `scale` (ширины или высоты холста), показывается в пикселях холста.
fn px(ui: &mut egui::Ui, v: &mut f32, scale: f32, range: RangeInclusive<f32>) {
    let mut p = *v * scale;
    let r = ui.add(egui::DragValue::new(&mut p).range(range.start() * scale..=range.end() * scale).speed(1.0).max_decimals(1).suffix(" px"));
    if r.changed() {
        *v = p / scale;
    }
}

fn percent(ui: &mut egui::Ui, v: &mut f32) {
    let mut p = *v * 100.0;
    if ui.add(egui::DragValue::new(&mut p).range(0.0..=100.0).speed(0.5).max_decimals(0).suffix(" %")).changed() {
        *v = p / 100.0;
    }
}

pub(crate) fn color(ui: &mut egui::Ui, c: &mut Color) {
    let [r, g, b, a] = c.0;
    let mut c32 = Color32::from_rgba_unmultiplied(r, g, b, a);
    // Сравнение по ответу виджета: круговое преобразование premultiplied ↔ straight неточно при малой альфе.
    if egui::color_picker::color_edit_button_srgba(ui, &mut c32, egui::color_picker::Alpha::OnlyBlend).changed() {
        c.0 = c32.to_srgba_unmultiplied();
    }
}

fn rect_props(ui: &mut egui::Ui, tr: &I18n, r: &mut Rect, canvas: Canvas) {
    let (cw, ch) = (canvas.width as f32, canvas.height as f32);
    let one = |s: f32| 1.0 / s;
    row(ui, tr.tr("prop-position"), |ui| {
        let (w, h) = (r.w, r.h);
        px(ui, &mut r.x, cw, 0.0..=(1.0 - w).max(0.0));
        px(ui, &mut r.y, ch, 0.0..=(1.0 - h).max(0.0));
    });
    row(ui, tr.tr("prop-size"), |ui| {
        let (x, y) = (r.x, r.y);
        px(ui, &mut r.w, cw, one(cw)..=(1.0 - x));
        px(ui, &mut r.h, ch, one(ch)..=(1.0 - y));
    });
}

fn shadow_props(ui: &mut egui::Ui, tr: &I18n, sh: &mut Option<Shadow>, cw: f32) {
    let mut on = sh.is_some();
    row(ui, tr.tr("prop-shadow"), |ui| {
        ui.checkbox(&mut on, "");
        if let Some(s) = sh {
            color(ui, &mut s.color);
        }
    });
    if on != sh.is_some() {
        *sh = on.then_some(Shadow { color: Color([0, 0, 0, 0xb0]), blur: 0.01, dx: 0.0, dy: 0.004 });
    }
    if let Some(s) = sh {
        row(ui, tr.tr("prop-shadow-blur"), |ui| px(ui, &mut s.blur, cw, 0.0..=0.2));
        row(ui, tr.tr("prop-shadow-offset"), |ui| {
            px(ui, &mut s.dx, cw, -0.2..=0.2);
            px(ui, &mut s.dy, cw, -0.2..=0.2);
        });
    }
}

fn file_row(ui: &mut egui::Ui, tr: &I18n, label: &str, path: Option<&std::path::Path>, target: FileTarget, pick: &mut Option<FileTarget>) {
    row(ui, tr.tr(label), |ui| {
        let name = path.map_or_else(|| tr.tr("prop-no-file"), file_name);
        let r = ui.label(name);
        if let Some(p) = path {
            r.on_hover_text(p.display().to_string());
        }
        if ui.button(tr.tr("prop-pick")).clicked() {
            *pick = Some(target);
        }
    });
}

fn align_combo<T: PartialEq + Copy>(ui: &mut egui::Ui, tr: &I18n, id: &str, v: &mut T, options: &[(T, &str)]) {
    let current = options.iter().find(|(o, _)| o == v).map_or(String::new(), |(_, k)| tr.tr(k));
    egui::ComboBox::from_id_salt(id).selected_text(current).show_ui(ui, |ui| {
        for (o, k) in options {
            ui.selectable_value(v, *o, tr.tr(k));
        }
    });
}

fn layer_props(ui: &mut egui::Ui, tr: &I18n, l: &mut Layer, i: usize, canvas: Canvas, pick: &mut Option<FileTarget>) {
    let cw = canvas.width as f32;
    grid(ui, "props", |ui| {
        match l {
            Layer::MainVideo { crop, dest, corner_radius, border, shadow } => {
                rect_props(ui, tr, dest, canvas);
                row(ui, tr.tr("prop-crop"), |ui| {
                    ui.weak(format!("{:.0}% × {:.0}%", crop.w * 100.0, crop.h * 100.0));
                    if ui.add_enabled(*crop != Rect::FULL, egui::Button::new(tr.tr("prop-crop-reset"))).clicked() {
                        *crop = Rect::FULL;
                    }
                });
                row(ui, tr.tr("prop-corner-radius"), |ui| px(ui, corner_radius, cw, 0.0..=0.5));
                edge_preset(ui, tr, border, shadow);
                let mut on = border.is_some();
                row(ui, tr.tr("prop-border"), |ui| {
                    ui.checkbox(&mut on, "");
                    if let Some(b) = border {
                        color(ui, &mut b.color);
                        px(ui, &mut b.width, cw, 1.0 / cw..=0.1);
                    }
                });
                if on != border.is_some() {
                    *border = on.then_some(Border { width: 0.003, color: Color::WHITE });
                }
                shadow_props(ui, tr, shadow, cw);
            }
            Layer::Text { rect, content, font, size, color: c, accent_color, accent_mode, outline, shadow, align, valign } => {
                rect_props(ui, tr, rect, canvas);
                row(ui, tr.tr("prop-content"), |ui| {
                    ui.add(egui::TextEdit::multiline(content).desired_rows(2).desired_width(200.0));
                });
                font_props(ui, tr, font, i, pick);
                row(ui, tr.tr("prop-font-size"), |ui| px(ui, size, cw, 0.005..=0.5));
                row(ui, tr.tr("prop-color"), |ui| color(ui, c));
                row(ui, tr.tr("prop-accent"), |ui| {
                    color(ui, accent_color);
                    align_combo(ui, tr, "accent", accent_mode, &[(AccentMode::Color, "accent-color"), (AccentMode::Plate, "accent-plate")]);
                });
                row(ui, tr.tr("prop-align"), |ui| {
                    align_combo(ui, tr, "align", align, &[(Align::Left, "align-left"), (Align::Center, "align-center"), (Align::Right, "align-right")]);
                    align_combo(ui, tr, "valign", valign, &[(VAlign::Top, "valign-top"), (VAlign::Center, "valign-center"), (VAlign::Bottom, "valign-bottom")]);
                });
                let mut on = outline.is_some();
                row(ui, tr.tr("prop-outline"), |ui| {
                    ui.checkbox(&mut on, "");
                    if let Some(o) = outline {
                        color(ui, &mut o.color);
                        px(ui, &mut o.width, cw, 1.0 / cw..=0.05);
                    }
                });
                if on != outline.is_some() {
                    *outline = on.then_some(Outline { width: 0.004, color: Color::BLACK });
                }
                shadow_props(ui, tr, shadow, cw);
            }
            Layer::Image { rect, path, opacity } => {
                rect_props(ui, tr, rect, canvas);
                file_row(ui, tr, "prop-file", Some(path.as_path()), FileTarget::LayerImage(i), pick);
                row(ui, tr.tr("prop-opacity"), |ui| percent(ui, opacity));
            }
            Layer::Capsule { rect, logo, text, font, text_color, fill } => {
                rect_props(ui, tr, rect, canvas);
                file_row(ui, tr, "prop-logo", logo.as_deref(), FileTarget::CapsuleLogo(i), pick);
                if logo.is_some() {
                    row(ui, String::new(), |ui| {
                        if ui.button(tr.tr("prop-logo-remove")).clicked() {
                            *logo = None;
                        }
                    });
                }
                row(ui, tr.tr("prop-content"), |ui| {
                    ui.add(egui::TextEdit::singleline(text).desired_width(200.0));
                });
                font_props(ui, tr, font, i, pick);
                row(ui, tr.tr("prop-color"), |ui| color(ui, text_color));
                row(ui, tr.tr("prop-fill"), |ui| color(ui, fill));
            }
            Layer::Shape { rect, paint, corner_radius } => {
                rect_props(ui, tr, rect, canvas);
                paint_props(ui, tr, paint);
                row(ui, tr.tr("prop-corner-radius"), |ui| px(ui, corner_radius, cw, 0.0..=0.5));
            }
            Layer::BannerSlot { rect } | Layer::Clip { rect, .. } => rect_props(ui, tr, rect, canvas),
            Layer::Subtitles {
                rect,
                font,
                size,
                color: c,
                active_color,
                active_mode,
                keyword_color,
                dim_color,
                mode,
                words_per_line,
                lines,
                uppercase,
                outline,
                shadow,
                plate,
                align,
                valign,
            } => {
                rect_props(ui, tr, rect, canvas);
                row(ui, tr.tr("prop-sub-mode"), |ui| {
                    let modes = [
                        (SubtitleMode::Static, "sub-mode-static"),
                        (SubtitleMode::Highlight, "sub-mode-highlight"),
                        (SubtitleMode::Reveal, "sub-mode-reveal"),
                    ];
                    align_combo(ui, tr, "sub-mode", mode, &modes);
                });
                font_props(ui, tr, font, i, pick);
                row(ui, tr.tr("prop-font-size"), |ui| px(ui, size, cw, 0.005..=0.5));
                row(ui, tr.tr("prop-color"), |ui| color(ui, c));
                if *mode != SubtitleMode::Static {
                    row(ui, tr.tr("prop-sub-active"), |ui| {
                        color(ui, active_color);
                        align_combo(ui, tr, "sub-active", active_mode, &[(AccentMode::Color, "accent-color"), (AccentMode::Plate, "accent-plate")]);
                    });
                }
                row(ui, tr.tr("prop-sub-keyword"), |ui| color(ui, keyword_color));
                if *mode == SubtitleMode::Reveal {
                    row(ui, tr.tr("prop-sub-unspoken"), |ui| {
                        let mut hidden = dim_color.0[3] == 0;
                        if !hidden {
                            color(ui, dim_color);
                        }
                        if ui.checkbox(&mut hidden, tr.tr("prop-sub-hide")).changed() {
                            dim_color.0[3] = if hidden { 0 } else { 0x60 };
                        }
                    });
                }
                row(ui, tr.tr("prop-sub-layout"), |ui| {
                    let (w, l) = (SUBTITLE_WORDS_PER_LINE, SUBTITLE_LINES);
                    ui.add(egui::DragValue::new(words_per_line).range(*w.start()..=*w.end()).speed(0.05).suffix(tr.tr("prop-sub-words")));
                    ui.add(egui::DragValue::new(lines).range(*l.start()..=*l.end()).speed(0.05).suffix(tr.tr("prop-sub-lines")));
                });
                row(ui, tr.tr("prop-sub-uppercase"), |ui| {
                    ui.checkbox(uppercase, "");
                });
                let mut on = plate.is_some();
                row(ui, tr.tr("prop-sub-plate"), |ui| {
                    ui.checkbox(&mut on, "");
                    if let Some(p) = plate {
                        color(ui, p);
                    }
                });
                if on != plate.is_some() {
                    *plate = on.then_some(Color([0, 0, 0, 0xb0]));
                }
                row(ui, tr.tr("prop-align"), |ui| {
                    align_combo(ui, tr, "align", align, &[(Align::Left, "align-left"), (Align::Center, "align-center"), (Align::Right, "align-right")]);
                    align_combo(ui, tr, "valign", valign, &[(VAlign::Top, "valign-top"), (VAlign::Center, "valign-center"), (VAlign::Bottom, "valign-bottom")]);
                });
                let mut on = outline.is_some();
                row(ui, tr.tr("prop-outline"), |ui| {
                    ui.checkbox(&mut on, "");
                    if let Some(o) = outline {
                        color(ui, &mut o.color);
                        px(ui, &mut o.width, cw, 1.0 / cw..=0.05);
                    }
                });
                if on != outline.is_some() {
                    *outline = on.then_some(Outline { width: 0.005, color: Color::BLACK });
                }
                shadow_props(ui, tr, shadow, cw);
            }
        }
    });
    if matches!(l, Layer::Subtitles { .. }) {
        ui.weak(tr.tr("prop-sub-hint"));
    }
    if matches!(l, Layer::Text { .. } | Layer::Capsule { .. }) {
        ui.weak(tr.tr("prop-content-hint"));
    }
    if matches!(l, Layer::MainVideo { .. }) {
        ui.weak(tr.tr("design-crop-hint"));
    }
}

/// Шрифт: семейство (встроенные, свои, системные — с поиском), начертание из имеющихся у семейства, свой файл.
pub(crate) fn font_props(ui: &mut egui::Ui, tr: &I18n, font: &mut String, i: usize, pick: &mut Option<FileTarget>) {
    use sc_raster::fonts::{FontSource, font_name, is_generic, nearest_weight, split_font_name};
    let list = sc_raster::fonts::families();
    let fams = list.as_deref().map_or(&[][..], Vec::as_slice);
    let find = |n: &str| fams.iter().find(|f| f.name.eq_ignore_ascii_case(n));
    let (family, weight) = split_font_name(font, |n| find(n).is_some());
    let (family, weight) = (family.to_string(), weight);
    let entry = find(&family);
    let mut set = None;
    row(ui, tr.tr("prop-font"), |ui| {
        egui::ComboBox::from_id_salt("font-family")
            .selected_text(&family)
            .width(196.0)
            .height(400.0)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show_ui(ui, |ui| {
                if list.is_none() {
                    ui.weak(tr.tr("font-loading"));
                    return;
                }
                let id = ui.id().with("font-filter");
                let mut filter: String = ui.data_mut(|d| d.get_temp(id)).unwrap_or_default();
                let search = ui.add(egui::TextEdit::singleline(&mut filter).hint_text(tr.tr("font-search")).desired_width(f32::INFINITY));
                // Печать сразу идёт в поиск.
                if ui.memory(|m| m.focused().is_none()) {
                    search.request_focus();
                }
                ui.data_mut(|d| d.insert_temp(id, filter.clone()));
                let filter = filter.to_lowercase();
                let mut section = None;
                for f in fams.iter().filter(|f| f.name.to_lowercase().contains(&filter)) {
                    if section != Some(f.source) {
                        section = Some(f.source);
                        ui.separator();
                        ui.weak(tr.tr(match f.source {
                            FontSource::Bundled => "font-bundled",
                            FontSource::User => "font-user",
                            FontSource::System => "font-system",
                        }));
                    }
                    if ui.selectable_label(f.name.eq_ignore_ascii_case(&family), &f.name).clicked() {
                        set = Some(font_name(&f.name, nearest_weight(&f.weights, weight)));
                        ui.close();
                    }
                }
            });
    });
    row(ui, String::new(), |ui| {
        match entry {
            Some(f) if f.weights.len() > 1 => {
                egui::ComboBox::from_id_salt("font-weight").selected_text(tr.tr(&weight_key(weight))).show_ui(ui, |ui| {
                    for &w in &f.weights {
                        if ui.selectable_label(w == weight, tr.tr(&weight_key(w))).clicked() {
                            set = Some(font_name(&f.name, w));
                        }
                    }
                });
            }
            Some(_) => {
                ui.weak(tr.tr(&weight_key(weight)));
            }
            None if list.is_some() && !is_generic(&family) => {
                ui.colored_label(Color32::from_rgb(255, 170, 60), "⚠").on_hover_text(tr.tr("font-missing"));
            }
            None => {}
        }
        if ui.button(tr.tr("font-add-file")).on_hover_text(tr.tr("font-add-file-hint")).clicked() {
            *pick = Some(FileTarget::Font(i));
        }
    });
    if let Some(name) = set {
        *font = name;
    }
}

/// Ключ i18n начертания: 800 → `font-weight-extrabold`.
fn weight_key(w: u16) -> String {
    format!("font-weight-{}", sc_raster::fonts::weight_name(w).to_lowercase())
}

fn paint_props(ui: &mut egui::Ui, tr: &I18n, paint: &mut Paint) {
    let linear = matches!(paint, Paint::Linear { .. });
    let mut want = linear;
    row(ui, tr.tr("prop-paint"), |ui| {
        align_combo(ui, tr, "paint", &mut want, &[(false, "paint-solid"), (true, "paint-linear")]);
    });
    if want != linear {
        *paint = match *paint {
            Paint::Solid { color } => Paint::Linear { from: Color([color.0[0], color.0[1], color.0[2], 0]), to: color, angle: 0.0 },
            Paint::Linear { to, .. } => Paint::Solid { color: to },
        };
    }
    match paint {
        Paint::Solid { color: c } => row(ui, tr.tr("prop-color"), |ui| color(ui, c)),
        Paint::Linear { from, to, angle } => {
            row(ui, tr.tr("prop-gradient"), |ui| {
                color(ui, from);
                color(ui, to);
            });
            row(ui, tr.tr("prop-angle"), |ui| {
                ui.add(egui::DragValue::new(angle).range(-360.0..=360.0).speed(1.0).suffix("°"));
            });
        }
    }
}

fn background_props(ui: &mut egui::Ui, tr: &I18n, bg: &mut Background, pick: &mut Option<FileTarget>) {
    #[derive(Clone, Copy, PartialEq)]
    enum Kind {
        Blur,
        Color,
        Gradient,
        Image,
        Video,
    }
    let kind = match bg {
        Background::Blur { .. } => Kind::Blur,
        Background::Color { .. } => Kind::Color,
        Background::Gradient { .. } => Kind::Gradient,
        Background::Image { .. } => Kind::Image,
        Background::Video { .. } => Kind::Video,
    };
    let mut want = kind;
    grid(ui, "bg-props", |ui| {
        row(ui, tr.tr("prop-bg-type"), |ui| {
            align_combo(
                ui,
                tr,
                "bg-kind",
                &mut want,
                &[(Kind::Blur, "bg-blur"), (Kind::Color, "bg-color"), (Kind::Gradient, "bg-gradient"), (Kind::Image, "bg-image"), (Kind::Video, "bg-video")],
            );
        });
        match bg {
            Background::Blur { darken, scale } => {
                row(ui, tr.tr("prop-darken"), |ui| percent(ui, darken));
                row(ui, tr.tr("prop-zoom"), |ui| {
                    ui.add(egui::DragValue::new(scale).range(1.0..=4.0).speed(0.01).max_decimals(2).prefix("×"));
                });
            }
            Background::Color { color: c } => row(ui, tr.tr("prop-color"), |ui| color(ui, c)),
            Background::Gradient { from, to, angle } => {
                row(ui, tr.tr("prop-gradient"), |ui| {
                    color(ui, from);
                    color(ui, to);
                });
                row(ui, tr.tr("prop-angle"), |ui| {
                    ui.add(egui::DragValue::new(angle).range(-360.0..=360.0).speed(1.0).suffix("°"));
                });
            }
            Background::Image { path, darken } => {
                file_row(ui, tr, "prop-file", Some(path.as_path()), FileTarget::BackgroundImage, pick);
                row(ui, tr.tr("prop-darken"), |ui| percent(ui, darken));
            }
            Background::Video { path, darken } => {
                file_row(ui, tr, "prop-file", Some(path.as_path()), FileTarget::BackgroundVideo, pick);
                row(ui, tr.tr("prop-darken"), |ui| percent(ui, darken));
            }
        }
    });
    if want != kind {
        let darken = bg_darken(bg);
        match want {
            Kind::Blur => *bg = Background::Blur { darken, scale: 1.0 },
            Kind::Color => *bg = Background::Color { color: Color::rgb(0x10, 0x10, 0x12) },
            Kind::Gradient => *bg = Background::Gradient { from: Color::rgb(0x10, 0x18, 0x20), to: Color::BLACK, angle: 0.0 },
            // Картинке и видео нужен файл — фон сменится, когда его выберут.
            Kind::Image => *pick = Some(FileTarget::BackgroundImage),
            Kind::Video => *pick = Some(FileTarget::BackgroundVideo),
        }
    }
}
