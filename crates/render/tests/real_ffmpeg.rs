//! Рендер на настоящем ffmpeg (пропускается, если ffmpeg не установлен).

use std::path::Path;
use std::process::Command;
use std::sync::atomic::AtomicBool;

use sc_core::{Layer, OutputSettings, Template};
use sc_render::{Encoder, RenderError, RenderJob};

fn ffmpeg(args: &[&str]) -> Option<String> {
    let out = Command::new("ffmpeg").args(["-nostdin", "-hide_banner", "-v", "error"]).args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

fn probe(path: &Path) -> String {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-show_entries", "stream=codec_type,codec_name,width,height", "-of", "csv=p=0"])
        .arg(path)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn job(dir: &Path, input: &Path, name: &str) -> RenderJob {
    let mut template = Template::minimal();
    let Layer::MainVideo { corner_radius, .. } = &mut template.layers[0];
    *corner_radius = 0.04;
    RenderJob {
        input: input.into(),
        src_size: (640, 360),
        has_audio: true,
        start: 0.5,
        end: 1.5,
        template,
        output: OutputSettings { video_bitrate_kbps: 2000, ..Default::default() },
        encoder: Encoder { name: "libx264".into(), vaapi_device: None },
        dest: dir.join(name),
        work_dir: dir.join(format!("work-{name}")),
    }
}

#[test]
fn renders_and_cancels() {
    let tmp = tempfile::tempdir().unwrap();
    let input = tmp.path().join("src.mp4");
    let src = input.to_str().unwrap();
    let made = ffmpeg(&[
        "-f", "lavfi", "-i", "testsrc2=s=640x360:r=30:d=3",
        "-f", "lavfi", "-i", "sine=f=440:d=3",
        "-c:v", "libx264", "-preset", "ultrafast", "-c:a", "aac", "-shortest", src,
    ]);
    if made.is_none() {
        eprintln!("ffmpeg с libx264 недоступен — тест пропущен");
        return;
    }

    let j = job(tmp.path(), &input, "short_1.mp4");
    let mut last = 0.0;
    j.run(&AtomicBool::new(false), |p| last = p).unwrap();
    assert_eq!(last, 1.0);
    let info = probe(&j.dest);
    assert!(info.contains("h264,video,1080,1920"), "{info}");
    assert!(info.contains("aac"), "{info}");
    assert!(!j.work_dir.exists(), "временный каталог не удалён");
    assert!(!tmp.path().join(".short_1.mp4.part").exists());

    // Отмена сразу: итогового и недописанного файла нет.
    let j = job(tmp.path(), &input, "short_2.mp4");
    let r = j.run(&AtomicBool::new(true), |_| {});
    assert!(matches!(r, Err(RenderError::Cancelled)), "{r:?}");
    assert!(!j.dest.exists());
    assert!(!tmp.path().join(".short_2.mp4.part").exists());
}
