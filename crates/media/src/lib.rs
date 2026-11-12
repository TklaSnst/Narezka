//! Работа с внешними ffmpeg/ffprobe: сведения о файле, быстрый хэш, проверка возможностей.
//!
//! ffmpeg — системный; что он умеет, выясняется при запуске (`Capabilities::detect`), а не предполагается.

pub mod caps;
pub mod hash;
pub mod probe;
mod process;

pub use caps::{Capabilities, EncoderInfo, EncoderKind};
pub use hash::quick_hash;
pub use probe::probe;

#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    #[error("не удалось запустить {tool}: {source}")]
    Spawn { tool: &'static str, source: std::io::Error },
    #[error("{tool} завершился с ошибкой: {stderr}")]
    Failed { tool: &'static str, stderr: String },
    #[error("{tool} не ответил за {secs} с")]
    Timeout { tool: &'static str, secs: u64 },
    #[error("не удалось разобрать вывод ffprobe: {0}")]
    Parse(String),
    #[error("в файле нет видеодорожки")]
    NoVideo,
    #[error("ошибка чтения файла: {0}")]
    Io(#[from] std::io::Error),
}
