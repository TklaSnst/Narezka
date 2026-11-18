//! Описание шортса рядом с видео (`<имя>_<n>.txt`, SPEC §6): заголовок, таймкоды в исходнике, текст, хэштеги.

/// YouTube не показывает ни одного хэштега, если их больше 15.
pub const MAX_HASHTAGS: usize = 15;

/// Подписи строк файла (из i18n).
pub struct MetaLabels {
    /// «Исходник».
    pub source: String,
    /// «Таймкоды».
    pub time: String,
}

/// Что пишется в файл.
pub struct ShortMeta {
    pub title: String,
    /// Имя файла исходника.
    pub source: String,
    /// Отрезок исходника, секунды.
    pub start: f64,
    pub end: f64,
    /// Слова речи шортса (с правками).
    pub words: Vec<String>,
    pub hashtags: Vec<String>,
}

/// Хэштег из слова или фразы: только буквы, цифры и `_`, слова слитно, строчными; пусто → `None`.
/// «Lumen VPN» → `#lumenvpn`, «#Linux,» → `#linux`.
pub fn hashtag(phrase: &str) -> Option<String> {
    let tag: String = phrase.chars().filter(|c| c.is_alphanumeric() || *c == '_').flat_map(char::to_lowercase).collect();
    (!tag.is_empty() && !tag.chars().all(|c| c == '_')).then(|| format!("#{tag}"))
}

/// Хэштеги шортса: сначала общие пачки (`extra` — через пробел, запятую или `#`), затем ключевые слова нарезки
/// (фраза — одним тегом); без повторов, не больше [`MAX_HASHTAGS`].
pub fn hashtags(extra: &str, keywords: &[String]) -> Vec<String> {
    let common = extra.split(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | '#')).filter_map(hashtag);
    let mut out: Vec<String> = Vec::new();
    for tag in common.chain(keywords.iter().filter_map(|k| hashtag(k))) {
        if out.len() < MAX_HASHTAGS && !out.contains(&tag) {
            out.push(tag);
        }
    }
    out
}

/// Время в исходнике с десятыми: 75.44 → «1:15.4», 6612.05 → «1:50:12.1».
pub fn timecode(secs: f64) -> String {
    let ds = (secs.max(0.0) * 10.0).round() as u64;
    let (t, d) = (ds / 10, ds % 10);
    let (h, m, s) = (t / 3600, t / 60 % 60, t % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}.{d}") } else { format!("{m}:{s:02}.{d}") }
}

/// Текст файла. Пустые части (нет заголовка, речи, хэштегов) пропускаются.
pub fn format(meta: &ShortMeta, labels: &MetaLabels) -> String {
    let mut parts = Vec::new();
    let title = meta.title.trim();
    if !title.is_empty() {
        parts.push(title.to_owned());
    }
    parts.push(format!(
        "{}: {}\n{}: {} – {} ({})",
        labels.source,
        meta.source,
        labels.time,
        timecode(meta.start),
        timecode(meta.end),
        timecode(meta.end - meta.start)
    ));
    let text = meta.words.iter().map(|w| w.trim()).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ");
    if !text.is_empty() {
        parts.push(text);
    }
    if !meta.hashtags.is_empty() {
        parts.push(meta.hashtags.join(" "));
    }
    parts.join("\n\n") + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashtags_from_batch_and_keywords() {
        assert_eq!(hashtag("Lumen VPN"), Some("#lumenvpn".into()));
        assert_eq!(hashtag("«Сервер»,"), Some("#сервер".into()));
        assert_eq!(hashtag(" - "), None);
        let kw = vec!["сервер".to_owned(), "Lumen VPN".into(), "!".into()];
        assert_eq!(hashtags("#Shorts, #lumenvpn linux", &kw), ["#shorts", "#lumenvpn", "#linux", "#сервер"]);
        let many: Vec<String> = (0..30).map(|i| format!("t{i}")).collect();
        assert_eq!(hashtags("", &many).len(), MAX_HASHTAGS);
    }

    #[test]
    fn file_text() {
        assert_eq!(timecode(75.44), "1:15.4");
        assert_eq!(timecode(6612.05), "1:50:12.1");
        let labels = MetaLabels { source: "Исходник".into(), time: "Таймкоды".into() };
        let mut meta = ShortMeta {
            title: " Как поднять мост ".into(),
            source: "ролик.mp4".into(),
            start: 63.0,
            end: 110.6,
            words: vec!["Настроим".into(), " сервер,".into(), "быстро.".into()],
            hashtags: vec!["#shorts".into(), "#сервер".into()],
        };
        assert_eq!(
            format(&meta, &labels),
            "Как поднять мост\n\nИсходник: ролик.mp4\nТаймкоды: 1:03.0 – 1:50.6 (0:47.6)\n\nНастроим сервер, быстро.\n\n#shorts #сервер\n"
        );
        meta.title.clear();
        meta.words.clear();
        meta.hashtags.clear();
        assert_eq!(format(&meta, &labels), "Исходник: ролик.mp4\nТаймкоды: 1:03.0 – 1:50.6 (0:47.6)\n");
    }
}
