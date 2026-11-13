//! Структура текста без GUI: предложения, паузы и границы тем по сохранённому транскрипту.
//! `cargo run --release -p sc-analysis --example structure -- transcript.json`
//! (транскрипт пишет пример `transcribe`; модель эмбеддингов — в `~/.cache/shortscutter/models`).

use std::sync::atomic::AtomicBool;

use sc_analysis::{QWEN3_EMBED, SplitOptions, TilingOptions, Transcript};

fn main() {
    sc_analysis::prepare_ggml_env();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let path = std::env::args_os().nth(1).expect("путь к transcript.json");
    let t: Transcript = serde_json::from_slice(&std::fs::read(path).expect("чтение")).expect("формат транскрипта");
    let sentences = sc_analysis::sentences(&t, &SplitOptions::default());
    let pauses = sc_analysis::pauses(&t, 0.3);
    eprintln!(
        "слов {}, предложений {} (повторов {}), пауз ≥0.3 с {} (из них с тишиной по VAD {})",
        t.words.len(),
        sentences.len(),
        sentences.iter().filter(|s| s.repeat).count(),
        pauses.len(),
        pauses.iter().filter(|p| p.silence.is_some()).count(),
    );

    let models = sc_core::Paths::from_env().expect("домашний каталог").models_dir();
    let cancel = AtomicBool::new(false);
    let started = std::time::Instant::now();
    let server = sc_llama::Server::start(&sc_llama::ServerOptions::embedding(&QWEN3_EMBED.path(&models)), &cancel)
        .expect("llama-server");
    let emb = sc_analysis::embed_sentences(&server, &sentences, &cancel).expect("эмбеддинги");
    drop(server);
    eprintln!("эмбеддинги за {:.1} с (с запуском сервера)", started.elapsed().as_secs_f32());
    let bounds = sc_analysis::topic_boundaries(&sentences, &emb, &TilingOptions::default());

    let mut next = bounds.iter().peekable();
    for (i, s) in sentences.iter().enumerate() {
        if let Some(b) = next.next_if(|b| b.sentence == i) {
            println!("──── тема? {:7.2} с, глубина {:.2}", b.time, b.depth);
        }
        let mark = if s.repeat { " [повтор]" } else { "" };
        println!("{i:4} [{:8.2} → {:8.2}] {}{mark}", s.start, s.end, s.text);
    }
    eprintln!("границ тем: {}", bounds.len());
}
