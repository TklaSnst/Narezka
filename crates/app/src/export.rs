//! Экран «Экспорт»: папка и имя файлов, формат, очередь рендера принятых шортсов
//! (N задач параллельно, прогресс по файлам, пауза, отмена, лог ошибки, политика конфликтов имён).

use std::path::PathBuf;
use std::time::Instant;

use eframe::egui;
use sc_core::{Batch, ConflictPolicy, TextVars};
use sc_render::{Encoder, Queue, QueueEvent, RenderError, RenderJob, plan_outputs_with};

use crate::App;
use crate::fmt;
use crate::import::file_name;
use crate::jobs::Msg;

/// Состояние файла в очереди.
enum FileState {
    Waiting,
    Running(f32),
    Done,
    Failed { tail: String, log: String },
    Cancelled,
}

struct QueuedFile {
    dest: PathBuf,
    input: PathBuf,
    /// Длительность шортса — вес в общем прогрессе.
    duration: f64,
    state: FileState,
    started: Option<Instant>,
    /// Время рендера, с (у законченных).
    elapsed: Option<f64>,
}

impl QueuedFile {
    fn finished(&self) -> bool {
        matches!(self.state, FileState::Done | FileState::Failed { .. } | FileState::Cancelled)
    }

    fn fraction(&self) -> f32 {
        match self.state {
            FileState::Waiting => 0.0,
            FileState::Running(f) => f,
            _ => 1.0,
        }
    }
}

struct Run {
    queue: Queue,
    files: Vec<QueuedFile>,
    /// Очередь закончилась (событие `Done`).
    done: bool,
}

/// Диалог «файлы уже есть» (политика «Спросить»).
struct AskConflict {
    files: Vec<PathBuf>,
}

#[derive(Default)]
pub struct ExportState {
    run: Option<Run>,
    ask: Option<AskConflict>,
    /// Открытое окно лога: номер файла в очереди.
    log: Option<usize>,
}

impl ExportState {
    pub fn is_running(&self) -> bool {
        self.run.as_ref().is_some_and(|r| !r.done)
    }

    pub fn is_paused(&self) -> bool {
        self.run.as_ref().is_some_and(|r| !r.done && r.queue.is_paused())
    }

    /// Общая доля (по длительности), сколько файлов закончено и сколько всего — для строки состояния.
    pub fn overall(&self) -> Option<(f32, usize, usize)> {
        let r = self.run.as_ref().filter(|r| !r.done)?;
        let total: f64 = r.files.iter().map(|f| f.duration.max(0.1)).sum();
        let done: f64 = r.files.iter().map(|f| f.duration.max(0.1) * f.fraction() as f64).sum();
        let finished = r.files.iter().filter(|f| f.finished()).count();
        Some(((done / total.max(1e-9)) as f32, finished, r.files.len()))
    }
}

impl App {
    fn encoder(&self) -> Option<Encoder> {
        let caps = self.caps.as_ref().filter(|c| c.usable())?;
        let out = &self.batch.output;
        if !caps.has_audio_encoder(sc_render::encode::audio_encoder(out.audio.codec)) {
            return None;
        }
        let e = caps.encoder_for(out.codec, out.encoder)?;
        let vaapi_device = (e.kind == sc_media::EncoderKind::Gpu).then(|| caps.vaapi_device.clone()).flatten();
        Some(Encoder { name: e.name.to_owned(), vaapi_device })
    }

    /// Задачи для принятых шортсов, у исходников которых есть сведения ffprobe.
    fn render_jobs(&self, encoder: &Encoder, policy: ConflictPolicy) -> Vec<RenderJob> {
        let batch: &Batch = &self.batch;
        plan_outputs_with(batch, policy)
            .into_iter()
            .filter_map(|(id, dest)| {
                let short = batch.short(id)?;
                let source = batch.source(short.source_id)?;
                let info = source.info.as_ref()?;
                let (template, banner_layers) = batch.template_for(id).with_banner_layers(self.banner_art(Some(id)).as_ref());
                let template = template.with_vars(&TextVars { part: batch.part_number(id), title: short.title.clone() });
                let insert = self.insert_info(id).map(|i| i.job(&template, &banner_layers));
                let cover = self.cover_job(id);
                Some(RenderJob {
                    input: source.path.clone(),
                    src_size: (info.width, info.height),
                    src_fps: info.fps,
                    has_audio: info.has_audio,
                    start: short.start,
                    end: short.end,
                    template,
                    insert,
                    cover,
                    subtitles: self
                        .transcript_of(short.source_id)
                        .map(|t| crate::subtitles::short_words(&t, short))
                        .unwrap_or_default(),
                    output: batch.output.clone(),
                    encoder: encoder.clone(),
                    dest,
                    work_dir: self.paths.tmp_dir().join(format!("render-{}", id.0)),
                })
            })
            .collect()
    }

    /// Старт по кнопке: при «Спросить» и занятых именах — сначала диалог.
    fn request_render(&mut self, encoder: &Encoder) {
        if self.batch.output.conflict_policy == ConflictPolicy::Ask {
            let files = sc_render::conflicts(&self.batch);
            if !files.is_empty() {
                self.export.ask = Some(AskConflict { files });
                return;
            }
        }
        // Без конфликтов «Спросить» — то же, что «Продолжить».
        let policy = match self.batch.output.conflict_policy {
            ConflictPolicy::Ask => ConflictPolicy::Continue,
            p => p,
        };
        self.start_render(self.render_jobs(encoder, policy));
    }

    fn start_render(&mut self, jobs: Vec<RenderJob>) {
        if jobs.is_empty() {
            return;
        }
        self.remember_batch();
        let files = (jobs.iter())
            .map(|j| QueuedFile {
                dest: j.dest.clone(),
                input: j.input.clone(),
                duration: j.duration(),
                state: FileState::Waiting,
                started: None,
                elapsed: None,
            })
            .collect();
        let notify = self.jobs.notifier();
        let queue = Queue::start(jobs, self.settings.render_jobs as usize, move |ev| notify.send(Msg::Render(ev)));
        self.export = ExportState { run: Some(Run { queue, files, done: false }), ..Default::default() };
    }

    pub(crate) fn on_render(&mut self, ev: QueueEvent) {
        let Some(r) = &mut self.export.run else { return };
        match ev {
            QueueEvent::Started { index } => {
                r.files[index].state = FileState::Running(0.0);
                r.files[index].started = Some(Instant::now());
            }
            QueueEvent::Progress { index, fraction } => {
                if let FileState::Running(f) = &mut r.files[index].state {
                    *f = fraction;
                }
            }
            QueueEvent::Finished { index, result } => {
                let f = &mut r.files[index];
                f.elapsed = f.started.map(|t| t.elapsed().as_secs_f64());
                f.state = match result {
                    Ok(()) => FileState::Done,
                    Err(RenderError::Cancelled) => FileState::Cancelled,
                    Err(e) => FileState::Failed { tail: e.to_string(), log: e.log() },
                };
            }
            QueueEvent::Done => {
                r.done = true;
                let count = |p: fn(&FileState) -> bool| r.files.iter().filter(|f| p(&f.state)).count() as i64;
                let ok = count(|s| matches!(s, FileState::Done));
                let failed = count(|s| matches!(s, FileState::Failed { .. }));
                let cancelled = count(|s| matches!(s, FileState::Cancelled));
                let mut status = if cancelled > 0 {
                    self.i18n.tr_args("export-status-cancelled", &[("n", ok.into())])
                } else {
                    self.i18n.tr_args("export-status-done", &[("n", ok.into())])
                };
                if failed > 0 {
                    status += &self.i18n.tr_args("export-status-failed", &[("n", failed.into())]);
                }
                self.status = Some(status);
            }
        }
    }

    /// Изменения вывода запоминаются и для следующих пачек.
    fn save_output_settings(&mut self) {
        self.settings.output = self.batch.output.clone();
        self.save_settings();
    }

    pub(crate) fn on_picked_dir(&mut self, dir: Option<PathBuf>) {
        self.dialog_open = false;
        if let Some(dir) = dir {
            self.batch.output.dir = Some(dir);
            self.save_output_settings();
        }
    }

    fn pick_output_dir(&mut self) {
        self.dialog_open = true;
        let title = self.i18n.tr("export-dir-dialog");
        let start = self.batch.output.dir.clone().or_else(|| self.settings.last_import_dir.clone());
        self.jobs.spawn(move || {
            let mut dialog = rfd::FileDialog::new().set_title(title);
            if let Some(dir) = start {
                dialog = dialog.set_directory(dir);
            }
            Msg::PickedDir(dialog.pick_folder())
        });
    }

    pub(crate) fn export_ui(&mut self, ui: &mut egui::Ui) {
        let running = self.export.is_running();
        ui.add_enabled_ui(!running, |ui| self.output_ui(ui));
        ui.add_space(8.0);
        ui.separator();

        let encoder = self.encoder();
        // План на экране: «Спросить» показывает продолжение нумерации (если конфликтов нет — это нумерация с 1).
        let policy = match self.batch.output.conflict_policy {
            ConflictPolicy::Overwrite => ConflictPolicy::Overwrite,
            _ => ConflictPolicy::Continue,
        };
        let jobs = encoder.as_ref().map(|e| self.render_jobs(e, policy)).unwrap_or_default();
        match &encoder {
            Some(e) => {
                let quality = self.quality_summary(sc_render::encode::is_vaapi(&e.name));
                ui.label(self.i18n.tr_args("export-encoder", &[("name", e.name.clone().into()), ("quality", quality.into())]));
            }
            None if self.caps.is_none() => {
                ui.label(self.i18n.tr("caps-checking"));
            }
            None => {
                let codec = crate::output::codec_label(self.batch.output.codec);
                ui.colored_label(ui.visuals().error_fg_color, self.i18n.tr_args("export-no-encoder", &[("codec", codec.into())]));
            }
        }

        if running {
            self.queue_controls(ui);
        } else {
            let n = jobs.len() as i64;
            if self.banner_missing() {
                ui.colored_label(ui.visuals().error_fg_color, self.i18n.tr("banner-missing"));
            }
            if self.insert_missing() {
                ui.colored_label(ui.visuals().error_fg_color, self.i18n.tr("insert-missing"));
            }
            let warn = egui::Color32::from_rgb(235, 180, 60);
            let unplaced = self.shorts_without_insert();
            if unplaced > 0 {
                ui.colored_label(warn, self.i18n.tr_args("insert-unplaced-export", &[("n", (unplaced as i64).into())]));
            }
            let long = self.shorts_too_long();
            if long > 0 {
                ui.colored_label(warn, self.i18n.tr_args("export-too-long", &[("n", (long as i64).into())]));
            }
            ui.horizontal(|ui| {
                let start = egui::Button::new(self.i18n.tr_args("export-start", &[("n", n.into())]));
                if ui.add_enabled(n > 0 && self.export.ask.is_none(), start).clicked()
                    && let Some(e) = &encoder
                {
                    self.request_render(e);
                }
                ui.label(self.i18n.tr("export-parallel"));
                let mut jobs_n = self.settings.render_jobs;
                let r = ui.add(egui::DragValue::new(&mut jobs_n).range(1..=8));
                r.clone().on_hover_text(self.i18n.tr("export-parallel-hint"));
                if jobs_n != self.settings.render_jobs {
                    self.settings.render_jobs = jobs_n;
                }
                if r.drag_stopped() || r.lost_focus() {
                    self.save_settings();
                }
                if self.export.run.is_some() && ui.button(self.i18n.tr("export-clear")).clicked() {
                    self.export.run = None;
                }
            });
            if n == 0 && encoder.is_some() {
                ui.weak(self.i18n.tr("export-nothing"));
            }
        }
        ui.add_space(8.0);

        egui::ScrollArea::vertical().show(ui, |ui| {
            if self.export.run.is_none() {
                self.plan_ui(ui, &jobs);
            } else {
                self.queue_ui(ui);
            }
        });
        self.conflict_dialog(ui.ctx(), encoder.as_ref());
        self.log_window(ui.ctx());
    }

    /// Общий прогресс, пауза, отмена всего.
    fn queue_controls(&mut self, ui: &mut egui::Ui) {
        let Some((overall, finished, total)) = self.export.overall() else { return };
        let paused = self.export.is_paused();
        let Some(r) = &self.export.run else { return };
        let text = self.i18n.tr_args(
            if paused { "export-overall-paused" } else { "export-overall" },
            &[("n", (finished as i64).into()), ("total", (total as i64).into())],
        );
        ui.add(egui::ProgressBar::new(overall).text(format!("{text} · {:.0}%", overall * 100.0)));
        ui.horizontal(|ui| {
            if paused {
                if ui.button(self.i18n.tr("export-resume")).clicked() {
                    r.queue.resume();
                }
            } else if ui.button(self.i18n.tr("export-pause")).clicked() {
                r.queue.pause();
            }
            if ui.button(self.i18n.tr("export-cancel-all")).clicked() {
                r.queue.cancel_all();
            }
        });
    }

    /// Файлы очереди: состояние, прогресс, время, отмена одного, лог ошибки.
    fn queue_ui(&mut self, ui: &mut egui::Ui) {
        let Some(r) = &self.export.run else { return };
        let mut open_log = None;
        let err = ui.visuals().error_fg_color;
        egui::Grid::new("queue").striped(true).num_columns(5).min_col_width(16.0).spacing([12.0, 4.0]).show(ui, |ui| {
            for (i, f) in r.files.iter().enumerate() {
                match &f.state {
                    FileState::Waiting => ui.weak("…"),
                    FileState::Running(_) => ui.label("▶"),
                    FileState::Done => ui.label("✔"),
                    FileState::Failed { .. } => ui.colored_label(err, "✖"),
                    FileState::Cancelled => ui.weak("—"),
                };
                let name = ui.monospace(file_name(&f.dest)).on_hover_text(f.dest.display().to_string());
                if matches!(f.state, FileState::Done) {
                    name.context_menu(|ui| {
                        if ui.button(self.i18n.tr("export-copy-path")).clicked() {
                            ui.ctx().copy_text(f.dest.display().to_string());
                            ui.close();
                        }
                    });
                }
                ui.label(file_name(&f.input));
                match &f.state {
                    FileState::Waiting => {
                        ui.weak(self.i18n.tr("export-waiting"));
                    }
                    FileState::Running(p) => {
                        ui.add(egui::ProgressBar::new(*p).desired_width(220.0).show_percentage());
                    }
                    FileState::Done => {
                        let t = f.elapsed.map(fmt::duration).unwrap_or_default();
                        ui.label(self.i18n.tr_args("export-done-in", &[("time", t.into())]));
                    }
                    FileState::Failed { tail, .. } => {
                        let first = tail.lines().last().unwrap_or_default().to_owned();
                        ui.colored_label(err, first).on_hover_text(tail);
                    }
                    FileState::Cancelled => {
                        ui.weak(self.i18n.tr("export-file-cancelled"));
                    }
                }
                match &f.state {
                    FileState::Waiting | FileState::Running(_) => {
                        let cancelling = r.queue.is_cancelled(i);
                        if ui
                            .add_enabled(!cancelling, egui::Button::new("✖").small())
                            .on_hover_text(self.i18n.tr("export-cancel-file"))
                            .clicked()
                        {
                            r.queue.cancel(i);
                        }
                    }
                    FileState::Failed { .. } => {
                        if ui.small_button(self.i18n.tr("export-log")).clicked() {
                            open_log = Some(i);
                        }
                    }
                    _ => {
                        ui.label("");
                    }
                }
                ui.end_row();
            }
        });
        if open_log.is_some() {
            self.export.log = open_log;
        }
    }

    /// Окно с командой ffmpeg и всем его stderr.
    fn log_window(&mut self, ctx: &egui::Context) {
        let Some(i) = self.export.log else { return };
        let Some(f) = self.export.run.as_ref().and_then(|r| r.files.get(i)) else {
            self.export.log = None;
            return;
        };
        let FileState::Failed { log, .. } = &f.state else {
            self.export.log = None;
            return;
        };
        let mut open = true;
        let mut text = log.as_str();
        egui::Window::new(self.i18n.tr_args("export-log-title", &[("name", file_name(&f.dest).into())]))
            .open(&mut open)
            .default_size([720.0, 420.0])
            .resizable(true)
            .show(ctx, |ui| {
                if ui.button(self.i18n.tr("export-log-copy")).clicked() {
                    ui.ctx().copy_text(log.clone());
                }
                egui::ScrollArea::both().show(ui, |ui| {
                    ui.add(egui::TextEdit::multiline(&mut text).code_editor().desired_width(f32::INFINITY));
                });
            });
        if !open {
            self.export.log = None;
        }
    }

    /// «Файлы уже есть»: продолжить нумерацию, перезаписать или отменить.
    fn conflict_dialog(&mut self, ctx: &egui::Context, encoder: Option<&Encoder>) {
        let Some(ask) = &self.export.ask else { return };
        let mut choice: Option<Option<ConflictPolicy>> = None;
        let modal = egui::Modal::new(egui::Id::new("export-conflict")).show(ctx, |ui| {
            ui.set_max_width(520.0);
            ui.heading(self.i18n.tr("export-conflict-title"));
            ui.label(self.i18n.tr_args("export-conflict-text", &[("n", (ask.files.len() as i64).into())]));
            for f in ask.files.iter().take(5) {
                ui.monospace(f.display().to_string());
            }
            if ask.files.len() > 5 {
                ui.weak(format!("… +{}", ask.files.len() - 5));
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(self.i18n.tr("export-conflict-continue")).clicked() {
                    choice = Some(Some(ConflictPolicy::Continue));
                }
                if ui.button(self.i18n.tr("export-conflict-overwrite")).clicked() {
                    choice = Some(Some(ConflictPolicy::Overwrite));
                }
                if ui.button(self.i18n.tr("export-conflict-cancel")).clicked() {
                    choice = Some(None);
                }
            });
        });
        if modal.should_close() && choice.is_none() {
            choice = Some(None);
        }
        let Some(choice) = choice else { return };
        self.export.ask = None;
        if let (Some(policy), Some(e)) = (choice, encoder) {
            self.start_render(self.render_jobs(e, policy));
        }
    }

    fn output_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading(self.i18n.tr("export-output"));
        egui::Grid::new("output").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            ui.label(self.i18n.tr("export-dir"));
            ui.horizontal(|ui| {
                match &self.batch.output.dir {
                    Some(d) => ui.monospace(d.display().to_string()),
                    None => ui.label(self.i18n.tr("export-dir-next-to-source")),
                };
                if ui.add_enabled(!self.dialog_open, egui::Button::new(self.i18n.tr("export-dir-pick"))).clicked() {
                    self.pick_output_dir();
                }
                if self.batch.output.dir.is_some() && ui.button(self.i18n.tr("export-dir-reset")).clicked() {
                    self.batch.output.dir = None;
                    self.save_output_settings();
                }
            });
            ui.end_row();

            ui.label(self.i18n.tr("export-base-name"));
            ui.horizontal(|ui| {
                let edit = ui.add(egui::TextEdit::singleline(&mut self.batch.output.base_name).desired_width(200.0));
                if edit.lost_focus() && self.settings.output.base_name != self.batch.output.base_name {
                    self.save_output_settings();
                }
                let base = sc_render::naming::sanitize_base(&self.batch.output.base_name);
                let example = format!("{base}_1.{}", self.batch.output.container.extension());
                ui.weak(self.i18n.tr_args("export-name-example", &[("example", example.into())]));
            });
            ui.end_row();

            ui.label(self.i18n.tr("export-conflict"));
            let policy = self.batch.output.conflict_policy;
            let label = |p: ConflictPolicy| match p {
                ConflictPolicy::Continue => "export-conflict-policy-continue",
                ConflictPolicy::Ask => "export-conflict-policy-ask",
                ConflictPolicy::Overwrite => "export-conflict-policy-overwrite",
            };
            egui::ComboBox::from_id_salt("conflict").selected_text(self.i18n.tr(label(policy))).show_ui(ui, |ui| {
                for p in [ConflictPolicy::Continue, ConflictPolicy::Ask, ConflictPolicy::Overwrite] {
                    ui.selectable_value(&mut self.batch.output.conflict_policy, p, self.i18n.tr(label(p)));
                }
            });
            if self.batch.output.conflict_policy != policy {
                self.save_output_settings();
            }
            ui.end_row();
        });
        ui.add_space(8.0);
        ui.heading(self.i18n.tr("export-format"));
        self.format_ui(ui);
    }

    fn plan_ui(&self, ui: &mut egui::Ui, jobs: &[RenderJob]) {
        egui::Grid::new("plan").striped(true).num_columns(4).spacing([16.0, 4.0]).show(ui, |ui| {
            for j in jobs {
                ui.monospace(file_name(&j.dest)).on_hover_text(j.dest.display().to_string());
                ui.label(file_name(&j.input));
                ui.label(format!("{} – {}", fmt::clock_ms(j.start), fmt::clock_ms(j.end)));
                let d = j.duration();
                let mut text = fmt::duration(d);
                if j.insert.is_some() {
                    text = self.i18n.tr_args("export-with-insert", &[("total", text.into())]);
                }
                if d > sc_core::SHORTS_MAX {
                    ui.colored_label(egui::Color32::from_rgb(235, 180, 60), format!("⚠ {text}"))
                        .on_hover_text(self.i18n.tr("insert-too-long"));
                } else {
                    ui.label(text);
                }
                ui.end_row();
            }
        });
    }
}
