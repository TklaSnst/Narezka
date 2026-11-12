//! Модель пачки: правила шортсов и сериализация (TOML и JSON).

use sc_core::*;

fn batch_with_source(duration: f64) -> (Batch, SourceId) {
    let mut b = Batch::default();
    let mut src = Source::new("/video/длинный ролик.mp4");
    src.info = Some(MediaInfo { duration, width: 1920, height: 1080, fps: 60.0, has_audio: true });
    src.hash = Some("abc123".into());
    src.status = SourceStatus::Ready;
    let id = b.add_source(src);
    (b, id)
}

#[test]
fn add_source_dedups_by_path() {
    let (mut b, id) = batch_with_source(100.0);
    assert_eq!(b.add_source(Source::new("/video/длинный ролик.mp4")), id);
    assert_eq!(b.sources.len(), 1);
}

#[test]
fn short_rules() {
    let (mut b, src) = batch_with_source(100.0);
    let a = b.add_short(Short::manual(src, 10.0, 40.0)).unwrap();

    assert_eq!(
        b.add_short(Short::manual(src, 30.0, 50.0)),
        Err(BatchError::Overlap(a)),
        "перекрытие запрещено"
    );
    // Касание границ — не перекрытие.
    let c = b.add_short(Short::manual(src, 40.0, 60.0)).unwrap();
    assert!(matches!(b.add_short(Short::manual(src, 5.0, 5.0)), Err(BatchError::BadRange { .. })));
    assert!(matches!(
        b.add_short(Short::manual(src, 90.0, 101.0)),
        Err(BatchError::OutOfSource { .. })
    ));
    assert_eq!(
        b.add_short(Short::manual(SourceId::new(), 0.0, 1.0)),
        Err(BatchError::UnknownSource)
    );

    // Редактирование самого себя не считается перекрытием.
    let mut edited = b.short(c).unwrap().clone();
    edited.end = 70.0;
    b.update_short(edited).unwrap();

    // Отклонённые не мешают.
    let mut rejected = b.short(a).unwrap().clone();
    rejected.status = ShortStatus::Rejected;
    b.update_short(rejected).unwrap();
    b.add_short(Short::manual(src, 0.0, 15.0)).unwrap();

    let starts: Vec<f64> = b.shorts_of(src).iter().map(|s| s.start).collect();
    assert_eq!(starts, [0.0, 10.0, 40.0]);

    b.remove_source(src).unwrap();
    assert!(b.shorts.is_empty());
}

#[test]
fn batch_serde_roundtrip() {
    let (mut b, src) = batch_with_source(3600.5);
    let mut s = Short::manual(src, 12.25, 58.5);
    s.title = "Почему «Lumen» — это *быстро*".into();
    s.score = Some(0.82);
    s.status = ShortStatus::Candidate;
    b.add_short(s).unwrap();
    b.sources.push(Source {
        status: SourceStatus::Failed("ffprobe: нет видеопотока".into()),
        ..Source::new("/video/broken.mkv")
    });
    b.output.resolution = Some(Canvas { width: 720, height: 1280 });
    b.output.fps = Some(30.0);

    let toml_text = toml::to_string_pretty(&b).unwrap();
    assert_eq!(toml::from_str::<Batch>(&toml_text).unwrap(), b, "{toml_text}");

    let json = serde_json::to_string(&b).unwrap();
    assert_eq!(serde_json::from_str::<Batch>(&json).unwrap(), b);
}
