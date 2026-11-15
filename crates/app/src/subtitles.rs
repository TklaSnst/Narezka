//! Слова субтитров для превью: из транскрипции выбранного шортса (с ключевыми словами нарезки и ручными
//! пометками) или образец, чтобы на «Оформлении» было что расставлять, пока речи нет.

use std::sync::Arc;

use sc_analysis::Transcript;
use sc_core::subtitles::keyword_hits;
use sc_core::{Short, ShortId, SubWord, SubtitleEdits};

use crate::App;
use crate::player::SubtitleInput;

/// Слово образца, которое помечено ключевым, и сколько слов образца «сказано» (последнее — текущее).
const SAMPLE_KEYWORD: usize = 1;
const SAMPLE_SPOKEN: usize = 3;
const SAMPLE_STEP: f64 = 0.4;

/// Что лежит в кэше слов: от чего они посчитаны.
#[derive(PartialEq)]
struct Key {
    short: ShortId,
    range: (f64, f64),
    keywords: Vec<String>,
    edits: SubtitleEdits,
    transcript: usize,
}

#[derive(Default)]
pub struct SubtitleCache {
    words: Option<(Key, Arc<Vec<SubWord>>)>,
    sample: Option<(String, Arc<Vec<SubWord>>)>,
}

/// Индекс первого слова, чья середина не раньше `t` (так же слова относятся к шортсу и в тексте «Нарезки»).
pub(crate) fn first_word_from(t: &Transcript, time: f64) -> usize {
    t.words.partition_point(|w| (w.start + w.end) / 2.0 < time)
}

/// Слова шортса — середина внутри [start, end); ключевые — по нарезке LLM, с ручными пометками поверх.
pub(crate) fn short_words(t: &Transcript, short: &Short) -> Vec<SubWord> {
    let first = first_word_from(t, short.start);
    let words: Vec<_> = t.words[first..].iter().take_while(|w| (w.start + w.end) / 2.0 < short.end).collect();
    let auto = keyword_hits(words.iter().map(|w| w.text.as_str()), &short.keywords);
    words
        .iter()
        .zip(auto)
        .map(|(w, auto)| SubWord { text: w.text.clone(), start: w.start, end: w.end, keyword: short.subtitles.keyword(w.start, auto) })
        .collect()
}

impl App {
    /// Слова субтитров для превью: шортса, выбранного в «Нарезке», если у ролика есть речь;
    /// на «Оформлении» без речи — образец.
    pub(crate) fn subtitle_input(&mut self, design: bool) -> Option<SubtitleInput> {
        let short = self.cut.selected.and_then(|id| self.batch.short(id)).cloned();
        let transcript = short.as_ref().and_then(|s| self.transcript_of(s.source_id));
        if let (Some(s), Some(t)) = (&short, &transcript) {
            let key = Key {
                short: s.id,
                range: (s.start, s.end),
                keywords: s.keywords.clone(),
                edits: s.subtitles.clone(),
                transcript: Arc::as_ptr(t) as usize,
            };
            let cache = &mut self.subtitle_cache;
            if cache.words.as_ref().is_none_or(|(k, _)| *k != key) {
                cache.words = Some((key, Arc::new(short_words(t, s))));
            }
            let words = cache.words.as_ref().unwrap().1.clone();
            if !words.is_empty() || !design {
                return Some(SubtitleInput { words, fixed_time: None });
            }
        }
        if !design {
            return None;
        }
        let text = self.i18n.tr("subtitles-sample");
        let cache = &mut self.subtitle_cache;
        if cache.sample.as_ref().is_none_or(|(t, _)| *t != text) {
            let words = text
                .split_whitespace()
                .enumerate()
                .map(|(i, w)| SubWord {
                    text: w.to_string(),
                    start: i as f64 * SAMPLE_STEP,
                    end: (i as f64 + 0.8) * SAMPLE_STEP,
                    keyword: i == SAMPLE_KEYWORD,
                })
                .collect();
            cache.sample = Some((text, Arc::new(words)));
        }
        let words = cache.sample.as_ref().unwrap().1.clone();
        let spoken = SAMPLE_SPOKEN.min(words.len()).max(1);
        Some(SubtitleInput { words, fixed_time: Some((spoken - 1) as f64 * SAMPLE_STEP + 0.01) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sc_analysis::Word;
    use sc_core::SourceId;

    fn transcript(words: &[(&str, f64)]) -> Transcript {
        Transcript {
            version: sc_analysis::transcript::FORMAT_VERSION,
            model: String::new(),
            language: "ru".into(),
            prompt: String::new(),
            words: words.iter().map(|&(t, s)| Word { text: t.into(), start: s, end: s + 0.4, dtw: None, p: 1.0 }).collect(),
            segments: vec![],
            speech: vec![],
        }
    }

    #[test]
    fn words_of_short_with_keywords_and_manual_marks() {
        let t = transcript(&[("до", 0.0), ("Настроим", 1.0), ("сервер,", 1.5), ("быстро.", 2.0), ("после", 3.0)]);
        let mut s = Short::manual(SourceId::new(), 0.9, 2.5);
        s.keywords = vec!["серверы".into()];
        let w = short_words(&t, &s);
        assert_eq!(w.iter().map(|w| w.text.as_str()).collect::<Vec<_>>(), ["Настроим", "сервер,", "быстро."]);
        assert_eq!(w.iter().map(|w| w.keyword).collect::<Vec<_>>(), [false, true, false]);
        // Ручная пометка: снять автоматическую и добавить свою.
        s.subtitles.toggle(1.5);
        s.subtitles.toggle(2.0);
        let w = short_words(&t, &s);
        assert_eq!(w.iter().map(|w| w.keyword).collect::<Vec<_>>(), [false, false, true]);
    }
}
