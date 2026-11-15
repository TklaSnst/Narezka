//! Список субтитров выбранного шортса на экране «Нарезка»: фразы так, как их разложит шаблон, с правкой текста.
//! Правка фразы делит новые слова между словами речи (`SubtitleEdits::set_range_text`), время берётся у речи.

use eframe::egui::{self, Color32, RichText};
use sc_core::{Layer, ShortId, SubtitleTrack};

use crate::App;
use crate::fmt;
use crate::subtitles::{short_words, speech_words};

/// Раскладка, если в шаблоне шортса нет слоя субтитров (как у «+ Слой → Субтитры»).
const DEFAULT_LAYOUT: (u32, u32) = (3, 2);
/// Высота списка, px.
const LIST_H: f32 = 180.0;

/// Фраза, которую сейчас правят: шортс, номер первого слова фразы и текст в поле.
pub struct SubEdit {
    short: ShortId,
    first: usize,
    text: String,
}

enum Act {
    Seek(f64),
    Commit { first: usize, end: usize, text: String },
    Reset { first: usize, end: usize },
}

impl App {
    pub(crate) fn subtitle_list_ui(&mut self, ui: &mut egui::Ui) {
        let Some(id) = self.cut.selected else { return };
        let Some(short) = self.batch.short(id).cloned() else { return };
        let Some(t) = self.transcript_of(short.source_id) else { return };

        let template = self.batch.template_for(id);
        let layout = template.layers.iter().find_map(|l| match l {
            &Layer::Subtitles { words_per_line, lines, .. } => Some((words_per_line, lines)),
            _ => None,
        });
        let (wpl, lines) = layout.unwrap_or(DEFAULT_LAYOUT);
        let originals = speech_words(&t, &short);
        let edited = short.subtitles.apply(originals.iter().copied());
        let track = SubtitleTrack::new(short_words(&t, &short), wpl, lines);
        let pos = self.player.as_ref().map_or(0.0, |p| p.state.time);

        ui.add_space(4.0);
        egui::CollapsingHeader::new(RichText::new(self.i18n.tr("sublist-title")).strong())
            .id_salt("sublist")
            .default_open(true)
            .show(ui, |ui| {
                if layout.is_none() {
                    ui.weak(self.i18n.tr("sublist-no-layer"));
                }
                if track.phrases.is_empty() {
                    ui.weak(self.i18n.tr("sublist-empty"));
                }
                let mut act = None;
                egui::ScrollArea::vertical().max_height(LIST_H).auto_shrink([false, true]).show(ui, |ui| {
                    for p in &track.phrases {
                        let current = p.start <= pos && pos < p.end;
                        let srcs = edited[p.words.start].src..edited[p.words.end - 1].src + 1;
                        let changed = originals[srcs.clone()].iter().any(|w| short.subtitles.text(w.3).is_some());
                        let phrase_text = || track.words[p.words.clone()].iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ");
                        ui.horizontal(|ui| {
                            let mut time = RichText::new(fmt::duration(p.start - short.start)).monospace().small();
                            if current {
                                time = time.color(Color32::from_rgb(235, 200, 70));
                            }
                            if ui.add(egui::Button::new(time).small().frame(false)).on_hover_text(self.i18n.tr("sublist-seek")).clicked() {
                                act = Some(Act::Seek(p.start));
                            }
                            let editing = self.cut.sub_edit.as_mut().filter(|e| e.short == id && e.first == p.words.start);
                            let mut text = editing.as_ref().map_or_else(phrase_text, |e| e.text.clone());
                            let width = ui.available_width() - if changed { 28.0 } else { 0.0 };
                            let r = ui.add(egui::TextEdit::singleline(&mut text).desired_width(width.max(60.0)));
                            if r.changed() {
                                self.cut.sub_edit = Some(SubEdit { short: id, first: p.words.start, text: text.clone() });
                            }
                            // Enter или уход из поля — сохранить (Esc — забыть правку).
                            if r.lost_focus() {
                                let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));
                                if let Some(e) = self.cut.sub_edit.take_if(|e| e.short == id && e.first == p.words.start)
                                    && !esc
                                    && e.text != phrase_text()
                                {
                                    act = Some(Act::Commit { first: p.words.start, end: p.words.end, text: e.text });
                                }
                            }
                            if changed && ui.small_button("↺").on_hover_text(self.i18n.tr("sublist-reset")).clicked() {
                                act = Some(Act::Reset { first: srcs.start, end: srcs.end });
                            }
                        });
                    }
                });
                ui.weak(self.i18n.tr("sublist-hint"));

                let mut s = short.clone();
                match act {
                    None => return,
                    Some(Act::Seek(at)) => {
                        if let Some(p) = &mut self.player {
                            p.seek(at, true);
                        }
                        return;
                    }
                    Some(Act::Commit { first, end, text }) => s.subtitles.set_range_text(&originals, &edited, first..end, &text),
                    Some(Act::Reset { first, end }) => s.subtitles.reset_texts(&originals, first..end),
                }
                if let Err(e) = self.batch.update_short(s) {
                    log::warn!("правка субтитров: {e}");
                }
            });
    }
}
