//! Экран «Экспорт»: папка и имя файлов, рендер принятых шортсов по одному, прогресс и отмена.
//!
//! Очередь с параллельными задачами — этап 7; здесь шортсы рендерятся последовательно в одном потоке.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use eframe::egui;
use sc_core::{Batch, TextVars};
use sc_render::{Encoder, RenderError, RenderJob, plan_outputs};

use crate::App;
use crate::fmt;
use crate::import::file_name;
use crate::jobs::Msg;

pub enum RenderEvent {
    Started { index: usize },
    Progress { index: usize, fraction: f32 },
    Finished { index: usize, result: Result<(), String> },
    /// Поток закончил (все готовы, отменено или ошибка запуска).
    Done,
}

struct Running {
    cancel: Arc<AtomicBool>,
    /// Итоговые файлы по порядку рендера.
    files: Vec<PathBuf>,
    index: usize,
    fraction: f32,
}

#[derive(Default)]
pub struct ExportState {
    running: Option<Running>,
    /// Итоги последнего запуска: файл и ошибка (`None` — готов).
    results: Vec<(PathBuf, Option<String>)>,
    cancelled: bool,
}

impl ExportState {
    pub fn is_running(&self) -> bool {
        self.running.is_some()
    }

    /// Общая доля и номер текущего шортса (с 1) — для строки состояния.
    pub fn overall(&self) -> Option<(f32, usize, usize)> {
        let r = self.running.as_ref()?;
        let total = r.files.len().max(1);
        Some(((r.index as f32 + r.fraction) / total as f32, r.index + 1, r.files.len()))
    }
}

impl App {
    fn encoder(&self) -> Option<Encoder> {
        let caps = self.caps.as_ref().filter(|c| c.usable())?;
        let out = &self.batch.output;
        let e = caps.encoder_for(out.codec, out.encoder)?;
        let vaapi_device = (e.kind == sc_media::EncoderKind::Gpu).then(|| caps.vaapi_device.clone()).flatten();
        Some(Encoder { name: e.name.to_owned(), vaapi_device })
    }

    /// Задачи для принятых шортсов, у исходников которых есть сведения ffprobe.
    fn render_jobs(&self, encoder: &Encoder) -> Vec<RenderJob> {
        let batch: &Batch = &self.batch;
        plan_outputs(batch)
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

    fn start_render(&mut self, jobs: Vec<RenderJob>) {
        let cancel = Arc::new(AtomicBool::new(false));
        self.export = ExportState {
            running: Some(Running {
                cancel: cancel.clone(),
                files: jobs.iter().map(|j| j.dest.clone()).collect(),
                index: 0,
                fraction: 0.0,
            }),
            ..Default::default()
        };
        let notify = self.jobs.notifier();
        std::thread::spawn(move || {
            for (index, job) in jobs.iter().enumerate() {
                notify.send(Msg::Render(RenderEvent::Started { index }));
                let mut last = -1.0;
                let result = job.run(&cancel, |fraction| {
                    // Не чаще чем раз в процент — UI не нужно больше.
                    if fraction - last >= 0.01 || fraction >= 1.0 {
                        last = fraction;
                        notify.send(Msg::Render(RenderEvent::Progress { index, fraction }));
                    }
                });
                let stop = matches!(result, Err(RenderError::Cancelled));
                if let Err(e) = &result {
                    log::warn!("рендер {}: {e}", job.dest.display());
                }
                notify.send(Msg::Render(RenderEvent::Finished { index, result: result.map_err(|e| e.to_string()) }));
                if stop {
                    break;
                }
            }
            notify.send(Msg::Render(RenderEvent::Done));
        });
    }

    pub(crate) fn on_render(&mut self, ev: RenderEvent) {
        let Some(r) = &mut self.export.running else { return };
        match ev {
            RenderEvent::Started { index } => {
                r.index = index;
                r.fraction = 0.0;
            }
            RenderEvent::Progress { index, fraction } => {
                r.index = index;
                r.fraction = fraction;
            }
            RenderEvent::Finished { index, result } => {
                let file = r.files[index].clone();
                match result {
                    Err(_) if r.cancel.load(Ordering::Relaxed) => self.export.cancelled = true,
                    Err(e) => self.export.results.push((file, Some(e))),
                    Ok(()) => self.export.results.push((file, None)),
                }
            }
            RenderEvent::Done => {
                let ok = self.export.results.iter().filter(|(_, e)| e.is_none()).count() as i64;
                self.status = Some(if self.export.cancelled {
                    self.i18n.tr_args("export-status-cancelled", &[("n", ok.into())])
                } else {
                    self.i18n.tr_args("export-status-done", &[("n", ok.into())])
                });
                self.export.running = None;
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
        let jobs = encoder.as_ref().map(|e| self.render_jobs(e)).unwrap_or_default();
        match &encoder {
            Some(e) => {
                let mbps = format!("{:.1}", self.batch.output.video_bitrate_kbps as f64 / 1000.0);
                ui.label(self.i18n.tr_args("export-encoder", &[("name", e.name.clone().into()), ("mbps", mbps.into())]));
            }
            None if self.caps.is_none() => {
                ui.label(self.i18n.tr("caps-checking"));
            }
            None => {
                ui.colored_label(ui.visuals().error_fg_color, self.i18n.tr("export-no-encoder"));
            }
        }

        if let Some(r) = &self.export.running {
            let (overall, n, total) = self.export.overall().unwrap_or_default();
            let name = file_name(&r.files[r.index]);
            ui.label(self.i18n.tr_args(
                "export-progress",
                &[("n", (n as i64).into()), ("total", (total as i64).into()), ("name", name.into())],
            ));
            ui.add(egui::ProgressBar::new(r.fraction).show_percentage());
            ui.add(egui::ProgressBar::new(overall).text(self.i18n.tr("export-overall")));
            let cancelling = r.cancel.load(Ordering::Relaxed);
            if ui.add_enabled(!cancelling, egui::Button::new(self.i18n.tr("export-cancel"))).clicked() {
                r.cancel.store(true, Ordering::Relaxed);
            }
        } else {
            let n = jobs.len() as i64;
            let start = egui::Button::new(self.i18n.tr_args("export-start", &[("n", n.into())]));
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
            if ui.add_enabled(n > 0, start).clicked() {
                self.remember_batch();
                self.start_render(jobs);
                return;
            }
            if n == 0 && encoder.is_some() {
                ui.weak(self.i18n.tr("export-nothing"));
            }
        }
        ui.add_space(8.0);

        egui::ScrollArea::vertical().show(ui, |ui| {
            if self.export.running.is_none() && self.export.results.is_empty() {
                self.plan_ui(ui, &jobs);
            }
            self.results_ui(ui);
        });
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
        });
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

    fn results_ui(&self, ui: &mut egui::Ui) {
        if self.export.cancelled {
            ui.label(self.i18n.tr("export-cancelled"));
        }
        for (file, error) in &self.export.results {
            ui.horizontal(|ui| match error {
                None => {
                    ui.label("✔");
                    ui.monospace(file.display().to_string());
                }
                Some(e) => {
                    ui.colored_label(ui.visuals().error_fg_color, "✖");
                    ui.monospace(file_name(file));
                    ui.colored_label(ui.visuals().error_fg_color, e);
                }
            });
        }
    }
}
