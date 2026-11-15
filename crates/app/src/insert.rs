//! Рекламная вставка в интерфейсе (SPEC §4): клип пачки, режим показа, «скрывать баннер», место у каждого
//! шортса (маркер на таймлайне прилипает к паузам между фразами), «без вставки» у шортса, превью и рендер.

use std::path::PathBuf;

use eframe::egui::{self, Color32, RichText};
use sc_core::{
    Asset, AssetId, AssetKind, BatchInsert, InsertMode, InsertPlan, SHORTS_MAX, Short, ShortId, ShortStatus, Template,
    Usage, insert_inside, pauses, snap_to_pause,
};
use sc_render::InsertJob;

use crate::App;
use crate::fmt;
use crate::player::PreviewInsert;

/// Насколько далеко маркер тянется к паузе, с.
const SNAP: f64 = 1.0;
const WARN: Color32 = Color32::from_rgb(235, 180, 60);

/// Вставка конкретного шортса: файл, место (секунды исходника) и как показывать.
pub(crate) struct InsertInfo {
    pub path: PathBuf,
    pub at: f64,
    pub duration: f64,
    pub size: (u32, u32),
    pub has_audio: bool,
    pub mode: InsertMode,
    pub hide_banner: bool,
}

impl InsertInfo {
    pub fn plan(&self, template: &Template, banner_layers: &[usize]) -> InsertPlan {
        template.insert_plan(self.mode, banner_layers, self.hide_banner)
    }

    pub fn job(&self, template: &Template, banner_layers: &[usize]) -> InsertJob {
        InsertJob {
            path: self.path.clone(),
            at: self.at,
            duration: self.duration,
            size: self.size,
            has_audio: self.has_audio,
            plan: self.plan(template, banner_layers),
        }
    }

    pub fn preview(&self, template: &Template, banner_layers: &[usize]) -> PreviewInsert {
        PreviewInsert { path: self.path.clone(), at: self.at, duration: self.duration, plan: self.plan(template, banner_layers) }
    }
}

/// Годится в вставки: видео (есть длительность и кадр).
fn playable(a: &Asset) -> bool {
    a.meta.duration.is_some_and(|d| d > 0.0) && a.meta.width > 0 && a.kind != AssetKind::Font
}

impl App {
    /// Ассет вставки пачки, если он в библиотеке и файл на месте.
    pub(crate) fn insert_asset(&self) -> Option<&Asset> {
        let ins = self.batch.insert.as_ref()?;
        self.assets.get(ins.asset).filter(|a| !self.assets.is_missing(a) && playable(a))
    }

    /// Вставка выбрана, но её нет в библиотеке или пропал файл.
    pub(crate) fn insert_missing(&self) -> bool {
        self.batch.insert.is_some() && self.insert_asset().is_none()
    }

    /// Вставка шортса `id`: есть вставка пачки, шортс не отказался, место отмечено.
    pub(crate) fn insert_info(&self, id: ShortId) -> Option<InsertInfo> {
        let (ins, at) = self.batch.insert_for(id)?;
        let a = self.insert_asset()?;
        Some(InsertInfo {
            path: self.assets.path(a),
            at,
            duration: a.meta.duration.unwrap_or(0.0),
            size: (a.meta.width, a.meta.height),
            has_audio: a.meta.has_audio,
            mode: ins.mode(&a.meta),
            hide_banner: a.hide_banner,
        })
    }

    /// Длительность вставки пачки (0 — нет).
    fn insert_duration(&self) -> f64 {
        self.insert_asset().and_then(|a| a.meta.duration).unwrap_or(0.0)
    }

    /// Итоговая длина шортса со вставкой.
    pub(crate) fn final_duration(&self, s: &Short) -> f64 {
        s.duration() + if self.insert_info(s.id).is_some() { self.insert_duration() } else { 0.0 }
    }

    /// Поставить вставку пачки (`None` — без вставки). Режим сбрасывается на «авто».
    pub(crate) fn set_insert(&mut self, asset: Option<AssetId>) {
        self.batch.insert = asset.map(|asset| BatchInsert { asset, mode: None });
        // Шортсам без места — место в паузе у середины, чтобы вставка сразу была видна.
        if asset.is_some() {
            self.place_all_inserts();
        }
    }

    /// Паузы между словами шортса (секунды исходника); нет транскрипта — пусто.
    fn short_pauses(&self, s: &Short) -> Vec<f64> {
        let Some(t) = self.transcript_of(s.source_id) else { return Vec::new() };
        pauses(t.words.iter().filter(|w| w.end > s.start && w.start < s.end).map(|w| (w.start, w.end)), s.start, s.end)
    }

    /// Место вставки шортса `id` в момент `t` (секунды исходника); `snap` — к ближайшей паузе.
    pub(crate) fn place_insert(&mut self, id: ShortId, t: f64, snap: bool) {
        let Some(s) = self.batch.short(id) else { return };
        let t = if snap { snap_to_pause(t, &self.short_pauses(s), SNAP) } else { t };
        let edge = sc_core::INSERT_EDGE;
        let (lo, hi) = (s.start + edge, s.end - edge);
        if hi <= lo {
            return;
        }
        let at = t.clamp(lo, hi);
        if let Some(s) = self.batch.short_mut(id) {
            s.insert_at = Some(at);
            s.no_insert = false;
        }
    }

    /// Место в паузе ближе к середине шортса (без пауз — середина).
    fn default_insert_at(&self, s: &Short) -> f64 {
        let mid = (s.start + s.end) / 2.0;
        self.short_pauses(s).into_iter().min_by(|a, b| (a - mid).abs().total_cmp(&(b - mid).abs())).unwrap_or(mid)
    }

    /// Всем неотклонённым шортсам без места вставки — место у середины.
    fn place_all_inserts(&mut self) {
        let todo: Vec<(ShortId, f64)> = (self.batch.shorts.iter())
            .filter(|s| s.status != ShortStatus::Rejected && !s.insert_at.is_some_and(|at| insert_inside(at, s.start, s.end)))
            .map(|s| (s.id, self.default_insert_at(s)))
            .collect();
        for (id, at) in todo {
            self.place_insert(id, at, false);
        }
    }

    /// Принятые шортсы, которым вставка положена, но место не отмечено (или вышло за края после правки).
    pub(crate) fn shorts_without_insert(&self) -> usize {
        if self.batch.insert.is_none() {
            return 0;
        }
        (self.batch.shorts.iter())
            .filter(|s| s.status == ShortStatus::Accepted && !s.no_insert && self.batch.insert_for(s.id).is_none())
            .count()
    }

    /// Принятые шортсы длиннее предела Shorts (со вставкой).
    pub(crate) fn shorts_too_long(&self) -> usize {
        (self.batch.shorts.iter()).filter(|s| s.status == ShortStatus::Accepted && self.final_duration(s) > SHORTS_MAX).count()
    }

    /// Раздел «Вставка» в «Нарезке»: клип пачки, режим, баннер, место у выбранного шортса.
    pub(crate) fn insert_ui(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.strong(self.i18n.tr("insert-title"));
        if self.insert_missing() {
            ui.colored_label(ui.visuals().error_fg_color, self.i18n.tr("insert-missing"));
        }
        // Вставки, потом прочие видео; недавние — первыми.
        let mut list: Vec<Asset> = self.assets.search("", None).into_iter().filter(|a| playable(a)).cloned().collect();
        let last = |a: &Asset| self.history.summary(a.id).map_or(0, |(_, at)| at);
        list.sort_by_key(|a| (a.kind != AssetKind::Insert, std::cmp::Reverse(last(a)), a.name.to_lowercase()));
        let current = self.batch.insert.as_ref().map(|i| i.asset);
        let label = |a: &Asset| format!("{} · {}", a.name, fmt::duration(a.meta.duration.unwrap_or(0.0)));
        let selected = match current.and_then(|id| list.iter().find(|a| a.id == id)) {
            Some(a) => label(a),
            None => self.i18n.tr("insert-none"),
        };
        let mut pick: Option<Option<AssetId>> = None;
        ui.horizontal(|ui| {
            ui.label(self.i18n.tr("insert-clip"));
            egui::ComboBox::from_id_salt("insert-clip").selected_text(selected).width(ui.available_width()).show_ui(ui, |ui| {
                if ui.selectable_label(current.is_none(), self.i18n.tr("insert-none")).clicked() {
                    pick = Some(None);
                }
                for a in &list {
                    if ui.selectable_label(current == Some(a.id), label(a)).clicked() {
                        pick = Some(Some(a.id));
                    }
                }
            });
        });
        if list.is_empty() && ui.link(self.i18n.tr("insert-library-empty")).clicked() {
            self.tab = crate::Tab::Library;
        }
        if let Some(p) = pick
            && p != current
        {
            self.set_insert(p);
        }
        let Some(asset) = self.insert_asset().cloned() else { return };

        // Режим и баннер.
        let auto = InsertMode::auto(&asset.meta);
        let orient = asset.meta.orientation().map_or_else(String::new, |o| self.i18n.tr(o.key()));
        let mut mode = self.batch.insert.as_ref().and_then(|i| i.mode);
        ui.horizontal(|ui| {
            ui.label(self.i18n.tr("insert-mode"));
            let auto_text = self.i18n.tr_args("insert-mode-auto", &[("mode", self.i18n.tr(auto.key()).into())]);
            let text = mode.map_or_else(|| auto_text.clone(), |m| self.i18n.tr(m.key()));
            let hint = self.i18n.tr_args("insert-mode-hint", &[("orientation", orient.into())]);
            egui::ComboBox::from_id_salt("insert-mode").selected_text(text).width(ui.available_width()).show_ui(ui, |ui| {
                ui.selectable_value(&mut mode, None, auto_text);
                for m in InsertMode::ALL {
                    ui.selectable_value(&mut mode, Some(m), self.i18n.tr(m.key()));
                }
            })
            .response
            .on_hover_text(hint);
        });
        if let Some(ins) = &mut self.batch.insert
            && ins.mode != mode
        {
            ins.mode = mode;
        }
        let mut hide = asset.hide_banner;
        if ui.checkbox(&mut hide, self.i18n.tr("library-hide-banner")).on_hover_text(self.i18n.tr("insert-hide-banner-hint")).changed() {
            let r = self.assets.update(asset.id, |a| a.hide_banner = hide);
            self.library_result(r);
        }
        let missing = self.shorts_without_insert();
        ui.horizontal(|ui| {
            if missing > 0 {
                ui.label(RichText::new(self.i18n.tr_args("insert-unplaced", &[("n", (missing as i64).into())])).color(WARN));
            }
            if ui.button(self.i18n.tr("insert-place-all")).on_hover_text(self.i18n.tr("insert-place-all-hint")).clicked() {
                self.place_all_inserts();
            }
        });

        // Выбранный шортс.
        let Some(s) = self.cut.selected.and_then(|id| self.batch.short(id)).cloned() else {
            ui.weak(self.i18n.tr("insert-select-short"));
            return;
        };
        ui.separator();
        let mut no_insert = s.no_insert;
        ui.checkbox(&mut no_insert, self.i18n.tr("insert-skip-short"));
        if no_insert != s.no_insert
            && let Some(m) = self.batch.short_mut(s.id)
        {
            m.no_insert = no_insert;
        }
        if no_insert {
            return;
        }
        let info = self.insert_info(s.id);
        match &info {
            Some(i) => {
                ui.label(self.i18n.tr_args("insert-at", &[("at", fmt::clock_ms(i.at - s.start).into())]));
            }
            None => {
                ui.label(RichText::new(self.i18n.tr("insert-not-placed")).color(WARN));
            }
        }
        ui.horizontal(|ui| {
            if ui.button(self.i18n.tr("insert-here")).on_hover_text(self.i18n.tr("insert-here-hint")).clicked() {
                self.insert_here();
            }
            if info.is_some() {
                if ui.button(self.i18n.tr("insert-play")).on_hover_text(self.i18n.tr("insert-play-hint")).clicked()
                    && let Some(p) = &mut self.player
                {
                    p.play_insert();
                }
                if ui.button("✖").on_hover_text(self.i18n.tr("insert-remove")).clicked()
                    && let Some(m) = self.batch.short_mut(s.id)
                {
                    m.insert_at = None;
                }
            }
        });
        let total = self.final_duration(&s);
        let text = self.i18n.tr_args(
            "insert-total",
            &[("total", fmt::duration(total).into()), ("short", fmt::duration(s.duration()).into()), ("insert", fmt::duration(self.insert_duration()).into())],
        );
        if total > SHORTS_MAX {
            ui.label(RichText::new(format!("⚠ {text} — {}", self.i18n.tr("insert-too-long"))).color(WARN));
        } else if info.is_some() {
            ui.weak(text);
        }
        if let Some((t, d)) = self.player.as_ref().and_then(|p| p.insert_progress()) {
            ui.weak(self.i18n.tr_args("insert-playing", &[("t", fmt::clock_ms(t).into()), ("d", fmt::clock_ms(d).into())]));
        }
    }

    /// Место вставки выбранного шортса — у курсора (к ближайшей паузе).
    pub(crate) fn insert_here(&mut self) {
        let (Some(id), Some(p)) = (self.cut.selected, &self.player) else { return };
        let t = p.state.time;
        if self.batch.insert.is_none() {
            return;
        }
        self.place_insert(id, t, true);
    }

    /// Рендер начат: вставка — в историю использования.
    pub(crate) fn remember_insert(&mut self, at: u64) {
        if let Some(i) = &self.batch.insert {
            self.history.record(Usage { asset: i.asset, role: AssetKind::Insert, at, rect: None });
        }
    }
}
