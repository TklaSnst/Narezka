//! Субтитры шортса: слова с таймкодами → фразы по строкам; какое состояние фразы показывать в момент `t`;
//! ключевые слова (из нарезки LLM и отмеченные вручную). Одна раскладка для превью и рендера.

use std::ops::Range;

use serde::{Deserialize, Serialize};

/// Пауза перед словом, после которой начинается новая фраза, секунды.
pub const PAUSE_BREAK: f64 = 0.7;
/// Сколько фраза держится после последнего слова, если следующая не началась раньше, секунды.
pub const HOLD: f64 = 0.6;

/// Слово субтитров (время — секунды исходника).
#[derive(Debug, Clone, PartialEq)]
pub struct SubWord {
    pub text: String,
    pub start: f64,
    pub end: f64,
    pub keyword: bool,
}

/// Фраза — то, что видно на экране одновременно.
#[derive(Debug, Clone, PartialEq)]
pub struct Phrase {
    /// Слова фразы — индексы в [`SubtitleTrack::words`].
    pub words: Range<usize>,
    /// Строки — индексы в [`SubtitleTrack::words`], подряд и без пропусков.
    pub lines: Vec<Range<usize>>,
    /// Когда фраза на экране: [start, end).
    pub start: f64,
    pub end: f64,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SubtitleTrack {
    pub words: Vec<SubWord>,
    pub phrases: Vec<Phrase>,
}

/// Ручные правки субтитров шортса.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct SubtitleEdits {
    /// Слова (начало в мс исходника), у которых пометка «ключевое» обратна автоматической.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub toggled: Vec<i64>,
}

impl SubtitleEdits {
    pub fn is_empty(&self) -> bool {
        self.toggled.is_empty()
    }

    fn id(start: f64) -> i64 {
        (start * 1000.0).round() as i64
    }

    /// Ключевое ли слово с началом `start`, если автоматически оно `auto`.
    pub fn keyword(&self, start: f64, auto: bool) -> bool {
        auto ^ self.toggled.contains(&Self::id(start))
    }

    /// Переключить пометку «ключевое» у слова с началом `start`.
    pub fn toggle(&mut self, start: f64) {
        let id = Self::id(start);
        match self.toggled.iter().position(|&x| x == id) {
            Some(i) => {
                self.toggled.remove(i);
            }
            None => self.toggled.push(id),
        }
    }
}

/// Слово без пунктуации по краям, в нижнем регистре, «ё» → «е».
pub fn normalize(word: &str) -> String {
    word.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase().replace('ё', "е")
}

/// Совпадает ли слово речи с словом ключевой фразы: точно, а у слов от 4 букв — с другим окончанием
/// (общее начало — всё, кроме 1–2 последних букв ключа, слово длиннее ключа не больше чем на 2 буквы): «ошибка» ~ «ошибки», «сервер» ~ «серверов».
fn token_matches(word: &str, key: &str) -> bool {
    if word == key {
        return true;
    }
    let (w, k): (Vec<char>, Vec<char>) = (word.chars().collect(), key.chars().collect());
    if k.len() < 4 || w.len() > k.len() + 2 {
        return false;
    }
    let common = w.iter().zip(&k).take_while(|(a, b)| a == b).count();
    common >= 4.max(k.len() - 2)
}

/// Индексы слов, входящих в какую-нибудь из ключевых фраз (`keywords` из нарезки: слово или несколько).
pub fn keyword_hits<'a>(words: impl IntoIterator<Item = &'a str>, keywords: &[String]) -> Vec<bool> {
    let words: Vec<String> = words.into_iter().map(normalize).collect();
    let mut hit = vec![false; words.len()];
    for k in keywords {
        let tokens: Vec<String> = k.split_whitespace().map(normalize).filter(|t| !t.is_empty()).collect();
        if tokens.is_empty() || tokens.len() > words.len() {
            continue;
        }
        for i in 0..=words.len() - tokens.len() {
            if tokens.iter().enumerate().all(|(j, t)| token_matches(&words[i + j], t)) {
                hit[i..i + tokens.len()].iter_mut().for_each(|h| *h = true);
            }
        }
    }
    hit
}

/// Конец предложения: точка, ! ? … (кавычки и скобки после них не мешают).
fn sentence_end(text: &str) -> bool {
    let t = text.trim_end_matches(['"', '»', '\'', ')', ']', '”']);
    t.ends_with(['.', '!', '?', '…'])
}

/// Делит `n` на `parts` почти равных кусков (первые — на 1 длиннее).
fn even_split(range: Range<usize>, parts: usize) -> Vec<Range<usize>> {
    let n = range.len();
    let parts = parts.clamp(1, n.max(1));
    let (base, extra) = (n / parts, n % parts);
    let mut at = range.start;
    (0..parts)
        .map(|i| {
            let len = base + usize::from(i < extra);
            let r = at..at + len;
            at += len;
            r
        })
        .collect()
}

impl SubtitleTrack {
    /// Раскладка: предложения (по знакам конца и паузам ≥ [`PAUSE_BREAK`]) делятся на фразы по
    /// `lines` × `words_per_line` слов поровну — без «висящего» слова в последней фразе; фраза — на строки поровну.
    pub fn new(words: Vec<SubWord>, words_per_line: u32, lines: u32) -> Self {
        let per_line = words_per_line.max(1) as usize;
        let capacity = per_line * lines.max(1) as usize;
        let mut sentences = Vec::new();
        let mut from = 0;
        for i in 0..words.len() {
            let last = i + 1 == words.len();
            if last || sentence_end(&words[i].text) || words[i + 1].start - words[i].end >= PAUSE_BREAK {
                sentences.push(from..i + 1);
                from = i + 1;
            }
        }
        let mut phrases: Vec<Phrase> = Vec::new();
        for s in sentences {
            for p in even_split(s.clone(), s.len().div_ceil(capacity)) {
                let lines = even_split(p.clone(), p.len().div_ceil(per_line));
                phrases.push(Phrase { start: words[p.start].start, end: words[p.end - 1].end + HOLD, words: p, lines });
            }
        }
        // Следующая фраза сменяет предыдущую, не накладываясь на неё.
        for i in 1..phrases.len() {
            let next = phrases[i].start;
            let prev = &mut phrases[i - 1];
            prev.end = prev.end.min(next).max(prev.start);
        }
        Self { words, phrases }
    }

    /// Что видно в момент `t`: (фраза, сколько её слов уже начато — от 1 до числа слов).
    pub fn at(&self, t: f64) -> Option<(usize, usize)> {
        let i = self.phrases.partition_point(|p| p.start <= t).checked_sub(1)?;
        let p = &self.phrases[i];
        if t >= p.end {
            return None;
        }
        let spoken = self.words[p.words.clone()].partition_point(|w| w.start <= t);
        Some((i, spoken.max(1)))
    }

    /// Моменты, когда меняется картинка субтитров, после `t` (для упреждающей растеризации): начала следующих слов.
    pub fn changes_after(&self, t: f64) -> impl Iterator<Item = f64> + '_ {
        let first = self.words.partition_point(|w| w.start <= t);
        self.words[first..].iter().map(|w| w.start)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(spec: &[(&str, f64)]) -> Vec<SubWord> {
        spec.iter().map(|&(t, s)| SubWord { text: t.into(), start: s, end: s + 0.3, keyword: false }).collect()
    }

    #[test]
    fn phrases_break_on_sentences_pauses_and_capacity() {
        // 7 слов подряд, точка после третьего; пауза 1 с перед последним.
        let w = words(&[("Раз", 0.0), ("два", 0.4), ("три.", 0.8), ("Четыре", 1.2), ("пять", 1.6), ("шесть", 2.0), ("семь", 3.3)]);
        let t = SubtitleTrack::new(w.clone(), 2, 1);
        let ranges: Vec<_> = t.phrases.iter().map(|p| p.words.clone()).collect();
        // «Раз два три.» — 3 слова при вместимости 2 → 2 + 1; «Четыре пять шесть» → 2 + 1; «семь» — после паузы.
        assert_eq!(ranges, vec![0..2, 2..3, 3..5, 5..6, 6..7]);
        // Две строки по 2 — фраза из 3 слов делится 2 + 1 по строкам.
        let t = SubtitleTrack::new(w, 2, 2);
        assert_eq!(t.phrases[0].words, 0..3);
        assert_eq!(t.phrases[0].lines, vec![0..2, 2..3]);
        // 5 слов при вместимости 4 → 3 + 2, а не 4 + 1.
        let w = words(&[("а", 0.0), ("б", 0.4), ("в", 0.8), ("г", 1.2), ("д", 1.6)]);
        let t = SubtitleTrack::new(w, 2, 2);
        assert_eq!(t.phrases.iter().map(|p| p.words.len()).collect::<Vec<_>>(), vec![3, 2]);
    }

    #[test]
    fn state_by_time() {
        let w = words(&[("Раз", 0.0), ("два", 0.4), ("три.", 0.8), ("Ещё", 3.0)]);
        let t = SubtitleTrack::new(w, 3, 1);
        assert_eq!(t.at(-0.1), None);
        assert_eq!(t.at(0.0), Some((0, 1)));
        assert_eq!(t.at(0.5), Some((0, 2)));
        assert_eq!(t.at(1.5), Some((0, 3)), "держится после последнего слова");
        assert_eq!(t.at(1.8), None, "и исчезает в паузе");
        assert_eq!(t.at(3.1), Some((1, 1)));
        // Фразы подряд не накладываются.
        let w = words(&[("а", 0.0), ("б", 0.3), ("в", 0.6)]);
        let t = SubtitleTrack::new(w, 1, 1);
        assert_eq!(t.phrases[0].end, 0.3);
        assert_eq!(t.at(0.35), Some((1, 1)));
        assert_eq!(t.changes_after(0.3).collect::<Vec<_>>(), vec![0.6]);
    }

    #[test]
    fn keywords_match_word_forms_and_phrases() {
        let text = ["Настроим", "Lumen", "VPN,", "чтобы", "ошибки", "сервера", "не", "мешали.", "Который", "кот?"];
        let hits = keyword_hits(text, &["lumen vpn".into(), "ошибка".into(), "сервер".into(), "кот".into()]);
        assert_eq!(hits, vec![false, true, true, false, true, true, false, false, false, true]);
        assert!(!token_matches("серверная", "сервер"), "слишком длинное окончание");
        assert_eq!(normalize("«Ёлка»,"), "елка");
    }

    #[test]
    fn manual_toggle() {
        let mut e = SubtitleEdits::default();
        assert!(!e.keyword(1.234, false));
        e.toggle(1.234);
        assert!(e.keyword(1.2341, false) && !e.keyword(1.2341, true));
        e.toggle(1.2339);
        assert!(e.is_empty());
    }
}
