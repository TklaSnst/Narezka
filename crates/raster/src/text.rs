//! Текст и капсула: раскладка — cosmic-text, контуры глифов — tiny-skia (заливка, обводка, плашки акцента).
//! Глифы рисуются векторно без хинтинга: результат не зависит от того, кто его потом масштабирует.

use cosmic_text::{Attrs, Buffer, CacheKeyFlags, Family, FontSystem, Metrics, Shaping, Weight, Wrap};
use sc_core::{AccentMode, Align, Color, VAlign};
use swash::zeno::Command;
use tiny_skia::{FillRule, LineJoin, PathBuilder, Pixmap, Stroke, Transform};

use crate::fonts::{FONTS, Fonts, split_font_name};

/// Межстрочный интервал в долях кегля.
const LINE_HEIGHT: f32 = 1.2;
/// Текст, не влезающий в прямоугольник, уменьшается до этой доли кегля, дальше — вылезает.
const MIN_FIT: f32 = 0.4;

/// Разметка акцента: `*так*` → куски `(текст, акцент)`. Непарная `*` остаётся в тексте.
pub fn parse_accent(s: &str) -> Vec<(String, bool)> {
    let mut parts: Vec<String> = s.split('*').map(String::from).collect();
    if parts.len().is_multiple_of(2) {
        let last = parts.pop().unwrap();
        parts.last_mut().unwrap().push_str(&format!("*{last}"));
    }
    parts.into_iter().enumerate().filter(|(_, p)| !p.is_empty()).map(|(i, p)| (p, i % 2 == 1)).collect()
}

/// Оформление текста в пикселях холста.
#[derive(Debug, Clone, Copy)]
pub struct TextStyle<'a> {
    pub font: &'a str,
    /// Кегль, px.
    pub size: f32,
    pub color: Color,
    pub accent_color: Color,
    pub accent_mode: AccentMode,
    /// Обводка: толщина px и цвет.
    pub outline: Option<(f32, Color)>,
    pub align: Align,
    pub valign: VAlign,
    /// Переносить по словам; без переноса — одна строка (переводы строк в тексте остаются), не влезает — ужимается.
    pub wrap: bool,
}

/// Как рисуется кусок текста: цвет глифов и, если есть, плашка под ним.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpanPaint {
    pub color: Color,
    pub plate: Option<Color>,
}

/// Рисует текст в прямоугольник `(x, y, w, h)` пиксмапа: перенос по словам (если `wrap`), выравнивание,
/// при нехватке места — уменьшение кегля. Возвращает итоговый кегль.
pub fn draw_text(pm: &mut Pixmap, rect: (f32, f32, f32, f32), content: &str, style: &TextStyle) -> f32 {
    let spans: Vec<(String, usize)> = parse_accent(content).into_iter().map(|(t, a)| (t, usize::from(a))).collect();
    let accent = match style.accent_mode {
        AccentMode::Color => SpanPaint { color: style.accent_color, plate: None },
        AccentMode::Plate => SpanPaint { color: style.color, plate: Some(style.accent_color) },
    };
    draw_spans(pm, rect, &spans, &[SpanPaint { color: style.color, plate: None }, accent], None, style)
}

/// Рисует куски текста `(текст, индекс в paints)` — каждый своим цветом и плашкой; `line_plate` — подложка
/// под каждой строкой. Соседние глифы одного куска делят одну плашку. Обводка куска — прозрачная настолько же,
/// насколько его цвет (скрытые слова не оставляют контура). Возвращает итоговый кегль.
pub fn draw_spans(
    pm: &mut Pixmap,
    (x, y, w, h): (f32, f32, f32, f32),
    spans: &[(String, usize)],
    paints: &[SpanPaint],
    line_plate: Option<Color>,
    style: &TextStyle,
) -> f32 {
    if spans.iter().all(|(t, _)| t.trim().is_empty()) {
        return style.size;
    }
    let mut fonts = FONTS.lock().unwrap_or_else(|e| e.into_inner());
    let Fonts { system, cache } = &mut *fonts;
    let (family, weight) = resolve_font(system, style.font);
    let attrs = Attrs::new().family(family).weight(weight).cache_key_flags(CacheKeyFlags::DISABLE_HINTING);
    let align = Some(match style.align {
        Align::Left => cosmic_text::Align::Left,
        Align::Center => cosmic_text::Align::Center,
        Align::Right => cosmic_text::Align::Right,
    });

    let mut size = style.size;
    let (buf, lines) = loop {
        let mut buf = Buffer::new(system, Metrics::new(size, size * LINE_HEIGHT));
        buf.set_wrap(if style.wrap { Wrap::Word } else { Wrap::None });
        buf.set_size(Some(w), None);
        let rich = spans.iter().map(|(t, m)| (t.as_str(), attrs.clone().metadata(*m)));
        buf.set_rich_text(rich, &attrs, Shaping::Advanced, align);
        buf.shape_until_scroll(system, false);
        let (lines, width) = buf.layout_runs().fold((0, 0f32), |(n, mw), r| (n + 1, mw.max(r.line_w)));
        let fits = lines as f32 * size * LINE_HEIGHT <= h + 0.5 && width <= w + 0.5;
        if fits || size * 0.92 < style.size * MIN_FIT {
            break (buf, lines);
        }
        size *= 0.92;
    };

    let total = lines as f32 * size * LINE_HEIGHT;
    let y0 = y + match style.valign {
        VAlign::Top => 0.0,
        VAlign::Center => (h - total) / 2.0,
        VAlign::Bottom => h - total,
    };
    let mut paths: Vec<PathBuilder> = (0..paints.len()).map(|_| PathBuilder::new()).collect();
    // (левый край, правый край, кусок, базовая линия)
    let mut plates: Vec<(f32, f32, usize, f32)> = Vec::new();
    let mut line_plates = Vec::new();
    for run in buf.layout_runs() {
        let baseline = y0 + run.line_y;
        let mut plate: Option<(f32, f32, usize)> = None;
        let mut extent: Option<(f32, f32)> = None;
        for g in run.glyphs {
            let m = g.metadata.min(paints.len() - 1);
            let visible = !run.text[g.start..g.end].trim().is_empty();
            if visible {
                let (l, r) = extent.get_or_insert((g.x, g.x + g.w));
                *l = l.min(g.x);
                *r = r.max(g.x + g.w);
            }
            match &mut plate {
                Some((l, r, pi)) if *pi == m => {
                    *l = l.min(g.x);
                    *r = r.max(g.x + g.w);
                }
                _ => {
                    if let Some((l, r, pi)) = plate.take() {
                        plates.push((l, r, pi, baseline));
                    }
                    if paints[m].plate.is_some() {
                        plate = Some((g.x, g.x + g.w, m));
                    }
                }
            }
            let key = g.physical((0.0, 0.0), 1.0).cache_key;
            let Some(cmds) = cache.get_outline_commands(system, key) else { continue };
            let gx = x + g.x + g.font_size * g.x_offset;
            let gy = baseline + g.y - g.font_size * g.y_offset;
            append_outline(&mut paths[m], cmds, gx, gy);
        }
        if let Some((l, r, pi)) = plate {
            plates.push((l, r, pi, baseline));
        }
        if let Some((l, r)) = extent {
            line_plates.push((l, r, baseline));
        }
    }

    let paint = |c: Color| {
        let mut p = tiny_skia::Paint::default();
        p.set_color_rgba8(c.0[0], c.0[1], c.0[2], c.0[3]);
        p.anti_alias = true;
        p
    };
    let fill_rounded = |pm: &mut Pixmap, l: f32, t: f32, r: f32, b: f32, radius: f32, c: Color| {
        if let Some(path) = tiny_skia::Rect::from_ltrb(l, t, r, b).and_then(|r| rounded_rect_path(r, radius)) {
            pm.fill_path(&path, &paint(c), FillRule::Winding, Transform::identity(), None);
        }
    };
    if let Some(c) = line_plate {
        // Подложка строки — по высоте строки, с запасом по бокам.
        let pad = size * 0.3;
        for (l, r, baseline) in line_plates {
            fill_rounded(pm, x + l - pad, baseline - size * 1.0, x + r + pad, baseline + size * 0.3, size * 0.2, c);
        }
    }
    // Плашка — по высоте прописных, с запасом по бокам.
    let pad = size * 0.22;
    for (l, r, m, baseline) in plates {
        if let Some(c) = paints[m].plate {
            fill_rounded(pm, x + l - pad, baseline - size * 0.95, x + r + pad, baseline + size * 0.32, size * 0.15, c);
        }
    }
    let paths: Vec<Option<tiny_skia::Path>> = paths.into_iter().map(PathBuilder::finish).collect();
    if let Some((width, color)) = style.outline {
        let stroke = Stroke { width: width * 2.0, line_join: LineJoin::Round, ..Default::default() };
        for (path, p) in paths.iter().zip(paints) {
            let a = (color.0[3] as u32 * p.color.0[3] as u32 / 255) as u8;
            if let Some(path) = path
                && a > 0
            {
                let c = Color([color.0[0], color.0[1], color.0[2], a]);
                pm.stroke_path(path, &paint(c), &stroke, Transform::identity(), None);
            }
        }
    }
    for (path, p) in paths.iter().zip(paints) {
        if let Some(path) = path
            && p.color.0[3] > 0
        {
            pm.fill_path(path, &paint(p.color), FillRule::Winding, Transform::identity(), None);
        }
    }
    size
}

/// Семейство и насыщенность по имени шрифта: «DejaVu Sans», «Montserrat ExtraBold», «sans-serif Bold».
fn resolve_font<'a>(system: &FontSystem, name: &'a str) -> (Family<'a>, Weight) {
    let known = |n: &str| system.db().faces().any(|f| f.families.iter().any(|(fam, _)| fam.eq_ignore_ascii_case(n)));
    let (family, weight) = split_font_name(name, known);
    let family = match family.to_ascii_lowercase().as_str() {
        "sans-serif" | "sans" => Family::SansSerif,
        "serif" => Family::Serif,
        "monospace" | "mono" => Family::Monospace,
        _ => Family::Name(family),
    };
    (family, Weight(weight))
}

/// Контур глифа (единицы — пиксели, ось y вверх) с началом в точке `(gx, gy)` холста.
fn append_outline(pb: &mut PathBuilder, cmds: &[Command], gx: f32, gy: f32) {
    let p = |v: swash::zeno::Vector| (gx + v.x, gy - v.y);
    for c in cmds {
        match *c {
            Command::MoveTo(a) => {
                let (x, y) = p(a);
                pb.move_to(x, y);
            }
            Command::LineTo(a) => {
                let (x, y) = p(a);
                pb.line_to(x, y);
            }
            Command::QuadTo(c, a) => {
                let ((cx, cy), (x, y)) = (p(c), p(a));
                pb.quad_to(cx, cy, x, y);
            }
            Command::CurveTo(c1, c2, a) => {
                let ((x1, y1), (x2, y2), (x, y)) = (p(c1), p(c2), p(a));
                pb.cubic_to(x1, y1, x2, y2, x, y);
            }
            Command::Close => pb.close(),
        }
    }
}

/// Скруглённый прямоугольник (дуги — кубические кривые).
pub fn rounded_rect_path(r: tiny_skia::Rect, radius: f32) -> Option<tiny_skia::Path> {
    let rad = radius.min(r.width() / 2.0).min(r.height() / 2.0).max(0.0);
    if rad <= 0.0 {
        return Some(PathBuilder::from_rect(r));
    }
    // Контрольные точки четверти окружности.
    let k = rad * 0.552_284_8;
    let (l, t, rr, b) = (r.left(), r.top(), r.right(), r.bottom());
    let mut pb = PathBuilder::new();
    pb.move_to(l + rad, t);
    pb.line_to(rr - rad, t);
    pb.cubic_to(rr - rad + k, t, rr, t + rad - k, rr, t + rad);
    pb.line_to(rr, b - rad);
    pb.cubic_to(rr, b - rad + k, rr - rad + k, b, rr - rad, b);
    pb.line_to(l + rad, b);
    pb.cubic_to(l + rad - k, b, l, b - rad + k, l, b - rad);
    pb.line_to(l, t + rad);
    pb.cubic_to(l, t + rad - k, l + rad - k, t, l + rad, t);
    pb.close();
    pb.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accent_markup() {
        let s = |v: &[(&str, bool)]| v.iter().map(|(t, a)| (t.to_string(), *a)).collect::<Vec<_>>();
        assert_eq!(parse_accent("ТОП *языков* тут"), s(&[("ТОП ", false), ("языков", true), (" тут", false)]));
        assert_eq!(parse_accent("*алерт*"), s(&[("алерт", true)]));
        assert_eq!(parse_accent("5 * 3"), s(&[("5 * 3", false)]));
        assert_eq!(parse_accent("*a* b * c"), s(&[("a", true), (" b * c", false)]));
        assert_eq!(parse_accent(""), s(&[]));
    }

    #[test]
    fn single_line_shrinks_instead_of_wrapping() {
        let draw = |wrap: bool| {
            let mut pm = Pixmap::new(400, 60).unwrap();
            let style = TextStyle {
                font: "Press Start 2P",
                size: 30.0,
                color: Color::WHITE,
                accent_color: Color::WHITE,
                accent_mode: AccentMode::Color,
                outline: None,
                align: Align::Center,
                valign: VAlign::Center,
                wrap,
            };
            let size = draw_text(&mut pm, (0.0, 0.0, 400.0, 60.0), "Часть 12 · Длинная подпись", &style);
            let rows: Vec<usize> = pm.data().chunks(4).enumerate().filter(|(_, p)| p[3] > 0).map(|(i, _)| i / 400).collect();
            (size, rows.iter().max().unwrap() - rows.iter().min().unwrap())
        };
        let (wrapped, wrapped_h) = draw(true);
        let (single, single_h) = draw(false);
        assert!(single < wrapped, "одна строка мельче: {single} vs {wrapped}");
        assert!((single_h as f32) < single * 1.2, "одна строка по высоте: {single_h} при кегле {single}");
        assert!(wrapped_h as f32 > wrapped * 1.2, "с переносом — две строки: {wrapped_h} при кегле {wrapped}");
    }
}
