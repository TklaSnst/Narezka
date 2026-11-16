//! Ключи ffmpeg для кодирования: видео по `VideoEncode`, звук, формат пикселей и мультиплексор.

use sc_core::{AudioCodec, AudioEncode, Codec, Container, ProResProfile, RateMode, Speed, VideoEncode, VideoProfile};

use crate::RenderError;

/// Кодировщик VAAPI (кадры в хвосте графа — `format=nv12,hwupload`).
pub fn is_vaapi(encoder: &str) -> bool {
    encoder.ends_with("_vaapi")
}

/// Формат пикселей перед CPU-кодировщиком (у VAAPI — `nv12`).
pub fn pix_fmt(codec: Codec, prores: ProResProfile) -> &'static str {
    match (codec, prores) {
        (Codec::ProRes, ProResProfile::P4444) => "yuv444p10le",
        (Codec::ProRes, _) => "yuv422p10le",
        _ => "yuv420p",
    }
}

/// `-c:v …` и параметры качества. `fps` — частота вывода (для GOP в секундах).
pub fn video_args(encoder: &str, v: &VideoEncode, prores: ProResProfile, fps: f64) -> Result<Vec<String>, RenderError> {
    let mut a: Vec<String> = vec!["-c:v".into(), encoder.into()];
    let mut push = |items: &[&str]| a.extend(items.iter().map(|s| s.to_string()));
    let kbps = v.bitrate_kbps.max(100);
    let vbr = [
        "-b:v".to_owned(),
        format!("{kbps}k"),
        "-maxrate".into(),
        format!("{}k", kbps * 3 / 2),
        "-bufsize".into(),
        format!("{}k", kbps * 2),
    ];
    let q = v.quality.to_string();
    match encoder {
        "h264_vaapi" | "hevc_vaapi" | "av1_vaapi" => match v.rate {
            RateMode::Bitrate => {
                push(&["-rc_mode", "VBR"]);
                push(&vbr.each_ref().map(String::as_str));
            }
            RateMode::Quality => push(&["-rc_mode", "CQP", "-qp", &q]),
        },
        "libx264" | "libx265" => {
            let speed = match (encoder, v.speed) {
                ("libx264", Speed::Slow) => "slow",
                ("libx264", Speed::Medium) => "medium",
                ("libx264", Speed::Fast) => "veryfast",
                ("libx264", Speed::Faster) => "superfast",
                // x265 втрое медленнее x264 на той же ступени (0.4: fast — 70 с на минуту)
                (_, Speed::Slow) => "medium",
                (_, Speed::Medium) => "fast",
                (_, Speed::Fast) => "veryfast",
                (_, Speed::Faster) => "ultrafast",
            };
            push(&["-preset", speed]);
            match v.rate {
                RateMode::Bitrate => push(&vbr.each_ref().map(String::as_str)),
                RateMode::Quality => push(&["-crf", &q]),
            }
            if encoder == "libx265" {
                push(&["-x265-params", "log-level=error"]);
            }
        }
        "libsvtav1" => {
            let preset = match v.speed {
                Speed::Slow => "6",
                Speed::Medium => "8",
                Speed::Fast => "10",
                Speed::Faster => "12",
            };
            push(&["-preset", preset]);
            match v.rate {
                RateMode::Bitrate => push(&["-b:v", &format!("{kbps}k")]),
                RateMode::Quality => push(&["-crf", &q]),
            }
            push(&["-svtav1-params", "log-level=1"]);
        }
        "libvpx-vp9" => {
            // good в 4 раза медленнее realtime при том же SSIM (замер 7.1: 1080×1920@60, CRF 32) — только «медленно».
            let (deadline, cpu) = match v.speed {
                Speed::Slow => ("good", "4"),
                Speed::Medium => ("realtime", "5"),
                Speed::Fast => ("realtime", "7"),
                Speed::Faster => ("realtime", "8"),
            };
            push(&["-deadline", deadline, "-cpu-used", cpu, "-row-mt", "1"]);
            match v.rate {
                RateMode::Bitrate => push(&vbr.each_ref().map(String::as_str)),
                // Постоянное качество у libvpx — только с -b:v 0.
                RateMode::Quality => push(&["-crf", &q, "-b:v", "0"]),
            }
        }
        "prores_ks" => {
            // apl0 — как у Apple: Final Cut и DaVinci не ругаются на «чужой» ProRes.
            push(&["-profile:v", &prores.id().to_string(), "-vendor", "apl0"]);
            return Ok(a);
        }
        _ => return Err(RenderError::UnsupportedEncoder(encoder.into())),
    }
    if let Some(p) = profile(encoder, v.profile) {
        push(&["-profile:v", p]);
    }
    if v.gop_seconds > 0.0 {
        let frames = (v.gop_seconds as f64 * fps).round().max(1.0) as u32;
        push(&["-g", &frames.to_string()]);
    }
    if encoder == "hevc_vaapi" || encoder == "libx265" {
        // Без тега hvc1 QuickTime и часть плееров не открывают HEVC в MP4.
        push(&["-tag:v", "hvc1"]);
    }
    Ok(a)
}

/// Имя профиля H.264 для `-profile:v` (у VAAPI baseline — constrained_baseline).
fn profile(encoder: &str, p: VideoProfile) -> Option<&'static str> {
    let vaapi = is_vaapi(encoder);
    match (encoder, p) {
        ("h264_vaapi" | "libx264", VideoProfile::Baseline) => Some(if vaapi { "constrained_baseline" } else { "baseline" }),
        ("h264_vaapi" | "libx264", VideoProfile::Main) => Some("main"),
        ("h264_vaapi" | "libx264", VideoProfile::High) => Some("high"),
        _ => None,
    }
}

/// `-c:a …` и битрейт.
pub fn audio_args(a: &AudioEncode) -> Vec<String> {
    let kbps = format!("{}k", a.bitrate_kbps.clamp(32, 512));
    match a.codec {
        AudioCodec::Aac => vec!["-c:a".into(), "aac".into(), "-b:a".into(), kbps],
        AudioCodec::Opus => vec!["-c:a".into(), "libopus".into(), "-b:a".into(), kbps],
        AudioCodec::Pcm => vec!["-c:a".into(), "pcm_s16le".into()],
    }
}

/// Имя звукового кодировщика ffmpeg (для проверки `Capabilities`).
pub fn audio_encoder(codec: AudioCodec) -> &'static str {
    match codec {
        AudioCodec::Aac => "aac",
        AudioCodec::Opus => "libopus",
        AudioCodec::Pcm => "pcm_s16le",
    }
}

/// Мультиплексор ffmpeg для пары контейнер–кодек.
pub fn muxer(container: Container, codec: Codec, audio: AudioCodec) -> Result<&'static str, RenderError> {
    if !container.codecs().contains(&codec) || !container.audio_codecs().contains(&audio) {
        return Err(RenderError::UnsupportedContainer(container.extension()));
    }
    Ok(match container {
        Container::Mp4 => "mp4",
        Container::Mov => "mov",
        Container::Webm => "webm",
        Container::Mkv => "matroska",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sc_core::{QualityPreset, preset};

    fn args(enc: &str, v: &VideoEncode) -> String {
        video_args(enc, v, ProResProfile::Hq, 60.0).unwrap().join(" ")
    }

    #[test]
    fn quality_modes_per_encoder() {
        let mut v = preset(Codec::H264, true, QualityPreset::Balanced);
        assert_eq!(args("h264_vaapi", &v), "-c:v h264_vaapi -rc_mode VBR -b:v 12000k -maxrate 18000k -bufsize 24000k");
        v.rate = RateMode::Quality;
        v.quality = 24;
        v.profile = VideoProfile::Baseline;
        v.gop_seconds = 2.0;
        assert_eq!(args("h264_vaapi", &v), "-c:v h264_vaapi -rc_mode CQP -qp 24 -profile:v constrained_baseline -g 120");
        assert_eq!(args("libx264", &v), "-c:v libx264 -preset medium -crf 24 -profile:v baseline -g 120");
        let av1 = preset(Codec::Av1, false, QualityPreset::Balanced);
        assert_eq!(args("libsvtav1", &av1), "-c:v libsvtav1 -preset 8 -crf 32 -svtav1-params log-level=1");
        let vp9 = preset(Codec::Vp9, false, QualityPreset::Fast);
        assert_eq!(args("libvpx-vp9", &vp9), "-c:v libvpx-vp9 -deadline realtime -cpu-used 8 -row-mt 1 -crf 34 -b:v 0");
        assert_eq!(args("prores_ks", &v), "-c:v prores_ks -profile:v 3 -vendor apl0");
        let hevc = preset(Codec::Hevc, false, QualityPreset::High);
        assert!(args("libx265", &hevc).ends_with("-tag:v hvc1"));
        assert!(video_args("av1_nvenc", &v, ProResProfile::Hq, 30.0).is_err());
    }

    #[test]
    fn formats_and_muxers() {
        assert_eq!(pix_fmt(Codec::ProRes, ProResProfile::P4444), "yuv444p10le");
        assert_eq!(pix_fmt(Codec::ProRes, ProResProfile::Proxy), "yuv422p10le");
        assert_eq!(pix_fmt(Codec::Vp9, ProResProfile::Hq), "yuv420p");
        assert_eq!(muxer(Container::Webm, Codec::Av1, AudioCodec::Opus).unwrap(), "webm");
        assert!(muxer(Container::Webm, Codec::H264, AudioCodec::Opus).is_err());
        assert!(muxer(Container::Mp4, Codec::H264, AudioCodec::Pcm).is_err());
        assert_eq!(audio_args(&AudioEncode { codec: AudioCodec::Pcm, bitrate_kbps: 192 }), ["-c:a", "pcm_s16le"]);
    }
}
