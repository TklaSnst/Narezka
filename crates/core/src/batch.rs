//! Пачка: исходники, шортсы на них, шаблон и настройки вывода.
//!
//! Пачка не сохраняется как проект (см. DECISIONS), но сериализуема — для отладки и
//! «как в прошлый раз».

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::template::{Canvas, Template};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SourceId(pub Uuid);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ShortId(pub Uuid);

impl SourceId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl ShortId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for SourceId {
    fn default() -> Self {
        Self::new()
    }
}

impl Default for ShortId {
    fn default() -> Self {
        Self::new()
    }
}

/// Сведения ffprobe об исходнике.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaInfo {
    /// Длительность, секунды.
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub has_audio: bool,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceStatus {
    /// Добавлен, ffprobe ещё не отработал.
    #[default]
    New,
    /// Прочитан, можно резать вручную.
    Ready,
    Analyzing,
    Analyzed,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Source {
    pub id: SourceId,
    pub path: PathBuf,
    #[serde(default)]
    pub info: Option<MediaInfo>,
    /// Быстрый хэш файла (ключ кэша транскрипции).
    #[serde(default)]
    pub hash: Option<String>,
    #[serde(default)]
    pub status: SourceStatus,
}

impl Source {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            id: SourceId::new(),
            path: path.into(),
            info: None,
            hash: None,
            status: SourceStatus::New,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShortStatus {
    /// Предложен анализом, пользователь ещё не решил.
    Candidate,
    #[default]
    Accepted,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Short {
    pub id: ShortId,
    pub source_id: SourceId,
    /// Начало и конец в секундах исходника.
    pub start: f64,
    pub end: f64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub score: Option<f32>,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub status: ShortStatus,
}

impl Short {
    /// Шортс, отмеченный вручную (сразу принят).
    pub fn manual(source_id: SourceId, start: f64, end: f64) -> Self {
        Self {
            id: ShortId::new(),
            source_id,
            start,
            end,
            title: String::new(),
            score: None,
            reason: String::new(),
            status: ShortStatus::Accepted,
        }
    }

    pub fn duration(&self) -> f64 {
        self.end - self.start
    }

    fn overlaps(&self, other: &Short) -> bool {
        self.source_id == other.source_id && self.start < other.end && other.start < self.end
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictPolicy {
    /// Продолжить нумерацию после существующих файлов.
    #[default]
    Continue,
    Ask,
    Overwrite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Container {
    #[default]
    Mp4,
    Mkv,
    Webm,
    Mov,
}

impl Container {
    pub fn extension(self) -> &'static str {
        match self {
            Container::Mp4 => "mp4",
            Container::Mkv => "mkv",
            Container::Webm => "webm",
            Container::Mov => "mov",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Codec {
    #[default]
    H264,
    Hevc,
    Av1,
    Vp9,
    ProRes,
}

/// Чем кодировать: `Auto` — GPU, если пробное кодирование при запуске прошло, иначе CPU.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EncoderPref {
    #[default]
    Auto,
    Gpu,
    Cpu,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OutputSettings {
    /// Папка вывода; `None` — рядом с исходником.
    pub dir: Option<PathBuf>,
    /// Базовое имя файлов: `<base_name>_1.mp4`, `<base_name>_2.mp4`…
    pub base_name: String,
    pub container: Container,
    pub codec: Codec,
    pub encoder: EncoderPref,
    pub video_bitrate_kbps: u32,
    pub audio_bitrate_kbps: u32,
    /// Выходное разрешение; `None` — размер холста шаблона.
    pub resolution: Option<Canvas>,
    /// Частота кадров; `None` — как у исходника.
    pub fps: Option<f64>,
    pub conflict_policy: ConflictPolicy,
    pub export_metadata: bool,
    pub notify: bool,
}

impl Default for OutputSettings {
    fn default() -> Self {
        Self {
            dir: None,
            base_name: "short".into(),
            container: Container::Mp4,
            codec: Codec::H264,
            encoder: EncoderPref::Auto,
            video_bitrate_kbps: 12_000,
            audio_bitrate_kbps: 192,
            resolution: None,
            fps: None,
            conflict_policy: ConflictPolicy::Continue,
            export_metadata: false,
            notify: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum BatchError {
    #[error("исходник не найден в пачке")]
    UnknownSource,
    #[error("шортс не найден")]
    UnknownShort,
    #[error("неверные границы шортса: {start:.3}–{end:.3} с")]
    BadRange { start: f64, end: f64 },
    #[error("шортс выходит за длительность исходника ({duration:.3} с)")]
    OutOfSource { duration: f64 },
    #[error("шортс пересекается с уже существующим")]
    Overlap(ShortId),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Batch {
    #[serde(default)]
    pub sources: Vec<Source>,
    #[serde(default)]
    pub shorts: Vec<Short>,
    pub template: Template,
    #[serde(default)]
    pub output: OutputSettings,
}

impl Default for Batch {
    fn default() -> Self {
        Self {
            sources: Vec::new(),
            shorts: Vec::new(),
            template: Template::minimal(),
            output: OutputSettings::default(),
        }
    }
}

impl Batch {
    pub fn source(&self, id: SourceId) -> Option<&Source> {
        self.sources.iter().find(|s| s.id == id)
    }

    pub fn source_mut(&mut self, id: SourceId) -> Option<&mut Source> {
        self.sources.iter_mut().find(|s| s.id == id)
    }

    pub fn short(&self, id: ShortId) -> Option<&Short> {
        self.shorts.iter().find(|s| s.id == id)
    }

    /// Добавляет исходник; повторное добавление того же пути возвращает существующий id.
    pub fn add_source(&mut self, source: Source) -> SourceId {
        if let Some(existing) = self.sources.iter().find(|s| s.path == source.path) {
            return existing.id;
        }
        let id = source.id;
        self.sources.push(source);
        id
    }

    /// Удаляет исходник вместе с его шортсами.
    pub fn remove_source(&mut self, id: SourceId) -> Result<Source, BatchError> {
        let idx = self.sources.iter().position(|s| s.id == id).ok_or(BatchError::UnknownSource)?;
        self.shorts.retain(|s| s.source_id != id);
        Ok(self.sources.remove(idx))
    }

    /// Шортсы исходника по времени начала.
    pub fn shorts_of(&self, source: SourceId) -> Vec<&Short> {
        let mut v: Vec<_> = self.shorts.iter().filter(|s| s.source_id == source).collect();
        v.sort_by(|a, b| a.start.total_cmp(&b.start));
        v
    }

    /// Проверяет границы шортса относительно исходника и остальных шортсов
    /// (кроме него самого — для редактирования). Перекрытия запрещены (DECISIONS).
    pub fn check_short(&self, short: &Short) -> Result<(), BatchError> {
        let source = self.source(short.source_id).ok_or(BatchError::UnknownSource)?;
        let (start, end) = (short.start, short.end);
        if !start.is_finite() || !end.is_finite() || start < 0.0 || end <= start {
            return Err(BatchError::BadRange { start, end });
        }
        if let Some(info) = &source.info
            && end > info.duration + 1e-3
        {
            return Err(BatchError::OutOfSource { duration: info.duration });
        }
        if let Some(other) = self
            .shorts
            .iter()
            .find(|o| o.id != short.id && o.status != ShortStatus::Rejected && o.overlaps(short))
        {
            return Err(BatchError::Overlap(other.id));
        }
        Ok(())
    }

    pub fn add_short(&mut self, short: Short) -> Result<ShortId, BatchError> {
        self.check_short(&short)?;
        let id = short.id;
        self.shorts.push(short);
        Ok(id)
    }

    /// Заменяет шортс с тем же id после проверки.
    pub fn update_short(&mut self, short: Short) -> Result<(), BatchError> {
        let idx = self.shorts.iter().position(|s| s.id == short.id).ok_or(BatchError::UnknownShort)?;
        self.check_short(&short)?;
        self.shorts[idx] = short;
        Ok(())
    }

    pub fn remove_short(&mut self, id: ShortId) -> Result<Short, BatchError> {
        let idx = self.shorts.iter().position(|s| s.id == id).ok_or(BatchError::UnknownShort)?;
        Ok(self.shorts.remove(idx))
    }
}
