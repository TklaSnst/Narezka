//! Баннер пачки в интерфейсе (SPEC §4): выбор из библиотеки, текст рядом, «без баннера» у шортса,
//! и «Сделать как в прошлый раз?» при новой пачке.
//!
//! Положение и размер баннера — рамка «Место под баннер» шаблона: её двигают мышью на «Оформлении».
//! В превью и рендер баннер попадает внутри шаблона ([`sc_core::Template::with_banner`]).

use eframe::egui::{self, Sense, Vec2};
use sc_core::{
    Asset, AssetId, AssetKind, Banner, BannerArt, BannerSide, BannerText, DEFAULT_BANNER_RECT, Layer, ShortId, Usage,
};

use crate::App;
use crate::design::{FileTarget, Snapshot, color};

/// Номер «слоя» для [`FileTarget::Font`]: свой шрифт текста баннера.
pub(crate) const BANNER_TEXT_FONT: usize = usize::MAX;

/// Карточка выбора баннера.
const PICK: Vec2 = Vec2::new(96.0, 78.0);

impl App {
    /// Баннер для показа: шортса `short` (если он не «без баннера») или пачки; ассета нет — `None`.
    pub(crate) fn banner_art(&self, short: Option<ShortId>) -> Option<BannerArt> {
        let b = match short {
            Some(id) => self.batch.banner_for(id),
            None => self.batch.banner.as_ref(),
        }?;
        let a = self.assets.get(b.asset).filter(|a| !self.assets.is_missing(a))?;
        Some(BannerArt::new(b, a, self.assets.path(a)))
    }

    /// Ассет баннера пачки, если он выбран, но его нет в библиотеке (удалён) или пропал файл.
    pub(crate) fn banner_missing(&self) -> bool {
        self.batch.banner.as_ref().is_some_and(|b| self.assets.get(b.asset).is_none_or(|a| self.assets.is_missing(a)))
    }

    /// Поставить баннер пачки (`None` — без баннера). Если в шаблоне пачки нет места под баннер — оно
    /// добавляется (его двигают мышью), отдельным шагом отмены.
    pub(crate) fn set_banner(&mut self, asset: Option<AssetId>) {
        let text = self.batch.banner.as_ref().and_then(|b| b.text.clone());
        self.batch.banner = asset.map(|asset| Banner { asset, text });
        if asset.is_some() && self.batch.template.banner_slot().is_none() {
            let mut t = self.batch.template.clone();
            t.layers.push(Layer::BannerSlot { rect: DEFAULT_BANNER_RECT });
            self.design_apply(Snapshot::Batch(t, self.batch.template_ref.clone()), true);
        }
    }

    /// Раздел «Баннер пачки» (панель свойств места под баннер на «Оформлении»).
    pub(crate) fn banner_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading(self.i18n.tr("banner-title"));
        ui.weak(self.i18n.tr("banner-place-hint"));
        ui.add_space(4.0);
        if self.banner_missing() {
            ui.colored_label(ui.visuals().error_fg_color, self.i18n.tr("banner-missing"));
        }

        // Баннеры, потом картинки; среди них — недавно использованные первыми.
        let mut list: Vec<Asset> = self
            .assets
            .search("", None)
            .into_iter()
            .filter(|a| matches!(a.kind, AssetKind::Banner | AssetKind::Image))
            .cloned()
            .collect();
        let last = |a: &Asset| self.history.summary(a.id).map_or(0, |(_, at)| at);
        list.sort_by_key(|a| (a.kind != AssetKind::Banner, std::cmp::Reverse(last(a)), a.name.to_lowercase()));

        let current = self.batch.banner.as_ref().map(|b| b.asset);
        let mut pick: Option<Option<AssetId>> = None;
        ui.horizontal_wrapped(|ui| {
            let (rect, resp) = ui.allocate_exact_size(PICK, Sense::click());
            frame(ui, rect, current.is_none(), resp.hovered());
            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, self.i18n.tr("banner-none"), egui::FontId::proportional(13.0), ui.visuals().text_color());
            if resp.clicked() {
                pick = Some(None);
            }
            for a in &list {
                let (rect, resp) = ui.allocate_exact_size(PICK, Sense::click());
                frame(ui, rect, current == Some(a.id), resp.hovered());
                let painter = ui.painter_at(rect);
                self.paint_preview(ui, &painter, rect.shrink(4.0), a);
                if resp.on_hover_text(&a.name).clicked() {
                    pick = Some(Some(a.id));
                }
            }
        });
        if list.is_empty() {
            ui.weak(self.i18n.tr("banner-library-empty"));
        }
        if ui.link(self.i18n.tr("banner-open-library")).clicked() {
            self.tab = crate::Tab::Library;
        }
        if let Some(p) = pick {
            self.set_banner(p);
        }

        let Some(mut banner) = self.batch.banner.clone() else { return };
        ui.add_space(8.0);
        ui.separator();
        let mut on = banner.text.is_some();
        ui.checkbox(&mut on, self.i18n.tr("banner-text-on")).on_hover_text(self.i18n.tr("banner-text-hint"));
        if on != banner.text.is_some() {
            banner.text = on.then(BannerText::default);
        }
        let mut font_pick = None;
        if let Some(text) = &mut banner.text {
            ui.add(egui::TextEdit::multiline(&mut text.content).desired_rows(1).desired_width(f32::INFINITY).hint_text(self.i18n.tr("banner-text-placeholder")));
            let tr = &self.i18n;
            egui::Grid::new("banner-text").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                ui.label(tr.tr("banner-side"));
                egui::ComboBox::from_id_salt("banner-side").selected_text(tr.tr(text.side.key())).show_ui(ui, |ui| {
                    for s in BannerSide::ALL {
                        ui.selectable_value(&mut text.side, s, tr.tr(s.key()));
                    }
                });
                ui.end_row();
                ui.label(tr.tr("prop-font-size"));
                let cw = self.batch.template.canvas.width as f32;
                let mut px = text.size * cw;
                if ui.add(egui::DragValue::new(&mut px).range(10.0..=200.0).suffix(" px")).changed() {
                    text.size = px / cw;
                }
                ui.end_row();
                ui.label(tr.tr("prop-color"));
                color(ui, &mut text.color);
                ui.end_row();
                ui.label(tr.tr("prop-accent"));
                color(ui, &mut text.accent_color);
                ui.end_row();
                ui.label(tr.tr("prop-outline"));
                let mut outline = text.outline.is_some();
                ui.checkbox(&mut outline, "");
                if outline != text.outline.is_some() {
                    text.outline = outline.then(|| BannerText::default().outline).flatten();
                }
                ui.end_row();
            });
            crate::design::font_props(ui, &self.i18n, &mut text.font, BANNER_TEXT_FONT, &mut font_pick);
        }
        if Some(&banner) != self.batch.banner.as_ref() {
            self.batch.banner = Some(banner);
        }
        if let Some(target) = font_pick {
            self.pick_file(target);
        }

        if let Some(id) = self.cut.selected
            && let Some(s) = self.batch.short_mut(id)
        {
            ui.add_space(8.0);
            ui.separator();
            ui.checkbox(&mut s.no_banner, self.i18n.tr("banner-skip-short"));
        }
    }

    /// Свой шрифт для текста баннера подключён.
    pub(crate) fn set_banner_font(&mut self, name: String) {
        if let Some(t) = self.batch.banner.as_mut().and_then(|b| b.text.as_mut()) {
            t.font = name;
        }
    }

    /// Рендер пачки начат: баннер — в историю использования, настройки пачки — для «как в прошлый раз».
    pub(crate) fn remember_batch(&mut self) {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        if let Some(b) = &self.batch.banner {
            let rect = self.batch.template.banner_slot().map(|i| self.batch.template.layers[i].rect());
            self.history.record(Usage { asset: b.asset, role: AssetKind::Banner, at: now, rect });
        }
        self.history.last_batch = Some(self.batch.preset(now));
        self.preset_offer = false;
        if let Err(e) = sc_library::save_history(&self.paths.history_file(), &self.history) {
            log::error!("история: {e}");
        }
    }

    /// «Сделать как в прошлый раз?» — окно при новой пачке, пока не ответили.
    pub(crate) fn preset_dialog(&mut self, ctx: &egui::Context) {
        if !self.preset_offer {
            return;
        }
        let Some(p) = self.history.last_batch.clone() else {
            self.preset_offer = false;
            return;
        };
        let banner = match &p.banner {
            Some(b) => match self.assets.get(b.asset) {
                Some(a) => a.name.clone(),
                None => self.i18n.tr("preset-banner-gone"),
            },
            None => self.i18n.tr("banner-none"),
        };
        let dir = p.output.dir.as_ref().map_or_else(|| self.i18n.tr("preset-dir-default"), |d| d.display().to_string());
        let when = crate::fmt::date(p.at);
        let mut answer = None;
        egui::Modal::new(egui::Id::new("preset")).show(ctx, |ui| {
            ui.set_max_width(460.0);
            ui.heading(self.i18n.tr("preset-title"));
            ui.weak(self.i18n.tr_args("preset-when", &[("when", when.into())]));
            ui.add_space(6.0);
            egui::Grid::new("preset-grid").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
                ui.label(self.i18n.tr("preset-template"));
                let name = p.template_ref.as_ref().map_or_else(|| p.template.name.clone(), |r| self.template_label(r));
                ui.strong(name);
                ui.end_row();
                ui.label(self.i18n.tr("preset-banner"));
                ui.strong(banner);
                ui.end_row();
                ui.label(self.i18n.tr("preset-output"));
                ui.strong(dir);
                ui.end_row();
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(self.i18n.tr("preset-yes")).clicked() {
                    answer = Some(true);
                }
                if ui.button(self.i18n.tr("preset-no")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    answer = Some(false);
                }
            });
        });
        match answer {
            Some(true) => {
                self.batch.apply_preset(&p);
                if self.banner_missing() {
                    self.batch.banner = None;
                }
                self.preset_offer = false;
            }
            Some(false) => self.preset_offer = false,
            None => {}
        }
    }
}

fn frame(ui: &egui::Ui, rect: egui::Rect, selected: bool, hovered: bool) {
    let v = ui.visuals();
    let bg = if hovered { v.widgets.hovered.weak_bg_fill } else { v.faint_bg_color };
    ui.painter().rect_filled(rect, 4.0, bg);
    if selected {
        ui.painter().rect_stroke(rect, 4.0, egui::Stroke::new(2.0, v.selection.stroke.color), egui::StrokeKind::Inside);
    }
}

impl FileTarget {
    /// Свой шрифт текста баннера (а не слоя шаблона).
    pub(crate) fn is_banner_font(self) -> bool {
        matches!(self, FileTarget::Font(BANNER_TEXT_FONT))
    }
}
