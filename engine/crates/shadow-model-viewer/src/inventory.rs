//! Предметы учебного участка: нож, BDA и кусачки. Подбор не равен экипировке.
use crate::ui_controls::{ButtonVisualState, UiButton};
use crate::{
    combat::Combat, map_scene::WORLD_SCALE, pickup, player, rig, scene, tutorial::Tutorial,
};
use bevy::{mesh::VertexAttributeValues, prelude::*};
use shadow_formats::{AssetCatalog, CampaignEventType as Event, ConfigLine, config_lines};
use std::{error::Error, io::Cursor};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Weapon {
    Knife,
    Bda,
    Cutters,
    Uzi,
}
impl Weapon {
    pub fn name(self) -> &'static str {
        match self {
            Self::Knife => "Нож",
            Self::Bda => "BDA",
            Self::Cutters => "Кусачки",
            Self::Uzi => "UZI",
        }
    }
}
pub struct GunDefinition {
    pub model: String,
    pub magazine: u32,
    pub range: f32,
    pub delay: f32,
    pub base_damage: f32,
    pub damage_mod: f32,
    pub auto_equip: bool,
    pub burst_count: u32,
    pub burst_round_delay: f32,
    pub sound: Vec<u8>,
    pub source: serde_json::Value,
}
impl GunDefinition {
    pub fn damage(&self) -> f32 {
        self.base_damage * self.damage_mod / 100.0
    }
}
pub(crate) fn field<'a>(lines: &'a [ConfigLine], key: &str) -> Result<&'a str, Box<dyn Error>> {
    let values: Vec<_> = lines.iter().filter(|l| l.words[0] == key).collect();
    match values.as_slice() {
        [l] if l.words.len() == 2 => Ok(&l.words[1]),
        _ => Err(format!("неверное поле предмета {key}").into()),
    }
}
pub(crate) fn block<'a>(
    lines: &'a [ConfigLine],
    key: &str,
    id: &str,
    end_key: &str,
) -> Result<&'a [ConfigLine], Box<dyn Error>> {
    let starts: Vec<_> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| {
            l.words.len() == 2 && l.words[0] == key && l.words[1].eq_ignore_ascii_case(id)
        })
        .map(|(i, _)| i)
        .collect();
    let [start] = starts.as_slice() else {
        return Err(format!("нужно одно определение {id}").into());
    };
    let end = lines[start + 1..]
        .iter()
        .position(|l| l.words[0] == end_key)
        .map(|i| start + 1 + i)
        .unwrap_or(lines.len());
    Ok(&lines[*start..end])
}
pub fn gun_definition(catalog: &AssetCatalog) -> Result<GunDefinition, Box<dyn Error>> {
    definition(catalog, "BDA", "CSTYLE_ONE_HAND")
}
pub fn definition(
    catalog: &AssetCatalog,
    name: &str,
    style: &str,
) -> Result<GunDefinition, Box<dyn Error>> {
    let path = "config/inventory_object_profiles/guns.txt";
    let lines = config_lines(&catalog.read(path)?)?;
    let gun = block(&lines, "OBJECT_NAME", name, "OBJECT_DEF")?;
    if field(gun, "OBJECT_EQUIP_LOC")? != "EQL_RIGHT_HAND"
        || field(gun, "OBJECT_CARRY_STYLE")? != style
        || !field(gun, "AMMO_TYPE")?.eq_ignore_ascii_case("9mm")
    {
        return Err("вариант BDA пока не поддерживается".into());
    }
    let auto_equip = match field(gun, "OBJECT_AUTO_EQUIP")?
        .to_ascii_uppercase()
        .as_str()
    {
        "TRUE" => true,
        "FALSE" => false,
        _ => return Err("неверный OBJECT_AUTO_EQUIP BDA".into()),
    };
    let magazine = field(gun, "MAGAZINE_SIZE")?.parse::<u32>()?;
    let burst_count = if name == "UZI" {
        field(gun, "BURST_COUNT_MAX")?.parse::<u32>()?
    } else {
        1
    };
    let burst_round_ms = if name == "UZI" {
        field(gun, "BURST_ROUND_DELAY")?.parse::<u32>()?
    } else {
        1
    };
    if !(1..=20).contains(&burst_count) || !(1..=60000).contains(&burst_round_ms) {
        return Err("неверные параметры очереди".into());
    }
    let delay_ms = field(gun, "FIRE_DELAY")?.parse::<u32>()?;
    let range_source = field(gun, "MAX_RANGE")?.parse::<f32>()?;
    let damage_mod = field(gun, "AMMO_DAMAGE_MOD")?.parse::<f32>()?;
    if !(1..=10000).contains(&magazine)
        || !(1..=60000).contains(&delay_ms)
        || !range_source.is_finite()
        || !(0.0..=1_000_000.0).contains(&range_source)
        || range_source == 0.0
        || !damage_mod.is_finite()
        || !(0.0..=500.0).contains(&damage_mod)
        || damage_mod == 0.0
    {
        return Err("неверные параметры BDA".into());
    }
    let shell_path = "config/shell_damage_models.txt";
    let shells = config_lines(&catalog.read(shell_path)?)?;
    let shell = block(&shells, "SHELL_DAMAGE_MODEL", "9MM", "SHELL_DAMAGE_MODEL")?;
    let base_damage = field(shell, "BASE_DAMAGE")?.parse::<f32>()?;
    if !base_damage.is_finite() || !(0.0..=100000.0).contains(&base_damage) || base_damage == 0.0 {
        return Err("неверный BASE_DAMAGE 9MM".into());
    }
    let model = player::model_path(catalog, field(gun, "OBJECT_MODEL")?)?;
    let sound_id = field(gun, "FIRE_SAMPLE")?;
    let audio = config_lines(&catalog.read("config/audio_defs.txt")?)?;
    let sample = block(&audio, "AUDIO_DEF", sound_id, "AUDIO_DEF")?;
    let filename = format!("{}.wav", field(sample, "FILE")?.to_ascii_lowercase());
    let matches: Vec<_> = catalog
        .paths()
        .filter(|p| p.rsplit('/').next() == Some(filename.as_str()))
        .collect();
    let [sound_path] = matches.as_slice() else {
        return Err("не найден один звук BDA".into());
    };
    let sound = catalog.read(sound_path)?;
    validate_sound(&sound)?;
    let source = serde_json::json!({"profile":path,"profile_origin":catalog.source_label(path)?,"model":model,
        "magazine_size":magazine,"max_range_source_units":range_source,"fire_delay_ms":delay_ms,
        "fire_delay_rand_ms":field(gun,"FIRE_DELAY_RAND")?.parse::<u32>()?,"reload_delay_ms":field(gun,"RELOAD_DELAY")?.parse::<u32>()?,
        "ammo_type":"9mm","shell_profile":shell_path,"base_damage":base_damage,"rand_damage":field(shell,"RAND_DAMAGE")?.parse::<f32>()?,"ammo_damage_mod":damage_mod,
        "prototype_damage":base_damage*damage_mod/100.0,"damage_formula_original_verified":false,"accuracy_original_verified":false,
        "random_damage_applied":false,"random_fire_delay_applied":false,"initial_full_magazine_original_verified":false,
        "sound":sound_path,"sound_origin":catalog.source_label(sound_path)?,"auto_equip":auto_equip,"name":name,"carry_style":style,"burst_count":burst_count,"burst_round_delay_ms":burst_round_ms,"burst_schedule_original_verified":false});
    Ok(GunDefinition {
        model,
        magazine,
        range: range_source * WORLD_SCALE,
        delay: delay_ms as f32 / 1000.0,
        base_damage,
        damage_mod,
        auto_equip,
        burst_count,
        burst_round_delay: burst_round_ms as f32 / 1000.0,
        sound,
        source,
    })
}
pub(crate) fn validate_sound(bytes: &[u8]) -> Result<(), Box<dyn Error>> {
    if bytes.len() > 32 * 1024 * 1024 {
        return Err("звук BDA слишком велик".into());
    }
    let mut reader = hound::WavReader::new(Cursor::new(bytes))?;
    let spec = reader.spec();
    if spec.channels == 0 || spec.sample_rate == 0 || reader.duration() == 0 {
        return Err("звук BDA без сэмплов".into());
    }
    match spec.sample_format {
        hound::SampleFormat::Int => {
            for sample in reader.samples::<i32>() {
                sample?;
            }
        }
        hound::SampleFormat::Float => {
            for sample in reader.samples::<f32>() {
                if !sample?.is_finite() {
                    return Err("нечисловой сэмпл BDA".into());
                }
            }
        }
    }
    Ok(())
}
#[derive(Resource)]
pub struct Inventory {
    pub gun: GunDefinition,
    pub uzi: Option<GunDefinition>,
    pub owns_uzi: bool,
    pub uzi_rounds: u32,
    pub uzi_shots: u32,
    pub uzi_sound_handle: Option<Handle<AudioSource>>,
    pub knife_model: String,
    pub cutters_model: Option<String>,
    pub owns_cutters: bool,
    pub rounds: u32,
    pub owns_knife: bool,
    pub collected: bool,
    pub available: bool,
    pub ordered: bool,
    pub open: bool,
    pub consumed_input: bool,
    pub drag: Option<Weapon>,
    pub shots: u32,
    pub position: Option<Vec3>,
    pub center: Option<Vec3>,
    model: Option<scene::PreparedModel>,
    triangles: Vec<[Vec3; 3]>,
    local_center: Vec3,
    lower_y: f32,
    pub drop_transform: Transform,
    pub sound_handle: Option<Handle<AudioSource>>,
}
impl Inventory {
    pub fn prepare(
        catalog: &AssetCatalog,
        actor: &mut player::Player,
    ) -> Result<Self, Box<dyn Error>> {
        let gun = gun_definition(catalog)?;
        let knife_model = actor
            .prepared_weapon
            .clone()
            .ok_or("нож должен быть подготовлен первым")?;
        let r = actor
            .rig
            .as_mut()
            .ok_or("BDA готовится до создания сцены")?;
        let first = r.parts.len();
        rig::prepare_weapon(catalog, r, &gun.model, "BDA")?;
        actor
            .equipment_parts
            .push((gun.model.clone(), first..r.parts.len()));
        let uzi = definition(catalog, "UZI", "CSTYLE_RIFLE")?;
        let first = r.parts.len();
        rig::prepare_weapon(catalog, r, &uzi.model, "UZI")?;
        actor
            .equipment_parts
            .push((uzi.model.clone(), first..r.parts.len()));
        let model = scene::prepare(catalog, &gun.model, false)?;
        let mut lower = Vec3::splat(f32::INFINITY);
        let mut upper = Vec3::splat(f32::NEG_INFINITY);
        let mut triangles = Vec::new();
        for part in &model.parts {
            let Some(VertexAttributeValues::Float32x3(v)) =
                part.mesh.attribute(Mesh::ATTRIBUTE_POSITION)
            else {
                return Err("нет геометрии BDA".into());
            };
            let points: Vec<_> = v.iter().map(|v| Vec3::from_array(*v)).collect();
            for &p in &points {
                lower = lower.min(p);
                upper = upper.max(p);
            }
            let indices: Vec<_> = part
                .mesh
                .indices()
                .ok_or("нет индексов BDA")?
                .iter()
                .collect();
            for i in indices.as_chunks::<3>().0 {
                triangles.push([points[i[0]], points[i[1]], points[i[2]]]);
            }
        }
        if !lower.is_finite() || !upper.is_finite() || triangles.is_empty() {
            return Err("пустая модель BDA".into());
        }
        Ok(Self {
            rounds: gun.magazine,
            uzi_rounds: uzi.magazine,
            uzi: Some(uzi),
            owns_uzi: false,
            uzi_shots: 0,
            uzi_sound_handle: None,
            gun,
            knife_model,
            cutters_model: None,
            owns_cutters: false,
            owns_knife: false,
            collected: false,
            available: false,
            ordered: false,
            open: false,
            consumed_input: false,
            drag: None,
            shots: 0,
            position: None,
            center: None,
            model: Some(model),
            triangles,
            local_center: (lower + upper) * 0.5,
            lower_y: lower.y,
            drop_transform: Transform::default(),
            sound_handle: None,
        })
    }
    pub fn observe(&mut self, item: &pickup::Pickup, combat: &Combat) {
        self.owns_knife = item.collected;
        if !combat.alive() && !self.available {
            self.available = true;
            // Прототип размещения возле погибшего: точная формула выпадения ещё не восстановлена.
            let p = combat.guard.position + Vec3::new(0.55, 0.03 - self.lower_y * WORLD_SCALE, 0.2);
            self.drop_transform = Transform::from_translation(p)
                .with_rotation(Quat::from_rotation_y(combat.guard.heading))
                .with_scale(Vec3::splat(WORLD_SCALE));
            self.position = Some(Vec3::new(p.x, combat.guard.position.y, p.z));
            self.center = Some(self.drop_transform.transform_point(self.local_center));
        }
    }
    pub fn weapon(&self, actor: &player::Player) -> Option<Weapon> {
        if self.owns_knife && actor.weapon.as_ref() == Some(&self.knife_model) {
            Some(Weapon::Knife)
        } else if self.collected && actor.weapon.as_ref() == Some(&self.gun.model) {
            Some(Weapon::Bda)
        } else if self.owns(Weapon::Cutters) && actor.weapon.as_ref() == self.cutters_model.as_ref()
        {
            Some(Weapon::Cutters)
        } else if self.owns_uzi
            && self
                .uzi
                .as_ref()
                .is_some_and(|g| actor.weapon.as_ref() == Some(&g.model))
        {
            Some(Weapon::Uzi)
        } else {
            None
        }
    }
    pub fn owns(&self, w: Weapon) -> bool {
        match w {
            Weapon::Knife => self.owns_knife,
            Weapon::Bda => self.collected,
            Weapon::Uzi => self.owns_uzi && self.uzi.is_some(),
            Weapon::Cutters => self.owns_cutters && self.cutters_model.is_some(),
        }
    }
    pub fn equip(&self, w: Weapon, actor: &mut player::Player) -> bool {
        if !self.owns(w) {
            return false;
        }
        actor.weapon = Some(match w {
            Weapon::Knife => self.knife_model.clone(),
            Weapon::Bda => self.gun.model.clone(),
            Weapon::Uzi => self.uzi.as_ref().unwrap().model.clone(),
            Weapon::Cutters => self.cutters_model.clone().unwrap(),
        });
        actor.rifle_equipped = w == Weapon::Uzi;
        true
    }
    pub fn gun_for(&self, w: Weapon) -> Option<&GunDefinition> {
        match w {
            Weapon::Bda => Some(&self.gun),
            Weapon::Uzi => self.uzi.as_ref(),
            _ => None,
        }
    }
    pub fn take_round_for(&mut self, w: Weapon) -> bool {
        if w == Weapon::Bda {
            self.take_round()
        } else if w == Weapon::Uzi && self.owns_uzi && self.uzi_rounds > 0 {
            self.uzi_rounds -= 1;
            self.uzi_shots += 1;
            true
        } else {
            false
        }
    }
    pub fn take_round(&mut self) -> bool {
        if !self.collected || self.rounds == 0 {
            false
        } else {
            self.rounds -= 1;
            self.shots += 1;
            true
        }
    }
    pub fn order(
        &mut self,
        actor: &mut player::Player,
        grid: &shadow_formats::TerrainGrid,
    ) -> bool {
        if !self.available || self.collected || actor.playback.paused {
            return false;
        }
        if actor.order(grid, self.position.unwrap(), false) {
            self.ordered = true;
            true
        } else {
            false
        }
    }
    pub fn cancel(&mut self) {
        self.ordered = false;
    }
    pub fn restart(&mut self) {
        self.rounds = self.gun.magazine;
        self.uzi_rounds = self.uzi.as_ref().map(|g| g.magazine).unwrap_or(0);
        self.owns_uzi = false;
        self.uzi_shots = 0;
        self.owns_knife = false;
        self.owns_cutters = false;
        self.collected = false;
        self.available = false;
        self.ordered = false;
        self.open = false;
        self.drag = None;
        self.position = None;
        self.center = None;
        self.shots = 0;
    }
    pub fn update(&mut self, actor: &mut player::Player, t: &mut Tutorial, callback: f32) {
        if !self.ordered
            || self.open
            || actor.playback.paused
            || actor.changing_posture()
            || t.pausing()
        {
            return;
        }
        let p = self.position.unwrap();
        let distance = Vec2::new(p.x - actor.position.x, p.z - actor.position.z).length();
        if actor.pickup_seconds.is_none()
            && actor.destination.is_none()
            && !self.collected
            && distance > 0.5
        {
            self.cancel();
            return;
        }
        if distance <= 0.5 && actor.pickup_seconds.is_none() && !self.collected {
            actor.destination = None;
            actor.begin_pickup();
        }
        if let Some(seconds) = actor.pickup_seconds {
            if !self.collected && seconds >= actor.pickup_callback(callback) && distance <= 0.5 {
                self.collected = true;
                if self.gun.auto_equip {
                    self.equip(Weapon::Bda, actor);
                }
                t.fire(Event::ObjectPickedUp, "BDA");
                t.tick(0.0, false);
            }
            if seconds >= actor.pickup_duration() {
                actor.finish_pickup();
                self.ordered = false;
            }
        }
    }
    pub fn clicked(
        &self,
        camera: &Camera,
        transform: &GlobalTransform,
        cursor: Vec2,
        geometry: &pickup::Pickup,
    ) -> bool {
        if !self.available || self.collected {
            return false;
        }
        let center = self.center.unwrap();
        if !geometry.clear_line(transform.translation(), center) {
            return false;
        }
        let Ok(ray) = camera.viewport_to_world(transform, cursor) else {
            return false;
        };
        let world = self.drop_transform.to_matrix();
        self.triangles.iter().any(|t| {
            pickup::ray_triangle(
                ray.origin,
                ray.direction.as_vec3(),
                t.map(|p| world.transform_point3(p)),
            )
            .is_some()
        }) || camera
            .world_to_viewport(transform, center)
            .is_ok_and(|p| p.distance(cursor) <= 14.0)
    }
    pub fn report(&self, actor: &player::Player) -> serde_json::Value {
        serde_json::json!({"owned_knife":self.owns_knife,"owned_uzi":self.owns_uzi,"uzi_rounds":self.uzi_rounds,"uzi_shots":self.uzi_shots,"uzi":self.uzi.as_ref().map(|g|&g.source),"owned_cutters":self.owns_cutters,"bda_available":self.available,"bda_collected":self.collected,"pickup_ordered":self.ordered,
            "primary":self.weapon(actor).map(|w|w.name()),"rounds":self.rounds,"shots":self.shots,"open":self.open,"dragging":self.drag.map(|w|w.name()),
            "drop_position_metres":self.position.map(|p|p.to_array()),"drop_center_metres":self.center.map(|p|p.to_array()),"gun":self.gun.source,
            "drop_placement_original_verified":false,"inventory_pause_original_verified":false,"reserve_ammunition":0,"reload_implemented":false})
    }
}
#[derive(Component)]
pub struct DropRoot;
#[derive(Component)]
pub struct DropRing;
#[derive(Component)]
pub struct InventoryPanel;
#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    Primary,
    Stored(Weapon),
    Medical,
    Patient,
    BodyArmor,
    Close,
}

pub fn setup(
    mut commands: Commands,
    inventory: Option<ResMut<Inventory>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut sounds: ResMut<Assets<AudioSource>>,
) {
    let Some(mut inv) = inventory else {
        return;
    };
    let model = inv.model.take().expect("inventory setup runs once");
    inv.sound_handle = Some(sounds.add(AudioSource {
        bytes: inv.gun.sound.clone().into(),
    }));
    if let Some(uzi) = &inv.uzi {
        inv.uzi_sound_handle = Some(sounds.add(AudioSource {
            bytes: uzi.sound.clone().into(),
        }));
    }
    let root = commands
        .spawn((
            player::SceneEntity,
            DropRoot,
            Name::new("Dropped BDA"),
            Transform::default(),
            Visibility::Hidden,
        ))
        .id();
    let textures = model
        .images
        .into_iter()
        .map(|(p, t)| (p, (images.add(t.image), t.alpha_mode)))
        .collect::<std::collections::BTreeMap<_, _>>();
    for part in model.parts {
        commands.spawn((
            Mesh3d(meshes.add(part.mesh)),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color_texture: part.texture.as_ref().map(|p| textures[p].0.clone()),
                alpha_mode: part.alpha_mode,
                cull_mode: None,
                perceptual_roughness: 0.95,
                ..default()
            })),
            Transform::default(),
            ChildOf(root),
        ));
    }
    commands.spawn((
        player::SceneEntity,
        DropRing,
        Mesh3d(meshes.add(Torus::new(0.22, 0.25))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.8, 0.12),
            unlit: true,
            ..default()
        })),
        Transform::default(),
        Visibility::Hidden,
    ));
}
#[allow(clippy::too_many_arguments)]
pub fn input(
    inventory: Option<ResMut<Inventory>>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    actor: Option<ResMut<player::Player>>,
    combat: Option<ResMut<Combat>>,
    item: Option<ResMut<pickup::Pickup>>,
    t: Option<Res<Tutorial>>,
    slots: Query<(&Slot, &ButtonVisualState)>,
    window: Query<&Window>,
    camera: Query<(&Camera, &GlobalTransform), With<crate::training::MapCamera>>,
    pointer: (
        Option<Res<pickup::Smoke>>,
        Option<Res<crate::selection::Selection>>,
    ),
    grenade: Option<ResMut<crate::grenade::Grenade>>,
    fence: Option<ResMut<crate::fence::Fence>>,
    uzi: Option<ResMut<crate::smg::Uzi>>,
    medical: Option<Res<crate::medical::Medical>>,
    armor: Option<ResMut<crate::armor::Armor>>,
) {
    let (smoke, selection) = pointer;
    let Some(mut inv) = inventory else {
        return;
    };
    inv.consumed_input = false;
    if t.is_some_and(|t| t.active.is_some() || t.consumed_input) {
        return;
    }
    let Some(mut actor) = actor else {
        return;
    };
    if keys.just_pressed(KeyCode::Home) && !inv.open {
        inv.restart();
        return;
    }
    if keys.just_pressed(KeyCode::KeyH) && actor.selected && !inv.open {
        inv.cancel();
    }
    let on_actor = if !selection.as_ref().is_some_and(|s| s.world_blocked)
        && mouse.just_pressed(MouseButton::Right)
        && !keys.pressed(KeyCode::ShiftLeft)
        && !keys.pressed(KeyCode::ShiftRight)
    {
        if let (Ok(window), Ok((camera, transform))) = (window.single(), camera.single()) {
            let cursor = armor.as_ref().and_then(|a| a.test_cursor).or_else(|| {
                medical.as_ref().and_then(|m| m.test_cursor).or_else(|| {
                    smoke
                        .as_ref()
                        .and_then(|s| s.cursor)
                        .or_else(|| window.cursor_position())
                })
            });
            cursor.is_some_and(|cursor| {
                crate::training::actor_clicked(camera, transform, cursor, &actor)
            })
        } else {
            false
        }
    } else {
        false
    };
    if on_actor {
        actor.selected = true;
    }
    if (keys.just_pressed(KeyCode::Space) || on_actor) && actor.selected {
        inv.open = !inv.open;
        inv.drag = None;
        inv.consumed_input = true;
        keys.reset(KeyCode::Space);
        return;
    }
    if !inv.open {
        return;
    }
    let close = keys.just_pressed(KeyCode::Escape)
        || slots
            .iter()
            .any(|(s, i)| *s == Slot::Close && *i == ButtonVisualState::Pressed);
    if close {
        inv.open = false;
        inv.drag = None;
        inv.consumed_input = true;
        keys.reset(KeyCode::Escape);
        return;
    }
    if actor.carry_movement.is_some() {
        inv.drag = None;
        inv.consumed_input = true;
        return;
    }
    if inv.drag.is_some()
        && !mouse.pressed(MouseButton::Left)
        && !mouse.just_released(MouseButton::Left)
    {
        inv.drag = None;
    }
    if mouse.just_pressed(MouseButton::Left) {
        inv.drag = slots.iter().find_map(|(s, i)| {
            if *i == ButtonVisualState::Pressed {
                if let Slot::Stored(w) = s {
                    inv.owns(*w).then_some(*w)
                } else {
                    None
                }
            } else {
                None
            }
        });
    }
    if mouse.just_released(MouseButton::Left) {
        let target = slots
            .iter()
            .any(|(s, i)| *s == Slot::Primary && *i != ButtonVisualState::None);
        if let Some(w) = inv.drag.take()
            && target
            && inv.equip(w, &mut actor)
        {
            if let Some(mut c) = combat {
                c.cancel_all(&mut actor);
            }
            if let Some(mut item) = item {
                item.cancel();
            }
            inv.cancel();
            if let Some(mut g) = grenade {
                g.cancel(&mut actor);
            }
            if let Some(mut f) = fence {
                f.cancel(&mut actor);
            }
            if let Some(mut u) = uzi {
                u.cancel(&mut actor);
            }
            if let Some(mut a) = armor {
                a.ground.cancel(&mut actor);
            }
            actor.pickup_seconds = None;
            actor.destination = None;
        }
    }
}
#[allow(clippy::too_many_arguments, clippy::type_complexity)] // Непересекающиеся запросы Bevy.
pub fn sync(
    inventory: Option<ResMut<Inventory>>,
    actor: Option<ResMut<player::Player>>,
    tutorial: Option<ResMut<Tutorial>>,
    item: Option<Res<pickup::Pickup>>,
    combat: Option<Res<Combat>>,
    mut roots: Query<(&mut Transform, &mut Visibility), (With<DropRoot>, Without<DropRing>)>,
    mut rings: Query<(&mut Transform, &mut Visibility), (With<DropRing>, Without<DropRoot>)>,
) {
    let (Some(mut inv), Some(mut actor), Some(mut t), Some(item), Some(c)) =
        (inventory, actor, tutorial, item, combat)
    else {
        return;
    };
    inv.observe(&item, &c);
    inv.update(&mut actor, &mut t, item.callback_seconds);
    let visible = inv.available && !inv.collected;
    for (mut transform, mut v) in &mut roots {
        *transform = inv.drop_transform;
        *v = if visible {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
    for (mut transform, mut v) in &mut rings {
        if let Some(p) = inv.position {
            transform.translation = p + Vec3::Y * 0.04;
        }
        *v = if visible {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
}
type PanelState = (
    bool,
    bool,
    bool,
    Option<Weapon>,
    u32,
    Option<Weapon>,
    Option<u32>,
    bool,
    (bool, u32),
    Option<(bool, u32, u32, bool)>,
    Option<(bool, bool)>,
);
#[allow(clippy::too_many_arguments)] // Ресурсы и запросы Bevy.
pub fn ui(
    mut commands: Commands,
    inv: Option<Res<Inventory>>,
    actor: Option<Res<player::Player>>,
    panels: Query<Entity, With<InventoryPanel>>,
    camera: Single<Entity, With<crate::training::MapCamera>>,
    mut fonts: ResMut<Assets<Font>>,
    mut previous: Local<Option<PanelState>>,
    grenade: Option<Res<crate::grenade::Grenade>>,
    medical: Option<Res<crate::medical::Medical>>,
    armor: Option<Res<crate::armor::Armor>>,
) {
    let (Some(inv), Some(actor)) = (inv, actor) else {
        return;
    };
    let state = (
        inv.open,
        inv.owns_knife,
        inv.collected,
        inv.weapon(&actor),
        inv.rounds,
        inv.drag,
        grenade.as_ref().map(|g| g.count),
        inv.owns_cutters,
        (inv.owns_uzi, inv.uzi_rounds),
        medical
            .as_ref()
            .map(|m| (m.ground.collected, m.uses, m.health.to_bits(), m.armed)),
        armor.as_ref().map(|a| (a.ground.collected, a.worn)),
    );
    if previous.as_ref() == Some(&state) && (!inv.open || !panels.is_empty()) {
        return;
    }
    *previous = Some(state);
    for e in &panels {
        commands.entity(e).despawn();
    }
    if !inv.open {
        return;
    }
    let font = fonts.add(Font::from_bytes(
        include_bytes!("../../shadow-viewer/assets/fonts/DejaVuSans.ttf").to_vec(),
    ));
    let overlay = commands
        .spawn((
            player::SceneEntity,
            InventoryPanel,
            UiTargetCamera(*camera),
            GlobalZIndex(300),
            Node {
                position_type: PositionType::Absolute,
                left: px(0),
                top: px(0),
                width: percent(100),
                height: percent(100),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.45)),
        ))
        .id();
    let panel = commands
        .spawn((
            Node {
                width: px(580),
                padding: UiRect::all(px(24)),
                row_gap: px(16),
                flex_direction: FlexDirection::Column,
                ..default()
            },
            BackgroundColor(Color::srgb(0.025, 0.055, 0.045)),
            ChildOf(overlay),
        ))
        .id();
    let label = |commands: &mut Commands, parent: Entity, text: String, size: f32| {
        commands.spawn((
            Text::new(text),
            TextFont {
                font: font.clone().into(),
                font_size: FontSize::Px(size),
                ..default()
            },
            TextColor(Color::srgb(0.9, 0.94, 0.8)),
            ChildOf(parent),
        ));
    };
    label(
        &mut commands,
        panel,
        "JOHN EMERSON · Инвентарь · пауза".into(),
        21.0,
    );
    label(
        &mut commands,
        panel,
        "Оружие: перетащить. Аптечка: ПКМ, затем ЛКМ по Джону.".into(),
        16.0,
    );
    label(
        &mut commands,
        panel,
        grenade
            .as_ref()
            .map(|g| {
                if g.secondary {
                    format!("Дополнительное: осколочная граната ×{}", g.count)
                } else {
                    "Дополнительное: нет".into()
                }
            })
            .unwrap_or_default(),
        18.0,
    );
    if let Some(m) = &medical {
        let patient = commands
            .spawn((
                UiButton,
                Slot::Patient,
                Node {
                    min_height: px(40),
                    padding: UiRect::all(px(8)),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.10, 0.22, 0.13)),
                ChildOf(panel),
            ))
            .id();
        label(
            &mut commands,
            patient,
            format!(
                "Джон · здоровье {:.0}/{:.0}{}",
                m.health,
                m.maximum,
                if m.armed {
                    " · ЛКМ: лечить"
                } else {
                    ""
                }
            ),
            18.0,
        );
        if m.ground.collected {
            let kit = commands
                .spawn((
                    UiButton,
                    Slot::Medical,
                    Node {
                        min_height: px(40),
                        padding: UiRect::all(px(8)),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.25, 0.10, 0.10)),
                    ChildOf(panel),
                ))
                .id();
            label(
                &mut commands,
                kit,
                format!("+ Аптечка · зарядов: {} · ПКМ: выбрать", m.uses),
                18.0,
            );
        }
    }
    if let Some(a) = &armor {
        let body = commands
            .spawn((
                UiButton,
                Slot::BodyArmor,
                Node {
                    min_height: px(40),
                    padding: UiRect::all(px(8)),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.10, 0.18, 0.24)),
                ChildOf(panel),
            ))
            .id();
        let text = if a.worn {
            "Тело: бронежилет · ЛКМ: снять"
        } else if a.ground.collected {
            "Тело: форма · ЛКМ: надеть бронежилет"
        } else {
            "Тело: форма"
        };
        label(&mut commands, body, text.into(), 18.0);
    }
    let primary = commands
        .spawn((
            UiButton,
            Slot::Primary,
            Node {
                min_height: px(70),
                padding: UiRect::all(px(14)),
                ..default()
            },
            BackgroundColor(Color::srgb(0.16, 0.28, 0.13)),
            ChildOf(panel),
        ))
        .id();
    label(
        &mut commands,
        primary,
        format!(
            "Основное оружие: {}",
            inv.weapon(&actor).map(|w| w.name()).unwrap_or("нет")
        ),
        18.0,
    );
    let row = commands
        .spawn((
            Node {
                column_gap: px(16),
                row_gap: px(12),
                flex_wrap: FlexWrap::Wrap,
                ..default()
            },
            ChildOf(panel),
        ))
        .id();
    for w in [Weapon::Knife, Weapon::Bda, Weapon::Cutters, Weapon::Uzi] {
        if inv.owns(w) && inv.weapon(&actor) != Some(w) {
            let button = commands
                .spawn((
                    UiButton,
                    Slot::Stored(w),
                    Node {
                        width: px(230),
                        min_height: px(65),
                        padding: UiRect::all(px(14)),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.13, 0.18, 0.16)),
                    ChildOf(row),
                ))
                .id();
            label(
                &mut commands,
                button,
                if w == Weapon::Bda {
                    format!("BDA · {}/{}", inv.rounds, inv.gun.magazine)
                } else if w == Weapon::Uzi {
                    format!(
                        "UZI · {}/{}",
                        inv.uzi_rounds,
                        inv.uzi.as_ref().unwrap().magazine
                    )
                } else {
                    w.name().into()
                },
                18.0,
            );
        }
    }
    label(
        &mut commands,
        panel,
        format!(
            "{}{}",
            if inv.collected {
                format!(
                    "BDA: {}/{} патронов · запасных: 0",
                    inv.rounds, inv.gun.magazine
                )
            } else {
                "BDA ещё не подобран".into()
            },
            inv.drag
                .map(|w| format!("\nПеренос: {} → основной слот", w.name()))
                .unwrap_or_default()
        ),
        16.0,
    );
    let close = commands
        .spawn((
            UiButton,
            Slot::Close,
            Node {
                align_self: AlignSelf::FlexEnd,
                padding: UiRect::axes(px(22), px(8)),
                ..default()
            },
            BackgroundColor(Color::srgb(0.18, 0.27, 0.16)),
            ChildOf(panel),
        ))
        .id();
    label(&mut commands, close, "Закрыть".into(), 16.0);
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    pub(crate) fn inventory() -> Inventory {
        Inventory {
            gun: GunDefinition {
                model: "bda".into(),
                magazine: 13,
                range: 20.0,
                delay: 0.3,
                base_damage: 27.0,
                damage_mod: 90.0,
                auto_equip: false,
                burst_count: 1,
                burst_round_delay: 0.001,
                sound: Vec::new(),
                source: serde_json::Value::Null,
            },
            uzi: None,
            owns_uzi: false,
            uzi_rounds: 0,
            uzi_shots: 0,
            uzi_sound_handle: None,
            knife_model: "knife".into(),
            cutters_model: None,
            owns_cutters: false,
            rounds: 13,
            owns_knife: true,
            collected: false,
            available: true,
            ordered: false,
            open: false,
            consumed_input: false,
            drag: None,
            shots: 0,
            position: Some(Vec3::new(0.5, 1.5, -0.5)),
            center: Some(Vec3::new(0.5, 1.5, -0.5)),
            model: None,
            triangles: Vec::new(),
            local_center: Vec3::ZERO,
            lower_y: 0.0,
            drop_transform: Transform::default(),
            sound_handle: None,
        }
    }
    #[test]
    fn pickup_callback_fires_once_and_respects_auto_equip() {
        for auto in [false, true] {
            let (_, mut p, mut t, grid) = pickup::tests::fixture();
            let mut inv = inventory();
            inv.gun.auto_equip = auto;
            p.weapon = Some("knife".into());
            assert!(inv.order(&mut p, &grid));
            inv.update(&mut p, &mut t, 0.3);
            p.step(&ButtonInput::default(), 0.29, &grid);
            inv.update(&mut p, &mut t, 0.3);
            assert!(!inv.collected);
            p.step(&ButtonInput::default(), 0.02, &grid);
            inv.update(&mut p, &mut t, 0.3);
            inv.update(&mut p, &mut t, 0.3);
            assert!(inv.collected);
            assert_eq!(
                p.weapon.as_deref(),
                Some(if auto { "bda" } else { "knife" })
            );
            assert_eq!(
                t.events
                    .iter()
                    .filter(|e| e.as_str() == "CET_OBJECT_PICKED_UP:BDA")
                    .count(),
                1
            );
            assert_eq!(inv.rounds, 13);
            assert!(t.completed.is_empty());
        }
    }
    #[test]
    fn swapping_preserves_both_weapons_and_spent_ammunition() {
        let mut inv = inventory();
        let mut p = player::tests::player();
        inv.collected = true;
        inv.rounds = 7;
        for w in [Weapon::Bda, Weapon::Knife, Weapon::Bda] {
            assert!(inv.equip(w, &mut p));
            assert_eq!(inv.weapon(&p), Some(w));
            assert!(inv.owns_knife && inv.collected);
            assert_eq!(inv.rounds, 7);
        }
    }
    #[test]
    fn absent_equipment_and_empty_magazine_are_rejected() {
        let mut inv = inventory();
        let mut p = player::tests::player();
        inv.owns_knife = false;
        assert!(!inv.equip(Weapon::Knife, &mut p));
        assert!(!inv.equip(Weapon::Bda, &mut p));
        assert!(!inv.take_round());
        inv.collected = true;
        inv.rounds = 1;
        assert!(inv.take_round());
        assert_eq!(inv.rounds, 0);
        assert!(!inv.take_round());
        assert_eq!(inv.shots, 1);
    }
    #[test]
    fn cancellation_before_callback_leaves_bda_on_ground() {
        let (_, mut p, mut t, grid) = pickup::tests::fixture();
        let mut inv = inventory();
        inv.order(&mut p, &grid);
        inv.update(&mut p, &mut t, 0.3);
        p.step(&ButtonInput::default(), 0.2, &grid);
        inv.cancel();
        p.step(&ButtonInput::default(), 1.0, &grid);
        inv.update(&mut p, &mut t, 0.3);
        assert!(!inv.collected);
        assert!(t.events.iter().all(|e| !e.ends_with(":BDA")));
    }
    #[test]
    fn inventory_and_help_pause_prevent_pending_pickup() {
        let (_, mut p, mut t, grid) = pickup::tests::fixture();
        let mut inv = inventory();
        inv.order(&mut p, &grid);
        inv.update(&mut p, &mut t, 0.3);
        p.step(&ButtonInput::default(), 0.4, &grid);
        inv.open = true;
        inv.update(&mut p, &mut t, 0.3);
        assert!(!inv.collected);
        inv.open = false;
        p.playback.paused = true;
        inv.update(&mut p, &mut t, 0.3);
        assert!(!inv.collected);
        p.playback.paused = false;
        t.tick(3.0, false);
        assert!(t.pausing());
        inv.update(&mut p, &mut t, 0.3);
        assert!(!inv.collected);
    }
    #[test]
    fn restart_restores_magazine_and_clears_ownership_and_drag() {
        let mut inv = inventory();
        inv.collected = true;
        inv.rounds = 3;
        inv.shots = 10;
        inv.open = true;
        inv.drag = Some(Weapon::Bda);
        inv.restart();
        assert_eq!(inv.rounds, 13);
        assert_eq!(inv.shots, 0);
        assert!(!inv.owns_knife && !inv.collected && !inv.available && !inv.open);
        assert!(inv.drag.is_none() && inv.position.is_none());
    }
    fn ui_app() -> App {
        let mut app = App::new();
        let mut inv = inventory();
        inv.collected = true;
        inv.rounds = 7;
        let mut p = player::tests::player();
        p.weapon = Some("knife".into());
        app.init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .insert_resource(inv)
            .insert_resource(p)
            .add_systems(Update, input);
        app
    }
    #[test]
    fn ui_drag_to_primary_swaps_weapon_and_escape_is_consumed() {
        let mut app = ui_app();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Space);
        app.update();
        assert!(app.world().resource::<Inventory>().open);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .reset_all();
        let stored = app
            .world_mut()
            .spawn((Slot::Stored(Weapon::Bda), ButtonVisualState::Pressed))
            .id();
        let primary = app
            .world_mut()
            .spawn((Slot::Primary, ButtonVisualState::None))
            .id();
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app.update();
        assert_eq!(app.world().resource::<Inventory>().drag, Some(Weapon::Bda));
        *app.world_mut()
            .get_mut::<ButtonVisualState>(stored)
            .unwrap() = ButtonVisualState::None;
        *app.world_mut()
            .get_mut::<ButtonVisualState>(primary)
            .unwrap() = ButtonVisualState::Hovered;
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .clear_just_pressed(MouseButton::Left);
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .release(MouseButton::Left);
        app.update();
        assert_eq!(
            app.world().resource::<player::Player>().weapon.as_deref(),
            Some("bda")
        );
        assert_eq!(app.world().resource::<Inventory>().rounds, 7);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Escape);
        app.update();
        assert!(!app.world().resource::<Inventory>().open);
        assert!(app.world().resource::<Inventory>().consumed_input);
        assert!(
            !app.world()
                .resource::<ButtonInput<KeyCode>>()
                .just_pressed(KeyCode::Escape)
        );
    }
    #[test]
    fn ui_release_outside_primary_keeps_weapon_and_ammo() {
        let mut app = ui_app();
        {
            let mut inv = app.world_mut().resource_mut::<Inventory>();
            inv.open = true;
            inv.drag = Some(Weapon::Bda);
        }
        {
            let mut mouse = app.world_mut().resource_mut::<ButtonInput<MouseButton>>();
            mouse.press(MouseButton::Left);
            mouse.clear_just_pressed(MouseButton::Left);
            mouse.release(MouseButton::Left);
        }
        app.update();
        assert_eq!(
            app.world().resource::<player::Player>().weapon.as_deref(),
            Some("knife")
        );
        assert_eq!(app.world().resource::<Inventory>().rounds, 7);
        assert!(app.world().resource::<Inventory>().drag.is_none());
    }
    #[test]
    fn wav_validation_rejects_truncation_and_non_finite_samples() {
        assert!(validate_sound(b"not a wave").is_err());
        let mut bytes = Cursor::new(Vec::new());
        {
            let mut writer = hound::WavWriter::new(
                &mut bytes,
                hound::WavSpec {
                    channels: 1,
                    sample_rate: 22050,
                    bits_per_sample: 32,
                    sample_format: hound::SampleFormat::Float,
                },
            )
            .unwrap();
            writer.write_sample(f32::NAN).unwrap();
            writer.finalize().unwrap();
        }
        assert!(validate_sound(bytes.get_ref()).is_err());
    }
}
