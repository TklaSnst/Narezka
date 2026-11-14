//! Растры шаблона для превью — в отдельном потоке, чтобы перетаскивание слоя мышью не ждало теней и текста
//! (весь `text.toml` — ~140 мс). Берётся последний запрошенный шаблон, промежуточные выбрасываются;
//! неизменённые слои не растрируются заново.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use eframe::egui;
use sc_core::{Background, Canvas, Layer, Template};
use sc_raster::{Bitmap, LayerArt};

/// Готовые растры шаблона: ключи слоёв ([`sc_raster::raster_key`]) и растры к ним.
#[derive(Clone)]
pub struct Rastered {
    pub canvas: Canvas,
    pub background: (Background, Option<Arc<Bitmap>>),
    pub layers: Vec<(Layer, Arc<LayerArt>)>,
    /// [`sc_raster::fonts::generation`], при котором нарисованы растры.
    pub fonts: u64,
}

impl Rastered {
    pub fn layer(&self, canvas: Canvas, key: &Layer) -> Option<&Arc<LayerArt>> {
        (self.canvas == canvas).then(|| self.layers.iter().find(|(k, _)| k == key).map(|(_, a)| a)).flatten()
    }

    pub fn background(&self, canvas: Canvas, bg: &Background) -> Option<Option<&Bitmap>> {
        (self.canvas == canvas && &self.background.0 == bg).then_some(self.background.1.as_deref())
    }
}

pub struct RasterWorker {
    tx: Sender<Template>,
    rx: Receiver<Rastered>,
}

impl RasterWorker {
    pub fn new(ctx: egui::Context) -> Self {
        let (tx, jobs) = channel::<Template>();
        let (done, rx) = channel();
        std::thread::Builder::new()
            .name("preview-raster".into())
            .spawn(move || run(jobs, done, ctx))
            .expect("поток растров превью");
        Self { tx, rx }
    }

    pub fn request(&self, template: Template) {
        let _ = self.tx.send(template);
    }

    pub fn poll(&self) -> Option<Rastered> {
        self.rx.try_iter().last()
    }
}

fn run(jobs: Receiver<Template>, done: Sender<Rastered>, ctx: egui::Context) {
    let mut cache: Option<Rastered> = None;
    while let Ok(mut t) = jobs.recv() {
        // Только последний запрос: пока считался прошлый, шаблон мог поменяться много раз.
        while let Ok(newer) = jobs.try_recv() {
            t = newer;
        }
        let canvas = t.canvas;
        let fonts = sc_raster::fonts::generation();
        if cache.as_ref().is_some_and(|c| c.fonts != fonts) {
            cache = None;
        }
        // Тот же растр, что в кэше, — тот же Arc (без копии пикселей).
        let background = match cache.as_ref().filter(|c| c.canvas == canvas && c.background.0 == t.background) {
            Some(c) => c.background.1.clone(),
            None => sc_raster::background(&t.background, canvas)
                .unwrap_or_else(|e| {
                    log::error!("фон шаблона: {e}");
                    None
                })
                .map(Arc::new),
        };
        let layers = t
            .layers
            .iter()
            .map(|l| {
                let key = sc_raster::raster_key(l);
                let art = match cache.as_ref().and_then(|c| c.layer(canvas, &key)) {
                    Some(a) => a.clone(),
                    None => Arc::new(sc_raster::layer(l, canvas).unwrap_or_else(|e| {
                        log::error!("слой шаблона: {e}");
                        LayerArt::default()
                    })),
                };
                (key, art)
            })
            .collect();
        let r = Rastered { canvas, background: (t.background, background), layers, fonts };
        cache = Some(r.clone());
        if done.send(r).is_err() {
            return;
        }
        ctx.request_repaint();
    }
}
