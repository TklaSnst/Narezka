//! LLM-нарезка без GUI: `cargo run --release -p sc-analysis --example cut -- <файл.json> [тема видео]`.
//!
//! Файл — транскрипт (пишет пример `transcribe`) или сценарий с разметкой разделов (`tests/data/script-*.json`,
//! предложение = слово); для сценария печатается полнота границ (±10 с), как в спайке 0.3.
//! Модели — в `~/.cache/shortscutter/models`, свои промпты — `~/.config/shortscutter/prompts/`.
//! Кэш не используется: каждый запуск — заново. Время каждого запроса к LLM: `RUST_LOG=info,sc_llama=debug`.

use std::sync::atomic::AtomicBool;

use sc_analysis::{BlockKind, CutJob, CutOptions, Prompts, QWEN35_9B, QWEN3_EMBED, Span, Transcript, Word};
use serde::Deserialize;

#[derive(Deserialize)]
struct Script {
    sentences: Vec<ScriptSentence>,
    sections: Vec<Section>,
}
#[derive(Deserialize)]
struct ScriptSentence {
    start: f64,
    end: f64,
    text: String,
}
#[derive(Deserialize)]
struct Section {
    start: f64,
}

fn mmss(t: f64) -> String {
    format!("{:02}:{:02}", t as u64 / 60, t as u64 % 60)
}

fn main() {
    sc_analysis::prepare_ggml_env();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let path = std::env::args_os().nth(1).expect("путь к transcript.json или сценарию");
    let title = std::env::args().nth(2).unwrap_or_default();
    let bytes = std::fs::read(&path).expect("чтение");

    // Сценарий: каждое предложение — одно «слово», речь — сплошная.
    let (t, truth) = match serde_json::from_slice::<Script>(&bytes) {
        Ok(s) => {
            let words: Vec<Word> =
                s.sentences.iter().map(|x| Word { text: x.text.clone(), start: x.start, end: x.end, dtw: None, p: 1.0 }).collect();
            let speech = words.iter().map(|w| Span { start: w.start, end: w.end }).collect();
            let t = Transcript {
                version: sc_analysis::transcript::FORMAT_VERSION,
                model: String::new(),
                language: "ru".into(),
                prompt: String::new(),
                words,
                segments: vec![],
                speech,
            };
            (t, Some(s.sections.iter().map(|x| x.start).filter(|&x| x > 0.0).collect::<Vec<_>>()))
        }
        Err(_) => (serde_json::from_slice(&bytes).expect("формат транскрипта"), None),
    };

    let paths = sc_core::Paths::from_env().expect("домашний каталог");
    let models = paths.models_dir();
    let tmp = tempfile::tempdir().expect("временный каталог");
    let job = CutJob {
        hash: "example".into(),
        cache_dir: tmp.path().into(),
        embed_model: QWEN3_EMBED.path(&models),
        llm_model: QWEN35_9B.path(&models),
        prompts: Prompts::load(&paths.prompts_dir()).expect("промпты"),
        opts: CutOptions {
            title,
            // Для экспериментов: SC_MIN_BLOCK=0 — без склейки коротких блоков.
            min_block: std::env::var("SC_MIN_BLOCK").ok().and_then(|v| v.parse().ok()).unwrap_or(CutOptions::default().min_block),
            ..CutOptions::default()
        },
    };
    let mut last = None;
    let c = job
        .run(&t, &AtomicBool::new(false), |stage, f| {
            if last != Some(stage) {
                eprintln!("— {stage:?}");
                last = Some(stage);
            }
            let _ = f;
        })
        .expect("нарезка");

    println!("| # | время | дл. | вид | граница | тема | оценка | шортс | заголовок / причина |\n|---|---|---|---|---|---|---|---|---|");
    for (i, b) in c.blocks.iter().enumerate() {
        let cand = c.candidates.iter().find(|x| x.block == i);
        let short = cand.map_or(String::new(), |x| format!("{}–{} ({:.0} с)", mmss(x.start), mmss(x.end), x.duration()));
        let score = cand.map_or(String::new(), |x| format!("{}{}", x.score, if x.flags.is_empty() { "" } else { " ⚠" }));
        let flags = cand.map_or(String::new(), |x| format!("{:?}", x.flags));
        let text = cand.map_or(String::new(), |x| format!("**{}** — {} {flags}", x.title, x.reason));
        println!(
            "| {i} | {} | {:.0} | {:?} | {:?} | {} | {score} | {short} | {text} |",
            mmss(b.start),
            b.end - b.start,
            b.kind,
            b.found,
            b.topic,
        );
    }
    let durs: Vec<f64> = c.candidates.iter().map(|x| x.duration()).collect();
    eprintln!(
        "блоков {}, кандидатов {}, ≤62 с: {}, средняя оценка {:.1}, реклама: {}",
        c.blocks.len(),
        c.candidates.len(),
        durs.iter().filter(|&&d| d <= 62.0).count(),
        c.candidates.iter().map(|x| x.score as f64).sum::<f64>() / c.candidates.len().max(1) as f64,
        c.blocks.iter().filter(|b| b.kind == BlockKind::Ad).map(|b| format!("{}–{}", mmss(b.start), mmss(b.end))).collect::<Vec<_>>().join(", "),
    );
    if let Some(truth) = truth {
        let starts: Vec<f64> = c.blocks.iter().map(|b| b.start).filter(|&x| x > 0.0).collect();
        let hit = truth.iter().filter(|&&g| starts.iter().any(|&p| (g - p).abs() <= 10.0)).count();
        let missed: Vec<String> = truth.iter().filter(|&&g| !starts.iter().any(|&p| (g - p).abs() <= 10.0)).map(|&g| mmss(g)).collect();
        eprintln!("полнота границ {:.2} ({hit} из {}), пропущены: {}", hit as f64 / truth.len() as f64, truth.len(), missed.join(" "));
    }
}
