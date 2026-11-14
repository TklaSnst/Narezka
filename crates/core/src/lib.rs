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
pub use geometry::{Color, Rect, rounded_rect_coverage};
pub use paths::Paths;
pub use settings::{AnalysisSettings, Language, Settings};
pub use template::{
    Align, AccentMode, Background, Border, Canvas, Layer, Outline, Paint, Shadow, Template, TemplateError, TextVars,
    VAlign,
    background_crop, gradient_t, main_video_crop, main_video_crop_px,
};
