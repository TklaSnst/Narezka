//! Что умеет системный ffmpeg: версия, нужные фильтры, рабочие кодировщики.
//!
//! Список `-encoders` не гарантирует, что кодировщик работает (`av1_vaapi` есть в сборке, но RX 6600
//! его не умеет), поэтому каждый кандидат проверяется пробным кодированием нескольких кадров.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;

use sc_core::{Codec, EncoderPref};

use crate::process;

const LIST_TIMEOUT: Duration = Duration::from_secs(10);
const TRIAL_TIMEOUT: Duration = Duration::from_secs(15);

/// Фильтры, без которых не построить граф шортса (ARCHITECTURE §6).
pub const REQUIRED_FILTERS: &[&str] =
    &["scale", "crop", "gblur", "overlay", "alphamerge", "split", "concat", "atrim", "asetpts", "ass"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncoderKind {
    Gpu,
    Cpu,
}

/// Кандидаты по кодекам; GPU — через VAAPI.
const CANDIDATES: &[(Codec, EncoderKind, &str)] = &[
    (Codec::H264, EncoderKind::Gpu, "h264_vaapi"),
    (Codec::H264, EncoderKind::Cpu, "libx264"),
    (Codec::Hevc, EncoderKind::Gpu, "hevc_vaapi"),
    (Codec::Hevc, EncoderKind::Cpu, "libx265"),
    (Codec::Av1, EncoderKind::Gpu, "av1_vaapi"),
    (Codec::Av1, EncoderKind::Cpu, "libsvtav1"),
    (Codec::Vp9, EncoderKind::Cpu, "libvpx-vp9"),
    (Codec::ProRes, EncoderKind::Cpu, "prores_ks"),
];

#[derive(Debug, Clone, PartialEq)]
pub struct EncoderInfo {
    pub codec: Codec,
    pub kind: EncoderKind,
    pub name: &'static str,
    /// Пробное кодирование прошло.
    pub works: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Capabilities {
    /// `None` — ffmpeg не найден или не запускается.
    pub ffmpeg_version: Option<String>,
    pub ffprobe: bool,
    /// Устройство VAAPI (`/dev/dri/renderD*`), если есть.
    pub vaapi_device: Option<PathBuf>,
    pub encoders: Vec<EncoderInfo>,
    /// Обязательные фильтры, которых нет в сборке.
    pub missing_filters: Vec<&'static str>,
    /// Звуковые кодировщики из [`AUDIO_ENCODERS`], которые есть в сборке.
    pub audio_encoders: Vec<&'static str>,
}

/// Звуковые кодировщики вывода (AAC, Opus, PCM): встроенные, кроме libopus.
pub const AUDIO_ENCODERS: &[&str] = &["aac", "libopus", "pcm_s16le"];

impl Capabilities {
    /// Полная проверка; занимает 1–3 с, вызывать в фоне.
    pub fn detect() -> Self {
        let Some(ffmpeg_version) = ffmpeg_version() else {
            return Self { ffprobe: has_ffprobe(), ..Default::default() };
        };
        let vaapi_device = find_render_node();
        let listed = list_names("-encoders");
        let filters = list_names("-filters");
        let missing_filters = REQUIRED_FILTERS.iter().copied().filter(|f| !filters.iter().any(|n| n == f)).collect();

        let trials: Vec<_> = CANDIDATES
            .iter()
            .filter(|(_, _, name)| listed.iter().any(|n| n == name))
            .filter(|(_, kind, _)| *kind == EncoderKind::Cpu || vaapi_device.is_some())
            .map(|&(codec, kind, name)| {
                let dev = vaapi_device.clone();
                (codec, kind, name, thread::spawn(move || trial_encode(name, kind, dev.as_deref())))
            })
            .collect();
        let mut encoders: Vec<EncoderInfo> = CANDIDATES
            .iter()
            .map(|&(codec, kind, name)| EncoderInfo { codec, kind, name, works: false })
            .collect();
        for (_, _, name, t) in trials {
            let works = t.join().unwrap_or(false);
            if let Some(e) = encoders.iter_mut().find(|e| e.name == name) {
                e.works = works;
            }
        }

        let audio_encoders = AUDIO_ENCODERS.iter().copied().filter(|a| listed.iter().any(|n| n == a)).collect();
        Self {
            ffmpeg_version: Some(ffmpeg_version),
            ffprobe: has_ffprobe(),
            vaapi_device,
            encoders,
            missing_filters,
            audio_encoders,
        }
    }

    /// Можно ли импортировать и рендерить вообще.
    pub fn usable(&self) -> bool {
        self.ffmpeg_version.is_some() && self.ffprobe
    }

    pub fn works(&self, name: &str) -> bool {
        self.encoders.iter().any(|e| e.name == name && e.works)
    }

    /// Кодировщик для кодека с учётом предпочтения; `Auto` — GPU, если он работает.
    pub fn encoder_for(&self, codec: Codec, pref: EncoderPref) -> Option<&EncoderInfo> {
        let pick = |kind| self.encoders.iter().find(|e| e.codec == codec && e.kind == kind && e.works);
        match pref {
            EncoderPref::Gpu => pick(EncoderKind::Gpu),
            EncoderPref::Cpu => pick(EncoderKind::Cpu),
            EncoderPref::Auto => pick(EncoderKind::Gpu).or_else(|| pick(EncoderKind::Cpu)),
        }
    }

    /// Есть ли рабочий кодировщик для кодека (GPU или CPU).
    pub fn has_codec(&self, codec: Codec) -> bool {
        self.encoder_for(codec, EncoderPref::Auto).is_some()
    }

    pub fn has_audio_encoder(&self, name: &str) -> bool {
        self.audio_encoders.contains(&name)
    }

    pub fn working_encoders(&self, kind: EncoderKind) -> impl Iterator<Item = &EncoderInfo> {
        self.encoders.iter().filter(move |e| e.kind == kind && e.works)
    }
}

fn ffmpeg_version() -> Option<String> {
    let out = process::run("ffmpeg", Command::new("ffmpeg").arg("-version"), LIST_TIMEOUT).ok()?;
    if !out.success {
        return None;
    }
    // «ffmpeg version 8.0.1-3ubuntu2 Copyright …» → «8.0.1-3ubuntu2»
    out.stdout.lines().next()?.strip_prefix("ffmpeg version ")?.split_whitespace().next().map(str::to_owned)
}

fn has_ffprobe() -> bool {
    process::run("ffprobe", Command::new("ffprobe").arg("-version"), LIST_TIMEOUT).is_ok_and(|o| o.success)
}

/// Имена из `ffmpeg -encoders` / `-filters`: второе слово строки «флаги имя описание…».
/// Строки легенды («V..... = Video») пропускаются; разделителя `---` у `-filters` в ffmpeg 8 нет.
fn list_names(flag: &str) -> Vec<String> {
    let Ok(out) = process::run("ffmpeg", Command::new("ffmpeg").args(["-hide_banner", flag]), LIST_TIMEOUT) else {
        return Vec::new();
    };
    parse_list(&out.stdout)
}

fn parse_list(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| {
            let mut words = l.split_whitespace();
            let (_flags, name, _rest) = (words.next()?, words.next()?, words.next()?);
            (name != "=").then(|| name.to_owned())
        })
        .collect()
}

fn find_render_node() -> Option<PathBuf> {
    let mut nodes: Vec<PathBuf> = std::fs::read_dir("/dev/dri")
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("renderD")))
        .collect();
    nodes.sort();
    nodes.into_iter().next()
}

/// Кодирует 3 кадра синтетического видео в никуда.
fn trial_encode(name: &str, kind: EncoderKind, vaapi_device: Option<&Path>) -> bool {
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-nostdin", "-hide_banner", "-v", "error"]);
    if let (EncoderKind::Gpu, Some(dev)) = (kind, vaapi_device) {
        cmd.arg("-vaapi_device").arg(dev);
    }
    cmd.args(["-f", "lavfi", "-i", "color=c=black:s=320x240:r=30", "-frames:v", "3"]);
    if kind == EncoderKind::Gpu {
        cmd.args(["-vf", "format=nv12,hwupload"]);
    }
    cmd.args(["-c:v", name, "-f", "null", "-"]);
    match process::run("ffmpeg", &mut cmd, TRIAL_TIMEOUT) {
        Ok(out) if out.success => true,
        Ok(out) => {
            log::info!("кодировщик {name} недоступен: {}", out.stderr.lines().last().unwrap_or(""));
            false
        }
        Err(e) => {
            log::warn!("пробное кодирование {name}: {e}");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_encoder_and_filter_lists() {
        let encoders = "Encoders:\n V..... = Video\n ------\n V....D libx264              libx264 H.264\n V....D h264_vaapi           H.264/AVC (VAAPI)\n";
        assert_eq!(parse_list(encoders), ["libx264", "h264_vaapi"]);
        let filters = "Filters:\n  T.. = Timeline support\n  | = Source or sink filter\n TS gblur             V->V       Apply Gaussian Blur filter.\n .. ass               V->V       Render ASS subtitles.\n";
        assert_eq!(parse_list(filters), ["gblur", "ass"]);
    }

    #[test]
    fn encoder_choice() {
        let enc = |codec, kind, name, works| EncoderInfo { codec, kind, name, works };
        let caps = Capabilities {
            ffmpeg_version: Some("8".into()),
            ffprobe: true,
            encoders: vec![
                enc(Codec::H264, EncoderKind::Gpu, "h264_vaapi", true),
                enc(Codec::H264, EncoderKind::Cpu, "libx264", true),
                enc(Codec::Av1, EncoderKind::Gpu, "av1_vaapi", false),
                enc(Codec::Av1, EncoderKind::Cpu, "libsvtav1", true),
            ],
            ..Default::default()
        };
        assert_eq!(caps.encoder_for(Codec::H264, EncoderPref::Auto).unwrap().name, "h264_vaapi");
        assert_eq!(caps.encoder_for(Codec::H264, EncoderPref::Cpu).unwrap().name, "libx264");
        assert_eq!(caps.encoder_for(Codec::Av1, EncoderPref::Auto).unwrap().name, "libsvtav1");
        assert!(caps.encoder_for(Codec::Av1, EncoderPref::Gpu).is_none());
        assert!(caps.encoder_for(Codec::Vp9, EncoderPref::Auto).is_none());
    }
}
