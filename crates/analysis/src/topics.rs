//! Кандидаты границ тем по эмбеддингам предложений — TextTiling (Hearst, 1997) на векторах вместо слов
//! (ARCHITECTURE §4, п. 5). Страховка для LLM-нарезки: граница, найденная обоими способами, — надёжная.
//!
//! Для каждого стыка предложений сравниваем средний вектор окна `window` секунд до стыка и после;
//! «глубина» провала сходства относительно соседних пиков — сила границы. Берём стыки глубже
//! `mean + k·std` и прореживаем: ближе `min_gap` секунд к более сильной границе — отбрасываем.
//! Параметры подобраны в спайке `spikes/06-embed` на сценариях с разметкой разделов.

use std::sync::atomic::AtomicBool;

use serde::{Deserialize, Serialize};

use crate::sentences::Sentence;

#[derive(Debug, Clone, Copy)]
pub struct TilingOptions {
    /// Окно по обе стороны стыка, с.
    pub window: f64,
    /// Порог глубины: `mean + k·std` (отрицательный — больше кандидатов).
    pub k: f32,
    /// Минимальное расстояние между границами, с.
    pub min_gap: f64,
}

impl Default for TilingOptions {
    fn default() -> Self {
        Self { window: 45.0, k: -0.5, min_gap: 20.0 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TopicBoundary {
    /// Тема начинается с этого предложения.
    pub sentence: usize,
    /// Середина паузы перед ним, с.
    pub time: f64,
    /// Глубина провала сходства (0…2); больше — увереннее.
    pub depth: f32,
}

/// Границы тем по порядку. `emb[i]` — L2-нормированный вектор `sentences[i]`.
pub fn topic_boundaries(sentences: &[Sentence], emb: &[Vec<f32>], opts: &TilingOptions) -> Vec<TopicBoundary> {
    assert_eq!(sentences.len(), emb.len());
    let n = sentences.len();
    if n < 3 {
        return vec![];
    }
    // sim[i] — сходство окон по обе стороны стыка перед предложением i (i ≥ 1).
    let mut sim = vec![0.0f32; n];
    for i in 1..n {
        let t = sentences[i].start;
        let left = (0..i).rev().take_while(|&j| j + 1 == i || sentences[j].end >= t - opts.window);
        let right = (i..n).take_while(|&j| j == i || sentences[j].start <= t + opts.window);
        sim[i] = dot(&mean(left.map(|j| &emb[j])), &mean(right.map(|j| &emb[j])));
    }
    let depth: Vec<f32> = (0..n).map(|i| if i == 0 { 0.0 } else { depth_at(&sim, i) }).collect();

    let d = &depth[1..];
    let mu = d.iter().sum::<f32>() / d.len() as f32;
    let sd = (d.iter().map(|x| (x - mu).powi(2)).sum::<f32>() / d.len() as f32).sqrt();
    let cut = mu + opts.k * sd;

    let mut cand: Vec<usize> = (1..n).filter(|&i| depth[i] > cut && depth[i] > 0.0).collect();
    cand.sort_by(|&a, &b| depth[b].total_cmp(&depth[a]));
    let mut picked: Vec<usize> = Vec::new();
    for i in cand {
        if picked.iter().all(|&j| (sentences[i].start - sentences[j].start).abs() >= opts.min_gap) {
            picked.push(i);
        }
    }
    picked.sort_unstable();
    picked
        .into_iter()
        .map(|i| TopicBoundary { sentence: i, time: (sentences[i - 1].end + sentences[i].start) / 2.0, depth: depth[i] })
        .collect()
}

/// Глубина провала: подъём до ближайшего пика слева и справа.
fn depth_at(sim: &[f32], i: usize) -> f32 {
    let (mut l, mut r) = (i, i);
    while l > 1 && sim[l - 1] >= sim[l] {
        l -= 1;
    }
    while r + 1 < sim.len() && sim[r + 1] >= sim[r] {
        r += 1;
    }
    (sim[l] - sim[i]) + (sim[r] - sim[i])
}

fn mean<'a>(vs: impl Iterator<Item = &'a Vec<f32>>) -> Vec<f32> {
    let mut acc: Vec<f32> = Vec::new();
    for v in vs {
        if acc.is_empty() {
            acc = vec![0.0; v.len()];
        }
        acc.iter_mut().zip(v).for_each(|(a, x)| *a += x);
    }
    let n = acc.iter().map(|x| x * x).sum::<f32>().sqrt();
    if n > 0.0 {
        acc.iter_mut().for_each(|x| *x /= n);
    }
    acc
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Эмбеддинги предложений через llama-server с моделью эмбеддингов.
pub fn embed_sentences(
    server: &sc_llama::Server,
    sentences: &[Sentence],
    cancel: &AtomicBool,
) -> Result<Vec<Vec<f32>>, sc_llama::LlamaError> {
    let texts: Vec<String> = sentences.iter().map(|s| s.text.clone()).collect();
    server.embeddings(&texts, cancel)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sentence(i: usize) -> Sentence {
        let start = i as f64 * 5.0;
        Sentence { words: i..i + 1, start, end: start + 4.0, text: String::new(), pause_after: 1.0, repeat: false }
    }

    /// Три «темы» по 12 предложений (по минуте), векторы — шум вокруг своей оси.
    #[test]
    fn finds_synthetic_topics() {
        let n = 36;
        let s: Vec<_> = (0..n).map(sentence).collect();
        let emb: Vec<Vec<f32>> = (0..n)
            .map(|i| {
                let mut v = vec![0.0f32; 8];
                v[i / 12] = 1.0;
                v[3 + i % 5] = 0.4; // общий для всех тем «шум»
                mean(std::iter::once(&v))
            })
            .collect();
        let b = topic_boundaries(&s, &emb, &TilingOptions::default());
        let at: Vec<usize> = b.iter().map(|b| b.sentence).collect();
        assert!(at.contains(&12) && at.contains(&24), "{b:?}");
        assert!((b.iter().find(|b| b.sentence == 12).unwrap().time - 59.5).abs() < 1e-9);
        // Остальные кандидаты (если есть) — слабее настоящих.
        let weakest_true = b.iter().filter(|b| [12, 24].contains(&b.sentence)).map(|b| b.depth).fold(f32::MAX, f32::min);
        assert!(b.iter().filter(|b| ![12, 24].contains(&b.sentence)).all(|b| b.depth < weakest_true));
    }

    #[test]
    fn min_gap_keeps_strongest() {
        let s: Vec<_> = (0..20).map(sentence).collect();
        // Две смены темы подряд (предложения 10 и 11) — ближе min_gap, остаётся одна.
        let emb: Vec<Vec<f32>> = (0..20)
            .map(|i| {
                let axis = if i < 10 { 0 } else if i == 10 { 1 } else { 2 };
                let mut v = vec![0.0f32; 3];
                v[axis] = 1.0;
                v
            })
            .collect();
        let b = topic_boundaries(&s, &emb, &TilingOptions::default());
        assert_eq!(b.iter().filter(|b| (9..=12).contains(&b.sentence)).count(), 1, "{b:?}");
    }

    #[test]
    fn too_short_input() {
        let s: Vec<_> = (0..2).map(sentence).collect();
        assert!(topic_boundaries(&s, &[vec![1.0], vec![1.0]], &TilingOptions::default()).is_empty());
    }
}

/// Сценарии роликов с разметкой разделов (спайк 0.3) — полнота границ на реальной модели.
/// `cargo test -p sc-analysis -- --ignored topics` (нужна модель эмбеддингов в `~/.cache/shortscutter/models`).
#[cfg(test)]
mod real {
    use super::*;
    use crate::models::QWEN3_EMBED;

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
        first: usize,
    }

    fn recall(json: &str, server: &sc_llama::Server) -> (f64, usize, usize) {
        let script: Script = serde_json::from_str(json).unwrap();
        let s: Vec<Sentence> = script
            .sentences
            .into_iter()
            .enumerate()
            .map(|(i, x)| Sentence { words: i..i + 1, start: x.start, end: x.end, text: x.text, pause_after: 0.0, repeat: false })
            .collect();
        let emb = embed_sentences(server, &s, &AtomicBool::new(false)).unwrap();
        let found = topic_boundaries(&s, &emb, &TilingOptions::default());
        let truth: Vec<f64> = script.sections.iter().filter(|x| x.first > 0).map(|x| s[x.first].start).collect();
        let hit = truth.iter().filter(|&&t| found.iter().any(|b| (s[b.sentence].start - t).abs() <= 10.0)).count();
        (hit as f64 / truth.len() as f64, found.len(), truth.len())
    }

    #[test]
    #[ignore]
    fn topics_on_scripts() {
        crate::prepare_ggml_env();
        let models = sc_core::Paths::from_env().unwrap().models_dir();
        let server =
            sc_llama::Server::start(&sc_llama::ServerOptions::embedding(&QWEN3_EMBED.path(&models)), &AtomicBool::new(false))
                .unwrap();
        // Спайк 2.2: 0.65 и 0.85 при точности ~0.5 — это страховка, полнота важнее.
        for (name, json, min) in [
            ("net", include_str!("../tests/data/script-net.json"), 0.6),
            ("scam", include_str!("../tests/data/script-scam.json"), 0.75),
        ] {
            let (r, found, truth) = recall(json, &server);
            eprintln!("{name}: найдено {found}, разделов {truth}, полнота {r:.2}");
            assert!(r >= min, "{name}: полнота {r:.2} < {min}");
        }
    }
}
