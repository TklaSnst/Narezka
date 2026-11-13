//! Рендер на настоящем ffmpeg (пропускается, если ffmpeg не установлен).

use std::path::Path;
use std::process::Command;
use std::sync::atomic::AtomicBool;

use sc_core::{Background, Border, Color, Layer, OutputSettings, Paint, Rect, Shadow, Template};
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
    let Layer::MainVideo { corner_radius, .. } = &mut template.layers[0] else { unreachable!() };
    *corner_radius = 0.04;
    RenderJob {
        input: input.into(),
        src_size: (640, 360),
        src_fps: 30.0,
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

/// Полный шаблон: фон-градиент, фигура, два слоя видео с тенью и рамкой. Частота — как у исходника
/// (сгенерированный фон не навязывает свои 25 к/с), длительность — как у отрезка.
#[test]
fn renders_full_template_at_source_rate() {
    let tmp = tempfile::tempdir().unwrap();
    let input = tmp.path().join("src.mp4");
    let made = ffmpeg(&[
        "-f", "lavfi", "-i", "testsrc2=s=640x360:r=30:d=3",
        "-c:v", "libx264", "-preset", "ultrafast", input.to_str().unwrap(),
    ]);
    if made.is_none() {
        eprintln!("ffmpeg с libx264 недоступен — тест пропущен");
        return;
    }
    let mut j = job(tmp.path(), &input, "full.mp4");
    j.has_audio = false;
    j.template.background = Background::Gradient { from: Color::rgb(20, 30, 60), to: Color::BLACK, angle: 30.0 };
    j.template.layers = vec![
        Layer::Shape { rect: Rect::new(0.0, 0.5, 1.0, 0.5), paint: Paint::Linear { from: Color([0, 0, 0, 0]), to: Color::BLACK, angle: 0.0 }, corner_radius: 0.0 },
        Layer::MainVideo {
            crop: Rect::new(0.0, 0.0, 0.5, 1.0),
            dest: Rect::new(0.05, 0.1, 0.9, 0.35),
            corner_radius: 0.04,
            border: Some(Border { width: 0.005, color: Color::WHITE }),
            shadow: Some(Shadow { color: Color([0, 0, 0, 200]), blur: 0.02, dx: 0.0, dy: 0.01 }),
        },
        Layer::MainVideo { crop: Rect::new(0.5, 0.0, 0.5, 1.0), dest: Rect::new(0.05, 0.55, 0.9, 0.35), corner_radius: 0.0, border: None, shadow: None },
    ];
    j.run(&AtomicBool::new(false), |_| {}).unwrap();
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "v", "-count_frames", "-show_entries", "stream=width,height,r_frame_rate,nb_read_frames", "-of", "csv=p=0"])
        .arg(&j.dest)
        .output()
        .unwrap();
    let info = String::from_utf8_lossy(&out.stdout);
    let f: Vec<&str> = info.trim().split(',').collect();
    assert_eq!(&f[..3], ["1080", "1920", "30/1"], "{info}");
    let frames: i32 = f[3].parse().unwrap();
    assert!((29..=30).contains(&frames), "{info}");
}
