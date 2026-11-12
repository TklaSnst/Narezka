//! Тексты интерфейса через Fluent. Ресурсы `locales/*.ftl` встроены в бинарник.
//!
//! Ключи — kebab-case; отсутствующий ключ возвращается как есть и пишется в лог.

use fluent_bundle::concurrent::FluentBundle;
use fluent_bundle::{FluentArgs, FluentResource, FluentValue};
use sc_core::Language;
use unic_langid::{LanguageIdentifier, langid};

const RU: &str = include_str!("../locales/ru.ftl");
const EN: &str = include_str!("../locales/en.ftl");

pub fn source(lang: Language) -> &'static str {
    match lang {
        Language::Ru => RU,
        Language::En => EN,
    }
}

fn langid(lang: Language) -> LanguageIdentifier {
    match lang {
        Language::Ru => langid!("ru"),
        Language::En => langid!("en"),
    }
}

pub struct I18n {
    lang: Language,
    bundle: FluentBundle<FluentResource>,
}

impl I18n {
    pub fn new(lang: Language) -> Self {
        let resource = FluentResource::try_new(source(lang).to_owned())
            .unwrap_or_else(|(_, errs)| panic!("ошибка синтаксиса в {}.ftl: {errs:?}", lang.code()));
        let mut bundle = FluentBundle::new_concurrent(vec![langid(lang)]);
        // Метки изоляции направления текста (U+2068/2069) egui рисует как мусор.
        bundle.set_use_isolating(false);
        bundle
            .add_resource(resource)
            .unwrap_or_else(|errs| panic!("повтор ключей в {}.ftl: {errs:?}", lang.code()));
        Self { lang, bundle }
    }

    pub fn lang(&self) -> Language {
        self.lang
    }

    pub fn set_lang(&mut self, lang: Language) {
        if lang != self.lang {
            *self = Self::new(lang);
        }
    }

    pub fn tr(&self, key: &str) -> String {
        self.format(key, None)
    }

    /// Текст с аргументами: `tr_args("batch-summary", &[("sources", 3.into())])`.
    pub fn tr_args(&self, key: &str, args: &[(&str, FluentValue<'_>)]) -> String {
        let mut fa = FluentArgs::new();
        for (name, value) in args {
            fa.set(*name, value.clone());
        }
        self.format(key, Some(&fa))
    }

    fn format(&self, key: &str, args: Option<&FluentArgs<'_>>) -> String {
        let Some(pattern) = self.bundle.get_message(key).and_then(|m| m.value()) else {
            log::warn!("нет перевода {key} ({})", self.lang.code());
            return key.to_owned();
        };
        let mut errors = Vec::new();
        let text = self.bundle.format_pattern(pattern, args, &mut errors);
        if !errors.is_empty() {
            log::warn!("ошибки в переводе {key} ({}): {errors:?}", self.lang.code());
        }
        text.into_owned()
    }
}

pub use fluent_bundle::FluentValue as Value;

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use fluent_syntax::ast::Entry;
    use fluent_syntax::parser;

    use super::*;

    fn keys(lang: Language) -> BTreeSet<String> {
        let ast = parser::parse(source(lang)).expect("ftl без ошибок синтаксиса");
        ast.body
            .iter()
            .filter_map(|e| match e {
                Entry::Message(m) => Some(m.id.name.to_owned()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn locales_have_same_keys() {
        let ru = keys(Language::Ru);
        let en = keys(Language::En);
        assert_eq!(ru.difference(&en).collect::<Vec<_>>(), Vec::<&String>::new(), "нет в en.ftl");
        assert_eq!(en.difference(&ru).collect::<Vec<_>>(), Vec::<&String>::new(), "нет в ru.ftl");
    }

    #[test]
    fn switching_language() {
        let mut i = I18n::new(Language::Ru);
        assert_eq!(i.tr("tab-settings"), "Настройки");
        i.set_lang(Language::En);
        assert_eq!(i.tr("tab-settings"), "Settings");
        assert_eq!(i.tr("no-such-key"), "no-such-key");
    }

    #[test]
    fn plurals_and_args() {
        let ru = I18n::new(Language::Ru);
        let n = |k: i64| ru.tr_args("batch-summary", &[("sources", k.into())]);
        assert_eq!(n(0), "Пачка пуста");
        assert_eq!(n(1), "В пачке 1 ролик");
        assert_eq!(n(3), "В пачке 3 ролика");
        assert_eq!(n(5), "В пачке 5 роликов");
        assert_eq!(n(21), "В пачке 21 ролик");
        let en = I18n::new(Language::En);
        assert_eq!(en.tr_args("batch-summary", &[("sources", 2.into())]), "2 videos in the batch");
        assert_eq!(
            en.tr_args("settings-save-failed", &[("error", "disk full".into())]),
            "Failed to save settings: disk full"
        );
    }
}
