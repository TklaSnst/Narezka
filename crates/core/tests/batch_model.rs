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

fn candidate(src: SourceId, start: f64, end: f64, score: f32) -> Short {
    Short { status: ShortStatus::Candidate, score: Some(score), auto: true, ..Short::manual(src, start, end) }
}

#[test]
fn candidates_accept_and_overlap_option() {
    let (mut b, src) = batch_with_source(100.0);
    let a = b.add_short(Short::manual(src, 10.0, 40.0)).unwrap();
    // Кандидаты могут перекрываться с чем угодно…
    let c = b.add_short(candidate(src, 30.0, 50.0, 7.0)).unwrap();
    // …но принять пересекающийся с принятым нельзя.
    assert_eq!(b.set_status(c, ShortStatus::Accepted), Err(BatchError::Overlap(a)));
    assert_eq!(b.short(c).unwrap().status, ShortStatus::Candidate);
    b.allow_overlap = true;
    b.set_status(c, ShortStatus::Accepted).unwrap();
    b.allow_overlap = false;
    b.set_status(c, ShortStatus::Rejected).unwrap();
}

#[test]
fn split_and_merge() {
    let (mut b, src) = batch_with_source(100.0);
    let a = b.add_short(candidate(src, 10.0, 40.0, 6.0)).unwrap();
    assert_eq!(b.split_short(a, 10.2), Err(BatchError::BadSplit));
    assert_eq!(b.split_short(a, 50.0), Err(BatchError::BadSplit));
    let second = b.split_short(a, 25.0).unwrap();
    assert_eq!((b.short(a).unwrap().end, b.short(second).unwrap().start), (25.0, 25.0));
    assert_eq!(b.short(second).unwrap().status, ShortStatus::Candidate);

    let c = b.add_short(Short { title: "лучший".into(), ..candidate(src, 50.0, 60.0, 9.0) }).unwrap();
    b.set_status(c, ShortStatus::Accepted).unwrap();
    // Отклонённый между ними пропускается.
    b.set_status(second, ShortStatus::Rejected).unwrap();
    assert_eq!(b.next_short(a).unwrap().id, c);
    b.merge_with_next(a).unwrap();
    let m = b.short(a).unwrap();
    assert_eq!((m.start, m.end, m.status, m.title.as_str()), (10.0, 60.0, ShortStatus::Accepted, "лучший"));
    assert!(b.short(c).is_none());
    assert_eq!(b.merge_with_next(a), Err(BatchError::NothingToMerge));

    // Объединение не пройдёт через принятый шортс — ничего не теряется.
    let d = b.add_short(Short::manual(src, 70.0, 80.0)).unwrap();
    let e = b.add_short(candidate(src, 85.0, 90.0, 5.0)).unwrap();
    let mut wide = b.short(e).unwrap().clone();
    wide.start = 65.0;
    b.update_short(wide).unwrap();
    // e начинается раньше d — следующий для a теперь e, и принятый итог перекрыл бы d.
    assert_eq!(b.merge_with_next(a), Err(BatchError::Overlap(d)));
    assert!(b.short(e).is_some());
}

#[test]
fn replace_candidates_keeps_accepted() {
    let (mut b, src) = batch_with_source(100.0);
    let old = b.add_short(candidate(src, 0.0, 20.0, 5.0)).unwrap();
    let kept = b.add_short(candidate(src, 30.0, 50.0, 8.0)).unwrap();
    b.set_status(kept, ShortStatus::Accepted).unwrap();
    let manual = b.add_short(Short { status: ShortStatus::Rejected, ..Short::manual(src, 60.0, 70.0) }).unwrap();

    b.replace_candidates(src, vec![candidate(src, 0.0, 25.0, 6.0), candidate(src, 31.0, 49.0, 8.0), candidate(src, 45.0, 75.0, 4.0)]);
    assert!(b.short(old).is_none());
    assert!(b.short(kept).is_some() && b.short(manual).is_some(), "принятые и ручные остаются");
    let starts: Vec<f64> = b.shorts_of(src).iter().map(|s| s.start).collect();
    // 31–49 почти целиком внутри принятого — не добавлен; 45–75 занят принятым лишь на 5 с из 30.
    assert_eq!(starts, [0.0, 30.0, 45.0, 60.0]);
}

#[test]
fn part_number_counts_accepted_of_same_source() {
    let (mut b, src) = batch_with_source(600.0);
    let mut s2 = b.source(src).unwrap().clone();
    s2.path = "/video/второй.mp4".into();
    s2.id = SourceId::default();
    let other = b.add_source(s2);
    let mut add = |source, start: f64, status| {
        let mut s = Short::manual(source, start, start + 20.0);
        s.status = status;
        b.add_short(s).unwrap()
    };
    let first = add(src, 0.0, ShortStatus::Accepted);
    let cand = add(src, 30.0, ShortStatus::Candidate);
    let second = add(src, 60.0, ShortStatus::Accepted);
    let elsewhere = add(other, 0.0, ShortStatus::Accepted);
    assert_eq!(b.part_number(first), 1);
    // кандидат получит номер 2, если его принять
    assert_eq!(b.part_number(cand), 2);
    assert_eq!(b.part_number(second), 2);
    assert_eq!(b.part_number(elsewhere), 1);
}

#[test]
fn short_template_overrides_batch() {
    let (mut b, src) = batch_with_source(100.0);
    assert_eq!(b.template_ref, Some(sc_core::TemplateRef::Builtin(sc_core::library::DEFAULT_BUILTIN.into())));
    let a = b.add_short(Short::manual(src, 10.0, 40.0)).unwrap();
    assert_eq!(b.template_for(a), &b.template, "без правок — шаблон пачки");

    let own = sc_core::library::builtin("split").unwrap();
    b.short_mut(a).unwrap().template = Some(own.clone());
    assert_eq!(b.template_for(a), &own);
    // Правки едут в обе половины при разделении.
    let second = b.split_short(a, 25.0).unwrap();
    assert_eq!(b.template_for(second), &own);
    // И переживают сериализацию.
    let json = serde_json::to_string(&b).unwrap();
    assert_eq!(serde_json::from_str::<Batch>(&json).unwrap(), b);
}

#[test]
fn insert_per_short_and_preset() {
    let (mut b, src) = batch_with_source(100.0);
    let a = b.add_short(Short::manual(src, 10.0, 40.0)).unwrap();
    b.short_mut(a).unwrap().insert_at = Some(25.0);
    assert!(b.insert_for(a).is_none(), "у пачки нет вставки");
    let asset = AssetId::new();
    b.insert = Some(BatchInsert { asset, mode: None });
    assert_eq!(b.insert_for(a).map(|(i, at)| (i.asset, at)), Some((asset, 25.0)));
    // вне шортса (после правки краёв) — не считается
    b.short_mut(a).unwrap().insert_at = Some(40.1);
    assert!(b.insert_for(a).is_none());
    b.short_mut(a).unwrap().insert_at = Some(25.0);
    b.short_mut(a).unwrap().no_insert = true;
    assert!(b.insert_for(a).is_none(), "шортс без вставки");
    b.short_mut(a).unwrap().no_insert = false;

    // при разделении место остаётся у той части, где оно есть
    let second = b.split_short(a, 20.0).unwrap();
    assert!(b.insert_for(a).is_none());
    assert_eq!(b.insert_for(second).map(|(_, at)| at), Some(25.0));
    b.merge_with_next(a).unwrap();
    assert_eq!(b.insert_for(a).map(|(_, at)| at), Some(25.0));

    // «как в прошлый раз» переносит вставку пачки
    let p = b.preset(1);
    let toml_text = toml::to_string_pretty(&p).unwrap();
    let back: BatchPreset = toml::from_str(&toml_text).unwrap();
    let mut fresh = Batch::default();
    fresh.apply_preset(&back);
    assert_eq!(fresh.insert, b.insert);
}
