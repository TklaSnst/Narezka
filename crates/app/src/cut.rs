//! Экран «Нарезка»: выбор ролика, превью 9:16, таймлайн, текст речи под ним, список шортсов и кандидатов
//! анализа (принять / отклонить / разделить / объединить), зоны пропуска и повторная нарезка.

use eframe::egui::{self, Color32, Key, Pos2, Rect, RichText, Sense, Vec2};
use sc_analysis::{BlockKind, Transcript};
use sc_core::{
    BatchError, Short, ShortFlag, ShortId, ShortStatus, SourceId, SourceStatus, Template, TimeRange,
};

use crate::App;
use crate::fmt;
use crate::import::file_name;
use crate::player::Player;
use crate::timeline::{self, Action, Block, Look, Timeline, Zone};

/// Действие кнопки, выполняемое после отрисовки (пока `self` занят интерфейсом).
type Deferred = Box<dyn FnOnce(&mut App)>;

/// Шаг перемотки по Shift+стрелкам, секунды.
const JUMP: f64 = 5.0;
/// Отступ от слова в паузу при клике по тексту, с (как у нарезки).
const WORD_PAD: f64 = 0.3;
/// Строк (сегментов whisper) в тексте под таймлайном и его высота.
const TEXT_LINES: usize = 6;
const TEXT_H: f32 = 118.0;
/// Сколько пикселей прокрутки колесом — одна строка текста.
const TEXT_SCROLL_STEP: f32 = 40.0;
/// Длина текста шортса в списке, символов.
const LIST_TEXT: usize = 160;

#[derive(Default)]
pub struct CutState {
    pub source: Option<SourceId>,
    pub selected: Option<ShortId>,
    pub mark_in: Option<f64>,
    pub mark_out: Option<f64>,
    pub timeline: Option<Timeline>,
    /// Ошибка последнего действия — ключ i18n.
    pub error: Option<&'static str>,
    /// Первая показанная строка текста и строка под курсором в прошлом кадре.
    text_first: usize,
    text_cur: Option<usize>,
    /// Накопленная прокрутка текста колесом, px.
    text_scroll: f32,
    /// Прокрутить список шортсов к выбранному (после выбора не из списка).
    scroll_to_selected: bool,
}

fn batch_error_key(e: &BatchError) -> &'static str {
    match e {
        BatchError::BadRange { .. } => "cut-err-bad-range",
        BatchError::OutOfSource { .. } => "cut-err-out-of-source",
        BatchError::Overlap(_) => "cut-err-overlap",
        BatchError::BadSplit => "cut-err-bad-split",
        BatchError::NothingToMerge => "cut-err-nothing-to-merge",
        BatchError::UnknownSource | BatchError::UnknownShort => "cut-err-unknown",
    }
}

fn flag_key(f: ShortFlag) -> &'static str {
    match f {
        ShortFlag::Unsure => "cut-flag-unsure",
        ShortFlag::NotStandalone => "cut-flag-not-standalone",
        ShortFlag::TooLong => "cut-flag-too-long",
        ShortFlag::NoScore => "cut-flag-no-score",
    }
}

fn zone_key(kind: BlockKind) -> &'static str {
    match kind {
        BlockKind::Intro => "cut-zone-intro",
        BlockKind::Outro => "cut-zone-outro",
        BlockKind::Ad => "cut-zone-ad",
        BlockKind::Skip | BlockKind::Content => "cut-zone-manual",
    }
}

/// Граница слов, ближайшая к `time`: середина паузы (или тишины по VAD) до или после слова под курсором.
fn word_cut(t: &Transcript, time: f64) -> f64 {
    let w = &t.words;
    let i = w.partition_point(|x| x.start <= time).saturating_sub(1);
    [i.checked_sub(1), Some(i)]
        .into_iter()
        .flatten()
        .filter_map(|k| sc_analysis::pause_after_word(t, k))
        .map(|p| p.cut())
        .min_by(|a, b| (a - time).abs().total_cmp(&(b - time).abs()))
        .unwrap_or(time)
}

/// Текст отрезка: слова, середина которых внутри; длиннее `max` символов — с многоточием.
fn text_between(t: &Transcript, start: f64, end: f64, max: usize) -> String {
    let first = t.words.partition_point(|w| (w.start + w.end) / 2.0 < start);
    let mut out = String::new();
    for w in t.words[first..].iter().take_while(|w| (w.start + w.end) / 2.0 < end) {
        if out.chars().count() + w.text.chars().count() >= max {
            out.push('…');
            break;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&w.text);
    }
    out
}

/// Зоны пропуска ролика: найденные анализом (интро/аутро/реклама) и размеченные вручную.
struct Zones {
    auto: Vec<(f64, f64, BlockKind)>,
    manual: Vec<TimeRange>,
}

impl Zones {
    fn contains(&self, t: f64) -> bool {
        self.auto.iter().any(|&(a, b, _)| a <= t && t < b) || self.manual.iter().any(|z| z.start <= t && t < z.end)
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
            && matches!(self.tab, crate::Tab::Cut | crate::Tab::Design)
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
    pub(crate) fn select_short(&mut self, id: Option<ShortId>) {
        self.cut.selected = id;
        self.cut.scroll_to_selected = true;
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

    /// Принять / отклонить выбранный; решённый кандидат — переход к следующему кандидату.
    fn set_selected_status(&mut self, status: ShortStatus) {
        let Some(id) = self.cut.selected else { return };
        let was = self.batch.short(id).map(|s| s.status);
        match self.batch.set_status(id, status) {
            Ok(()) => {
                if was == Some(ShortStatus::Candidate) {
                    self.select_next_candidate(id);
                }
            }
            Err(e) => self.cut.error = Some(batch_error_key(&e)),
        }
    }

    /// Следующий по времени кандидат: после выбранного шортса или, если выбора нет, после курсора.
    fn next_candidate(&mut self) {
        match (self.cut.selected, &self.player) {
            (Some(id), _) => self.select_next_candidate(id),
            (None, Some(p)) => self.select_candidate_after(p.state.time - 1e-3),
            _ => {}
        }
    }

    fn select_next_candidate(&mut self, after: ShortId) {
        if let Some(cur) = self.batch.short(after) {
            self.select_candidate_after(cur.start);
        }
    }

    fn select_candidate_after(&mut self, start: f64) {
        let Some(source) = self.cut.source else { return };
        let next = self
            .batch
            .shorts_of(source)
            .into_iter()
            .find(|s| s.status == ShortStatus::Candidate && s.start > start)
            .map(|s| s.id);
        if next.is_some() {
            self.select_short(next);
        }
    }

    /// Разделить выбранный шортс у курсора — по ближайшей границе слов, если есть текст.
    fn split_selected(&mut self) {
        let (Some(id), Some(source), Some(p)) = (self.cut.selected, self.cut.source, &self.player) else { return };
        let pos = p.state.time;
        let t = self.transcript_of(source).map_or(pos, |t| word_cut(&t, pos));
        match self.batch.split_short(id, t) {
            Ok(_) => self.select_short(Some(id)),
            Err(e) => self.cut.error = Some(batch_error_key(&e)),
        }
    }

    fn merge_selected(&mut self) {
        let Some(id) = self.cut.selected else { return };
        match self.batch.merge_with_next(id) {
            Ok(id) => self.select_short(Some(id)),
            Err(e) => self.cut.error = Some(batch_error_key(&e)),
        }
    }

    /// Зона пропуска между отметками (конец — курсор, если не отмечен).
    fn add_zone_from_marks(&mut self) {
        let (Some(source), Some(p)) = (self.cut.source, &self.player) else { return };
        let Some(a) = self.cut.mark_in else {
            self.cut.error = Some("cut-err-no-in");
            return;
        };
        let b = self.cut.mark_out.unwrap_or(p.state.time);
        let (start, end) = (a.min(b), a.max(b));
        if end - start < sc_core::MIN_SPLIT {
            self.cut.error = Some("cut-err-bad-range");
            return;
        }
        let Some(s) = self.batch.source_mut(source) else { return };
        s.skip.push(TimeRange { start, end });
        s.skip.sort_by(|x, y| x.start.total_cmp(&y.start));
        self.cut.mark_in = None;
        self.cut.mark_out = None;
    }

    /// Клик по слову: первый — начало (с отступом в паузу), второй — конец; курсор — на слово.
    fn click_word(&mut self, t: &Transcript, i: usize) {
        let (start, _) = sc_analysis::cutting::snap(t, i..i + 1, WORD_PAD);
        let (_, end) = sc_analysis::cutting::snap(t, i..i + 1, WORD_PAD);
        match (self.cut.mark_in, self.cut.mark_out) {
            (Some(a), None) if end > a => self.cut.mark_out = Some(end),
            _ => {
                self.cut.mark_in = Some(start);
                self.cut.mark_out = None;
            }
        }
        self.cut.error = None;
        self.seek_to(t.words[i].start, true);
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
            let keys = [
                Key::Space,
                Key::I,
                Key::O,
                Key::Enter,
                Key::Delete,
                Key::Backspace,
                Key::Escape,
                Key::ArrowLeft,
                Key::ArrowRight,
                Key::A,
                Key::R,
                Key::S,
                Key::M,
                Key::Z,
                Key::N,
            ];
            (i.modifiers.shift, keys.into_iter().filter(|k| i.key_pressed(*k)).collect::<Vec<_>>())
        });
        if !pressed.is_empty() {
            self.cut.error = None;
        }
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
                Key::A => self.set_selected_status(ShortStatus::Accepted),
                Key::R => self.set_selected_status(ShortStatus::Rejected),
                Key::S => self.split_selected(),
                Key::M => self.merge_selected(),
                Key::Z => self.add_zone_from_marks(),
                Key::N => self.next_candidate(),
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
                let n = self.batch.shorts_of(s.id).iter().filter(|s| s.status == ShortStatus::Accepted).count();
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

        egui::Panel::right("cut-list").resizable(true).min_size(300.0).default_size(380.0).show(ui, |ui| self.shorts_list(ui));
        egui::Panel::bottom("cut-timeline").show(ui, |ui| {
            ui.add_enabled_ui(ready, |ui| {
                self.transport(ui);
                self.timeline_ui(ui);
                self.transcript_ui(ui);
            });
        });
        egui::CentralPanel::default().show(ui, |ui| {
            let player = self.player.as_mut().unwrap();
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
            ui.separator();
            let sel = self.cut.selected.is_some();
            let buttons: [(&str, Option<&str>, Deferred); 5] = [
                ("cut-accept", None, Box::new(|a| a.set_selected_status(ShortStatus::Accepted))),
                ("cut-reject", None, Box::new(|a| a.set_selected_status(ShortStatus::Rejected))),
                ("cut-split", Some("cut-split-hint"), Box::new(App::split_selected)),
                ("cut-merge", Some("cut-merge-hint"), Box::new(App::merge_selected)),
                ("cut-delete-short", None, Box::new(App::delete_selected)),
            ];
            for (key, hint, f) in buttons {
                let r = ui.add_enabled(sel, egui::Button::new(self.i18n.tr(key)));
                let r = match hint {
                    Some(h) => r.on_hover_text(self.i18n.tr(h)),
                    None => r,
                };
                if r.clicked() {
                    act = Some(f);
                }
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
        let mut blocks: Vec<Block> = self
            .shorts_numbered(source)
            .into_iter()
            .map(|(s, n)| Block {
                id: s.id,
                start: s.start,
                end: s.end,
                label: match (s.status, n) {
                    (ShortStatus::Accepted, Some(n)) => format!("#{n}"),
                    (ShortStatus::Candidate, _) => s.score.map_or("?".into(), |x| format!("★{x:.0}")),
                    _ => String::new(),
                },
                color: timeline::block_color(n.unwrap_or(1) - 1),
                look: match s.status {
                    ShortStatus::Accepted => Look::Accepted,
                    ShortStatus::Candidate => Look::Candidate,
                    ShortStatus::Rejected => Look::Rejected,
                },
            })
            .collect();
        // Сверху — принятые, под ними кандидаты, ниже всех — отклонённые.
        blocks.sort_by_key(|b| match b.look {
            Look::Rejected => 0,
            Look::Candidate => 1,
            Look::Accepted => 2,
        });
        let z = self.zones(source);
        let zones: Vec<Zone> = (z.auto.iter().map(|&(start, end, _)| Zone { start, end, manual: false }))
            .chain(z.manual.iter().map(|r| Zone { start: r.start, end: r.end, manual: true }))
            .collect();
        let Some(tl) = &mut self.cut.timeline else { return };
        if !paused && !tl.is_dragging() {
            tl.view.follow(pos);
        }
        let actions = tl.show(ui, pos, &blocks, &zones, self.cut.selected, self.cut.mark_in, self.cut.mark_out);
        ui.weak(self.i18n.tr("cut-timeline-hint"));
        for a in actions {
            match a {
                Action::Seek { t, exact } => self.seek_to(t, exact),
                Action::Select(id) => self.select_short(Some(id)),
                Action::Resize { id, start, end } => self.resize_short(id, start, end),
            }
        }
    }

    /// Шортсы ролика по времени с номером среди принятых (как будут названы файлы).
    fn shorts_numbered(&self, source: SourceId) -> Vec<(Short, Option<usize>)> {
        let mut n = 0;
        self.batch
            .shorts_of(source)
            .into_iter()
            .map(|s| {
                let num = (s.status == ShortStatus::Accepted).then(|| {
                    n += 1;
                    n
                });
                (s.clone(), num)
            })
            .collect()
    }

    fn zones(&self, source: SourceId) -> Zones {
        let auto = self.cutting_of(source).map_or(vec![], |c| {
            (c.blocks.iter())
                .filter(|b| matches!(b.kind, BlockKind::Intro | BlockKind::Outro | BlockKind::Ad))
                .map(|b| (b.start, b.end, b.kind))
                .collect()
        });
        let manual = self.batch.source(source).map_or(vec![], |s| s.skip.clone());
        Zones { auto, manual }
    }

    /// Правая панель: шортсы и кандидаты, перекрытия, зоны пропуска и повторная нарезка.
    fn shorts_list(&mut self, ui: &mut egui::Ui) {
        let Some(source) = self.cut.source else { return };
        let transcript = self.transcript_of(source);
        let shorts = self.shorts_numbered(source);
        let count = |st: ShortStatus| shorts.iter().filter(|(s, _)| s.status == st).count() as i64;
        ui.add_space(4.0);
        ui.heading(self.i18n.tr("cut-list"));
        ui.label(self.i18n.tr_args(
            "cut-list-counts",
            &[
                ("accepted", count(ShortStatus::Accepted).into()),
                ("candidates", count(ShortStatus::Candidate).into()),
                ("rejected", count(ShortStatus::Rejected).into()),
            ],
        ));
        ui.checkbox(&mut self.batch.allow_overlap, self.i18n.tr("cut-allow-overlap"))
            .on_hover_text(self.i18n.tr("cut-allow-overlap-hint"));
        ui.separator();

        let has_cutting = self.cutting_of(source).is_some();
        let mut act: Option<Deferred> = None;
        // Нижняя часть (зоны) — фиксированной высоты, список — всё остальное.
        egui::Panel::bottom("cut-zones").resizable(false).show(ui, |ui| self.zones_ui(ui, source, &mut act));
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            if shorts.is_empty() {
                let key = if has_cutting { "cut-shorts-empty" } else { "cut-no-analysis" };
                ui.weak(self.i18n.tr(key));
            }
            for (s, num) in &shorts {
                let selected = self.cut.selected == Some(s.id);
                let frame = egui::Frame::group(ui.style()).fill(if selected {
                    ui.visuals().selection.bg_fill.gamma_multiply(0.35)
                } else {
                    Color32::TRANSPARENT
                });
                let resp = frame.show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        let (icon, color, key) = match s.status {
                            ShortStatus::Accepted => ("✔", Color32::from_rgb(90, 200, 110), "cut-status-accepted"),
                            ShortStatus::Candidate => ("?", Color32::from_rgb(140, 160, 200), "cut-status-candidate"),
                            ShortStatus::Rejected => ("✖", Color32::from_gray(120), "cut-status-rejected"),
                        };
                        ui.label(RichText::new(icon).color(color).strong()).on_hover_text(self.i18n.tr(key));
                        if let Some(n) = num {
                            ui.label(RichText::new(format!("#{n}")).strong());
                        }
                        if let Some(score) = s.score {
                            let r = ui.label(format!("★{score:.0}"));
                            let mut hint = self.i18n.tr_args("cut-score", &[("score", (score as i64).into())]);
                            if !s.reason.is_empty() {
                                hint = format!("{hint}\n{}", s.reason);
                            }
                            r.on_hover_text(hint);
                        }
                        ui.monospace(format!("{} · {}", fmt::duration(s.start), fmt::duration(s.duration())));
                        if !s.flags.is_empty() {
                            let hint: Vec<String> = s.flags.iter().map(|f| self.i18n.tr(flag_key(*f))).collect();
                            ui.label(RichText::new("⚠").color(Color32::from_rgb(235, 180, 60))).on_hover_text(hint.join("\n"));
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let id = s.id;
                            if s.status != ShortStatus::Rejected
                                && ui.small_button("✖").on_hover_text(self.i18n.tr("cut-reject")).clicked()
                            {
                                act = Some(Box::new(move |a| {
                                    a.select_short(Some(id));
                                    a.set_selected_status(ShortStatus::Rejected);
                                }));
                            }
                            if s.status != ShortStatus::Accepted
                                && ui.small_button("✔").on_hover_text(self.i18n.tr("cut-accept")).clicked()
                            {
                                act = Some(Box::new(move |a| {
                                    a.select_short(Some(id));
                                    a.set_selected_status(ShortStatus::Accepted);
                                }));
                            }
                        });
                    });
                    let title = if s.title.is_empty() { self.i18n.tr("cut-untitled") } else { s.title.clone() };
                    let mut title = RichText::new(title);
                    if s.status == ShortStatus::Rejected {
                        title = title.weak().strikethrough();
                    } else {
                        title = title.strong();
                    }
                    let mut clicked = ui.add(egui::Label::new(title).sense(Sense::click())).clicked();
                    if let Some(t) = &transcript {
                        let text = text_between(t, s.start, s.end, LIST_TEXT);
                        if !text.is_empty() {
                            clicked |= ui.add(egui::Label::new(RichText::new(text).weak().small()).sense(Sense::click())).clicked();
                        }
                    }
                    if !s.keywords.is_empty() {
                        ui.label(RichText::new(s.keywords.join(", ")).small().italics().weak());
                    }
                    clicked
                });
                if resp.inner {
                    let id = s.id;
                    act = Some(Box::new(move |a| {
                        a.select_short(Some(id));
                        a.cut.scroll_to_selected = false;
                    }));
                }
                if selected && self.cut.scroll_to_selected {
                    resp.response.scroll_to_me(None);
                    self.cut.scroll_to_selected = false;
                }
            }
        });
        if let Some(f) = act {
            self.cut.error = None;
            f(self);
        }
    }

    fn zones_ui(&self, ui: &mut egui::Ui, source: SourceId, act: &mut Option<Deferred>) {
        ui.add_space(4.0);
        ui.strong(self.i18n.tr("cut-zones"));
        let z = self.zones(source);
        if z.auto.is_empty() && z.manual.is_empty() {
            ui.weak(self.i18n.tr("cut-zones-empty"));
        }
        let mut rows: Vec<(f64, f64, &'static str, Option<usize>)> =
            z.auto.iter().map(|&(a, b, k)| (a, b, zone_key(k), None)).collect();
        rows.extend(z.manual.iter().enumerate().map(|(i, r)| (r.start, r.end, "cut-zone-manual", Some(i))));
        rows.sort_by(|x, y| x.0.total_cmp(&y.0));
        for (a, b, key, manual) in rows {
            ui.horizontal(|ui| {
                let r = ui.link(format!("{}–{}", fmt::duration(a), fmt::duration(b)));
                if r.clicked() {
                    *act = Some(Box::new(move |app| app.seek_to(a, true)));
                }
                ui.label(self.i18n.tr(key));
                if let Some(i) = manual
                    && ui.small_button("🗑").on_hover_text(self.i18n.tr("cut-zone-remove")).clicked()
                {
                    *act = Some(Box::new(move |app| {
                        if let Some(s) = app.batch.source_mut(source)
                            && i < s.skip.len()
                        {
                            s.skip.remove(i);
                        }
                    }));
                }
            });
        }
        ui.horizontal(|ui| {
            if ui.add_enabled(self.cut.mark_in.is_some(), egui::Button::new(self.i18n.tr("cut-zone-add"))).clicked() {
                *act = Some(Box::new(App::add_zone_from_marks));
            }
        });
        ui.add_space(4.0);
        let cutting = self.cutting_of(source);
        let changed = cutting.as_ref().is_some_and(|c| {
            let now: Vec<(f64, f64)> = z.manual.iter().map(|r| (r.start, r.end)).collect();
            c.skips != now
        });
        ui.horizontal_wrapped(|ui| match self.analysis_busy(source) {
            Some(Some(stage)) => {
                ui.spinner();
                ui.label(self.i18n.tr_args("cut-recut-running", &[("stage", stage.into())]));
            }
            Some(None) => {
                ui.label(self.i18n.tr("cut-recut-queued"));
            }
            None => {
                let can = self.transcript_of(source).is_some();
                let r = ui.add_enabled(can, egui::Button::new(self.i18n.tr("cut-recut")));
                if r.on_hover_text(self.i18n.tr("cut-recut-hint")).clicked() {
                    *act = Some(Box::new(move |a| a.recut(source)));
                }
                if changed {
                    ui.colored_label(Color32::from_rgb(235, 180, 60), self.i18n.tr("cut-zones-changed"));
                }
            }
        });
        ui.add_space(4.0);
    }

    /// Текст речи под таймлайном: строки whisper вокруг курсора; слово под курсором подсвечено,
    /// клик по слову ставит отметки.
    fn transcript_ui(&mut self, ui: &mut egui::Ui) {
        let (Some(source), Some(p)) = (self.cut.source, &self.player) else { return };
        let pos = p.state.time;
        ui.add_space(2.0);
        let Some(t) = self.transcript_of(source).filter(|t| !t.segments.is_empty()) else {
            ui.weak(self.i18n.tr("cut-text-none"));
            return;
        };
        let segs = &t.segments;
        // Текст листается, когда курсор уходит за показанные строки.
        let cur = segs.partition_point(|s| s.start <= pos).saturating_sub(1);
        if self.cut.text_cur != Some(cur) {
            self.cut.text_cur = Some(cur);
            if !(self.cut.text_first..self.cut.text_first + TEXT_LINES).contains(&cur) {
                self.cut.text_first = cur.saturating_sub(1);
            }
        }
        let max_first = segs.len().saturating_sub(TEXT_LINES);
        self.cut.text_first = self.cut.text_first.min(max_first);

        let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), TEXT_H), Sense::hover());
        ui.painter().rect_filled(rect, 2.0, ui.visuals().extreme_bg_color);
        if resp.hovered() {
            self.cut.text_scroll += ui.input(|i| i.smooth_scroll_delta.y);
            while self.cut.text_scroll.abs() >= TEXT_SCROLL_STEP {
                let up = self.cut.text_scroll > 0.0;
                self.cut.text_scroll -= TEXT_SCROLL_STEP.copysign(self.cut.text_scroll);
                self.cut.text_first = if up { self.cut.text_first.saturating_sub(1) } else { (self.cut.text_first + 1).min(max_first) };
            }
        }

        let zones = self.zones(source);
        let selected = self.cut.selected.and_then(|id| self.batch.short(id)).map(|s| (s.start, s.end));
        let accepted: Vec<(f64, f64)> = (self.batch.shorts_of(source).into_iter())
            .filter(|s| s.status == ShortStatus::Accepted)
            .map(|s| (s.start, s.end))
            .collect();
        let marks = match (self.cut.mark_in, self.cut.mark_out) {
            (Some(a), Some(b)) => Some((a.min(b), a.max(b))),
            (Some(a), None) => Some((a, a.max(pos))),
            _ => None,
        };
        let inside = |r: Option<(f64, f64)>, x: f64| r.is_some_and(|(a, b)| a <= x && x < b);
        let visuals = ui.visuals().clone();

        let mut clicked = None;
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink(6.0)));
        child.set_clip_rect(rect);
        for seg in &segs[self.cut.text_first..(self.cut.text_first + TEXT_LINES).min(segs.len())] {
            child.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.label(RichText::new(fmt::duration(seg.start)).monospace().small().weak());
                for i in seg.words.clone() {
                    let w = &t.words[i];
                    let mid = (w.start + w.end) / 2.0;
                    let mut text = RichText::new(&w.text);
                    if w.start <= pos && pos < w.end.max(w.start + 0.05) {
                        text = text.background_color(Color32::from_rgb(235, 200, 70)).color(Color32::BLACK);
                    } else if inside(marks, mid) {
                        text = text.background_color(Color32::from_rgb(80, 200, 120).gamma_multiply(0.3));
                    }
                    if zones.contains(mid) {
                        text = text.strikethrough().color(visuals.weak_text_color());
                    } else if inside(selected, mid) {
                        text = text.strong().color(Color32::WHITE);
                    } else if !accepted.iter().any(|&r| inside(Some(r), mid)) {
                        text = text.color(visuals.weak_text_color());
                    }
                    if ui.add(egui::Label::new(text).sense(Sense::click())).clicked() {
                        clicked = Some(i);
                    }
                }
            });
        }
        ui.weak(self.i18n.tr("cut-text-hint"));
        if let Some(i) = clicked {
            self.click_word(&t, i);
        }
    }
}

/// Холст 9:16 по шаблону, собранный композитором; размер на экране сообщается плееру.
pub(crate) fn preview_canvas(ui: &egui::Ui, area: Rect, player: &mut Player, template: &Template, empty_text: &str) -> Rect {
    let (cw, ch) = (template.canvas.width as f32, template.canvas.height as f32);
    let mut size = Vec2::new(area.height() * cw / ch, area.height());
    if size.x > area.width() {
        size = Vec2::new(area.width(), area.width() * ch / cw);
    }
    // Холст — по целым физическим пикселям: текстура показывается один к одному, без пересэмплирования.
    let ppp = ui.ctx().pixels_per_point();
    let (pw, ph) = ((size.x * ppp).floor().max(2.0), (size.y * ppp).floor().max(2.0));
    let canvas = Rect::from_center_size(area.center(), Vec2::new(pw, ph) / ppp);
    player.request_canvas(pw as u32, ph as u32);

    let painter = ui.painter_at(canvas);
    painter.rect_filled(canvas, 0.0, Color32::BLACK);
    match player.canvas_texture() {
        Some(tex) => {
            let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
            painter.image(tex, canvas, uv, Color32::WHITE);
        }
        None => {
            painter.text(canvas.center(), egui::Align2::CENTER_CENTER, empty_text, egui::FontId::proportional(16.0), Color32::GRAY);
        }
    }
    ui.painter().rect_stroke(canvas, 0.0, egui::Stroke::new(1.0, Color32::from_gray(70)), egui::StrokeKind::Outside);
    canvas
}

#[cfg(test)]
mod tests {
    use super::*;
    use sc_analysis::Word;

    /// Слова «а б в г» по 1 с с паузами по 1 с: 0–1, 2–3, 4–5, 6–7.
    fn transcript() -> Transcript {
        let words = ["а", "б", "в", "г"]
            .iter()
            .enumerate()
            .map(|(i, w)| Word { text: (*w).into(), start: i as f64 * 2.0, end: i as f64 * 2.0 + 1.0, dtw: None, p: 1.0 })
            .collect();
        Transcript {
            version: sc_analysis::transcript::FORMAT_VERSION,
            model: String::new(),
            language: "ru".into(),
            prompt: String::new(),
            words,
            segments: vec![],
            speech: vec![],
        }
    }

    #[test]
    fn split_point_snaps_to_word_boundary() {
        let t = transcript();
        // Внутри слова «б» (2–3): ближе к паузе до него (1.5) или после (3.5).
        assert_eq!(word_cut(&t, 2.2), 1.5);
        assert_eq!(word_cut(&t, 2.9), 3.5);
        // В паузе — её середина.
        assert_eq!(word_cut(&t, 3.9), 3.5);
        // После последнего слова пауз нет дальше — ближайшая назад.
        assert_eq!(word_cut(&t, 9.0), 5.5);
    }

    #[test]
    fn short_text_by_word_middles() {
        let t = transcript();
        assert_eq!(text_between(&t, 1.8, 5.0, 100), "б в");
        assert_eq!(text_between(&t, 0.0, 8.0, 4), "а б…");
        assert_eq!(text_between(&t, 7.5, 8.0, 100), "");
    }
}
