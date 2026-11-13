//! Предложения и паузы по словам транскрипта (ARCHITECTURE §4, п. 3).
//!
//! Whisper с VAD ставит пунктуацию, поэтому предложение кончается на `.!?…`; длинная пауза без точки — тоже граница
//! (живая речь), а слишком длинное «предложение» делится по самой длинной паузе или запятой.
//! Паузы — промежутки между словами; VAD уточняет, где внутри них действительно тихо (`silencedetect` не нужен:
//! под голосом бывает музыка, а VAD её отличает).

use std::ops::Range;

use serde::{Deserialize, Serialize};

use crate::transcript::{Span, Transcript, Word};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sentence {
    /// Слова — индексы в [`Transcript::words`].
    pub words: Range<usize>,
    pub start: f64,
    pub end: f64,
    pub text: String,
    /// Тишина до следующего предложения, с; у последнего — 0.
    pub pause_after: f64,
    /// Тот же текст, что у предыдущего предложения: петля whisper («Не трогайте меня…» ×10).
    pub repeat: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct SplitOptions {
    /// Пауза между словами, после которой начинается новое предложение даже без точки, с.
    pub pause_break: f64,
    /// Длиннее — делим по самой длинной паузе или запятой, с.
    pub max_len: f64,
    /// Части при делении длинного предложения не короче, с.
    pub min_part: f64,
}

impl Default for SplitOptions {
    fn default() -> Self {
        Self { pause_break: 1.2, max_len: 30.0, min_part: 3.0 }
    }
}

/// Предложения транскрипта по порядку; каждое слово попадает ровно в одно предложение.
pub fn sentences(t: &Transcript, opts: &SplitOptions) -> Vec<Sentence> {
    let w = &t.words;
    let mut ranges = Vec::new();
    let mut first = 0;
    for i in 0..w.len() {
        let last = i + 1 == w.len();
        if last || ends_sentence(&w[i].text) || w[i + 1].start - w[i].end >= opts.pause_break {
            split_long(w, first..i + 1, opts, &mut ranges);
            first = i + 1;
        }
    }

    let mut out: Vec<Sentence> = Vec::with_capacity(ranges.len());
    for r in ranges {
        let text = w[r.clone()].iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ");
        let repeat = out.last().is_some_and(|p| same_text(&p.text, &text));
        out.push(Sentence { start: w[r.start].start, end: w[r.end - 1].end, words: r, text, pause_after: 0.0, repeat });
    }
    for i in 1..out.len() {
        out[i - 1].pause_after = (out[i].start - out[i - 1].end).max(0.0);
    }
    out
}

/// Делит слишком длинный отрезок слов в лучшей точке (пауза + бонус за запятую), рекурсивно.
fn split_long(w: &[Word], r: Range<usize>, opts: &SplitOptions, out: &mut Vec<Range<usize>>) {
    let (start, end) = (w[r.start].start, w[r.end - 1].end);
    if end - start <= opts.max_len {
        out.push(r);
        return;
    }
    let best = (r.start..r.end - 1)
        .filter(|&i| w[i].end - start >= opts.min_part && end - w[i + 1].start >= opts.min_part)
        .max_by(|&a, &b| cut_score(w, a).total_cmp(&cut_score(w, b)));
    match best {
        Some(i) => {
            split_long(w, r.start..i + 1, opts, out);
            split_long(w, i + 1..r.end, opts, out);
        }
        None => out.push(r),
    }
}

/// Насколько удачно резать после слова `i`: пауза после него, запятая/двоеточие/тире добавляют полсекунды.
fn cut_score(w: &[Word], i: usize) -> f64 {
    let gap = w[i + 1].start - w[i].end;
    let punct = trim_closing(&w[i].text).ends_with([',', ';', ':', '—', '–']) || w[i + 1].text.starts_with(['—', '–']);
    gap + if punct { 0.5 } else { 0.0 }
}

fn trim_closing(s: &str) -> &str {
    s.trim_end_matches(['»', '"', '\'', ')', ']', '”'])
}

fn ends_sentence(word: &str) -> bool {
    trim_closing(word).ends_with(['.', '!', '?', '…'])
}

/// Совпадение без учёта регистра и пунктуации.
fn same_text(a: &str, b: &str) -> bool {
    let norm = |s: &str| s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect::<String>();
    let a = norm(a);
    !a.is_empty() && a == norm(b)
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Pause {
    /// Промежуток между словами по их таймкодам.
    pub gap: Span,
    /// Самый длинный кусок промежутка вне речи по VAD; `None` — VAD слышит там звук (дыхание, музыка, неточный таймкод).
    pub silence: Option<Span>,
}

impl Pause {
    /// Куда ставить разрез: середина тишины по VAD, иначе середина промежутка.
    pub fn cut(&self) -> f64 {
        let s = self.silence.unwrap_or(self.gap);
        (s.start + s.end) / 2.0
    }
}

/// Паузы между словами не короче `min` секунд, по порядку.
pub fn pauses(t: &Transcript, min: f64) -> Vec<Pause> {
    (0..t.words.len().saturating_sub(1))
        .filter(|&i| t.words[i + 1].start - t.words[i].end >= min)
        .filter_map(|i| pause_after_word(t, i))
        .collect()
}

/// Промежуток между словом `i` и следующим (может быть нулевым); `None` у последнего слова.
pub fn pause_after_word(t: &Transcript, i: usize) -> Option<Pause> {
    let next = t.words.get(i + 1)?;
    let gap = Span { start: t.words[i].end, end: next.start.max(t.words[i].end) };
    Some(Pause { gap, silence: longest_silence(gap, &t.speech) })
}

/// Самый длинный кусок `gap`, не покрытый участками речи (`speech` отсортированы).
fn longest_silence(gap: Span, speech: &[Span]) -> Option<Span> {
    let mut best: Option<Span> = None;
    let mut cur = gap.start;
    let mut take = |s: f64, e: f64| {
        if e > s && best.is_none_or(|b| e - s > b.end - b.start) {
            best = Some(Span { start: s, end: e });
        }
    };
    for sp in speech.iter().filter(|sp| sp.end > gap.start && sp.start < gap.end) {
        take(cur, sp.start.min(gap.end));
        cur = cur.max(sp.end);
    }
    take(cur, gap.end);
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcript::FORMAT_VERSION;

    fn word(text: &str, start: f64, end: f64) -> Word {
        Word { text: text.into(), start, end, dtw: None, p: 1.0 }
    }

    fn transcript(words: Vec<Word>, speech: Vec<Span>) -> Transcript {
        Transcript {
            version: FORMAT_VERSION,
            model: String::new(),
            language: "ru".into(),
            prompt: String::new(),
            words,
            segments: vec![],
            speech,
        }
    }

    #[test]
    fn splits_on_punctuation_and_long_pauses() {
        let t = transcript(
            vec![
                word("Привет.", 0.0, 0.5),
                word("Как", 0.6, 0.8),
                word("дела?»", 0.9, 1.2),
                word("Нормально", 1.3, 1.8),
                // пауза 2 с без точки — граница
                word("ну", 3.8, 4.0),
                word("ладно", 4.1, 4.5),
            ],
            vec![],
        );
        let s = sentences(&t, &SplitOptions::default());
        let texts: Vec<_> = s.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(texts, ["Привет.", "Как дела?»", "Нормально", "ну ладно"]);
        assert_eq!(s[1].words, 1..3);
        assert!((s[2].pause_after - 2.0).abs() < 1e-9);
        assert_eq!(s[3].pause_after, 0.0);
    }

    #[test]
    fn long_sentence_is_cut_at_comma_or_pause() {
        // 40 слов по 1 с без точек; после 15-го — запятая, после 25-го — пауза 0.8 с (меньше pause_break).
        let mut words = Vec::new();
        let mut t = 0.0;
        for i in 0..40 {
            let text = if i == 14 { "раз," } else { "слово" };
            words.push(word(text, t, t + 0.9));
            t += if i == 24 { 1.7 } else { 1.0 };
        }
        let s = sentences(&transcript(words, vec![]), &SplitOptions::default());
        assert!(s.iter().all(|s| s.end - s.start <= 30.0), "{s:?}");
        // Пауза 0.8 + 0.1 перевешивает запятую (0.1 + 0.5): первый разрез — после 25-го слова.
        assert_eq!(s[0].words, 0..25);
        assert_eq!(s.iter().map(|s| s.words.len()).sum::<usize>(), 40);
    }

    #[test]
    fn marks_whisper_loops() {
        let t = transcript(
            vec![word("Не", 0.0, 0.2), word("трогайте.", 0.3, 0.8), word("Не", 1.0, 1.2), word("трогайте!", 1.3, 1.8)],
            vec![],
        );
        let s = sentences(&t, &SplitOptions::default());
        assert_eq!((s[0].repeat, s[1].repeat), (false, true));
    }

    #[test]
    fn pauses_use_vad_silence() {
        let t = transcript(
            vec![word("а", 0.0, 1.0), word("б", 3.0, 4.0), word("в", 4.1, 5.0), word("г", 7.0, 8.0)],
            // Речь по VAD залезает в первую паузу до 1.5 с; вторая пауза целиком «звучит».
            vec![Span { start: 0.0, end: 1.5 }, Span { start: 2.8, end: 8.0 }],
        );
        let p = pauses(&t, 0.5);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].silence, Some(Span { start: 1.5, end: 2.8 }));
        assert!((p[0].cut() - 2.15).abs() < 1e-9);
        assert_eq!(p[1].silence, None);
        assert!((p[1].cut() - 6.0).abs() < 1e-9);
    }

    /// Сохранённый транскрипт 10-минутного Twitch-VOD (шаг 2.1).
    #[test]
    fn real_transcript() {
        let t: Transcript = serde_json::from_str(include_str!("../tests/data/twitch-10min.transcript.json")).unwrap();
        let opts = SplitOptions::default();
        let s = sentences(&t, &opts);
        // Каждое слово ровно один раз и по порядку.
        assert_eq!(s.first().unwrap().words.start, 0);
        assert_eq!(s.last().unwrap().words.end, t.words.len());
        assert!(s.windows(2).all(|p| p[0].words.end == p[1].words.start));
        for x in &s {
            assert!(x.end - x.start <= opts.max_len, "{x:?}");
            let w = &t.words[x.words.end - 1];
            let next_gap = t.words.get(x.words.end).map_or(f64::INFINITY, |n| n.start - w.end);
            assert!(ends_sentence(&w.text) || next_gap >= opts.pause_break || x.end - x.start >= opts.min_part, "{x:?}");
        }
        // Предложений больше, чем сегментов whisper быть не обязано, но и не на порядок меньше.
        assert!(s.len() * 2 > t.segments.len(), "{} предложений на {} сегментов", s.len(), t.segments.len());
        assert!(pauses(&t, 0.3).iter().all(|p| p.gap.end > p.gap.start));
    }
}
