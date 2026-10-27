//! Нативная проверка перевода: открытые виджеты, изменение данных и черновик языка.
use super::*;
use crate::localization::{Language, TranslatedText, has_cyrillic};

#[derive(Resource)]
pub(super) struct Smoke {
    path: PathBuf,
    stage: u8,
    frames: u32,
    changed: u32,
    inventory_title: Option<Entity>,
    player_root: Option<Entity>,
    checks: Vec<String>,
    hint_index: usize,
}
impl Smoke {
    pub(super) fn new(path: PathBuf) -> Self {
        Self {
            path,
            stage: 0,
            frames: 0,
            changed: 0,
            inventory_title: None,
            player_root: None,
            checks: Vec::new(),
            hint_index: 0,
        }
    }
    fn advance(&mut self) {
        self.stage += 1;
        self.changed = self.frames;
    }
    fn snapshot(&self, world: &mut World, name: &str) {
        let path = self.path.parent().unwrap().join(name);
        world
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path));
    }
}
fn action(world: &mut World, action: Action) {
    world
        .resource_mut::<Messages<ActionRequest>>()
        .write(ActionRequest(action));
}
fn text_entity(world: &mut World, contains: &str) -> Option<Entity> {
    world
        .query::<(Entity, &Text)>()
        .iter(world)
        .find_map(|(e, t)| t.0.contains(contains).then_some(e))
}
fn audit(world: &mut World, smoke: &mut Smoke, name: &str) {
    assert_eq!(*world.resource::<Language>(), Language::English);
    let untranslated = world
        .query::<&Text>()
        .iter(world)
        .filter(|t| has_cyrillic(&t.0))
        .map(|t| t.0.clone())
        .collect::<Vec<_>>();
    assert!(
        untranslated.is_empty(),
        "{name}: untranslated UI: {untranslated:#?}"
    );
    smoke.checks.push(name.into());
}
fn same_inventory(world: &World, smoke: &Smoke, word: &str) {
    assert!(
        world
            .get::<Text>(smoke.inventory_title.unwrap())
            .unwrap()
            .0
            .contains(word)
    );
    assert!(
        world
            .get::<player::Root>(smoke.player_root.unwrap())
            .is_some(),
        "scene was recreated"
    );
}

pub(super) fn run(world: &mut World) {
    let mut smoke = world.remove_resource::<Smoke>().unwrap();
    smoke.frames += 1;
    assert!(
        smoke.frames < 6000,
        "localization smoke timeout at stage {}",
        smoke.stage
    );
    let state = *world.resource::<State<Screen>>().get();
    let ready = smoke.frames - smoke.changed >= 20;
    match smoke.stage {
        0 if ready && state == Screen::Main => {
            audit(world, &mut smoke, "english-main-menu");
            action(world, Action::Training);
            smoke.advance();
        }
        1 if ready && state == Screen::Training && text_entity(world, "Training ·").is_some() => {
            let hints = [
                "help_combat_4",
                "help_combat_6",
                "help_combat_8",
                "help_combat_10",
            ];
            if smoke.hint_index == 0 {
                smoke.player_root = world
                    .query_filtered::<Entity, With<player::Root>>()
                    .iter(world)
                    .next();
                smoke.snapshot(world, "locale-en-training.png");
                audit(
                    world,
                    &mut smoke,
                    "english-tutorial-hud-and-hidden-diagnostics",
                );
            } else {
                let name = format!("english-engine-help-{}", hints[smoke.hint_index - 1]);
                audit(world, &mut smoke, &name);
                if smoke.hint_index == 1 {
                    smoke.snapshot(world, "locale-en-equipment-help.png");
                }
            }
            if let Some(id) = hints.get(smoke.hint_index) {
                world.resource_mut::<crate::tutorial::Tutorial>().active = Some((*id).into());
                smoke.hint_index += 1;
                smoke.changed = smoke.frames;
                world.insert_resource(smoke);
                return;
            }
            // Состояния предметов задаются для проверки всех строк инвентаря;
            // эта проверка не моделирует их получение игровыми приказами.
            world.resource_mut::<crate::tutorial::Tutorial>().close();
            let mut inventory = world.resource_mut::<crate::inventory::Inventory>();
            inventory.open = true;
            inventory.collected = true;
            inventory.owns_cutters = true;
            inventory.rounds = 11;
            let mut grenade = world.resource_mut::<crate::grenade::Grenade>();
            grenade.collected = true;
            grenade.secondary = true;
            grenade.count = 3;
            let mut medical = world.resource_mut::<crate::medical::Medical>();
            medical.ground.collected = true;
            medical.health = 230.;
            medical.uses = 4;
            medical.armed = true;
            let mut armor = world.resource_mut::<crate::armor::Armor>();
            armor.ground.collected = true;
            armor.worn = true;
            smoke.advance();
        }
        2 if ready
            && state == Screen::Training
            && text_entity(world, "JOHN EMERSON · Inventory").is_some() =>
        {
            audit(
                world,
                &mut smoke,
                "english-seeded-inventory-medical-armor-and-grenade-labels",
            );
            smoke.inventory_title = text_entity(world, "JOHN EMERSON · Inventory");
            assert!(text_entity(world, "John · health 230/").is_some());
            assert!(text_entity(world, "Body: flak jacket").is_some());
            smoke.snapshot(world, "locale-en-inventory.png");
            action(world, Action::Pause);
            smoke.advance();
        }
        3 if ready && state == Screen::Pause => {
            action(world, Action::Options);
            smoke.advance();
        }
        4 if ready && state == Screen::Options => {
            action(world, Action::Language);
            smoke.advance();
        }
        5 if ready && *world.resource::<Language>() == Language::Russian => {
            same_inventory(world, &smoke, "Инвентарь");
            assert!(world.resource::<Context>().saved.english);
            smoke
                .checks
                .push("draft-language-updates-existing-widget-without-recreating-scene".into());
            action(world, Action::Back);
            smoke.advance();
        }
        6 if ready && state == Screen::Pause => {
            audit(world, &mut smoke, "cancel-restores-english");
            same_inventory(world, &smoke, "Inventory");
            action(world, Action::Options);
            smoke.advance();
        }
        7 if ready && state == Screen::Options => {
            action(world, Action::Language);
            action(world, Action::Apply);
            smoke.advance();
        }
        8 if ready && !world.resource::<Context>().saved.english => {
            action(world, Action::Language);
            smoke.stage = 21;
            smoke.changed = smoke.frames;
        }
        21 if ready && *world.resource::<Language>() == Language::English => {
            audit(
                world,
                &mut smoke,
                "saved-notice-follows-english-draft-without-reapplying",
            );
            assert!(text_entity(world, "Settings saved").is_some());
            action(world, Action::Language);
            smoke.stage = 22;
            smoke.changed = smoke.frames;
        }
        22 if ready && *world.resource::<Language>() == Language::Russian => {
            assert!(text_entity(world, "Настройки сохранены").is_some());
            action(world, Action::Back);
            smoke.stage = 9;
            smoke.changed = smoke.frames;
        }
        9 if ready && state == Screen::Pause => {
            action(world, Action::Resume);
            smoke.advance();
        }
        10 if ready && state == Screen::Training => {
            same_inventory(world, &smoke, "Инвентарь");
            smoke.snapshot(world, "locale-ru-inventory.png");
            smoke
                .checks
                .push("applied-russian-persists-and-restores-inventory-source".into());
            action(world, Action::Pause);
            smoke.advance();
        }
        11 if ready && state == Screen::Pause => {
            action(world, Action::Options);
            smoke.advance();
        }
        12 if ready && state == Screen::Options => {
            action(world, Action::Language);
            action(world, Action::Apply);
            smoke.advance();
        }
        13 if ready && world.resource::<Context>().saved.english => {
            action(world, Action::Back);
            smoke.advance();
        }
        14 if ready && state == Screen::Pause => {
            action(world, Action::Resume);
            smoke.advance();
        }
        15 if ready && state == Screen::Training => {
            audit(world, &mut smoke, "applied-english-restores-live-inventory");
            same_inventory(world, &smoke, "Inventory");
            world.resource_mut::<crate::inventory::Inventory>().rounds = 7;
            world.resource_mut::<crate::medical::Medical>().health = 225.;
            smoke.advance();
        }
        16 if ready && state == Screen::Training => {
            audit(
                world,
                &mut smoke,
                "dynamic-health-and-ammo-values-update-in-english",
            );
            assert!(text_entity(world, "John · health 225/").is_some());
            assert!(text_entity(world, "BDA: 7/").is_some());
            assert!(
                Preferences::load(&world.resource::<Context>().settings_path)
                    .unwrap()
                    .english
            );
            action(world, Action::Pause);
            smoke.advance();
        }
        17 if ready && state == Screen::Pause => {
            action(world, Action::Main);
            smoke.advance();
        }
        18 if ready && state == Screen::Main => {
            assert!(
                world
                    .query::<&TranslatedText>()
                    .iter(world)
                    .all(|t| !t.source.starts_with("JOHN EMERSON ·"))
            );
            audit(world, &mut smoke, "training-widget-caches-are-cleaned-up");
            world.resource_mut::<Context>().notice =
                "Не удалось загрузить обучение: проверка сообщения".into();
            smoke.advance();
        }
        19 if ready && state == Screen::Main => {
            audit(world, &mut smoke, "english-error-summary");
            assert!(text_entity(world, "Could not load training.").is_some());
            smoke.snapshot(world, "locale-en-main.png");
            smoke.advance();
        }
        20 if ready => {
            let report = serde_json::json!({"status":"passed","checks":smoke.checks,"frames":smoke.frames,"inventory_states_seeded":true,"help_states_seeded":true,"physical_pointer_tested":false,"scene_preserved_across_language_changes":true});
            std::fs::write(&smoke.path, serde_json::to_string_pretty(&report).unwrap()).unwrap();
            world
                .resource_mut::<Messages<AppExit>>()
                .write(AppExit::Success);
        }
        _ => {}
    }
    world.insert_resource(smoke);
}
