//! Форматы вывода и качество (SPEC §6): контейнеры, кодеки, пресеты качества, расширенные параметры,
//! звук, свои пресеты. Конкретные ключи ffmpeg — в `sc-render`; здесь только смысл и значения по умолчанию.

use std::ops::RangeInclusive;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Container {
    #[default]
    Mp4,
    /// В интерфейсе не показывается (нет в SPEC), но рендерится — из старых настроек.
    Mkv,
    Webm,
    Mov,
}

impl Container {
    /// Контейнеры, которые предлагает интерфейс.
    pub const UI: [Container; 3] = [Container::Mp4, Container::Webm, Container::Mov];

    pub fn extension(self) -> &'static str {
        match self {
            Container::Mp4 => "mp4",
            Container::Mkv => "mkv",
            Container::Webm => "webm",
            Container::Mov => "mov",
        }
    }

    /// Кодеки, которые помещаются в контейнер; первый — по умолчанию.
    pub fn codecs(self) -> &'static [Codec] {
        match self {
            Container::Mp4 => &[Codec::H264, Codec::Hevc, Codec::Av1],
            Container::Webm => &[Codec::Vp9, Codec::Av1],
            Container::Mov => &[Codec::ProRes, Codec::H264, Codec::Hevc],
            Container::Mkv => &Codec::ALL,
        }
    }

    /// Звуковые кодеки контейнера; первый — по умолчанию.
    pub fn audio_codecs(self) -> &'static [AudioCodec] {
        match self {
            Container::Mp4 => &[AudioCodec::Aac, AudioCodec::Opus],
            Container::Webm => &[AudioCodec::Opus],
            Container::Mov => &[AudioCodec::Aac, AudioCodec::Pcm],
            Container::Mkv => &[AudioCodec::Aac, AudioCodec::Opus, AudioCodec::Pcm],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Codec {
    #[default]
    H264,
    Hevc,
    Av1,
    Vp9,
    ProRes,
}

impl Codec {
    pub const ALL: [Codec; 5] = [Codec::H264, Codec::Hevc, Codec::Av1, Codec::Vp9, Codec::ProRes];

    /// У ProRes качество задаёт вариант (Proxy…4444), пресетов и битрейта нет.
    pub fn has_quality(self) -> bool {
        self != Codec::ProRes
    }

    /// Профиль выбирается только у H.264 (у остальных — единственный для 8 бит 4:2:0).
    pub fn has_profile(self) -> bool {
        self == Codec::H264
    }

    /// Диапазон CRF/QP: у GPU (VAAPI) — QP кодировщика.
    pub fn quality_range(self, gpu: bool) -> RangeInclusive<u8> {
        match (self, gpu) {
            (Codec::Av1, true) => 1..=255,
            (Codec::Av1 | Codec::Vp9, false) => 0..=63,
            _ => 0..=51,
        }
    }
}

/// Варианты ProRes (`prores_ks -profile:v 0…4`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProResProfile {
    Proxy,
    Lt,
    #[default]
    Standard,
    Hq,
    #[serde(rename = "4444")]
    P4444,
}

impl ProResProfile {
    pub const ALL: [ProResProfile; 5] =
        [ProResProfile::Proxy, ProResProfile::Lt, ProResProfile::Standard, ProResProfile::Hq, ProResProfile::P4444];

    /// Номер для `-profile:v`.
    pub fn id(self) -> u8 {
        self as u8
    }

    pub fn label(self) -> &'static str {
        match self {
            ProResProfile::Proxy => "422 Proxy",
            ProResProfile::Lt => "422 LT",
            ProResProfile::Standard => "422",
            ProResProfile::Hq => "422 HQ",
            ProResProfile::P4444 => "4444",
        }
    }
}

/// Пресет качества (SPEC §6); `Custom` — значения из расширенных настроек.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QualityPreset {
    High,
    #[default]
    Balanced,
    Fast,
    Custom,
}

impl QualityPreset {
    pub const PRESETS: [QualityPreset; 3] = [QualityPreset::High, QualityPreset::Balanced, QualityPreset::Fast];
}

/// Как кодировщик держит качество: средний битрейт (VBR) или постоянное качество (CRF/QP).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RateMode {
    #[default]
    Bitrate,
    Quality,
}

/// Скорость CPU-кодировщика (у VAAPI не влияет); переводится в `-preset`/`-cpu-used` по кодировщику.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Speed {
    Slow,
    Medium,
    #[default]
    Fast,
    Faster,
}

impl Speed {
    pub const ALL: [Speed; 4] = [Speed::Slow, Speed::Medium, Speed::Fast, Speed::Faster];
}

/// Профиль H.264.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoProfile {
    /// Решает кодировщик (у x264 и VAAPI — High).
    #[default]
    Auto,
    Baseline,
    Main,
    High,
}

impl VideoProfile {
    pub const ALL: [VideoProfile; 4] = [VideoProfile::Auto, VideoProfile::Baseline, VideoProfile::Main, VideoProfile::High];
}

/// Параметры видеокодировщика — то, что меняется в расширенных настройках.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VideoEncode {
    pub rate: RateMode,
    /// Средний битрейт (maxrate ×1.5, bufsize ×2), кбит/с.
    pub bitrate_kbps: u32,
    /// CRF (CPU) или QP (VAAPI).
    pub quality: u8,
    pub speed: Speed,
    pub profile: VideoProfile,
    /// Интервал ключевых кадров, с; 0 — как решит кодировщик.
    pub gop_seconds: f32,
}

impl Default for VideoEncode {
    fn default() -> Self {
        preset(Codec::H264, true, QualityPreset::Balanced)
    }
}

/// Значения пресета для кодека; `gpu` — кодировщик VAAPI. `Custom` здесь — как `Balanced`.
///
/// Сбалансированный H.264 — решение этапа 0.4 (VAAPI VBR 12 Мбит/с, x264 veryfast 12 Мбит/с);
/// AV1 и VP9 — постоянное качество, CRF 32 давал ~4 Мбит/с при SSIM как у H.264 12 Мбит/с.
pub fn preset(codec: Codec, gpu: bool, q: QualityPreset) -> VideoEncode {
    let level = match q {
        QualityPreset::High => 0,
        QualityPreset::Balanced | QualityPreset::Custom => 1,
        QualityPreset::Fast => 2,
    };
    let bitrate = |kbps: [u32; 3], speed: [Speed; 3]| VideoEncode {
        rate: RateMode::Bitrate,
        bitrate_kbps: kbps[level],
        quality: 23,
        speed: speed[level],
        profile: VideoProfile::Auto,
        gop_seconds: 0.0,
    };
    let quality = |crf: [u8; 3], speed: [Speed; 3], kbps: u32| VideoEncode {
        rate: RateMode::Quality,
        bitrate_kbps: kbps,
        quality: crf[level],
        speed: speed[level],
        profile: VideoProfile::Auto,
        gop_seconds: 0.0,
    };
    use Speed::*;
    match (codec, gpu) {
        (Codec::H264, true) => bitrate([16_000, 12_000, 8_000], [Medium; 3]),
        (Codec::H264, false) => bitrate([16_000, 12_000, 10_000], [Medium, Fast, Faster]),
        (Codec::Hevc, true) => bitrate([14_000, 10_000, 8_000], [Medium; 3]),
        (Codec::Hevc, false) => bitrate([12_000, 10_000, 8_000], [Medium, Fast, Faster]),
        (Codec::Av1, true) => bitrate([10_000, 8_000, 6_000], [Medium; 3]),
        (Codec::Av1, false) => quality([28, 32, 35], [Slow, Medium, Fast], 6_000),
        (Codec::Vp9, _) => quality([28, 32, 34], [Slow, Medium, Faster], 8_000),
        (Codec::ProRes, _) => VideoEncode { rate: RateMode::Quality, ..bitrate([0; 3], [Medium; 3]) },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioCodec {
    #[default]
    Aac,
    Opus,
    /// PCM 16 бит без сжатия (MOV для монтажа).
    Pcm,
}

impl AudioCodec {
    pub fn label(self) -> &'static str {
        match self {
            AudioCodec::Aac => "AAC",
            AudioCodec::Opus => "Opus",
            AudioCodec::Pcm => "PCM",
        }
    }

    /// Есть ли у кодека битрейт.
    pub fn has_bitrate(self) -> bool {
        self != AudioCodec::Pcm
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioEncode {
    pub codec: AudioCodec,
    pub bitrate_kbps: u32,
}

impl Default for AudioEncode {
    fn default() -> Self {
        Self { codec: AudioCodec::Aac, bitrate_kbps: 192 }
    }
}

/// Готовые разрешения вывода (9:16); `None` в настройках — холст шаблона (1080×1920).
pub const RESOLUTIONS: [(u32, u32); 3] = [(720, 1280), (1080, 1920), (1440, 2560)];

/// Частоты кадров на выбор; `None` — как у исходника.
pub const FRAME_RATES: [f64; 2] = [30.0, 60.0];

/// Свой пресет вывода: формат и качество, без папки и имени.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OutputPreset {
    pub name: String,
    pub container: Container,
    pub codec: Codec,
    pub prores: ProResProfile,
    pub encoder: crate::EncoderPref,
    pub quality: QualityPreset,
    pub custom: VideoEncode,
    pub audio: AudioEncode,
    pub resolution: Option<crate::Canvas>,
    pub fps: Option<f64>,
}

impl Default for OutputPreset {
    fn default() -> Self {
        Self::from_output(String::new(), &crate::OutputSettings::default())
    }
}

impl OutputPreset {
    pub fn from_output(name: String, o: &crate::OutputSettings) -> Self {
        Self {
            name,
            container: o.container,
            codec: o.codec,
            prores: o.prores,
            encoder: o.encoder,
            quality: o.quality,
            custom: o.custom,
            audio: o.audio,
            resolution: o.resolution,
            fps: o.fps,
        }
    }

    pub fn apply_to(&self, o: &mut crate::OutputSettings) {
        o.container = self.container;
        o.codec = self.codec;
        o.prores = self.prores;
        o.encoder = self.encoder;
        o.quality = self.quality;
        o.custom = self.custom;
        o.audio = self.audio;
        o.resolution = self.resolution;
        o.fps = self.fps;
        o.normalize();
    }

    /// Совпадает ли формат и качество с настройками (для подсветки выбранного пресета).
    pub fn matches(&self, o: &crate::OutputSettings) -> bool {
        Self::from_output(self.name.clone(), o) == *self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OutputSettings;

    #[test]
    fn balanced_h264_is_phase0_default() {
        let v = preset(Codec::H264, true, QualityPreset::Balanced);
        assert_eq!((v.rate, v.bitrate_kbps), (RateMode::Bitrate, 12_000));
        assert_eq!(preset(Codec::H264, false, QualityPreset::Balanced).speed, Speed::Fast);
        assert_eq!(VideoEncode::default(), v);
        assert_eq!(preset(Codec::Vp9, false, QualityPreset::Fast).speed, Speed::Faster);
    }

    #[test]
    fn every_container_has_codecs_and_audio() {
        for c in Container::UI {
            assert!(!c.codecs().is_empty() && !c.audio_codecs().is_empty());
        }
        assert_eq!(Container::Webm.codecs()[0], Codec::Vp9);
        assert_eq!(Container::Mov.codecs()[0], Codec::ProRes);
    }

    #[test]
    fn normalize_fixes_codec_and_audio_for_container() {
        let mut o = OutputSettings { codec: Codec::H264, ..Default::default() };
        o.container = Container::Webm;
        o.normalize();
        assert_eq!((o.codec, o.audio.codec), (Codec::Vp9, AudioCodec::Opus));
        o.container = Container::Mp4;
        o.normalize();
        // Opus в MP4 допустим — остаётся; AV1 переезжает из WebM в MP4
        assert_eq!(o.audio.codec, AudioCodec::Opus);
        o.codec = Codec::Av1;
        o.container = Container::Webm;
        o.normalize();
        assert_eq!(o.codec, Codec::Av1);
    }

    #[test]
    fn video_params_follow_preset_or_custom() {
        let mut o = OutputSettings { quality: QualityPreset::High, ..Default::default() };
        assert_eq!(o.video(true).bitrate_kbps, 16_000);
        o.quality = QualityPreset::Custom;
        o.custom.bitrate_kbps = 5_000;
        assert_eq!(o.video(true).bitrate_kbps, 5_000);
    }

    #[test]
    fn presets_roundtrip_toml() {
        let mut o = OutputSettings { container: Container::Mov, codec: Codec::ProRes, ..Default::default() };
        o.prores = ProResProfile::P4444;
        o.audio.codec = AudioCodec::Pcm;
        let p = OutputPreset::from_output("Монтаж".into(), &o);
        let text = toml::to_string(&p).unwrap();
        assert!(text.contains("prores = \"4444\""), "{text}");
        let back: OutputPreset = toml::from_str(&text).unwrap();
        assert_eq!(back, p);
        let mut fresh = OutputSettings::default();
        back.apply_to(&mut fresh);
        assert!(p.matches(&fresh));
        assert_eq!(fresh.prores, ProResProfile::P4444);
    }
}
