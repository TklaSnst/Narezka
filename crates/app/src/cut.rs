//! Экран «Нарезка»: выбор ролика, превью 9:16, таймлайн, ручные шортсы.

use eframe::egui::{self, Color32, Key, Pos2, Rect, Vec2};
use sc_core::{Background, BatchError, Layer, Short, ShortId, SourceId, SourceStatus, Template, main_video_crop};

use crate::App;
use crate::fmt;
use crate::import::file_name;
use crate::player::Player;
use crate::timeline::{self, Action, Block, Timeline};

/// Действие кнопки, выполняемое после отрисовки (пока `self` занят интерфейсом).
type Deferred = Box<dyn FnOnce(&mut App)>;

/// Шаг перемотки по Shift+стрелкам, секунды.
const JUMP: f64 = 5.0;

#[derive(Default)]
pub struct CutState {
    pub source: Option<SourceId>,
    pub selected: Option<ShortId>,
    pub mark_in: Option<f64>,
    pub mark_out: Option<f64>,
    pub timeline: Option<Timeline>,
    /// Ошибка последнего действия — ключ i18n.
    pub error: Option<&'static str>,
}

fn batch_error_key(e: &BatchError) -> &'static str {
    match e {
        BatchError::BadRange { .. } => "cut-err-bad-range",
        BatchError::OutOfSource { .. } => "cut-err-out-of-source",
        BatchError::Overlap(_) => "cut-err-overlap",
        BatchError::UnknownSource | BatchError::UnknownShort => "cut-err-unknown",
    }
}

/// Ролик можно смотреть и резать: прочитан (анализ идёт или закончен — тоже).
fn playable(status: &SourceStatus) -> bool {
    matches!(status, SourceStatus::Ready | SourceStatus::Analyzing | SourceStatus::Analyzed)
}

impl App {
    /// Каждый кадр, на любой вкладке: исходник мог исчезнуть из пачки или ещё не выбран.
    pub(crate) fn cut_sync(&mut self) {
        let alive = self.cut.source.and_then(|id| self.batch.source(id)).is_some_and(|s| playable(&s.status));
        if !alive {
            self.cut = CutState::default();
            if let Some(p) = &mut self.player {
                p.unload();
            }
        }
        if self.cut.source.is_none()
            && self.tab == crate::Tab::Cut
            && let Some(id) = self.batch.sources.iter().find(|s| playable(&s.status)).map(|s| s.id)
        {
            self.select_source(id);
        }
        // Шортс мог удалиться (вместе с исходником или иначе).
        if self.cut.selected.is_some_and(|id| self.batch.short(id).is_none()) {
            self.select_short(None);
        }
    }

    fn select_source(&mut self, id: SourceId) {
        let Some(s) = self.batch.source(id) else { return };
        let duration = s.info.as_ref().map_or(0.0, |i| i.duration);
        let path = s.path.clone();
        self.cut = CutState { source: Some(id), timeline: Some(Timeline::new(duration)), ..Default::default() };
        if let Some(p) = &mut self.player {
            p.load(&path);
        }
    }

    /// Выбрать шортс: курсор — в его начало, воспроизведение — по кругу внутри него.
    fn select_short(&mut self, id: Option<ShortId>) {
        self.cut.selected = id;
        let Some(p) = &mut self.player else { return };
        match id.and_then(|id| self.batch.short(id)) {
            Some(s) => {
                p.set_range(s.start, s.end);
                p.seek(s.start, true);
            }
            None => p.clear_range(),
        }
    }

    fn add_short_from_marks(&mut self) {
        let (Some(source), Some(p)) = (self.cut.source, &self.player) else { return };
        let Some(a) = self.cut.mark_in else {
            self.cut.error = Some("cut-err-no-in");
            return;
        };
        let b = self.cut.mark_out.unwrap_or(p.state.time);
        match self.batch.add_short(Short::manual(source, a.min(b), a.max(b))) {
            Ok(id) => {
                self.cut.mark_in = None;
                self.cut.mark_out = None;
                self.cut.error = None;
                self.select_short(Some(id));
            }
            Err(e) => self.cut.error = Some(batch_error_key(&e)),
        }
    }

    fn delete_selected(&mut self) {
        if let Some(id) = self.cut.selected {
            let _ = self.batch.remove_short(id);
            self.select_short(None);
        }
    }

    fn resize_short(&mut self, id: ShortId, start: f64, end: f64) {
        let Some(mut s) = self.batch.short(id).cloned() else { return };
        s.start = start;
        s.end = end;
        // Неверное положение (перекрытие, ноль длины) просто не применяется — блок упирается.
        if self.batch.update_short(s).is_ok()
            && let Some(p) = &mut self.player
        {
            p.set_range(start, end);
        }
    }

    fn seek_to(&mut self, t: f64, exact: bool) {
        let Some(p) = &mut self.player else { return };
        // Перемотка за пределы выбранного шортса снимает выбор (и петлю).
        if let Some((a, b)) = p.range()
            && !(a..=b).contains(&t)
        {
            self.cut.selected = None;
            p.clear_range();
        }
        p.seek(t, exact);
    }

    fn cut_keys(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let Some(p) = &self.player else { return };
        if !p.is_ready() {
            return;
        }
        let pos = p.state.time;
        let (shift, pressed) = ctx.input(|i| {
            let keys = [Key::Space, Key::I, Key::O, Key::Enter, Key::Delete, Key::Backspace, Key::Escape, Key::ArrowLeft, Key::ArrowRight];
            (i.modifiers.shift, keys.into_iter().filter(|k| i.key_pressed(*k)).collect::<Vec<_>>())
        });
        for key in pressed {
            match key {
                Key::Space => self.player.as_mut().unwrap().toggle_pause(),
                Key::I => self.cut.mark_in = Some(pos),
                Key::O => self.cut.mark_out = Some(pos),
                Key::Enter => self.add_short_from_marks(),
                Key::Delete | Key::Backspace => self.delete_selected(),
                Key::Escape => {
                    self.cut.mark_in = None;
                    self.cut.mark_out = None;
                    self.select_short(None);
                }
                Key::ArrowLeft if shift => self.seek_to(pos - JUMP, true),
                Key::ArrowRight if shift => self.seek_to(pos + JUMP, true),
                Key::ArrowLeft => self.player.as_mut().unwrap().frame_step(false),
                Key::ArrowRight => self.player.as_mut().unwrap().frame_step(true),
                _ => {}
            }
        }
    }

    pub(crate) fn cut_ui(&mut self, ui: &mut egui::Ui) {
        self.cut_keys(&ui.ctx().clone());

        egui::Panel::left("cut-sources").resizable(true).default_size(220.0).show(ui, |ui| {
            ui.heading(self.i18n.tr("cut-sources"));
            ui.add_space(4.0);
            if self.batch.sources.is_empty() {
                ui.label(self.i18n.tr("cut-no-sources"));
            }
            let mut pick = None;
            for s in &self.batch.sources {
                let ready = playable(&s.status);
                let mut text = file_name(&s.path);
                if let Some(info) = &s.info {
                    text = format!("{text}\n{}", fmt::duration(info.duration));
                }
                let n = self.batch.shorts_of(s.id).len();
                if n > 0 {
                    text = format!("{text} · {}", self.i18n.tr_args("cut-shorts-count", &[("n", (n as i64).into())]));
                }
                let r = ui.add_enabled(ready, egui::Button::selectable(self.cut.source == Some(s.id), text));
                let r = if ready { r } else { r.on_disabled_hover_text(self.i18n.tr("cut-source-not-ready")) };
                if r.clicked() {
                    pick = Some(s.id);
                }
            }
            if let Some(id) = pick
                && self.cut.source != Some(id)
            {
                self.select_source(id);
            }
        });

        let Some(player) = &self.player else {
            egui::CentralPanel::default().show(ui, |ui| {
                let err = self.player_error.clone().unwrap_or_default();
                ui.colored_label(ui.visuals().error_fg_color, self.i18n.tr_args("cut-player-failed", &[("error", err.into())]));
            });
            return;
        };
        let ready = player.is_ready();

        egui::Panel::bottom("cut-shorts").show(ui, |ui| self.shorts_strip(ui));
        egui::Panel::bottom("cut-timeline").show(ui, |ui| {
            ui.add_enabled_ui(ready, |ui| {
                self.transport(ui);
                self.timeline_ui(ui);
            });
        });
        egui::CentralPanel::default().show(ui, |ui| {
            let player = self.player.as_ref().unwrap();
            if let Some(e) = &player.state.error {
                ui.colored_label(ui.visuals().error_fg_color, self.i18n.tr_args("cut-open-failed", &[("error", e.clone().into())]));
            }
            let rect = ui.available_rect_before_wrap();
            let empty = self.i18n.tr(if self.cut.source.is_none() { "cut-preview-empty" } else { "cut-preview-loading" });
            preview_canvas(ui, rect, player, &self.batch.template, &empty);
        });
    }

    fn transport(&mut self, ui: &mut egui::Ui) {
        let Some(p) = &self.player else { return };
        let (time, duration, paused) = (p.state.time, p.state.duration, p.state.paused);
        let mut act: Option<Deferred> = None;
        ui.horizontal(|ui| {
            let play = if paused { "▶" } else { "⏸" };
            if ui.button(play).on_hover_text(self.i18n.tr("cut-play")).clicked() {
                act = Some(Box::new(|a| a.player.as_mut().unwrap().toggle_pause()));
            }
            if ui.button("⏮").on_hover_text(self.i18n.tr("cut-frame-back")).clicked() {
                act = Some(Box::new(|a| a.player.as_mut().unwrap().frame_step(false)));
            }
            if ui.button("⏭").on_hover_text(self.i18n.tr("cut-frame-fwd")).clicked() {
                act = Some(Box::new(|a| a.player.as_mut().unwrap().frame_step(true)));
            }
            ui.monospace(format!("{} / {}", fmt::clock_ms(time), fmt::clock_ms(duration)));
            ui.separator();

            if ui.button(self.i18n.tr("cut-mark-in")).clicked() {
                self.cut.mark_in = Some(time);
            }
            ui.monospace(self.cut.mark_in.map_or("—".into(), fmt::clock_ms));
            if ui.button(self.i18n.tr("cut-mark-out")).clicked() {
                self.cut.mark_out = Some(time);
            }
            ui.monospace(self.cut.mark_out.map_or("—".into(), fmt::clock_ms));
            if ui.add_enabled(self.cut.mark_in.is_some(), egui::Button::new(self.i18n.tr("cut-add-short"))).clicked() {
                act = Some(Box::new(App::add_short_from_marks));
            }
            if ui.add_enabled(self.cut.selected.is_some(), egui::Button::new(self.i18n.tr("cut-delete-short"))).clicked() {
                act = Some(Box::new(App::delete_selected));
            }
            ui.separator();
            if ui.button(self.i18n.tr("cut-zoom-fit")).clicked()
                && let Some(t) = &mut self.cut.timeline
            {
                t.view.fit();
            }
            if let Some(key) = self.cut.error {
                ui.separator();
                ui.colored_label(ui.visuals().error_fg_color, self.i18n.tr(key));
            }
        });
        if let Some(f) = act {
            self.cut.error = None;
            f(self);
        }
    }

    fn timeline_ui(&mut self, ui: &mut egui::Ui) {
        let (Some(source), Some(p)) = (self.cut.source, &self.player) else { return };
        let (pos, paused) = (p.state.time, p.state.paused);
        let blocks: Vec<Block> = self
            .batch
            .shorts_of(source)
            .iter()
            .enumerate()
            .map(|(i, s)| Block {
                id: s.id,
                start: s.start,
                end: s.end,
                label: format!("#{}", i + 1),
                color: timeline::block_color(i),
            })
            .collect();
        let Some(tl) = &mut self.cut.timeline else { return };
        if !paused && !tl.is_dragging() {
            tl.view.follow(pos);
        }
        let actions = tl.show(ui, pos, &blocks, self.cut.selected, self.cut.mark_in, self.cut.mark_out);
        ui.weak(self.i18n.tr("cut-timeline-hint"));
        for a in actions {
            match a {
                Action::Seek { t, exact } => self.seek_to(t, exact),
                Action::Select(id) => self.select_short(Some(id)),
                Action::Resize { id, start, end } => self.resize_short(id, start, end),
            }
        }
    }

    fn shorts_strip(&mut self, ui: &mut egui::Ui) {
        let Some(source) = self.cut.source else { return };
        let shorts: Vec<(ShortId, f64, f64)> = self.batch.shorts_of(source).iter().map(|s| (s.id, s.start, s.end)).collect();
        ui.add_space(4.0);
        if shorts.is_empty() {
            ui.weak(self.i18n.tr("cut-shorts-empty"));
            ui.add_space(4.0);
            return;
        }
        let mut pick = None;
        egui::ScrollArea::horizontal().show(ui, |ui| {
            ui.horizontal(|ui| {
                for (i, (id, start, end)) in shorts.into_iter().enumerate() {
                    let text = format!("#{}  {}–{}  ({})", i + 1, fmt::duration(start), fmt::duration(end), fmt::duration(end - start));
                    let selected = self.cut.selected == Some(id);
                    let text = egui::RichText::new(text).color(if selected { Color32::WHITE } else { timeline::block_color(i) });
                    if ui.add(egui::Button::selectable(selected, text)).clicked() {
                        pick = Some(id);
                    }
                }
            });
        });
        ui.add_space(4.0);
        if let Some(id) = pick {
            self.select_short(Some(id));
        }
    }
}

/// Холст 9:16 по шаблону: фон (размытый кадр или цвет) и кадр основного видео.
fn preview_canvas(ui: &egui::Ui, area: Rect, player: &Player, template: &Template, empty_text: &str) {
    let (cw, ch) = (template.canvas.width as f32, template.canvas.height as f32);
    let mut size = Vec2::new(area.height() * cw / ch, area.height());
    if size.x > area.width() {
        size = Vec2::new(area.width(), area.width() * ch / cw);
    }
    let canvas = Rect::from_center_size(area.center(), size);
    let painter = ui.painter_at(canvas);
    let uv_full = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
    let to_screen = |r: sc_core::Rect| {
        Rect::from_min_size(
            canvas.min + Vec2::new(r.x * canvas.width(), r.y * canvas.height()),
            Vec2::new(r.w * canvas.width(), r.h * canvas.height()),
        )
    };

    painter.rect_filled(canvas, 0.0, Color32::BLACK);
    match &template.background {
        Background::Blur { .. } => {
            if let Some(bg) = player.background_texture() {
                painter.image(bg, canvas, uv_full, Color32::WHITE);
            }
        }
        Background::Color { color } => {
            let [r, g, b, a] = color.0;
            painter.rect_filled(canvas, 0.0, Color32::from_rgba_unmultiplied(r, g, b, a));
        }
    }

    match (player.video_texture(), player.src_size()) {
        (Some(tex), Some(src)) => {
            for layer in &template.layers {
                let Layer::MainVideo { crop, dest, corner_radius } = layer;
                let uv = main_video_crop(*crop, src, *dest, template.canvas);
                let uv = Rect::from_min_size(Pos2::new(uv.x, uv.y), Vec2::new(uv.w, uv.h));
                let radius = corner_radius * canvas.width();
                painter.add(egui::epaint::RectShape::filled(to_screen(*dest), radius, Color32::WHITE).with_texture(tex, uv));
            }
        }
        _ => {
            painter.text(canvas.center(), egui::Align2::CENTER_CENTER, empty_text, egui::FontId::proportional(16.0), Color32::GRAY);
        }
    }
    ui.painter().rect_stroke(canvas, 0.0, egui::Stroke::new(1.0, Color32::from_gray(70)), egui::StrokeKind::Outside);
}
