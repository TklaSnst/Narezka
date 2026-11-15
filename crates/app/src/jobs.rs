//! Фоновые задачи: результаты приходят в UI через канал и будят перерисовку.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;

use eframe::egui;
use sc_core::{MediaInfo, SourceId};
use sc_media::{Capabilities, MediaError};

use crate::analysis::{AnalysisEvent, DownloadEvent};
use crate::design::FileTarget;
use crate::export::RenderEvent;

pub enum Msg {
    Caps(Capabilities),
    /// Файлы из диалога; пустой список — диалог закрыт без выбора.
    Picked(Vec<PathBuf>),
    /// ffprobe + хэш; ошибка — ключ i18n.
    Probed { id: SourceId, result: Result<(MediaInfo, String), &'static str> },
    /// Папка вывода из диалога; `None` — диалог закрыт без выбора.
    PickedDir(Option<PathBuf>),
    Render(RenderEvent),
    Analysis(AnalysisEvent),
    Download(DownloadEvent),
    /// Файл модели whisper из диалога; `None` — диалог закрыт без выбора.
    PickedModel(Option<PathBuf>),
    /// Картинка или видео для шаблона из диалога; `None` — диалог закрыт без выбора.
    PickedFile(FileTarget, Option<PathBuf>),
    /// Файлы для библиотеки из диалога; пустой список — диалог закрыт без выбора.
    LibraryPicked(Vec<PathBuf>),
    /// Файл скопирован в библиотеку (или не получилось).
    LibraryImported(crate::library::Imported),
}

/// Отправитель для долгих задач, которые шлют несколько сообщений (рендер, анализ, загрузка).
pub struct Notifier {
    tx: Sender<Msg>,
    ctx: egui::Context,
}

impl Notifier {
    pub fn send(&self, msg: Msg) {
        let _ = self.tx.send(msg);
        self.ctx.request_repaint();
    }
}

pub struct Jobs {
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    ctx: egui::Context,
}

impl Jobs {
    pub fn new(ctx: egui::Context) -> Self {
        let (tx, rx) = channel();
        Self { tx, rx, ctx }
    }

    pub fn spawn(&self, f: impl FnOnce() -> Msg + Send + 'static) {
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        thread::spawn(move || {
            let _ = tx.send(f());
            ctx.request_repaint();
        });
    }

    pub fn notifier(&self) -> Notifier {
        Notifier { tx: self.tx.clone(), ctx: self.ctx.clone() }
    }

    pub fn drain(&self) -> impl Iterator<Item = Msg> + '_ {
        self.rx.try_iter()
    }

    pub fn probe(&self, id: SourceId, path: PathBuf) {
        self.spawn(move || {
            let result = sc_media::probe(&path)
                .and_then(|info| Ok((info, sc_media::quick_hash(&path)?)))
                .map_err(|e| {
                    log::warn!("{}: {e}", path.display());
                    media_error_key(&e)
                });
            Msg::Probed { id, result }
        });
    }
}

fn media_error_key(e: &MediaError) -> &'static str {
    match e {
        MediaError::Spawn { .. } => "media-err-no-ffprobe",
        MediaError::Timeout { .. } => "media-err-timeout",
        MediaError::NoVideo => "media-err-no-video",
        MediaError::Io(_) => "media-err-io",
        MediaError::Failed { .. } | MediaError::Parse(_) => "media-err-unreadable",
    }
}
