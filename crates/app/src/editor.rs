//! Геометрия редактора шаблона мышью — без интерфейса, чтобы проверять тестами: попадание в слой,
//! перемещение и ручки масштаба с привязками (края и центр холста, другие слои, сетка), кроп видео.
//! Всё в долях холста; пороги привязки — тоже в долях (UI пересчитывает из пикселей экрана).

use sc_core::{Layer, Rect};

/// Меньше этого (в долях стороны холста) слой не сжимается.
pub const MIN_SIZE: f32 = 0.02;
/// Кроп видео — не меньше этой доли исходника по ширине.
pub const MIN_CROP: f32 = 0.05;

/// За что тянут выделенный слой.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handle {
    Move,
    N,
    S,
    E,
    W,
    NE,
    NW,
    SE,
    SW,
}

impl Handle {
    pub const RESIZE: [Handle; 8] = [Handle::NW, Handle::N, Handle::NE, Handle::E, Handle::SE, Handle::S, Handle::SW, Handle::W];

    /// Точка ручки на прямоугольнике в его долях: (0, 0) — левый верхний угол.
    pub fn anchor(self) -> (f32, f32) {
        match self {
            Handle::Move => (0.5, 0.5),
            Handle::N => (0.5, 0.0),
            Handle::S => (0.5, 1.0),
            Handle::E => (1.0, 0.5),
            Handle::W => (0.0, 0.5),
            Handle::NE => (1.0, 0.0),
            Handle::NW => (0.0, 0.0),
            Handle::SE => (1.0, 1.0),
            Handle::SW => (0.0, 1.0),
        }
    }

    /// Какие края двигает: (левый, правый, верхний, нижний).
    fn edges(self) -> (bool, bool, bool, bool) {
        let (ax, ay) = self.anchor();
        (ax == 0.0, ax == 1.0, ay == 0.0, ay == 1.0)
    }

    pub fn is_corner(self) -> bool {
        matches!(self, Handle::NE | Handle::NW | Handle::SE | Handle::SW)
    }
}

/// К чему липнуть: координаты вертикальных (`xs`) и горизонтальных (`ys`) линий.
#[derive(Debug, Clone, Default)]
pub struct Targets {
    pub xs: Vec<f32>,
    pub ys: Vec<f32>,
}

impl Targets {
    /// Края и центр холста и всех слоёв, кроме `skip` (того, что двигают).
    pub fn new(layers: &[Layer], skip: usize) -> Self {
        let mut t = Targets { xs: vec![0.0, 0.5, 1.0], ys: vec![0.0, 0.5, 1.0] };
        for (i, l) in layers.iter().enumerate() {
            if i == skip {
                continue;
            }
            let r = l.rect();
            t.xs.extend([r.x, r.x + r.w / 2.0, r.right()]);
            t.ys.extend([r.y, r.y + r.h / 2.0, r.bottom()]);
        }
        t
    }
}

/// Настройки привязки для одного перетаскивания.
#[derive(Debug, Clone, Copy)]
pub struct SnapOpts {
    /// Порог в долях холста по x и по y; `None` — привязка выключена.
    pub threshold: Option<(f32, f32)>,
    /// Шаг сетки по x и по y; `None` — без сетки.
    pub grid: Option<(f32, f32)>,
}

/// Линии, к которым прилип слой (рисуются поверх превью).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Guides {
    pub xs: Vec<f32>,
    pub ys: Vec<f32>,
}

/// Ближайшая линия к одной из точек `points`: (сдвиг, линия).
fn nearest(points: &[f32], lines: &[f32], threshold: f32) -> Option<(f32, f32)> {
    let mut best: Option<(f32, f32)> = None;
    for &p in points {
        for &l in lines {
            let d = l - p;
            if d.abs() <= threshold && best.is_none_or(|(b, _)| d.abs() < b.abs()) {
                best = Some((d, l));
            }
        }
    }
    best
}

/// Сдвиг для одной оси: сначала линии слоёв и холста, потом сетка (по первой точке — краю слоя).
fn snap_axis(points: &[f32], lines: &[f32], threshold: Option<f32>, grid: Option<f32>, guides: &mut Vec<f32>) -> f32 {
    if let Some(th) = threshold
        && let Some((d, line)) = nearest(points, lines, th)
    {
        guides.push(line);
        return d;
    }
    match grid {
        Some(step) if step > 0.0 => (points[0] / step).round() * step - points[0],
        _ => 0.0,
    }
}

/// Слой `start` потянули за `handle` на `delta` (доли холста). `aspect` — ширина/высота холста в пикселях;
/// `keep_aspect` — угловые ручки сохраняют пропорции слоя на экране. Результат всегда внутри холста.
pub fn drag(start: Rect, handle: Handle, delta: (f32, f32), keep_aspect: bool, aspect: f32, targets: &Targets, opts: SnapOpts) -> (Rect, Guides) {
    let mut guides = Guides::default();
    let th = opts.threshold;
    let (gx, gy) = (opts.grid.map(|g| g.0), opts.grid.map(|g| g.1));
    if handle == Handle::Move {
        let mut r = Rect::new(start.x + delta.0, start.y + delta.1, start.w, start.h);
        r.x += snap_axis(&[r.x, r.x + r.w / 2.0, r.right()], &targets.xs, th.map(|t| t.0), gx, &mut guides.xs);
        r.y += snap_axis(&[r.y, r.y + r.h / 2.0, r.bottom()], &targets.ys, th.map(|t| t.1), gy, &mut guides.ys);
        r.x = r.x.clamp(0.0, 1.0 - r.w);
        r.y = r.y.clamp(0.0, 1.0 - r.h);
        // Показываем все линии, с которыми слой совпал после сдвига (верх и низ могут прилипнуть сразу).
        let on = |points: [f32; 3], lines: &[f32]| -> Vec<f32> {
            lines.iter().copied().filter(|&l| points.iter().any(|&p| (p - l).abs() < 1e-4)).collect()
        };
        if th.is_some() {
            guides.xs = on([r.x, r.x + r.w / 2.0, r.right()], &targets.xs);
            guides.ys = on([r.y, r.y + r.h / 2.0, r.bottom()], &targets.ys);
        }
        return (r, guides);
    }

    let (left, right, top, bottom) = handle.edges();
    let (mut x0, mut x1, mut y0, mut y1) = (start.x, start.right(), start.y, start.bottom());
    let edge = |v: &mut f32, moving: bool, d: f32, lines: &[f32], th: Option<f32>, grid: Option<f32>, out: &mut Vec<f32>| {
        if moving {
            *v += d;
            *v += snap_axis(&[*v], lines, th, grid, out);
        }
    };
    edge(&mut x0, left, delta.0, &targets.xs, th.map(|t| t.0), gx, &mut guides.xs);
    edge(&mut x1, right, delta.0, &targets.xs, th.map(|t| t.0), gx, &mut guides.xs);
    edge(&mut y0, top, delta.1, &targets.ys, th.map(|t| t.1), gy, &mut guides.ys);
    edge(&mut y1, bottom, delta.1, &targets.ys, th.map(|t| t.1), gy, &mut guides.ys);
    // Края не переходят друг через друга и не выходят за холст.
    if left {
        x0 = x0.clamp(0.0, x1 - MIN_SIZE);
    }
    if right {
        x1 = x1.clamp(x0 + MIN_SIZE, 1.0);
    }
    if top {
        y0 = y0.clamp(0.0, y1 - MIN_SIZE);
    }
    if bottom {
        y1 = y1.clamp(y0 + MIN_SIZE, 1.0);
    }
    let mut r = Rect::new(x0, y0, x1 - x0, y1 - y0);
    if keep_aspect && handle.is_corner() && start.w > 0.0 && start.h > 0.0 {
        r = keep_ratio(start, r, handle, aspect);
        guides = Guides::default();
    }
    (r, guides)
}

/// Подогнать `r` под пропорции `start` (на экране), не двигая противоположный ручке угол.
fn keep_ratio(start: Rect, r: Rect, handle: Handle, aspect: f32) -> Rect {
    let ratio = start.w * aspect / start.h; // ширина/высота в пикселях
    let (ax, ay) = handle.anchor();
    // Неподвижный угол.
    let fx = if ax == 0.0 { start.right() } else { start.x };
    let fy = if ay == 0.0 { start.bottom() } else { start.y };
    // Сколько места до края холста от неподвижного угла.
    let max_w = if ax == 0.0 { fx } else { 1.0 - fx };
    let max_h = if ay == 0.0 { fy } else { 1.0 - fy };
    // Ведёт та сторона, что изменилась сильнее.
    let mut w = r.w.max(r.h * ratio / aspect);
    let mut h = w * aspect / ratio;
    let k = (max_w / w).min(max_h / h).min(1.0);
    w *= k;
    h *= k;
    let min_k = (MIN_SIZE / w).max(MIN_SIZE / h).max(1.0);
    if min_k * w <= max_w && min_k * h <= max_h {
        w *= min_k;
        h *= min_k;
    }
    let x = if ax == 0.0 { fx - w } else { fx };
    let y = if ay == 0.0 { fy - h } else { fy };
    Rect::new(x, y, w, h)
}

/// Кроп `eff` (то, что сейчас видно, в долях исходника) сдвинут так, что картинка уехала на `delta`
/// долей слоя (тянут мышью вправо — видно то, что было левее). Остаётся внутри исходника.
pub fn pan_crop(eff: Rect, delta: (f32, f32)) -> Rect {
    let x = (eff.x - delta.0 * eff.w).clamp(0.0, (1.0 - eff.w).max(0.0));
    let y = (eff.y - delta.1 * eff.h).clamp(0.0, (1.0 - eff.h).max(0.0));
    Rect::new(x, y, eff.w, eff.h)
}

/// Приблизить кроп в `k` раз (`k` < 1 — отдалить) вокруг точки `at` (доли слоя), сохраняя его пропорции;
/// не больше исходника и не меньше [`MIN_CROP`].
pub fn zoom_crop(eff: Rect, k: f32, at: (f32, f32)) -> Rect {
    let mut w = eff.w / k;
    let mut h = eff.h / k;
    let fit = (1.0 / w).min(1.0 / h).min(1.0);
    w *= fit;
    h *= fit;
    let grow = (MIN_CROP / w).max(1.0);
    w *= grow;
    h *= grow;
    let px = eff.x + at.0 * eff.w;
    let py = eff.y + at.1 * eff.h;
    let x = (px - at.0 * w).clamp(0.0, (1.0 - w).max(0.0));
    let y = (py - at.1 * h).clamp(0.0, (1.0 - h).max(0.0));
    Rect::new(x, y, w, h)
}

/// Совпадают с точностью float-формул кропа.
pub fn same_rect(a: Rect, b: Rect) -> bool {
    [a.x - b.x, a.y - b.y, a.w - b.w, a.h - b.h].iter().all(|d| d.abs() < 1e-4)
}

/// Где на холсте был бы весь кадр исходника, если его участок `eff` показан в `dest`.
pub fn source_frame(dest: Rect, eff: Rect) -> Rect {
    let w = dest.w / eff.w;
    let h = dest.h / eff.h;
    Rect::new(dest.x - eff.x * w, dest.y - eff.y * h, w, h)
}

/// Верхний слой под точкой; выделенный — в приоритете, если точка внутри него (чтобы его можно было тянуть,
/// даже когда сверху лежит другой слой).
pub fn hit(layers: &[Layer], p: (f32, f32), selected: Option<usize>) -> Option<usize> {
    let inside = |i: usize| {
        let r = layers[i].rect();
        p.0 >= r.x && p.0 <= r.right() && p.1 >= r.y && p.1 <= r.bottom()
    };
    if let Some(s) = selected.filter(|&s| s < layers.len() && inside(s)) {
        return Some(s);
    }
    (0..layers.len()).rev().find(|&i| inside(i))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NO_SNAP: SnapOpts = SnapOpts { threshold: None, grid: None };
    const ASPECT: f32 = 1080.0 / 1920.0;

    use super::same_rect as close;

    fn banner(r: Rect) -> Layer {
        Layer::BannerSlot { rect: r }
    }

    #[test]
    fn move_clamps_to_canvas() {
        let s = Rect::new(0.1, 0.1, 0.4, 0.2);
        let (r, _) = drag(s, Handle::Move, (0.2, 0.05), false, ASPECT, &Targets::default(), NO_SNAP);
        assert!(close(r, Rect::new(0.3, 0.15, 0.4, 0.2)));
        let (r, _) = drag(s, Handle::Move, (-0.5, 2.0), false, ASPECT, &Targets::default(), NO_SNAP);
        assert!(close(r, Rect::new(0.0, 0.8, 0.4, 0.2)), "{r:?}");
    }

    #[test]
    fn move_snaps_center_and_edges() {
        let t = Targets::new(&[banner(Rect::new(0.1, 0.6, 0.3, 0.1))], usize::MAX);
        let opts = SnapOpts { threshold: Some((0.01, 0.01)), grid: None };
        // центр 0.49 → 0.5 (центр холста)
        let s = Rect::new(0.2, 0.3, 0.585, 0.1);
        let (r, g) = drag(s, Handle::Move, (0.0, 0.0), false, ASPECT, &t, opts);
        assert!((r.x + r.w / 2.0 - 0.5).abs() < 1e-5, "{r:?}");
        assert_eq!(g.xs, vec![0.5]);
        assert!(g.ys.is_empty());
        // верх, центр и низ слоя — у верха, центра и низа другого слоя той же высоты
        let (r, g) = drag(Rect::new(0.5, 0.595, 0.2, 0.1), Handle::Move, (0.0, 0.0), false, ASPECT, &t, opts);
        assert!((r.y - 0.6).abs() < 1e-5);
        assert_eq!(g.ys.len(), 3, "{g:?}");
        // без привязки — как было
        let (r, g) = drag(Rect::new(0.5, 0.595, 0.2, 0.1), Handle::Move, (0.0, 0.0), false, ASPECT, &t, NO_SNAP);
        assert!((r.y - 0.595).abs() < 1e-6 && g == Guides::default());
    }

    #[test]
    fn grid_when_no_line_near() {
        let opts = SnapOpts { threshold: Some((0.001, 0.001)), grid: Some((0.1, 0.05)) };
        let (r, _) = drag(Rect::new(0.13, 0.27, 0.2, 0.1), Handle::Move, (0.0, 0.0), false, ASPECT, &Targets::default(), opts);
        assert!(close(r, Rect::new(0.1, 0.25, 0.2, 0.1)), "{r:?}");
    }

    #[test]
    fn resize_edges_and_limits() {
        let s = Rect::new(0.2, 0.2, 0.4, 0.2);
        let (r, _) = drag(s, Handle::E, (0.1, 0.3), false, ASPECT, &Targets::default(), NO_SNAP);
        assert!(close(r, Rect::new(0.2, 0.2, 0.5, 0.2)), "y не меняется: {r:?}");
        let (r, _) = drag(s, Handle::NW, (-0.5, -0.5), false, ASPECT, &Targets::default(), NO_SNAP);
        assert!(close(r, Rect::new(0.0, 0.0, 0.6, 0.4)), "за холст не выходит: {r:?}");
        let (r, _) = drag(s, Handle::W, (0.9, 0.0), false, ASPECT, &Targets::default(), NO_SNAP);
        assert!((r.w - MIN_SIZE).abs() < 1e-5 && (r.right() - 0.6).abs() < 1e-5, "{r:?}");
    }

    #[test]
    fn corner_keeps_aspect() {
        let s = Rect::new(0.2, 0.2, 0.4, 0.1);
        let ratio = |r: Rect| r.w * 1080.0 / (r.h * 1920.0);
        let (r, _) = drag(s, Handle::SE, (0.2, 0.0), true, ASPECT, &Targets::default(), NO_SNAP);
        assert!((ratio(r) - ratio(s)).abs() < 1e-3 && (r.x - 0.2).abs() < 1e-6 && (r.w - 0.6).abs() < 1e-5, "{r:?}");
        // упирается в край холста — уменьшается целиком, угол на месте
        let (r, _) = drag(s, Handle::NW, (-0.5, -0.5), true, ASPECT, &Targets::default(), NO_SNAP);
        assert!((ratio(r) - ratio(s)).abs() < 1e-3, "{r:?}");
        assert!((r.right() - 0.6).abs() < 1e-5 && (r.bottom() - 0.3).abs() < 1e-5 && r.x >= -1e-6 && r.y >= -1e-6, "{r:?}");
    }

    #[test]
    fn crop_pan_and_zoom() {
        let eff = Rect::new(0.3, 0.0, 0.3, 1.0);
        // тянем картинку вправо на половину слоя — кроп уходит влево на половину своей ширины
        assert!(close(pan_crop(eff, (0.5, 0.0)), Rect::new(0.15, 0.0, 0.3, 1.0)));
        assert!(close(pan_crop(eff, (5.0, 0.3)), Rect::new(0.0, 0.0, 0.3, 1.0)));
        // приблизить вдвое вокруг левого верхнего угла
        assert!(close(zoom_crop(eff, 2.0, (0.0, 0.0)), Rect::new(0.3, 0.0, 0.15, 0.5)));
        // отдалить — не больше исходника, пропорции те же
        let z = zoom_crop(Rect::new(0.3, 0.2, 0.3, 0.5), 0.1, (0.5, 0.5));
        assert!(close(z, Rect::new(0.15, 0.0, 0.6, 1.0)), "{z:?}");
        // кадр исходника на холсте
        let f = source_frame(Rect::new(0.0, 0.25, 1.0, 0.5), Rect::new(0.25, 0.0, 0.5, 1.0));
        assert!(close(f, Rect::new(-0.5, 0.25, 2.0, 0.5)), "{f:?}");
    }

    #[test]
    fn hit_prefers_selected_then_top() {
        let layers = [banner(Rect::FULL), banner(Rect::new(0.4, 0.4, 0.2, 0.2))];
        assert_eq!(hit(&layers, (0.5, 0.5), None), Some(1));
        assert_eq!(hit(&layers, (0.5, 0.5), Some(0)), Some(0));
        assert_eq!(hit(&layers, (0.1, 0.1), Some(1)), Some(0));
        assert_eq!(hit(&layers[1..], (0.1, 0.1), None), None);
    }
}
