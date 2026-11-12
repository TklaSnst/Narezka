//! Фоновые задачи: результаты приходят в UI через канал и будят перерисовку.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;

use eframe::egui;
use sc_core::{MediaInfo, SourceId};
use sc_media::{Capabilities, MediaError};

pub enum Msg {
    Caps(Capabilities),
    /// Файлы из диалога; пустой список — диалог закрыт без выбора.
    Picked(Vec<PathBuf>),
    /// ffprobe + хэш; ошибка — ключ i18n.
    Probed { id: SourceId, result: Result<(MediaInfo, String), &'static str> },
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
