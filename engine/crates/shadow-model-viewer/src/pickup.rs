//! Нож начала обучения: связь MTF → профиль предмета → SMF → правая рука.
use crate::ui_controls::ButtonVisualState;
use crate::{
    map_scene::{MapScene, WORLD_SCALE},
    motion::Clip,
    player, rig,
    tutorial::Tutorial,
};
use bevy::{mesh::VertexAttributeValues, prelude::*};
use shadow_formats::{AssetCatalog, BmfMotion, CampaignEventType, config_lines};
use std::error::Error;

#[derive(Component)]
pub struct MapObject(pub [i32; 2]);
#[derive(Component)]
pub struct EquippedPart(pub String);
#[derive(Component)]
pub struct ItemRing;
struct Geometry {
    part: String,
    id: [i32; 2],
    opaque: bool,
    triangles: Vec<[Vec3; 3]>,
}
#[derive(Resource)]
pub struct Pickup {
    pub id: [i32; 2],
    pub name: String,
    pub model: String,
    pub position: Vec3,
    pub center: Vec3,
    pub ordered: bool,
    pub collected: bool,
    pub callback_seconds: f32,
    pub source: serde_json::Value,
    geometry: Vec<Geometry>,
}
impl Pickup {
    pub fn prepare(
        catalog: &AssetCatalog,
        scene: &MapScene,
        actor: &mut player::Player,
    ) -> Result<Self, Box<dyn Error>> {
        let profile_path = "config/inventory_object_profiles/knives.txt";
        let lines = config_lines(&catalog.read(profile_path)?)?;
        let start = lines
            .iter()
            .position(|l| {
                l.words[0] == "OBJECT_DEF"
                    && l.words
                        .get(1)
                        .is_some_and(|s| s.eq_ignore_ascii_case("knife"))
            })
            .ok_or("нет определения knife")?;
        let end = lines[start + 1..]
            .iter()
            .position(|l| l.words[0] == "OBJECT_DEF")
            .map(|i| start + 1 + i)
            .unwrap_or(lines.len());
        let field = |key: &str| -> Result<&str, Box<dyn Error>> {
            let matches = lines[start + 1..end]
                .iter()
                .filter(|l| l.words[0] == key)
                .collect::<Vec<_>>();
            match matches.as_slice() {
                [line] if line.words.len() == 2 => Ok(line.words[1].as_str()),
                _ => Err(format!("неверное поле ножа {key}").into()),
            }
        };
        let model_name = field("OBJECT_MODEL")?;
        if !field("OBJECT_EQUIP_LOC")?.eq_ignore_ascii_case("EQL_RIGHT_HAND")
            || !field("OBJECT_AUTO_EQUIP")?.eq_ignore_ascii_case("TRUE")
            || !field("OBJECT_CARRY_STYLE")?.eq_ignore_ascii_case("CSTYLE_ONE_HAND")
        {
            return Err("вариант экипировки ножа пока не поддерживается".into());
        }
        let basename = format!("{}.smf", model_name.to_ascii_lowercase());
        let candidates = scene
            .instances
            .iter()
            .filter(|i| {
                i.inventory
                    && i.path.rsplit('/').next() == Some(basename.as_str())
                    && i.name.eq_ignore_ascii_case("knife")
            })
            .collect::<Vec<_>>();
        let [item] = candidates.as_slice() else {
            return Err("нужен один стартовый нож в MTF".into());
        };
        // Имя клипа и момент выдачи предмета читаются из оригинального sequencer.
        let sequence_path = "config/mot_sequencer_defs.txt";
        let lines = config_lines(&catalog.read(sequence_path)?)?;
        let start = lines
            .iter()
            .position(|l| l.words == ["BEGIN_SEQUENCE", "MSEQ_STAND_PICK_UP_ONE_HAND"])
            .ok_or("нет последовательности подбора")?;
        let end = lines[start + 1..]
            .iter()
            .position(|l| l.words[0] == "END_SEQUENCE")
            .map(|i| start + 1 + i)
            .ok_or("последовательность подбора не завершена")?;
        let motion = lines[start + 1..end]
            .iter()
            .filter(|l| l.words[0] == "MOTION")
            .collect::<Vec<_>>();
        let [motion] = motion.as_slice() else {
            return Err("составная анимация подбора пока не поддерживается".into());
        };
        let path = format!("motions/{}.bmf", motion.words[1].to_ascii_lowercase());
        let callback = lines[start + 1..end]
            .iter()
            .filter(|l| l.words.get(2).is_some_and(|s| s == "MCB_PICK_UP_ITEM"))
            .collect::<Vec<_>>();
        let [callback] = callback.as_slice() else {
            return Err("нужен один callback подбора".into());
        };
        let frame = callback.words[1].parse::<usize>()?;
        let rig = actor
            .rig
            .as_mut()
            .ok_or("экипировка должна готовиться до создания сцены")?;
        let clip = Clip::new(&BmfMotion::from_bytes(&catalog.read(&path)?)?, &rig.bones)?;
        if frame >= clip.frames.len() {
            return Err("callback подбора за пределами BMF".into());
        }
        let callback_seconds = frame as f32 / clip.fps;
        actor.clips.push(clip);
        let first = rig.parts.len();
        rig::prepare_weapon(catalog, rig, &item.path, field("OBJECT_NAME")?)?;
        actor
            .equipment_parts
            .push((item.path.clone(), first..rig.parts.len()));
        actor.prepared_weapon = Some(item.path.clone());
        let mut geometry = Vec::new();
        let mut lower = Vec3::splat(f32::INFINITY);
        let mut upper = Vec3::splat(f32::NEG_INFINITY);
        for instance in &scene.instances {
            let world = Mat4::from_scale(Vec3::splat(WORLD_SCALE)) * instance.transform.to_matrix();
            for part in &scene.models[&instance.path].parts {
                let Some(VertexAttributeValues::Float32x3(vertices)) =
                    part.mesh.attribute(Mesh::ATTRIBUTE_POSITION)
                else {
                    return Err("нет позиций геометрии карты".into());
                };
                let points = vertices
                    .iter()
                    .map(|v| world.transform_point3(Vec3::from_array(*v)))
                    .collect::<Vec<_>>();
                if instance.id == item.id {
                    for &p in &points {
                        lower = lower.min(p);
                        upper = upper.max(p);
                    }
                }
                let indices = part
                    .mesh
                    .indices()
                    .ok_or("нет индексов геометрии карты")?
                    .iter()
                    .collect::<Vec<_>>();
                let triangles = indices
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .map(|i| [points[i[0]], points[i[1]], points[i[2]]])
                    .collect();
                geometry.push(Geometry {
                    id: instance.id,
                    part: part.name.clone(),
                    opaque: part.alpha_mode == AlphaMode::Opaque,
                    triangles,
                });
            }
        }
        if !lower.is_finite() || !upper.is_finite() {
            return Err("у ножа нет геометрии".into());
        }
        Ok(Self {
            id: item.id,
            name: item.name.clone(),
            model: item.path.clone(),
            position: item.transform.translation * WORLD_SCALE,
            center: (lower + upper) * 0.5,
            ordered: false,
            collected: false,
            callback_seconds,
            geometry,
            source: serde_json::json!({"profile":profile_path,"profile_origin":catalog.source_label(profile_path)?,"sequence":sequence_path,
                "motion":path,"pickup_callback_frame":frame,"fps":actor.clips[4].fps,"equipment_bone":rig::RIGHT_HAND,
                "approach_distance_metres":0.5,"approach_distance_original_verified":false}),
        })
    }
    pub fn order(
        &mut self,
        actor: &mut player::Player,
        grid: &shadow_formats::TerrainGrid,
    ) -> bool {
        if self.collected {
            return false;
        }
        if actor.order(grid, self.position, false) {
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
        self.ordered = false;
        self.collected = false;
    }
    pub fn visible_from(&self, origin: Vec3) -> bool {
        self.clear_line(origin, self.center)
    }
    pub fn clear_line(&self, origin: Vec3, target: Vec3) -> bool {
        self.clear_line_ignoring(origin, target, None)
    }
    // Центр выбранного предмета может находиться за его собственной поверхностью.
    pub fn clear_line_ignoring(
        &self,
        origin: Vec3,
        target: Vec3,
        selected: Option<[i32; 2]>,
    ) -> bool {
        let difference = target - origin;
        let distance = difference.length();
        if !distance.is_finite() || distance < 1e-6 {
            return false;
        }
        let direction = difference / distance;
        !self
            .geometry
            .iter()
            .filter(|g| g.id != self.id && Some(g.id) != selected && g.opaque)
            .flat_map(|g| &g.triangles)
            .any(|t| ray_triangle(origin, direction, *t).is_some_and(|d| d < distance - 0.02))
    }
    /// Перемещаем исходные треугольники машины вместе с видимой моделью.
    pub fn move_object(&mut self, id: [i32; 2], delta: Mat4) {
        for g in self.geometry.iter_mut().filter(|g| g.id == id) {
            for triangle in &mut g.triangles {
                *triangle = triangle.map(|p| delta.transform_point3(p));
            }
        }
    }
    /// Перемещаем выбранную часть, оставляя раму того же MTF-объекта на месте.
    pub fn move_parts(&mut self, id: [i32; 2], parts: &[String], delta: Mat4) {
        for g in self
            .geometry
            .iter_mut()
            .filter(|g| g.id == id && parts.contains(&g.part))
        {
            for triangle in &mut g.triangles {
                *triangle = triangle.map(|p| delta.transform_point3(p));
            }
        }
    }
    pub fn segment_hit(&self, from: Vec3, to: Vec3) -> Option<(f32, Vec3)> {
        let delta = to - from;
        let length = delta.length();
        if !length.is_finite() || length < 1e-6 {
            return None;
        }
        let direction = delta / length;
        self.geometry
            .iter()
            .filter(|g| g.id != self.id && g.opaque)
            .flat_map(|g| &g.triangles)
            .filter_map(|t| {
                let distance = ray_triangle(from, direction, *t)?;
                if distance > length {
                    return None;
                }
                let mut normal = (t[1] - t[0]).cross(t[2] - t[0]).normalize_or_zero();
                if normal.dot(direction) > 0.0 {
                    normal = -normal;
                }
                Some((distance, normal))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
    }
    pub fn clicked(&self, camera: &Camera, transform: &GlobalTransform, cursor: Vec2) -> bool {
        if self.collected || !self.visible_from(transform.translation()) {
            return false;
        }
        let Ok(ray) = camera.viewport_to_world(transform, cursor) else {
            return false;
        };
        let exact = self
            .geometry
            .iter()
            .filter(|g| g.id == self.id)
            .flat_map(|g| &g.triangles)
            .any(|t| ray_triangle(ray.origin, ray.direction.as_vec3(), *t).is_some());
        exact
            || camera
                .world_to_viewport(transform, self.center)
                .is_ok_and(|p| p.distance(cursor) <= 14.0)
    }
    pub fn report(&self) -> serde_json::Value {
        serde_json::json!({"id":self.id,"name":self.name,"model":self.model,"position_metres":self.position.to_array(),
            "center_metres":self.center.to_array(),"ordered":self.ordered,"collected":self.collected,"source":self.source})
    }
    /// Переход к предмету, анимация, callback. Замена приказа и H отменяют неподнятый предмет.
    pub fn update(&mut self, actor: &mut player::Player, tutorial: &mut Tutorial) {
        if !self.ordered {
            return;
        }
        if actor.pickup_seconds.is_none()
            && actor.destination.is_none()
            && !self.collected
            && Vec2::new(
                actor.position.x - self.position.x,
                actor.position.z - self.position.z,
            )
            .length()
                > 0.5
        {
            self.cancel();
            return;
        }
        if actor.playback.paused || actor.changing_posture() || tutorial.pausing() {
            return;
        }
        let distance = Vec2::new(
            actor.position.x - self.position.x,
            actor.position.z - self.position.z,
        )
        .length();
        if distance <= 0.5 && actor.pickup_seconds.is_none() && !self.collected {
            actor.destination = None;
            actor.begin_pickup();
        }
        if let Some(seconds) = actor.pickup_seconds {
            if !self.collected
                && seconds >= actor.pickup_callback(self.callback_seconds)
                && distance <= 0.5
            {
                self.collected = true;
                actor.weapon = Some(self.model.clone());
                actor.rifle_equipped = false;
                tutorial.fire(CampaignEventType::ObjectPickedUp, &self.name);
                tutorial.tick(0.0, false);
            }
            if seconds >= actor.pickup_duration() {
                actor.finish_pickup();
                self.ordered = false;
            }
        }
    }
}
pub fn ray_triangle(origin: Vec3, direction: Vec3, [a, b, c]: [Vec3; 3]) -> Option<f32> {
    if !origin.is_finite() || !direction.is_finite() {
        return None;
    }
    let e1 = b - a;
    let e2 = c - a;
    let p = direction.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-8 {
        return None;
    }
    let inv = det.recip();
    let t = origin - a;
    let u = t.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = t.cross(e1);
    let v = direction.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let d = e2.dot(q) * inv;
    (d >= 0.0).then_some(d)
}

pub fn input(
    keys: Res<ButtonInput<KeyCode>>,
    item: Option<ResMut<Pickup>>,
    tutorial: Option<ResMut<Tutorial>>,
    actor: Option<ResMut<player::Player>>,
    inventory: Option<Res<crate::inventory::Inventory>>,
) {
    if inventory.is_some_and(|inv| inv.open || inv.consumed_input) {
        return;
    }
    let Some(mut item) = item else {
        return;
    };
    if tutorial
        .as_ref()
        .is_some_and(|t| t.active.is_some() || t.consumed_input)
    {
        return;
    }
    if keys.just_pressed(KeyCode::Home) {
        item.restart();
        if let Some(mut actor) = actor {
            actor.weapon = None;
            actor.pickup_seconds = None;
        }
        if let Some(mut t) = tutorial {
            t.restart();
        }
    } else if keys.just_pressed(KeyCode::KeyH) && actor.is_some_and(|a| a.selected) {
        item.cancel();
    }
}
pub fn sync(
    mut commands: Commands,
    item: Option<ResMut<Pickup>>,
    actor: Option<ResMut<player::Player>>,
    tutorial: Option<ResMut<Tutorial>>,
    objects: Query<(Entity, &MapObject)>,
    mut equipment: Query<(&EquippedPart, &mut Visibility)>,
    inventory: Option<Res<crate::inventory::Inventory>>,
) {
    let (Some(mut item), Some(mut actor), Some(mut tutorial)) = (item, actor, tutorial) else {
        return;
    };
    if !inventory.is_some_and(|inv| inv.open) {
        item.update(&mut actor, &mut tutorial);
    }
    for (entity, id) in &objects {
        if id.0 == item.id {
            commands.entity(entity).insert(if item.collected {
                Visibility::Hidden
            } else {
                Visibility::Inherited
            });
        }
    }
    for (part, mut visible) in &mut equipment {
        *visible = if actor.weapon.as_ref() == Some(&part.0) {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
}
pub fn ring(
    item: Option<Res<Pickup>>,
    tutorial: Option<Res<Tutorial>>,
    time: Res<Time>,
    mut rings: Query<(&mut Visibility, &mut Transform), With<ItemRing>>,
) {
    let Some(item) = item else {
        return;
    };
    let pulsing = tutorial.as_ref().is_some_and(|t| t.pulse);
    for (mut visible, mut transform) in &mut rings {
        *visible = if item.collected {
            Visibility::Hidden
        } else {
            Visibility::Visible
        };
        let pulse = if pulsing {
            1.0 + 0.15 * (time.elapsed_secs() * 4.0).sin()
        } else {
            1.0
        };
        transform.scale = Vec3::splat(pulse);
    }
}

fn smoke_snapshot(commands: &mut Commands, path: Option<&std::path::Path>, tag: &str) {
    if let Some(path) = path {
        let stem = path.file_stem().unwrap().to_string_lossy();
        let target = path.with_file_name(format!("{stem}-{tag}.png"));
        use bevy::render::view::screenshot::{Screenshot, save_to_disk};
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(target));
    }
}

#[derive(Resource)]
#[allow(dead_code)] // Оконный тест запускается отдельной витриной карты.
pub struct Smoke {
    path: std::path::PathBuf,
    pub combat: bool,
    pub bda: bool,
    pub grenade: bool,
    pub fence: bool,
    pub smg: bool,
    smg_target: Option<usize>,
    logged_stage: Option<u8>,
    stage_frames: u32,
    pub cursor: Option<Vec2>,
    screenshot: Option<std::path::PathBuf>,
    stage: u8,
    frames: u32,
    wait: u32,
    initial: Option<Vec3>,
    camera: Option<Vec3>,
    observations: Vec<serde_json::Value>,
}
impl Smoke {
    #[allow(dead_code)]
    pub fn new(path: std::path::PathBuf, screenshot: Option<std::path::PathBuf>) -> Self {
        Self {
            path,
            combat: false,
            bda: false,
            grenade: false,
            fence: false,
            smg: false,
            smg_target: None,
            logged_stage: None,
            stage_frames: 0,
            cursor: None,
            screenshot,
            stage: 0,
            frames: 0,
            wait: 0,
            initial: None,
            camera: None,
            observations: Vec::new(),
        }
    }
}
#[allow(clippy::too_many_arguments, clippy::type_complexity)] // Независимые запросы оконной проверки.
pub fn smoke_input(
    mut commands: Commands,
    smoke: Option<ResMut<Smoke>>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    window: Single<&Window>,
    camera: Single<(&Camera, &GlobalTransform), With<crate::training::MapCamera>>,
    actor: Option<Res<player::Player>>,
    item: Option<Res<Pickup>>,
    tutorial: Option<Res<Tutorial>>,
    mut orbit: ResMut<crate::training::Orbit>,
    combat: Option<Res<crate::combat::Combat>>,
    inventory: Option<Res<crate::inventory::Inventory>>,
    mut slots: Query<(&crate::inventory::Slot, &mut ButtonVisualState)>,
    equipment: Query<(&EquippedPart, &Visibility, &GlobalTransform)>,
    mut exit: MessageWriter<AppExit>,
    barriers: (
        Option<Res<crate::grenade::Grenade>>,
        Option<Res<crate::fence::Fence>>,
        Option<Res<crate::smg::Uzi>>,
        Query<(
            Option<&MapObject>,
            Option<&crate::fence::CutRoot>,
            Option<&crate::fence::ToolRing>,
            &Visibility,
        )>,
    ),
) {
    let (grenade, fence, uzi, fence_scene) = barriers;
    let (Some(mut smoke), Some(actor), Some(item), Some(t)) = (smoke, actor, item, tutorial) else {
        return;
    };
    keys.reset_all();
    mouse.reset_all();
    smoke.frames += 1;
    if smoke.frames == 1 && smoke.smg && fence.as_ref().is_some_and(|f| f.cut && f.crossings > 0) {
        smoke.stage = 34;
        smoke
            .observations
            .push(serde_json::json!({"resumed_from_numeric_fence_checkpoint":true}));
    }
    if smoke.logged_stage != Some(smoke.stage) {
        println!(
            "tutorial smoke stage {}: posture={:?}, inventory={:?}, help={:?}",
            smoke.stage,
            actor.posture,
            inventory.as_ref().map(|i| (i.open, i.drag)),
            t.active
        );
        smoke.logged_stage = Some(smoke.stage);
        smoke.stage_frames = 0;
    } else {
        smoke.stage_frames += 1;
    }
    assert!(
        smoke.stage_frames < 600,
        "tutorial smoke stuck in stage {}: posture {:?}, inventory {:?}, help {:?}",
        smoke.stage,
        actor.posture,
        inventory.as_ref().map(|i| (i.open, i.drag)),
        t.active
    );

    assert!(
        smoke.frames < 4000,
        "tutorial smoke timeout: stage {}",
        smoke.stage
    );
    match smoke.stage {
        0 if t.active.as_deref() == Some("help_combat_1") => {
            if smoke.initial.is_none() {
                smoke.initial = Some(actor.position);
            }
            assert!(t.pausing());
            assert_eq!(actor.position, smoke.initial.unwrap());
            smoke.wait += 1;
            if smoke.wait == 1 {
                smoke_snapshot(&mut commands, smoke.screenshot.as_deref(), "welcome");
            }
            if smoke.wait >= 4 {
                smoke.observations.push(
                    serde_json::json!({"welcome_pauses":true,"position":actor.position.to_array()}),
                );
                keys.press(KeyCode::Enter);
                smoke.stage = 1;
            }
        }
        1 if t.active.is_none() && !t.consumed_input => {
            assert!(!item.collected);
            let cursor = camera
                .0
                .world_to_viewport(camera.1, item.center)
                .expect("нож виден на учебной площадке");
            assert!(cursor.x > 315.0 && cursor.y >= 0.0 && cursor.y < window.height());
            assert!(
                item.clicked(camera.0, camera.1, cursor),
                "видимый нож доступен для приказа"
            );
            smoke.cursor = Some(cursor);
            mouse.press(MouseButton::Left);
            smoke.stage = 2;
        }
        2 if item.collected => {
            assert_eq!(actor.weapon.as_deref(), Some(item.model.as_str()));
            assert!(
                equipment
                    .iter()
                    .any(|(part, v, _)| actor.weapon.as_ref() == Some(&part.0)
                        && *v != Visibility::Hidden)
                    && equipment
                        .iter()
                        .all(|(part, v, transform)| (*v != Visibility::Hidden)
                            == (actor.weapon.as_ref() == Some(&part.0))
                            && transform.translation().is_finite())
            );
            assert!(actor.position.distance(smoke.initial.unwrap()) > 0.1);
            smoke.observations.push(serde_json::json!({"pickup_by_actual_mouse_order":true,"weapon":actor.weapon,"equipment_parts":equipment.iter().count(),"position":actor.position.to_array()}));
            smoke.stage = 3;
            smoke.wait = 0;
        }
        3 if t.active.as_deref() == Some("help_combat_2") => {
            assert!(!t.pausing());
            if smoke.camera.is_none() {
                smoke.camera = Some(orbit.target);
            }
            keys.press(KeyCode::Numpad4);
            smoke.wait += 1;
            if smoke.wait == 3 {
                smoke_snapshot(&mut commands, smoke.screenshot.as_deref(), "camera-help");
            }
            if smoke.wait >= 10 {
                assert!(orbit.target.distance(smoke.camera.unwrap()) > 0.1);
                smoke.observations.push(serde_json::json!({"camera_help_does_not_pause":true,"numpad_camera_moved":true}));
                keys.press(KeyCode::Enter);
                smoke.stage = 4;
                smoke.wait = 0;
            }
        }
        4 if t.active.is_none() && smoke.wait == 0 => {
            keys.press(KeyCode::KeyT);
            smoke.wait = 1;
        }
        4 if smoke.combat && t.active.as_deref() == Some("help_combat_3") => {
            assert!(t.pausing());
            keys.press(KeyCode::Enter);
            smoke.stage = 5;
            smoke.wait = 0;
        }
        5 if t.active.is_none() && !t.consumed_input => {
            let c = combat.as_ref().unwrap();
            if smoke.wait == 0 {
                if actor.follow_camera {
                    keys.press(KeyCode::KeyT);
                }
                orbit.target = c.target();
                orbit.distance = 14.0;
            }
            smoke.wait += 1;
            if smoke.wait >= 4 {
                let cursor = camera.0.world_to_viewport(camera.1, c.target()).unwrap();
                assert!(
                    cursor.x > 315.0
                        && cursor.x < window.width()
                        && cursor.y > 0.0
                        && cursor.y < window.height()
                );
                assert!(
                    c.clicked(camera.0, camera.1, cursor, &item),
                    "часовой виден и доступен для приказа"
                );
                smoke_snapshot(&mut commands, smoke.screenshot.as_deref(), "sentry");
                smoke.cursor = Some(cursor);
                mouse.press(MouseButton::Left);
                smoke.observations.push(serde_json::json!({"attack_by_actual_mouse_order":true,"health_before":c.health,"guard_weapon":c.guard.weapon}));
                smoke.stage = 6;
                smoke.wait = 0;
            }
        }
        6 if t.active.as_deref() == Some("help_combat_4") => {
            let c = combat.as_ref().unwrap();
            assert!(!c.alive());
            assert_eq!(c.hits, 2);
            assert!(t.pausing());
            assert!(t.completed.contains("obj_eliminate_sentry1"));
            assert!(t.objectives.contains("obj_eliminate_sentry2"));
            smoke.wait += 1;
            if smoke.wait == 2 {
                smoke_snapshot(
                    &mut commands,
                    smoke.screenshot.as_deref(),
                    "after-combat-help",
                );
            }
            if smoke.wait >= 6 {
                keys.press(KeyCode::Enter);
                smoke.stage = 7;
                smoke.wait = 0;
            }
        }
        7 if smoke.bda && combat.as_ref().unwrap().death_finished() && actor.action.is_none() => {
            smoke.stage = 8;
            smoke.wait = 0;
        }
        8 if smoke.bda => {
            let inv = inventory.as_ref().unwrap();
            assert!(inv.available && !inv.collected);
            if smoke.wait == 0 {
                orbit.target = inv.center.unwrap();
                orbit.pitch = 0.95;
                orbit.distance = 12.0;
            }
            smoke.wait += 1;
            if smoke.wait >= 4 {
                let cursor = camera
                    .0
                    .world_to_viewport(camera.1, inv.center.unwrap())
                    .unwrap();
                assert!(
                    inv.clicked(camera.0, camera.1, cursor, &item),
                    "выпавший BDA виден и доступен"
                );
                smoke_snapshot(&mut commands, smoke.screenshot.as_deref(), "bda-drop");
                smoke.cursor = Some(cursor);
                mouse.press(MouseButton::Left);
                smoke.stage = 9;
            }
        }
        9 if inventory.as_ref().unwrap().collected && actor.pickup_seconds.is_none() => {
            let inv = inventory.as_ref().unwrap();
            assert_eq!(actor.weapon.as_deref(), Some(item.model.as_str()));
            assert_eq!(inv.rounds, inv.gun.magazine);
            assert_eq!(inv.shots, 0);
            assert_eq!(
                t.events
                    .iter()
                    .filter(|e| e.as_str() == "CET_OBJECT_PICKED_UP:BDA")
                    .count(),
                1
            );
            smoke.observations.push(serde_json::json!({"bda_picked_up_by_mouse":true,"auto_equip_false_respected":true,"rounds":inv.rounds}));
            keys.press(KeyCode::Space);
            smoke.stage = 10;
            smoke.wait = 0;
        }
        10 if inventory.as_ref().unwrap().open => {
            smoke.wait += 1;
            if smoke.wait == 2 {
                smoke_snapshot(&mut commands, smoke.screenshot.as_deref(), "inventory");
            }
            if smoke.wait >= 4 {
                for (slot, mut interaction) in &mut slots {
                    if *slot == crate::inventory::Slot::Stored(crate::inventory::Weapon::Bda) {
                        *interaction = ButtonVisualState::Pressed;
                    }
                }
                mouse.press(MouseButton::Left);
                smoke.stage = 11;
            }
        }
        11 if inventory.as_ref().unwrap().drag == Some(crate::inventory::Weapon::Bda) => {
            for (slot, mut interaction) in &mut slots {
                *interaction = if *slot == crate::inventory::Slot::Primary {
                    ButtonVisualState::Hovered
                } else {
                    ButtonVisualState::None
                };
            }
            mouse.press(MouseButton::Left);
            mouse.clear_just_pressed(MouseButton::Left);
            mouse.release(MouseButton::Left);
            smoke.stage = 12;
            smoke.wait = 0;
        }
        12 if inventory.as_ref().unwrap().weapon(&actor) == Some(crate::inventory::Weapon::Bda) => {
            let inv = inventory.as_ref().unwrap();
            assert!(inv.owns_knife && inv.collected);
            assert!(
                equipment
                    .iter()
                    .all(|(part, v, _)| (*v != Visibility::Hidden) == (part.0 == inv.gun.model))
            );
            smoke.wait += 1;
            if smoke.wait == 2 {
                smoke_snapshot(
                    &mut commands,
                    smoke.screenshot.as_deref(),
                    "inventory-equipped",
                );
            }
            if smoke.wait >= 4 {
                keys.press(KeyCode::Space);
                smoke.stage = 13;
                smoke.wait = 0;
                smoke.observations.push(serde_json::json!({"inventory_opened_with_space":true,"drag_to_primary_via_button_interactions":true,"only_bda_visible_in_right_hand":true}));
            }
        }
        13 if !inventory.as_ref().unwrap().open && !inventory.as_ref().unwrap().consumed_input => {
            let c = combat.as_ref().unwrap();
            if smoke.wait == 0 {
                orbit.target = c.guard_at(1).target();
                orbit.pitch = 0.95;
                orbit.distance = 14.0;
            }
            smoke.wait += 1;
            if smoke.wait >= 4 {
                let cursor = camera
                    .0
                    .world_to_viewport(camera.1, c.guard_at(1).target())
                    .unwrap();
                assert_eq!(
                    c.clicked_target(camera.0, camera.1, cursor, &item),
                    Some(1),
                    "второй часовой виден"
                );
                smoke_snapshot(&mut commands, smoke.screenshot.as_deref(), "second-sentry");
                smoke.cursor = Some(cursor);
                mouse.press(MouseButton::Left);
                smoke.stage = 14;
                smoke.wait = 0;
            }
        }
        14 => {
            let inv = inventory.as_ref().unwrap();
            if inv.shots > 0 && smoke.wait == 0 {
                smoke_snapshot(&mut commands, smoke.screenshot.as_deref(), "first-shot");
                smoke.wait = 1;
            }
            if t.active.as_deref() == Some("help_combat_6") {
                assert!(t.completed.contains("obj_eliminate_sentry2"));
                assert!(t.objectives.contains("obj_eliminate_sentry3"));
                assert_eq!(inv.shots, 4);
                assert_eq!(inv.rounds, 9);
                smoke.stage = 15;
                smoke.wait = 0;
            }
        }
        15 => {
            assert!(t.pausing());
            smoke.wait += 1;
            if smoke.wait == 2 {
                smoke_snapshot(
                    &mut commands,
                    smoke.screenshot.as_deref(),
                    "after-shooting-help",
                );
            }
            if smoke.wait >= 6 {
                keys.press(KeyCode::Enter);
                smoke.stage = 16;
                smoke.wait = 0;
            }
        }
        16 if smoke.grenade => {
            smoke.wait += 1;
            if smoke.wait >= 20
                && combat.as_ref().unwrap().guard_at(1).death_finished()
                && actor.action.is_none()
            {
                smoke.stage = 18;
                smoke.wait = 0;
            }
        }
        18 => {
            let g = grenade.as_ref().unwrap();
            assert!(g.available && !g.collected);
            if smoke.wait == 0 {
                orbit.target = g.center.unwrap();
                orbit.pitch = 1.15;
                orbit.distance = 12.0;
            }
            smoke.wait += 1;
            if smoke.wait >= 4 {
                let cursor = camera
                    .0
                    .world_to_viewport(camera.1, g.center.unwrap())
                    .unwrap();
                assert!(
                    g.clicked(camera.0, camera.1, cursor, &item),
                    "выпавшая граната видна"
                );
                smoke_snapshot(&mut commands, smoke.screenshot.as_deref(), "grenade-drop");
                smoke.cursor = Some(cursor);
                mouse.press(MouseButton::Left);
                smoke.stage = 19;
            }
        }
        19 if grenade.as_ref().unwrap().collected && actor.pickup_seconds.is_none() => {
            let g = grenade.as_ref().unwrap();
            let inv = inventory.as_ref().unwrap();
            assert_eq!(g.count, 1);
            assert!(g.secondary);
            assert_eq!(inv.weapon(&actor), Some(crate::inventory::Weapon::Bda));
            assert_eq!(inv.rounds, 9);
            smoke.observations.push(serde_json::json!({"grenade_pickup_by_mouse":true,"secondary_assigned_without_changing_primary":true}));
            keys.press(KeyCode::Space);
            smoke.stage = 20;
            smoke.wait = 0;
        }
        20 if inventory.as_ref().unwrap().open => {
            smoke.wait += 1;
            if smoke.wait == 2 {
                smoke_snapshot(&mut commands, smoke.screenshot.as_deref(), "secondary-slot");
            }
            if smoke.wait >= 4 {
                keys.press(KeyCode::Space);
                smoke.stage = 21;
                smoke.wait = 0;
            }
        }
        21 if !inventory.as_ref().unwrap().open && !inventory.as_ref().unwrap().consumed_input => {
            let c = combat.as_ref().unwrap();
            if smoke.wait == 0 {
                orbit.target = c.guard_at(2).target();
                orbit.pitch = 1.15;
                orbit.distance = 14.0;
            }
            smoke.wait += 1;
            if smoke.wait >= 4 {
                let cursor = camera
                    .0
                    .world_to_viewport(camera.1, c.guard_at(2).target())
                    .unwrap();
                assert_eq!(c.clicked_target(camera.0, camera.1, cursor, &item), Some(2));
                smoke_snapshot(&mut commands, smoke.screenshot.as_deref(), "third-sentry");
                smoke.cursor = Some(cursor);
                mouse.press(MouseButton::Right);
                smoke.stage = 22;
                smoke.wait = 0;
            }
        }
        22 => {
            let g = grenade.as_ref().unwrap();
            if g.flight.is_some() && smoke.wait == 0 {
                assert_eq!(g.count, 0);
                smoke_snapshot(&mut commands, smoke.screenshot.as_deref(), "grenade-flight");
                smoke.wait = 1;
            }
            if t.active.as_deref() == Some("help_combat_8") {
                assert_eq!(g.throws, 1);
                assert_eq!(g.explosions, 1);
                assert!(t.completed.contains("obj_eliminate_sentry3"));
                assert!(t.objectives.contains("obj_eliminate_guards"));
                smoke.stage = 23;
                smoke.wait = 0;
            }
        }
        23 => {
            assert!(t.pausing());
            smoke.wait += 1;
            if smoke.wait == 2 {
                smoke_snapshot(
                    &mut commands,
                    smoke.screenshot.as_deref(),
                    "after-grenade-help",
                );
            }
            if smoke.wait >= 6 {
                keys.press(KeyCode::Enter);
                smoke.stage = 24;
                smoke.wait = 0;
            }
        }
        24 => {
            smoke.wait += 1;
            let c = combat.as_ref().unwrap();
            let g = grenade.as_ref().unwrap();
            let inv = inventory.as_ref().unwrap();
            if smoke.wait >= 20 && c.guard_at(2).death_finished() && actor.action.is_none() {
                assert_eq!(inv.rounds, 9);
                assert_eq!(g.count, 0);
                assert!(!g.secondary);
                for event in [
                    "CET_OBJECT_PICKED_UP:Frag Grenade",
                    "CET_ALL_PLAYERS_INFANTRY_KILLED:AI_Grenade",
                ] {
                    assert_eq!(t.events.iter().filter(|s| s.as_str() == event).count(), 1);
                }
                smoke.observations.push(serde_json::json!({"throw_by_right_mouse_order":true,"third_objective_and_help_completed":true,"primary_ammunition_preserved":true}));
                if smoke.fence {
                    smoke.stage = 26;
                    smoke.wait = 0;
                    return;
                }
                let value = serde_json::json!({"status":"passed","frames":smoke.frames,"window":[window.width(),window.height()],"checks":smoke.observations,"combat":c.report(),"inventory":inv.report(&actor),"grenade":g.report(),"tutorial":t.report(),"gui_drag_input":"project ButtonVisualState injection (Bevy 0.20); world orders use viewport cursor"});
                std::fs::write(
                    &smoke.path,
                    serde_json::to_string_pretty(&value).unwrap() + "\n",
                )
                .unwrap();
                if let Some(path) = &smoke.screenshot {
                    use bevy::render::view::screenshot::{
                        Screenshot, ScreenshotCaptured, save_to_disk,
                    };
                    commands
                        .spawn(Screenshot::primary_window())
                        .observe(save_to_disk(path.clone()))
                        .observe(
                            |_: On<ScreenshotCaptured>, mut exit: MessageWriter<AppExit>| {
                                exit.write(AppExit::Success);
                            },
                        );
                } else {
                    exit.write(AppExit::Success);
                }
                smoke.stage = 25;
            }
        }

        26 if t.active.is_none() && !t.consumed_input => {
            let f = fence.as_ref().unwrap();
            if smoke.wait == 0 {
                orbit.target = f.tool_center;
                orbit.pitch = 1.15;
                orbit.distance = 12.0;
            }
            smoke.wait += 1;
            if smoke.wait >= 4 {
                let cursor = camera.0.world_to_viewport(camera.1, f.tool_center).unwrap();
                assert!(
                    f.tool_clicked(camera.0, camera.1, cursor, &item),
                    "кусачки видны"
                );
                smoke_snapshot(&mut commands, smoke.screenshot.as_deref(), "cutters-ground");
                smoke.cursor = Some(cursor);
                mouse.press(MouseButton::Left);
                smoke.stage = 27;
                smoke.wait = 0;
            }
        }
        27 if t.active.as_deref() == Some("help_combat_9") => {
            let f = fence.as_ref().unwrap();
            let inv = inventory.as_ref().unwrap();
            assert!(f.collected && inv.owns_cutters);
            assert_eq!(inv.weapon(&actor), Some(crate::inventory::Weapon::Cutters));
            assert_eq!(inv.rounds, 9);
            smoke_snapshot(&mut commands, smoke.screenshot.as_deref(), "cutters-help");
            keys.press(KeyCode::Enter);
            smoke.stage = 28;
        }
        28 if t.active.is_none() && actor.pickup_seconds.is_none() && !t.consumed_input => {
            keys.press(KeyCode::Space);
            smoke.stage = 29;
            smoke.wait = 0;
        }
        29 if inventory.as_ref().unwrap().open => {
            smoke.wait += 1;
            if smoke.wait == 2 {
                smoke_snapshot(
                    &mut commands,
                    smoke.screenshot.as_deref(),
                    "cutters-primary",
                );
            }
            if smoke.wait >= 4 {
                keys.press(KeyCode::Space);
                smoke.stage = 30;
                smoke.wait = 0;
            }
        }
        30 if !inventory.as_ref().unwrap().open && !inventory.as_ref().unwrap().consumed_input => {
            let f = fence.as_ref().unwrap();
            if smoke.wait == 0 {
                orbit.target = f.center;
                orbit.pitch = 1.15;
                orbit.distance = 12.0;
            }
            smoke.wait += 1;
            if smoke.wait >= 4 {
                let cursor = camera.0.world_to_viewport(camera.1, f.center).unwrap();
                assert!(
                    f.fence_clicked(camera.0, camera.1, cursor, &item),
                    "сетка видна"
                );
                smoke_snapshot(&mut commands, smoke.screenshot.as_deref(), "closed-fence");
                smoke.cursor = Some(cursor);
                mouse.press(MouseButton::Left);
                smoke.stage = 31;
                smoke.wait = 0;
            }
        }
        31 if t.active.as_deref() == Some("help_combat_10") => {
            let f = fence.as_ref().unwrap();
            assert!(f.cut);
            assert_eq!(f.cuts, 1);
            smoke_snapshot(&mut commands, smoke.screenshot.as_deref(), "cut-fence-help");
            keys.press(KeyCode::Enter);
            smoke.stage = 32;
            smoke.wait = 0;
        }
        32 if t.active.is_none() && !t.consumed_input && !fence.as_ref().unwrap().has_order() => {
            let f = fence.as_ref().unwrap();
            let inv = inventory.as_ref().unwrap();
            assert_eq!(inv.weapon(&actor), Some(crate::inventory::Weapon::Bda));
            assert_eq!(inv.rounds, 9);
            assert!(f.movement_hit(f.crawl_sides[0], f.crawl_sides[1]).is_none());
            let cursor = camera.0.world_to_viewport(camera.1, f.center).unwrap();
            assert!(f.fence_clicked(camera.0, camera.1, cursor, &item));
            smoke_snapshot(&mut commands, smoke.screenshot.as_deref(), "cut-passage");
            smoke.cursor = Some(cursor);
            mouse.press(MouseButton::Left);
            smoke.stage = 33;
            smoke.wait = 0;
        }
        33 => {
            let f = fence.as_ref().unwrap();
            let inv = inventory.as_ref().unwrap();
            if f.crossing() && smoke.wait == 0 {
                smoke_snapshot(&mut commands, smoke.screenshot.as_deref(), "crawling");
                smoke.wait = 1;
            }
            if f.crossings == 1 && !f.has_order() {
                assert_eq!(t.completed.len(), 3);
                assert_eq!(inv.weapon(&actor), Some(crate::inventory::Weapon::Bda));
                for event in [
                    "CET_OBJECT_PICKED_UP:Fence Cutters",
                    "CET_FENCE_CUT:AlStFn-Consertinafence_END",
                ] {
                    assert_eq!(t.events.iter().filter(|s| s.as_str() == event).count(), 1);
                }
                let mut closed_hidden = false;
                let mut tool_hidden = false;
                let mut cut_visible = false;
                let mut ring_hidden = false;
                for (object, cut, ring, v) in &fence_scene {
                    if object.is_some_and(|o| o.0 == f.id) {
                        closed_hidden = *v == Visibility::Hidden;
                    }
                    if object.is_some_and(|o| o.0 == f.tool_id) {
                        tool_hidden = *v == Visibility::Hidden;
                    }
                    if cut.is_some() {
                        cut_visible = *v != Visibility::Hidden;
                    }
                    if ring.is_some() {
                        ring_hidden = *v == Visibility::Hidden;
                    }
                }
                assert!(closed_hidden && tool_hidden && cut_visible && ring_hidden);
                smoke.observations.push(serde_json::json!({"cutters_pickup_by_mouse":true,"cutters_auto_equip":true,"cut_by_left_mouse":true,"crawl_by_second_left_mouse":true,"original_cut_mesh_visible":cut_visible,"closed_fence_hidden":closed_hidden,"picked_tool_hidden":tool_hidden,"primary_restored_without_spending_ammunition":true,"still_three_main_objectives_complete":true}));
                if smoke.smg {
                    smoke.stage = 34;
                    smoke.wait = 0;
                    return;
                }
                let value = serde_json::json!({"status":"passed","frames":smoke.frames,"window":[window.width(),window.height()],"checks":smoke.observations,"combat":combat.as_ref().unwrap().report(),"inventory":inv.report(&actor),"grenade":grenade.as_ref().unwrap().report(),"fence":f.report(),"player_position_metres":actor.position.to_array(),"tutorial":t.report(),"gui_drag_input":"project ButtonVisualState injection (Bevy 0.20); world orders use viewport cursor"});
                std::fs::write(
                    &smoke.path,
                    serde_json::to_string_pretty(&value).unwrap() + "\n",
                )
                .unwrap();
                if let Some(path) = &smoke.screenshot {
                    use bevy::render::view::screenshot::{
                        Screenshot, ScreenshotCaptured, save_to_disk,
                    };
                    commands
                        .spawn(Screenshot::primary_window())
                        .observe(save_to_disk(path.clone()))
                        .observe(
                            |_: On<ScreenshotCaptured>, mut exit: MessageWriter<AppExit>| {
                                exit.write(AppExit::Success);
                            },
                        );
                } else {
                    exit.write(AppExit::Success);
                }
                smoke.stage = 34;
            }
        }
        34 if smoke.smg && t.active.is_none() && !t.consumed_input => {
            if actor.playback.paused {
                keys.press(KeyCode::ControlLeft);
                keys.press(KeyCode::KeyP);
                return;
            }
            keys.press(KeyCode::KeyC);
            smoke.stage = 35;
        }
        35 if actor.posture == crate::stance::Posture::Crouch && !actor.changing_posture() => {
            orbit.target = uzi.as_ref().unwrap().center;
            orbit.pitch = 1.15;
            orbit.distance = 12.0;
            smoke.wait = 0;
            smoke.stage = 36;
        }
        36 => {
            smoke.wait += 1;
            if smoke.wait >= 4 {
                let u = uzi.as_ref().unwrap();
                let cursor = camera.0.world_to_viewport(camera.1, u.center).unwrap();
                assert!(u.clicked(camera.0, camera.1, cursor, &item), "UZI виден");
                smoke.cursor = Some(cursor);
                mouse.press(MouseButton::Left);
                smoke.stage = 37;
            }
        }
        37 if t.active.as_deref() == Some("help_combat_11") => {
            let inv = inventory.as_ref().unwrap();
            assert!(uzi.as_ref().unwrap().collected && inv.owns_uzi);
            assert_eq!(inv.weapon(&actor), Some(crate::inventory::Weapon::Bda));
            assert_eq!((inv.rounds, inv.uzi_rounds), (9, 32));
            smoke_snapshot(&mut commands, smoke.screenshot.as_deref(), "uzi-help");
            keys.press(KeyCode::Enter);
            smoke.stage = 38;
        }
        38 if t.active.is_none() && !t.consumed_input && !uzi.as_ref().unwrap().ordered => {
            keys.press(KeyCode::Space);
            smoke.stage = 39;
        }
        39 if inventory.as_ref().unwrap().open && !inventory.as_ref().unwrap().consumed_input => {
            use crate::inventory::{Slot, Weapon};
            let mut found = false;
            for (slot, mut visual) in &mut slots {
                *visual = if *slot == Slot::Stored(Weapon::Uzi) {
                    found = true;
                    ButtonVisualState::Pressed
                } else {
                    ButtonVisualState::None
                };
            }
            assert!(found);
            mouse.press(MouseButton::Left);
            smoke.stage = 40;
        }
        40 => {
            use crate::inventory::{Slot, Weapon};
            assert_eq!(inventory.as_ref().unwrap().drag, Some(Weapon::Uzi));
            for (slot, mut visual) in &mut slots {
                *visual = if *slot == Slot::Primary {
                    ButtonVisualState::Hovered
                } else {
                    ButtonVisualState::None
                };
            }
            mouse.press(MouseButton::Left);
            mouse.clear_just_pressed(MouseButton::Left);
            mouse.clear_just_pressed(MouseButton::Left);
            mouse.release(MouseButton::Left);
            smoke.stage = 42;
        }
        42 if inventory.as_ref().unwrap().weapon(&actor) == Some(crate::inventory::Weapon::Uzi) => {
            assert!(actor.rifle_equipped);
            assert_eq!(inventory.as_ref().unwrap().rounds, 9);
            smoke_snapshot(&mut commands, smoke.screenshot.as_deref(), "uzi-equipped");
            keys.press(KeyCode::Space);
            smoke.stage = 43;
        }
        43 if !inventory.as_ref().unwrap().open && !inventory.as_ref().unwrap().consumed_input => {
            keys.press(KeyCode::KeyP);
            smoke.stage = 44;
        }
        44 if !actor.changing_posture() => {
            let c = combat.as_ref().unwrap();
            let count = c.smg.iter().filter(|g| !g.alive()).count();
            let expected = if count == 0 {
                crate::stance::Posture::Prone
            } else if count == 1 {
                crate::stance::Posture::Stand
            } else {
                crate::stance::Posture::Crouch
            };
            assert_eq!(actor.posture, expected);
            let index = (3..c.guard_count())
                .find(|&i| c.guard_at(i).alive())
                .unwrap();
            smoke.smg_target = Some(index);
            orbit.target = c.guard_at(index).target();
            orbit.pitch = 1.15;
            orbit.distance = 16.0;
            smoke.wait = 0;
            smoke.stage = 45;
        }
        45 => {
            smoke.wait += 1;
            if smoke.wait >= 4 {
                let c = combat.as_ref().unwrap();
                let index = smoke.smg_target.unwrap();
                let cursor = camera
                    .0
                    .world_to_viewport(camera.1, c.guard_at(index).target())
                    .unwrap();
                assert_eq!(
                    c.clicked_target(camera.0, camera.1, cursor, &item),
                    Some(index)
                );
                smoke.cursor = Some(cursor);
                mouse.press(MouseButton::Left);
                smoke.stage = 46;
            }
        }
        46 => {
            let c = combat.as_ref().unwrap();
            let index = smoke.smg_target.unwrap();
            if t.active.is_some() {
                keys.press(KeyCode::Enter);
                return;
            }
            if !c.guard_at(index).alive()
                && c.guard_at(index).death_finished()
                && !c.has_order()
                && actor.action.is_none()
            {
                let count = c.smg.iter().filter(|g| !g.alive()).count();
                smoke.observations.push(serde_json::json!({"smg_guard_by_mouse":index,"posture":actor.posture.name(),"hits":c.guard_at(index).hits,"completed_objectives":t.completed}));
                if count == 6 {
                    smoke.stage = 47;
                    smoke.wait = 0;
                } else {
                    if count == 1 {
                        keys.press(KeyCode::KeyS);
                    } else if count == 2 {
                        keys.press(KeyCode::KeyC);
                    }
                    smoke.stage = 44;
                }
            }
        }
        47 => {
            if t.active.is_some() {
                keys.press(KeyCode::Enter);
                return;
            }
            smoke.wait += 1;
            if smoke.wait < 20 {
                return;
            }
            let c = combat.as_ref().unwrap();
            let inv = inventory.as_ref().unwrap();
            let u = uzi.as_ref().unwrap();
            assert!(c.smg.iter().all(|g| !g.alive()));
            assert_eq!(inv.uzi_shots, 18);
            assert_eq!(inv.uzi_rounds, 14);
            assert_eq!(inv.rounds, 9);
            assert_eq!(t.completed.len(), 4);
            assert!(t.completed.contains("obj_eliminate_guards"));
            assert!(t.shown.iter().any(|s| s == "help_combat_12"));
            assert_eq!(
                t.events
                    .iter()
                    .filter(|s| s.as_str() == "CET_ALL_PLAYERS_INFANTRY_KILLED:AI_SMG")
                    .count(),
                1
            );
            assert_eq!(
                t.events
                    .iter()
                    .filter(|s| s.as_str() == "CET_OBJECT_PICKED_UP:UZI")
                    .count(),
                1
            );
            let mut hidden = false;
            for (object, _, _, visible) in &fence_scene {
                if object.is_some_and(|o| o.0 == u.id) {
                    hidden = *visible == Visibility::Hidden;
                }
            }
            assert!(hidden);
            assert!(inv.owns_knife && inv.collected && inv.owns_cutters && inv.owns_uzi);
            smoke.observations.push(serde_json::json!({"s_c_p_via_keys":true,"uzi_crouched_pickup":true,"uzi_auto_equip_false":true,"uzi_drag_to_primary_via_buttons":true,"uzi_ground_hidden":hidden,"six_guards":true,"fourth_objective_once":true,"bda_magazine_preserved":true}));
            let value = serde_json::json!({"status":"passed","frames":smoke.frames,"window":[window.width(),window.height()],"checks":smoke.observations,"combat":c.report(),"inventory":inv.report(&actor),"grenade":grenade.as_ref().unwrap().report(),"fence":fence.as_ref().unwrap().report(),"uzi_pickup":u.report(),"posture":actor.posture.name(),"player_position_metres":actor.position.to_array(),"tutorial":t.report(),"gui_drag_input":"ButtonVisualState injection (Bevy 0.20); world orders use viewport cursor"});
            std::fs::write(
                &smoke.path,
                serde_json::to_string_pretty(&value).unwrap() + "\n",
            )
            .unwrap();
            if let Some(path) = &smoke.screenshot {
                use bevy::render::view::screenshot::{
                    Screenshot, ScreenshotCaptured, save_to_disk,
                };
                commands
                    .spawn(Screenshot::primary_window())
                    .observe(save_to_disk(path.clone()))
                    .observe(
                        |_: On<ScreenshotCaptured>, mut exit: MessageWriter<AppExit>| {
                            exit.write(AppExit::Success);
                        },
                    );
            } else {
                exit.write(AppExit::Success);
            }
            smoke.stage = 48;
        }

        16 if !smoke.grenade => {
            let c = combat.as_ref().unwrap();
            let inv = inventory.as_ref().unwrap();
            smoke.wait += 1;
            if smoke.wait >= 20 && c.guard_at(1).death_finished() && actor.action.is_none() {
                let value = serde_json::json!({"status":"passed","frames":smoke.frames,"window":[window.width(),window.height()],"checks":smoke.observations,
                    "combat":c.report(),"pickup":item.report(),"inventory":inv.report(&actor),"tutorial":t.report(),"gui_drag_input":"project ButtonVisualState injection (Bevy 0.20); pickup and target clicks use viewport cursor"});
                std::fs::write(
                    &smoke.path,
                    serde_json::to_string_pretty(&value).unwrap() + "\n",
                )
                .unwrap();
                if let Some(path) = &smoke.screenshot {
                    use bevy::render::view::screenshot::{
                        Screenshot, ScreenshotCaptured, save_to_disk,
                    };
                    commands
                        .spawn(Screenshot::primary_window())
                        .observe(save_to_disk(path.clone()))
                        .observe(
                            |_: On<ScreenshotCaptured>, mut exit: MessageWriter<AppExit>| {
                                exit.write(AppExit::Success);
                            },
                        );
                } else {
                    exit.write(AppExit::Success);
                }
                smoke.stage = 17;
            }
        }
        7 if !smoke.bda => {
            smoke.wait += 1;
            let c = combat.as_ref().unwrap();
            if smoke.wait >= 20 && c.death_finished() && actor.action.is_none() {
                let report = serde_json::json!({"status":"passed","frames":smoke.frames,"window":[window.width(),window.height()],"checks":smoke.observations,
                    "combat":c.report(),"pickup":item.report(),"tutorial":t.report(),"equipment_parts":equipment.iter().count()});
                std::fs::write(
                    &smoke.path,
                    serde_json::to_string_pretty(&report).unwrap() + "\n",
                )
                .unwrap();
                if let Some(path) = &smoke.screenshot {
                    use bevy::render::view::screenshot::{
                        Screenshot, ScreenshotCaptured, save_to_disk,
                    };
                    commands
                        .spawn(Screenshot::primary_window())
                        .observe(save_to_disk(path.clone()))
                        .observe(
                            |_: On<ScreenshotCaptured>, mut exit: MessageWriter<AppExit>| {
                                exit.write(AppExit::Success);
                            },
                        );
                } else {
                    exit.write(AppExit::Success);
                }
                smoke.stage = 8;
            }
        }
        4 if t.active.as_deref() == Some("help_combat_3") => {
            assert!(t.pausing());
            smoke.wait += 1;
            if smoke.wait >= 20 {
                assert_eq!(t.shown, ["help_combat_1", "help_combat_2", "help_combat_3"]);
                let report = serde_json::json!({"status":"passed","frames":smoke.frames,"window":[window.width(),window.height()],"checks":smoke.observations,
                    "pickup":item.report(),"tutorial":t.report(),"equipped_world_positions":equipment.iter().filter(|(part,_,_)| actor.weapon.as_ref()==Some(&part.0)).map(|(_,_,t)|t.translation().to_array()).collect::<Vec<_>>()});
                std::fs::write(
                    &smoke.path,
                    serde_json::to_string_pretty(&report).unwrap() + "\n",
                )
                .unwrap();
                if let Some(path) = &smoke.screenshot {
                    use bevy::render::view::screenshot::{
                        Screenshot, ScreenshotCaptured, save_to_disk,
                    };
                    commands
                        .spawn(Screenshot::primary_window())
                        .observe(save_to_disk(path.clone()))
                        .observe(
                            |_: On<ScreenshotCaptured>, mut exit: MessageWriter<AppExit>| {
                                exit.write(AppExit::Success);
                            },
                        );
                } else {
                    exit.write(AppExit::Success);
                }
                smoke.stage = 5;
            }
        }
        _ => {}
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    pub(crate) fn fixture() -> (
        Pickup,
        player::Player,
        Tutorial,
        shadow_formats::TerrainGrid,
    ) {
        let mut actor = player::tests::player();
        actor.clips.push(Clip {
            name: "pickup".into(),
            fps: 20.0,
            frames: (0..21)
                .map(|_| crate::motion::Pose {
                    bones: vec![],
                    root: Vec3::ZERO,
                })
                .collect(),
        });
        let item = Pickup {
            id: [1, 2],
            name: "knife".into(),
            model: "knife.smf".into(),
            position: Vec3::new(1.5, 0.0, -0.5),
            center: Vec3::ZERO,
            ordered: false,
            collected: false,
            callback_seconds: 0.3,
            source: serde_json::Value::Null,
            geometry: vec![],
        };
        (
            item,
            actor,
            crate::tutorial::tests::tutorial(),
            player::tests::grid(),
        )
    }
    #[test]
    fn moving_vehicle_geometry_clears_old_occlusion_and_home_restores_it() {
        let (mut geometry, _, _, _) = fixture();
        let id = [1, -767];
        geometry.geometry.push(Geometry {
            part: String::new(),
            id,
            opaque: true,
            triangles: vec![[
                Vec3::new(-1.0, 0.0, -2.0),
                Vec3::new(1.0, 0.0, -2.0),
                Vec3::new(0.0, 3.0, -2.0),
            ]],
        });
        let from = Vec3::new(0.0, 1.0, 0.0);
        let to = Vec3::new(0.0, 1.0, -4.0);
        let shift = Vec3::X * 10.0;
        assert!(!geometry.clear_line(from, to));
        geometry.move_object(id, Mat4::from_translation(shift));
        assert!(geometry.clear_line(from, to));
        assert!(!geometry.clear_line(from + shift, to + shift));
        geometry.move_object(id, Mat4::from_translation(-shift));
        assert!(!geometry.clear_line(from, to));
        assert!(geometry.clear_line(from + shift, to + shift));
    }
    #[test]
    fn approach_then_callback_equips_once_and_animation_finishes_without_looping() {
        let (mut item, mut actor, mut tutorial, grid) = fixture();
        assert!(item.order(&mut actor, &grid));
        for _ in 0..100 {
            actor.step(&ButtonInput::default(), 0.02, &grid);
            item.update(&mut actor, &mut tutorial);
        }
        assert!(item.collected);
        assert_eq!(actor.weapon.as_deref(), Some("knife.smf"));
        assert!(actor.position.x > 0.5 && actor.position.x <= 1.5);
        assert!(actor.pickup_seconds.is_none());
        assert_eq!(actor.playback.selected, 0);
        assert_eq!(
            tutorial
                .events
                .iter()
                .filter(|e| e.ends_with(":knife"))
                .count(),
            1
        );
        assert!(!item.order(&mut actor, &grid));
    }
    #[test]
    fn pause_halt_and_replacement_do_not_consume_item_before_callback() {
        let (mut item, mut actor, mut tutorial, grid) = fixture();
        item.position = actor.position;
        assert!(item.order(&mut actor, &grid));
        item.update(&mut actor, &mut tutorial);
        assert_eq!(actor.pickup_seconds, Some(0.0));
        actor.step(&ButtonInput::default(), 0.2, &grid);
        item.update(&mut actor, &mut tutorial);
        assert!(!item.collected);
        actor.playback.paused = true;
        actor.step(&ButtonInput::default(), 1.0, &grid);
        item.update(&mut actor, &mut tutorial);
        assert!(!item.collected);
        actor.playback.paused = false;
        let mut keys = ButtonInput::default();
        keys.press(KeyCode::KeyH);
        actor.step(&keys, 0.01, &grid);
        item.cancel();
        item.update(&mut actor, &mut tutorial);
        assert!(!item.collected);
        assert!(actor.pickup_seconds.is_none());
        assert!(actor.weapon.is_none());
        item.order(&mut actor, &grid);
        item.update(&mut actor, &mut tutorial);
        actor.order(&grid, Vec3::new(1.9, 0.0, -1.9), true);
        item.cancel();
        for _ in 0..60 {
            actor.step(&ButtonInput::default(), 0.02, &grid);
            item.update(&mut actor, &mut tutorial);
        }
        assert!(!item.collected);
        assert!(actor.weapon.is_none());
    }
    #[test]
    fn actual_home_system_restores_item_and_restarts_script() {
        let (mut item, mut actor, tutorial, _) = fixture();
        item.collected = true;
        actor.weapon = Some(item.model.clone());
        let mut app = App::new();
        app.init_resource::<ButtonInput<KeyCode>>()
            .insert_resource(item)
            .insert_resource(actor)
            .insert_resource(tutorial)
            .add_systems(Update, input);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Home);
        app.update();
        assert!(!app.world().resource::<Pickup>().collected);
        assert!(app.world().resource::<player::Player>().weapon.is_none());
        assert_eq!(app.world().resource::<Tutorial>().clock, 0.0);
    }
    #[test]
    fn mesh_ray_is_double_sided_and_rejects_misses_and_invalid_inputs() {
        let triangle = [Vec3::ZERO, Vec3::X, Vec3::Z];
        assert_eq!(
            ray_triangle(Vec3::new(0.2, 1.0, 0.2), -Vec3::Y, triangle),
            Some(1.0)
        );
        assert_eq!(
            ray_triangle(Vec3::new(0.2, -1.0, 0.2), Vec3::Y, triangle),
            Some(1.0)
        );
        assert!(ray_triangle(Vec3::new(2.0, 1.0, 2.0), -Vec3::Y, triangle).is_none());
        assert!(ray_triangle(Vec3::splat(f32::NAN), Vec3::Y, triangle).is_none());
        assert!(ray_triangle(Vec3::ZERO, Vec3::ZERO, triangle).is_none());
    }
    pub(crate) fn add_wall(p: &mut Pickup) {
        p.geometry.push(Geometry {
            part: String::new(),
            id: [9, 9],
            opaque: true,
            triangles: vec![
                [
                    Vec3::new(0.9, 0.0, -2.0),
                    Vec3::new(0.9, 5.0, -2.0),
                    Vec3::new(0.9, 5.0, 1.0),
                ],
                [
                    Vec3::new(0.9, 0.0, -2.0),
                    Vec3::new(0.9, 5.0, 1.0),
                    Vec3::new(0.9, 0.0, 1.0),
                ],
            ],
        });
    }
    #[test]
    fn early_cutters_in_hand_do_not_prevent_picking_up_the_training_knife() {
        let (mut item, mut p, mut t, grid) = fixture();
        p.weapon = Some("cutters".into());
        p.position = item.position;
        assert!(item.order(&mut p, &grid));
        item.update(&mut p, &mut t);
        p.step(&ButtonInput::default(), 0.31, &grid);
        item.update(&mut p, &mut t);
        assert!(item.collected);
        assert_eq!(p.weapon.as_deref(), Some(item.model.as_str()));
        assert!(!item.order(&mut p, &grid));
    }
    #[test]
    fn selected_item_does_not_occlude_itself_but_other_solid_objects_still_do() {
        let (mut item, _, _, _) = fixture();
        let tool = [1, 99];
        let triangle = [
            Vec3::new(-1.0, -1.0, 0.0),
            Vec3::new(1.0, -1.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
        ];
        item.geometry.push(Geometry {
            part: String::new(),
            id: tool,
            opaque: true,
            triangles: vec![triangle],
        });
        let (origin, target) = (Vec3::Z, Vec3::NEG_Z);
        assert!(!item.clear_line(origin, target));
        assert!(item.clear_line_ignoring(origin, target, Some(tool)));
        item.geometry.push(Geometry {
            part: String::new(),
            id: [1, 100],
            opaque: true,
            triangles: vec![triangle.map(|v| v + Vec3::Z * 0.5)],
        });
        assert!(!item.clear_line_ignoring(origin, target, Some(tool)));
    }
    #[test]
    fn solid_geometry_occludes_small_item_selection() {
        let mut p = Pickup {
            id: [1, 2],
            name: "Knife".into(),
            model: "knife.smf".into(),
            position: Vec3::ZERO,
            center: Vec3::new(0.2, -1.0, 0.2),
            ordered: false,
            collected: false,
            callback_seconds: 0.3,
            source: serde_json::Value::Null,
            geometry: vec![Geometry {
                part: String::new(),
                id: [1, 3],
                opaque: true,
                triangles: vec![[Vec3::ZERO, Vec3::X, Vec3::Z]],
            }],
        };
        assert!(!p.visible_from(Vec3::new(0.2, 1.0, 0.2)));
        p.geometry[0].opaque = false;
        assert!(p.visible_from(Vec3::new(0.2, 1.0, 0.2)));
    }
    #[test]
    fn moving_leaf_updates_ray_geometry_without_moving_frame_or_other_objects() {
        let (mut item, _, _, _) = fixture();
        let triangle = [Vec3::ZERO, Vec3::X, Vec3::Y];
        for (id, part) in [([7, 8], "leaf"), ([7, 8], "frame"), ([7, 9], "leaf")] {
            item.geometry.push(Geometry {
                id,
                part: part.into(),
                opaque: true,
                triangles: vec![triangle],
            });
        }
        let before: Vec<_> = item.geometry.iter().map(|g| g.triangles.clone()).collect();
        let delta = Mat4::from_translation(Vec3::new(2.0, 0.0, 3.0));
        item.move_parts([7, 8], &["leaf".into()], delta);
        for (g, old) in item.geometry.iter().zip(&before) {
            assert_eq!(
                g.triangles,
                if g.id == [7, 8] && g.part == "leaf" {
                    vec![triangle.map(|v| delta.transform_point3(v))]
                } else {
                    old.clone()
                }
            );
        }
        item.move_parts([7, 8], &["leaf".into()], delta.inverse());
        assert_eq!(
            item.geometry
                .iter()
                .map(|g| g.triangles.clone())
                .collect::<Vec<_>>(),
            before
        );
    }
}
