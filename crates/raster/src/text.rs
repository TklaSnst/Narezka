//! Текст и капсула: раскладка — cosmic-text, контуры глифов — tiny-skia (заливка, обводка, плашки акцента).
//! Глифы рисуются векторно без хинтинга: результат не зависит от того, кто его потом масштабирует.

use std::sync::{LazyLock, Mutex};

use cosmic_text::{Attrs, Buffer, CacheKeyFlags, Family, FontSystem, Metrics, Shaping, SwashCache, Weight, Wrap};
use sc_core::{AccentMode, Align, Color, VAlign};
use swash::zeno::Command;
use tiny_skia::{FillRule, LineJoin, PathBuilder, Pixmap, Stroke, Transform};

/// Межстрочный интервал в долях кегля.
const LINE_HEIGHT: f32 = 1.2;
/// Текст, не влезающий в прямоугольник, уменьшается до этой доли кегля, дальше — вылезает.
const MIN_FIT: f32 = 0.4;

struct Fonts {
    system: FontSystem,
    cache: SwashCache,
}

static FONTS: LazyLock<Mutex<Fonts>> =
    LazyLock::new(|| Mutex::new(Fonts { system: FontSystem::new(), cache: SwashCache::new() }));

/// Загружает системные шрифты (первый раз — сотни миллисекунд); вызывать заранее в фоне.
pub fn warm_up() {
    LazyLock::force(&FONTS);
}

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
}

/// Рисует текст в прямоугольник `(x, y, w, h)` пиксмапа: перенос по словам, выравнивание,
/// при нехватке места — уменьшение кегля. Возвращает итоговый кегль.
pub fn draw_text(pm: &mut Pixmap, (x, y, w, h): (f32, f32, f32, f32), content: &str, style: &TextStyle) -> f32 {
    let spans = parse_accent(content);
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
        buf.set_wrap(Wrap::Word);
        buf.set_size(Some(w), None);
        let rich = spans.iter().map(|(t, accent)| (t.as_str(), attrs.clone().metadata(usize::from(*accent))));
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
    let mut plain = PathBuilder::new();
    let mut accent = PathBuilder::new();
    let mut plates = Vec::new();
    for run in buf.layout_runs() {
        let baseline = y0 + run.line_y;
        let mut plate: Option<(f32, f32)> = None;
        for g in run.glyphs {
            let is_accent = g.metadata == 1;
            if is_accent {
                let (l, r) = plate.get_or_insert((g.x, g.x + g.w));
                *l = l.min(g.x);
                *r = r.max(g.x + g.w);
            } else if let Some(p) = plate.take() {
                plates.push((p, baseline));
            }
            let key = g.physical((0.0, 0.0), 1.0).cache_key;
            let Some(cmds) = cache.get_outline_commands(system, key) else { continue };
            let gx = x + g.x + g.font_size * g.x_offset;
            let gy = baseline + g.y - g.font_size * g.y_offset;
            let pb = if is_accent { &mut accent } else { &mut plain };
            append_outline(pb, cmds, gx, gy);
        }
        if let Some(p) = plate {
            plates.push((p, baseline));
        }
    }

    let paint = |c: Color| {
        let mut p = tiny_skia::Paint::default();
        p.set_color_rgba8(c.0[0], c.0[1], c.0[2], c.0[3]);
        p.anti_alias = true;
        p
    };
    let plain = plain.finish();
    let accent = accent.finish();
    if style.accent_mode == AccentMode::Plate {
        // Плашка — по высоте прописных, с запасом по бокам.
        let pad = size * 0.22;
        for ((l, r), baseline) in plates {
            let rect = tiny_skia::Rect::from_ltrb(x + l - pad, baseline - size * 0.95, x + r + pad, baseline + size * 0.32);
            if let Some(path) = rect.and_then(|r| rounded_rect_path(r, size * 0.15)) {
                pm.fill_path(&path, &paint(style.accent_color), FillRule::Winding, Transform::identity(), None);
            }
        }
    }
    if let Some((width, color)) = style.outline {
        let stroke = Stroke { width: width * 2.0, line_join: LineJoin::Round, ..Default::default() };
        for path in plain.iter().chain(accent.iter()) {
            pm.stroke_path(path, &paint(color), &stroke, Transform::identity(), None);
        }
    }
    let accent_fill = match style.accent_mode {
        AccentMode::Color => style.accent_color,
        AccentMode::Plate => style.color,
    };
    for (path, color) in [(&plain, style.color), (&accent, accent_fill)] {
        if let Some(path) = path {
            pm.fill_path(path, &paint(color), FillRule::Winding, Transform::identity(), None);
        }
    }
    size
}

/// Семейство и насыщенность по имени шрифта: «DejaVu Sans», «DejaVu Sans Bold», «sans-serif».
/// Начертание в конце имени отделяется, если семейства с полным именем нет.
fn resolve_font<'a>(system: &FontSystem, name: &'a str) -> (Family<'a>, Weight) {
    let generic = match name {
        "sans-serif" | "sans" => Some(Family::SansSerif),
        "serif" => Some(Family::Serif),
        "monospace" | "mono" => Some(Family::Monospace),
        _ => None,
    };
    if let Some(f) = generic {
        return (f, Weight::NORMAL);
    }
    let known = |n: &str| system.db().faces().any(|f| f.families.iter().any(|(fam, _)| fam.eq_ignore_ascii_case(n)));
    if !known(name)
        && let Some((family, last)) = name.rsplit_once(' ')
    {
        let weight = match last.to_ascii_lowercase().as_str() {
            "thin" => Some(Weight::THIN),
            "extralight" => Some(Weight::EXTRA_LIGHT),
            "light" => Some(Weight::LIGHT),
            "regular" => Some(Weight::NORMAL),
            "medium" => Some(Weight::MEDIUM),
            "semibold" => Some(Weight::SEMIBOLD),
            "bold" => Some(Weight::BOLD),
            "extrabold" => Some(Weight::EXTRA_BOLD),
            "black" => Some(Weight::BLACK),
            _ => None,
        };
        if let Some(w) = weight {
            return (Family::Name(family), w);
        }
    }
    (Family::Name(name), Weight::NORMAL)
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
}
