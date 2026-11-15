//! Рендер шортсов через системный ffmpeg: граф шаблона, имена файлов, запуск с прогрессом и отменой.
//!
//! От превью не зависит: кропы и параметры фона берутся из модели `sc-core` по тем же формулам.
//! Кодировщик выбирает вызывающий (по `sc_media::Capabilities`). Очередь задач — этап 7.

pub mod graph;
pub mod job;
pub mod naming;
pub mod subtitles;

pub use job::{Encoder, InsertJob, RenderError, RenderJob};
pub use naming::plan_outputs;
pub use subtitles::{SubtitleSeq, sequence as subtitle_sequence};
