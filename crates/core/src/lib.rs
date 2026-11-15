//! Модель данных ShortsCutter: шаблоны, пачка, шортсы, ассеты библиотеки, настройки и пути XDG.
//!
//! Крейт ни от чего внутри workspace не зависит; источник истины для превью и рендера.

pub mod asset;
pub mod batch;
pub mod geometry;
pub mod library;
pub mod paths;
pub mod settings;
pub mod subtitles;
pub mod template;

pub use asset::{Asset, AssetId, AssetKind, AssetMeta, History, Orientation, Usage, parse_tags, suggest_kind};
pub use batch::{
    Batch, BatchError, Codec, ConflictPolicy, Container, EncoderPref, MediaInfo, OutputSettings,
    MIN_SPLIT, Short, ShortFlag, ShortId, ShortStatus, Source, SourceId, SourceStatus, TimeRange,
};
pub use geometry::{Color, Rect, rounded_rect_coverage};
pub use library::{LibraryError, TemplateRef, UserTemplate, UserTemplates};
pub use paths::Paths;
pub use subtitles::{EditedWord, Phrase, SpeechWord, SubWord, SubtitleEdits, SubtitleTrack, WordText, word_id, word_ids};
pub use settings::{AnalysisSettings, Language, Settings};
pub use template::{
    Align, AccentMode, Background, Border, Canvas, Layer, Outline, Paint, Shadow, SubtitleMode, Template, TemplateError,
    TextVars, VAlign, SUBTITLE_LINES, SUBTITLE_WORDS_PER_LINE,
    SHORTS_UNSAFE_ZONES, background_crop, gradient_t, main_video_crop, main_video_crop_px,
};
