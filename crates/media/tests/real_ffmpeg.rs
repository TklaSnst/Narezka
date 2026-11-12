//! Проверки на системном ffmpeg; без него тесты пропускаются.

use std::process::Command;

use sc_media::{Capabilities, probe, quick_hash};

fn have_ffmpeg() -> bool {
    Command::new("ffmpeg").arg("-version").output().is_ok_and(|o| o.status.success())
}

#[test]
fn probe_generated_clip() {
    if !have_ffmpeg() {
        eprintln!("ffmpeg не найден — пропуск");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("clip.mkv");
    let ok = Command::new("ffmpeg")
        .args(["-nostdin", "-v", "error", "-f", "lavfi", "-i", "testsrc2=s=640x360:r=25:d=2"])
        .args(["-f", "lavfi", "-i", "sine=d=2", "-c:v", "ffv1", "-c:a", "flac", "-shortest"])
        .arg(&path)
        .status()
        .unwrap()
        .success();
    assert!(ok);

    let info = probe(&path).unwrap();
    assert_eq!((info.width, info.height), (640, 360));
    assert_eq!(info.fps, 25.0);
    assert!(info.has_audio);
    assert!((info.duration - 2.0).abs() < 0.1, "{}", info.duration);
    assert!(quick_hash(&path).is_ok());

    let txt = dir.path().join("not-video.mp4");
    std::fs::write(&txt, "hello").unwrap();
    assert!(probe(&txt).is_err());
}

#[test]
fn detect_capabilities() {
    if !have_ffmpeg() {
        return;
    }
    let caps = Capabilities::detect();
    eprintln!("{caps:#?}");
    assert!(caps.usable());
    assert!(caps.missing_filters.is_empty(), "{:?}", caps.missing_filters);
    // libx264 есть в любой нормальной сборке ffmpeg — значит пробное кодирование вообще работает.
    assert!(caps.works("libx264"));
}
