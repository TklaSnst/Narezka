//! Экран «Импорт»: добавление роликов в пачку и их статусы.

use std::path::PathBuf;

use eframe::egui;
use sc_core::{Source, SourceId, SourceStatus};

use crate::App;
use crate::jobs::Msg;

/// Расширения для фильтра диалога; перетащить можно любой файл — решит ffprobe.
const VIDEO_EXTS: &[&str] = &["mp4", "mkv", "webm", "mov", "m4v", "avi", "flv", "ts", "mts"];

impl App {
    pub(crate) fn add_paths(&mut self, paths: Vec<PathBuf>) {
        for path in paths.into_iter().filter(|p| p.is_file()) {
            let path = path.canonicalize().unwrap_or(path);
            let source = Source::new(path.clone());
            let id = source.id;
            if self.batch.add_source(source) == id {
                self.jobs.probe(id, path);
            }
        }
    }

    pub(crate) fn open_file_dialog(&mut self) {
        self.dialog_open = true;
        let title = self.i18n.tr("import-dialog-title");
        let filter = self.i18n.tr("import-dialog-filter");
        let dir = self.settings.last_import_dir.clone();
        self.jobs.spawn(move || {
            let mut dialog = rfd::FileDialog::new().set_title(title).add_filter(filter, VIDEO_EXTS);
            if let Some(dir) = dir {
                dialog = dialog.set_directory(dir);
            }
            Msg::Picked(dialog.pick_files().unwrap_or_default())
        });
    }

    pub(crate) fn on_picked(&mut self, paths: Vec<PathBuf>) {
        self.dialog_open = false;
        if let Some(dir) = paths.first().and_then(|p| p.parent())
            && self.settings.last_import_dir.as_deref() != Some(dir)
        {
            self.settings.last_import_dir = Some(dir.to_owned());
            self.save_settings();
        }
        self.add_paths(paths);
    }

    pub(crate) fn on_probed(&mut self, id: SourceId, result: Result<(sc_core::MediaInfo, String), &'static str>) {
        match result {
            Ok((info, hash)) => {
                // Тот же файл под другим путём (копия, симлинк) — второй раз не добавляем.
                if let Some(dup) = self.batch.sources.iter().find(|s| s.id != id && s.hash.as_ref() == Some(&hash)) {
                    let name = file_name(&dup.path);
                    let _ = self.batch.remove_source(id);
                    self.status = Some(self.i18n.tr_args("import-duplicate", &[("name", name.into())]));
                    return;
                }
                if let Some(s) = self.batch.source_mut(id) {
                    s.info = Some(info);
                    s.hash = Some(hash);
                    s.status = SourceStatus::Ready;
                }
            }
            Err(key) => {
                if let Some(s) = self.batch.source_mut(id) {
                    s.status = SourceStatus::Failed(key.to_owned());
                }
            }
        }
    }

    pub(crate) fn import_ui(&mut self, ui: &mut egui::Ui) {
        let usable = self.caps.as_ref().is_none_or(|c| c.usable());
        ui.horizontal(|ui| {
            let add = egui::Button::new(self.i18n.tr("import-add"));
            if ui.add_enabled(usable && !self.dialog_open, add).clicked() {
                self.open_file_dialog();
            }
            if self.dnd_supported {
                ui.weak(self.i18n.tr("import-drop-hint"));
            }
        });
        if !usable {
            ui.colored_label(ui.visuals().error_fg_color, self.i18n.tr("caps-no-ffmpeg"));
        }
        ui.add_space(8.0);

        if self.batch.sources.is_empty() {
            ui.label(self.i18n.tr("import-empty"));
            return;
        }

        let mut remove = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("sources").striped(true).num_columns(7).spacing([16.0, 6.0]).show(ui, |ui| {
                for key in ["import-col-file", "import-col-duration", "import-col-size", "import-col-fps", "import-col-audio", "import-col-status"] {
                    ui.strong(self.i18n.tr(key));
                }
                ui.end_row();

                for s in &self.batch.sources {
                    ui.label(file_name(&s.path)).on_hover_text(s.path.display().to_string());
                    match &s.info {
                        Some(info) => {
                            ui.label(fmt_duration(info.duration));
                            ui.label(format!("{}×{}", info.width, info.height));
                            ui.label(format!("{:.3}", info.fps).trim_end_matches('0').trim_end_matches('.').to_owned());
                            ui.label(self.i18n.tr(if info.has_audio { "import-audio-yes" } else { "import-audio-no" }));
                        }
                        None => {
                            for _ in 0..4 {
                                ui.label("—");
                            }
                        }
                    }
                    self.status_cell(ui, &s.status);
                    if ui.small_button("✖").on_hover_text(self.i18n.tr("import-remove")).clicked() {
                        remove = Some(s.id);
                    }
                    ui.end_row();
                }
            });
        });
        if let Some(id) = remove {
            let _ = self.batch.remove_source(id);
        }
    }

    fn status_cell(&self, ui: &mut egui::Ui, status: &SourceStatus) {
        match status {
            SourceStatus::New => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(self.i18n.tr("source-status-new"));
                });
            }
            SourceStatus::Ready => {
                ui.label(self.i18n.tr("source-status-ready"));
            }
            SourceStatus::Analyzing => {
                ui.label(self.i18n.tr("source-status-analyzing"));
            }
            SourceStatus::Analyzed => {
                ui.label(self.i18n.tr("source-status-analyzed"));
            }
            SourceStatus::Failed(key) => {
                ui.colored_label(ui.visuals().error_fg_color, self.i18n.tr(key));
            }
        }
    }
}

fn file_name(path: &std::path::Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string())
}

/// 75.4 → «1:15», 6612 → «1:50:12».
fn fmt_duration(secs: f64) -> String {
    let t = secs.round() as u64;
    let (h, m, s) = (t / 3600, t / 60 % 60, t % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

#[cfg(test)]
mod tests {
    #[test]
    fn durations() {
        assert_eq!(super::fmt_duration(75.4), "1:15");
        assert_eq!(super::fmt_duration(6612.0), "1:50:12");
        assert_eq!(super::fmt_duration(0.2), "0:00");
    }
}
