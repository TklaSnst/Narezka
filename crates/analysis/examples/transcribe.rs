//! Распознавание без GUI — для проверки и замеров.
//! `cargo run --release -p sc-analysis --example transcribe -- <видео> [термин…]`
//! Модели — из `~/.cache/shortscutter/models`, кэш — во временном каталоге (каждый запуск распознаёт заново).
//! `SC_CANCEL_AFTER=<с>` — отменить через столько секунд (проверка отмены).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use sc_analysis::{SILERO_VAD, Stage, TranscribeJob, WHISPER_TURBO};

fn main() {
    sc_analysis::prepare_ggml_env();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let mut args = std::env::args_os().skip(1);
    let input = std::path::PathBuf::from(args.next().expect("путь к видео"));
    let terms = args.map(|a| a.to_string_lossy().into_owned()).collect();
    let models = sc_core::Paths::from_env().expect("домашний каталог").models_dir();
    let cache = tempfile::tempdir().unwrap();
    let duration = std::process::Command::new("ffprobe")
        .args(["-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0"])
        .arg(&input)
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok()?.trim().parse().ok())
        .unwrap_or(0.0);
    let job = TranscribeJob {
        input,
        hash: "example".into(),
        duration,
        whisper_model: WHISPER_TURBO.path(&models),
        vad_model: SILERO_VAD.path(&models),
        language: "ru".into(),
        terms,
        cache_dir: cache.path().to_owned(),
    };
    let cancel = Arc::new(AtomicBool::new(false));
    if let Some(secs) = std::env::var("SC_CANCEL_AFTER").ok().and_then(|s| s.parse::<f64>().ok()) {
        let cancel = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs_f64(secs));
            eprintln!("отмена");
            cancel.store(true, Ordering::Relaxed);
        });
    }
    let started = std::time::Instant::now();
    let mut last = (Stage::Audio, -1.0);
    let result = job.run(&cancel, |stage, f| {
            if stage != last.0 || f - last.1 >= 0.1 || f >= 1.0 {
                eprintln!("{stage:?} {:.0}%", f * 100.0);
                last = (stage, f);
            }
        });
    eprintln!("за {:.1} с", started.elapsed().as_secs_f32());
    let t = result.expect("распознавание");
    for s in &t.segments {
        println!("[{:8.2} → {:8.2}] {}", s.start, s.end, s.text);
    }
    std::fs::write("transcript.json", serde_json::to_vec_pretty(&t).unwrap()).unwrap();
    eprintln!("слов {}, сегментов {}, участков речи {} → transcript.json", t.words.len(), t.segments.len(), t.speech.len());
}
