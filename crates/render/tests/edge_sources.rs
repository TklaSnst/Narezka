//! Крайние исходники (шаг 8.1): без звука, VFR, повёрнутый, очень короткий.
//! Исходник читается через `sc_media::probe` — как в приложении, — и рендерится libx264.
//! Без ffmpeg/libx264 тесты пропускаются.

use std::path::Path;
use std::process::Command;
use std::sync::atomic::AtomicBool;

use sc_core::{OutputSettings, QualityPreset, Template, VideoEncode};
use sc_render::{Encoder, RenderJob};

fn ffmpeg(args: &[&str]) -> bool {
    Command::new("ffmpeg")
        .args(["-nostdin", "-hide_banner", "-v", "error", "-y"])
        .args(args)
        .status()
        .is_ok_and(|s| s.success())
}

/// Ширина, высота, r_frame_rate, число кадров, длительность видео; есть ли звук и его длительность.
struct Out {
    w: u32,
    h: u32,
    rate: String,
    frames: u32,
    dur: f64,
    audio: Option<f64>,
}

fn inspect(path: &Path) -> Out {
    let run = |sel: &str, entries: &str| {
        let o = Command::new("ffprobe")
            .args(["-v", "error", "-select_streams", sel, "-count_frames", "-show_entries", entries, "-of", "csv=p=0"])
            .arg(path)
            .output()
            .unwrap();
        String::from_utf8_lossy(&o.stdout).trim().to_owned()
    };
    let v = run("v", "stream=width,height,r_frame_rate,nb_read_frames:format=duration");
    let f: Vec<&str> = v.split([',', '\n']).collect();
    let a = run("a", "stream=duration");
    Out {
        w: f[0].parse().unwrap(),
        h: f[1].parse().unwrap(),
        rate: f[2].into(),
        frames: f[3].parse().unwrap(),
        dur: f.last().unwrap().parse().unwrap(),
        audio: (!a.is_empty()).then(|| a.lines().next().unwrap().parse().unwrap_or(0.0)),
    }
}

/// Рендерит [start, end) исходника так же, как очередь приложения: размер, fps и звук — из probe.
fn render(dir: &Path, input: &Path, start: f64, end: f64) -> Out {
    let info = sc_media::probe(input).unwrap();
    let name = input.file_stem().unwrap().to_string_lossy();
    let job = RenderJob {
        input: input.into(),
        src_size: (info.width, info.height),
        src_fps: info.fps,
        has_audio: info.has_audio,
        start,
        end: end.min(info.duration),
        template: Template::minimal(),
        subtitles: vec![],
        insert: None,
        cover: None,
        output: OutputSettings {
            quality: QualityPreset::Custom,
            custom: VideoEncode { bitrate_kbps: 1500, ..Default::default() },
            ..Default::default()
        },
        encoder: Encoder { name: "libx264".into(), vaapi_device: None },
        dest: dir.join(format!("{name}-out.mp4")),
        work_dir: dir.join(format!("work-{name}")),
    };
    job.run(&AtomicBool::new(false), |_| {}).unwrap_or_else(|e| panic!("{name}: {e}"));
    inspect(&job.dest)
}

fn have_x264(dir: &Path) -> bool {
    let ok = ffmpeg(&["-f", "lavfi", "-i", "testsrc2=s=64x64:d=0.1", "-c:v", "libx264", dir.join("probe.mp4").to_str().unwrap()]);
    if !ok {
        eprintln!("ffmpeg с libx264 недоступен — тест пропущен");
    }
    ok
}

#[test]
fn source_without_audio() {
    let tmp = tempfile::tempdir().unwrap();
    if !have_x264(tmp.path()) {
        return;
    }
    let src = tmp.path().join("noaudio.mp4");
    assert!(ffmpeg(&["-f", "lavfi", "-i", "testsrc2=s=640x360:r=30:d=3", "-c:v", "libx264", "-preset", "ultrafast", src.to_str().unwrap()]));
    let o = render(tmp.path(), &src, 0.5, 2.5);
    assert_eq!((o.w, o.h), (1080, 1920));
    assert!(o.audio.is_none(), "звука в исходнике нет — дорожки быть не должно");
    assert!((o.dur - 2.0).abs() < 0.1, "{}", o.dur);
}

/// VFR: mkv с «дырами» в кадрах. Вывод — постоянная частота, длительность как у отрезка, звук не короче видео.
#[test]
fn variable_frame_rate_source() {
    let tmp = tempfile::tempdir().unwrap();
    if !have_x264(tmp.path()) {
        return;
    }
    let src = tmp.path().join("vfr.mkv");
    assert!(ffmpeg(&[
        "-f", "lavfi", "-i", "testsrc2=s=640x360:r=60:d=4,select='not(mod(n\\,3))+between(t\\,1\\,2)'",
        "-f", "lavfi", "-i", "sine=d=4",
        "-fps_mode", "vfr", "-c:v", "libx264", "-preset", "ultrafast", "-c:a", "aac", "-shortest",
        src.to_str().unwrap(),
    ]));
    let o = render(tmp.path(), &src, 0.5, 3.5);
    assert_eq!((o.w, o.h), (1080, 1920));
    let fps: f64 = {
        let (n, d) = o.rate.split_once('/').unwrap();
        n.parse::<f64>().unwrap() / d.parse::<f64>().unwrap()
    };
    assert!((o.dur - 3.0).abs() < 0.15, "{}", o.dur);
    // Постоянная частота: кадров ровно столько, сколько fps × длительность.
    let expect = fps * o.dur;
    assert!((o.frames as f64 - expect).abs() <= 2.0, "кадров {} при {} к/с × {:.2} с", o.frames, o.rate, o.dur);
    let a = o.audio.expect("звук потерян");
    assert!((a - o.dur).abs() < 0.15, "звук {a}, видео {}", o.dur);
}

/// Телефонное видео с поворотом 90°: probe отдаёт 360×640, кадр в рендере стоит, а не лежит.
#[test]
fn rotated_source() {
    let tmp = tempfile::tempdir().unwrap();
    if !have_x264(tmp.path()) {
        return;
    }
    let base = tmp.path().join("base.mp4");
    let src = tmp.path().join("rotated.mp4");
    assert!(ffmpeg(&[
        "-f", "lavfi", "-i", "color=c=red:s=640x360:r=30:d=2",
        "-vf", "drawbox=x=0:y=0:w=320:h=360:c=blue:t=fill",
        "-c:v", "libx264", "-preset", "ultrafast", base.to_str().unwrap(),
    ]));
    assert!(ffmpeg(&["-display_rotation", "90", "-i", base.to_str().unwrap(), "-c", "copy", src.to_str().unwrap()]));
    let info = sc_media::probe(&src).unwrap();
    assert_eq!((info.width, info.height), (360, 640));
    let o = render(tmp.path(), &src, 0.0, 2.0);
    assert_eq!((o.w, o.h), (1080, 1920));
    // Вертикальный исходник 9:16 заполняет кадр целиком: после поворота синяя половина — сверху или снизу,
    // а не слева/справа. Сравниваем центр верхней и нижней четверти.
    let px = |y: u32| {
        let out = Command::new("ffmpeg")
            .args(["-nostdin", "-v", "error", "-ss", "1", "-i"])
            .arg(tmp.path().join("rotated-out.mp4"))
            .args(["-frames:v", "1", "-vf", &format!("crop=2:2:539:{y}"), "-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
            .output()
            .unwrap();
        [out.stdout[0], out.stdout[1], out.stdout[2]]
    };
    let (top, bottom) = (px(480), px(1440));
    let blue = |p: [u8; 3]| p[2] > 150 && p[0] < 100;
    let red = |p: [u8; 3]| p[0] > 150 && p[2] < 100;
    assert!((blue(top) && red(bottom)) || (red(top) && blue(bottom)), "верх {top:?}, низ {bottom:?}");
}

/// Исходник короче секунды: шортс на весь ролик рендерится, конец не выходит за длительность.
#[test]
fn very_short_source() {
    let tmp = tempfile::tempdir().unwrap();
    if !have_x264(tmp.path()) {
        return;
    }
    let src = tmp.path().join("tiny.mp4");
    assert!(ffmpeg(&[
        "-f", "lavfi", "-i", "testsrc2=s=640x360:r=30:d=0.4", "-f", "lavfi", "-i", "sine=d=0.4",
        "-c:v", "libx264", "-preset", "ultrafast", "-c:a", "aac", "-shortest", src.to_str().unwrap(),
    ]));
    let o = render(tmp.path(), &src, 0.0, 5.0);
    assert_eq!((o.w, o.h), (1080, 1920));
    assert!(o.frames >= 10, "кадров {}", o.frames);
    assert!(o.dur < 0.6, "{}", o.dur);
}
