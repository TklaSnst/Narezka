//! Рендер на настоящем ffmpeg (пропускается, если ffmpeg не установлен).

use std::path::Path;
use std::process::Command;
use std::sync::atomic::AtomicBool;

use sc_core::{AudioCodec, Codec, Container, ProResProfile, QualityPreset, VideoEncode, AccentMode, Align, Background, BannerArt, BannerSide, BannerText, Border, Color, Layer, OutputSettings, Paint, Rect, Shadow, SubWord, SubtitleMode, Template, VAlign};
use sc_render::{CoverFrame, CoverJob, Encoder, InsertJob, RenderError, RenderJob};

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
        subtitles: vec![],
        insert: None,
        cover: None,
        output: OutputSettings {
            quality: QualityPreset::Custom,
            custom: VideoEncode { bitrate_kbps: 2000, ..Default::default() },
            ..Default::default()
        },
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

/// Пиксели кадра в момент `t` (секунды вывода), RGB, участок `w`×`h` с (`x`, `y`).
fn frame_rgb(path: &Path, t: f64, (x, y, w, h): (u32, u32, u32, u32)) -> Vec<[u8; 3]> {
    let out = Command::new("ffmpeg")
        .args(["-nostdin", "-v", "error", "-ss", &format!("{t:.3}"), "-i"])
        .arg(path)
        .args(["-frames:v", "1", "-vf", &format!("crop={w}:{h}:{x}:{y}"), "-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        .output()
        .unwrap();
    out.stdout.as_chunks::<3>().0.to_vec()
}

/// Субтитры в рендере: картинка меняется вместе со словами, в паузе пропадает.
#[test]
fn renders_subtitles_in_sync() {
    let tmp = tempfile::tempdir().unwrap();
    let input = tmp.path().join("src.mp4");
    let made = ffmpeg(&[
        "-f", "lavfi", "-i", "color=c=black:s=640x360:r=30:d=4",
        "-c:v", "libx264", "-preset", "ultrafast", input.to_str().unwrap(),
    ]);
    if made.is_none() {
        eprintln!("ffmpeg с libx264 недоступен — тест пропущен");
        return;
    }
    let mut j = job(tmp.path(), &input, "subs.mp4");
    j.has_audio = false;
    j.end = 3.5;
    j.template.background = Background::Color { color: Color::BLACK };
    j.template.layers.push(Layer::Subtitles {
        rect: Rect::new(0.05, 0.75, 0.9, 0.15),
        font: "Montserrat ExtraBold".into(),
        size: 0.08,
        color: Color::WHITE,
        active_color: Color::rgb(255, 214, 10),
        active_mode: AccentMode::Color,
        keyword_color: Color::rgb(255, 59, 48),
        dim_color: Color([255, 255, 255, 0]),
        mode: SubtitleMode::Highlight,
        words_per_line: 3,
        lines: 1,
        uppercase: true,
        outline: None,
        shadow: None,
        plate: None,
        align: Align::Center,
        valign: VAlign::Center,
    });
    // Время исходника: «Раз» 1.0, «два.» 1.5, фраза держится до 1.8 + 0.6 = 2.4 (вывода — 0.5 … 1.9).
    let w = |t: &str, s: f64| SubWord { text: t.into(), start: s, end: s + 0.3, keyword: false };
    j.subtitles = vec![w("Раз", 1.0), w("два.", 1.5)];
    j.run(&AtomicBool::new(false), |_| {}).unwrap();

    let area = (0, 1440, 1080, 288);
    let count = |t: f64| {
        let px = frame_rgb(&j.dest, t, area);
        assert!(!px.is_empty(), "кадр {t} не прочитан");
        let yellow = px.iter().filter(|p| p[0] > 200 && p[1] > 160 && p[2] < 90).count();
        let white = px.iter().filter(|p| p.iter().all(|&c| c > 220)).count();
        (yellow, white)
    };
    let (y, w) = count(0.2);
    assert!(y == 0 && w == 0, "до речи пусто: {y} {w}");
    let (y1, w1) = count(0.7);
    assert!(y1 > 500 && w1 > 500, "«Раз» жёлтое, «два.» белое: {y1} {w1}");
    let (y2, w2) = count(1.2);
    assert!(y2 > 500 && w2 > 500 && y2 != y1, "подсветка перешла на «два.»: {y2} {w2}");
    let (y, w) = count(2.5);
    assert!(y == 0 && w == 0, "после фразы пусто: {y} {w}");
}

/// Баннер: картинка с прозрачностью и текст рядом; анимация (VP9 с альфой) по кругу в такт выводу.
#[test]
fn renders_banners() {
    let tmp = tempfile::tempdir().unwrap();
    let p = |n: &str| tmp.path().join(n);
    let made = ffmpeg(&[
        "-f", "lavfi", "-i", "color=c=black:s=640x360:r=30:d=4",
        "-c:v", "libx264", "-preset", "ultrafast", p("src.mp4").to_str().unwrap(),
    ]);
    let png = ffmpeg(&["-f", "lavfi", "-i", "color=c=red@0.5:s=400x100,format=rgba", "-frames:v", "1", p("b.png").to_str().unwrap()]);
    // 0.5 с красного, 0.5 с зелёного, непрозрачные; по кругу — период 1 с.
    let webm = ffmpeg(&[
        "-f", "lavfi", "-i", "color=c=red:s=400x100:r=30:d=0.5",
        "-f", "lavfi", "-i", "color=c=lime:s=400x100:r=30:d=0.5",
        "-filter_complex", "[0][1]concat=n=2,format=yuva420p",
        "-c:v", "libvpx-vp9", "-auto-alt-ref", "0", p("b.webm").to_str().unwrap(),
    ]);
    if made.is_none() || png.is_none() || webm.is_none() {
        eprintln!("ffmpeg с libx264/libvpx недоступен — тест пропущен");
        return;
    }
    let slot = Rect::new(0.1, 0.05, 0.8, 0.1);
    let mut j = job(tmp.path(), &p("src.mp4"), "png.mp4");
    j.has_audio = false;
    j.end = 3.0;
    j.template.background = Background::Color { color: Color::BLACK };
    j.template.layers.push(Layer::BannerSlot { rect: slot });
    let base = j.template.clone();
    let text = BannerText { content: "SHORTS".into(), side: BannerSide::Below, ..Default::default() };
    let art = BannerArt { path: p("b.png"), size: (400, 100), animated: false, decoder: None, text: Some(text) };
    j.template = base.with_banner(Some(&art));
    j.run(&AtomicBool::new(false), |_| {}).unwrap();
    // Рамка 864×192 → картинка 768×192 по центру (x 156…924, y 96…288): полупрозрачный красный на чёрном.
    let px = frame_rgb(&j.dest, 1.0, (300, 150, 400, 80));
    assert!(px.iter().all(|c| (110..=145).contains(&c[0]) && c[1] < 20 && c[2] < 20), "{:?}", &px[..3]);
    // Под баннером — белый текст.
    let below = frame_rgb(&j.dest, 1.0, (156, 300, 768, 70));
    let white = below.iter().filter(|c| c.iter().all(|&v| v > 220)).count();
    assert!(white > 1000, "текст под баннером: {white}");

    let mut j2 = job(tmp.path(), &p("src.mp4"), "webm.mp4");
    j2.has_audio = false;
    j2.end = 3.0;
    let art = BannerArt { path: p("b.webm"), size: (400, 100), animated: true, decoder: Some("libvpx-vp9".into()), text: None };
    j2.template = base.with_banner(Some(&art));
    j2.run(&AtomicBool::new(false), |_| {}).unwrap();
    let color_at = |t: f64| {
        let px = frame_rgb(&j2.dest, t, (500, 180, 80, 40));
        let n = px.len() as u32;
        let avg = |k: usize| px.iter().map(|c| c[k] as u32).sum::<u32>() / n;
        (avg(0), avg(1))
    };
    // 0.25 и 1.25 — красный (второй круг), 0.75 и 1.75 — зелёный.
    for t in [0.25, 1.25, 2.25] {
        let (r, g) = color_at(t);
        assert!(r > 200 && g < 60, "t={t}: {r} {g}");
    }
    for t in [0.75, 1.75] {
        let (r, g) = color_at(t);
        assert!(r < 60 && g > 200, "t={t}: {r} {g}");
    }
    let dur = Command::new("ffprobe")
        .args(["-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0"])
        .arg(&j2.dest)
        .output()
        .unwrap();
    let d: f64 = String::from_utf8_lossy(&dur.stdout).trim().parse().unwrap();
    assert!((d - 2.5).abs() < 0.1, "длительность как у отрезка: {d}");
}

/// Наибольшая громкость звука на участке `[t, t + d)`, дБ.
fn max_volume(path: &Path, t: f64, d: f64) -> f64 {
    let out = Command::new("ffmpeg")
        .args(["-nostdin", "-hide_banner", "-ss", &format!("{t:.3}"), "-t", &format!("{d:.3}"), "-i"])
        .arg(path)
        .args(["-vn", "-af", "volumedetect", "-f", "null", "-"])
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    let line = err.lines().find(|l| l.contains("max_volume:")).unwrap_or_else(|| panic!("{err}"));
    line.split("max_volume:").nth(1).unwrap().trim().trim_end_matches(" dB").parse().unwrap()
}

fn duration_of(path: &Path, stream: &str) -> f64 {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", stream, "-show_entries", "stream=duration", "-of", "csv=p=0"])
        .arg(path)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().parse().unwrap()
}

/// Вставка: основное видео стоит, вставка идёт в слоте или на весь экран, звук после неё не разъезжается.
#[test]
fn renders_inserts() {
    let tmp = tempfile::tempdir().unwrap();
    let p = |n: &str| tmp.path().join(n);
    let made = ffmpeg(&[
        "-f", "lavfi", "-i", "color=c=blue:s=640x360:r=30:d=4",
        "-f", "lavfi", "-i", "sine=f=440:d=4",
        "-c:v", "libx264", "-preset", "ultrafast", "-c:a", "aac", "-shortest", p("src.mp4").to_str().unwrap(),
    ]);
    // Горизонтальная без звука и вертикальная со звуком, по 1 с.
    let wide = ffmpeg(&[
        "-f", "lavfi", "-i", "color=c=red:s=640x360:r=25:d=1",
        "-c:v", "libx264", "-preset", "ultrafast", p("wide.mp4").to_str().unwrap(),
    ]);
    let tall = ffmpeg(&[
        "-f", "lavfi", "-i", "color=c=lime:s=360x640:r=30:d=1",
        "-f", "lavfi", "-i", "sine=f=1000:d=1",
        "-c:v", "libx264", "-preset", "ultrafast", "-c:a", "aac", "-shortest", p("tall.mp4").to_str().unwrap(),
    ]);
    if made.is_none() || wide.is_none() || tall.is_none() {
        eprintln!("ffmpeg с libx264 недоступен — тест пропущен");
        return;
    }
    let avg = |path: &Path, t: f64, r: (u32, u32, u32, u32)| {
        let px = frame_rgb(path, t, r);
        let n = px.len() as u32;
        [0, 1, 2].map(|k| px.iter().map(|c| c[k] as u32).sum::<u32>() / n)
    };

    // В слот: шортс 0.5…2.5 с, вставка в 1.5 с (1.0 с от начала) — итог 3 с.
    let mut j = job(tmp.path(), &p("src.mp4"), "slot.mp4");
    j.end = 2.5;
    j.template.background = Background::Color { color: Color::BLACK };
    let plan = j.template.insert_plan(sc_core::InsertMode::Slot, &[], false);
    j.insert = Some(InsertJob { path: p("wide.mp4"), at: 1.5, duration: 1.0, size: (640, 360), has_audio: false, plan });
    j.run(&AtomicBool::new(false), |_| {}).unwrap();
    let slot = (440, 900, 200, 100);
    let [r, g, b] = avg(&j.dest, 0.5, slot);
    assert!(b > 200 && r < 40, "до вставки — исходник: {r} {g} {b}");
    let [r, g, b] = avg(&j.dest, 1.5, slot);
    assert!(r > 200 && b < 40, "вставка в слоте: {r} {g} {b}");
    let [r, g, b] = avg(&j.dest, 2.5, slot);
    assert!(b > 200 && r < 40, "после вставки — исходник: {r} {g} {b}");
    for s in ["v:0", "a:0"] {
        let d = duration_of(&j.dest, s);
        assert!((d - 3.0).abs() < 0.08, "{s}: {d}");
    }
    // Звук исходника — до и после, во время вставки (без звука) — тишина.
    assert!(max_volume(&j.dest, 0.2, 0.6) > -30.0);
    assert!(max_volume(&j.dest, 1.1, 0.8) < -60.0, "во время вставки тихо");
    assert!(max_volume(&j.dest, 2.1, 0.6) > -30.0);

    // На весь экран, со звуком вставки.
    let mut j = job(tmp.path(), &p("src.mp4"), "full.mp4");
    j.end = 2.5;
    let plan = j.template.insert_plan(sc_core::InsertMode::Fullscreen, &[], false);
    j.insert = Some(InsertJob { path: p("tall.mp4"), at: 1.5, duration: 1.0, size: (360, 640), has_audio: true, plan });
    j.run(&AtomicBool::new(false), |_| {}).unwrap();
    let top = (100, 100, 200, 200);
    let [r, g, b] = avg(&j.dest, 1.5, top);
    assert!(g > 200 && r < 60 && b < 60, "вставка на весь экран: {r} {g} {b}");
    let [r, g, b] = avg(&j.dest, 2.5, top);
    assert!(g < 100, "после вставки — шаблон: {r} {g} {b}");
    assert!(max_volume(&j.dest, 1.1, 0.8) > -30.0, "звук вставки");
    let d = duration_of(&j.dest, "a:0");
    assert!((d - 3.0).abs() < 0.08, "звук: {d}");
}

/// Обложка: кадр (из любого места исходника или картинка) стоит первым, звук на нём — тишина;
/// «звук сразу» — поверх первых кадров без сдвига.
#[test]
fn renders_covers() {
    let tmp = tempfile::tempdir().unwrap();
    let p = |n: &str| tmp.path().join(n);
    // Исходник: 2 с красного, потом 2 с синего; звук всё время.
    let made = ffmpeg(&[
        "-f", "lavfi", "-i", "color=c=red:s=640x360:r=30:d=2",
        "-f", "lavfi", "-i", "color=c=blue:s=640x360:r=30:d=2",
        "-f", "lavfi", "-i", "sine=f=440:d=4",
        "-filter_complex", "[0:v][1:v]concat=n=2:v=1:a=0[v]", "-map", "[v]", "-map", "2:a",
        "-c:v", "libx264", "-preset", "ultrafast", "-c:a", "aac", p("src.mp4").to_str().unwrap(),
    ]);
    let pic = ffmpeg(&["-f", "lavfi", "-i", "color=c=lime:s=320x320", "-frames:v", "1", p("pic.png").to_str().unwrap()]);
    if made.is_none() || pic.is_none() {
        eprintln!("ffmpeg с libx264 недоступен — тест пропущен");
        return;
    }
    let avg = |path: &Path, t: f64| {
        let px = frame_rgb(path, t, (440, 900, 200, 100));
        let n = px.len() as u32;
        [0, 1, 2].map(|k| px.iter().map(|c| c[k] as u32).sum::<u32>() / n)
    };
    let cover = |frame: CoverFrame, sound_now: bool| {
        let mut template = Template::minimal();
        template.background = Background::Color { color: Color::BLACK };
        Some(CoverJob { frame, template, frames: 15, sound_now })
    };

    // Кадр из красной части, шортс — в синей (2.5…3.5 с); обложка 15 кадров при 30 к/с = 0.5 с.
    let mut j = job(tmp.path(), &p("src.mp4"), "frame.mp4");
    (j.start, j.end) = (2.5, 3.5);
    j.template.background = Background::Color { color: Color::BLACK };
    j.cover = cover(CoverFrame::Source { t: 0.5 }, false);
    j.run(&AtomicBool::new(false), |_| {}).unwrap();
    let [r, g, b] = avg(&j.dest, 0.1);
    assert!(r > 200 && b < 40, "обложка — красный кадр: {r} {g} {b}");
    let [r, g, b] = avg(&j.dest, 0.8);
    assert!(b > 200 && r < 40, "дальше — шортс: {r} {g} {b}");
    for s in ["v:0", "a:0"] {
        let d = duration_of(&j.dest, s);
        assert!((d - 1.5).abs() < 0.08, "{s}: {d}");
    }
    assert!(max_volume(&j.dest, 0.0, 0.4) < -60.0, "на обложке тихо");
    assert!(max_volume(&j.dest, 0.7, 0.6) > -30.0);

    // Картинка вместо кадра, «звук сразу»: длина шортса не меняется, звук с первого кадра.
    let mut j = job(tmp.path(), &p("src.mp4"), "image.mp4");
    (j.start, j.end) = (2.5, 3.5);
    j.cover = cover(CoverFrame::Image { path: p("pic.png"), size: (320, 320) }, true);
    j.run(&AtomicBool::new(false), |_| {}).unwrap();
    let [r, g, b] = avg(&j.dest, 0.1);
    assert!(g > 200 && r < 60 && b < 60, "обложка — картинка: {r} {g} {b}");
    let [r, g, b] = avg(&j.dest, 0.8);
    assert!(b > 200 && g < 60, "дальше — шортс: {r} {g} {b}");
    for s in ["v:0", "a:0"] {
        let d = duration_of(&j.dest, s);
        assert!((d - 1.0).abs() < 0.08, "{s}: {d}");
    }
    assert!(max_volume(&j.dest, 0.0, 0.4) > -30.0, "звук сразу");

    // Обложка вместе со вставкой: вставка сдвигается на длину обложки, звук — тоже.
    let made = ffmpeg(&[
        "-f", "lavfi", "-i", "color=c=white:s=640x360:r=30:d=1",
        "-c:v", "libx264", "-preset", "ultrafast", p("ad.mp4").to_str().unwrap(),
    ]);
    assert!(made.is_some());
    let mut j = job(tmp.path(), &p("src.mp4"), "both.mp4");
    (j.start, j.end) = (2.5, 3.5);
    j.template.background = Background::Color { color: Color::BLACK };
    let plan = j.template.insert_plan(sc_core::InsertMode::Slot, &[], false);
    j.insert = Some(InsertJob { path: p("ad.mp4"), at: 3.0, duration: 1.0, size: (640, 360), has_audio: false, plan });
    j.cover = cover(CoverFrame::Source { t: 0.5 }, false);
    j.run(&AtomicBool::new(false), |_| {}).unwrap();
    let [r, _, b] = avg(&j.dest, 0.1);
    assert!(r > 200 && b < 40, "обложка");
    let [r, _, b] = avg(&j.dest, 0.8);
    assert!(b > 200 && r < 40, "шортс до вставки");
    let [r, g, b] = avg(&j.dest, 1.5);
    assert!(r > 200 && g > 200 && b > 200, "вставка сдвинута на обложку: {r} {g} {b}");
    for s in ["v:0", "a:0"] {
        let d = duration_of(&j.dest, s);
        assert!((d - 2.5).abs() < 0.08, "{s}: {d}");
    }
    assert!(max_volume(&j.dest, 0.0, 0.4) < -60.0, "на обложке тихо");
    assert!(max_volume(&j.dest, 1.1, 0.8) < -60.0, "во время вставки тихо");
    assert!(max_volume(&j.dest, 2.1, 0.3) > -30.0, "после вставки звук");
}

fn has_encoder(name: &str) -> bool {
    ffmpeg(&["-encoders"]).is_some_and(|l| l.lines().any(|l| l.split_whitespace().nth(1) == Some(name)))
}

/// Все форматы SPEC §6 на CPU-кодировщиках (отсутствующие в сборке — пропускаются):
/// кодек, звук, формат пикселей и разрешение/частота вывода.
#[test]
fn renders_every_format() {
    let tmp = tempfile::tempdir().unwrap();
    let input = tmp.path().join("src.mp4");
    let made = ffmpeg(&[
        "-f", "lavfi", "-i", "testsrc2=s=640x360:r=30:d=3",
        "-f", "lavfi", "-i", "sine=f=440:d=3",
        "-c:v", "libx264", "-preset", "ultrafast", "-c:a", "aac", "-shortest", input.to_str().unwrap(),
    ]);
    if made.is_none() {
        eprintln!("ffmpeg с libx264 недоступен — тест пропущен");
        return;
    }
    let cases = [
        (Container::Mp4, Codec::Hevc, "libx265", AudioCodec::Aac, "hevc", "yuv420p", "aac"),
        (Container::Mp4, Codec::Av1, "libsvtav1", AudioCodec::Opus, "av1", "yuv420p", "opus"),
        (Container::Webm, Codec::Vp9, "libvpx-vp9", AudioCodec::Opus, "vp9", "yuv420p", "opus"),
        (Container::Webm, Codec::Av1, "libsvtav1", AudioCodec::Opus, "av1", "yuv420p", "opus"),
        // декодер ProRes сообщает 4444 как 12 бит
        (Container::Mov, Codec::ProRes, "prores_ks", AudioCodec::Pcm, "prores", "yuv444p12le", "pcm_s16le"),
        (Container::Mov, Codec::ProRes, "prores_ks", AudioCodec::Aac, "prores", "yuv422p10le", "aac"),
    ];
    for (k, (container, codec, encoder, audio, vname, pix, aname)) in cases.into_iter().enumerate() {
        if !has_encoder(encoder) || (audio == AudioCodec::Opus && !has_encoder("libopus")) {
            eprintln!("{encoder}: нет в сборке ffmpeg — пропуск");
            continue;
        }
        let name = format!("f{k}.{}", container.extension());
        let mut j = job(tmp.path(), &input, &name);
        j.encoder = Encoder { name: encoder.into(), vaapi_device: None };
        j.output.container = container;
        j.output.codec = codec;
        j.output.quality = QualityPreset::Fast;
        j.output.audio.codec = audio;
        j.output.prores = if pix == "yuv444p12le" { ProResProfile::P4444 } else { ProResProfile::Proxy };
        j.output.resolution = Some(sc_core::Canvas { width: 720, height: 1280 });
        j.output.fps = Some(60.0);
        j.run(&AtomicBool::new(false), |_| {}).unwrap_or_else(|e| panic!("{encoder} в {name}: {e}"));
        let out = Command::new("ffprobe")
            .args(["-v", "error", "-show_entries", "stream=codec_name,width,height,pix_fmt,r_frame_rate", "-of", "csv=p=0"])
            .arg(&j.dest)
            .output()
            .unwrap();
        let info = String::from_utf8_lossy(&out.stdout);
        assert!(info.contains(&format!("{vname},720,1280,{pix},60/1")), "{name}: {info}");
        assert!(info.contains(aname), "{name}: {info}");
    }
}

/// Очередь: 2 задачи параллельно, пауза держит ffmpeg (готовых нет), отмена не начатой,
/// ошибка одного файла не мешает остальным и отдаёт полный лог.
#[test]
fn queue_pauses_cancels_and_survives_errors() {
    use sc_render::{Queue, QueueEvent};
    use std::sync::mpsc;
    use std::time::Duration;

    let tmp = tempfile::tempdir().unwrap();
    let input = tmp.path().join("src.mp4");
    let made = ffmpeg(&[
        "-f", "lavfi", "-i", "testsrc2=s=640x360:r=30:d=3",
        "-f", "lavfi", "-i", "sine=f=440:d=3",
        "-c:v", "libx264", "-preset", "ultrafast", "-c:a", "aac", "-shortest", input.to_str().unwrap(),
    ]);
    if made.is_none() {
        eprintln!("ffmpeg с libx264 недоступен — тест пропущен");
        return;
    }
    let mut jobs: Vec<RenderJob> = (1..=4).map(|n| job(tmp.path(), &input, &format!("q_{n}.mp4"))).collect();
    jobs[1].input = tmp.path().join("нет-такого.mp4");
    let dests: Vec<_> = jobs.iter().map(|j| j.dest.clone()).collect();

    let (tx, rx) = mpsc::channel();
    let tx = std::sync::Mutex::new(tx);
    let q = Queue::start(jobs, 2, move |ev| {
        let _ = tx.lock().unwrap().send(ev);
    });
    q.pause();
    q.cancel(3);
    std::thread::sleep(Duration::from_millis(1500));
    assert!(q.is_paused());
    let during: Vec<QueueEvent> = rx.try_iter().collect();
    assert!(
        !during.iter().any(|e| matches!(e, QueueEvent::Finished { result: Ok(()), .. })),
        "на паузе файлы не дописываются: {during:?}"
    );
    q.resume();

    let mut results: Vec<Option<Result<(), RenderError>>> = (0..4).map(|_| None).collect();
    for ev in during.into_iter().chain(std::iter::from_fn(|| rx.recv_timeout(Duration::from_secs(60)).ok())) {
        match ev {
            QueueEvent::Finished { index, result } => results[index] = Some(result),
            QueueEvent::Done => break,
            _ => {}
        }
    }
    assert!(matches!(results[0], Some(Ok(()))), "{:?}", results[0]);
    assert!(matches!(results[2], Some(Ok(()))), "{:?}", results[2]);
    assert!(matches!(results[3], Some(Err(RenderError::Cancelled))), "{:?}", results[3]);
    match &results[1] {
        Some(Err(e @ RenderError::Failed { .. })) => {
            let log = e.log();
            assert!(log.starts_with("ffmpeg ") && log.contains("нет-такого.mp4"), "{log}");
        }
        r => panic!("ожидалась ошибка ffmpeg: {r:?}"),
    }
    assert!(dests[0].exists() && dests[2].exists());
    assert!(!dests[1].exists() && !dests[3].exists());
    assert!(!tmp.path().join(".q_2.mp4.part").exists());
}

/// Отмена идущего рендера на паузе: остановленный ffmpeg убивается, недописанный файл удаляется.
#[test]
fn queue_cancels_paused_job() {
    use sc_render::{Queue, QueueEvent};
    use std::sync::mpsc;
    use std::time::Duration;

    let tmp = tempfile::tempdir().unwrap();
    let input = tmp.path().join("src.mp4");
    let made = ffmpeg(&[
        "-f", "lavfi", "-i", "testsrc2=s=640x360:r=30:d=12",
        "-f", "lavfi", "-i", "sine=f=440:d=12",
        "-c:v", "libx264", "-preset", "ultrafast", "-c:a", "aac", "-shortest", input.to_str().unwrap(),
    ]);
    if made.is_none() {
        eprintln!("ffmpeg с libx264 недоступен — тест пропущен");
        return;
    }
    let mut j = job(tmp.path(), &input, "long.mp4");
    j.end = 11.5;
    let dest = j.dest.clone();
    let (tx, rx) = mpsc::channel();
    let tx = std::sync::Mutex::new(tx);
    let q = Queue::start(vec![j], 3, move |ev| {
        let _ = tx.lock().unwrap().send(ev);
    });
    // Дождаться прогресса — ffmpeg точно запущен.
    loop {
        match rx.recv_timeout(Duration::from_secs(30)).expect("прогресс") {
            QueueEvent::Progress { fraction, .. } if fraction > 0.0 => break,
            QueueEvent::Finished { result, .. } => panic!("закончился раньше паузы: {result:?}"),
            _ => {}
        }
    }
    q.pause();
    std::thread::sleep(Duration::from_millis(300));
    q.cancel_all();
    let mut cancelled = false;
    while let Ok(ev) = rx.recv_timeout(Duration::from_secs(10)) {
        match ev {
            QueueEvent::Finished { result, .. } => cancelled = matches!(result, Err(RenderError::Cancelled)),
            QueueEvent::Done => break,
            _ => {}
        }
    }
    assert!(cancelled);
    assert!(!dest.exists() && !tmp.path().join(".long.mp4.part").exists());
}
