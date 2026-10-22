//! Flak Jacket: предмет MTF, свободный слот тела и смена исходной текстуры Джона.
use crate::{
    inventory::{self, Inventory, Slot},
    map_scene::MapScene,
    pickup::{MapObject, Pickup},
    player,
    smg::Uzi,
    tutorial::Tutorial,
    ui_controls::ButtonVisualState,
};
use bevy::prelude::*;
use shadow_formats::{
    AssetCatalog, CampaignEventType as Event, ConfigLine, SmfModel, config_lines,
};
use std::error::Error;

fn outfit_lines(bytes: &[u8]) -> Result<Vec<ConfigLine>, Box<dyn Error>> {
    // В оригинальном Uniform одна описательная строка имеет незакрытую кавычку.
    // Описания здесь не используются; игровые поля остаются под строгим парсером.
    let mut filtered = Vec::with_capacity(bytes.len());
    for line in bytes.split_inclusive(|b| *b == b'\n') {
        let description = line
            .trim_ascii_start()
            .strip_prefix(b"OBJECT_DESC_LINE")
            .is_some_and(|tail| tail.first().is_some_and(u8::is_ascii_whitespace));
        if description {
            filtered.push(b'\n');
        } else {
            filtered.extend_from_slice(line);
        }
    }
    Ok(config_lines(&filtered)?)
}

#[derive(Debug)]
pub struct Protection {
    pub blast: f32,
    pub light: [f32; 2],
    pub heavy: [f32; 2],
    pub melee: [f32; 2],
}
impl Protection {
    fn parse(lines: &[ConfigLine]) -> Result<Self, Box<dyn Error>> {
        let scalar = |key: &str, positive: bool| -> Result<f32, Box<dyn Error>> {
            let n = inventory::field(lines, key)?.parse::<f32>()?;
            if !n.is_finite() || !(0.0..=100000.0).contains(&n) || (positive && n == 0.0) {
                return Err(format!("неверный параметр брони {key}").into());
            }
            Ok(n)
        };
        let pair = |name: &str| -> Result<[f32; 2], Box<dyn Error>> {
            Ok([
                scalar(&format!("ARMOR_{name}_ARMS_DR"), false)?,
                scalar(&format!("ARMOR_{name}_ARMS_DD"), true)?,
            ])
        };
        Ok(Self {
            blast: scalar("ARMOR_BLAST_MULT", false)?,
            light: pair("LIGHT")?,
            heavy: pair("HEAVY")?,
            melee: pair("MELEE")?,
        })
    }
    pub fn report(&self) -> serde_json::Value {
        serde_json::json!({"blast_multiplier":self.blast,"light_arms_dr_dd":self.light,"heavy_arms_dr_dd":self.heavy,"melee_dr_dd":self.melee,
            "incoming_damage_connected":false,"original_damage_formula_verified":false})
    }
}
#[derive(Resource)]
pub struct Armor {
    pub ground: Uzi,
    pub worn: bool,
    pub uniform_path: String,
    pub flak_path: String,
    pub body_parts: usize,
    pub protection: Protection,
    pub source: serde_json::Value,
    pub(crate) test_cursor: Option<Vec2>,
    textures: Option<[(Handle<Image>, AlphaMode); 2]>,
}
impl Armor {
    pub fn prepare(
        catalog: &AssetCatalog,
        scene: &MapScene,
        p: &mut player::Player,
    ) -> Result<Self, Box<dyn Error>> {
        let path = "config/inventory_object_profiles/outfits.txt";
        let lines = outfit_lines(&catalog.read(path)?)?;
        let item = inventory::block(&lines, "OBJECT_NAME", "Flak Jacket", "OBJECT_DEF")?;
        if inventory::field(item, "OBJECT_EQUIP_LOC")? != "EQL_BODY"
            || inventory::field(item, "OBJECT_CARRY_STYLE")? != "CSTYLE_ONE_HAND"
        {
            return Err("неподдержанный слот бронежилета".into());
        }
        let auto_equip = match inventory::field(item, "OBJECT_AUTO_EQUIP")?
            .to_ascii_uppercase()
            .as_str()
        {
            "TRUE" => true,
            "FALSE" => false,
            _ => return Err("неверный OBJECT_AUTO_EQUIP бронежилета".into()),
        };
        let protection = Protection::parse(item)?;
        let model = player::model_path(catalog, inventory::field(item, "OBJECT_MODEL")?)?;
        let ground = Uzi::ground_item(scene, "Flak Jacket", &model, auto_equip)?;
        let body_type = inventory::field(item, "BODY_TYPE")?;
        let (profile_path, profile) = player::profile_for(catalog, &p.name)?;
        let initial = profile.initial_body();
        let body = profile
            .bodies
            .iter()
            .find(|b| b.kind.eq_ignore_ascii_case(body_type))
            .ok_or("нет BODY_DEFINITION для бронежилета")?;
        if !initial.kind.eq_ignore_ascii_case("BT_UNIFORM")
            || !body.extra.is_empty()
            || body.body_model != initial.body_model
            || body.head_model != initial.head_model
            || body.head_map != initial.head_map
            || body.pack != initial.pack
        {
            return Err("изменённая геометрия одежды пока не поддерживается; исходный Flak Jacket меняет BODY_MAP".into());
        }
        let body_model = player::model_path(catalog, &initial.body_model)?;
        let uniform_path = catalog
            .resolve_texture(&body_model, &initial.body_map)?
            .ok_or("нет текстуры формы")?;
        let flak_path = catalog
            .resolve_texture(&body_model, &body.body_map)?
            .ok_or("нет текстуры бронежилета")?;
        // prepare_appearance добавляет тело перед головой, рюкзаком и оружием.
        let mesh = SmfModel::from_bytes(&catalog.read(&body_model)?)?;
        let body_parts = mesh
            .nodes
            .iter()
            .flat_map(|n| &n.meshes)
            .filter(|m| !m.faces.is_empty())
            .count();
        let rig = p.rig.as_mut().ok_or("броня готовится до создания сцены")?;
        if body_parts == 0
            || body_parts > rig.parts.len()
            || !rig.parts[..body_parts]
                .iter()
                .all(|part| part.texture == uniform_path)
        {
            return Err("части тела не совпали с исходной моделью".into());
        }
        if !rig.images.contains_key(&flak_path) {
            rig.images.insert(
                flak_path.clone(),
                crate::asset_mesh::image(catalog, &flak_path)?,
            );
        }
        let source = serde_json::json!({"profile":path,"profile_origin":catalog.source_label(path)?,"model":model,"equip_location":"EQL_BODY","auto_equip_flag":auto_equip,
            "character_profile":profile_path,"initial_body":initial.kind,"worn_body":body.kind,"body_model":body_model,
            "uniform_texture":uniform_path,"flak_texture":flak_path,"uniform_origin":catalog.source_label(&uniform_path)?,"flak_origin":catalog.source_label(&flak_path)?,
            "body_material_count":body_parts,"mesh_replacement":false,"head_pack_weapon_changed":false,
            "pickup_equip_evidence":"help_combat_15: Flak Jacket went directly into your body slot",
            "pickup_equip_rule":"first body item fills the empty body slot; BODY_INITIAL is the default appearance",
            "empty_body_slot_rule_original_code_verified":false,"damage_parameters":protection.report()});
        Ok(Self {
            ground,
            worn: false,
            uniform_path,
            flak_path,
            body_parts,
            protection,
            source,
            test_cursor: None,
            textures: None,
        })
    }
    pub fn bind_textures(
        &mut self,
        uniform: (Handle<Image>, AlphaMode),
        flak: (Handle<Image>, AlphaMode),
    ) {
        self.textures = Some([uniform, flak]);
    }
    // Используется проверкой выгрузки сцены в shadow-game, но не просмотрщиком карты.
    #[allow(dead_code)]
    pub fn image_handles(&self) -> impl Iterator<Item = &Handle<Image>> {
        self.textures
            .iter()
            .flat_map(|pair| pair.iter().map(|(handle, _)| handle))
    }
    pub fn toggle(&mut self) -> bool {
        if !self.ground.collected {
            return false;
        }
        self.worn = !self.worn;
        true
    }
    pub fn restart(&mut self, p: &mut player::Player) {
        self.ground.restart(p);
        self.worn = false;
    }
    pub fn update(
        &mut self,
        p: &mut player::Player,
        inv: &Inventory,
        t: &mut Tutorial,
        callback: f32,
    ) {
        if !self.ground.ordered
            || inv.open
            || p.playback.paused
            || p.changing_posture()
            || t.pausing()
        {
            return;
        }
        let d = Vec2::new(
            p.position.x - self.ground.position.x,
            p.position.z - self.ground.position.z,
        )
        .length();
        if !self.ground.collected && p.pickup_seconds.is_none() && d <= 0.5 {
            p.destination = None;
            p.begin_pickup();
        }
        if !self.ground.collected
            && p.pickup_seconds.is_none()
            && p.destination.is_none()
            && d > 0.5
        {
            self.ground.cancel(p);
            return;
        }
        if let Some(seconds) = p.pickup_seconds {
            if !self.ground.collected && seconds >= p.pickup_callback(callback) && d <= 0.5 {
                self.ground.collected = true;
                self.worn = true;
                t.fire(Event::ObjectPickedUp, "Flak Jacket");
                t.tick(0.0, false);
            }
            if seconds >= p.pickup_duration() {
                p.finish_pickup();
                self.ground.ordered = false;
            }
        }
    }
    pub fn report(&self) -> serde_json::Value {
        serde_json::json!({"pickup":self.ground.report(),"worn":self.worn,"body_slot":if self.worn {"Flak Jacket"} else {"empty; default uniform"},
            "current_body_texture":if self.worn {&self.flak_path} else {&self.uniform_path},"protection":self.protection.report(),"source":self.source})
    }
}
#[derive(Component)]
pub struct BodySurface;
#[derive(Component)]
pub struct Ring;
pub fn setup(
    mut commands: Commands,
    a: Option<Res<Armor>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let Some(a) = a else {
        return;
    };
    commands.spawn((
        player::SceneEntity,
        Ring,
        Mesh3d(meshes.add(Torus::new(0.22, 0.25))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.35, 0.85, 1.0),
            unlit: true,
            ..default()
        })),
        Transform::from_translation(a.ground.position + Vec3::Y * 0.04),
        Visibility::default(),
    ));
}
#[allow(clippy::too_many_arguments)]
pub fn input(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    a: Option<ResMut<Armor>>,
    p: Option<ResMut<player::Player>>,
    inv: Option<ResMut<Inventory>>,
    t: Option<Res<Tutorial>>,
    slots: Query<(&Slot, &ButtonVisualState)>,
    medical: Option<ResMut<crate::medical::Medical>>,
) {
    let (Some(mut a), Some(mut p), Some(mut inv)) = (a, p, inv) else {
        return;
    };
    if t.is_some_and(|t| t.active.is_some() || t.consumed_input) {
        return;
    }
    if !inv.open {
        if keys.just_pressed(KeyCode::Home) {
            a.restart(&mut p);
        } else if keys.just_pressed(KeyCode::KeyH) && p.selected {
            a.ground.cancel(&mut p);
        }
        return;
    }
    if mouse.just_pressed(MouseButton::Left)
        && slots
            .iter()
            .any(|(s, v)| *s == Slot::BodyArmor && *v == ButtonVisualState::Pressed)
        && a.toggle()
    {
        inv.consumed_input = true;
        inv.drag = None;
        if let Some(mut m) = medical {
            m.cancel(&mut p);
        }
    }
}
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn sync(
    a: Option<ResMut<Armor>>,
    p: Option<ResMut<player::Player>>,
    inv: Option<Res<Inventory>>,
    t: Option<ResMut<Tutorial>>,
    item: Option<Res<Pickup>>,
    mut objects: Query<(&MapObject, &mut Visibility)>,
    mut rings: Query<&mut Visibility, (With<Ring>, Without<MapObject>)>,
    body: Query<&MeshMaterial3d<StandardMaterial>, With<BodySurface>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let Some(mut a) = a else {
        return;
    };
    if let (Some(mut p), Some(inv), Some(mut t), Some(item)) = (p, inv, t, item) {
        a.update(&mut p, &inv, &mut t, item.callback_seconds);
    }
    for (id, mut v) in &mut objects {
        if id.0 == a.ground.id {
            *v = if a.ground.collected {
                Visibility::Hidden
            } else {
                Visibility::Inherited
            };
        }
    }
    for mut v in &mut rings {
        *v = if a.ground.collected {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
    }
    if let Some(textures) = &a.textures {
        let (image, alpha) = &textures[usize::from(a.worn)];
        for handle in &body {
            if let Some(mut material) = materials.get_mut(&handle.0)
                && (material.base_color_texture.as_ref() != Some(image)
                    || material.alpha_mode != *alpha)
            {
                material.base_color_texture = Some(image.clone());
                material.alpha_mode = *alpha;
            }
        }
    }
}
#[derive(Resource)]
pub struct NativeSmoke {
    path: std::path::PathBuf,
    screenshot: Option<std::path::PathBuf>,
    stage: u32,
    frames: u32,
    checks: Vec<serde_json::Value>,
    last_stage: Option<u32>,
}
impl NativeSmoke {
    pub fn new(path: std::path::PathBuf, screenshot: Option<std::path::PathBuf>) -> Self {
        Self {
            path,
            screenshot,
            stage: 0,
            frames: 0,
            checks: Vec::new(),
            last_stage: None,
        }
    }
}
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn native_smoke(
    mut commands: Commands,
    s: Option<ResMut<NativeSmoke>>,
    a: Option<ResMut<Armor>>,
    p: Option<ResMut<player::Player>>,
    inv: Option<ResMut<Inventory>>,
    t: Option<Res<Tutorial>>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    camera: Single<(&Camera, &GlobalTransform), With<crate::training::MapCamera>>,
    mut orbit: ResMut<crate::training::Orbit>,
    mut slots: Query<(&Slot, &mut ButtonVisualState)>,
    window: Single<&Window>,
    world: (Option<Res<Pickup>>, Option<Res<crate::medical::Medical>>),
    render: (
        Query<&MeshMaterial3d<StandardMaterial>, With<BodySurface>>,
        Res<Assets<StandardMaterial>>,
    ),
    objects: Query<(&MapObject, &Visibility)>,
    mut exit: MessageWriter<AppExit>,
) {
    let (Some(mut s), Some(mut a), Some(mut p), Some(inv), Some(t), (Some(geometry), Some(m))) =
        (s, a, p, inv, t, world)
    else {
        return;
    };
    s.frames += 1;
    if s.last_stage != Some(s.stage) {
        eprintln!(
            "armor smoke stage {}: collected={} worn={} ordered={} inventory={}",
            s.stage, a.ground.collected, a.worn, a.ground.ordered, inv.open
        );
        s.last_stage = Some(s.stage);
    }
    assert!(
        s.frames < 1800,
        "armor native smoke timeout stage {}",
        s.stage
    );
    keys.reset_all();
    mouse.reset_all();
    if s.frames < 20 {
        return;
    }
    if t.active.is_some() {
        keys.press(KeyCode::Enter);
        return;
    }
    let (camera, transform) = *camera;
    let (body, materials) = render;
    let material_matches = |worn: bool| {
        let expected = &a.textures.as_ref().expect("body textures bound")[usize::from(worn)].0;
        body.iter().count() == a.body_parts
            && body.iter().all(|h| {
                materials
                    .get(&h.0)
                    .is_some_and(|mat| mat.base_color_texture.as_ref() == Some(expected))
            })
    };
    let body_click = |slots: &mut Query<(&Slot, &mut ButtonVisualState)>,
                      mouse: &mut ButtonInput<MouseButton>| {
        for (slot, mut visual) in slots {
            *visual = if *slot == Slot::BodyArmor {
                ButtonVisualState::Pressed
            } else {
                ButtonVisualState::None
            };
        }
        mouse.press(MouseButton::Left);
    };
    match s.stage {
        0 => {
            p.playback.paused = false;
            p.follow_camera = false;
            orbit.target = a.ground.center;
            keys.press(KeyCode::KeyS);
            s.stage = 1;
        }
        1 if !p.changing_posture() => {
            assert!(material_matches(false));
            let Ok(cursor) = camera.world_to_viewport(transform, a.ground.center) else {
                return;
            };
            if cursor.x <= 315.0 || !a.ground.clicked(camera, transform, cursor, &geometry) {
                orbit.yaw += 0.12;
                return;
            }
            a.test_cursor = Some(cursor);
            mouse.press(MouseButton::Left);
            s.checks.push(serde_json::json!({"ground_pickup_by_mouse":cursor.to_array(),"uniform_before_pickup":true}));
            s.stage = 2;
        }
        2 if !a.ground.collected && !a.ground.ordered && p.destination.is_none() => {
            s.stage = 1;
        }
        2 if a.ground.collected
            && !a.ground.ordered
            && p.pickup_seconds.is_none()
            && t.report()["shown_help"]
                .as_array()
                .unwrap()
                .iter()
                .any(|h| h == "help_combat_15") =>
        {
            assert!(a.worn && material_matches(true));
            assert_eq!(inv.weapon(&p), Some(inventory::Weapon::Uzi));
            assert!(
                objects
                    .iter()
                    .any(|(id, v)| id.0 == a.ground.id && *v == Visibility::Hidden)
            );
            assert_eq!((m.health, m.uses), (327.0, 1));
            s.checks.push(serde_json::json!({"equipped_on_first_pickup":true,"flak_texture_applied":true,"ground_hidden":true,"body_materials":a.body_parts}));
            p.follow_camera = true;
            orbit.distance = 12.0;
            s.stage = 3;
        }
        3 => {
            s.stage = 4;
        }
        4 => {
            if let Some(path) = &s.screenshot {
                commands
                    .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
                    .observe(bevy::render::view::screenshot::save_to_disk(
                        path.with_file_name(format!(
                            "{}-world.png",
                            path.file_stem().unwrap().to_string_lossy()
                        )),
                    ));
            }
            keys.press(KeyCode::Space);
            s.stage = 5;
        }
        5 if inv.open && slots.iter().any(|(s, _)| *s == Slot::BodyArmor) => {
            body_click(&mut slots, &mut mouse);
            s.stage = 6;
        }
        6 => {
            assert!(!a.worn && material_matches(false));
            assert_eq!(inv.weapon(&p), Some(inventory::Weapon::Uzi));
            s.checks.push(serde_json::json!({"inventory_remove":true,"uniform_restored":true,"primary_preserved":"UZI"}));
            s.stage = 7;
        }
        7 => {
            body_click(&mut slots, &mut mouse);
            s.stage = 8;
        }
        8 => {
            assert!(a.worn && material_matches(true));
            assert_eq!((inv.rounds, inv.uzi_rounds), (9, 14));
            assert_eq!((m.health, m.uses), (327.0, 1));
            assert_eq!(
                t.report()["events"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|e| e.as_str() == Some("CET_OBJECT_PICKED_UP:Flak Jacket"))
                    .count(),
                1
            );
            assert_eq!(
                t.report()["completed_objectives"].as_array().unwrap().len(),
                4
            );
            s.checks.push(serde_json::json!({"inventory_wear_again":true,"flak_restored":true,"ammo_and_health_preserved":true}));
            if let Some(path) = &s.screenshot {
                commands
                    .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
                    .observe(bevy::render::view::screenshot::save_to_disk(path.clone()));
            }
            s.stage = 9;
        }
        9 => {
            let report = serde_json::json!({"status":"passed","window":[window.resolution.width(),window.resolution.height()],"armor":a.report(),"inventory":inv.report(&p),"medical":m.report(),"tutorial":t.report(),"checks":s.checks,"starting_checkpoint":"numeric medical-360","physical_pointer_tested":false,"native_input":"viewport mouse for ground item; project ButtonVisualState and mouse buttons for body slot"});
            std::fs::write(
                &s.path,
                serde_json::to_string_pretty(&report).unwrap() + "\n",
            )
            .unwrap();
            s.stage = 10;
            s.frames = 0;
        }
        10 if s.frames >= 20 => {
            exit.write(AppExit::Success);
        }
        _ => {}
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (
        Armor,
        player::Player,
        Inventory,
        Tutorial,
        shadow_formats::TerrainGrid,
    ) {
        let (ground, p, inv, _, grid) = crate::smg::tests::fixture();
        let mission=shadow_formats::MissionDefinition::from_bytes(b"SPECIFY_MTF training.mtf\nCER_DEFINITION\nCER_FOR_TYPE CET_OBJECT_PICKED_UP\nCER_FOR \"Flak Jacket\"\nCER_BODY_DEFINITION\nCER_BODY_TYPE CER_BODY_SHOW_CONTEXT_HELP\nCER_BODY_IDENTIFIER armor\nCER_BODY_MS_DELAY 1000\nCER_BODY_DEFINITION\nCER_BODY_TYPE CER_BODY_REMOVE_CER\n").unwrap();
        let helps=shadow_formats::HelpDefinitions::from_bytes(b"HELP_DEF\nHELP_ID armor\nHELP_CONTEXT\nHELP_BODY \"Flak Jacket went directly into body slot\"\n").unwrap();
        let a = Armor {
            ground,
            worn: false,
            uniform_path: "uniform.bmp".into(),
            flak_path: "flak.bmp".into(),
            body_parts: 1,
            protection: Protection {
                blast: 0.9,
                light: [10.0, 1.0],
                heavy: [10.0, 1.0],
                melee: [0.0, 1.0],
            },
            source: serde_json::Value::Null,
            test_cursor: None,
            textures: None,
        };
        (a, p, inv, Tutorial::new(mission, helps).unwrap(), grid)
    }
    #[test]
    fn callback_collects_and_fills_body_slot_once_without_changing_primary() {
        let (mut a, mut p, inv, mut t, grid) = fixture();
        let weapon = p.weapon.clone();
        assert!(a.ground.order(&mut p, &grid));
        a.update(&mut p, &inv, &mut t, 0.6);
        p.step(&ButtonInput::default(), 0.59, &grid);
        a.update(&mut p, &inv, &mut t, 0.6);
        assert!(!a.ground.collected && !a.worn);
        p.step(&ButtonInput::default(), 0.02, &grid);
        a.update(&mut p, &inv, &mut t, 0.6);
        for _ in 0..5 {
            a.update(&mut p, &inv, &mut t, 0.6);
        }
        assert!(a.ground.collected && a.worn);
        assert_eq!(p.weapon, weapon);
        assert_eq!(inv.rounds, 9);
        assert_eq!(
            t.report()["events"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|e| e.as_str() == Some("CET_OBJECT_PICKED_UP:Flak Jacket"))
                .count(),
            1
        );
        t.tick(0.999, false);
        assert!(t.active.is_none());
        t.tick(0.002, false);
        assert_eq!(t.active.as_deref(), Some("armor"));
    }
    #[test]
    fn replacing_pending_pickup_keeps_new_movement_and_does_not_collect() {
        let (mut a, mut p, inv, mut t, grid) = fixture();
        a.ground.order(&mut p, &grid);
        a.update(&mut p, &inv, &mut t, 0.6);
        p.step(&ButtonInput::default(), 0.3, &grid);
        let target = Vec3::new(0.5, 0.0, -0.5);
        assert!(p.order(&grid, target, false));
        a.ground.cancel(&mut p);
        a.update(&mut p, &inv, &mut t, 0.6);
        assert!(!a.ground.ordered && !a.ground.collected && !a.worn);
        assert!(p.destination.is_some());
        assert!(p.pickup_seconds.is_none());
    }
    #[test]
    fn inventory_pause_posture_transition_and_context_window_gate_callback() {
        let (mut a, mut p, mut inv, mut t, _) = fixture();
        a.ground.ordered = true;
        p.pickup_seconds = Some(99.0);
        inv.open = true;
        a.update(&mut p, &inv, &mut t, 0.6);
        assert!(!a.ground.collected);
        inv.open = false;
        p.playback.paused = true;
        a.update(&mut p, &inv, &mut t, 0.6);
        assert!(!a.ground.collected);
        p.playback.paused = false;
        p.posture_transition = Some((0, 0.0, crate::stance::Posture::Crouch));
        a.update(&mut p, &inv, &mut t, 0.6);
        assert!(!a.ground.collected);
        p.posture_transition = None;
        t.active = Some("armor".into());
        a.update(&mut p, &inv, &mut t, 0.6);
        assert!(!a.ground.collected);
        t.close();
        a.update(&mut p, &inv, &mut t, 0.6);
        assert!(a.ground.collected && a.worn);
        assert!(p.pickup_seconds.is_none());
    }
    #[test]
    fn only_owned_armor_can_be_toggled_and_primary_ammo_position_remain_unchanged() {
        let (mut a, p, inv, _, _) = fixture();
        let before = inv.report(&p);
        let position = p.position;
        assert!(!a.toggle());
        a.ground.collected = true;
        assert!(a.toggle() && a.worn);
        assert!(a.toggle() && !a.worn);
        assert_eq!(before, inv.report(&p));
        assert_eq!(position, p.position);
    }
    #[test]
    fn home_input_restores_ground_item_and_uniform() {
        let (mut a, p, inv, t, _) = fixture();
        a.ground.collected = true;
        a.worn = true;
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(a)
            .insert_resource(p)
            .insert_resource(inv)
            .insert_resource(t)
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .add_systems(Update, input);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Home);
        app.update();
        let a = app.world().resource::<Armor>();
        assert!(!a.ground.collected && !a.worn && !a.ground.ordered);
    }
    #[test]
    fn body_slot_input_toggles_equipment_and_consumes_world_click() {
        let (mut a, p, mut inv, t, _) = fixture();
        a.ground.collected = true;
        a.worn = true;
        inv.open = true;
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(a)
            .insert_resource(p)
            .insert_resource(inv)
            .insert_resource(t)
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .add_systems(Update, input);
        app.world_mut()
            .spawn((Slot::BodyArmor, ButtonVisualState::Pressed));
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app.update();
        assert!(!app.world().resource::<Armor>().worn);
        assert!(app.world().resource::<Inventory>().consumed_input);
        assert_eq!(
            app.world()
                .resource::<Inventory>()
                .weapon(app.world().resource::<player::Player>()),
            Some(inventory::Weapon::Bda)
        );
    }
    #[test]
    fn renderer_updates_only_body_material_and_restores_alpha_on_removal() {
        let (mut a, _, _, _, _) = fixture();
        let mut images = Assets::<Image>::default();
        let uniform = images.add(Image::default());
        let flak = images.add(Image::default());
        a.bind_textures(
            (uniform.clone(), AlphaMode::Opaque),
            (flak.clone(), AlphaMode::Mask(0.5)),
        );
        a.ground.collected = true;
        a.worn = true;
        let mut materials = Assets::<StandardMaterial>::default();
        let make = || StandardMaterial {
            base_color_texture: Some(uniform.clone()),
            perceptual_roughness: 0.3,
            ..default()
        };
        let body = materials.add(make());
        let head = materials.add(make());
        let weapon = materials.add(make());
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(a)
            .insert_resource(materials)
            .add_systems(Update, sync);
        app.world_mut()
            .spawn((BodySurface, MeshMaterial3d(body.clone())));
        app.world_mut().spawn(MeshMaterial3d(head.clone()));
        app.world_mut().spawn(MeshMaterial3d(weapon.clone()));
        app.update();
        let check = |app: &App, image: &Handle<Image>, alpha: AlphaMode| {
            let materials = app.world().resource::<Assets<StandardMaterial>>();
            assert_eq!(
                materials.get(&body).unwrap().base_color_texture.as_ref(),
                Some(image)
            );
            assert_eq!(materials.get(&body).unwrap().alpha_mode, alpha);
            assert_eq!(materials.get(&body).unwrap().perceptual_roughness, 0.3);
            assert_eq!(
                materials.get(&head).unwrap().base_color_texture.as_ref(),
                Some(&uniform)
            );
            assert_eq!(
                materials.get(&weapon).unwrap().base_color_texture.as_ref(),
                Some(&uniform)
            );
        };
        check(&app, &flak, AlphaMode::Mask(0.5));
        app.world_mut().resource_mut::<Armor>().toggle();
        app.update();
        check(&app, &uniform, AlphaMode::Opaque);
    }
    #[test]
    fn malformed_optional_descriptions_do_not_weaken_game_field_validation() {
        let lines = outfit_lines(
            b"OBJECT_DESC_LINE \"original unterminated description\nARMOR_BLAST_MULT 0.9\n",
        )
        .unwrap();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].number, 2);
        assert_eq!(lines[0].words, ["ARMOR_BLAST_MULT", "0.9"]);
        assert!(outfit_lines(b"ARMOR_BLAST_MULT \"0.9\n").is_err());
    }
    #[test]
    fn invalid_protection_numbers_are_rejected_before_scene_creation() {
        let text = "ARMOR_BLAST_MULT 0.9\nARMOR_LIGHT_ARMS_DR 10\nARMOR_LIGHT_ARMS_DD 1\nARMOR_HEAVY_ARMS_DR 10\nARMOR_HEAVY_ARMS_DD 1\nARMOR_MELEE_ARMS_DR 0\nARMOR_MELEE_ARMS_DD 1\n";
        assert!(Protection::parse(&config_lines(text.as_bytes()).unwrap()).is_ok());
        for (old, new) in [
            ("MULT 0.9", "MULT NaN"),
            ("LIGHT_ARMS_DR 10", "LIGHT_ARMS_DR -1"),
            ("HEAVY_ARMS_DD 1", "HEAVY_ARMS_DD 0"),
            ("MELEE_ARMS_DD 1", "MELEE_ARMS_DD inf"),
        ] {
            assert!(
                Protection::parse(&config_lines(text.replace(old, new).as_bytes()).unwrap())
                    .is_err()
            );
        }
    }
}
