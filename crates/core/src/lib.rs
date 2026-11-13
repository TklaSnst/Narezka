//! Модель данных ShortsCutter: шаблоны, пачка, шортсы, настройки и пути XDG.
//!
//! Крейт ни от чего внутри workspace не зависит; источник истины для превью и рендера.

pub mod batch;
pub mod geometry;
pub mod paths;
pub mod settings;
pub mod template;

pub use batch::{
    Batch, BatchError, Codec, ConflictPolicy, Container, EncoderPref, MediaInfo, OutputSettings,
    MIN_SPLIT, Short, ShortFlag, ShortId, ShortStatus, Source, SourceId, SourceStatus, TimeRange,
};
pub use geometry::{Color, Rect};
pub use paths::Paths;
pub use settings::{AnalysisSettings, Language, Settings};
pub use template::{Background, Canvas, Layer, Template, TemplateError, background_crop, main_video_crop};
