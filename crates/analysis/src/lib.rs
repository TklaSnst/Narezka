//! Анализ исходника: звук → whisper (Vulkan + VAD) → транскрипт со словами и таймкодами.
//!
//! Структура текста: предложения и паузы ([`sentences`]), границы тем по эмбеддингам ([`topics`]).
//! Дальше по этапу 2: LLM-нарезка (ARCHITECTURE §4).

pub mod audio;
pub mod models;
pub mod sentences;
pub mod topics;
pub mod transcript;
pub mod whisper;

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

pub use models::{ModelSpec, QWEN3_EMBED, SILERO_VAD, WHISPER_TURBO};
pub use sentences::{Pause, Sentence, SplitOptions, pauses, sentences};
pub use topics::{TilingOptions, TopicBoundary, embed_sentences, topic_boundaries};
pub use transcript::{Segment, Span, Transcript, Word};
pub use whisper::{Whisper, WhisperOptions};

#[derive(Debug, thiserror::Error)]
pub enum AnalysisError {
    #[error("не удалось запустить {tool}: {source}")]
    Spawn { tool: &'static str, source: std::io::Error },
    #[error("{tool} завершился с ошибкой: {stderr}")]
    Failed { tool: &'static str, stderr: String },
    #[error("в файле нет звука")]
    NoAudio,
    #[error("путь с нулевым байтом")]
    BadPath,
    #[error("нет модели {}", .0.display())]
    ModelMissing(PathBuf),
    #[error("whisper не загрузил модель {}", .0.display())]
    ModelLoad(PathBuf),
    #[error("whisper_full вернул {0}")]
    Whisper(i32),
    #[error("загрузка: {0}")]
    Download(String),
    #[error("{0}: размер или SHA-256 не совпали")]
    Checksum(&'static str),
    #[error("отменено")]
    Cancelled,
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Выставляет переменные ggml. Вызывать в начале `main`, до потоков и до первой загрузки модели.
///
/// Без ReBAR RADV отдаёт «host-visible VRAM», которая на деле в системной RAM: whisper в 5 раз медленнее
/// (DECISIONS, шаг 0.2).
pub fn prepare_ggml_env() {
    if std::env::var_os("GGML_VK_DISABLE_HOST_VISIBLE_VIDMEM").is_none() {
        // SAFETY: вызывается из main до запуска других потоков.
        unsafe { std::env::set_var("GGML_VK_DISABLE_HOST_VISIBLE_VIDMEM", "1") };
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// Проверка кэша и извлечение звука.
    Audio,
    /// Загрузка модели whisper на GPU (несколько секунд, прогресса нет).
    Model,
    /// Поиск участков речи (Silero VAD внутри whisper, на CPU): ~1% длительности записи,
    /// без прогресса и без отмены изнутри.
    Vad,
    /// Распознавание.
    Speech,
}

/// Распознавание одного исходника; результат кэшируется по хэшу.
#[derive(Debug, Clone)]
pub struct TranscribeJob {
    pub input: PathBuf,
    pub hash: String,
    /// Длительность по ffprobe — для прогресса извлечения звука.
    pub duration: f64,
    pub whisper_model: PathBuf,
    pub vad_model: PathBuf,
    pub language: String,
    /// Словарь терминов (имена, названия) → initial prompt.
    pub terms: Vec<String>,
    pub cache_dir: PathBuf,
}

impl TranscribeJob {
    pub fn prompt(&self) -> String {
        self.terms.iter().map(|t| t.trim()).filter(|t| !t.is_empty()).collect::<Vec<_>>().join(", ")
    }

    /// Транскрипт из кэша или распознавание. Модель whisper выгружается из VRAM до возврата.
    pub fn run(&self, cancel: &AtomicBool, mut progress: impl FnMut(Stage, f32)) -> Result<Transcript, AnalysisError> {
        if let Some(t) = Transcript::load_cached(&self.cache_dir, &self.hash) {
            log::info!("{}: транскрипт из кэша", self.input.display());
            return Ok(t);
        }
        for model in [&self.whisper_model, &self.vad_model] {
            if !model.is_file() {
                return Err(AnalysisError::ModelMissing(model.clone()));
            }
        }
        let started = std::time::Instant::now();
        progress(Stage::Audio, 0.0);
        let pcm = audio::extract_pcm(&self.input, self.duration, cancel, |f| progress(Stage::Audio, f))?;
        let audio_secs = started.elapsed().as_secs_f32();

        progress(Stage::Model, 0.0);
        let opts = WhisperOptions { language: self.language.clone(), prompt: self.prompt(), threads: threads() };
        let mut w = Whisper::load(&self.whisper_model)?;
        let model_secs = started.elapsed().as_secs_f32() - audio_secs;
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(AnalysisError::Cancelled);
        }
        let transcript = w.transcribe(&pcm, &self.vad_model, &opts, cancel, &mut progress)?;
        drop(w); // VRAM освобождается до записи кэша и следующих задач
        log::info!(
            "{}: {} слов, {} участков речи; звук {audio_secs:.1} с, модель {model_secs:.1} с, всего {:.1} с на {:.0} с записи",
            self.input.display(),
            transcript.words.len(),
            transcript.speech.len(),
            started.elapsed().as_secs_f32(),
            pcm.len() as f64 / audio::SAMPLE_RATE as f64,
        );
        if let Err(e) = transcript.save_cached(&self.cache_dir, &self.hash) {
            log::warn!("кэш транскрипта не записан: {e}");
        }
        Ok(transcript)
    }
}

/// Потоки CPU для whisper: считает GPU, больше 4 не дают выигрыша (шаг 0.2).
fn threads() -> i32 {
    std::thread::available_parallelism().map_or(4, |n| n.get().min(4) as i32)
}

/// Путь к модели whisper: свой файл из настроек или стандартная модель в каталоге моделей.
pub fn whisper_model_path(custom: Option<&Path>, models_dir: &Path) -> PathBuf {
    custom.map_or_else(|| WHISPER_TURBO.path(models_dir), Path::to_path_buf)
}
