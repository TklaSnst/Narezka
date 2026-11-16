//! Рендер шортсов через системный ffmpeg: граф шаблона, имена файлов, запуск с прогрессом и отменой.
//!
//! От превью не зависит: кропы и параметры фона берутся из модели `sc-core` по тем же формулам.
//! Кодировщик выбирает вызывающий (по `sc_media::Capabilities`). Очередь — [`queue::Queue`].

pub mod encode;
pub mod graph;
pub mod job;
pub mod naming;
pub mod queue;
pub mod subtitles;

pub use job::{CoverFrame, CoverJob, Encoder, InsertJob, RenderError, RenderJob};
pub use naming::{conflicts, plan_outputs, plan_outputs_with};
pub use queue::{Queue, QueueEvent};
pub use subtitles::{SubtitleSeq, sequence as subtitle_sequence};
