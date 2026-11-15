//! Ассеты библиотеки: сведения о картинке/видео (с альфой) и миниатюра.

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use sc_core::AssetMeta;
use serde::Deserialize;

use crate::{MediaError, process};

const TIMEOUT: Duration = Duration::from_secs(30);

/// Размер, длительность (у картинки — нет), fps, альфа, звук, кодек.
pub fn probe_asset(path: &Path) -> Result<AssetMeta, MediaError> {
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
    format_name: Option<String>,
    duration: Option<String>,
}

#[derive(Deserialize)]
struct Stream {
    codec_type: Option<String>,
    codec_name: Option<String>,
    pix_fmt: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    avg_frame_rate: Option<String>,
    r_frame_rate: Option<String>,
    duration: Option<String>,
    #[serde(default)]
    side_data_list: Vec<SideData>,
    /// В matroska теги в верхнем регистре (`ALPHA_MODE`), поэтому — как карта.
    #[serde(default)]
    tags: std::collections::HashMap<String, String>,
    #[serde(default)]
    disposition: Disposition,
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

pub(crate) fn parse(json: &str) -> Result<AssetMeta, MediaError> {
    let p: Probe = serde_json::from_str(json).map_err(|e| MediaError::Parse(e.to_string()))?;
    let video = p
        .streams
        .iter()
        .find(|s| s.codec_type.as_deref() == Some("video") && s.disposition.attached_pic == 0)
        .ok_or(MediaError::NoVideo)?;
    let (mut width, mut height) = match (video.width, video.height) {
        (Some(w), Some(h)) if w > 0 && h > 0 => (w, h),
        _ => return Err(MediaError::Parse("нет размера кадра".into())),
    };
    let tag = |k: &str| video.tags.iter().find(|(key, _)| key.eq_ignore_ascii_case(k)).map(|(_, v)| v.as_str());
    let rotation = video
        .side_data_list
        .iter()
        .find_map(|d| d.rotation)
        .or_else(|| tag("rotate").and_then(|r| r.parse().ok()))
        .unwrap_or(0.0);
    if (rotation.round() as i64).rem_euclid(180) == 90 {
        std::mem::swap(&mut width, &mut height);
    }
    let codec = video.codec_name.clone().unwrap_or_default();
    // VP9 с альфой: pix_fmt yuv420p, альфа — отдельным слоем (тег ALPHA_MODE=1).
    let alpha = video.pix_fmt.as_deref().is_some_and(pix_fmt_alpha) || tag("alpha_mode") == Some("1");

    let format = p.format.as_ref().and_then(|f| f.format_name.as_deref()).unwrap_or("");
    // Картинка: демультиплексор image2 / *_pipe (png, jpeg, webp…). У GIF — длительность, если кадров больше одного.
    let still = format == "image2" || format.ends_with("_pipe");
    let duration = [p.format.as_ref().and_then(|f| f.duration.as_ref()), video.duration.as_ref()]
        .into_iter()
        .flatten()
        .find_map(|d| d.parse::<f64>().ok().filter(|d| d.is_finite() && *d > 0.0));
    let duration = if still { None } else { duration.filter(|d| codec != "gif" || *d > 0.05) };
    let fps = duration.and_then(|_| {
        [&video.avg_frame_rate, &video.r_frame_rate].into_iter().find_map(|r| r.as_deref().and_then(parse_rate))
    });
    if !still && codec != "gif" && duration.is_none() {
        return Err(MediaError::Parse("нет длительности".into()));
    }
    let has_audio = p.streams.iter().any(|s| s.codec_type.as_deref() == Some("audio"));
    Ok(AssetMeta { width, height, duration, fps, alpha, has_audio, codec, font: None })
}

/// Форматы пикселей с альфой (`pal8` — палитра PNG/GIF, прозрачность возможна).
fn pix_fmt_alpha(f: &str) -> bool {
    f.starts_with("yuva")
        || f.starts_with("gbrap")
        || f.starts_with("ya")
        || ["rgba", "bgra", "argb", "abgr"].iter().any(|p| f.contains(p))
        || f == "pal8"
}

fn parse_rate(s: &str) -> Option<f64> {
    let (n, d) = s.split_once('/').unwrap_or((s, "1"));
    let (n, d): (f64, f64) = (n.trim().parse().ok()?, d.trim().parse().ok()?);
    let r = n / d;
    (r.is_finite() && r > 0.0).then_some(r)
}

/// Миниатюра ассета в PNG (RGBA, вписана в `max`×`max`): кадр на трети видео (не дальше 1 с) или картинка.
/// VP9 с альфой декодирует только libvpx — если его нет, миниатюра будет без прозрачности.
pub fn thumbnail(src: &Path, meta: &AssetMeta, dest: &Path, max: u32) -> Result<(), MediaError> {
    let tmp = dest.with_extension("png.tmp");
    let run = |libvpx: bool| -> Result<bool, MediaError> {
        let mut cmd = Command::new("ffmpeg");
        cmd.args(["-v", "error", "-nostdin"]);
        if libvpx {
            cmd.args(["-c:v", "libvpx-vp9"]);
        }
        if let Some(d) = meta.duration.filter(|_| meta.codec != "gif") {
            cmd.args(["-ss", &format!("{:.3}", (d / 3.0).min(1.0))]);
        }
        cmd.arg("-i").arg(src).args([
            "-frames:v",
            "1",
            "-vf",
            &format!("scale={max}:{max}:force_original_aspect_ratio=decrease,format=rgba"),
            "-c:v",
            "png",
            "-f",
            "image2",
            "-y",
        ]);
        cmd.arg(&tmp);
        let out = process::run("ffmpeg", &mut cmd, TIMEOUT)?;
        if !out.success {
            log::warn!("миниатюра {}: {}", src.display(), out.stderr.trim());
        }
        Ok(out.success && tmp.exists())
    };
    let vp9_alpha = meta.codec == "vp9" && meta.alpha;
    let ok = (vp9_alpha && run(true)?) || run(false)?;
    if !ok {
        let _ = std::fs::remove_file(&tmp);
        return Err(MediaError::Failed { tool: "ffmpeg", stderr: "миниатюра не создана".into() });
    }
    std::fs::rename(&tmp, dest)?;
    Ok(())
}

/// Кадры анимации (GIF, видео) для превью: RGBA с обычной альфой, `w`×`h`, ровно `fps` кадров в секунду,
/// один проход, не больше `max_frames`. `decoder` — декодер с альфой (`libvpx-vp9`); не сработал — повтор без него.
pub fn decode_frames(src: &Path, decoder: Option<&str>, w: u32, h: u32, fps: f64, max_frames: usize) -> Result<Vec<Vec<u8>>, MediaError> {
    use std::io::Read;
    use std::process::Stdio;
    let frame = (w * h * 4) as usize;
    let run = |decoder: Option<&str>| -> Result<(Vec<Vec<u8>>, String), MediaError> {
        let mut cmd = Command::new("ffmpeg");
        cmd.args(["-v", "error", "-nostdin"]);
        if let Some(d) = decoder {
            cmd.args(["-c:v", d]);
        }
        cmd.arg("-i").arg(src).args([
            "-an",
            "-frames:v",
            &max_frames.to_string(),
            "-vf",
            &format!("fps={fps:.4},scale={w}:{h}:flags=lanczos,format=rgba"),
            "-f",
            "rawvideo",
            "-",
        ]);
        let mut child = cmd
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|source| MediaError::Spawn { tool: "ffmpeg", source })?;
        let mut err = child.stderr.take().expect("stderr piped");
        let err_t = std::thread::spawn(move || {
            let mut s = String::new();
            let _ = err.read_to_string(&mut s);
            s
        });
        let mut out = child.stdout.take().expect("stdout piped");
        let mut frames = Vec::new();
        loop {
            let mut buf = vec![0u8; frame];
            match out.read_exact(&mut buf) {
                Ok(()) => frames.push(buf),
                Err(_) => break,
            }
        }
        let _ = child.wait();
        Ok((frames, err_t.join().unwrap_or_default()))
    };
    let (mut frames, mut stderr) = run(decoder)?;
    if frames.is_empty() && decoder.is_some() {
        log::warn!("кадры {} через {decoder:?}: {} — без него", src.display(), stderr.trim());
        (frames, stderr) = run(None)?;
    }
    if frames.is_empty() {
        return Err(MediaError::Failed { tool: "ffmpeg", stderr });
    }
    Ok(frames)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_with_alpha() {
        let json = r#"{"streams": [{"codec_type": "video", "codec_name": "png", "pix_fmt": "rgba", "width": 600, "height": 200,
            "avg_frame_rate": "0/0", "r_frame_rate": "25/1"}], "format": {"format_name": "png_pipe"}}"#;
        let m = parse(json).unwrap();
        assert_eq!((m.width, m.height, m.alpha, m.duration, m.fps), (600, 200, true, None, None));
        assert_eq!(m.codec, "png");
    }

    #[test]
    fn jpeg_and_animated_gif() {
        let jpg = r#"{"streams": [{"codec_type": "video", "codec_name": "mjpeg", "pix_fmt": "yuvj420p", "width": 1080, "height": 1920}],
            "format": {"format_name": "image2", "duration": "0.04"}}"#;
        let m = parse(jpg).unwrap();
        assert!(!m.alpha && m.duration.is_none());

        let gif = r#"{"streams": [{"codec_type": "video", "codec_name": "gif", "pix_fmt": "bgra", "width": 480, "height": 270,
            "avg_frame_rate": "10/1", "duration": "2.4"}], "format": {"format_name": "gif", "duration": "2.4"}}"#;
        let m = parse(gif).unwrap();
        assert_eq!((m.alpha, m.duration, m.fps), (true, Some(2.4), Some(10.0)));
        let one = r#"{"streams": [{"codec_type": "video", "codec_name": "gif", "pix_fmt": "bgra", "width": 48, "height": 48}],
            "format": {"format_name": "gif"}}"#;
        assert_eq!(parse(one).unwrap().duration, None, "GIF из одного кадра — картинка");
    }

    #[test]
    fn vp9_alpha_and_prores() {
        let webm = r#"{"streams": [{"codec_type": "video", "codec_name": "vp9", "pix_fmt": "yuv420p", "width": 1280, "height": 720,
            "avg_frame_rate": "30/1", "tags": {"ALPHA_MODE": "1", "DURATION": "00:00:05.000"}},
            {"codec_type": "audio", "codec_name": "opus"}], "format": {"format_name": "matroska,webm", "duration": "5.0"}}"#;
        let m = parse(webm).unwrap();
        assert!(m.alpha && m.has_audio);
        assert_eq!((m.duration, m.fps), (Some(5.0), Some(30.0)));

        let mov = r#"{"streams": [{"codec_type": "video", "codec_name": "prores", "pix_fmt": "yuva444p12le", "width": 1920, "height": 1080,
            "avg_frame_rate": "60/1"}], "format": {"format_name": "mov,mp4,m4a,3gp,3g2,mj2", "duration": "3.5"}}"#;
        assert!(parse(mov).unwrap().alpha);

        let mp4 = r#"{"streams": [{"codec_type": "video", "codec_name": "h264", "pix_fmt": "yuv420p", "width": 1080, "height": 1920,
            "avg_frame_rate": "30/1"}], "format": {"format_name": "mov,mp4,m4a,3gp,3g2,mj2", "duration": "15"}}"#;
        let m = parse(mp4).unwrap();
        assert!(!m.alpha && !m.has_audio);
        let broken = r#"{"streams": [{"codec_type": "video", "codec_name": "h264", "width": 10, "height": 10}],
            "format": {"format_name": "mov,mp4,m4a,3gp,3g2,mj2"}}"#;
        assert!(parse(broken).is_err(), "видео без длительности");
    }
}
