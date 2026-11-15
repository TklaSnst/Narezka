//! ShortsCutter — точка входа: окно, вкладки, фоновые задачи.

mod analysis;
mod cut;
mod design;
mod editor;
mod export;
mod fmt;
mod import;
mod jobs;
mod player;
mod raster_worker;
mod sub_list;
mod subtitle_worker;
mod subtitles;
mod templates;
mod timeline;

use eframe::{egui, glow};
use sc_core::{Batch, Language, Paths, Settings, UserTemplates};
use sc_i18n::I18n;
use sc_media::{Capabilities, EncoderKind};

use analysis::AnalysisState;
use cut::CutState;
use design::DesignState;
use export::ExportState;
use jobs::{Jobs, Msg};
use player::Player;
use templates::Library;

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
    /// `None` — libmpv не поднялся (причина в `player_error`), превью недоступно.
    player: Option<Player>,
    player_error: Option<String>,
    cut: CutState,
    design: DesignState,
    export: ExportState,
    analysis: AnalysisState,
    /// Свои шаблоны пользователя.
    templates: Library,
    subtitle_cache: subtitles::SubtitleCache,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, paths: Paths, settings: Settings) -> Self {
        let mut batch = Batch { output: settings.output.clone(), ..Default::default() };
        if let Some(t) = dev_template() {
            batch.template = t;
            batch.template_ref = None;
        }
        let jobs = Jobs::new(cc.egui_ctx.clone());
        let (player, player_error) = match Player::new(cc) {
            Ok(p) => (Some(p), None),
            Err(e) => {
                log::error!("превью недоступно: {e}");
                (None, Some(e))
            }
        };
        jobs.spawn(|| Msg::Caps(Capabilities::detect()));
        Self {
            i18n: I18n::new(settings.language),
            analysis: AnalysisState::new(&settings),
            templates: Library::new(UserTemplates::new(paths.templates_dir())),
            paths,
            settings,
            batch,
            tab: Tab::Import,
            status: None,
            jobs,
            caps: None,
            dialog_open: false,
            dnd_supported: std::env::var_os("WAYLAND_DISPLAY").is_none(),
            player,
            player_error,
            cut: CutState::default(),
            design: DesignState::default(),
            export: ExportState::default(),
            subtitle_cache: Default::default(),
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
                Msg::PickedDir(dir) => self.on_picked_dir(dir),
                Msg::Render(ev) => self.on_render(ev),
                Msg::Analysis(ev) => self.on_analysis(ev),
                Msg::Download(ev) => self.on_download(ev),
                Msg::PickedModel(file) => self.on_picked_model(file),
                Msg::PickedFile(target, file) => self.on_picked_file(target, file),
            }
        }
        self.pump_analysis();
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
        self.analysis_settings_ui(ui);
        ui.add_space(12.0);
        ui.separator();
        self.caps_ui(ui);
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.poll_jobs();
        self.handle_drop(&ui.ctx().clone());
        self.cut_sync();
        self.design_sync_target();
        // Подстановки — по выбранному шортсу; шаблон — его (или тот, что правится на «Оформлении»).
        let template = self.player.is_some().then(|| self.preview_template(self.tab == Tab::Design).with_vars(&templates::preview_vars(self)));
        let design = self.tab == Tab::Design;
        let subs = template.as_ref().filter(|t| t.has_subtitles()).and_then(|_| self.subtitle_input(design));
        if let (Some(p), Some(template)) = (&mut self.player, template) {
            p.set_subtitles(subs);
            // Превью играет только на вкладках с ним.
            if !matches!(self.tab, Tab::Cut | Tab::Design) {
                p.set_paused(true);
            }
            p.pump(frame, &template);
        }
        // Ctrl+1…6 — вкладки по порядку.
        let digits = [egui::Key::Num1, egui::Key::Num2, egui::Key::Num3, egui::Key::Num4, egui::Key::Num5, egui::Key::Num6];
        if let Some(tab) = ui.input(|i| {
            i.modifiers.command.then(|| Tab::ALL.iter().zip(digits).find(|(_, k)| i.key_pressed(*k)).map(|(t, _)| *t)).flatten()
        }) {
            self.tab = tab;
        }
        egui::Panel::top("tabs").show(ui, |ui| {
            ui.horizontal(|ui| {
                for tab in Tab::ALL {
                    ui.selectable_value(&mut self.tab, tab, self.i18n.tr(tab.key()))
                        .on_hover_text(format!("Ctrl+{}", Tab::ALL.iter().position(|t| *t == tab).unwrap_or(0) + 1));
                }
            });
        });
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                let n = self.batch.sources.len() as i64;
                ui.label(self.i18n.tr_args("batch-summary", &[("sources", n.into())]));
                ui.separator();
                ui.label(self.caps_summary());
                if let Some((fraction, n, total)) = self.export.overall() {
                    ui.separator();
                    let pct = (fraction * 100.0).round() as i64;
                    ui.label(self.i18n.tr_args(
                        "export-status-running",
                        &[("n", (n as i64).into()), ("total", (total as i64).into()), ("pct", pct.into())],
                    ));
                } else if let Some(s) = self.analysis_status() {
                    ui.separator();
                    ui.label(s);
                } else if let Some(s) = &self.status {
                    ui.separator();
                    ui.label(s);
                }
            });
        });
        egui::CentralPanel::default().show(ui, |ui| match self.tab {
            Tab::Import => self.import_ui(ui),
            Tab::Cut => self.cut_ui(ui),
            Tab::Design => self.design_ui(ui),
            Tab::Export => self.export_ui(ui),
            Tab::Settings => self.settings_ui(ui),
            _ => {
                ui.label(self.i18n.tr("placeholder-not-ready"));
            }
        });
        if let Some(p) = &self.player {
            p.end_frame();
        }
    }

    fn on_exit(&mut self, gl: Option<&glow::Context>) {
        // render context mpv освобождается при текущем GL-контексте.
        if let (Some(p), Some(gl)) = (self.player.take(), gl) {
            p.destroy(gl);
        }
    }
}

/// Шаблон из файла `SC_TEMPLATE` — для разработки (подменяет шаблон пачки при запуске).
fn dev_template() -> Option<sc_core::Template> {
    let path = std::env::var_os("SC_TEMPLATE")?;
    let parsed = std::fs::read_to_string(&path)
        .map_err(|e| e.to_string())
        .and_then(|s| sc_core::Template::from_toml(&s).map_err(|e| e.to_string()))
        .and_then(|t| t.validate().map(|()| t).map_err(|e| e.to_string()));
    match parsed {
        Ok(t) => {
            log::info!("шаблон из SC_TEMPLATE: {}", t.name);
            Some(t)
        }
        Err(e) => {
            log::error!("SC_TEMPLATE {path:?}: {e}");
            None
        }
    }
}

fn main() -> eframe::Result {
    // До любых потоков и до инициализации ggml (whisper).
    sc_analysis::prepare_ggml_env();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    sc_mpv::set_c_numeric_locale();

    let paths = Paths::from_env().expect("не удалось определить домашний каталог");
    if let Err(e) = paths.ensure() {
        log::error!("не удалось создать каталоги приложения: {e}");
    }
    // Шрифты для текстовых слоёв грузятся сотни миллисекунд — заранее и не в UI-потоке.
    sc_raster::fonts::set_user_dir(paths.fonts_dir());
    std::thread::spawn(sc_raster::warm_up);
    let settings = Settings::load_or_recover(&paths.settings_file());
    let title = I18n::new(settings.language).tr("app-title");

    // `shortscutter ролик1.mp4 ролик2.mkv` — сразу добавить в пачку.
    let initial: Vec<std::path::PathBuf> = std::env::args_os().skip(1).map(Into::into).collect();

    let options = eframe::NativeOptions {
        // Превью — через GL (mpv render API → текстура), поэтому только glow.
        renderer: eframe::Renderer::Glow,
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
            let mut app = App::new(cc, paths, settings);
            app.add_paths(initial);
            Ok(Box::new(app))
        }),
    )
}
