//! ShortsCutter — точка входа: окно, вкладки, фоновые задачи.

mod import;
mod jobs;

use eframe::egui;
use sc_core::{Batch, Language, Paths, Settings};
use sc_i18n::I18n;
use sc_media::{Capabilities, EncoderKind};

use jobs::{Jobs, Msg};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Import,
    Cut,
    Design,
    Export,
    Library,
    Settings,
}

impl Tab {
    const ALL: [Tab; 6] = [Tab::Import, Tab::Cut, Tab::Design, Tab::Export, Tab::Library, Tab::Settings];

    fn key(self) -> &'static str {
        match self {
            Tab::Import => "tab-import",
            Tab::Cut => "tab-cut",
            Tab::Design => "tab-design",
            Tab::Export => "tab-export",
            Tab::Library => "tab-library",
            Tab::Settings => "tab-settings",
        }
    }
}

struct App {
    paths: Paths,
    settings: Settings,
    i18n: I18n,
    batch: Batch,
    tab: Tab,
    status: Option<String>,
    jobs: Jobs,
    /// `None` — проверка ffmpeg ещё идёт.
    caps: Option<Capabilities>,
    dialog_open: bool,
    /// winit 0.30 не умеет drag-and-drop на Wayland — подсказку показываем только на X11.
    dnd_supported: bool,
}

impl App {
    fn new(ctx: egui::Context, paths: Paths, settings: Settings) -> Self {
        let batch = Batch { output: settings.output.clone(), ..Default::default() };
        let jobs = Jobs::new(ctx);
        jobs.spawn(|| Msg::Caps(Capabilities::detect()));
        Self {
            i18n: I18n::new(settings.language),
            paths,
            settings,
            batch,
            tab: Tab::Import,
            status: None,
            jobs,
            caps: None,
            dialog_open: false,
            dnd_supported: std::env::var_os("WAYLAND_DISPLAY").is_none(),
        }
    }

    fn poll_jobs(&mut self) {
        let msgs: Vec<Msg> = self.jobs.drain().collect();
        for msg in msgs {
            match msg {
                Msg::Caps(caps) => {
                    log::info!("возможности ffmpeg: {caps:?}");
                    self.caps = Some(caps);
                }
                Msg::Picked(paths) => self.on_picked(paths),
                Msg::Probed { id, result } => self.on_probed(id, result),
            }
        }
    }

    /// Файлы, брошенные в окно; пока их тащат над окном — подсказка поверх всего.
    fn handle_drop(&mut self, ctx: &egui::Context) {
        let (hovering, dropped) = ctx.input(|i| {
            let dropped: Vec<_> = i.raw.dropped_files.iter().map(|f| f.path().to_owned()).filter(|p| !p.as_os_str().is_empty()).collect();
            (!i.raw.hovered_files.is_empty(), dropped)
        });
        if hovering {
            let screen = ctx.content_rect();
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("drop")));
            painter.rect_filled(screen, 0.0, egui::Color32::from_black_alpha(160));
            painter.text(
                screen.center(),
                egui::Align2::CENTER_CENTER,
                self.i18n.tr("import-drop-here"),
                egui::FontId::proportional(28.0),
                egui::Color32::WHITE,
            );
        }
        if !dropped.is_empty() {
            self.tab = Tab::Import;
            self.add_paths(dropped);
        }
    }

    fn caps_summary(&self) -> String {
        match &self.caps {
            None => self.i18n.tr("caps-checking"),
            Some(c) if !c.usable() => self.i18n.tr("caps-no-ffmpeg"),
            Some(c) => {
                let gpu: Vec<&str> = c.working_encoders(EncoderKind::Gpu).map(|e| e.name).collect();
                let gpu = if gpu.is_empty() { self.i18n.tr("caps-none") } else { gpu.join(", ") };
                let version = c.ffmpeg_version.clone().unwrap_or_default();
                self.i18n.tr_args("caps-summary", &[("version", version.into()), ("gpu", gpu.into())])
            }
        }
    }

    fn caps_ui(&self, ui: &mut egui::Ui) {
        ui.heading(self.i18n.tr("settings-ffmpeg"));
        let Some(c) = &self.caps else {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(self.i18n.tr("caps-checking"));
            });
            return;
        };
        ui.label(self.caps_summary());
        if !c.usable() {
            return;
        }
        let device = c.vaapi_device.as_ref().map_or_else(|| self.i18n.tr("caps-none"), |d| d.display().to_string());
        ui.label(self.i18n.tr_args("caps-vaapi-device", &[("device", device.into())]));
        if !c.missing_filters.is_empty() {
            let filters = c.missing_filters.join(", ");
            ui.colored_label(
                ui.visuals().error_fg_color,
                self.i18n.tr_args("caps-missing-filters", &[("filters", filters.into())]),
            );
        }
        ui.add_space(4.0);
        egui::Grid::new("encoders").striped(true).show(ui, |ui| {
            for e in &c.encoders {
                let kind = self.i18n.tr(match e.kind {
                    EncoderKind::Gpu => "caps-kind-gpu",
                    EncoderKind::Cpu => "caps-kind-cpu",
                });
                ui.label(if e.works { "✔" } else { "✖" });
                ui.monospace(e.name);
                ui.label(kind);
                ui.end_row();
            }
        });
    }

    fn save_settings(&mut self) {
        self.status = Some(match self.settings.save(&self.paths.settings_file()) {
            Ok(()) => self.i18n.tr("settings-saved"),
            Err(e) => self.i18n.tr_args("settings-save-failed", &[("error", e.to_string().into())]),
        });
    }

    fn settings_ui(&mut self, ui: &mut egui::Ui) {
        let mut lang = self.settings.language;
        ui.horizontal(|ui| {
            ui.label(self.i18n.tr("settings-language"));
            for l in Language::ALL {
                let label = self.i18n.tr(&format!("lang-{}", l.code()));
                ui.radio_value(&mut lang, l, label);
            }
        });
        if lang != self.settings.language {
            self.settings.language = lang;
            self.i18n.set_lang(lang);
            self.save_settings();
        }
        ui.add_space(12.0);
        ui.separator();
        self.caps_ui(ui);
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll_jobs();
        self.handle_drop(&ui.ctx().clone());
        egui::Panel::top("tabs").show(ui, |ui| {
            ui.horizontal(|ui| {
                for tab in Tab::ALL {
                    ui.selectable_value(&mut self.tab, tab, self.i18n.tr(tab.key()));
                }
            });
        });
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                let n = self.batch.sources.len() as i64;
                ui.label(self.i18n.tr_args("batch-summary", &[("sources", n.into())]));
                ui.separator();
                ui.label(self.caps_summary());
                if let Some(s) = &self.status {
                    ui.separator();
                    ui.label(s);
                }
            });
        });
        egui::CentralPanel::default().show(ui, |ui| match self.tab {
            Tab::Import => self.import_ui(ui),
            Tab::Settings => self.settings_ui(ui),
            _ => {
                ui.label(self.i18n.tr("placeholder-not-ready"));
            }
        });
    }
}

fn main() -> eframe::Result {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let paths = Paths::from_env().expect("не удалось определить домашний каталог");
    if let Err(e) = paths.ensure() {
        log::error!("не удалось создать каталоги приложения: {e}");
    }
    let settings = Settings::load_or_recover(&paths.settings_file());
    let title = I18n::new(settings.language).tr("app-title");

    // `shortscutter ролик1.mp4 ролик2.mkv` — сразу добавить в пачку.
    let initial: Vec<std::path::PathBuf> = std::env::args_os().skip(1).map(Into::into).collect();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(&title)
            .with_app_id("shortscutter")
            .with_inner_size([1280.0, 800.0]),
        ..Default::default()
    };
    eframe::run_native(
        &title,
        options,
        Box::new(|cc| {
            let mut app = App::new(cc.egui_ctx.clone(), paths, settings);
            app.add_paths(initial);
            Ok(Box::new(app))
        }),
    )
}
