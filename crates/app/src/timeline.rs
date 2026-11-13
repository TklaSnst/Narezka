//! Таймлайн исходника: шкала, шортсы цветными блоками (кандидаты и отклонённые — блёклые), зоны пропуска,
//! отметки in/out, курсор воспроизведения.
//! Колесо — масштаб вокруг указателя, горизонтальная прокрутка (Shift+колесо) — сдвиг,
//! клик/перетаскивание — перемотка, края выбранного шортса тянутся.

use eframe::egui::{self, Color32, CursorIcon, Pos2, Rect, Sense, Stroke, Vec2};
use sc_core::ShortId;

/// Самое крупное приближение: столько секунд на всю ширину.
const MIN_SPAN: f64 = 2.0;
/// Насколько близко к краю выбранного шортса (px) начинается растягивание.
const EDGE_GRAB: f32 = 6.0;
const RULER_H: f32 = 18.0;
const TRACK_H: f32 = 40.0;
/// Минимальное расстояние между подписями шкалы, px.
const TICK_MIN_PX: f64 = 80.0;
const TICK_STEPS: &[f64] =
    &[0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0, 900.0, 1800.0, 3600.0];

/// Как рисовать шортс.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Look {
    Accepted,
    Candidate,
    Rejected,
}

/// Шортс на таймлайне. Рисуются по порядку: последний — сверху (и ловит клик первым).
pub struct Block {
    pub id: ShortId,
    pub start: f64,
    pub end: f64,
    pub label: String,
    pub color: Color32,
    pub look: Look,
}

/// Зона пропуска на таймлайне.
pub struct Zone {
    pub start: f64,
    pub end: f64,
    /// Размечена пользователем (иначе — найдена анализом).
    pub manual: bool,
}

const CANDIDATE_COLOR: Color32 = Color32::from_rgb(95, 115, 150);
const CANDIDATE_EDGE: Color32 = Color32::from_rgb(140, 160, 200);
const REJECTED_COLOR: Color32 = Color32::from_gray(58);
const ZONE_AUTO: Color32 = Color32::from_rgb(220, 150, 60);
const ZONE_MANUAL: Color32 = Color32::from_rgb(220, 70, 70);

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Action {
    /// `exact = false` — во время перетаскивания (по ключевым кадрам, быстро).
    Seek { t: f64, exact: bool },
    Select(ShortId),
    Resize { id: ShortId, start: f64, end: f64 },
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Drag {
    Seek,
    Start { id: ShortId, end: f64 },
    End { id: ShortId, start: f64 },
}

/// Видимое окно таймлайна в секундах.
#[derive(Debug, Clone, PartialEq)]
pub struct View {
    pub start: f64,
    pub span: f64,
    pub duration: f64,
}

impl View {
    pub fn new(duration: f64) -> Self {
        let duration = duration.max(0.001);
        Self { start: 0.0, span: duration, duration }
    }

    fn min_span(&self) -> f64 {
        MIN_SPAN.min(self.duration)
    }

    fn clamp(&mut self) {
        self.span = self.span.clamp(self.min_span(), self.duration);
        self.start = self.start.clamp(0.0, self.duration - self.span);
    }

    /// Масштаб в `factor` раз (>1 — отдалить), точка `anchor` остаётся на месте.
    pub fn zoom(&mut self, factor: f64, anchor: f64) {
        let new_span = (self.span * factor).clamp(self.min_span(), self.duration);
        let k = (anchor - self.start) / self.span;
        self.start = anchor - k * new_span;
        self.span = new_span;
        self.clamp();
    }

    /// Показать весь ролик.
    pub fn fit(&mut self) {
        self.start = 0.0;
        self.span = self.duration;
    }

    pub fn pan(&mut self, dt: f64) {
        self.start += dt;
        self.clamp();
    }

    /// Если `t` ушло за край окна (воспроизведение), перелистнуть так, чтобы оно было у левого края.
    pub fn follow(&mut self, t: f64) {
        if t < self.start || t > self.start + self.span {
            self.start = t - self.span * 0.05;
            self.clamp();
        }
    }

    pub fn t_to_x(&self, t: f64, rect: Rect) -> f32 {
        rect.min.x + ((t - self.start) / self.span * rect.width() as f64) as f32
    }

    pub fn x_to_t(&self, x: f32, rect: Rect) -> f64 {
        (self.start + (x - rect.min.x) as f64 / rect.width() as f64 * self.span).clamp(0.0, self.duration)
    }

    /// Шаг подписей шкалы для ширины `width_px`.
    pub fn tick_step(&self, width_px: f32) -> f64 {
        let px_per_sec = width_px as f64 / self.span;
        TICK_STEPS.iter().copied().find(|s| s * px_per_sec >= TICK_MIN_PX).unwrap_or(7200.0)
    }
}

pub struct Timeline {
    pub view: View,
    drag: Option<Drag>,
}

impl Timeline {
    pub fn new(duration: f64) -> Self {
        Self { view: View::new(duration), drag: None }
    }

    pub fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        pos: f64,
        blocks: &[Block],
        zones: &[Zone],
        selected: Option<ShortId>,
        mark_in: Option<f64>,
        mark_out: Option<f64>,
    ) -> Vec<Action> {
        let mut actions = vec![];
        let (rect, resp) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), RULER_H + TRACK_H), Sense::click_and_drag());
        let ruler = Rect::from_min_size(rect.min, Vec2::new(rect.width(), RULER_H));
        let track = Rect::from_min_max(Pos2::new(rect.min.x, ruler.max.y), rect.max);
        let v = &mut self.view;

        // Масштаб и прокрутка.
        if resp.hovered() {
            let (scroll, zoom) = ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta()));
            if let Some(p) = resp.hover_pos() {
                let anchor = v.x_to_t(p.x, rect);
                if scroll.y != 0.0 {
                    v.zoom((-scroll.y as f64 * 0.004).exp(), anchor);
                }
                if zoom != 1.0 {
                    v.zoom(1.0 / zoom as f64, anchor);
                }
            }
            if scroll.x != 0.0 {
                v.pan(-scroll.x as f64 / rect.width() as f64 * v.span);
            }
        }

        // Что под указателем: край выбранного шортса или блок.
        let edge_at = |x: f32| -> Option<Drag> {
            let b = blocks.iter().find(|b| Some(b.id) == selected)?;
            let (x0, x1) = (v.t_to_x(b.start, rect), v.t_to_x(b.end, rect));
            if (x - x1).abs() <= EDGE_GRAB {
                Some(Drag::End { id: b.id, start: b.start })
            } else if (x - x0).abs() <= EDGE_GRAB {
                Some(Drag::Start { id: b.id, end: b.end })
            } else {
                None
            }
        };
        let block_at = |p: Pos2| -> Option<ShortId> {
            if !track.contains(p) {
                return None;
            }
            let t = v.x_to_t(p.x, rect);
            blocks.iter().rev().find(|b| b.start <= t && t <= b.end).map(|b| b.id)
        };

        if let Some(p) = resp.hover_pos()
            && track.contains(p)
            && (edge_at(p.x).is_some() || matches!(self.drag, Some(Drag::Start { .. } | Drag::End { .. })))
        {
            ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
        }

        if resp.drag_started() {
            let origin = ui.input(|i| i.pointer.press_origin()).unwrap_or(rect.center());
            self.drag = Some(if track.contains(origin) { edge_at(origin.x).unwrap_or(Drag::Seek) } else { Drag::Seek });
        }
        // В кадре отпускания interact_pointer_pos может уже не быть — берём последнюю позицию.
        let pointer = resp.interact_pointer_pos().or_else(|| ui.input(|i| i.pointer.latest_pos()));
        if let (Some(drag), Some(p)) = (self.drag, pointer) {
            let t = v.x_to_t(p.x, rect);
            let finished = resp.drag_stopped();
            match drag {
                Drag::Seek => actions.push(Action::Seek { t, exact: finished }),
                Drag::Start { id, end } => actions.push(Action::Resize { id, start: t.min(end), end }),
                Drag::End { id, start } => actions.push(Action::Resize { id, start, end: t.max(start) }),
            }
            if finished {
                self.drag = None;
            }
        } else if resp.drag_stopped() {
            self.drag = None;
        }
        if resp.clicked()
            && let Some(p) = resp.interact_pointer_pos()
        {
            match block_at(p) {
                Some(id) => actions.push(Action::Select(id)),
                None => actions.push(Action::Seek { t: v.x_to_t(p.x, rect), exact: true }),
            }
        }

        self.paint(ui, rect, ruler, track, pos, blocks, zones, selected, mark_in, mark_out);
        actions
    }

    #[allow(clippy::too_many_arguments)]
    fn paint(
        &self,
        ui: &egui::Ui,
        rect: Rect,
        ruler: Rect,
        track: Rect,
        pos: f64,
        blocks: &[Block],
        zones: &[Zone],
        selected: Option<ShortId>,
        mark_in: Option<f64>,
        mark_out: Option<f64>,
    ) {
        let v = &self.view;
        let painter = ui.painter_at(rect);
        let visuals = ui.visuals();
        painter.rect_filled(ruler, 0.0, visuals.faint_bg_color);
        painter.rect_filled(track, 2.0, visuals.extreme_bg_color);

        // Шкала.
        let step = v.tick_step(rect.width());
        let text_color = visuals.weak_text_color();
        let font = egui::FontId::monospace(11.0);
        let mut t = (v.start / step).floor() * step;
        while t <= v.start + v.span {
            let x = v.t_to_x(t, rect);
            painter.line_segment([Pos2::new(x, ruler.max.y - 6.0), Pos2::new(x, ruler.max.y)], Stroke::new(1.0, text_color));
            painter.text(Pos2::new(x + 3.0, ruler.min.y + 1.0), egui::Align2::LEFT_TOP, crate::fmt::tick(t, step), font.clone(), text_color);
            // Мелкие деления — пятая часть шага.
            for k in 1..5 {
                let xm = v.t_to_x(t + step * k as f64 / 5.0, rect);
                painter.line_segment([Pos2::new(xm, ruler.max.y - 3.0), Pos2::new(xm, ruler.max.y)], Stroke::new(1.0, text_color));
            }
            t += step;
        }

        // Зоны пропуска: подложка и полоса по нижнему краю.
        for z in zones {
            let (x0, x1) = (v.t_to_x(z.start, rect), v.t_to_x(z.end, rect));
            if x1 < rect.min.x || x0 > rect.max.x {
                continue;
            }
            let color = if z.manual { ZONE_MANUAL } else { ZONE_AUTO };
            painter.rect_filled(Rect::from_x_y_ranges(x0..=x1, track.y_range()), 0.0, color.gamma_multiply(0.12));
            painter.rect_filled(Rect::from_x_y_ranges(x0..=x1, (track.max.y - 3.0)..=track.max.y), 0.0, color);
        }

        // Шортсы.
        for b in blocks {
            let r = Rect::from_x_y_ranges(v.t_to_x(b.start, rect)..=v.t_to_x(b.end, rect), track.shrink2(Vec2::new(0.0, 5.0)).y_range());
            if r.max.x < rect.min.x || r.min.x > rect.max.x {
                continue;
            }
            let is_sel = Some(b.id) == selected;
            let base = match b.look {
                Look::Accepted => b.color,
                Look::Candidate => CANDIDATE_COLOR,
                Look::Rejected => REJECTED_COLOR,
            };
            let fill = if is_sel || b.look == Look::Rejected { base } else { base.gamma_multiply(0.6) };
            painter.rect_filled(r, 3.0, fill);
            if b.look == Look::Candidate && !is_sel {
                painter.rect_stroke(r, 3.0, Stroke::new(1.0, CANDIDATE_EDGE), egui::StrokeKind::Inside);
            }
            if is_sel {
                painter.rect_stroke(r, 3.0, Stroke::new(2.0, Color32::WHITE), egui::StrokeKind::Inside);
            }
            if r.width() > 28.0 {
                let galley = painter.layout_no_wrap(b.label.clone(), egui::FontId::proportional(12.0), Color32::WHITE);
                if galley.size().x < r.width() - 6.0 {
                    painter.galley(Pos2::new(r.min.x.max(rect.min.x) + 4.0, r.center().y - galley.size().y / 2.0), galley, Color32::WHITE);
                }
            }
        }

        // Отметки in/out и область между ними.
        let mark_color = Color32::from_rgb(80, 200, 120);
        if let (Some(a), Some(b)) = (mark_in, mark_out)
            && b > a
        {
            let r = Rect::from_x_y_ranges(v.t_to_x(a, rect)..=v.t_to_x(b, rect), track.y_range());
            painter.rect_filled(r, 0.0, mark_color.gamma_multiply(0.18));
        }
        for (m, is_in) in [(mark_in, true), (mark_out, false)] {
            let Some(m) = m else { continue };
            let x = v.t_to_x(m, rect);
            painter.line_segment([Pos2::new(x, track.min.y), Pos2::new(x, track.max.y)], Stroke::new(2.0, mark_color));
            let dx = if is_in { 5.0 } else { -5.0 };
            painter.line_segment([Pos2::new(x, track.min.y + 1.0), Pos2::new(x + dx, track.min.y + 1.0)], Stroke::new(2.0, mark_color));
            painter.line_segment([Pos2::new(x, track.max.y - 1.0), Pos2::new(x + dx, track.max.y - 1.0)], Stroke::new(2.0, mark_color));
        }

        // Курсор воспроизведения.
        let x = v.t_to_x(pos, rect);
        let red = Color32::from_rgb(235, 70, 60);
        painter.line_segment([Pos2::new(x, rect.min.y), Pos2::new(x, rect.max.y)], Stroke::new(2.0, red));
        painter.add(egui::Shape::convex_polygon(
            vec![Pos2::new(x - 5.0, rect.min.y), Pos2::new(x + 5.0, rect.min.y), Pos2::new(x, rect.min.y + 7.0)],
            red,
            Stroke::NONE,
        ));
    }
}

/// Цвета шортсов по номеру — различимы на тёмном фоне.
pub fn block_color(i: usize) -> Color32 {
    const PALETTE: [Color32; 6] = [
        Color32::from_rgb(66, 135, 245),
        Color32::from_rgb(240, 160, 50),
        Color32::from_rgb(170, 100, 230),
        Color32::from_rgb(40, 180, 170),
        Color32::from_rgb(230, 90, 140),
        Color32::from_rgb(150, 190, 60),
    ];
    PALETTE[i % PALETTE.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_keeps_anchor_and_limits() {
        let mut v = View::new(600.0);
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 50.0));
        v.zoom(0.1, 300.0);
        assert!((v.span - 60.0).abs() < 1e-9);
        // точка 300 с осталась на середине
        assert!((v.t_to_x(300.0, rect) - 500.0).abs() < 1e-3);
        assert!((v.x_to_t(500.0, rect) - 300.0).abs() < 1e-6);
        // не ближе MIN_SPAN и не дальше длительности
        v.zoom(1e-6, 300.0);
        assert_eq!(v.span, MIN_SPAN);
        v.zoom(1e6, 300.0);
        assert_eq!((v.start, v.span), (0.0, 600.0));
    }

    #[test]
    fn pan_and_follow_stay_in_bounds() {
        let mut v = View::new(100.0);
        v.zoom(0.1, 0.0);
        v.pan(-50.0);
        assert_eq!(v.start, 0.0);
        v.pan(1000.0);
        assert_eq!(v.start, 90.0);
        v.follow(20.0);
        assert!(v.start <= 20.0 && 20.0 <= v.start + v.span);
        // короткий ролик: окно не меньше его длительности и не больше
        let mut short = View::new(1.0);
        short.zoom(0.1, 0.5);
        assert_eq!(short.span, 1.0);
    }

    #[test]
    fn tick_steps() {
        let mut v = View::new(6000.0);
        // 1000 px на 100 минут → 1 px = 6 с → шаг не меньше 480 с → 600
        assert_eq!(v.tick_step(1000.0), 600.0);
        v.zoom(0.001, 0.0);
        assert_eq!(v.span, 6.0);
        assert_eq!(v.tick_step(1000.0), 0.5);
    }
}
