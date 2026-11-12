//! Имена выходных файлов: `<имя>_1.mp4`, `<имя>_2.mp4`…

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use sc_core::{Batch, ConflictPolicy, ShortId, ShortStatus};

/// Базовое имя без символов, недопустимых в имени файла; пустое → `short`.
pub fn sanitize_base(name: &str) -> String {
    let s: String = name
        .trim()
        .chars()
        .map(|c| if c == '/' || c == '\0' || c.is_control() { '_' } else { c })
        .collect();
    let s = s.trim_matches('.').to_owned();
    if s.is_empty() { "short".into() } else { s }
}

/// Наибольший номер `N` среди файлов `<base>_N.<ext>` в каталоге (0 — таких нет).
pub fn max_existing(dir: &Path, base: &str, ext: &str) -> u32 {
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    let prefix = format!("{base}_");
    let suffix = format!(".{ext}");
    entries
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|name| {
            let n = name.strip_prefix(&prefix)?.strip_suffix(&suffix)?;
            (!n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())).then(|| n.parse().ok()).flatten()
        })
        .max()
        .unwrap_or(0)
}

/// Принятые шортсы пачки (по порядку исходников, затем по времени) и их выходные файлы.
///
/// Нумерация сквозная в пределах каталога. «Продолжить» — после самого большого существующего номера,
/// «Перезаписать» — с 1. «Спросить» пока ведёт себя как «Продолжить» (диалог — этап 7).
pub fn plan_outputs(batch: &Batch) -> Vec<(ShortId, PathBuf)> {
    let out = &batch.output;
    let base = sanitize_base(&out.base_name);
    let ext = out.container.extension();
    let mut next: HashMap<PathBuf, u32> = HashMap::new();
    let mut plan = Vec::new();
    for source in &batch.sources {
        let dir = match &out.dir {
            Some(d) => d.clone(),
            None => source.path.parent().map(Path::to_path_buf).unwrap_or_default(),
        };
        for short in batch.shorts_of(source.id).into_iter().filter(|s| s.status == ShortStatus::Accepted) {
            let n = next.entry(dir.clone()).or_insert_with(|| match out.conflict_policy {
                ConflictPolicy::Overwrite => 0,
                ConflictPolicy::Continue | ConflictPolicy::Ask => max_existing(&dir, &base, ext),
            });
            *n += 1;
            plan.push((short.id, dir.join(format!("{base}_{n}.{ext}"))));
        }
    }
    plan
}

/// Временный файл рядом с итоговым: `.<имя>.part` — недописанный файл не выглядит готовым.
pub fn part_path(dest: &Path) -> PathBuf {
    let name = dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    dest.with_file_name(format!(".{name}.part"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sc_core::{MediaInfo, Short, Source};

    fn batch_with(dir: Option<PathBuf>, src_dir: &Path) -> Batch {
        let mut b = Batch::default();
        b.output.dir = dir;
        b.output.base_name = "клип".into();
        for (name, starts) in [("a.mp4", &[30.0, 0.0][..]), ("b.mp4", &[5.0][..])] {
            let mut s = Source::new(src_dir.join(name));
            s.info = Some(MediaInfo { duration: 100.0, width: 1920, height: 1080, fps: 30.0, has_audio: true });
            let id = b.add_source(s);
            for &st in starts {
                b.add_short(Short::manual(id, st, st + 10.0)).unwrap();
            }
        }
        // отклонённый шортс не рендерится
        let mut rejected = Short::manual(b.sources[1].id, 50.0, 60.0);
        rejected.status = ShortStatus::Rejected;
        b.add_short(rejected).unwrap();
        b
    }

    #[test]
    fn continues_numbering_in_output_dir() {
        let tmp = tempfile::tempdir().unwrap();
        for f in ["клип_1.mp4", "клип_7.mp4", "клип_x.mp4", "клип_9.mkv", "другое_20.mp4"] {
            std::fs::write(tmp.path().join(f), b"").unwrap();
        }
        assert_eq!(max_existing(tmp.path(), "клип", "mp4"), 7);
        let b = batch_with(Some(tmp.path().into()), Path::new("/src"));
        let plan = plan_outputs(&b);
        let names: Vec<_> = plan.iter().map(|(_, p)| p.file_name().unwrap().to_str().unwrap().to_owned()).collect();
        assert_eq!(names, ["клип_8.mp4", "клип_9.mp4", "клип_10.mp4"]);
        // a.mp4 — сначала шортс с 0 с, потом с 30 с
        assert_eq!(plan[0].0, b.shorts[1].id);
    }

    #[test]
    fn overwrite_and_next_to_source() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("клип_3.mp4"), b"").unwrap();
        let mut b = batch_with(None, tmp.path());
        b.output.conflict_policy = ConflictPolicy::Overwrite;
        let plan = plan_outputs(&b);
        assert_eq!(plan[0].1, tmp.path().join("клип_1.mp4"));
        assert_eq!(plan[2].1, tmp.path().join("клип_3.mp4"));
    }

    #[test]
    fn base_and_part_names() {
        assert_eq!(sanitize_base("  a/b\tc "), "a_b_c");
        assert_eq!(sanitize_base(" .. "), "short");
        assert_eq!(part_path(Path::new("/x/клип_1.mp4")), Path::new("/x/.клип_1.mp4.part"));
    }
}
