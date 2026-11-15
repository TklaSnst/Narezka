//! Растры субтитров для превью — в отдельном потоке: фраза меняет вид на каждом слове, а растр с тенью
//! считается миллисекунды–десятки миллисекунд. Плеер просит текущее состояние и несколько следующих
//! (упреждение при воспроизведении); из очереди берётся только последний набор просьб.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use eframe::egui;
use sc_core::{Canvas, Layer, SubtitleTrack};
use sc_raster::LayerArt;

/// Что нарисовано: слой (со стилем и прямоугольником), дорожка, фраза и сколько слов начато.
#[derive(Clone, PartialEq)]
pub struct SubKey {
    pub layer: Layer,
    pub canvas: Canvas,
    pub track: Arc<SubtitleTrack>,
    pub phrase: usize,
    pub spoken: usize,
    /// [`sc_raster::fonts::generation`] на момент просьбы.
    pub fonts: u64,
}

impl SubKey {
    /// Те же ли данные (дорожка сравнивается по указателю — она неизменна).
    pub fn same(&self, other: &SubKey) -> bool {
        Arc::ptr_eq(&self.track, &other.track)
            && (self.phrase, self.spoken, self.fonts, self.canvas) == (other.phrase, other.spoken, other.fonts, other.canvas)
            && self.layer == other.layer
    }
}

pub struct SubtitleWorker {
    tx: Sender<Vec<SubKey>>,
    rx: Receiver<(SubKey, Arc<LayerArt>)>,
}

impl SubtitleWorker {
    pub fn new(ctx: egui::Context) -> Self {
        let (tx, jobs) = channel::<Vec<SubKey>>();
        let (done, rx) = channel();
        std::thread::Builder::new()
            .name("preview-subtitles".into())
            .spawn(move || run(jobs, done, ctx))
            .expect("поток субтитров превью");
        Self { tx, rx }
    }

    /// Нарисовать по порядку (первое — нужное сейчас); прошлый набор, если не досчитан, отменяется.
    pub fn request(&self, keys: Vec<SubKey>) {
        let _ = self.tx.send(keys);
    }

    pub fn poll(&self) -> impl Iterator<Item = (SubKey, Arc<LayerArt>)> + '_ {
        self.rx.try_iter()
    }
}

fn run(jobs: Receiver<Vec<SubKey>>, done: Sender<(SubKey, Arc<LayerArt>)>, ctx: egui::Context) {
    let mut queue: Vec<SubKey> = Vec::new();
    loop {
        if queue.is_empty() {
            match jobs.recv() {
                Ok(keys) => queue = keys,
                Err(_) => return,
            }
        }
        // Новый набор просьб заменяет недосчитанный.
        while let Ok(keys) = jobs.try_recv() {
            queue = keys;
        }
        if queue.is_empty() {
            continue;
        }
        let key = queue.remove(0);
        let t0 = std::time::Instant::now();
        let art = sc_raster::subtitle_art(&key.layer, &key.track, key.phrase, key.spoken, key.canvas);
        log::debug!("субтитры: фраза {} слово {} — {:.1} мс", key.phrase, key.spoken, t0.elapsed().as_secs_f64() * 1e3);
        if done.send((key, Arc::new(art))).is_err() {
            return;
        }
        ctx.request_repaint();
    }
}
