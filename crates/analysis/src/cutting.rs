//! LLM-нарезка (ARCHITECTURE §4, пп. 4–9): транскрипт → смысловые блоки → кандидаты в шортсы.
//!
//! Проход 1 — блоки по окнам 6 мин с перехлёстом 1 мин (из окна берём только границы из его середины);
//! слияние с границами тем по эмбеддингам; проход 2 — оценка, заголовок и лучший подотрезок каждого
//! содержательного блока; доводка — прилипание к паузам и проверка длительности.
//! Схема ответа гарантирует формат, но не смысл: номера предложений проверяются здесь (DECISIONS).

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::sentences::{Sentence, pause_after_word};
use crate::topics::TopicBoundary;
use crate::transcript::Transcript;
use crate::AnalysisError;

/// Версия формата кэша нарезки; при несовпадении нарезка делается заново.
pub const FORMAT_VERSION: u32 = 2;

// --- Промпты ---

const PROMPT_FILES: [&str; 4] = ["pass1-boundaries.md", "pass1.schema.json", "pass2-score.md", "pass2.schema.json"];
const BUILTIN: [&str; 4] = [
    include_str!("../prompts/pass1-boundaries.md"),
    include_str!("../prompts/pass1.schema.json"),
    include_str!("../prompts/pass2-score.md"),
    include_str!("../prompts/pass2.schema.json"),
];

/// Промпты и JSON-схемы двух проходов.
#[derive(Debug, Clone)]
pub struct Prompts {
    pub pass1: String,
    pub pass1_schema: Value,
    pub pass2: String,
    pub pass2_schema: Value,
    /// Хэш текстов: правка промпта делает кэш нарезки устаревшим.
    pub fingerprint: String,
}

impl Prompts {
    pub fn builtin() -> Self {
        Self::from_texts(BUILTIN.map(String::from)).expect("встроенные схемы — валидный JSON")
    }

    /// Встроенные промпты, перекрытые одноимёнными файлами из `dir` (если есть) — чтобы итерировать
    /// промпты без пересборки.
    pub fn load(dir: &Path) -> Result<Self, AnalysisError> {
        let mut texts = BUILTIN.map(String::from);
        for (text, name) in texts.iter_mut().zip(PROMPT_FILES) {
            match std::fs::read_to_string(dir.join(name)) {
                Ok(t) => {
                    log::info!("промпт {name} из {}", dir.display());
                    *text = t;
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
        Self::from_texts(texts)
    }

    fn from_texts([pass1, s1, pass2, s2]: [String; 4]) -> Result<Self, AnalysisError> {
        let mut h = Sha256::new();
        for t in [&pass1, &s1, &pass2, &s2] {
            h.update(t.as_bytes());
            h.update([0]);
        }
        let fingerprint = h.finalize().iter().take(8).map(|b| format!("{b:02x}")).collect();
        let schema = |name: &str, t: &str| {
            serde_json::from_str(t).map_err(|e| AnalysisError::Prompt(format!("{name}: {e}")))
        };
        Ok(Self {
            pass1_schema: schema(PROMPT_FILES[1], &s1)?,
            pass2_schema: schema(PROMPT_FILES[3], &s2)?,
            pass1,
            pass2,
            fingerprint,
        })
    }
}

// --- Результат ---

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BlockKind {
    Content,
    Intro,
    Outro,
    Ad,
    /// Зона пропуска, размеченная пользователем (модели не показывается).
    Skip,
}

/// Кто нашёл границу в начале блока.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Found {
    /// Начало ролика.
    Start,
    Llm,
    Embed,
    Both,
    /// Край зоны пропуска пользователя.
    Manual,
}

impl Found {
    /// Граница подтверждена или очевидна.
    pub fn sure(self) -> bool {
        matches!(self, Found::Start | Found::Both | Found::Manual)
    }
}

/// Смысловой блок прохода 1 (после слияния): содержание или зона интро/аутро/рекламы.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Block {
    /// Индексы в списке предложений.
    pub sentences: Range<usize>,
    pub start: f64,
    pub end: f64,
    pub kind: BlockKind,
    pub topic: String,
    pub found: Found,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Flag {
    /// Граница кандидата найдена только одним способом (LLM или эмбеддинги) — проверь.
    Unsure,
    /// Модель считает, что без остального видео непонятно.
    NotStandalone,
    /// Длиннее цели даже после повторного запроса.
    TooLong,
    /// Модель ответила не по смыслу (номера вне блока) — взят весь блок, оценки нет.
    NoScore,
}

/// Кандидат в шортс.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    pub sentences: Range<usize>,
    /// Секунды исходника, уже с отступами в паузы.
    pub start: f64,
    pub end: f64,
    /// Индекс блока в [`Cutting::blocks`].
    pub block: usize,
    /// 1–10; 0 — нет оценки.
    pub score: u8,
    pub reason: String,
    pub title: String,
    pub keywords: Vec<String>,
    pub flags: Vec<Flag>,
}

impl Candidate {
    pub fn duration(&self) -> f64 {
        self.end - self.start
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cutting {
    pub version: u32,
    /// Файл LLM.
    pub model: String,
    /// [`Prompts::fingerprint`].
    pub prompts: String,
    pub blocks: Vec<Block>,
    pub candidates: Vec<Candidate>,
    /// Зоны пропуска пользователя, с которыми нарезали ([`CutOptions::skip`]).
    #[serde(default)]
    pub skips: Vec<(f64, f64)>,
}

impl Cutting {
    pub fn cache_file(dir: &Path, hash: &str) -> PathBuf {
        dir.join(format!("{hash}.cut.json"))
    }

    /// Нарезка из кэша, если она сделана той же моделью с теми же промптами и зонами пропуска.
    pub fn load_cached(dir: &Path, hash: &str, model: &str, prompts: &str, skips: &[(f64, f64)]) -> Option<Self> {
        let path = Self::cache_file(dir, hash);
        let text = std::fs::read_to_string(&path).ok()?;
        match serde_json::from_str::<Cutting>(&text) {
            Ok(c) if c.version == FORMAT_VERSION && c.model == model && c.prompts == prompts && c.skips == skips => Some(c),
            Ok(_) => {
                log::info!("{}: другая модель, промпты, зоны пропуска или формат — нарезка заново", path.display());
                None
            }
            Err(e) => {
                log::warn!("{}: {e}", path.display());
                None
            }
        }
    }

    pub fn save_cached(&self, dir: &Path, hash: &str) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let path = Self::cache_file(dir, hash);
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec(self).map_err(std::io::Error::other)?)?;
        std::fs::rename(&tmp, &path)
    }
}

// --- Параметры ---

#[derive(Debug, Clone)]
pub struct CutOptions {
    /// Окно прохода 1 и перехлёст соседних окон, с.
    pub window: f64,
    pub overlap: f64,
    /// Границы LLM и эмбеддингов совпали, если ближе, с. Кандидаты эмбеддингов идут в среднем раз в ~30 с,
    /// поэтому при ±10 с совпадение почти случайно (шаг 2.3: «подтверждены» 20 границ из 24).
    pub match_tol: f64,
    /// Содержательный блок короче — склеивается с соседом, с.
    pub min_block: f64,
    /// …если склейка не длиннее, с.
    pub max_merged: f64,
    /// Блок длиннее — делится по границам эмбеддингов внутри него, с.
    pub split_over: f64,
    /// Цель длительности шортса, с: длиннее — повторный запрос «выбери короче».
    pub target: f64,
    /// Отступ от слова в паузу, с.
    pub pad: f64,
    /// Тема видео для прохода 2 (название файла).
    pub title: String,
    /// Зоны пропуска пользователя (начало, конец), с: эти предложения модель не видит, кандидатов в них нет.
    pub skip: Vec<(f64, f64)>,
}

impl Default for CutOptions {
    fn default() -> Self {
        Self {
            window: 360.0,
            overlap: 60.0,
            match_tol: 5.0,
            min_block: 20.0,
            max_merged: 90.0,
            split_over: 120.0,
            target: 60.0,
            pad: 0.3,
            title: String::new(),
            skip: Vec::new(),
        }
    }
}

/// Запас по длительности: модель считает по времени начала предложений, шортс на 2 с длиннее цели — не повод переспрашивать.
const TARGET_SLACK: f64 = 2.0;

// --- Подготовка текста ---

fn mmss(t: f64) -> String {
    let t = t.max(0.0) as u64;
    format!("{:02}:{:02}", t / 60, t % 60)
}

/// Строки `[номер] мм:сс текст` для предложений `range`; повторы (петли whisper) пропускаются.
pub fn prompt_lines(sentences: &[Sentence], range: Range<usize>) -> String {
    let mut out = String::new();
    for i in range {
        let s = &sentences[i];
        if s.repeat {
            continue;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&format!("[{i}] {} {}", mmss(s.start), s.text));
    }
    out
}

/// Окно прохода 1: предложения и «середина», из которой берутся границы.
#[derive(Debug, Clone, PartialEq)]
pub struct Window {
    pub sentences: Range<usize>,
    pub core: (f64, f64),
}

/// Окна по времени начала предложений: `[w0, w0 + window)`, следующее — с перехлёстом `overlap`.
pub fn windows(sentences: &[Sentence], opts: &CutOptions) -> Vec<Window> {
    let Some(last) = sentences.last() else { return vec![] };
    let dur = last.end;
    let mut out = Vec::new();
    let mut w0 = 0.0;
    loop {
        let w1 = (w0 + opts.window).min(dur);
        let a = sentences.partition_point(|s| s.start < w0);
        let b = sentences.partition_point(|s| s.start < w1).max(if w1 >= dur { sentences.len() } else { 0 });
        let core0 = if w0 == 0.0 { f64::NEG_INFINITY } else { w0 + opts.overlap / 2.0 };
        let core1 = if w1 >= dur { f64::INFINITY } else { w1 - opts.overlap / 2.0 };
        if a < b {
            out.push(Window { sentences: a..b, core: (core0, core1) });
        }
        if w1 >= dur {
            return out;
        }
        w0 = w1 - opts.overlap;
    }
}

// --- Проход 1 ---

#[derive(Debug, Clone, Deserialize)]
struct Pass1 {
    blocks: Vec<Pass1Block>,
}

#[derive(Debug, Clone, Deserialize)]
struct Pass1Block {
    start: usize,
    kind: BlockKind,
    topic: String,
}

/// Граница блока по ответу LLM.
#[derive(Debug, Clone, PartialEq)]
pub struct LlmBoundary {
    pub sentence: usize,
    pub kind: BlockKind,
    pub topic: String,
}

/// Границы из ответа на одно окно: только номера, показанные модели, и только из середины окна.
fn accept_pass1(sentences: &[Sentence], w: &Window, answer: Pass1, out: &mut Vec<LlmBoundary>) {
    for b in answer.blocks {
        if !w.sentences.contains(&b.start) || sentences[b.start].repeat {
            log::warn!("проход 1: номер {} вне окна {:?}", b.start, w.sentences);
            continue;
        }
        let t = sentences[b.start].start;
        if w.core.0 <= t && t < w.core.1 {
            out.push(LlmBoundary { sentence: b.start, kind: b.kind, topic: b.topic });
        }
    }
}

/// Не больше блоков в окне, чем по одному на 25 с: иначе модель дробит окно на блоки по 1–2 предложения
/// и тратит на ответ до 2000 токенов (шаг 2.3, первое окно «сети»).
fn max_blocks(sentences: &[Sentence], w: &Window) -> usize {
    let dur = sentences[w.sentences.end - 1].end - sentences[w.sentences.start].start;
    ((dur / 25.0).ceil() as usize).max(2)
}

/// Сколько раз переспрашивать остаток окна, если модель упёрлась в лимит блоков.
const PASS1_RETRIES: usize = 3;

/// Модель исчерпала лимит блоков, раздробив начало окна, — последний блок тянется до конца окна.
/// Остаток окна с последней границы спрашиваем заново; середина окна сдвигается к этой границе.
fn rest_of_window(sentences: &[Sentence], w: &Window, last: usize) -> Option<Window> {
    let rest = Window { sentences: last..w.sentences.end, core: (sentences[last].start, w.core.1) };
    (last > w.sentences.start && rest.sentences.len() >= 3 && rest.core.0 < rest.core.1).then_some(rest)
}

/// Схема прохода 1 с `maxItems` для массива `blocks` (если такой массив в схеме есть).
fn limit_blocks(schema: &Value, max: usize) -> Value {
    let mut s = schema.clone();
    if let Some(blocks) = s.pointer_mut("/properties/blocks").and_then(Value::as_object_mut) {
        blocks.insert("maxItems".into(), max.into());
    }
    s
}

// --- Слияние ---

/// Блоки из границ LLM и эмбеддингов (ARCHITECTURE §4, п. 6).
///
/// Граница LLM, рядом с которой есть граница эмбеддингов, — надёжная (`Both`). Границы только эмбеддингов
/// (точность ~0.5) режут лишь слишком длинные содержательные блоки. Слишком короткие содержательные блоки
/// склеиваются с соседом, на который они больше похожи по эмбеддингам — LLM дробит мельче, чем просили (шаг 0.3).
pub fn merge_blocks(
    sentences: &[Sentence],
    llm: &[LlmBoundary],
    embed: &[TopicBoundary],
    emb: &[Vec<f32>],
    opts: &CutOptions,
) -> Vec<Block> {
    if sentences.is_empty() {
        return vec![];
    }
    let mut llm: Vec<LlmBoundary> = llm.to_vec();
    llm.sort_by_key(|b| b.sentence);
    llm.dedup_by_key(|b| b.sentence);
    if llm.first().is_none_or(|b| b.sentence != 0) {
        llm.insert(0, LlmBoundary { sentence: 0, kind: BlockKind::Content, topic: String::new() });
    }
    let near_embed = |i: usize| embed.iter().any(|e| (sentences[e.sentence].start - sentences[i].start).abs() <= opts.match_tol);

    let mut blocks: Vec<Block> = Vec::with_capacity(llm.len());
    for (k, b) in llm.iter().enumerate() {
        let end = llm.get(k + 1).map_or(sentences.len(), |n| n.sentence);
        let found = if b.sentence == 0 {
            Found::Start
        } else if near_embed(b.sentence) {
            Found::Both
        } else {
            Found::Llm
        };
        blocks.push(make_block(sentences, b.sentence..end, b.kind, b.topic.clone(), found));
    }

    // Длинные блоки — по границам эмбеддингов, которых LLM не видела.
    let mut split = Vec::with_capacity(blocks.len());
    for b in blocks {
        if b.kind != BlockKind::Content || b.end - b.start <= opts.split_over {
            split.push(b);
            continue;
        }
        let cuts: Vec<usize> = embed
            .iter()
            .map(|e| e.sentence)
            .filter(|&i| i > b.sentences.start && i < b.sentences.end)
            .filter(|&i| sentences[i].start - b.start >= opts.min_block && b.end - sentences[i].start >= opts.min_block)
            .collect();
        let mut first = b.sentences.start;
        let mut found = b.found;
        for c in cuts.into_iter().chain([b.sentences.end]) {
            split.push(make_block(sentences, first..c, b.kind, b.topic.clone(), found));
            (first, found) = (c, Found::Embed);
        }
    }

    merge_short(sentences, split, emb, opts)
}

fn make_block(sentences: &[Sentence], r: Range<usize>, kind: BlockKind, topic: String, found: Found) -> Block {
    Block { start: sentences[r.start].start, end: sentences[r.end - 1].end, sentences: r, kind, topic, found }
}

/// Склейка коротких содержательных блоков с соседом, пока есть что склеивать: через неподтверждённую границу,
/// при равенстве — с более похожим по эмбеддингам. Склейка только по сходству теряла настоящие границы
/// (шаг 2.3, «сеть»: полнота 0.92 до склейки и 0.54 после).
fn merge_short(sentences: &[Sentence], mut blocks: Vec<Block>, emb: &[Vec<f32>], opts: &CutOptions) -> Vec<Block> {
    let centroid = |b: &Block| mean(&emb[b.sentences.clone()]);
    loop {
        let mut best: Option<(usize, f32, bool)> = None; // (левый блок пары, сходство, граница между ними подтверждена)
        for i in 0..blocks.len() {
            let b = &blocks[i];
            if b.kind != BlockKind::Content || b.end - b.start >= opts.min_block {
                continue;
            }
            for j in [i.wrapping_sub(1), i + 1] {
                let Some(n) = blocks.get(j) else { continue };
                if n.kind != BlockKind::Content || b.end.max(n.end) - b.start.min(n.start) > opts.max_merged {
                    continue;
                }
                // Сначала убираем неподтверждённую границу, при равенстве — к более похожему соседу.
                let sure = blocks[i.max(j)].found.sure();
                let sim = if emb.is_empty() { 0.0 } else { dot(&centroid(b), &centroid(n)) };
                if best.is_none_or(|(_, s, bs)| (!sure, sim) > (!bs, s)) {
                    best = Some((i.min(j), sim, sure));
                }
            }
        }
        let Some((i, ..)) = best else { return blocks };
        let right = blocks.remove(i + 1);
        let left = &mut blocks[i];
        let topic = if left.topic.is_empty() { right.topic } else { left.topic.clone() };
        *left = make_block(sentences, left.sentences.start..right.sentences.end, BlockKind::Content, topic, left.found);
    }
}

fn mean(vs: &[Vec<f32>]) -> Vec<f32> {
    let mut acc = vec![0.0f32; vs.first().map_or(0, Vec::len)];
    for v in vs {
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

// --- Зоны пропуска ---

/// Предложение в зоне пропуска — если его середина внутри зоны.
pub fn skipped(sentences: &[Sentence], skip: &[(f64, f64)]) -> Vec<bool> {
    sentences
        .iter()
        .map(|s| {
            let mid = (s.start + s.end) / 2.0;
            skip.iter().any(|&(a, b)| a <= mid && mid < b)
        })
        .collect()
}

/// Блоки с вырезанными зонами пропуска: каждая серия пропущенных предложений — блок `Skip`,
/// края зон — надёжные границы (`Found::Manual`) для соседних блоков.
pub fn apply_skips(sentences: &[Sentence], blocks: Vec<Block>, skipped: &[bool]) -> Vec<Block> {
    if !skipped.contains(&true) {
        return blocks;
    }
    let mut out: Vec<Block> = Vec::with_capacity(blocks.len());
    for b in blocks {
        let mut first = b.sentences.start;
        let mut found = b.found;
        for i in b.sentences.clone() {
            let run_ends = i + 1 == b.sentences.end || skipped[i + 1] != skipped[i];
            if !run_ends {
                continue;
            }
            if skipped[i] {
                match out.last_mut() {
                    // Зона продолжается из прошлого блока.
                    Some(last) if last.kind == BlockKind::Skip && last.sentences.end == first => {
                        *last = make_block(sentences, last.sentences.start..i + 1, BlockKind::Skip, String::new(), last.found);
                    }
                    _ => out.push(make_block(sentences, first..i + 1, BlockKind::Skip, String::new(), Found::Manual)),
                }
            } else {
                let after_skip = first > 0 && skipped[first - 1];
                let f = if after_skip { Found::Manual } else { found };
                out.push(make_block(sentences, first..i + 1, b.kind, b.topic.clone(), f));
            }
            first = i + 1;
            found = Found::Manual;
        }
    }
    out
}

// --- Проход 2 ---

#[derive(Debug, Clone, Deserialize)]
struct Pass2 {
    reason: String,
    standalone: bool,
    score: u8,
    start: usize,
    end: usize,
    title: String,
    keywords: Vec<String>,
}

fn pass2_user(sentences: &[Sentence], b: &Block, title: &str) -> String {
    let mut s = String::new();
    if !title.is_empty() {
        s.push_str(&format!("Тема видео: {title}\n"));
    }
    s.push_str(&format!("Длительность блока: {:.0} с\n\n{}", b.end - b.start, prompt_lines(sentences, b.sentences.clone())));
    s
}

/// Ответ прохода 2 → кандидат: номера проверены, границы и длительность — по предложениям (без отступов).
fn candidate_from(sentences: &[Sentence], blocks: &[Block], bi: usize, a: Option<Pass2>) -> Candidate {
    let b = &blocks[bi];
    let valid = a.as_ref().filter(|a| b.sentences.contains(&a.start) && b.sentences.contains(&a.end) && a.start <= a.end);
    let (range, mut c) = match valid {
        Some(a) => (
            a.start..a.end + 1,
            Candidate {
                sentences: 0..0,
                start: 0.0,
                end: 0.0,
                block: bi,
                score: a.score.clamp(1, 10),
                reason: a.reason.clone(),
                title: a.title.clone(),
                keywords: a.keywords.clone(),
                flags: if a.standalone { vec![] } else { vec![Flag::NotStandalone] },
            },
        ),
        None => {
            if let Some(a) = &a {
                log::warn!("проход 2: отрезок {}–{} вне блока {:?}", a.start, a.end, b.sentences);
            }
            let c = Candidate {
                sentences: 0..0,
                start: 0.0,
                end: 0.0,
                block: bi,
                score: 0,
                reason: String::new(),
                title: b.topic.clone(),
                keywords: vec![],
                flags: vec![Flag::NoScore],
            };
            (b.sentences.clone(), c)
        }
    };
    // Петли whisper в конце отрезка не нужны.
    let mut end = range.end;
    while end > range.start + 1 && sentences[end - 1].repeat {
        end -= 1;
    }
    c.start = sentences[range.start].start;
    c.end = sentences[end - 1].end;
    c.sentences = range.start..end;
    let unsure_start = c.sentences.start == b.sentences.start && !b.found.sure();
    let unsure_end = c.sentences.end == b.sentences.end && blocks.get(bi + 1).is_some_and(|n| !n.found.sure());
    if unsure_start || unsure_end {
        c.flags.push(Flag::Unsure);
    }
    c
}

// --- Доводка ---

/// Границы отрезка слов `words` с отступом `pad` в паузу: в тишину по VAD, если она есть, и никогда — в соседнее слово.
pub fn snap(t: &Transcript, words: Range<usize>, pad: f64) -> (f64, f64) {
    let w = &t.words;
    let (first, last) = (words.start, words.end - 1);
    let into = |want: f64, i: usize| {
        let p = pause_after_word(t, i).expect("не последнее слово");
        let s = p.silence.unwrap_or(p.gap);
        want.clamp(s.start, s.end)
    };
    let start = if first == 0 { (w[0].start - pad).max(0.0) } else { into(w[first].start - pad, first - 1) };
    let end = if last + 1 == w.len() { w[last].end + pad } else { into(w[last].end + pad, last) };
    (start, end)
}

/// Соседние кандидаты, прилипшие к одной паузе с двух сторон, делят её пополам — иначе их нельзя принять оба.
pub fn separate(candidates: &mut [Candidate]) {
    for i in 1..candidates.len() {
        let (a, b) = candidates.split_at_mut(i);
        let (prev, next) = (&mut a[i - 1], &mut b[0]);
        if prev.end > next.start && prev.start < next.start {
            let mid = (prev.end + next.start) / 2.0;
            (prev.end, next.start) = (mid, mid);
        }
    }
}

// --- Задача ---

/// Стадии нарезки для прогресса.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CutStage {
    /// Эмбеддинги предложений (с запуском сервера).
    Structure,
    /// Загрузка LLM на GPU (без прогресса).
    Model,
    /// Проходы 1 и 2.
    Llm,
}

/// Доля прохода 1 в прогрессе стадии `Llm` (по замерам 0.3: 64 с против 143 с).
const PASS1_SHARE: f32 = 0.3;

/// Нарезка одного исходника; результат кэшируется по хэшу.
#[derive(Debug, Clone)]
pub struct CutJob {
    pub hash: String,
    pub cache_dir: PathBuf,
    pub embed_model: PathBuf,
    pub llm_model: PathBuf,
    pub prompts: Prompts,
    pub opts: CutOptions,
}

impl CutJob {
    fn model_name(&self) -> String {
        self.llm_model.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    }

    pub fn cached(&self) -> Option<Cutting> {
        Cutting::load_cached(&self.cache_dir, &self.hash, &self.model_name(), &self.prompts.fingerprint, &self.opts.skip)
    }

    /// Нарезка из кэша или заново. Серверы эмбеддингов и LLM живут по очереди и завершаются до возврата.
    pub fn run(
        &self,
        t: &Transcript,
        cancel: &AtomicBool,
        mut progress: impl FnMut(CutStage, f32),
    ) -> Result<Cutting, AnalysisError> {
        if let Some(c) = self.cached() {
            return Ok(c);
        }
        let mut sentences = crate::sentences(t, &crate::SplitOptions::default());
        let skipped = skipped(&sentences, &self.opts.skip);
        // Речи нет (тишина, музыка, ролик в пару секунд) — резать нечего, модели не грузим.
        if sentences.is_empty() {
            let cutting = self.finish(vec![], vec![]);
            progress(CutStage::Llm, 1.0);
            return Ok(cutting);
        }
        for model in [&self.embed_model, &self.llm_model] {
            if !model.is_file() {
                return Err(AnalysisError::ModelMissing(model.clone()));
            }
        }
        let started = std::time::Instant::now();

        progress(CutStage::Structure, 0.0);
        let (emb, embed_bounds) = if sentences.len() < 3 {
            (vec![], vec![])
        } else {
            let server = sc_llama::Server::start(&sc_llama::ServerOptions::embedding(&self.embed_model), cancel)?;
            let emb = crate::embed_sentences(&server, &sentences, cancel)?;
            drop(server); // VRAM — под LLM
            let b = crate::topic_boundaries(&sentences, &emb, &crate::TilingOptions::default());
            (emb, b)
        };
        let embed_secs = started.elapsed().as_secs_f32();
        // Зоны пропуска модели не показываем — как повторы whisper (эмбеддинги уже посчитаны по всему тексту).
        for (s, &skip) in sentences.iter_mut().zip(&skipped) {
            s.repeat |= skip;
        }

        progress(CutStage::Model, 0.0);
        let server = sc_llama::Server::start(&sc_llama::ServerOptions::chat(&self.llm_model), cancel)?;
        let load_secs = started.elapsed().as_secs_f32() - embed_secs;

        let wins = windows(&sentences, &self.opts);
        let mut llm = Vec::new();
        for (k, w) in wins.iter().enumerate() {
            progress(CutStage::Llm, PASS1_SHARE * k as f32 / wins.len() as f32);
            let mut w = w.clone();
            for _ in 0..PASS1_RETRIES {
                let user = prompt_lines(&sentences, w.sentences.clone());
                if user.is_empty() {
                    break; // окно целиком в зоне пропуска
                }
                let max = max_blocks(&sentences, &w);
                let req = sc_llama::ChatRequest {
                    system: self.prompts.pass1.clone(),
                    user,
                    schema: limit_blocks(&self.prompts.pass1_schema, max),
                    temperature: 0.3,
                    max_tokens: 3072,
                };
                let answer = match server.chat_json::<Pass1>(&req, cancel) {
                    Ok(a) => a,
                    Err(sc_llama::LlamaError::BadAnswer(e)) => {
                        log::warn!("проход 1, окно {}: {e}", mmss(sentences[w.sentences.start].start));
                        break;
                    }
                    Err(e) => return Err(e.into()),
                };
                let full = answer.blocks.len() >= max;
                let last = answer.blocks.iter().map(|b| b.start).filter(|i| w.sentences.contains(i)).max();
                accept_pass1(&sentences, &w, answer, &mut llm);
                match last.filter(|_| full).and_then(|l| rest_of_window(&sentences, &w, l)) {
                    Some(rest) => w = rest,
                    None => break,
                }
            }
        }
        let blocks = merge_blocks(&sentences, &llm, &embed_bounds, &emb, &self.opts);
        let blocks = apply_skips(&sentences, blocks, &skipped);
        let pass1_secs = started.elapsed().as_secs_f32() - embed_secs - load_secs;

        let content: Vec<usize> = (0..blocks.len()).filter(|&i| blocks[i].kind == BlockKind::Content).collect();
        let mut candidates = Vec::with_capacity(content.len());
        for (k, &bi) in content.iter().enumerate() {
            progress(CutStage::Llm, PASS1_SHARE + (1.0 - PASS1_SHARE) * k as f32 / content.len() as f32);
            let mut c = self.score(&server, &sentences, &blocks, bi, None, cancel)?;
            if c.duration() > self.opts.target + TARGET_SLACK && blocks[bi].end - blocks[bi].start > self.opts.target {
                // Ужать (п. 8): повторный запрос с длительностью прошлого выбора.
                let shorter = self.score(&server, &sentences, &blocks, bi, Some(&c), cancel)?;
                if shorter.duration() < c.duration() && !shorter.flags.contains(&Flag::NoScore) {
                    c = shorter;
                }
                if c.duration() > self.opts.target + TARGET_SLACK {
                    c.flags.push(Flag::TooLong);
                }
            }
            let (start, end) = snap(t, sentences[c.sentences.start].words.start..sentences[c.sentences.end - 1].words.end, self.opts.pad);
            (c.start, c.end) = (start, end);
            candidates.push(c);
        }
        drop(server);
        separate(&mut candidates);
        progress(CutStage::Llm, 1.0);
        log::info!(
            "нарезка: {} предложений → {} блоков ({} с подтверждённой границей), {} кандидатов; эмбеддинги {embed_secs:.1} с, \
             загрузка LLM {load_secs:.1} с, проход 1 {pass1_secs:.0} с, всего {:.0} с",
            sentences.len(),
            blocks.len(),
            blocks.iter().filter(|b| b.found == Found::Both).count(),
            candidates.len(),
            started.elapsed().as_secs_f32(),
        );
        Ok(self.finish(blocks, candidates))
    }

    /// Итог нарезки с записью в кэш.
    fn finish(&self, blocks: Vec<Block>, candidates: Vec<Candidate>) -> Cutting {
        let cutting = Cutting {
            version: FORMAT_VERSION,
            model: self.model_name(),
            prompts: self.prompts.fingerprint.clone(),
            blocks,
            candidates,
            skips: self.opts.skip.clone(),
        };
        if let Err(e) = cutting.save_cached(&self.cache_dir, &self.hash) {
            log::warn!("кэш нарезки не записан: {e}");
        }
        cutting
    }

    /// Проход 2 для блока `bi`; `previous` — прошлый слишком длинный выбор (просим короче).
    fn score(
        &self,
        server: &sc_llama::Server,
        sentences: &[Sentence],
        blocks: &[Block],
        bi: usize,
        previous: Option<&Candidate>,
        cancel: &AtomicBool,
    ) -> Result<Candidate, AnalysisError> {
        let mut user = pass2_user(sentences, &blocks[bi], &self.opts.title);
        if let Some(p) = previous {
            user.push_str(&format!(
                "\n\nПрошлый выбор — предложения {}–{} — длится {:.0} с, это больше {:.0} с. \
                 Выбери внутри блока более короткий законченный отрезок.",
                p.sentences.start,
                p.sentences.end - 1,
                p.duration(),
                self.opts.target,
            ));
        }
        let req = sc_llama::ChatRequest {
            system: self.prompts.pass2.clone(),
            user,
            schema: self.prompts.pass2_schema.clone(),
            temperature: 0.3,
            max_tokens: 1024,
        };
        let answer = match server.chat_json::<Pass2>(&req, cancel) {
            Ok(a) => Some(a),
            Err(sc_llama::LlamaError::BadAnswer(e)) => {
                log::warn!("проход 2, блок {}: {e}", mmss(blocks[bi].start));
                None
            }
            Err(e) => return Err(e.into()),
        };
        if cancel.load(Ordering::Relaxed) {
            return Err(AnalysisError::Cancelled);
        }
        Ok(candidate_from(sentences, blocks, bi, answer))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcript::{FORMAT_VERSION as TR_VERSION, Span, Word};

    /// Предложения по 5 с (4 с речи + 1 с паузы), по одному «слову» в каждом.
    fn sents(n: usize) -> Vec<Sentence> {
        (0..n)
            .map(|i| {
                let start = i as f64 * 5.0;
                Sentence { words: i..i + 1, start, end: start + 4.0, text: format!("п{i}."), pause_after: 1.0, repeat: false }
            })
            .collect()
    }

    /// Ролик без речи: нарезка пустая и не требует моделей (их файлов нет).
    #[test]
    fn no_speech_cuts_nothing_without_models() {
        let tmp = tempfile::tempdir().unwrap();
        let job = CutJob {
            hash: "silent".into(),
            cache_dir: tmp.path().into(),
            embed_model: tmp.path().join("нет-эмбеддингов.gguf"),
            llm_model: tmp.path().join("нет-llm.gguf"),
            prompts: Prompts::builtin(),
            opts: CutOptions::default(),
        };
        let t = Transcript {
            version: TR_VERSION,
            model: String::new(),
            language: "ru".into(),
            prompt: String::new(),
            words: vec![],
            segments: vec![],
            speech: vec![],
        };
        let c = job.run(&t, &AtomicBool::new(false), |_, _| {}).unwrap();
        assert!(c.candidates.is_empty() && c.blocks.is_empty());
        assert!(job.cached().is_some(), "пустой итог тоже кэшируется");
    }

    fn lb(sentence: usize, kind: BlockKind) -> LlmBoundary {
        LlmBoundary { sentence, kind, topic: format!("т{sentence}") }
    }

    fn eb(sentence: usize) -> TopicBoundary {
        TopicBoundary { sentence, time: sentence as f64 * 5.0 - 0.5, depth: 1.0 }
    }

    #[test]
    fn builtin_prompts_parse() {
        let p = Prompts::builtin();
        assert_eq!(p.pass1_schema["required"][0], "blocks");
        assert_eq!(p.fingerprint.len(), 16);
    }

    #[test]
    fn prompt_override_changes_fingerprint() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(Prompts::load(tmp.path()).unwrap().fingerprint, Prompts::builtin().fingerprint);
        std::fs::write(tmp.path().join("pass2-score.md"), "другой промпт").unwrap();
        let p = Prompts::load(tmp.path()).unwrap();
        assert_eq!(p.pass2, "другой промпт");
        assert_ne!(p.fingerprint, Prompts::builtin().fingerprint);
        std::fs::write(tmp.path().join("pass1.schema.json"), "{").unwrap();
        assert!(matches!(Prompts::load(tmp.path()), Err(AnalysisError::Prompt(_))));
    }

    #[test]
    fn window_schema_limits_blocks() {
        let s = sents(72); // 0–359 с
        let w = &windows(&s, &CutOptions::default())[0];
        let schema = limit_blocks(&Prompts::builtin().pass1_schema, max_blocks(&s, w));
        assert_eq!(schema["properties"]["blocks"]["maxItems"], 15);
        assert_eq!(schema["properties"]["blocks"]["minItems"], 1);
    }

    #[test]
    fn rest_of_window_moves_core() {
        let s = sents(200);
        let w = &windows(&s, &CutOptions::default())[1]; // 300–660 с, середина 330–630
        let rest = rest_of_window(&s, w, 70).unwrap();
        assert_eq!((rest.sentences.clone(), rest.core), (70..132, (350.0, 630.0)));
        // Граница в самом начале или за серединой окна — переспрашивать нечего.
        assert_eq!(rest_of_window(&s, w, 60), None);
        assert_eq!(rest_of_window(&s, w, 128), None);
    }

    #[test]
    fn lines_skip_repeats() {
        let mut s = sents(3);
        s[1].repeat = true;
        s[2].start = 65.0;
        assert_eq!(prompt_lines(&s, 0..3), "[0] 00:00 п0.\n[2] 01:05 п2.");
    }

    #[test]
    fn windows_overlap_and_cover() {
        let s = sents(200); // 1000 с
        let w = windows(&s, &CutOptions::default());
        // 0–360, 300–660, 600–960, 900–1000
        assert_eq!(w.len(), 4);
        assert_eq!(w[0].sentences, 0..72);
        assert_eq!(w[1].sentences, 60..132);
        assert_eq!(w[3].sentences.end, 200);
        // Середины покрывают всю шкалу без дыр и наложений.
        for p in w.windows(2) {
            assert_eq!(p[0].core.1, p[1].core.0);
        }
        assert_eq!(w[0].core.0, f64::NEG_INFINITY);
        assert_eq!(w[3].core.1, f64::INFINITY);
    }

    #[test]
    fn pass1_rejects_numbers_outside_window_and_core() {
        let s = sents(200);
        let w = &windows(&s, &CutOptions::default())[1]; // 300–660 с, середина 330–630
        let answer = Pass1 {
            blocks: vec![
                Pass1Block { start: 60, kind: BlockKind::Content, topic: String::new() }, // 300 с — в перехлёсте
                Pass1Block { start: 80, kind: BlockKind::Ad, topic: String::new() },      // 400 с — берём
                Pass1Block { start: 5, kind: BlockKind::Content, topic: String::new() },  // вне окна
            ],
        };
        let mut out = vec![];
        accept_pass1(&s, w, answer, &mut out);
        assert_eq!(out.iter().map(|b| (b.sentence, b.kind)).collect::<Vec<_>>(), [(80, BlockKind::Ad)]);
    }

    #[test]
    fn merge_marks_confirmed_and_glues_short() {
        let s = sents(40); // 200 с
        // LLM: 0, 10 (подтверждена), 12 (блок 10–11 короткий, 10 с), 20 (реклама), 24, 32.
        let llm = [
            lb(0, BlockKind::Intro),
            lb(10, BlockKind::Content),
            lb(12, BlockKind::Content),
            lb(20, BlockKind::Ad),
            lb(24, BlockKind::Content),
            lb(32, BlockKind::Content),
        ];
        let emb: Vec<Vec<f32>> = (0..40).map(|i| if i < 12 { vec![1.0, 0.0] } else { vec![0.0, 1.0] }).collect();
        let b = merge_blocks(&s, &llm, &[eb(11), eb(33)], &emb, &CutOptions::default());
        let got: Vec<_> = b.iter().map(|b| (b.sentences.clone(), b.kind, b.found)).collect();
        assert_eq!(
            got,
            [
                (0..10, BlockKind::Intro, Found::Start),
                // Блок 10–11 (10 с) склеен с правым соседом: левый — интро.
                (10..20, BlockKind::Content, Found::Both),
                (20..24, BlockKind::Ad, Found::Llm),
                (24..32, BlockKind::Content, Found::Llm),
                (32..40, BlockKind::Content, Found::Both),
            ]
        );
    }

    #[test]
    fn long_block_is_split_by_embeddings() {
        let s = sents(60); // 300 с одним блоком
        let b = merge_blocks(&s, &[lb(0, BlockKind::Content)], &[eb(2), eb(30)], &[], &CutOptions::default());
        // Граница у 10 с слишком близко к началу (< min_block) — не режем; 150 с — режем.
        assert_eq!(b.iter().map(|b| (b.sentences.clone(), b.found)).collect::<Vec<_>>(), [(0..30, Found::Start), (30..60, Found::Embed)]);
    }

    #[test]
    fn pass2_validation_and_flags() {
        let s = sents(40);
        let blocks = vec![
            make_block(&s, 0..10, BlockKind::Content, "а".into(), Found::Start),
            make_block(&s, 10..20, BlockKind::Content, "б".into(), Found::Llm),
            make_block(&s, 20..40, BlockKind::Content, "в".into(), Found::Both),
        ];
        let answer = |start, end| Pass2 {
            reason: "р".into(),
            standalone: false,
            score: 12,
            start,
            end,
            title: "з".into(),
            keywords: vec![],
        };
        // Внутри блока, до его конца: граница 10 (Llm) не задета.
        let c = candidate_from(&s, &blocks, 0, Some(answer(2, 5)));
        assert_eq!((c.sentences.clone(), c.score, c.start, c.end), (2..6, 10, 10.0, 29.0));
        assert_eq!(c.flags, [Flag::NotStandalone]);
        // Кончается на неподтверждённой границе следующего блока.
        let c = candidate_from(&s, &blocks, 0, Some(answer(0, 9)));
        assert_eq!(c.flags, [Flag::NotStandalone, Flag::Unsure]);
        // Номер вне блока — весь блок без оценки; начало блока неподтверждённое.
        let c = candidate_from(&s, &blocks, 1, Some(answer(5, 12)));
        assert_eq!((c.sentences.clone(), c.score, c.title.as_str()), (10..20, 0, "б"));
        assert_eq!(c.flags, [Flag::NoScore, Flag::Unsure]);
    }

    #[test]
    fn trailing_repeats_are_dropped() {
        let mut s = sents(10);
        s[8].repeat = true;
        s[9].repeat = true;
        let blocks = vec![make_block(&s, 0..10, BlockKind::Content, String::new(), Found::Start)];
        let c = candidate_from(&s, &blocks, 0, None);
        assert_eq!(c.sentences, 0..8);
    }

    #[test]
    fn snap_into_silence_not_words() {
        let w = |text: &str, start, end| Word { text: text.into(), start, end, dtw: None, p: 1.0 };
        let t = Transcript {
            version: TR_VERSION,
            model: String::new(),
            language: "ru".into(),
            prompt: String::new(),
            // Пауза 2.0–3.0 с тишиной по VAD 2.2–2.9; пауза 4.0–4.1 почти нулевая.
            words: vec![w("а.", 0.5, 2.0), w("б", 3.0, 4.0), w("в.", 4.1, 5.0), w("г", 5.05, 6.0)],
            segments: vec![],
            speech: vec![Span { start: 0.4, end: 2.2 }, Span { start: 2.9, end: 6.0 }],
        };
        let near = |(a, b): (f64, f64), (x, y): (f64, f64)| assert!((a - x).abs() < 1e-9 && (b - y).abs() < 1e-9, "{a}, {b}");
        // Начало 3.0 − 0.3 = 2.7 — в тишине; конец 5.0 + 0.3 упирается в следующее слово (5.05).
        near(snap(&t, 1..3, 0.3), (2.7, 5.05));
        // Отступ 0.9 вылез бы из тишины по VAD — начало на её краю; конец — на следующем слове.
        near(snap(&t, 1..2, 0.9), (2.2, 4.1));
        // Края записи.
        near(snap(&t, 0..4, 0.8), (0.0, 6.8));
    }

    #[test]
    fn cache_checks_model_and_prompts() {
        let tmp = tempfile::tempdir().unwrap();
        let c = Cutting { version: FORMAT_VERSION, model: "m.gguf".into(), prompts: "p1".into(), blocks: vec![], candidates: vec![], skips: vec![] };
        c.save_cached(tmp.path(), "h").unwrap();
        assert_eq!(Cutting::load_cached(tmp.path(), "h", "m.gguf", "p1", &[]), Some(c));
        assert_eq!(Cutting::load_cached(tmp.path(), "h", "m.gguf", "p2", &[]), None);
        assert_eq!(Cutting::load_cached(tmp.path(), "h", "x.gguf", "p1", &[]), None);
        assert_eq!(Cutting::load_cached(tmp.path(), "h", "m.gguf", "p1", &[(10.0, 20.0)]), None, "другие зоны пропуска");
    }

    #[test]
    fn skip_zones_split_blocks() {
        let s = sents(20); // предложения по 5 с: 0–4, 5–9, …
        let blocks = vec![
            make_block(&s, 0..8, BlockKind::Content, "а".into(), Found::Start),
            make_block(&s, 8..20, BlockKind::Content, "б".into(), Found::Llm),
        ];
        // Зона 24–46 с: середины предложений 5..=8 (27, 32, 37, 42 с) — внутри, через границу блоков.
        let skipped = skipped(&s, &[(24.0, 46.0)]);
        assert_eq!(skipped.iter().filter(|&&x| x).count(), 4);
        let out = apply_skips(&s, blocks.clone(), &skipped);
        let got: Vec<_> = out.iter().map(|b| (b.sentences.clone(), b.kind, b.found)).collect();
        assert_eq!(
            got,
            [
                (0..5, BlockKind::Content, Found::Start),
                (5..9, BlockKind::Skip, Found::Manual),
                (9..20, BlockKind::Content, Found::Manual),
            ]
        );
        assert_eq!(out[2].topic, "б");
        // Без зон — без изменений.
        assert_eq!(apply_skips(&s, blocks.clone(), &[false; 20]), blocks);
        // Пропущенные предложения не попадают в текст для модели.
        let mut hidden = s.clone();
        for (x, &k) in hidden.iter_mut().zip(&skipped) {
            x.repeat |= k;
        }
        let lines = prompt_lines(&hidden, 4..10);
        assert!(lines.contains("[4]") && lines.contains("[9]") && !lines.contains("[5]"), "{lines}");
    }

    #[test]
    fn neighbours_share_pause() {
        let c = |start: f64, end: f64| Candidate {
            sentences: 0..1,
            start,
            end,
            block: 0,
            score: 5,
            reason: String::new(),
            title: String::new(),
            keywords: vec![],
            flags: vec![],
        };
        let mut v = vec![c(0.0, 10.3), c(10.1, 20.0), c(25.0, 30.0)];
        separate(&mut v);
        assert_eq!(v.iter().map(|c| (c.start, c.end)).collect::<Vec<_>>(), [(0.0, 10.2), (10.2, 20.0), (25.0, 30.0)]);
    }
}
