use std::collections::BTreeMap;
use std::sync::OnceLock;

pub const SUPPORTED_LANGUAGES: &[&str] = &["en", "zh-CN"];
const ENGLISH: &str = include_str!("../locales/en.toml");
const CHINESE: &str = include_str!("../locales/zh-CN.toml");

#[derive(Debug)]
pub struct Locale {
    strings: BTreeMap<String, String>,
}

impl Locale {
    pub fn load(english: &str, translation: Option<&str>) -> Result<Self, toml::de::Error> {
        let mut strings: BTreeMap<String, String> = toml::from_str(english)?;
        if let Some(translation) = translation {
            let selected: BTreeMap<String, String> = toml::from_str(translation)?;
            for (key, value) in selected {
                if strings.contains_key(&key) {
                    strings.insert(key, value);
                }
            }
        }
        Ok(Self { strings })
    }

    pub fn get<'a>(&'a self, key: &'a str) -> &'a str {
        self.strings
            .get(key)
            .map(String::as_str)
            .unwrap_or_else(|| {
                tracing::error!("Missing English locale key: {key}");
                key
            })
    }
}

fn bundled(language: &str) -> Result<&'static Locale, &'static toml::de::Error> {
    static EN: OnceLock<Result<Locale, toml::de::Error>> = OnceLock::new();
    static ZH: OnceLock<Result<Locale, toml::de::Error>> = OnceLock::new();
    let english = EN.get_or_init(|| Locale::load(ENGLISH, None));
    let english = english.as_ref()?;
    if language == "zh-CN" {
        match ZH.get_or_init(|| {
            let result = Locale::load(ENGLISH, Some(CHINESE));
            if let Err(error) = &result {
                tracing::warn!("Bundled zh-CN locale is invalid; using English: {error}");
            }
            result
        }) {
            Ok(locale) => return Ok(locale),
            Err(_) => return Ok(english),
        }
    }
    Ok(english)
}

pub fn text(language: &str, key: &str) -> String {
    match bundled(language) {
        Ok(locale) => locale.get(key).to_string(),
        Err(error) => {
            tracing::error!("Bundled English locale is invalid: {error}");
            key.to_string()
        }
    }
}

pub fn format(language: &str, key: &str, values: &[(&str, &str)]) -> String {
    substitute(&text(language, key), values)
}

fn substitute(template: &str, values: &[(&str, &str)]) -> String {
    let mut result = String::new();
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        result.push_str(&rest[..start]);
        let Some(end) = rest[start..].find('}') else {
            result.push_str(&rest[start..]);
            return result;
        };
        let name = &rest[start + 1..start + end];
        if let Some((_, value)) = values.iter().find(|(key, _)| *key == name) {
            result.push_str(value);
        } else {
            result.push_str(&rest[start..=start + end]);
        }
        rest = &rest[start + end + 1..];
    }
    result.push_str(rest);
    result
}

pub fn page_json(language: &str) -> String {
    match bundled(language) {
        Ok(locale) => serde_json::to_string(&locale.strings).unwrap_or_else(|_| "{}".to_string()),
        Err(_) => "{}".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_catalogs_are_complete_with_matching_placeholders() {
        let english: BTreeMap<String, String> = toml::from_str(ENGLISH).unwrap();
        let chinese: BTreeMap<String, String> = toml::from_str(CHINESE).unwrap();
        assert_eq!(
            english.keys().collect::<Vec<_>>(),
            chinese.keys().collect::<Vec<_>>()
        );
        let placeholders = regex::Regex::new(r"\{([A-Za-z_][A-Za-z_0-9]*)\}").unwrap();
        for (key, value) in &english {
            let names = |text: &str| {
                let mut names = placeholders
                    .captures_iter(text)
                    .map(|capture| capture[1].to_string())
                    .collect::<Vec<_>>();
                names.sort();
                names
            };
            assert_eq!(names(value), names(&chinese[key]), "{key}");
        }
    }

    #[test]
    fn loader_rejects_invalid_and_duplicate_keys() {
        for invalid in [
            "not TOML",
            "\"key\" = \"one\"\n\"key\" = \"two\"",
            "key = 3",
            "[nested]\nkey = \"value\"",
        ] {
            assert!(Locale::load(invalid, None).is_err());
            assert!(Locale::load("key = \"English\"", Some(invalid)).is_err());
        }
    }

    #[test]
    fn missing_translation_and_unsupported_language_use_english() {
        let locale = Locale::load(
            "one = \"English\"\ntwo = \"Fallback\"",
            Some("one = \"翻译\""),
        )
        .unwrap();
        assert_eq!(locale.get("one"), "翻译");
        assert_eq!(locale.get("two"), "Fallback");
        assert_eq!(page_json("unsupported"), page_json("en"));
    }

    #[test]
    fn substitution_is_single_pass_and_preserves_unknown_placeholders() {
        assert_eq!(
            substitute(
                "{name}: {count} {unknown}",
                &[("name", "{count}"), ("count", "2")]
            ),
            "{count}: 2 {unknown}"
        );
    }

    #[test]
    fn referenced_keys_exist_in_english() {
        let english: BTreeMap<String, String> = toml::from_str(ENGLISH).unwrap();
        let references = regex::Regex::new(
            r#"["']((?:settings|tray|notification|dialog)\.(?:[a-z_0-9-]+\.)*[a-z_0-9-]+)["']"#,
        )
        .unwrap();
        for source in [
            include_str!("settings/html.rs"),
            include_str!("tray.rs"),
            include_str!("event_handler.rs"),
            include_str!("main.rs"),
            include_str!("settings/win32.rs"),
        ] {
            for capture in references.captures_iter(source) {
                assert!(
                    english.contains_key(&capture[1]),
                    "Unknown locale key: {}",
                    &capture[1]
                );
            }
        }
    }
}
