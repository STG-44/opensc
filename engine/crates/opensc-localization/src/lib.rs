//! Shared, offline UI language contract for the launcher and engine.
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::{collections::BTreeMap, sync::OnceLock};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub enum Locale {
    #[default]
    #[serde(rename = "ru")]
    Russian,
    #[serde(rename = "en")]
    English,
    #[serde(rename = "uk")]
    Ukrainian,
    #[serde(rename = "de")]
    German,
    #[serde(rename = "fr")]
    French,
    #[serde(rename = "es")]
    Spanish,
    #[serde(rename = "it")]
    Italian,
    #[serde(rename = "pt-BR")]
    Portuguese,
    #[serde(rename = "pl")]
    Polish,
    #[serde(rename = "cs")]
    Czech,
    #[serde(rename = "tr")]
    Turkish,
    #[serde(rename = "zh-Hans")]
    ChineseSimplified,
    #[serde(rename = "zh-Hant")]
    ChineseTraditional,
    #[serde(rename = "ja")]
    Japanese,
    #[serde(rename = "ko")]
    Korean,
}

impl Locale {
    pub const ALL: [Self; 15] = [
        Self::Russian,
        Self::English,
        Self::Ukrainian,
        Self::German,
        Self::French,
        Self::Spanish,
        Self::Italian,
        Self::Portuguese,
        Self::Polish,
        Self::Czech,
        Self::Turkish,
        Self::ChineseSimplified,
        Self::ChineseTraditional,
        Self::Japanese,
        Self::Korean,
    ];
    pub fn code(self) -> &'static str {
        match self {
            Self::Russian => "ru",
            Self::English => "en",
            Self::Ukrainian => "uk",
            Self::German => "de",
            Self::French => "fr",
            Self::Spanish => "es",
            Self::Italian => "it",
            Self::Portuguese => "pt-BR",
            Self::Polish => "pl",
            Self::Czech => "cs",
            Self::Turkish => "tr",
            Self::ChineseSimplified => "zh-Hans",
            Self::ChineseTraditional => "zh-Hant",
            Self::Japanese => "ja",
            Self::Korean => "ko",
        }
    }
    pub fn native_name(self) -> &'static str {
        match self {
            Self::Russian => "Русский",
            Self::English => "English",
            Self::Ukrainian => "Українська",
            Self::German => "Deutsch",
            Self::French => "Français",
            Self::Spanish => "Español",
            Self::Italian => "Italiano",
            Self::Portuguese => "Português (Brasil)",
            Self::Polish => "Polski",
            Self::Czech => "Čeština",
            Self::Turkish => "Türkçe",
            Self::ChineseSimplified => "简体中文",
            Self::ChineseTraditional => "繁體中文",
            Self::Japanese => "日本語",
            Self::Korean => "한국어",
        }
    }
    pub fn parse(code: &str) -> Result<Self, &'static str> {
        let canonical = match code {
            "zh" | "zh-CN" => "zh-Hans",
            "zh-TW" => "zh-Hant",
            "pt" => "pt-BR",
            other => other,
        };
        Self::ALL
            .into_iter()
            .find(|l| l.code().eq_ignore_ascii_case(canonical))
            .ok_or("unsupported UI language")
    }
    pub fn cjk_font(self) -> Option<&'static [u8]> {
        static FONTS: [OnceLock<Vec<u8>>; 4] = [const { OnceLock::new() }; 4];
        let (index, compressed): (usize, &[u8]) = match self {
            Self::ChineseSimplified => (0, include_bytes!("../assets/fonts/OpenSC-UI-SC.ttf.gz")),
            Self::ChineseTraditional => (1, include_bytes!("../assets/fonts/OpenSC-UI-TC.ttf.gz")),
            Self::Japanese => (2, include_bytes!("../assets/fonts/OpenSC-UI-JP.ttf.gz")),
            Self::Korean => (3, include_bytes!("../assets/fonts/OpenSC-UI-KR.ttf.gz")),
            _ => return None,
        };
        Some(
            FONTS[index]
                .get_or_init(|| {
                    let mut bytes = Vec::new();
                    flate2::read::GzDecoder::new(compressed)
                        .read_to_end(&mut bytes)
                        .expect("valid bundled UI font");
                    bytes
                })
                .as_slice(),
        )
    }
    pub fn translate(self, text: &str) -> String {
        self.translate_depth(text, 0)
    }
    fn translate_depth(self, text: &str, depth: u8) -> String {
        if self == Self::Russian || text.is_empty() {
            return text.into();
        }
        if text.contains('\n') {
            return text
                .split('\n')
                .map(|line| self.translate_depth(line, depth))
                .collect::<Vec<_>>()
                .join("\n");
        }
        let dictionary = &catalogs()[Self::ALL.iter().position(|l| *l == self).unwrap() - 1];
        if let Some(value) = dictionary.exact.get(text) {
            return value.clone();
        }
        if depth >= 8 {
            return dictionary.phrases(text);
        }
        for template in &dictionary.templates {
            if let Some(captures) = template.capture(text) {
                let mut rendered = template.target.clone();
                // Replace indices in one pass so user data containing braces is never interpreted.
                let mut result = String::new();
                while let Some(start) = rendered.find('{') {
                    result.push_str(&rendered[..start]);
                    let rest = &rendered[start + 1..];
                    if let Some(end) = rest.find('}')
                        && let Ok(index) = rest[..end].parse::<usize>()
                        && let Some(value) = captures.get(index)
                    {
                        if value.contains('/') || value.contains('\\') {
                            result.push_str(value);
                        } else {
                            result.push_str(&self.translate_depth(value, depth + 1));
                        }
                        rendered = rest[end + 1..].into();
                    } else {
                        result.push('{');
                        rendered = rest.into();
                    }
                }
                result.push_str(&rendered);
                return result;
            }
        }
        dictionary.phrases(text)
    }
}

struct Dictionary {
    exact: BTreeMap<String, String>,
    phrases: BTreeMap<char, Vec<(String, String)>>,
    templates: Vec<Template>,
}
struct Template {
    parts: Vec<String>,
    target: String,
    literal_bytes: usize,
}
impl Template {
    fn parse(source: &str, target: &str) -> Option<Self> {
        let mut rest = source;
        let mut parts = Vec::new();
        while let Some(start) = rest.find('{') {
            let end = rest[start..].find('}')? + start;
            if rest[start..].starts_with("{{") {
                return None;
            }
            parts.push(rest[..start].into());
            rest = &rest[end + 1..];
        }
        if parts.is_empty() {
            return None;
        }
        parts.push(rest.into());
        let literal_bytes = parts.iter().map(String::len).sum();
        (literal_bytes >= 3).then_some(Self {
            parts,
            target: target.into(),
            literal_bytes,
        })
    }
    fn capture<'a>(&self, text: &'a str) -> Option<Vec<&'a str>> {
        let mut remaining = text.strip_prefix(&self.parts[0])?;
        let mut captures = Vec::new();
        for (i, next) in self.parts.iter().enumerate().skip(1) {
            if i == self.parts.len() - 1 {
                captures.push(remaining.strip_suffix(next)?);
                remaining = "";
            } else {
                if next.is_empty() {
                    return None;
                }
                let end = remaining.find(next)?;
                captures.push(&remaining[..end]);
                remaining = &remaining[end + next.len()..];
            }
        }
        remaining.is_empty().then_some(captures)
    }
}
fn word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}
impl Dictionary {
    fn phrases(&self, text: &str) -> String {
        let mut output = String::with_capacity(text.len());
        let mut offset = 0;
        while offset < text.len() {
            let c = text[offset..].chars().next().unwrap();
            let found = self.phrases.get(&c).and_then(|entries| {
                entries.iter().find(|(source, _)| {
                    text[offset..].starts_with(source)
                        && (!source.starts_with(word)
                            || offset == 0
                            || !text[..offset].ends_with(word))
                        && (!source.ends_with(word)
                            || !text[offset + source.len()..].starts_with(word))
                })
            });
            if let Some((source, target)) = found {
                output.push_str(target);
                offset += source.len();
            } else {
                output.push(c);
                offset += c.len_utf8();
            }
        }
        output
    }
}
fn catalogs() -> &'static Vec<Dictionary> {
    static CATALOGS: OnceLock<Vec<Dictionary>> = OnceLock::new();
    CATALOGS.get_or_init(|| {
        let raw = [
            include_str!("../assets/locales/en.json"),
            include_str!("../assets/locales/uk.json"),
            include_str!("../assets/locales/de.json"),
            include_str!("../assets/locales/fr.json"),
            include_str!("../assets/locales/es.json"),
            include_str!("../assets/locales/it.json"),
            include_str!("../assets/locales/pt-BR.json"),
            include_str!("../assets/locales/pl.json"),
            include_str!("../assets/locales/cs.json"),
            include_str!("../assets/locales/tr.json"),
            include_str!("../assets/locales/zh-Hans.json"),
            include_str!("../assets/locales/zh-Hant.json"),
            include_str!("../assets/locales/ja.json"),
            include_str!("../assets/locales/ko.json"),
        ];
        let english: BTreeMap<String, String> =
            serde_json::from_str(raw[0]).expect("valid English UI catalog");
        raw.iter()
            .map(|raw| {
                let exact: BTreeMap<String, String> =
                    serde_json::from_str(raw).expect("valid bundled UI catalog");
                let mut phrases: BTreeMap<char, Vec<(String, String)>> = BTreeMap::new();
                let mut templates = Vec::new();
                for (source, target) in &exact {
                    assert!(!source.is_empty() && !target.is_empty());
                    if let Some(template) = Template::parse(source, target) {
                        templates.push(template);
                    } else {
                        phrases
                            .entry(source.chars().next().unwrap())
                            .or_default()
                            .push((source.clone(), target.clone()));
                    }
                }
                for entries in phrases.values_mut() {
                    entries.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then(a.0.cmp(&b.0)));
                }
                templates.sort_by_key(|t| std::cmp::Reverse(t.literal_bytes));
                assert!(
                    exact.keys().eq(english.keys()),
                    "all UI languages must cover the same source keys"
                );
                Dictionary {
                    exact,
                    phrases,
                    templates,
                }
            })
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_shared_language_codes_roundtrip_and_have_matching_catalogs() {
        for locale in Locale::ALL {
            assert_eq!(Locale::parse(locale.code()).unwrap(), locale);
            assert_eq!(
                serde_json::from_str::<Locale>(&serde_json::to_string(&locale).unwrap()).unwrap(),
                locale
            );
            if locale != Locale::Russian {
                assert!(!locale.translate("Язык интерфейса").is_empty());
                assert!(
                    !locale
                        .translate("Проверка при открытии и перед игрой")
                        .contains("Проверка")
                );
            }
        }
        assert_eq!(Locale::parse("zh-CN").unwrap(), Locale::ChineseSimplified);
        assert_eq!(Locale::parse("zh-TW").unwrap(), Locale::ChineseTraditional);
        assert_ne!(
            Locale::ChineseSimplified.translate("НАСТРОЙКИ"),
            Locale::ChineseTraditional.translate("НАСТРОЙКИ")
        );
    }
    #[test]
    fn templates_keep_numbers_paths_and_multiline_boundaries() {
        for locale in Locale::ALL {
            let source = "Версия игры: 0.1.0-test.20261103.3\nBDA: 11/14 патронов";
            let value = locale.translate(source);
            assert!(
                value.contains("0.1.0-test.20261103.3") && value.contains("11/14"),
                "{locale:?}: {value}"
            );
            assert_eq!(value.matches('\n').count(), 1);
            assert!(
                locale
                    .translate("Ошибка: /tmp/нет/{0}/game.smf")
                    .contains("/tmp/нет/{0}/game.smf")
            );
        }
        let value = Locale::English.translate("10.0 МБ / 20.0 МБ  ·  1.0 МБ/с  ·  50%");
        assert!(!value.contains("МБ"), "{value}");
    }
    #[test]
    fn cjk_fonts_are_bundled_complete_true_type_fonts() {
        for locale in [
            Locale::ChineseSimplified,
            Locale::ChineseTraditional,
            Locale::Japanese,
            Locale::Korean,
        ] {
            let bytes = locale.cjk_font().unwrap();
            assert!(bytes.len() > 1_000_000);
            assert!(bytes.starts_with(&[0, 1, 0, 0]));
        }
    }
}
