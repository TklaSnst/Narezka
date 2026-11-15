//! Вкладка «Библиотека»: импорт ассетов (копия в библиотеку), сетка миниатюр, поиск по имени и тегам,
//! переименование, тип, теги, удаление; история использования.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use eframe::egui::{self, Color32, Rect, Sense, Stroke, Vec2};
use sc_core::{Asset, AssetId, AssetKind, parse_tags};
use sc_library::{Added, LibraryError, Staged};

use crate::App;
use crate::jobs::Msg;

const CARD: Vec2 = Vec2::new(168.0, 176.0);
const THUMB_H: f32 = 124.0;
const DETAILS_W: f32 = 330.0;

#[derive(Default)]
pub struct LibraryState {
    query: String,
    kind: Option<AssetKind>,
    selected: Option<AssetId>,
    /// Миниатюры; `None` — файла миниатюры нет или он не читается.
    thumbs: HashMap<AssetId, Option<egui::TextureHandle>>,
    /// Сколько файлов ещё импортируется.
    importing: usize,
    /// Поля правки выбранного ассета (имя, теги) — до Enter/ухода фокуса.
    name_edit: Option<(AssetId, String)>,
    tags_edit: Option<(AssetId, String)>,
    confirm_delete: Option<AssetId>,
    /// (ошибка?, текст).
    message: Option<(bool, String)>,
}

/// Чем закончился импорт файла в фоновом потоке.
pub struct Imported {
    name: String,
    result: Result<Staged, LibraryError>,
}

impl App {
    pub(crate) fn library_pick(&mut self) {
        self.dialog_open = true;
        let (title, filter) = (self.i18n.tr("library-add-title"), self.i18n.tr("library-filter-files"));
        self.jobs.spawn(move || {
            let exts: Vec<&str> =
                [sc_library::IMAGE_EXTS, sc_library::VIDEO_EXTS, sc_library::FONT_EXTS].concat();
            let files = rfd::FileDialog::new().set_title(title).add_filter(filter, &exts).pick_files();
            Msg::LibraryPicked(files.unwrap_or_default())
        });
    }

    /// Копия, ffprobe и миниатюра — по файлу в фоновом потоке; шрифт ещё и подключается к растеризатору.
    pub(crate) fn library_import(&mut self, paths: Vec<PathBuf>) {
        for path in paths {
            let dir = self.assets.dir().to_owned();
            let known: HashSet<String> = self.assets.assets().iter().map(|a| a.hash.clone()).collect();
            self.lib.importing += 1;
            self.jobs.spawn(move || {
                let name = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
                let mut result = sc_library::stage(&dir, &path, |h| known.contains(h));
                if let Ok(s) = &mut result
                    && s.asset.kind == AssetKind::Font
                {
                    match sc_raster::fonts::install_font(&s.path()) {
                        Ok(font) => s.asset.meta.font = Some(font),
                        Err(e) => {
                            log::warn!("{name}: {e}");
                            if let Ok(s) = result {
                                s.discard();
                            }
                            result = Err(LibraryError::Media(sc_media::MediaError::Parse(e.to_string())));
                        }
                    }
                }
                Msg::LibraryImported(Imported { name, result })
            });
        }
    }

    pub(crate) fn on_library_imported(&mut self, imp: Imported) {
        self.lib.importing = self.lib.importing.saturating_sub(1);
        let name = imp.name;
        let msg = match imp.result.and_then(|s| self.assets.add(s)) {
            Ok(Added::New(id)) => {
                self.library_reveal(id);
                (false, self.i18n.tr_args("library-added", &[("name", name.into())]))
            }
            Ok(Added::Duplicate(id)) => {
                self.library_reveal(id);
                (false, self.i18n.tr_args("library-duplicate", &[("name", name.into())]))
            }
            Err(LibraryError::Duplicate(hash)) => {
                if let Some(id) = self.assets.by_hash(&hash).map(|a| a.id) {
                    self.library_reveal(id);
                }
                (false, self.i18n.tr_args("library-duplicate", &[("name", name.into())]))
            }
            Err(LibraryError::Media(e)) => {
                log::warn!("{name}: {e}");
                (true, self.i18n.tr_args("library-err-unreadable", &[("name", name.into())]))
            }
            Err(e) => (true, self.i18n.tr_args("library-err-io", &[("name", name.into()), ("error", e.to_string().into())])),
        };
        self.lib.message = Some(msg);
    }

    /// Выбрать ассет и снять фильтры, если он под них не попадает.
    fn library_reveal(&mut self, id: AssetId) {
        if let Some(a) = self.assets.get(id) {
            if self.lib.kind.is_some_and(|k| k != a.kind) {
                self.lib.kind = None;
            }
            if !a.matches(&self.lib.query) {
                self.lib.query.clear();
            }
        }
        self.lib.selected = Some(id);
    }

    fn library_result(&mut self, r: Result<(), LibraryError>) {
        if let Err(e) = r {
            self.lib.message = Some((true, self.i18n.tr_args("library-err-save", &[("error", e.to_string().into())])));
        }
    }

    pub(crate) fn library_ui(&mut self, ui: &mut egui::Ui) {
        egui::Panel::right("library-details").resizable(false).exact_size(DETAILS_W).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.library_details(ui));
        });
        egui::Panel::top("library-tools").show(ui, |ui| self.library_toolbar(ui));
        egui::CentralPanel::default().show(ui, |ui| self.library_grid(ui));
    }

    fn library_toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            if ui.add_enabled(!self.dialog_open, egui::Button::new(self.i18n.tr("library-add"))).clicked() {
                self.library_pick();
            }
            if self.dnd_supported {
                ui.weak(self.i18n.tr("library-drop-hint"));
            }
            if self.lib.importing > 0 {
                ui.spinner();
                ui.label(self.i18n.tr_args("library-importing", &[("n", (self.lib.importing as i64).into())]));
            }
            ui.separator();
            ui.add(egui::TextEdit::singleline(&mut self.lib.query).hint_text(self.i18n.tr("library-search-hint")).desired_width(220.0));
            if !self.lib.query.is_empty() && ui.small_button("✖").clicked() {
                self.lib.query.clear();
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(&mut self.lib.kind, None, self.i18n.tr("library-all"));
            for k in AssetKind::ALL {
                let n = self.assets.assets().iter().filter(|a| a.kind == k).count();
                ui.selectable_value(&mut self.lib.kind, Some(k), format!("{} {n}", self.i18n.tr(k.key())));
            }
            let tags = self.assets.tags();
            if !tags.is_empty() {
                ui.separator();
            }
            for (tag, n) in tags {
                let term = format!("#{tag}");
                let on = self.lib.query.split_whitespace().any(|t| t == term);
                if ui.selectable_label(on, format!("{term} {n}")).clicked() {
                    self.lib.query = if on {
                        self.lib.query.split_whitespace().filter(|t| *t != term).collect::<Vec<_>>().join(" ")
                    } else {
                        format!("{} {term}", self.lib.query.trim()).trim_start().to_owned()
                    };
                }
            }
        });
        if let Some((err, text)) = &self.lib.message {
            let color = if *err { ui.visuals().error_fg_color } else { ui.visuals().weak_text_color() };
            ui.colored_label(color, text);
        }
        ui.add_space(2.0);
    }

    fn library_grid(&mut self, ui: &mut egui::Ui) {
        if self.assets.assets().is_empty() {
            ui.add_space(24.0);
            ui.vertical_centered(|ui| ui.label(self.i18n.tr("library-empty")));
            return;
        }
        let items: Vec<Asset> = self.assets.search(&self.lib.query, self.lib.kind).into_iter().cloned().collect();
        if items.is_empty() {
            ui.label(self.i18n.tr("library-nothing-found"));
            return;
        }
        let gap = 10.0;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let cols = (((ui.available_width() + gap) / (CARD.x + gap)).floor() as usize).max(1);
            ui.spacing_mut().item_spacing = Vec2::splat(gap);
            for row in items.chunks(cols) {
                ui.horizontal(|ui| {
                    for a in row {
                        self.library_card(ui, a);
                    }
                });
            }
        });
    }

    fn library_card(&mut self, ui: &mut egui::Ui, a: &Asset) {
        let (rect, resp) = ui.allocate_exact_size(CARD, Sense::click());
        let selected = self.lib.selected == Some(a.id);
        let visuals = ui.visuals().clone();
        let painter = ui.painter_at(rect);
        let bg = if resp.hovered() { visuals.widgets.hovered.weak_bg_fill } else { visuals.faint_bg_color };
        painter.rect_filled(rect, 6.0, bg);
        if selected {
            painter.rect_stroke(rect, 6.0, Stroke::new(2.0, visuals.selection.stroke.color), egui::StrokeKind::Inside);
        }
        let thumb_rect = Rect::from_min_size(rect.min + Vec2::splat(6.0), Vec2::new(CARD.x - 12.0, THUMB_H));
        self.paint_preview(ui, &painter, thumb_rect, a);

        let text_rect = Rect::from_min_max(egui::pos2(rect.min.x + 6.0, thumb_rect.max.y + 4.0), rect.max - Vec2::new(6.0, 4.0));
        let name = if self.assets.is_missing(a) { format!("⚠ {}", a.name) } else { a.name.clone() };
        let sub = self.short_info(a);
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(text_rect).layout(egui::Layout::top_down(egui::Align::Min)));
        child.add(egui::Label::new(egui::RichText::new(name).strong()).truncate().selectable(false));
        child.add(egui::Label::new(egui::RichText::new(sub).small().weak()).truncate().selectable(false));

        let resp = resp.on_hover_text(a.name.clone());
        if resp.clicked() {
            self.lib.selected = Some(a.id);
        }
    }

    /// «Баннер · 600×200 · α», «Вставка · 0:12 · 1920×1080».
    fn short_info(&self, a: &Asset) -> String {
        let mut parts = vec![self.i18n.tr(a.kind.key())];
        if let Some(d) = a.meta.duration {
            parts.push(crate::fmt::duration(d));
        }
        if a.meta.width > 0 {
            parts.push(format!("{}×{}", a.meta.width, a.meta.height));
        }
        if a.meta.alpha {
            parts.push("α".into());
        }
        if let Some(f) = &a.meta.font {
            parts.push(f.clone());
        }
        parts.join(" · ")
    }

    /// Миниатюра, вписанная в `area` (с шахматкой под прозрачным); у шрифта — образец «Аа» системным шрифтом egui.
    fn paint_preview(&mut self, ui: &egui::Ui, painter: &egui::Painter, area: Rect, a: &Asset) {
        painter.rect_filled(area, 4.0, Color32::from_gray(24));
        if a.kind == AssetKind::Font || a.meta.width == 0 {
            painter.text(area.center(), egui::Align2::CENTER_CENTER, "Аа", egui::FontId::proportional(area.height() * 0.4), Color32::from_gray(210));
            return;
        }
        let tex = thumb(ui.ctx(), &mut self.lib.thumbs, &self.assets.thumb(a.id), a.id);
        let Some(tex) = tex else {
            painter.text(area.center(), egui::Align2::CENTER_CENTER, if a.meta.is_animated() { "▶" } else { "🖼" }, egui::FontId::proportional(28.0), Color32::from_gray(140));
            return;
        };
        let size = tex.size_vec2();
        let scale = (area.width() / size.x).min(area.height() / size.y);
        let r = Rect::from_center_size(area.center(), size * scale);
        if a.meta.alpha {
            checker(painter, r);
        }
        painter.image(tex.id(), r, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
        if a.meta.is_animated() {
            let badge = egui::pos2(area.max.x - 4.0, area.max.y - 4.0);
            painter.text(badge, egui::Align2::RIGHT_BOTTOM, "▶", egui::FontId::proportional(14.0), Color32::from_gray(230));
        }
    }

    fn library_details(&mut self, ui: &mut egui::Ui) {
        let Some(a) = self.lib.selected.and_then(|id| self.assets.get(id)).cloned() else {
            ui.add_space(12.0);
            ui.weak(self.i18n.tr("library-select-hint"));
            return;
        };
        let id = a.id;
        ui.add_space(6.0);
        let w = ui.available_width();
        let (area, _) = ui.allocate_exact_size(Vec2::new(w, w * 0.75), Sense::hover());
        let painter = ui.painter_at(area);
        self.paint_preview(ui, &painter, area, &a);
        if self.assets.is_missing(&a) {
            ui.colored_label(ui.visuals().error_fg_color, self.i18n.tr("library-missing"));
        }
        ui.add_space(6.0);

        // Имя и теги правятся в буфере, сохраняются по Enter или уходу фокуса.
        ui.label(self.i18n.tr("library-name"));
        let buf = match &mut self.lib.name_edit {
            Some((eid, s)) if *eid == id => s,
            slot => &mut slot.insert((id, a.name.clone())).1,
        };
        let resp = ui.add(egui::TextEdit::singleline(buf).desired_width(f32::INFINITY));
        if resp.lost_focus() && buf.trim() != a.name {
            let name = buf.clone();
            let r = self.assets.rename(id, &name);
            if r.is_err() {
                self.lib.name_edit = None;
            }
            self.library_result(r);
        }

        ui.label(self.i18n.tr("library-tags"));
        let tags_text = a.tags.join(", ");
        let buf = match &mut self.lib.tags_edit {
            Some((eid, s)) if *eid == id => s,
            slot => &mut slot.insert((id, tags_text.clone())).1,
        };
        let resp = ui.add(egui::TextEdit::singleline(buf).hint_text(self.i18n.tr("library-tags-hint")).desired_width(f32::INFINITY));
        if resp.lost_focus() {
            let tags = parse_tags(buf);
            *buf = tags.join(", ");
            if tags != a.tags {
                let r = self.assets.update(id, |a| a.tags = tags);
                self.library_result(r);
            }
        }

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(self.i18n.tr("library-kind"));
            let mut kind = a.kind;
            // Шрифт — только файл шрифта, и он всегда шрифт.
            if kind == AssetKind::Font {
                ui.label(self.i18n.tr(kind.key()));
            } else {
                egui::ComboBox::from_id_salt("library-kind").selected_text(self.i18n.tr(kind.key())).show_ui(ui, |ui| {
                    for k in AssetKind::ALL.into_iter().filter(|k| *k != AssetKind::Font) {
                        ui.selectable_value(&mut kind, k, self.i18n.tr(k.key()));
                    }
                });
            }
            if kind != a.kind {
                let r = self.assets.update(id, |a| a.kind = kind);
                self.library_result(r);
            }
        });
        if a.kind == AssetKind::Insert {
            let mut hide = a.hide_banner;
            if ui.checkbox(&mut hide, self.i18n.tr("library-hide-banner")).changed() {
                let r = self.assets.update(id, |a| a.hide_banner = hide);
                self.library_result(r);
            }
        }

        ui.add_space(8.0);
        egui::Grid::new("library-info").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
            for (key, value) in self.info_rows(&a) {
                ui.weak(self.i18n.tr(key));
                ui.label(value);
                ui.end_row();
            }
        });

        ui.add_space(12.0);
        if self.lib.confirm_delete == Some(id) {
            ui.label(self.i18n.tr_args("library-delete-confirm", &[("name", a.name.clone().into())]));
            ui.horizontal(|ui| {
                if ui.button(self.i18n.tr("library-delete")).clicked() {
                    self.library_delete(id);
                }
                if ui.button(self.i18n.tr("library-cancel")).clicked() {
                    self.lib.confirm_delete = None;
                }
            });
        } else {
            ui.horizontal(|ui| {
                if ui.button(format!("🗑 {}", self.i18n.tr("library-delete"))).clicked() {
                    self.lib.confirm_delete = Some(id);
                }
                if ui.button(self.i18n.tr("library-open-dir")).clicked() {
                    open_dir(self.assets.dir());
                }
            });
        }
    }

    fn info_rows(&self, a: &Asset) -> Vec<(&'static str, String)> {
        let m = &a.meta;
        let yes_no = |b: bool| self.i18n.tr(if b { "library-yes" } else { "library-no" });
        let mut rows = Vec::new();
        if let Some(f) = &m.font {
            rows.push(("library-info-font", f.clone()));
        }
        if !m.codec.is_empty() {
            rows.push(("library-info-codec", m.codec.clone()));
        }
        if let Some(o) = m.orientation() {
            rows.push(("library-info-size", format!("{}×{}, {}", m.width, m.height, self.i18n.tr(o.key()))));
        }
        if m.width > 0 {
            let d = match (m.duration, m.fps) {
                (Some(d), Some(fps)) => format!("{} · {} fps", crate::fmt::clock_ms(d), crate::fmt::number(fps)),
                (Some(d), None) => crate::fmt::clock_ms(d),
                _ => self.i18n.tr("library-still"),
            };
            rows.push(("library-info-duration", d));
            rows.push(("library-info-alpha", yes_no(m.alpha)));
            if m.is_animated() {
                rows.push(("library-info-audio", yes_no(m.has_audio)));
            }
        }
        rows.push(("library-info-file", self.bytes(a.bytes)));
        rows.push(("library-info-added", crate::fmt::date(a.added)));
        let used = match self.history.summary(a.id) {
            Some((n, last)) => self.i18n.tr_args("library-used", &[("n", (n as i64).into()), ("date", crate::fmt::date(last).into())]),
            None => self.i18n.tr("library-unused"),
        };
        rows.push(("library-info-used", used));
        rows
    }

    fn bytes(&self, n: u64) -> String {
        let n = n as f64;
        let (key, v) = if n >= 1e9 {
            ("size-gb", n / 1e9)
        } else if n >= 1e6 {
            ("size-mb", n / 1e6)
        } else {
            ("size-kb", (n / 1e3).max(0.1))
        };
        self.i18n.tr_args(key, &[("n", crate::fmt::number(v).into())])
    }

    fn library_delete(&mut self, id: AssetId) {
        self.lib.confirm_delete = None;
        match self.assets.delete(id) {
            Ok(a) => {
                self.lib.thumbs.remove(&id);
                self.lib.selected = None;
                self.history.forget(id);
                if let Err(e) = sc_library::save_history(&self.paths.history_file(), &self.history) {
                    log::warn!("история: {e}");
                }
                self.lib.message = Some((false, self.i18n.tr_args("library-deleted", &[("name", a.name.into())])));
            }
            Err(e) => self.library_result(Err(e)),
        }
    }
}

fn thumb(ctx: &egui::Context, cache: &mut HashMap<AssetId, Option<egui::TextureHandle>>, path: &Path, id: AssetId) -> Option<egui::TextureHandle> {
    cache
        .entry(id)
        .or_insert_with(|| {
            let img = image::open(path).ok()?.to_rgba8();
            let size = [img.width() as usize, img.height() as usize];
            let color = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
            Some(ctx.load_texture(format!("asset-{id}"), color, egui::TextureOptions::LINEAR))
        })
        .clone()
}

/// Шахматка под прозрачной картинкой.
fn checker(painter: &egui::Painter, r: Rect) {
    let cell = 8.0;
    painter.rect_filled(r, 0.0, Color32::from_gray(70));
    let (nx, ny) = ((r.width() / cell).ceil() as i32, (r.height() / cell).ceil() as i32);
    for y in 0..ny {
        for x in (y % 2..nx).step_by(2) {
            let min = r.min + Vec2::new(x as f32 * cell, y as f32 * cell);
            let c = Rect::from_min_size(min, Vec2::splat(cell)).intersect(r);
            painter.rect_filled(c, 0.0, Color32::from_gray(100));
        }
    }
}

/// Каталог в файловом менеджере (xdg-open), без ожидания.
fn open_dir(dir: &Path) {
    match std::process::Command::new("xdg-open").arg(dir).spawn() {
        // Ждём в своём потоке, чтобы не оставить зомби.
        Ok(mut child) => drop(std::thread::spawn(move || child.wait())),
        Err(e) => log::warn!("xdg-open {}: {e}", dir.display()),
    }
}
