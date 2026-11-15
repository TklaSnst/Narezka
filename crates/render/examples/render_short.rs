//! Ручная проверка рендера без интерфейса:
//! `cargo run --release -p sc-render --example render_short -- <видео> <ширина> <высота> <начало> <конец> <выход.mp4> [кодировщик]`
//! Шаблон — из файла `SC_TEMPLATE` (TOML), иначе минимальный; `{title}` — из `SC_TITLE`;
//! слова субтитров — из транскрипта `SC_TRANSCRIPT` (`<хэш>.transcript.json` из кэша приложения), без ключевых.

use std::sync::atomic::AtomicBool;
use std::time::Instant;

use sc_core::{OutputSettings, SubWord, Template, TextVars};
use sc_render::{Encoder, RenderJob};

#[derive(serde::Deserialize)]
struct Transcript {
    words: Vec<Word>,
}

#[derive(serde::Deserialize)]
struct Word {
    text: String,
    start: f64,
    end: f64,
}

/// Слова транскрипта, чья середина внутри [start, end) — как у шортса в приложении.
fn words(start: f64, end: f64) -> Vec<SubWord> {
    let Some(path) = std::env::var_os("SC_TRANSCRIPT") else { return vec![] };
    let t: Transcript = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    (t.words.into_iter())
        .filter(|w| (start..end).contains(&((w.start + w.end) / 2.0)))
        .map(|w| SubWord { text: w.text, start: w.start, end: w.end, keyword: false })
        .collect()
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    if a.len() < 6 {
        eprintln!("использование: <видео> <ширина> <высота> <начало> <конец> <выход.mp4> [h264_vaapi|libx264]");
        std::process::exit(2);
    }
    let name = a.get(6).cloned().unwrap_or_else(|| "h264_vaapi".into());
    let vaapi_device = name.ends_with("_vaapi").then(|| "/dev/dri/renderD128".into());
    let dest: std::path::PathBuf = a[5].clone().into();
    let (start, end): (f64, f64) = (a[3].parse().unwrap(), a[4].parse().unwrap());
    let job = RenderJob {
        input: a[0].clone().into(),
        src_size: (a[1].parse().unwrap(), a[2].parse().unwrap()),
        src_fps: 60.0,
        has_audio: true,
        start,
        end,
        subtitles: words(start, end),
        insert: None,
        cover: None,
        template: std::env::var_os("SC_TEMPLATE")
            .map(|p| Template::from_toml(&std::fs::read_to_string(p).unwrap()).unwrap())
            .unwrap_or_else(Template::minimal)
            .with_vars(&TextVars { part: 1, title: std::env::var("SC_TITLE").unwrap_or_default() }),
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
