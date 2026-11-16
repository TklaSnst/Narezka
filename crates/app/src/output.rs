//! «Экспорт» → формат и качество (SPEC §6): контейнер, кодек, кодировщик, пресет, расширенные настройки,
//! разрешение и частота, звук, свои пресеты. Недоступное по `Capabilities` — неактивно с подсказкой.

use eframe::egui;
use sc_core::{
    Canvas, Codec, Container, EncoderPref, FRAME_RATES, OutputPreset, OutputSettings, ProResProfile,
    QualityPreset, RESOLUTIONS, RateMode, Speed, VideoProfile,
};
use sc_media::{Capabilities, EncoderKind};

use crate::App;

/// Состояние блока между кадрами.
#[derive(Default)]
pub struct OutputUi {
    /// Выбрано «Своё» разрешение (поле ширины открыто, даже если число совпало с готовым).
    custom_res: bool,
    /// Имя нового пресета, пока открыто поле «Сохранить как».
    new_preset: Option<String>,
    /// Поле имени только что открыто — дать ему фокус.
    focus_name: bool,
    /// Настройки поменялись, а запись отложена до отпускания мыши (не писать файл на каждом кадре перетаскивания).
    dirty: bool,
}

pub fn container_label(c: Container) -> &'static str {
    match c {
        Container::Mp4 => "MP4",
        Container::Mkv => "MKV",
        Container::Webm => "WebM",
        Container::Mov => "MOV",
    }
}

pub fn codec_label(c: Codec) -> &'static str {
    match c {
        Codec::H264 => "H.264",
        Codec::Hevc => "H.265 (HEVC)",
        Codec::Av1 => "AV1",
        Codec::Vp9 => "VP9",
        Codec::ProRes => "ProRes",
    }
}

fn quality_key(q: QualityPreset) -> &'static str {
    match q {
        QualityPreset::High => "quality-high",
        QualityPreset::Balanced => "quality-balanced",
        QualityPreset::Fast => "quality-fast",
        QualityPreset::Custom => "quality-custom",
    }
}

fn speed_key(s: Speed) -> &'static str {
    match s {
        Speed::Slow => "speed-slow",
        Speed::Medium => "speed-medium",
        Speed::Fast => "speed-fast",
        Speed::Faster => "speed-faster",
    }
}

fn profile_key(p: VideoProfile) -> &'static str {
    match p {
        VideoProfile::Auto => "profile-auto",
        VideoProfile::Baseline => "profile-baseline",
        VideoProfile::Main => "profile-main",
        VideoProfile::High => "profile-high",
    }
}

/// Будет ли кодек кодироваться на GPU при этом предпочтении.
pub fn uses_gpu(caps: Option<&Capabilities>, out: &OutputSettings) -> bool {
    caps.and_then(|c| c.encoder_for(out.codec, out.encoder)).is_some_and(|e| e.kind == EncoderKind::Gpu)
}

impl App {
    /// Краткое описание качества для строки «Кодировщик: …».
    pub(crate) fn quality_summary(&self, gpu: bool) -> String {
        let out = &self.batch.output;
        if out.codec == Codec::ProRes {
            return format!("ProRes {}", out.prores.label());
        }
        let v = out.video(gpu);
        let mut s = self.i18n.tr(quality_key(out.quality));
        s += " · ";
        s += &match v.rate {
            RateMode::Bitrate => self.i18n.tr_args("export-mbps", &[("mbps", format!("{:.1}", v.bitrate_kbps as f64 / 1000.0).into())]),
            RateMode::Quality => format!("{} {}", if gpu { "QP" } else { "CRF" }, v.quality),
        };
        s
    }

    pub(crate) fn format_ui(&mut self, ui: &mut egui::Ui) {
        let before = self.batch.output.clone();
        let caps = self.caps.clone();
        let caps = caps.as_ref();
        let canvas = self.batch.template.canvas;
        let i18n = &self.i18n;
        let ou = &mut self.output_ui;
        let out = &mut self.batch.output;
        let codec_ok = |c: Codec| caps.is_none_or(|k| k.has_codec(c));

        egui::Grid::new("format").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            ui.label(i18n.tr("export-container"));
            ui.horizontal(|ui| {
                for c in Container::UI {
                    ui.selectable_value(&mut out.container, c, container_label(c));
                }
                if out.container == Container::Mkv {
                    ui.selectable_value(&mut out.container, Container::Mkv, container_label(Container::Mkv));
                }
            });
            ui.end_row();
            out.normalize();

            ui.label(i18n.tr("export-codec"));
            ui.horizontal(|ui| {
                for &c in out.container.codecs() {
                    let ok = codec_ok(c);
                    let r = ui.add_enabled(ok, egui::Button::selectable(out.codec == c, codec_label(c)));
                    let r = r.on_disabled_hover_text(i18n.tr("export-codec-missing"));
                    if r.clicked() {
                        out.codec = c;
                    }
                }
            });
            ui.end_row();
            // Свои значения — для конкретного кодека (CRF 32 у VP9 и у x264 — разное качество).
            if out.codec != before.codec && out.quality == QualityPreset::Custom {
                out.quality = QualityPreset::Balanced;
            }

            if out.codec == Codec::ProRes {
                ui.label(i18n.tr("export-prores"));
                ui.horizontal(|ui| {
                    for p in ProResProfile::ALL {
                        ui.selectable_value(&mut out.prores, p, p.label());
                    }
                });
                ui.end_row();
            }

            ui.label(i18n.tr("export-encoder-kind"));
            ui.horizontal(|ui| {
                let codec = out.codec;
                let kind_ok = |k| caps.is_none_or(|c| c.encoder_for(codec, k).is_some());
                for (pref, key) in
                    [(EncoderPref::Auto, "encoder-auto"), (EncoderPref::Gpu, "caps-kind-gpu"), (EncoderPref::Cpu, "caps-kind-cpu")]
                {
                    let ok = kind_ok(pref);
                    let r = ui.add_enabled(ok, egui::Button::selectable(out.encoder == pref, i18n.tr(key)));
                    if r.on_disabled_hover_text(i18n.tr("export-encoder-kind-missing")).clicked() {
                        out.encoder = pref;
                    }
                }
                if let Some(e) = caps.and_then(|c| c.encoder_for(out.codec, out.encoder)) {
                    ui.weak(e.name);
                }
            });
            ui.end_row();

            if out.codec.has_quality() {
                ui.label(i18n.tr("export-quality"));
                ui.horizontal(|ui| {
                    for q in QualityPreset::PRESETS {
                        ui.selectable_value(&mut out.quality, q, i18n.tr(quality_key(q)));
                    }
                    if out.quality == QualityPreset::Custom {
                        ui.selectable_value(&mut out.quality, QualityPreset::Custom, i18n.tr("quality-custom"));
                    }
                });
                ui.end_row();
            }

            ui.label(i18n.tr("export-resolution"));
            ui.horizontal(|ui| {
                let current = out.resolution.unwrap_or(canvas);
                let label = |c: Canvas| format!("{}×{}", c.width, c.height);
                egui::ComboBox::from_id_salt("resolution")
                    .selected_text(if ou.custom_res {
                        i18n.tr("export-resolution-custom")
                    } else if out.resolution.is_none() {
                        i18n.tr_args("export-resolution-template", &[("size", label(current).into())])
                    } else {
                        label(current)
                    })
                    .show_ui(ui, |ui| {
                        for (w, h) in RESOLUTIONS {
                            let c = Canvas { width: w, height: h };
                            let text = if c == canvas { i18n.tr_args("export-resolution-template", &[("size", label(c).into())]) } else { label(c) };
                            if ui.selectable_label(!ou.custom_res && current == c, text).clicked() {
                                out.resolution = (c != canvas).then_some(c);
                                ou.custom_res = false;
                            }
                        }
                        if ui.selectable_label(ou.custom_res, i18n.tr("export-resolution-custom")).clicked() {
                            ou.custom_res = true;
                        }
                    });
                if ou.custom_res {
                    // Пропорции — холста шаблона, стороны чётные (требование yuv420p).
                    let mut w = current.width;
                    ui.add(egui::DragValue::new(&mut w).range(360..=4320).speed(2.0).suffix(" px"));
                    let w = w / 2 * 2;
                    let h = ((w as f64 * canvas.height as f64 / canvas.width as f64) / 2.0).round() as u32 * 2;
                    ui.label(format!("× {h}"));
                    let c = Canvas { width: w, height: h };
                    out.resolution = (c != canvas).then_some(c);
                }
            });
            ui.end_row();

            ui.label(i18n.tr("export-fps"));
            ui.horizontal(|ui| {
                ui.selectable_value(&mut out.fps, None, i18n.tr("export-fps-source"));
                for f in FRAME_RATES {
                    ui.selectable_value(&mut out.fps, Some(f), format!("{f:.0}"));
                }
            });
            ui.end_row();

            ui.label(i18n.tr("export-audio"));
            ui.horizontal(|ui| {
                for &a in out.container.audio_codecs() {
                    let ok = caps.is_none_or(|c| c.has_audio_encoder(sc_render::encode::audio_encoder(a)));
                    let r = ui.add_enabled(ok, egui::Button::selectable(out.audio.codec == a, a.label()));
                    if r.on_disabled_hover_text(i18n.tr("export-audio-missing")).clicked() {
                        out.audio.codec = a;
                    }
                }
                if out.audio.codec.has_bitrate() {
                    ui.add(egui::DragValue::new(&mut out.audio.bitrate_kbps).range(32..=512).speed(4.0).suffix(" kbps"));
                }
            });
            ui.end_row();
        });

        if out.codec.has_quality() {
            let gpu = uses_gpu(caps, out);
            egui::CollapsingHeader::new(i18n.tr("export-advanced")).id_salt("advanced").show(ui, |ui| {
                let mut v = out.video(gpu);
                egui::Grid::new("advanced").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                    ui.label(i18n.tr("export-rate"));
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut v.rate, RateMode::Bitrate, i18n.tr("export-rate-bitrate"));
                        let q = if gpu { "export-rate-qp" } else { "export-rate-crf" };
                        ui.selectable_value(&mut v.rate, RateMode::Quality, i18n.tr(q));
                    });
                    ui.end_row();
                    match v.rate {
                        RateMode::Bitrate => {
                            ui.label(i18n.tr("export-bitrate"));
                            let mut mbps = v.bitrate_kbps as f64 / 1000.0;
                            ui.add(egui::DragValue::new(&mut mbps).range(0.5..=200.0).speed(0.1).max_decimals(1).suffix(" Mbit/s"));
                            v.bitrate_kbps = (mbps * 1000.0).round() as u32;
                        }
                        RateMode::Quality => {
                            ui.label(i18n.tr(if gpu { "export-rate-qp" } else { "export-rate-crf" }));
                            ui.add(egui::Slider::new(&mut v.quality, out.codec.quality_range(gpu)))
                                .on_hover_text(i18n.tr("export-crf-hint"));
                        }
                    }
                    ui.end_row();

                    ui.label(i18n.tr("export-speed"));
                    ui.add_enabled_ui(!gpu, |ui| {
                        ui.horizontal(|ui| {
                            for s in Speed::ALL {
                                ui.selectable_value(&mut v.speed, s, i18n.tr(speed_key(s)));
                            }
                        })
                    })
                    .response
                    .on_disabled_hover_text(i18n.tr("export-speed-gpu"));
                    ui.end_row();

                    if out.codec.has_profile() {
                        ui.label(i18n.tr("export-profile"));
                        ui.horizontal(|ui| {
                            for p in VideoProfile::ALL {
                                ui.selectable_value(&mut v.profile, p, i18n.tr(profile_key(p)));
                            }
                        });
                        ui.end_row();
                    }

                    ui.label(i18n.tr("export-gop"));
                    ui.horizontal(|ui| {
                        ui.add(egui::DragValue::new(&mut v.gop_seconds).range(0.0..=20.0).speed(0.1).max_decimals(1).suffix(" s"));
                        if v.gop_seconds == 0.0 {
                            ui.weak(i18n.tr("export-gop-auto"));
                        }
                    });
                    ui.end_row();
                });
                if v != out.video(gpu) {
                    out.custom = v;
                    out.quality = QualityPreset::Custom;
                }
            });
        } else {
            ui.weak(i18n.tr("export-prores-hint"));
        }

        self.presets_ui(ui);

        if self.batch.output != before {
            self.output_ui.dirty = true;
        }
        if self.output_ui.dirty && !ui.input(|i| i.pointer.any_down()) {
            self.output_ui.dirty = false;
            self.settings.output = self.batch.output.clone();
            self.save_settings();
        }
    }

    fn presets_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(self.i18n.tr("export-presets"));
            let current = self.settings.output_presets.iter().position(|p| p.matches(&self.batch.output));
            let text = current.map_or_else(|| self.i18n.tr("export-preset-none"), |i| self.settings.output_presets[i].name.clone());
            let mut apply = None;
            ui.add_enabled_ui(!self.settings.output_presets.is_empty(), |ui| {
                egui::ComboBox::from_id_salt("output-presets").selected_text(text).show_ui(ui, |ui| {
                    for (i, p) in self.settings.output_presets.iter().enumerate() {
                        if ui.selectable_label(current == Some(i), &p.name).clicked() {
                            apply = Some(i);
                        }
                    }
                });
            });
            if let Some(i) = apply {
                self.settings.output_presets[i].apply_to(&mut self.batch.output);
                self.output_ui.custom_res = false;
            }
            match &mut self.output_ui.new_preset {
                None => {
                    if ui.button(self.i18n.tr("export-preset-save")).clicked() {
                        self.output_ui.new_preset = Some(String::new());
                        self.output_ui.focus_name = true;
                    }
                    if let Some(i) = current
                        && ui.button(self.i18n.tr("export-preset-delete")).clicked()
                    {
                        self.settings.output_presets.remove(i);
                        self.save_settings();
                    }
                }
                Some(name) => {
                    let edit = ui.add(egui::TextEdit::singleline(name).hint_text(self.i18n.tr("export-preset-name")).desired_width(160.0));
                    if std::mem::take(&mut self.output_ui.focus_name) {
                        edit.request_focus();
                    }
                    let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    let name = name.trim().to_owned();
                    let ok = ui.add_enabled(!name.is_empty(), egui::Button::new(self.i18n.tr("export-preset-ok"))).clicked();
                    let cancel = ui.button(self.i18n.tr("export-preset-cancel")).clicked()
                        || ui.input(|i| i.key_pressed(egui::Key::Escape));
                    if (ok || enter) && !name.is_empty() {
                        let preset = OutputPreset::from_output(name.clone(), &self.batch.output);
                        // То же имя — замена.
                        match self.settings.output_presets.iter_mut().find(|p| p.name == name) {
                            Some(p) => *p = preset,
                            None => self.settings.output_presets.push(preset),
                        }
                        self.output_ui.new_preset = None;
                        self.save_settings();
                    } else if cancel {
                        self.output_ui.new_preset = None;
                    }
                }
            }
        });
    }
}
