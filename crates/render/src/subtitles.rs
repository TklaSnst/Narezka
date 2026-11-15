//! Субтитры в рендере: те же растры `sc_raster::subtitle_art`, что в превью, по одному на состояние фразы
//! (фраза × начатое слово). Все состояния слоя — одного размера (общий охват, обрезанный холстом) и стоят в одной точке,
//! поэтому их можно отдать ffmpeg одним входом — concat-списком картинок с длительностями — и наложить одним `overlay`.

use sc_core::{Canvas, Layer, SubWord, SubtitleMode, SubtitleTrack};
use sc_raster::Bitmap;

/// Последовательность картинок субтитров одного слоя: время — от начала шортса.
#[derive(Debug, Clone, PartialEq)]
pub struct SubtitleSeq {
    /// Левый верхний угол всех картинок на холсте.
    pub x: i32,
    pub y: i32,
    /// Разные картинки (первая может быть пустой — «ничего не видно»).
    pub frames: Vec<Bitmap>,
    /// По порядку: номер картинки в `frames` и сколько секунд она видна. Сумма — длина шортса.
    pub timeline: Vec<(usize, f64)>,
}

/// Что видно на интервале: фраза и число начатых слов (у статичного — 0).
type State = Option<(usize, usize)>;

/// Интервалы `[a, b)` отрезка `[start, end)` (секунды исходника) с одинаковой картинкой.
fn states(track: &SubtitleTrack, mode: SubtitleMode, start: f64, end: f64) -> Vec<(State, f64, f64)> {
    let mut cuts: Vec<f64> = vec![start, end];
    for p in &track.phrases {
        cuts.extend([p.start, p.end]);
    }
    cuts.extend(track.words.iter().map(|w| w.start));
    cuts.retain(|&t| (start..=end).contains(&t));
    cuts.sort_by(f64::total_cmp);
    cuts.dedup();

    let mut out: Vec<(State, f64, f64)> = Vec::new();
    for w in cuts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let state = track.at(a).map(|(p, s)| (p, if mode == SubtitleMode::Static { 0 } else { s }));
        match out.last_mut() {
            Some(last) if last.0 == state => last.2 = b,
            _ => out.push((state, a, b)),
        }
    }
    out
}

/// Последовательность для слоя субтитров `layer` и слов шортса `words` на отрезке `[start, end)`.
/// Не слой субтитров, нет слов или ничего не попадает на холст — `None`.
pub fn sequence(layer: &Layer, words: &[SubWord], start: f64, end: f64, canvas: Canvas) -> Option<SubtitleSeq> {
    let &Layer::Subtitles { words_per_line, lines, mode, .. } = layer else { return None };
    if words.is_empty() || end <= start {
        return None;
    }
    let track = SubtitleTrack::new(words.to_vec(), words_per_line, lines);
    let intervals = states(&track, mode, start, end);

    // Растры разных состояний.
    let mut keys: Vec<(usize, usize)> = Vec::new();
    for (s, _, _) in &intervals {
        if let Some(k) = s
            && !keys.contains(k)
        {
            keys.push(*k);
        }
    }
    let sprites: Vec<_> = keys.iter().map(|&(p, s)| sc_raster::subtitle_art(layer, &track, p, s, canvas).above).collect();

    // Общий охват, обрезанный холстом.
    let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for s in sprites.iter().flatten() {
        x0 = x0.min(s.x);
        y0 = y0.min(s.y);
        x1 = x1.max(s.x + s.bitmap.width as i32);
        y1 = y1.max(s.y + s.bitmap.height as i32);
    }
    let (x0, y0) = (x0.max(0), y0.max(0));
    let (x1, y1) = (x1.min(canvas.width as i32), y1.min(canvas.height as i32));
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let (w, h) = ((x1 - x0) as u32, (y1 - y0) as u32);

    // Картинка 0 — пустая, дальше — по состоянию.
    let mut frames = vec![Bitmap::new(w, h)];
    for s in &sprites {
        let mut bm = Bitmap::new(w, h);
        if let Some(s) = s {
            bm.blit(&s.bitmap, s.x - x0, s.y - y0);
        }
        frames.push(bm);
    }
    let timeline = intervals
        .iter()
        .map(|(s, a, b)| {
            let frame = s.map_or(0, |k| 1 + keys.iter().position(|&x| x == k).unwrap());
            (frame, b - a)
        })
        .collect();
    Some(SubtitleSeq { x: x0, y: y0, frames, timeline })
}

impl SubtitleSeq {
    /// Список для concat-демультиплексора ffmpeg; `file(k)` — имя файла картинки `k` (рядом со списком).
    /// Последняя картинка повторена: без этого concat не учитывает длительность последней записи.
    pub fn ffconcat(&self, file: impl Fn(usize) -> String) -> String {
        let mut s = String::from("ffconcat version 1.0\n");
        for &(k, d) in &self.timeline {
            s += &format!("file '{}'\nduration {d:.6}\n", file(k));
        }
        if let Some(&(k, _)) = self.timeline.last() {
            s += &format!("file '{}'\n", file(k));
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sc_core::{AccentMode, Align, Color, Rect, VAlign};

    fn layer(mode: SubtitleMode) -> Layer {
        Layer::Subtitles {
            rect: Rect::new(0.1, 0.6, 0.8, 0.2),
            font: "Montserrat ExtraBold".into(),
            size: 0.06,
            color: Color::WHITE,
            active_color: Color::rgb(255, 214, 10),
            active_mode: AccentMode::Color,
            keyword_color: Color::rgb(255, 59, 48),
            dim_color: Color([255, 255, 255, 0]),
            mode,
            words_per_line: 2,
            lines: 1,
            uppercase: true,
            outline: None,
            shadow: None,
            plate: None,
            align: Align::Center,
            valign: VAlign::Center,
        }
    }

    fn words() -> Vec<SubWord> {
        let w = |t: &str, s: f64| SubWord { text: t.into(), start: s, end: s + 0.3, keyword: false };
        // «Раз два» — фраза 10.0–10.7+0.6, пауза, «три.» с 12.0
        vec![w("Раз", 10.0), w("два", 10.4), w("три.", 12.0)]
    }

    #[test]
    fn timeline_follows_words_and_pauses() {
        let canvas = Canvas { width: 360, height: 640 };
        let s = sequence(&layer(SubtitleMode::Highlight), &words(), 9.5, 13.0, canvas).unwrap();
        // пусто 0.5 → «Раз» 0.4 → «два» до 10.7+0.6 = 11.3 → пусто до 12 → «три.» до 12.3+0.6 → пусто
        let t: Vec<(usize, f64)> = s.timeline.iter().map(|&(k, d)| (k, (d * 1000.0).round() / 1000.0)).collect();
        assert_eq!(t, vec![(0, 0.5), (1, 0.4), (2, 0.9), (0, 0.7), (3, 0.9), (0, 0.1)]);
        assert_eq!(s.frames.len(), 4);
        assert!((s.timeline.iter().map(|x| x.1).sum::<f64>() - 3.5).abs() < 1e-9);
        // Все картинки одного размера; пустая — прозрачная, остальные — нет.
        assert!(s.frames.iter().all(|f| (f.width, f.height) == (s.frames[0].width, s.frames[0].height)));
        assert!(s.frames[0].data.iter().all(|&b| b == 0));
        assert!(s.frames[1..].iter().all(|f| f.data.chunks(4).any(|p| p[3] > 0)));
        // Картинка совпадает с растром превью, сдвинутым в общий охват.
        let track = SubtitleTrack::new(words(), 2, 1);
        let p = sc_raster::subtitle_art(&layer(SubtitleMode::Highlight), &track, 0, 2, canvas).above.unwrap();
        let mut expect = Bitmap::new(s.frames[0].width, s.frames[0].height);
        expect.blit(&p.bitmap, p.x - s.x, p.y - s.y);
        assert_eq!(s.frames[2], expect);

        let list = s.ffconcat(|k| format!("s{k}.png"));
        assert!(list.starts_with("ffconcat version 1.0\nfile 's0.png'\nduration 0.500000\nfile 's1.png'\n"), "{list}");
        assert!(list.ends_with("file 's3.png'\nduration 0.900000\nfile 's0.png'\nduration 0.100000\nfile 's0.png'\n"), "{list}");
    }

    #[test]
    fn static_mode_has_one_frame_per_phrase_and_clips_to_short() {
        let canvas = Canvas { width: 360, height: 640 };
        // Шортс начинается посреди первой фразы и кончается до «три.».
        let s = sequence(&layer(SubtitleMode::Static), &words(), 10.2, 11.8, canvas).unwrap();
        let t: Vec<(usize, f64)> = s.timeline.iter().map(|&(k, d)| (k, (d * 1000.0).round() / 1000.0)).collect();
        assert_eq!(t, vec![(1, 1.1), (0, 0.5)]);
        assert!(sequence(&layer(SubtitleMode::Static), &[], 0.0, 1.0, canvas).is_none());
        assert!(sequence(&Layer::BannerSlot { rect: Rect::new(0.0, 0.0, 1.0, 1.0) }, &words(), 9.0, 13.0, canvas).is_none());
    }
}
