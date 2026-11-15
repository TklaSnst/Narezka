//! Слова субтитров для превью: из транскрипции выбранного шортса (с ключевыми словами нарезки и ручными
//! пометками) или образец, чтобы на «Оформлении» было что расставлять, пока речи нет.

use std::sync::Arc;

use sc_analysis::Transcript;
use sc_core::subtitles::keyword_hits;
use std::ops::Range;

use sc_core::{EditedWord, Short, ShortId, SpeechWord, SubWord, SubtitleEdits, word_ids};

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

/// Слова речи шортса — середина внутри [start, end).
pub(crate) fn short_range(t: &Transcript, short: &Short) -> std::ops::Range<usize> {
    let first = first_word_from(t, short.start);
    let n = t.words[first..].iter().take_while(|w| (w.start + w.end) / 2.0 < short.end).count();
    first..first + n
}

/// [`word_ids`] слов речи `range`: счёт одинаковых начал — с первого такого слова во всей речи, а не в куске.
pub(crate) fn speech_ids(t: &Transcript, range: Range<usize>) -> Vec<i64> {
    let ms = |i: usize| (t.words[i].start * 1000.0).round() as i64;
    let mut from = range.start;
    while from > 0 && from < t.words.len() && ms(from - 1) == ms(from) {
        from -= 1;
    }
    let skip = range.start - from;
    word_ids(t.words[from..range.end].iter().map(|w| w.start)).split_off(skip)
}

/// Слова речи шортса для правок: текст, время и [`sc_core::word_id`].
pub(crate) fn speech_words<'a>(t: &'a Transcript, short: &Short) -> Vec<SpeechWord<'a>> {
    let r = short_range(t, short);
    let ids = speech_ids(t, r.clone());
    t.words[r].iter().zip(ids).map(|(w, id)| (w.text.as_str(), w.start, w.end, id)).collect()
}

/// Слова речи шортса с правками текста (`EditedWord::src` — номер в [`speech_words`]).
pub(crate) fn edited_words(t: &Transcript, short: &Short) -> Vec<EditedWord> {
    short.subtitles.apply(speech_words(t, short))
}

/// Слова субтитров шортса: речь с правками текста; ключевые — по нарезке LLM, с ручными пометками поверх.
pub(crate) fn short_words(t: &Transcript, short: &Short) -> Vec<SubWord> {
    let words = edited_words(t, short);
    let auto = keyword_hits(words.iter().map(|w| w.text.as_str()), &short.keywords);
    words
        .into_iter()
        .zip(auto)
        .map(|(w, auto)| SubWord { keyword: short.subtitles.keyword(w.key, auto), text: w.text, start: w.start, end: w.end })
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
        s.subtitles.toggle(sc_core::word_id(1.5, 0));
        s.subtitles.toggle(sc_core::word_id(2.0, 0));
        let w = short_words(&t, &s);
        assert_eq!(w.iter().map(|w| w.keyword).collect::<Vec<_>>(), [false, false, true]);
        // Правка текста: ключевые ищутся уже в исправленном.
        s.subtitles.set_text(sc_core::word_id(1.0, 0), Some("Настроим серверы".into()));
        let w = short_words(&t, &s);
        assert_eq!(w.iter().map(|w| (w.text.as_str(), w.keyword)).collect::<Vec<_>>(), [
            ("Настроим", false),
            ("серверы", true),
            ("сервер,", false),
            ("быстро.", true)
        ]);
    }

    #[test]
    fn ids_count_same_starts_across_short_edge() {
        let mut t = transcript(&[("а", 1.0), ("б", 1.0), ("в", 1.0), ("г", 2.0)]);
        t.words[0].end = 1.0;
        assert_eq!(speech_ids(&t, 1..4), sc_core::word_ids([1.0, 1.0, 1.0, 2.0])[1..].to_vec());
        assert_eq!(speech_ids(&t, 0..2), sc_core::word_ids([1.0, 1.0]));
    }
}
