//! Библиотека шаблонов на «Оформлении»: выбор шаблона пачки (встроенные и свои), сохранить / сохранить как /
//! дублировать / переименовать / удалить свой шаблон, локальные правки выбранного шортса.

use eframe::egui;
use sc_core::library::{self, LibraryError, TemplateRef, UserTemplate, UserTemplates};
use sc_core::{Template, TextVars};

use crate::App;
use crate::design::Snapshot;

/// Свои шаблоны: каталог и прочитанный из него список (перечитывается после каждой операции).
pub struct Library {
    store: UserTemplates,
    list: Vec<UserTemplate>,
}

impl Library {
    pub fn new(store: UserTemplates) -> Self {
        let list = store.list();
        Self { store, list }
    }

    fn refresh(&mut self) {
        self.list = self.store.list();
    }

    fn user(&self, id: &str) -> Option<&UserTemplate> {
        self.list.iter().find(|u| u.id == id)
    }

    /// Шаблон библиотеки, на который ссылается пачка.
    fn source(&self, r: &TemplateRef) -> Option<Template> {
        match r {
            TemplateRef::Builtin(id) => library::builtin(id),
            TemplateRef::User(id) => self.user(id).map(|u| u.template.clone()),
        }
    }

    /// Свободное имя вида «база», «база 2», «база 3»…
    fn free_name(&self, base: &str) -> String {
        let taken = |n: &str| self.list.iter().any(|u| u.template.name.trim().eq_ignore_ascii_case(n.trim()));
        (1..).map(|i| if i == 1 { base.to_owned() } else { format!("{base} {i}") }).find(|n| !taken(n)).unwrap()
    }
}

/// Открытый диалог библиотеки.
pub enum LibDialog {
    SaveAs { name: String, error: Option<String> },
    Rename { id: String, name: String, error: Option<String> },
    Delete { id: String, name: String },
}

/// Одинаковы ли шаблоны без учёта имени (имя — подпись в библиотеке, не оформление).
fn same_look(a: &Template, b: &Template) -> bool {
    a.canvas == b.canvas && a.background == b.background && a.layers == b.layers
}

impl App {
    /// Подпись шаблона библиотеки.
    fn template_label(&self, r: &TemplateRef) -> String {
        match r {
            TemplateRef::Builtin(id) => self.i18n.tr(&format!("template-{id}")),
            TemplateRef::User(id) => self.templates.user(id).map_or_else(|| id.clone(), |u| u.template.name.clone()),
        }
    }

    /// Шаблон пачки отличается от своего источника в библиотеке.
    fn batch_template_modified(&self) -> bool {
        match &self.batch.template_ref {
            Some(r) => self.templates.source(r).is_none_or(|src| !same_look(&src, &self.batch.template)),
            None => false,
        }
    }

    fn lib_error(&self, e: &LibraryError) -> String {
        match e {
            LibraryError::EmptyName => self.i18n.tr("tpl-err-empty"),
            LibraryError::NameTaken(n) => self.i18n.tr_args("tpl-err-taken", &[("name", n.clone().into())]),
            LibraryError::Invalid(_) => self.i18n.tr("design-invalid"),
            LibraryError::NotFound | LibraryError::Io(_) => self.i18n.tr_args("tpl-err-io", &[("error", e.to_string().into())]),
        }
    }

    /// Выбор шаблона из библиотеки — для пачки или, при локальных правках, для выбранного шортса. Отменяется.
    fn pick_template(&mut self, r: TemplateRef) {
        let Some(t) = self.templates.source(&r) else { return };
        let next = match self.design.short {
            Some(id) => Snapshot::Short(id, Some(t)),
            None => Snapshot::Batch(t, Some(r)),
        };
        self.design.last_edit = None;
        self.design_apply(next, true);
        self.design.layer = None;
    }

    /// Блок «Шаблон» над списком слоёв.
    pub(crate) fn template_ui(&mut self, ui: &mut egui::Ui) {
        // Диалог — до кнопок: открывший его клик не должен в том же кадре попасть в подложку и закрыть его.
        self.lib_dialog_ui(ui.ctx());
        ui.heading(self.i18n.tr("tpl-heading"));
        let modified = self.batch_template_modified();
        let current = match &self.batch.template_ref {
            Some(r) => self.template_label(r),
            None => self.batch.template.name.clone(),
        };
        let current = if modified { format!("{current} •") } else { current };

        let mut pick = None;
        let builtins: Vec<TemplateRef> = library::builtin_ids().map(|id| TemplateRef::Builtin(id.into())).collect();
        let users: Vec<TemplateRef> = self.templates.list.iter().map(|u| TemplateRef::User(u.id.clone())).collect();
        egui::ComboBox::from_id_salt("design-template")
            .width(ui.available_width() - 8.0)
            .selected_text(current)
            .show_ui(ui, |ui| {
                ui.weak(self.i18n.tr("tpl-builtin"));
                for r in &builtins {
                    if ui.selectable_label(self.batch.template_ref.as_ref() == Some(r), self.template_label(r)).clicked() {
                        pick = Some(r.clone());
                    }
                }
                ui.separator();
                ui.weak(self.i18n.tr("tpl-user"));
                if users.is_empty() {
                    ui.weak(self.i18n.tr("tpl-user-empty"));
                }
                for r in &users {
                    if ui.selectable_label(self.batch.template_ref.as_ref() == Some(r), self.template_label(r)).clicked() {
                        pick = Some(r.clone());
                    }
                }
            })
            .response
            .on_hover_text(self.i18n.tr(if modified { "tpl-modified-hint" } else { "tpl-pick-hint" }));
        if let Some(r) = pick {
            self.pick_template(r);
        }

        let local = self.design.short.is_some();
        let user_id = match &self.batch.template_ref {
            Some(TemplateRef::User(id)) if self.templates.user(id).is_some() => Some(id.clone()),
            _ => None,
        };
        ui.horizontal_wrapped(|ui| {
            let can_save = !local && modified && user_id.is_some();
            if ui.add_enabled(can_save, egui::Button::new(self.i18n.tr("tpl-save"))).on_hover_text(self.i18n.tr("tpl-save-hint")).clicked() {
                self.save_template();
            }
            if ui.button(self.i18n.tr("tpl-save-as")).on_hover_text(self.i18n.tr("tpl-save-as-hint")).clicked() {
                let base = match &self.batch.template_ref {
                    Some(r) if !local => self.template_label(r),
                    _ => self.i18n.tr("tpl-new-name"),
                };
                self.design.dialog = Some(LibDialog::SaveAs { name: self.templates.free_name(&base), error: None });
            }
            let can_dup = !local && self.batch.template_ref.is_some();
            if ui.add_enabled(can_dup, egui::Button::new(self.i18n.tr("tpl-duplicate"))).on_hover_text(self.i18n.tr("tpl-duplicate-hint")).clicked() {
                self.duplicate_template();
            }
            if let Some(id) = &user_id {
                let name = self.template_label(&TemplateRef::User(id.clone()));
                if ui.button(self.i18n.tr("tpl-rename")).clicked() {
                    self.design.dialog = Some(LibDialog::Rename { id: id.clone(), name: name.clone(), error: None });
                }
                if ui.button(self.i18n.tr("tpl-delete")).clicked() {
                    self.design.dialog = Some(LibDialog::Delete { id: id.clone(), name });
                }
            }
        });

        // Локальные правки выбранного шортса.
        let selected = self.cut.selected.and_then(|id| self.batch.short(id));
        let has_own = selected.is_some_and(|s| s.template.is_some());
        ui.add_enabled_ui(selected.is_some(), |ui| {
            ui.checkbox(&mut self.design.local, self.i18n.tr("tpl-local")).on_hover_text(self.i18n.tr("tpl-local-hint"));
        });
        if selected.is_none() {
            ui.weak(self.i18n.tr("tpl-local-no-short"));
        } else if has_own {
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(egui::Color32::from_rgb(255, 200, 60), self.i18n.tr("tpl-local-has"));
                if ui.button(self.i18n.tr("tpl-local-reset")).on_hover_text(self.i18n.tr("tpl-local-reset-hint")).clicked()
                    && let Some(id) = self.cut.selected
                {
                    self.design.last_edit = None;
                    self.design_apply(Snapshot::Short(id, None), true);
                }
            });
        }
    }

    /// «Сохранить»: перезаписать свой шаблон, из которого взят шаблон пачки.
    fn save_template(&mut self) {
        let Some(TemplateRef::User(id)) = self.batch.template_ref.clone() else { return };
        match self.templates.store.save(&id, &self.batch.template) {
            Ok(()) => {
                self.templates.refresh();
                self.status = Some(self.i18n.tr_args("tpl-saved", &[("name", self.template_label(&TemplateRef::User(id)).into())]));
            }
            Err(e) => self.status = Some(self.lib_error(&e)),
        }
    }

    /// «Дублировать»: копия шаблона библиотеки под свободным именем; пачка переходит на копию,
    /// несохранённые правки остаются правками уже относительно копии.
    fn duplicate_template(&mut self) {
        let Some(r) = self.batch.template_ref.clone() else { return };
        let Some(src) = self.templates.source(&r) else { return };
        let name = self.templates.free_name(&self.i18n.tr_args("tpl-copy-name", &[("name", self.template_label(&r).into())]));
        match self.templates.store.create(&name, &src) {
            Ok(id) => {
                self.templates.refresh();
                self.batch.template.name = name.clone();
                self.batch.template_ref = Some(TemplateRef::User(id));
                self.status = Some(self.i18n.tr_args("tpl-saved", &[("name", name.into())]));
            }
            Err(e) => self.status = Some(self.lib_error(&e)),
        }
    }

    fn lib_dialog_ui(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.design.dialog.take() else { return };
        let (mut close, mut submit) = (false, false);
        let tr = &self.i18n;
        let title = match &dialog {
            LibDialog::SaveAs { .. } => "tpl-save-as",
            LibDialog::Rename { .. } => "tpl-rename",
            LibDialog::Delete { .. } => "tpl-delete",
        };
        let modal = egui::Modal::new(egui::Id::new("tpl-dialog")).show(ctx, |ui| {
            ui.set_width(340.0);
            ui.heading(tr.tr(title));
            match &mut dialog {
                LibDialog::SaveAs { name, error } | LibDialog::Rename { name, error, .. } => {
                    ui.label(tr.tr("tpl-name"));
                    let edit = ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY));
                    edit.request_focus();
                    if let Some(e) = error {
                        ui.colored_label(ui.visuals().error_fg_color, e.as_str());
                    }
                    let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                    ui.horizontal(|ui| {
                        submit = ui.button(tr.tr("tpl-ok")).clicked() || enter;
                        close = ui.button(tr.tr("tpl-cancel")).clicked();
                    });
                }
                LibDialog::Delete { name, .. } => {
                    ui.label(tr.tr_args("tpl-delete-confirm", &[("name", name.clone().into())]));
                    ui.horizontal(|ui| {
                        submit = ui.button(tr.tr("tpl-delete")).clicked();
                        close = ui.button(tr.tr("tpl-cancel")).clicked();
                    });
                }
            }
        });
        if modal.should_close() {
            close = true;
        }
        if submit && !close {
            let result = match &dialog {
                LibDialog::SaveAs { name, .. } => self.save_template_as(name),
                LibDialog::Rename { id, name, .. } => self.rename_template(id, name),
                LibDialog::Delete { id, .. } => self.delete_template(id),
            };
            match result {
                Ok(()) => close = true,
                Err(e) => {
                    let msg = self.lib_error(&e);
                    match &mut dialog {
                        LibDialog::SaveAs { error, .. } | LibDialog::Rename { error, .. } => *error = Some(msg),
                        LibDialog::Delete { .. } => {
                            self.status = Some(msg);
                            close = true;
                        }
                    }
                }
            }
        }
        if !close {
            self.design.dialog = Some(dialog);
        }
    }

    fn delete_template(&mut self, id: &str) -> Result<(), LibraryError> {
        match self.templates.store.delete(id) {
            Ok(()) | Err(LibraryError::NotFound) => {}
            Err(e) => return Err(e),
        }
        self.templates.refresh();
        // Пачка остаётся со своим шаблоном, но уже ни на что не ссылается.
        if self.batch.template_ref == Some(TemplateRef::User(id.to_owned())) {
            self.batch.template_ref = None;
        }
        Ok(())
    }

    /// «Сохранить как»: текущий правящийся шаблон (пачки или шортса) — новым своим шаблоном.
    /// Шаблон пачки после этого ссылается на новый.
    fn save_template_as(&mut self, name: &str) -> Result<(), LibraryError> {
        let t = self.preview_template(true).clone();
        let id = self.templates.store.create(name, &t)?;
        self.templates.refresh();
        let name = self.template_label(&TemplateRef::User(id.clone()));
        if self.design.short.is_none() {
            self.batch.template.name = name.clone();
            self.batch.template_ref = Some(TemplateRef::User(id));
        }
        self.status = Some(self.i18n.tr_args("tpl-saved", &[("name", name.into())]));
        Ok(())
    }

    fn rename_template(&mut self, id: &str, name: &str) -> Result<(), LibraryError> {
        let t = self.templates.store.rename(id, name)?;
        self.templates.refresh();
        if self.batch.template_ref == Some(TemplateRef::User(id.to_owned())) {
            self.batch.template.name = t.name;
        }
        Ok(())
    }
}

/// Подстановки превью по выбранному шортсу (без выбора: «Часть 1», пустой заголовок).
pub(crate) fn preview_vars(app: &App) -> TextVars {
    match app.cut.selected.and_then(|id| app.batch.short(id)) {
        Some(s) => TextVars { part: app.batch.part_number(s.id), title: s.title.clone() },
        None => TextVars { part: 1, title: String::new() },
    }
}
