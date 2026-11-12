//! ShortsCutter — точка входа. Шаг 1.1: окно с вкладками-заглушками и переключением языка.

use eframe::egui;
use sc_core::{Batch, Language, Paths, Settings};
use sc_i18n::I18n;

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
}

impl App {
    fn new(paths: Paths, settings: Settings) -> Self {
        let batch = Batch { output: settings.output.clone(), ..Default::default() };
        Self {
            i18n: I18n::new(settings.language),
            paths,
            settings,
            batch,
            tab: Tab::Import,
            status: None,
        }
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
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
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
                if let Some(s) = &self.status {
                    ui.separator();
                    ui.label(s);
                }
            });
        });
        egui::CentralPanel::default().show(ui, |ui| match self.tab {
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

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(&title)
            .with_app_id("shortscutter")
            .with_inner_size([1280.0, 800.0]),
        ..Default::default()
    };
    eframe::run_native(&title, options, Box::new(|_cc| Ok(Box::new(App::new(paths, settings)))))
}
