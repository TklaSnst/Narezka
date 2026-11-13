//! Анализ исходников: очередь (по одному ролику) «распознавание → структура → LLM-нарезка», результаты из кэша,
//! загрузка моделей. Транскрипт показывается сразу, нарезка приходит следом.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use eframe::egui;
use sc_analysis::{
    AnalysisError, CutJob, CutOptions, CutStage, Cutting, ModelSpec, Prompts, QWEN3_EMBED, QWEN35_9B, SILERO_VAD, Stage,
    TranscribeJob, Transcript, WHISPER_TURBO,
};
use sc_core::{Settings, Source, SourceId, SourceStatus};

use crate::App;
use crate::import::file_name;
use crate::jobs::Msg;

/// Где сейчас анализ ролика.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Transcribe(Stage),
    Cut(CutStage),
}

pub enum AnalysisEvent {
    Progress { id: SourceId, step: Step, fraction: f32 },
    /// Ошибка — ключ i18n.
    Transcribed { id: SourceId, result: Result<Transcript, &'static str> },
    Cut { id: SourceId, result: Result<Cutting, &'static str> },
    /// Задача ролика закончилась (после всех результатов).
    Done { id: SourceId },
}

pub enum DownloadEvent {
    Progress { file: &'static str, done: u64, total: u64 },
    Done(Result<(), &'static str>),
}

struct Running {
    id: SourceId,
    cancel: Arc<AtomicBool>,
    step: Step,
    fraction: f32,
}

struct Download {
    cancel: Arc<AtomicBool>,
    file: &'static str,
    done: u64,
    total: u64,
}

pub struct AnalysisState {
    queue: VecDeque<SourceId>,
    running: Option<Running>,
    transcripts: HashMap<SourceId, Arc<Transcript>>,
    /// Почему ролик не распознан (ключ i18n); `analysis-cancelled` — отменён пользователем.
    errors: HashMap<SourceId, &'static str>,
    /// Кандидаты в шортсы (экран «Нарезка» — шаг 2.4).
    cuttings: HashMap<SourceId, Arc<Cutting>>,
    /// Почему распознанный ролик не нарезан (ключ i18n).
    cut_errors: HashMap<SourceId, &'static str>,
    download: Option<Download>,
    download_error: Option<&'static str>,
    /// Словарь терминов в поле ввода — по строке на термин; в настройки уходит при потере фокуса.
    terms_text: String,
}

impl AnalysisState {
    pub fn new(settings: &Settings) -> Self {
        Self {
            queue: VecDeque::new(),
            running: None,
            transcripts: HashMap::new(),
            errors: HashMap::new(),
            cuttings: HashMap::new(),
            cut_errors: HashMap::new(),
            download: None,
            download_error: None,
            terms_text: settings.analysis.terms.join("\n"),
        }
    }

    fn busy(&self, id: SourceId) -> bool {
        self.queue.contains(&id) || self.running.as_ref().is_some_and(|r| r.id == id)
    }
}

/// Что сделать с роликом по кнопке в ячейке состояния.
pub enum RowAction {
    Analyze(SourceId),
    Cancel(SourceId),
    Redo(SourceId),
    Recut(SourceId),
}

impl App {
    fn whisper_model(&self) -> PathBuf {
        sc_analysis::whisper_model_path(self.settings.analysis.whisper_model.as_deref(), &self.paths.models_dir())
    }

    fn whisper_present(&self) -> bool {
        match &self.settings.analysis.whisper_model {
            Some(p) => p.is_file(),
            None => WHISPER_TURBO.present(&self.paths.models_dir()),
        }
    }

    pub(crate) fn models_ready(&self) -> bool {
        self.whisper_present() && SILERO_VAD.present(&self.paths.models_dir())
    }

    /// Модели нарезки: эмбеддинги и LLM.
    fn cut_models_ready(&self) -> bool {
        let dir = self.paths.models_dir();
        QWEN3_EMBED.present(&dir) && QWEN35_9B.present(&dir)
    }

    /// Модели, которых не хватает и которые можно скачать: сначала нужные для распознавания.
    fn missing_models(&self) -> Vec<ModelSpec> {
        let dir = self.paths.models_dir();
        let mut v = Vec::new();
        if !SILERO_VAD.present(&dir) {
            v.push(SILERO_VAD);
        }
        if self.settings.analysis.whisper_model.is_none() && !WHISPER_TURBO.present(&dir) {
            v.push(WHISPER_TURBO);
        }
        v.extend([QWEN3_EMBED, QWEN35_9B].into_iter().filter(|m| !m.present(&dir)));
        v
    }

    /// Ставит ролик в очередь; транскрипт и нарезка из кэша придут сразу, без загрузки моделей.
    pub(crate) fn enqueue_analysis(&mut self, id: SourceId) {
        let Some(s) = self.batch.source(id) else { return };
        if self.analysis.busy(id) || self.analysis.cuttings.contains_key(&id) || s.hash.is_none() {
            return;
        }
        if s.info.as_ref().is_some_and(|i| !i.has_audio) {
            self.analysis.errors.insert(id, "analysis-err-no-audio");
            return;
        }
        self.analysis.errors.remove(&id);
        self.analysis.cut_errors.remove(&id);
        self.analysis.queue.push_back(id);
    }

    /// Запускает следующий ролик из очереди, если сейчас ничего не распознаётся.
    pub(crate) fn pump_analysis(&mut self) {
        if self.analysis.running.is_some() {
            return;
        }
        while let Some(id) = self.analysis.queue.pop_front() {
            let Some(s) = self.batch.source(id) else { continue };
            let Some(job) = self.transcribe_job(s) else { continue };
            let cut = self.cut_job(s);
            if let Some(s) = self.batch.source_mut(id) {
                s.status = SourceStatus::Analyzing;
            }
            let cancel = Arc::new(AtomicBool::new(false));
            let step = Step::Transcribe(Stage::Audio);
            self.analysis.running = Some(Running { id, cancel: cancel.clone(), step, fraction: 0.0 });
            let notify = self.jobs.notifier();
            std::thread::spawn(move || {
                let mut last = (step, -1.0);
                let mut report = |step: Step, fraction: f32| {
                    if step != last.0 || fraction - last.1 >= 0.01 || fraction >= 1.0 {
                        last = (step, fraction);
                        notify.send(Msg::Analysis(AnalysisEvent::Progress { id, step, fraction }));
                    }
                };
                let name = job.input.display().to_string();
                let transcript = job.run(&cancel, |stage, f| report(Step::Transcribe(stage), f));
                let transcript = match transcript {
                    Ok(t) => {
                        notify.send(Msg::Analysis(AnalysisEvent::Transcribed { id, result: Ok(t.clone()) }));
                        Some(t)
                    }
                    Err(e) => {
                        log_error(&name, &e);
                        notify.send(Msg::Analysis(AnalysisEvent::Transcribed { id, result: Err(analysis_error_key(&e)) }));
                        None
                    }
                };
                if let Some(t) = transcript {
                    let result = match cut {
                        Ok(cut) => cut.run(&t, &cancel, |stage, f| report(Step::Cut(stage), f)).map_err(|e| {
                            log_error(&name, &e);
                            cut_error_key(&e)
                        }),
                        Err(key) => Err(key),
                    };
                    notify.send(Msg::Analysis(AnalysisEvent::Cut { id, result }));
                }
                notify.send(Msg::Analysis(AnalysisEvent::Done { id }));
            });
            return;
        }
    }

    /// Задача нарезки; ошибка (ключ i18n) — если промпты не читаются.
    fn cut_job(&self, s: &Source) -> Result<CutJob, &'static str> {
        let prompts = Prompts::load(&self.paths.prompts_dir()).map_err(|e| {
            log::warn!("промпты нарезки: {e}");
            "analysis-err-prompt"
        })?;
        let dir = self.paths.models_dir();
        Ok(CutJob {
            hash: s.hash.clone().unwrap_or_default(),
            cache_dir: self.paths.analysis_dir(),
            embed_model: QWEN3_EMBED.path(&dir),
            llm_model: QWEN35_9B.path(&dir),
            prompts,
            opts: CutOptions {
                title: s.path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                ..CutOptions::default()
            },
        })
    }

    fn transcribe_job(&self, s: &Source) -> Option<TranscribeJob> {
        Some(TranscribeJob {
            input: s.path.clone(),
            hash: s.hash.clone()?,
            duration: s.info.as_ref()?.duration,
            whisper_model: self.whisper_model(),
            vad_model: SILERO_VAD.path(&self.paths.models_dir()),
            language: self.settings.analysis.language.clone(),
            terms: self.settings.analysis.terms.clone(),
            cache_dir: self.paths.analysis_dir(),
        })
    }

    pub(crate) fn on_analysis(&mut self, ev: AnalysisEvent) {
        match ev {
            AnalysisEvent::Progress { id, step, fraction } => {
                if let Some(r) = self.analysis.running.as_mut().filter(|r| r.id == id) {
                    r.step = step;
                    r.fraction = fraction;
                }
            }
            AnalysisEvent::Done { id } => {
                if self.analysis.running.as_ref().is_some_and(|r| r.id == id) {
                    self.analysis.running = None;
                }
            }
            AnalysisEvent::Cut { id, result } => {
                if self.batch.source(id).is_none() {
                    return; // убран из пачки во время работы
                }
                match result {
                    Ok(c) => {
                        self.analysis.cuttings.insert(id, Arc::new(c));
                    }
                    Err(key) => {
                        self.analysis.cut_errors.insert(id, key);
                    }
                }
            }
            AnalysisEvent::Transcribed { id, result } => {
                let Some(s) = self.batch.source_mut(id) else { return }; // убран из пачки во время работы
                match result {
                    Ok(t) => {
                        s.status = SourceStatus::Analyzed;
                        self.analysis.transcripts.insert(id, Arc::new(t));
                    }
                    Err(key) => {
                        s.status = SourceStatus::Ready;
                        self.analysis.errors.insert(id, key);
                    }
                }
            }
        }
    }

    pub(crate) fn cancel_analysis(&mut self, id: SourceId) {
        if let Some(pos) = self.analysis.queue.iter().position(|q| *q == id) {
            self.analysis.queue.remove(pos);
            self.analysis.errors.insert(id, "analysis-cancelled");
        }
        if let Some(r) = self.analysis.running.as_ref().filter(|r| r.id == id) {
            r.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// Ролик убран из пачки: остановить и забыть (кэш на диске остаётся).
    pub(crate) fn forget_analysis(&mut self, id: SourceId) {
        self.cancel_analysis(id);
        self.analysis.transcripts.remove(&id);
        self.analysis.errors.remove(&id);
        self.analysis.cuttings.remove(&id);
        self.analysis.cut_errors.remove(&id);
    }

    /// Распознать заново — например, после правки словаря терминов. Нарезка — тоже заново.
    fn redo_analysis(&mut self, id: SourceId) {
        if let Some(hash) = self.batch.source(id).and_then(|s| s.hash.clone()) {
            let dir = self.paths.analysis_dir();
            let _ = std::fs::remove_file(Transcript::cache_file(&dir, &hash));
            let _ = std::fs::remove_file(Cutting::cache_file(&dir, &hash));
        }
        self.analysis.transcripts.remove(&id);
        self.analysis.cuttings.remove(&id);
        if let Some(s) = self.batch.source_mut(id) {
            s.status = SourceStatus::Ready;
        }
        self.enqueue_analysis(id);
    }

    /// Нарезать заново по тому же транскрипту — например, после правки промптов.
    fn recut(&mut self, id: SourceId) {
        if let Some(hash) = self.batch.source(id).and_then(|s| s.hash.clone()) {
            let _ = std::fs::remove_file(Cutting::cache_file(&self.paths.analysis_dir(), &hash));
        }
        self.analysis.cuttings.remove(&id);
        self.enqueue_analysis(id);
    }

    pub(crate) fn apply_row_action(&mut self, action: RowAction) {
        match action {
            RowAction::Analyze(id) => self.enqueue_analysis(id),
            RowAction::Cancel(id) => self.cancel_analysis(id),
            RowAction::Redo(id) => self.redo_analysis(id),
            RowAction::Recut(id) => self.recut(id),
        }
    }

    /// Строка состояния: какой ролик распознаётся.
    pub(crate) fn analysis_status(&self) -> Option<String> {
        let r = self.analysis.running.as_ref()?;
        let name = self.batch.source(r.id).map(|s| file_name(&s.path)).unwrap_or_default();
        let stage = self.stage_text(r);
        let queued = self.analysis.queue.len() as i64;
        Some(self.i18n.tr_args(
            "analysis-status-running",
            &[("name", name.into()), ("stage", stage.into()), ("queued", queued.into())],
        ))
    }

    /// Ячейка «Состояние» в «Импорте» для прочитанного ролика: анализ, прогресс, ошибка.
    pub(crate) fn analysis_cell(&self, ui: &mut egui::Ui, s: &Source) -> Option<RowAction> {
        let id = s.id;
        let mut action = None;
        ui.horizontal(|ui| {
            if let Some(r) = self.analysis.running.as_ref().filter(|r| r.id == id) {
                let bar = egui::ProgressBar::new(r.fraction).desired_width(180.0).text(self.stage_text(r));
                // У загрузки моделей и VAD нет прогресса — только анимация.
                ui.add(bar.animate(!has_progress(r.step)));
                let cancelling = r.cancel.load(Ordering::Relaxed);
                let stop = egui::Button::new(self.i18n.tr("analysis-stop")).small();
                if ui.add_enabled(!cancelling, stop).on_hover_text(self.i18n.tr("analysis-cancel")).clicked() {
                    action = Some(RowAction::Cancel(id));
                }
            } else if self.analysis.queue.contains(&id) {
                ui.label(self.i18n.tr("analysis-queued"));
                if ui.small_button(self.i18n.tr("analysis-stop")).on_hover_text(self.i18n.tr("analysis-cancel")).clicked() {
                    action = Some(RowAction::Cancel(id));
                }
            } else if let Some(t) = self.analysis.transcripts.get(&id) {
                let n = t.words.len() as i64;
                ui.label(self.i18n.tr_args("analysis-done", &[("words", n.into())]));
                if let Some(c) = self.analysis.cuttings.get(&id) {
                    let n = c.candidates.len() as i64;
                    ui.label(self.i18n.tr_args("analysis-cut-done", &[("n", n.into())]));
                    if ui.small_button("✂").on_hover_text(self.i18n.tr("analysis-recut")).clicked() {
                        action = Some(RowAction::Recut(id));
                    }
                } else {
                    if let Some(key) = self.analysis.cut_errors.get(&id) {
                        if *key == "analysis-cancelled" {
                            ui.weak(self.i18n.tr("analysis-cut-cancelled"));
                        } else {
                            ui.colored_label(ui.visuals().error_fg_color, self.i18n.tr(key));
                        }
                    }
                    if ui.small_button("▶").on_hover_text(self.i18n.tr("analysis-cut-start")).clicked() {
                        action = Some(RowAction::Analyze(id));
                    }
                }
                if ui.small_button("⟳").on_hover_text(self.i18n.tr("analysis-redo")).clicked() {
                    action = Some(RowAction::Redo(id));
                }
            } else if let Some(key) = self.analysis.errors.get(&id) {
                if *key == "analysis-cancelled" {
                    ui.weak(self.i18n.tr(key));
                } else {
                    ui.colored_label(ui.visuals().error_fg_color, self.i18n.tr(key));
                }
                if *key != "analysis-err-no-audio" && ui.small_button("▶").on_hover_text(self.i18n.tr("analysis-start")).clicked() {
                    action = Some(RowAction::Analyze(id));
                }
            } else {
                ui.label(self.i18n.tr("source-status-ready"));
                if ui.small_button("▶").on_hover_text(self.i18n.tr("analysis-start")).clicked() {
                    action = Some(RowAction::Analyze(id));
                }
            }
        });
        action
    }

    fn stage_text(&self, r: &Running) -> String {
        let name = self.i18n.tr(step_key(r.step));
        if has_progress(r.step) { format!("{name} {:.0}%", r.fraction * 100.0) } else { name }
    }

    // --- Модели ---

    fn start_download(&mut self) {
        let specs = self.missing_models();
        let Some(first) = specs.first() else { return };
        let cancel = Arc::new(AtomicBool::new(false));
        self.analysis.download_error = None;
        self.analysis.download = Some(Download { cancel: cancel.clone(), file: first.file, done: 0, total: first.size });
        let dir = self.paths.models_dir();
        let notify = self.jobs.notifier();
        std::thread::spawn(move || {
            let mut result = Ok(());
            for spec in specs {
                let r = spec.download(&dir, &cancel, |done, total| {
                    notify.send(Msg::Download(DownloadEvent::Progress { file: spec.file, done, total }));
                });
                if let Err(e) = r {
                    if !matches!(e, AnalysisError::Cancelled) {
                        log::warn!("загрузка {}: {e}", spec.file);
                    }
                    result = Err(analysis_error_key(&e));
                    break;
                }
            }
            notify.send(Msg::Download(DownloadEvent::Done(result)));
        });
    }

    pub(crate) fn on_download(&mut self, ev: DownloadEvent) {
        match ev {
            DownloadEvent::Progress { file, done, total } => {
                if let Some(d) = &mut self.analysis.download {
                    (d.file, d.done, d.total) = (file, done, total);
                }
            }
            DownloadEvent::Done(result) => {
                self.analysis.download = None;
                match result {
                    Ok(()) => self.retry_without_models(),
                    Err("analysis-cancelled") => {}
                    Err(key) => self.analysis.download_error = Some(key),
                }
            }
        }
    }

    /// Модели появились — повторить ролики, которые на них споткнулись.
    fn retry_without_models(&mut self) {
        let a = &self.analysis;
        let ids: Vec<SourceId> = (a.errors.iter().filter(|(_, k)| **k == "analysis-err-no-model"))
            .chain(a.cut_errors.iter().filter(|(_, k)| **k == "analysis-err-no-llm"))
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            self.enqueue_analysis(id);
        }
    }

    fn pick_model_file(&mut self) {
        self.dialog_open = true;
        let title = self.i18n.tr("models-pick-dialog");
        let filter = self.i18n.tr("models-pick-filter");
        let dir = self.paths.models_dir();
        self.jobs.spawn(move || {
            let file = rfd::FileDialog::new().set_title(title).add_filter(filter, &["bin"]).set_directory(dir).pick_file();
            Msg::PickedModel(file)
        });
    }

    pub(crate) fn on_picked_model(&mut self, file: Option<PathBuf>) {
        self.dialog_open = false;
        if let Some(file) = file {
            self.settings.analysis.whisper_model = Some(file);
            self.save_settings();
            self.retry_without_models();
        }
    }

    fn download_ui(&mut self, ui: &mut egui::Ui) {
        if let Some(d) = &self.analysis.download {
            let fraction = if d.total > 0 { d.done as f32 / d.total as f32 } else { 0.0 };
            let text = self.i18n.tr_args(
                "models-downloading",
                &[("file", d.file.into()), ("done", gb(d.done).into()), ("total", gb(d.total).into())],
            );
            ui.add(egui::ProgressBar::new(fraction).text(text).desired_width(420.0));
            let cancelling = d.cancel.load(Ordering::Relaxed);
            if ui.add_enabled(!cancelling, egui::Button::new(self.i18n.tr("models-download-cancel"))).clicked() {
                d.cancel.store(true, Ordering::Relaxed);
            }
            return;
        }
        let missing = self.missing_models();
        ui.horizontal(|ui| {
            if !missing.is_empty() {
                let size: u64 = missing.iter().map(|m| m.size).sum();
                if ui.button(self.i18n.tr_args("models-download", &[("size", gb(size).into())])).clicked() {
                    self.start_download();
                }
            }
            if ui.add_enabled(!self.dialog_open, egui::Button::new(self.i18n.tr("models-pick"))).clicked() {
                self.pick_model_file();
            }
        });
        if let Some(key) = self.analysis.download_error {
            ui.colored_label(ui.visuals().error_fg_color, self.i18n.tr(key));
        }
    }

    /// Плашка на «Импорте», пока нет моделей: без них работает только ручная нарезка.
    pub(crate) fn models_banner(&mut self, ui: &mut egui::Ui) {
        if self.models_ready() && self.cut_models_ready() {
            return;
        }
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.label(self.i18n.tr("models-needed"));
            self.download_ui(ui);
        });
        ui.add_space(8.0);
    }

    /// Раздел «Распознавание речи» в настройках.
    pub(crate) fn analysis_settings_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading(self.i18n.tr("settings-analysis"));
        let dir = self.paths.models_dir();
        let mark = |ok: bool| if ok { "✔" } else { "✖" };
        egui::Grid::new("models").num_columns(3).spacing([12.0, 6.0]).show(ui, |ui| {
            ui.label(self.i18n.tr("models-whisper"));
            ui.label(mark(self.whisper_present()));
            ui.horizontal(|ui| {
                ui.monospace(self.whisper_model().display().to_string());
                if self.settings.analysis.whisper_model.is_some()
                    && ui.small_button(self.i18n.tr("models-reset")).on_hover_text(self.i18n.tr("models-reset-hint")).clicked()
                {
                    self.settings.analysis.whisper_model = None;
                    self.save_settings();
                }
            });
            ui.end_row();
            for (key, spec) in [("models-vad", SILERO_VAD), ("models-embed", QWEN3_EMBED), ("models-llm", QWEN35_9B)] {
                ui.label(self.i18n.tr(key));
                ui.label(mark(spec.present(&dir)));
                ui.monospace(spec.path(&dir).display().to_string());
                ui.end_row();
            }
        });
        self.download_ui(ui);
        ui.weak(self.i18n.tr_args("settings-prompts-hint", &[("dir", self.paths.prompts_dir().display().to_string().into())]));

        ui.add_space(8.0);
        ui.label(self.i18n.tr("settings-terms"));
        ui.weak(self.i18n.tr("settings-terms-hint"));
        let edit = ui.add(
            egui::TextEdit::multiline(&mut self.analysis.terms_text).desired_rows(4).desired_width(420.0),
        );
        if edit.lost_focus() {
            let terms: Vec<String> =
                self.analysis.terms_text.lines().map(str::trim).filter(|t| !t.is_empty()).map(str::to_owned).collect();
            if terms != self.settings.analysis.terms {
                self.settings.analysis.terms = terms;
                self.save_settings();
            }
        }
    }
}

fn step_key(step: Step) -> &'static str {
    match step {
        Step::Transcribe(Stage::Audio) => "analysis-stage-audio",
        Step::Transcribe(Stage::Model) => "analysis-stage-model",
        Step::Transcribe(Stage::Vad) => "analysis-stage-vad",
        Step::Transcribe(Stage::Speech) => "analysis-stage-speech",
        Step::Cut(CutStage::Structure) => "analysis-stage-structure",
        Step::Cut(CutStage::Model) => "analysis-stage-llm-model",
        Step::Cut(CutStage::Llm) => "analysis-stage-cut",
    }
}

/// У стадии есть доля выполненного (иначе — только анимация).
fn has_progress(step: Step) -> bool {
    matches!(step, Step::Transcribe(Stage::Audio | Stage::Speech) | Step::Cut(CutStage::Llm))
}

fn log_error(name: &str, e: &AnalysisError) {
    if !matches!(e, AnalysisError::Cancelled) {
        log::warn!("анализ {name}: {e}");
    }
}

/// Ошибка нарезки → ключ i18n: нет моделей нарезки — своё сообщение.
fn cut_error_key(e: &AnalysisError) -> &'static str {
    match e {
        AnalysisError::ModelMissing(_) => "analysis-err-no-llm",
        e => analysis_error_key(e),
    }
}

fn analysis_error_key(e: &AnalysisError) -> &'static str {
    match e {
        AnalysisError::Spawn { .. } => "caps-no-ffmpeg",
        AnalysisError::Failed { .. } => "analysis-err-ffmpeg",
        AnalysisError::NoAudio => "analysis-err-no-audio",
        AnalysisError::ModelMissing(_) => "analysis-err-no-model",
        AnalysisError::ModelLoad(_) => "analysis-err-model-load",
        AnalysisError::Whisper(_) => "analysis-err-whisper",
        AnalysisError::Download(_) => "models-err-network",
        AnalysisError::Checksum(_) => "models-err-checksum",
        AnalysisError::Prompt(_) => "analysis-err-prompt",
        AnalysisError::Llama(_) => "analysis-err-llm",
        AnalysisError::Cancelled => "analysis-cancelled",
        AnalysisError::Io(_) | AnalysisError::BadPath => "analysis-err-io",
    }
}

/// Байты → «1.6» (ГБ) для подписей.
fn gb(bytes: u64) -> String {
    format!("{:.1}", bytes as f64 / 1e9)
}
