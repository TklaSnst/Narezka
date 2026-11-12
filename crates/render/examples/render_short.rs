//! Ручная проверка рендера без интерфейса:
//! `cargo run --release -p sc-render --example render_short -- <видео> <ширина> <высота> <начало> <конец> <выход.mp4> [кодировщик]`

use std::sync::atomic::AtomicBool;
use std::time::Instant;

use sc_core::{OutputSettings, Template};
use sc_render::{Encoder, RenderJob};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    if a.len() < 6 {
        eprintln!("использование: <видео> <ширина> <высота> <начало> <конец> <выход.mp4> [h264_vaapi|libx264]");
        std::process::exit(2);
    }
    let name = a.get(6).cloned().unwrap_or_else(|| "h264_vaapi".into());
    let vaapi_device = name.ends_with("_vaapi").then(|| "/dev/dri/renderD128".into());
    let dest: std::path::PathBuf = a[5].clone().into();
    let job = RenderJob {
        input: a[0].clone().into(),
        src_size: (a[1].parse().unwrap(), a[2].parse().unwrap()),
        has_audio: true,
        start: a[3].parse().unwrap(),
        end: a[4].parse().unwrap(),
        template: Template::minimal(),
        output: OutputSettings::default(),
        encoder: Encoder { name, vaapi_device },
        work_dir: std::env::temp_dir().join("sc-render-example"),
        dest,
    };
    let t = Instant::now();
    let mut shown = -1;
    job.run(&AtomicBool::new(false), |p| {
        let pct = (p * 100.0) as i32;
        if pct / 10 != shown / 10 {
            eprintln!("{pct}%");
            shown = pct;
        }
    })
    .unwrap_or_else(|e| panic!("{e}"));
    eprintln!("готово за {:.1} с: {}", t.elapsed().as_secs_f64(), job.dest.display());
}
