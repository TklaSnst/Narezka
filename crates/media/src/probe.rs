//! ffprobe → `MediaInfo`.

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use sc_core::MediaInfo;
use serde::Deserialize;

use crate::{MediaError, process};

const TIMEOUT: Duration = Duration::from_secs(30);

/// Читает длительность, размер кадра (с учётом поворота), fps и наличие звука.
pub fn probe(path: &Path) -> Result<MediaInfo, MediaError> {
    let out = process::run(
        "ffprobe",
        Command::new("ffprobe")
            .args(["-v", "error", "-print_format", "json", "-show_format", "-show_streams"])
            .arg(path),
        TIMEOUT,
    )?;
    if !out.success {
        return Err(MediaError::Failed { tool: "ffprobe", stderr: out.stderr.trim().to_owned() });
    }
    parse(&out.stdout)
}

#[derive(Deserialize)]
struct Probe {
    #[serde(default)]
    streams: Vec<Stream>,
    format: Option<Format>,
}

#[derive(Deserialize)]
struct Format {
    duration: Option<String>,
}

#[derive(Deserialize)]
struct Stream {
    codec_type: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    avg_frame_rate: Option<String>,
    r_frame_rate: Option<String>,
    duration: Option<String>,
    #[serde(default)]
    disposition: Disposition,
    #[serde(default)]
    side_data_list: Vec<SideData>,
    #[serde(default)]
    tags: Tags,
}

#[derive(Deserialize, Default)]
struct Disposition {
    #[serde(default)]
    attached_pic: u8,
}

#[derive(Deserialize)]
struct SideData {
    rotation: Option<f64>,
}

#[derive(Deserialize, Default)]
struct Tags {
    rotate: Option<String>,
}

pub(crate) fn parse(json: &str) -> Result<MediaInfo, MediaError> {
    let p: Probe = serde_json::from_str(json).map_err(|e| MediaError::Parse(e.to_string()))?;

    // Обложка (attached_pic) в mp4/mkv — тоже «видеопоток», её пропускаем.
    let video = p
        .streams
        .iter()
        .find(|s| s.codec_type.as_deref() == Some("video") && s.disposition.attached_pic == 0)
        .ok_or(MediaError::NoVideo)?;
    let has_audio = p.streams.iter().any(|s| s.codec_type.as_deref() == Some("audio"));

    let (mut width, mut height) = match (video.width, video.height) {
        (Some(w), Some(h)) if w > 0 && h > 0 => (w, h),
        _ => return Err(MediaError::Parse("нет размера кадра".into())),
    };
    let rotation = video
        .side_data_list
        .iter()
        .find_map(|d| d.rotation)
        .or_else(|| video.tags.rotate.as_deref().and_then(|r| r.parse().ok()))
        .unwrap_or(0.0);
    if (rotation.round() as i64).rem_euclid(180) == 90 {
        std::mem::swap(&mut width, &mut height);
    }

    let fps = [&video.avg_frame_rate, &video.r_frame_rate]
        .into_iter()
        .find_map(|r| r.as_deref().and_then(parse_rate))
        .ok_or_else(|| MediaError::Parse("нет частоты кадров".into()))?;

    let duration = [p.format.as_ref().and_then(|f| f.duration.as_ref()), video.duration.as_ref()]
        .into_iter()
        .flatten()
        .find_map(|d| d.parse::<f64>().ok().filter(|d| d.is_finite() && *d > 0.0))
        .ok_or_else(|| MediaError::Parse("нет длительности".into()))?;

    Ok(MediaInfo { duration, width, height, fps, has_audio })
}

/// `"30000/1001"` → 29.97; `"0/0"` → None.
fn parse_rate(s: &str) -> Option<f64> {
    let (n, d) = s.split_once('/').unwrap_or((s, "1"));
    let (n, d): (f64, f64) = (n.trim().parse().ok()?, d.trim().parse().ok()?);
    let r = n / d;
    (r.is_finite() && r > 0.0).then_some(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rates() {
        assert_eq!(parse_rate("60/1"), Some(60.0));
        assert!((parse_rate("30000/1001").unwrap() - 29.97).abs() < 0.01);
        assert_eq!(parse_rate("0/0"), None);
        assert_eq!(parse_rate("25"), Some(25.0));
    }

    #[test]
    fn typical_mp4() {
        let json = r#"{
            "streams": [
                {"codec_type": "video", "width": 1920, "height": 1080, "avg_frame_rate": "60/1",
                 "r_frame_rate": "60/1", "disposition": {"attached_pic": 0}},
                {"codec_type": "audio"}
            ],
            "format": {"duration": "6612.345000"}
        }"#;
        let info = parse(json).unwrap();
        assert_eq!((info.width, info.height), (1920, 1080));
        assert_eq!(info.fps, 60.0);
        assert!(info.has_audio);
        assert!((info.duration - 6612.345).abs() < 1e-9);
    }

    #[test]
    fn rotated_phone_video_and_cover_art() {
        let json = r#"{
            "streams": [
                {"codec_type": "video", "width": 600, "height": 600, "avg_frame_rate": "0/0",
                 "disposition": {"attached_pic": 1}},
                {"codec_type": "video", "width": 1920, "height": 1080, "avg_frame_rate": "0/0",
                 "r_frame_rate": "30/1", "duration": "12.5", "side_data_list": [{"rotation": -90}]}
            ],
            "format": {}
        }"#;
        let info = parse(json).unwrap();
        assert_eq!((info.width, info.height), (1080, 1920));
        assert_eq!(info.fps, 30.0);
        assert!(!info.has_audio);
        assert_eq!(info.duration, 12.5);
    }

    #[test]
    fn audio_only_is_rejected() {
        let json = r#"{"streams": [{"codec_type": "audio"}], "format": {"duration": "10"}}"#;
        assert!(matches!(parse(json), Err(MediaError::NoVideo)));
    }
}
