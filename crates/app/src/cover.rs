//! Обложка-кадр в интерфейсе (SPEC §5): блок «Обложка шортса» на «Оформлении» — включить, откуда кадр
//! (любое место исходника или картинка библиотеки), длительность в кадрах, «звук сразу», стиль обложки
//! (встроенные — надписи поверх шаблона шортса, свои — сохранённые шаблоны обложек). Надписи правятся
//! тем же редактором, что и шаблон: цель правки [`Target::Cover`].

use std::time::Instant;

use eframe::egui;
use sc_core::library::LibraryError;
use sc_core::{
    AssetId, AssetKind, COVER_FRAMES, Cover, CoverSource, CoverStyleRef, DEFAULT_COVER_STYLE, ShortId, ShortStatus, Template,
    TextVars, builtin_style, builtin_style_ids, cover_duration,
};
use sc_render::{CoverFrame, CoverJob};

use crate::App;
use crate::design::{Snapshot, Target};
use crate::fmt;

/// Правки одного поля обложки подряд (с паузами меньше этой) — один шаг отмены.
const COALESCE: std::time::Duration = std::time::Duration::from_millis(1500);

/// Состояние блока обложки.
pub struct CoverUi {
    /// Стиль для новых обложек — последний выбранный.
    pub style: CoverStyleRef,
    /// Открыт ввод имени для «Сохранить стиль как…»: имя и ошибка.
    save_as: Option<(String, Option<String>)>,
}

impl Default for CoverUi {
    fn default() -> Self {
        Self { style: CoverStyleRef::Builtin(DEFAULT_COVER_STYLE.into()), save_as: None }
    }
}

impl App {
    /// Шаблон обложки по стилю `r` для шортса `id` (встроенный — поверх шаблона шортса).
    fn cover_style_template(&self, r: &CoverStyleRef, id: ShortId) -> Option<Template> {
        match r {
            CoverStyleRef::Builtin(s) => builtin_style(s, self.batch.template_for(id)),
            CoverStyleRef::User(u) => self.cover_styles.user(u).map(|u| u.template.clone()),
        }
    }

    fn cover_style_label(&self, r: &CoverStyleRef) -> String {
        match r {
            CoverStyleRef::Builtin(id) => self.i18n.tr(&format!("cover-style-{id}")),
            CoverStyleRef::User(id) => self.cover_styles.user(id).map_or_else(|| id.clone(), |u| u.template.name.clone()),
        }
    }

    /// Частота кадров вывода для шортса (как у исходника, если не задана).
    fn cover_fps(&self, id: ShortId) -> f64 {
        let src = self.batch.short(id).and_then(|s| self.batch.source(s.source_id)).and_then(|s| s.info.as_ref()).map(|i| i.fps);
        self.batch.output.fps.or(src).unwrap_or(30.0)
    }

    /// Новая обложка шортса: последний выбранный стиль, кадр — текущий в превью, если он внутри шортса,
    /// иначе ближе к началу шортса.
    fn cover_new(&self, id: ShortId) -> Option<Cover> {
        let s = self.batch.short(id)?;
        let style = self.cover_style_template(&self.cover_ui.style, id)
            .or_else(|| builtin_style(DEFAULT_COVER_STYLE, self.batch.template_for(id)))?;
        let now = self.player.as_ref().map(|p| p.state.time).filter(|t| (s.start..s.end).contains(t));
        let t = now.unwrap_or(s.start + (s.duration() / 3.0).min(1.0));
        Some(Cover { source: CoverSource::Frame { t }, template: style, frames: self.settings.cover_frames, sound_now: false })
    }

    /// Поставить обложку шортса (`None` — убрать) одним шагом отмены; правки с одним `key` подряд склеиваются.
    fn set_cover(&mut self, id: ShortId, cover: Option<Cover>, key: &str) {
        let now = Instant::now();
        let same = matches!(&self.design.last_edit, Some((k, at)) if k == key && now.duration_since(*at) < COALESCE);
        self.design_apply(Snapshot::Cover(id, cover), !same);
        self.design.last_edit = Some((key.to_owned(), now));
    }

    /// Обложку правят на «Оформлении»: в превью — её кадр (перемотка к нему или картинка вместо видео).
    /// Вне правки обложки превью возвращается к шортсу. Каждый кадр.
    pub(crate) fn cover_sync_preview(&mut self) {
        let want = match self.design.target {
            Target::Cover(id) if self.tab == crate::Tab::Design => self.batch.short(id).and_then(|s| s.cover.as_ref()).map(|c| c.source.clone()),
            _ => None,
        };
        if want == self.design.cover_shown {
            return;
        }
        let back_to = self.cut.selected.and_then(|id| self.batch.short(id)).map(|s| s.start);
        let still = match &want {
            Some(CoverSource::Image { asset }) => self.cover_image_path(*asset),
            _ => None,
        };
        let Some(p) = &mut self.player else { return };
        p.set_still(still.as_deref());
        match &want {
            Some(CoverSource::Frame { t }) => {
                p.set_paused(true);
                p.seek(*t, true);
            }
            Some(CoverSource::Image { .. }) => {}
            None => {
                if let Some(t) = back_to {
                    p.seek(t, true);
                }
            }
        }
        self.design.cover_shown = want;
    }

    fn cover_image_path(&self, asset: AssetId) -> Option<std::path::PathBuf> {
        self.assets.get(asset).filter(|a| !self.assets.is_missing(a)).map(|a| self.assets.path(a))
    }

    /// Картинки библиотеки, годные в обложку: без длительности (не видео и не GIF-анимация) и не шрифты.
    fn cover_images(&self) -> Vec<(AssetId, String)> {
        let mut v: Vec<(AssetId, String)> = (self.assets.assets().iter())
            .filter(|a| a.kind != AssetKind::Font && a.meta.duration.is_none() && a.meta.width > 0 && !self.assets.is_missing(a))
            .map(|a| (a.id, a.name.clone()))
            .collect();
        v.sort_by_key(|(_, n)| n.to_lowercase());
        v
    }

    /// Обложка шортса для рендера: шаблон обложки с баннером шортса и подстановками; картинки нет — без обложки.
    pub(crate) fn cover_job(&self, id: ShortId) -> Option<CoverJob> {
        let c = self.batch.short(id)?.cover.as_ref()?;
        let frame = match c.source {
            CoverSource::Frame { t } => CoverFrame::Source { t },
            CoverSource::Image { asset } => {
                let a = self.assets.get(asset).filter(|a| !self.assets.is_missing(a))?;
                CoverFrame::Image { path: self.assets.path(a), size: (a.meta.width, a.meta.height) }
            }
        };
        let (template, _) = c.template.with_banner_layers(self.banner_art(Some(id)).as_ref());
        let s = self.batch.short(id)?;
        let template = template.with_vars(&TextVars { part: self.batch.part_number(id), title: s.title.clone() });
        Some(CoverJob { frame, template, frames: c.frames, sound_now: c.sound_now })
    }

    /// На сколько обложка удлиняет шортс, с (0 — нет обложки или «звук сразу»).
    pub(crate) fn cover_extra(&self, id: ShortId) -> f64 {
        let Some(c) = self.batch.short(id).and_then(|s| s.cover.as_ref()) else { return 0.0 };
        c.shift(self.cover_fps(id))
    }

    /// Блок «Обложка шортса» над списком слоёв.
    pub(crate) fn cover_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading(self.i18n.tr("cover-heading"));
        let Some(short) = self.cut.selected.and_then(|id| self.batch.short(id)).cloned() else {
            ui.weak(self.i18n.tr("cover-no-short"));
            return;
        };
        let id = short.id;
        let mut on = short.cover.is_some();
        if ui.checkbox(&mut on, self.i18n.tr("cover-enable")).on_hover_text(self.i18n.tr("cover-enable-hint")).changed() {
            let c = if on { self.cover_new(id) } else { None };
            self.design.last_edit = None;
            self.set_cover(id, c, "cover-enable");
            self.design.cover = on;
        }
        let Some(cover) = short.cover.clone() else {
            ui.add_enabled_ui(self.batch.shorts.iter().any(|s| s.status == ShortStatus::Accepted && s.cover.is_none()), |ui| {
                if ui.button(self.i18n.tr("cover-all")).on_hover_text(self.i18n.tr("cover-all-hint")).clicked() {
                    self.cover_all();
                }
            });
            return;
        };
        ui.toggle_value(&mut self.design.cover, self.i18n.tr("cover-edit")).on_hover_text(self.i18n.tr("cover-edit-hint"));

        let mut c = cover.clone();
        let mut key;
        // Откуда кадр.
        let images = self.cover_images();
        ui.horizontal(|ui| {
            let frame = matches!(c.source, CoverSource::Frame { .. });
            if ui.selectable_label(frame, self.i18n.tr("cover-src-frame")).clicked() && !frame {
                c.source = CoverSource::Frame { t: short.start };
            }
            let pick = ui.add_enabled(!images.is_empty(), egui::Button::selectable(!frame, self.i18n.tr("cover-src-image")));
            let pick = if images.is_empty() { pick.on_disabled_hover_text(self.i18n.tr("cover-no-images")) } else { pick };
            if pick.clicked() && frame {
                c.source = CoverSource::Image { asset: images[0].0 };
            }
        });
        match &mut c.source {
            CoverSource::Frame { t } => {
                key = "cover-t";
                let duration = self.batch.source(short.source_id).and_then(|s| s.info.as_ref()).map_or(short.end, |i| i.duration);
                let step = 1.0 / self.cover_fps(id);
                ui.spacing_mut().slider_width = (ui.available_width() - 12.0).max(80.0);
                ui.add(egui::Slider::new(t, 0.0..=duration).show_value(false))
                    .on_hover_text(self.i18n.tr("cover-frame-hint"));
                ui.horizontal_wrapped(|ui| {
                    if ui.small_button("◀").on_hover_text(self.i18n.tr("cover-prev-frame")).clicked() {
                        *t = (*t - step).max(0.0);
                    }
                    ui.add(egui::DragValue::new(t).speed(step).range(0.0..=duration).custom_formatter(|v, _| fmt::clock_ms(v)));
                    if ui.small_button("▶").on_hover_text(self.i18n.tr("cover-next-frame")).clicked() {
                        *t = (*t + step).min(duration);
                    }
                    if ui.small_button(self.i18n.tr("cover-short-start")).clicked() {
                        *t = short.start;
                    }
                    // Кадр, на котором стоит превью (когда обложка не правится, превью идёт по шортсу).
                    let now = self.player.as_ref().map(|p| p.state.time);
                    if let Some(now) = now
                        && !self.design.cover
                        && ui.small_button("📍").on_hover_text(self.i18n.tr("cover-take-current")).clicked()
                    {
                        *t = now;
                    }
                });
                if !(short.start..=short.end).contains(t) {
                    ui.weak(self.i18n.tr("cover-outside"));
                }
            }
            CoverSource::Image { asset } => {
                key = "cover-image";
                let current = images.iter().find(|(a, _)| a == asset).map_or_else(|| self.i18n.tr("cover-image-missing"), |(_, n)| n.clone());
                egui::ComboBox::from_id_salt("cover-image").width(ui.available_width() - 8.0).selected_text(current).show_ui(ui, |ui| {
                    for (a, name) in &images {
                        ui.selectable_value(asset, *a, name);
                    }
                });
            }
        }
        // Длительность и звук.
        ui.horizontal_wrapped(|ui| {
            ui.label(self.i18n.tr("cover-frames"));
            let unit = self.i18n.tr("cover-frames-unit");
            if ui.add(egui::DragValue::new(&mut c.frames).range(COVER_FRAMES).suffix(unit)).changed() {
                key = "cover-frames";
            }
            let ms = (cover_duration(c.frames, self.cover_fps(id)) * 1000.0).round() as i64;
            ui.weak(self.i18n.tr_args("cover-frames-ms", &[("ms", ms.into())]));
        });
        ui.checkbox(&mut c.sound_now, self.i18n.tr("cover-sound-now")).on_hover_text(self.i18n.tr("cover-sound-now-hint"));
        if c != cover {
            self.set_cover(id, Some(c.clone()), key);
            if c.frames != cover.frames {
                // Длительность запоминается для новых обложек.
                self.settings.cover_frames = c.frames;
                self.save_settings();
            }
        }
        ui.add_space(4.0);
        self.cover_style_ui(ui, id);
    }

    /// Стиль обложки: выбрать (встроенный или свой), сохранить свой, удалить.
    fn cover_style_ui(&mut self, ui: &mut egui::Ui, id: ShortId) {
        let mut pick = None;
        let builtins: Vec<CoverStyleRef> = builtin_style_ids().map(|s| CoverStyleRef::Builtin(s.into())).collect();
        let users: Vec<CoverStyleRef> = self.cover_styles.list.iter().map(|u| CoverStyleRef::User(u.id.clone())).collect();
        let current = self.cover_ui.style.clone();
        ui.horizontal(|ui| {
            ui.label(self.i18n.tr("cover-style"));
            egui::ComboBox::from_id_salt("cover-style")
                .width(ui.available_width() - 8.0)
                .selected_text(self.cover_style_label(&current))
                .show_ui(ui, |ui| {
                    let mut item = |ui: &mut egui::Ui, r: &CoverStyleRef| {
                        if ui.selectable_label(*r == current, self.cover_style_label(r)).clicked() {
                            pick = Some(r.clone());
                        }
                    };
                    ui.weak(self.i18n.tr("tpl-builtin"));
                    for r in &builtins {
                        item(ui, r);
                    }
                    ui.separator();
                    ui.weak(self.i18n.tr("tpl-user"));
                    if users.is_empty() {
                        ui.weak(self.i18n.tr("cover-style-user-empty"));
                    }
                    for r in &users {
                        item(ui, r);
                    }
                })
                .response
                .on_hover_text(self.i18n.tr("cover-style-hint"));
        });
        if let Some(r) = pick
            && let Some(t) = self.cover_style_template(&r, id)
            && let Some(mut c) = self.batch.short(id).and_then(|s| s.cover.clone())
        {
            c.template = t;
            self.design.last_edit = None;
            self.set_cover(id, Some(c), "cover-style");
            self.cover_ui.style = r;
            self.design.layer = None;
        }

        ui.horizontal_wrapped(|ui| {
            if ui.button(self.i18n.tr("cover-style-save")).on_hover_text(self.i18n.tr("cover-style-save-hint")).clicked() {
                let base = self.i18n.tr("cover-style-new-name");
                self.cover_ui.save_as = Some((self.cover_styles.free_name(&base), None));
            }
            if let CoverStyleRef::User(uid) = self.cover_ui.style.clone()
                && self.cover_styles.user(&uid).is_some()
                && ui.button(self.i18n.tr("cover-style-delete")).clicked()
            {
                match self.cover_styles.store.delete(&uid) {
                    Ok(()) => {
                        self.cover_styles.refresh();
                        self.cover_ui.style = CoverStyleRef::Builtin(DEFAULT_COVER_STYLE.into());
                    }
                    Err(e) => self.status = Some(self.lib_error(&e)),
                }
            }
        });
        let mut close = false;
        if let Some((name, error)) = &mut self.cover_ui.save_as {
            let mut save = false;
            ui.horizontal(|ui| {
                let r = ui.text_edit_singleline(name);
                save = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                save |= ui.button("✔").clicked();
                close = ui.button("✖").clicked();
            });
            if let Some(e) = error {
                ui.colored_label(ui.visuals().error_fg_color, e.as_str());
            }
            if save {
                let name = name.clone();
                match self.save_cover_style(id, &name) {
                    Ok(()) => close = true,
                    Err(e) => {
                        let msg = self.lib_error(&e);
                        if let Some((_, err)) = &mut self.cover_ui.save_as {
                            *err = Some(msg);
                        }
                    }
                }
            }
        }
        if close {
            self.cover_ui.save_as = None;
        }
    }

    /// Обложка шортса `id` — новым своим стилем обложки (шаблоном обложки) с именем `name`.
    fn save_cover_style(&mut self, id: ShortId, name: &str) -> Result<(), LibraryError> {
        let Some(c) = self.batch.short(id).and_then(|s| s.cover.as_ref()) else { return Ok(()) };
        let uid = self.cover_styles.store.create(name, &c.template)?;
        self.cover_styles.refresh();
        self.cover_ui.style = CoverStyleRef::User(uid);
        self.status = Some(self.i18n.tr_args("cover-style-saved", &[("name", name.trim().to_owned().into())]));
        Ok(())
    }

    /// Обложки всем принятым шортсам без обложки — текущим стилем.
    fn cover_all(&mut self) {
        let ids: Vec<ShortId> = (self.batch.shorts.iter()).filter(|s| s.status == ShortStatus::Accepted && s.cover.is_none()).map(|s| s.id).collect();
        for id in ids {
            let c = self.cover_new(id);
            self.design.last_edit = None;
            self.set_cover(id, c, "cover-all");
        }
    }
}
