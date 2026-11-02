//! Локализация существующих текстовых виджетов прототипа.
//! Исходный текст сохраняется на сущности: смена языка не перезапускает сцену,
//! не меняет сообщения симуляции и не пересоздаёт открытые окна.
use bevy::prelude::*;
use std::{collections::BTreeMap, sync::OnceLock};

#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Language {
    #[default]
    Russian,
    English,
}
impl Language {
    pub fn from_english(english: bool) -> Self {
        if english {
            Self::English
        } else {
            Self::Russian
        }
    }
    pub fn parse(value: &str) -> Result<Self, &'static str> {
        match value {
            "ru" => Ok(Self::from_english(false)),
            "en" => Ok(Self::from_english(true)),
            _ => Err("language must be ru or en"),
        }
    }
}

pub(crate) struct LocalizationPlugin;
impl Plugin for LocalizationPlugin {
    fn build(&self, app: &mut App) {
        // Проверяем встроенный словарь до первого игрового кадра.
        catalog();
        app.init_resource::<Language>()
            .add_systems(PostUpdate, localize.before(bevy::ui::UiSystems::Prepare));
    }
}

#[derive(Component)]
pub(crate) struct TranslatedText {
    pub source: String,
    rendered: String,
    language: Language,
}

struct Catalog(BTreeMap<char, Vec<(String, String)>>);
fn catalog() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        let phrases: BTreeMap<String, String> =
            serde_json::from_str(include_str!("../assets/locales/en.json"))
                .expect("valid bundled English UI catalog");
        let mut index: BTreeMap<char, Vec<(String, String)>> = BTreeMap::new();
        for (source, target) in phrases {
            let first = source.chars().next().expect("nonempty translation key");
            assert!(!target.is_empty(), "nonempty translation value");
            index.entry(first).or_default().push((source, target));
        }
        for phrases in index.values_mut() {
            phrases.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then(a.0.cmp(&b.0)));
        }
        Catalog(index)
    })
}
pub(crate) fn has_cyrillic(text: &str) -> bool {
    text.chars().any(|c| ('\u{0400}'..='\u{052f}').contains(&c))
}
fn word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}
fn boundary(text: &str, offset: usize, source: &str) -> bool {
    (!source.starts_with(word) || offset == 0 || !text[..offset].ends_with(word))
        && (!source.ends_with(word) || !text[offset + source.len()..].starts_with(word))
}

/// Технические подробности остаются в журнале; уведомление в окне — на языке UI.
fn failure(text: &str) -> Option<&'static str> {
    for (prefix, summary) in [
        (
            "Настройки не прочитаны:",
            "Could not read settings. Defaults are in use. See the log for details.",
        ),
        (
            "Не удалось сохранить настройки:",
            "Could not save settings. Check write access to the settings folder.",
        ),
        (
            "Не удалось загрузить обучение:",
            "Could not load training. Check the original game files. See the log for details.",
        ),
        (
            "Ролик не воспроизведён:",
            "Could not play the movie. Check the movie file. See the log for details.",
        ),
    ] {
        if text.starts_with(prefix) {
            return Some(summary);
        }
    }
    None
}

/// Сначала применяются полные фразы, затем части динамических подписей.
/// Числа, имена ресурсов и неизвестные тексты сохраняются; границы слов
/// исключают подмену части другого слова (например, «нет» внутри «кабинет»).
pub(crate) fn english(text: &str) -> String {
    if let Some(summary) = failure(text) {
        return summary.into();
    }
    if !has_cyrillic(text) {
        return text.into();
    }
    let mut result = String::with_capacity(text.len());
    let mut offset = 0;
    while offset < text.len() {
        let c = text[offset..].chars().next().unwrap();
        let found = catalog().0.get(&c).and_then(|phrases| {
            phrases.iter().find(|(source, _)| {
                text[offset..].starts_with(source) && boundary(text, offset, source)
            })
        });
        if let Some((source, target)) = found {
            result.push_str(target);
            offset += source.len();
        } else {
            result.push(c);
            offset += c.len_utf8();
        }
    }
    result
}

fn localize(
    mut commands: Commands,
    language: Res<Language>,
    mut texts: Query<(Entity, &mut Text, Option<&mut TranslatedText>)>,
) {
    for (entity, mut text, cache) in &mut texts {
        if let Some(mut cache) = cache {
            let changed = text.0 != cache.rendered && text.0 != cache.source;
            if changed {
                cache.source.clone_from(&text.0);
            }
            if changed || cache.language != *language {
                if *language == Language::English && failure(&cache.source).is_some() {
                    warn!("{}", cache.source);
                }
                cache.rendered = if *language == Language::English {
                    english(&cache.source)
                } else {
                    cache.source.clone()
                };
                cache.language = *language;
            }
            // Существующие UI-системы могут записывать ту же исходную фразу
            // каждый кадр. В этом случае повторно используем готовый перевод.
            if text.0 != cache.rendered {
                text.0.clone_from(&cache.rendered);
            }
        } else {
            let source = text.0.clone();
            if *language == Language::English && failure(&source).is_some() {
                warn!("{source}");
            }
            let rendered = if *language == Language::English {
                english(&source)
            } else {
                source.clone()
            };
            if text.0 != rendered {
                text.0.clone_from(&rendered);
            }
            commands.entity(entity).insert(TranslatedText {
                source,
                rendered,
                language: *language,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_numbers_controls_and_resource_names_in_dynamic_labels() {
        assert_eq!(
            english("Джон · здоровье 230/300 · ЛКМ: лечить"),
            "John · health 230/300 · LMB: treat"
        );
        assert_eq!(
            english("BDA: 11/14 патронов · запасных: 0\nПеренос: Кусачки → основной слот"),
            "BDA: 11/14 rounds · spare: 0\nDragging: Wire cutters → primary slot"
        );
        assert_eq!(
            english("Hummer · 23.5 км/ч\nВыбери свободную точку внутри круга"),
            "Hummer · 23.5 km/h\nChoose a clear point inside the circle"
        );
        assert_eq!(
            english("нет кабинет Ножницы motions/bipedal_stand.bmf"),
            "none кабинет Ножницы motions/bipedal_stand.bmf"
        );
    }
    #[test]
    fn technical_error_details_have_an_english_summary() {
        for text in [
            "Настройки не прочитаны: неверный язык",
            "Не удалось сохранить настройки: доступ запрещён",
            "Не удалось загрузить обучение: нет модели",
            "Ролик не воспроизведён: неверный AVI",
        ] {
            assert!(!has_cyrillic(&english(text)), "{text}");
        }
    }
    #[test]
    fn composed_diagnostics_and_catalog_values_are_english() {
        let source = "Оружие: Нож\nСтойка: В приседе · S/C/P\n1.00 / 2.00 с · ПАУЗА\nПозиция: 1.00, 2.00, 3.00 м\nВыбор: нет\nОхрана: 2/6 · Нет линии огня · гранат: 3";
        assert!(!has_cyrillic(&english(source)), "{}", english(source));
        for entries in catalog().0.values() {
            for (_, value) in entries {
                assert!(!has_cyrillic(value), "{value}");
            }
        }
    }
    #[test]
    fn live_widgets_restore_the_source_and_accept_upstream_updates() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, LocalizationPlugin));
        let static_label = app.world_mut().spawn(Text::new("Джон")).id();
        let dynamic = app
            .world_mut()
            .spawn(Text::new("Дополнительное: осколочная граната ×3"))
            .id();
        app.update();
        app.insert_resource(Language::English);
        app.update();
        assert_eq!(app.world().get::<Text>(static_label).unwrap().0, "John");
        assert_eq!(
            app.world().get::<Text>(dynamic).unwrap().0,
            "Secondary: frag grenade ×3"
        );
        app.world_mut().get_mut::<Text>(dynamic).unwrap().0 =
            "Дополнительное: осколочная граната ×3".into();
        app.update();
        assert_eq!(
            app.world().get::<Text>(dynamic).unwrap().0,
            "Secondary: frag grenade ×3"
        );
        app.world_mut().get_mut::<Text>(dynamic).unwrap().0 =
            "Дополнительное: осколочная граната ×2".into();
        app.update();
        assert_eq!(
            app.world().get::<Text>(dynamic).unwrap().0,
            "Secondary: frag grenade ×2"
        );
        app.insert_resource(Language::Russian);
        app.update();
        assert_eq!(app.world().get::<Text>(static_label).unwrap().0, "Джон");
        assert_eq!(
            app.world().get::<Text>(dynamic).unwrap().0,
            "Дополнительное: осколочная граната ×2"
        );
    }
}
