//! Субтитры: растр одной фразы в одном состоянии (сколько слов уже сказано). Раскладка фразы на строки —
//! [`sc_core::SubtitleTrack`]; здесь только цвета слов и отрисовка — одна и та же для превью и рендера.

use sc_core::{AccentMode, Canvas, Color, Layer, SubtitleMode, SubtitleTrack};

use crate::text::{SpanPaint, TextStyle, draw_spans};
use crate::{LayerArt, rect_px, text_sprite};

/// Растр фразы `phrase` дорожки `track`, когда начаты `spoken` её слов (1…), для слоя субтитров `layer`.
/// Не слой субтитров или нет такой фразы — пустой растр.
pub fn subtitle_art(layer: &Layer, track: &SubtitleTrack, phrase: usize, spoken: usize, canvas: Canvas) -> LayerArt {
    let Layer::Subtitles {
        rect,
        font,
        size,
        color,
        active_color,
        active_mode,
        keyword_color,
        dim_color,
        mode,
        uppercase,
        outline,
        shadow,
        plate,
        align,
        valign,
        ..
    } = layer
    else {
        return LayerArt::default();
    };
    let Some(p) = track.phrases.get(phrase) else { return LayerArt::default() };
    let cw = canvas.width as f32;

    // paints[0] — пробелы и переводы строк, дальше — по слову фразы.
    let mut paints = vec![SpanPaint { color: *color, plate: None }];
    let mut spans: Vec<(String, usize)> = Vec::new();
    for (li, line) in p.lines.iter().enumerate() {
        if li > 0 {
            spans.push(("\n".into(), 0));
        }
        for (wi, i) in line.clone().enumerate() {
            if wi > 0 {
                spans.push((" ".into(), 0));
            }
            let w = &track.words[i];
            let k = i - p.words.start; // номер слова во фразе
            let normal = if w.keyword { *keyword_color } else { *color };
            let current = *mode != SubtitleMode::Static && k + 1 == spoken;
            let paint = if current {
                match active_mode {
                    AccentMode::Color => SpanPaint { color: *active_color, plate: None },
                    AccentMode::Plate => SpanPaint { color: normal, plate: Some(*active_color) },
                }
            } else if *mode == SubtitleMode::Reveal && k >= spoken {
                SpanPaint { color: *dim_color, plate: None }
            } else {
                SpanPaint { color: normal, plate: None }
            };
            spans.push((if *uppercase { w.text.to_uppercase() } else { w.text.clone() }, paints.len()));
            paints.push(paint);
        }
    }

    let (x, y, w, h) = rect_px(*rect, canvas);
    let style = TextStyle {
        font,
        size: size * cw,
        color: *color,
        accent_color: *active_color,
        accent_mode: *active_mode,
        outline: outline.map(|o| ((o.width * cw).max(0.5), o.color)),
        align: *align,
        valign: *valign,
        wrap: false,
    };
    let line_plate: Option<Color> = *plate;
    let above = text_sprite(x, y, w, h, &style, shadow.as_ref(), cw, |pm, r| {
        draw_spans(pm, r, &spans, &paints, line_plate, &style);
    });
    LayerArt { below: None, above }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sc_core::{Outline, Rect, SubWord};

    fn layer(mode: SubtitleMode) -> Layer {
        Layer::Subtitles {
            rect: Rect::new(0.0, 0.0, 1.0, 1.0),
            font: "Montserrat ExtraBold".into(),
            size: 0.1,
            color: Color::WHITE,
            active_color: Color::rgb(255, 214, 10),
            active_mode: AccentMode::Color,
            keyword_color: Color::rgb(255, 59, 48),
            dim_color: Color([255, 255, 255, 0]),
            mode,
            words_per_line: 2,
            lines: 2,
            uppercase: true,
            outline: Some(Outline { width: 0.005, color: Color::BLACK }),
            shadow: None,
            plate: None,
            align: sc_core::Align::Center,
            valign: sc_core::VAlign::Center,
        }
    }

    fn track() -> SubtitleTrack {
        let w = |t: &str, s: f64, keyword| SubWord { text: t.into(), start: s, end: s + 0.3, keyword };
        SubtitleTrack::new(vec![w("раз", 0.0, false), w("два", 0.4, true), w("три", 0.8, false)], 2, 2)
    }

    /// Пиксели растра по цвету: (жёлтые, красные, белые) — непрозрачные с запасом от сглаживания краёв.
    fn colors(art: &LayerArt) -> (usize, usize, usize) {
        let bm = &art.above.as_ref().unwrap().bitmap;
        let mut n = (0, 0, 0);
        for p in bm.data.as_chunks::<4>().0.iter().filter(|p| p[3] == 255) {
            match (p[0], p[1], p[2]) {
                (255, 214, 10) => n.0 += 1,
                (255, 59, 48) => n.1 += 1,
                (255, 255, 255) => n.2 += 1,
                _ => {}
            }
        }
        n
    }

    #[test]
    fn modes_paint_words_differently() {
        let canvas = Canvas { width: 400, height: 300 };
        let t = track();
        assert_eq!(t.phrases.len(), 1);
        // Подсветка: первое слово жёлтое, ключевое — красное.
        let (y1, r1, w1) = colors(&subtitle_art(&layer(SubtitleMode::Highlight), &t, 0, 1, canvas));
        assert!(y1 > 0 && r1 > 0 && w1 > 0, "{y1} {r1} {w1}");
        // Текущее — ключевое слово: красного нет, жёлтое есть.
        let (y2, r2, _) = colors(&subtitle_art(&layer(SubtitleMode::Highlight), &t, 0, 2, canvas));
        assert!(y2 > 0 && r2 == 0);
        // Статичный: жёлтого нет вовсе.
        let (ys, rs, _) = colors(&subtitle_art(&layer(SubtitleMode::Static), &t, 0, 2, canvas));
        assert!(ys == 0 && rs > 0);
        // Проявление со скрытыми несказанными: после первого слова только оно — жёлтое, белых и красных нет.
        let reveal = subtitle_art(&layer(SubtitleMode::Reveal), &t, 0, 1, canvas);
        let (yr, rr, wr) = colors(&reveal);
        assert!(yr > 0 && rr == 0 && wr == 0, "{yr} {rr} {wr}");
        // Раскладка не прыгает: растр того же размера и места во всех состояниях.
        let full = subtitle_art(&layer(SubtitleMode::Reveal), &t, 0, 3, canvas);
        let (a, b) = (reveal.above.unwrap(), full.above.unwrap());
        assert_eq!((a.x, a.y, a.bitmap.width, a.bitmap.height), (b.x, b.y, b.bitmap.width, b.bitmap.height));
        // Нет фразы — пусто.
        assert!(subtitle_art(&layer(SubtitleMode::Reveal), &t, 5, 1, canvas).above.is_none());
    }
}
