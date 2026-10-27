//! Один управляемый боец. Позиция — в метрах, BMF — в исходных единицах.
use crate::{
    map_scene::WORLD_SCALE,
    motion::{Clip, Playback, to_world_delta},
    rig,
};
use bevy::prelude::*;
use shadow_formats::{
    AssetCatalog, BmfMotion, CharacterProfile, MissionDefinition, MtfObject, TerrainGrid,
    config_lines,
};
use std::error::Error;

const MOTIONS: [&str; 4] = [
    "motions/bipedal_stand.bmf",
    "motions/bipedal_stand_walk_80.bmf",
    "motions/bipedal_stand_run.bmf",
    "motions/bipedal_stand_wave.bmf",
];

#[derive(Resource)]
pub struct Player {
    pub name: String,
    pub display_name: String,
    pub source: serde_json::Value,
    pub rig: Option<rig::Rig>,
    pub clips: Vec<Clip>,
    pub playback: Playback,
    pub position: Vec3,
    pub heading: f32,
    pub follow_camera: bool,
    pub selected: bool,
    pub destination: Option<Vec3>,
    pub running: bool,
    pub prepared_weapon: Option<String>,
    pub weapon: Option<String>,
    pub pickup_seconds: Option<f32>,
    pub action: Option<(usize, f32)>,
    pub equipment_parts: Vec<(String, std::ops::Range<usize>)>,
    pub posture: crate::stance::Posture,
    pub postures: Option<crate::stance::Clips>,
    pub posture_transition: Option<(usize, f32, crate::stance::Posture)>,
    pub rifle_equipped: bool,
    pub carry_movement: Option<[usize; 3]>,
    basis: Transform,
    focus: Vec3,
    spawn: Vec3,
    spawn_heading: f32,
    gesture: bool,
}
#[derive(Component)]
pub struct SceneEntity;
#[derive(Component)]
pub struct Bone(pub usize);
#[derive(Component)]
pub struct Root;
#[derive(Component)]
pub struct Status;
#[derive(Component)]
pub struct Marker(pub bool); // true — выбранный боец; false — точка приказа.

pub fn prepare(
    catalog: &AssetCatalog,
    grid: &TerrainGrid,
    actor: &MtfObject,
) -> Result<Player, Box<dyn Error>> {
    prepare_actor(catalog, grid, actor, false)
}
pub(crate) fn prepare_actor(
    catalog: &AssetCatalog,
    grid: &TerrainGrid,
    actor: &MtfObject,
    equipment: bool,
) -> Result<Player, Box<dyn Error>> {
    let (profile_path, profile) = profile_for(catalog, &actor.name)?;
    let appearance = profile.initial_body();
    if !appearance.extra.is_empty() || (!equipment && !profile.equipment.is_empty()) {
        return Err("дополнительные части внешности и стартовая экипировка пока не поддерживаются в учебной сцене".into());
    }
    if !appearance.body_model.eq_ignore_ascii_case(&actor.model) {
        return Err("модель актёра MTF отличается от BODY_INITIAL профиля".into());
    }
    let body = model_path(catalog, &appearance.body_model)?;
    let head = model_path(catalog, &appearance.head_model)?;
    let pack = appearance
        .pack
        .as_ref()
        .map(|(model, texture)| model_path(catalog, model).map(|path| (path, texture.as_str())))
        .transpose()?;
    let mut rig = rig::prepare_appearance(
        catalog,
        &rig::Appearance {
            body: &body,
            head: &head,
            body_texture: &appearance.body_map,
            head_texture: Some(&appearance.head_map),
            pack: pack
                .as_ref()
                .map(|(path, texture)| (path.as_str(), *texture)),
        },
        false,
    )?;
    let [x, y, z] = actor.position.ok_or("неопределённая позиция актёра MTF")?;
    let angles = actor.rotation.ok_or("неопределённый поворот актёра MTF")?;
    if angles[0].abs() > 1e-5 || angles[1].abs() > 1e-5 {
        return Err("наклон начального актёра пока не поддерживается".into());
    }
    let mut spawn = Vec3::new(x, z, -y) * WORLD_SCALE;
    let heading = angles[2];
    // Витрина нормирует человека до трёх метров. На карте — масштаб всех ресурсов 0.01.
    let factor = WORLD_SCALE / rig.root.scale.x;
    rig.root.translation *= factor;
    rig.root.scale = Vec3::splat(WORLD_SCALE);
    rig.focus *= factor;
    spawn.y = surface(grid, spawn).ok_or("точка появления вне рельефа")?;
    let clips = MOTIONS
        .iter()
        .map(|path| Clip::new(&BmfMotion::from_bytes(&catalog.read(path)?)?, &rig.bones))
        .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
    Ok(Player {
        name: profile.id.clone(),
        display_name: profile.display_name(),
        source: serde_json::json!({"actor_id":actor.id,"source_position":actor.position,"source_rotation":actor.rotation,
            "profile":profile_path,"profile_source":catalog.source_label(&profile_path)?,
            "initial_body":appearance.kind,"body":body,"head":head,"body_texture":appearance.body_map,
            "head_texture":appearance.head_map,"pack":pack.as_ref().map(|(p,_)|p),
            "textures":rig.images.keys().collect::<Vec<_>>(),"mission_execution":false}),
        playback: Playback::new(&clips, 0, 0.0, false, true),
        basis: rig.root,
        focus: rig.focus,
        rig: Some(rig),
        clips,
        position: spawn,
        spawn,
        heading,
        spawn_heading: heading,
        follow_camera: false,
        selected: true,
        destination: None,
        running: false,
        prepared_weapon: None,
        weapon: None,
        pickup_seconds: None,
        action: None,
        equipment_parts: Vec::new(),
        posture: crate::stance::Posture::Stand,
        postures: None,
        posture_transition: None,
        rifle_equipped: false,
        carry_movement: None,
        gesture: false,
    })
}

/// ID менеджера игрока из сценария определяет начальную команду, а не порядок объектов MTF.
pub fn initial_actor<'a>(
    mission: &MissionDefinition,
    actors: &'a [MtfObject],
) -> Result<&'a MtfObject, Box<dyn Error>> {
    let ids: Vec<_> = mission
        .players
        .iter()
        .filter(|p| !p.multiplayer && p.kind.eq_ignore_ascii_case("LOCAL_NONSCRIPTED"))
        .map(|p| p.id)
        .collect();
    let mut candidates = actors.iter().filter(|a| ids.contains(&a.id[0]));
    let actor = candidates
        .next()
        .ok_or("в MTF нет бойца начальной команды")?;
    if candidates.next().is_some() {
        return Err("учебная сцена пока поддерживает одного начального бойца".into());
    }
    Ok(actor)
}

pub(crate) fn profile_for(
    catalog: &AssetCatalog,
    name: &str,
) -> Result<(String, CharacterProfile), Box<dyn Error>> {
    let mut found = None;
    for path in catalog
        .paths()
        .filter(|p| p.starts_with("config/character_profiles/") && p.ends_with(".txt"))
    {
        let bytes = catalog.read(path)?;
        if profile_identifier(&bytes)?
            .as_deref()
            .is_some_and(|id| id.eq_ignore_ascii_case(name))
        {
            if found.is_some() {
                return Err(format!("неоднозначный профиль персонажа {name}").into());
            }
            found = Some((path.to_owned(), CharacterProfile::from_bytes(&bytes)?));
        }
    }
    found.ok_or_else(|| format!("не найден профиль персонажа {name}").into())
}
/// Индекс зависит только от CHARACTER. Биографии других персонажей не относятся
/// к загрузке выбранного бойца (в двух исходных DOSSIER_LINE пропущены кавычки).
fn profile_identifier(bytes: &[u8]) -> Result<Option<String>, Box<dyn Error>> {
    if bytes.len() > 16 * 1024 * 1024 || bytes.contains(&0) {
        return Err("профиль слишком велик или содержит NUL".into());
    }
    let mut id = None;
    for (index, line) in bytes.split(|b| *b == b'\n').enumerate() {
        if line.len() > 16384 {
            return Err(format!("профиль: слишком длинная строка {}", index + 1).into());
        }
        let key = line
            .split(|b| b.is_ascii_whitespace())
            .find(|word| !word.is_empty());
        if key != Some(b"CHARACTER".as_slice()) {
            continue;
        }
        let values = config_lines(line)
            .map_err(|e| format!("заголовок профиля, строка {}: {e}", index + 1))?;
        let words = &values[0].words;
        if words.len() != 2 || words[1].is_empty() || id.is_some() {
            return Err(format!(
                "профиль: неверный или повторный CHARACTER, строка {}",
                index + 1
            )
            .into());
        }
        id = Some(words[1].clone());
    }
    Ok(id)
}
pub(crate) fn model_path(catalog: &AssetCatalog, name: &str) -> Result<String, Box<dyn Error>> {
    let filename = format!("{}.smf", name.to_ascii_lowercase());
    let mut paths = catalog
        .paths()
        .filter(|p| p.rsplit('/').next() == Some(filename.as_str()));
    let path = paths
        .next()
        .ok_or_else(|| format!("не найдена модель профиля {name}"))?;
    if paths.next().is_some() {
        return Err(format!("неоднозначная модель профиля {name}").into());
    }
    Ok(path.to_owned())
}

pub fn surface(grid: &TerrainGrid, position: Vec3) -> Option<f32> {
    grid.height_at(position.x / WORLD_SCALE, -position.z / WORLD_SCALE)
        .map(|h| h * WORLD_SCALE)
}

impl Player {
    pub fn request_posture(&mut self, target: crate::stance::Posture) -> bool {
        if !self.selected
            || self.playback.paused
            || self.posture_transition.is_some()
            || self.carry_movement.is_some()
            || target == self.posture
        {
            return false;
        }
        let Some(index) = self
            .postures
            .as_ref()
            .and_then(|c| c.transition[self.posture.index()][target.index()])
        else {
            return false;
        };
        self.action = None;
        self.pickup_seconds = None;
        self.gesture = false;
        self.posture_transition = Some((index, 0.0, target));
        self.playback.select(&self.clips, index);
        self.playback.seconds = 0.0;
        self.playback.pose = self.clips[index].sample(0.0);
        true
    }
    pub fn changing_posture(&self) -> bool {
        self.posture_transition.is_some()
    }
    pub fn idle_clip(&self) -> usize {
        if let Some(clips) = self.carry_movement {
            return clips[0];
        }
        self.postures
            .as_ref()
            .map(|c| c.idle[self.posture.index()])
            .unwrap_or(0)
    }
    pub fn pickup_clip(&self) -> usize {
        self.postures
            .as_ref()
            .map(|c| c.pickup[self.posture.index()].0)
            .unwrap_or(4)
    }
    pub fn pickup_callback(&self, fallback: f32) -> f32 {
        self.postures
            .as_ref()
            .map(|c| c.pickup[self.posture.index()].1)
            .unwrap_or(fallback)
    }
    pub fn pickup_duration(&self) -> f32 {
        self.clips[self.pickup_clip()].duration()
    }
    pub fn fire_clips(&self, rifle: bool) -> Option<(usize, usize)> {
        self.postures.as_ref().map(|c| {
            if rifle {
                c.rifle[self.posture.index()]
            } else {
                c.pistol[self.posture.index()]
            }
        })
    }
    pub fn order(&mut self, grid: &TerrainGrid, mut point: Vec3, running: bool) -> bool {
        let Some(height) = surface(grid, point) else {
            return false;
        };
        point.y = height;
        self.pickup_seconds = None;
        self.action = None;
        self.destination = Some(point);
        self.running = running;
        self.gesture = false;
        true
    }
    pub fn begin_action(&mut self, index: usize) {
        self.destination = None;
        self.pickup_seconds = None;
        self.action = Some((index, 0.0));
        self.playback.selected = index;
        self.playback.seconds = 0.0;
        self.playback.pose = self.clips[index].sample(0.0);
    }
    pub fn begin_pickup(&mut self) {
        self.action = None;
        self.pickup_seconds = Some(0.0);
        let index = self.pickup_clip();
        self.playback.select(&self.clips, index);
        self.playback.seconds = 0.0;
        self.playback.pose = self.clips[index].sample(0.0);
    }
    pub fn finish_pickup(&mut self) {
        self.pickup_seconds = None;
        self.action = None;
        self.playback.select(&self.clips, self.idle_clip());
    }
    pub fn preview_action(&mut self, action: &str, grid: &TerrainGrid) {
        if action == "wave" {
            self.gesture = true;
        }
        if action == "walk" || action == "run" {
            let point = Vec3::new(
                self.position.x,
                0.0,
                -((grid.config.height - 1) as f32) * grid.config.edge * WORLD_SCALE,
            );
            self.order(grid, point, action == "run");
        }
    }
    pub fn transform(&self) -> Transform {
        let turn = Quat::from_rotation_y(self.heading);
        Transform {
            translation: self.position + turn * self.basis.translation,
            rotation: turn * self.basis.rotation,
            scale: self.basis.scale,
        }
    }
    pub fn camera_target(&self) -> Vec3 {
        self.position + self.focus
    }

    /// Вращаем только новую дельту движения: поворот не меняет пройденный путь.
    pub fn step(&mut self, input: &ButtonInput<KeyCode>, dt: f32, grid: &TerrainGrid) {
        if !dt.is_finite() || dt < 0.0 {
            return;
        }
        if input.just_pressed(KeyCode::Home) {
            self.posture = crate::stance::Posture::Stand;
            self.posture_transition = None;
            self.rifle_equipped = false;
            self.carry_movement = None;
            self.weapon = None;
            self.pickup_seconds = None;
            self.action = None;
            self.position = self.spawn;
            self.heading = self.spawn_heading;
            self.gesture = false;
            self.destination = None;
            self.playback.select(&self.clips, 0);
            self.playback.reset(&self.clips);
        }
        let ctrl = input.pressed(KeyCode::ControlLeft) || input.pressed(KeyCode::ControlRight);
        if input.just_pressed(KeyCode::Escape) || (ctrl && input.just_pressed(KeyCode::KeyP)) {
            self.playback.paused = !self.playback.paused;
        }
        if input.just_pressed(KeyCode::KeyT) && self.selected {
            self.follow_camera = !self.follow_camera;
        }
        if input.just_pressed(KeyCode::Digit1) || input.just_pressed(KeyCode::F11) {
            self.selected = true;
        }
        if input.just_pressed(KeyCode::KeyH) && self.selected {
            self.pickup_seconds = None;
            self.action = None;
            self.destination = None;
            self.gesture = false;
            if !self.changing_posture() {
                self.playback.select(&self.clips, self.idle_clip());
            }
        }
        if let Some((index, seconds, target)) = self.posture_transition {
            if !self.playback.paused {
                let seconds = (seconds + dt).min(self.clips[index].duration());
                self.posture_transition = Some((index, seconds, target));
                self.playback.selected = index;
                self.playback.seconds = seconds;
                self.playback.pose = self.clips[index].sample(seconds);
                if seconds >= self.clips[index].duration() {
                    self.posture = target;
                    self.posture_transition = None;
                    let idle = self.idle_clip();
                    self.playback.select(&self.clips, idle);
                    self.playback.pose = self.clips[idle].sample(0.0);
                }
            }
            return;
        }
        if let Some((index, seconds)) = self.action {
            if !self.playback.paused {
                let seconds = (seconds + dt).min(self.clips[index].duration());
                self.action = Some((index, seconds));
                self.playback.selected = index;
                self.playback.seconds = seconds;
                self.playback.pose = self.clips[index].sample(seconds);
            }
            return;
        }
        if let Some(seconds) = self.pickup_seconds {
            if !self.playback.paused {
                let index = self.pickup_clip();
                let seconds = (seconds + dt).min(self.clips[index].duration());
                self.pickup_seconds = Some(seconds);
                self.playback.selected = index;
                self.playback.seconds = seconds;
                self.playback.pose = self.clips[index].sample(seconds);
            }
            return;
        }
        let remaining = self
            .destination
            .map(|p| Vec3::new(p.x - self.position.x, 0.0, p.z - self.position.z))
            .unwrap_or(Vec3::ZERO);
        let direction = remaining.normalize_or_zero();
        if !self.playback.paused {
            let selected = if direction != Vec3::ZERO {
                self.heading = (-direction.x).atan2(-direction.z);
                self.gesture = false;
                if let Some(clips) = self.carry_movement {
                    clips[1 + usize::from(self.running)]
                } else if let Some(c) = &self.postures {
                    if self.posture == crate::stance::Posture::Stand && self.rifle_equipped {
                        c.rifle_move[usize::from(self.running)]
                    } else if self.running {
                        c.run[self.posture.index()]
                    } else {
                        c.walk[self.posture.index()]
                    }
                } else if self.running {
                    2
                } else {
                    1
                }
            } else if self.gesture {
                3
            } else {
                self.idle_clip()
            };
            self.playback.select(&self.clips, selected);
        }
        let previous = self.playback.travel;
        for (key, direction) in [(KeyCode::BracketLeft, -1.0), (KeyCode::BracketRight, 1.0)] {
            if input.just_pressed(key) {
                self.playback.step_frame(&self.clips, direction);
            }
        }
        self.playback.tick(&self.clips, dt);
        let turn = Quat::from_rotation_y(self.heading);
        let delta = turn * to_world_delta(&self.basis, self.playback.travel - previous);
        let c = &grid.config;
        let horizontal = Vec3::new(delta.x, 0.0, delta.z);
        let horizontal = if horizontal.length() >= remaining.length() {
            remaining
        } else {
            horizontal
        };
        let mut next = self.position + horizontal;
        next.x = next
            .x
            .clamp(0.0, (c.width - 1) as f32 * c.edge * WORLD_SCALE);
        next.z = next
            .z
            .clamp(-((c.height - 1) as f32) * c.edge * WORLD_SCALE, 0.0);
        if let Some(height) = surface(grid, next) {
            next.y = height;
            self.position = next;
        }
        if self.destination.is_some_and(|p| {
            Vec2::new(p.x - self.position.x, p.z - self.position.z).length() < 0.001
        }) {
            self.destination = None;
            self.playback.select(&self.clips, self.idle_clip());
        }
    }

    pub fn report(&self) -> serde_json::Value {
        let rig = self
            .rig
            .as_ref()
            .expect("report is written before spawning meshes");
        let clip = &self.clips[self.playback.selected];
        serde_json::json!({
            "name":self.name,"display_name":self.display_name,"source":self.source,
            "position_metres":self.position.to_array(), "spawn_metres":self.spawn.to_array(),
            "heading_radians":self.heading, "world_scale":self.basis.scale.to_array(),
            "selected":self.selected,"destination_metres":self.destination.map(|p|p.to_array()),"running":self.running,
            "posture":self.posture.name(),"posture_transition":self.posture_transition.map(|(i,s,p)|(i,s,p.name())),"posture_source":self.postures.as_ref().map(|c|&c.source),
            "body_height_metres":self.focus.y * 2.0, "root_translation_metres":self.transform().translation.to_array(),
            "motion":clip.name, "seconds":self.playback.seconds,"duration":clip.duration(),
            "bones":rig.bones.len(), "parts":rig.parts.len(),"vertices":rig.vertices,"triangles":rig.triangles,
            "armed":rig.armed || self.weapon.is_some(),"weapon":self.weapon,"prepared_weapon":self.prepared_weapon,"pickup_seconds":self.pickup_seconds,"action_animation":self.action,"weapon_bone_id":self.weapon.is_some().then_some(rig::RIGHT_HAND),
            "pack_bone_id":rig.pack_bone_id,
            "weapon_world_vertices":if self.weapon.is_some() {self.equipment_parts.iter().find(|(path,_)| self.weapon.as_ref()==Some(path)).map(|(_,range)| rig::accessory_world_vertices(rig, &self.playback.pose.bones, self.transform(),range.clone())).unwrap_or_default()} else {Vec::new()},
            "obstacle_collisions":false,"navigation":false,"foot_ik":false
        })
    }
}

#[allow(clippy::too_many_arguments)]
pub fn setup(
    mut commands: Commands,
    player: Option<ResMut<Player>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut fonts: ResMut<Assets<Font>>,
    camera: Single<Entity, With<Camera3d>>,
    mut armor: Option<ResMut<crate::armor::Armor>>,
) {
    let Some(mut player) = player else {
        return;
    };
    let rig = player.rig.take().expect("startup runs once");
    let ring = meshes.add(Torus::new(0.48, 0.52));
    let ring_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.3, 1.0, 0.4),
        unlit: true,
        ..default()
    });
    for selected in [true, false] {
        commands.spawn((
            SceneEntity,
            Marker(selected),
            Mesh3d(ring.clone()),
            MeshMaterial3d(ring_material.clone()),
            Transform::default(),
            Visibility::Hidden,
        ));
    }
    let root = commands
        .spawn((
            SceneEntity,
            Root,
            Name::new(player.display_name.clone()),
            player.transform(),
            Visibility::default(),
        ))
        .id();
    let bones = rig
        .bones
        .iter()
        .enumerate()
        .map(|(i, b)| {
            commands
                .spawn((
                    Bone(i),
                    Name::new(b.name.clone()),
                    player.playback.pose.bones[i],
                    Visibility::default(),
                ))
                .id()
        })
        .collect::<Vec<_>>();
    for (i, b) in rig.bones.iter().enumerate() {
        commands
            .entity(bones[i])
            .insert(ChildOf(b.parent.map(|p| bones[p]).unwrap_or(root)));
    }
    let textures = rig
        .images
        .into_iter()
        .map(|(name, image)| (name, (images.add(image.image), image.alpha_mode)))
        .collect::<std::collections::BTreeMap<_, _>>();
    if let Some(armor) = &mut armor {
        let uniform = textures[&armor.uniform_path].clone();
        let flak = textures[&armor.flak_path].clone();
        armor.bind_textures(uniform, flak);
    }
    let equipment_parts = &player.equipment_parts;
    for (index, part) in rig.parts.into_iter().enumerate() {
        let mut entity = commands.spawn((
            Name::new(part.name),
            Mesh3d(meshes.add(part.mesh)),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color_texture: Some(textures[&part.texture].0.clone()),
                alpha_mode: textures[&part.texture].1,
                perceptual_roughness: 0.95,
                cull_mode: None,
                ..default()
            })),
            Transform::default(),
            ChildOf(bones[part.bone]),
        ));
        if armor.as_ref().is_some_and(|a| index < a.body_parts) {
            entity.insert(crate::armor::BodySurface);
        }
        if let Some((path, _)) = equipment_parts
            .iter()
            .find(|(_, range)| range.contains(&index))
        {
            entity.insert((
                crate::pickup::EquippedPart(path.clone()),
                if player.weapon.as_ref() == Some(path) {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                },
            ));
        }
    }
    let font = fonts.add(Font::from_bytes(
        include_bytes!("../../shadow-viewer/assets/fonts/DejaVuSans.ttf").to_vec(),
    ));
    commands.spawn((
        SceneEntity,
        UiTargetCamera(*camera),
        Status,
        crate::hud::Diagnostic,
        GlobalZIndex(10),
        Text::default(),
        TextFont {
            font: font.into(),
            font_size: bevy::text::FontSize::Px(16.0),
            ..default()
        },
        TextColor(Color::srgb(0.86, 0.91, 0.78)),
        Node {
            position_type: PositionType::Absolute,
            left: px(25),
            bottom: px(25),
            width: px(285),
            ..default()
        },
    ));
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::motion::Pose;
    use shadow_formats::TerrainConfig;
    pub(crate) fn grid() -> TerrainGrid {
        // Наклонная поверхность: h(x,y) = x + 2*y в исходных единицах.
        TerrainGrid {
            config: TerrainConfig {
                name: "test".into(),
                width: 3,
                height: 3,
                edge: 100.0,
                height_step: 1.0,
                water_index: 0,
            },
            heights: vec![0.0, 100.0, 200.0, 200.0, 300.0, 400.0, 400.0, 500.0, 600.0],
        }
    }
    pub(crate) fn player() -> Player {
        let clips = (0..4)
            .map(|i| Clip {
                name: format!("{i}"),
                fps: 1.0,
                frames: vec![
                    Pose {
                        bones: vec![],
                        root: Vec3::ZERO,
                    },
                    Pose {
                        bones: vec![],
                        root: Vec3::Y
                            * if i == 2 {
                                200.0
                            } else if i == 1 {
                                100.0
                            } else {
                                0.0
                            },
                    },
                ],
            })
            .collect::<Vec<_>>();
        Player {
            name: "Test".into(),
            display_name: "Test Actor".into(),
            source: serde_json::Value::Null,
            rig: None,
            playback: Playback::new(&clips, 0, 0.0, false, true),
            clips,
            position: Vec3::new(0.5, 1.5, -0.5),
            spawn: Vec3::new(0.5, 1.5, -0.5),
            heading: 0.0,
            spawn_heading: 0.0,
            follow_camera: false,
            selected: true,
            destination: None,
            running: false,
            prepared_weapon: None,
            weapon: None,
            pickup_seconds: None,
            action: None,
            equipment_parts: Vec::new(),
            posture: crate::stance::Posture::Stand,
            postures: None,
            posture_transition: None,
            rifle_equipped: false,
            carry_movement: None,
            basis: Transform::from_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2))
                .with_scale(Vec3::splat(WORLD_SCALE)),
            focus: Vec3::Y,
            gesture: false,
        }
    }
    fn input(keys: &[KeyCode]) -> ButtonInput<KeyCode> {
        let mut input = ButtonInput::default();
        for key in keys {
            input.press(*key);
        }
        input
    }
    fn near(a: Vec3, b: Vec3) {
        assert!((a - b).length() < 1e-4, "{a:?} != {b:?}");
    }
    #[test]
    fn initial_actor_follows_scenario_manager_id_instead_of_name_or_mtf_order() {
        let mission = MissionDefinition::from_bytes(b"SPECIFY_MTF test.mtf\nSPM_PLAYER_MANAGER_ADD_PLAYER LOCAL_NONSCRIPTED Team 9\nMPM_PLAYER_MANAGER_ADD_PLAYER LOCAL_NONSCRIPTED Remote 8").unwrap();
        let map = shadow_formats::MtfMap::from_bytes(b"OBJECT Bipedal body Jack\nOBJECT_POSITION 1 2 3\nOBJECT_ROTATION 0 0 0\nOBJECT_ID 8 -1\nOBJECT Bipedal body Hero\nOBJECT_POSITION 4 5 6\nOBJECT_ROTATION 0 0 1\nOBJECT_ID 9 -2").unwrap();
        assert_eq!(initial_actor(&mission, &map.objects).unwrap().name, "Hero");
        assert!(initial_actor(&mission, &map.objects[..1]).is_err());
    }
    #[test]
    fn profile_index_ignores_unrelated_biography_but_validates_identifier() {
        let profile = b"CHARACTER_BEGIN\nCHARACTER Test\nBODY_DEFINITION\nBODY_TYPE Uniform\nHEAD_MODEL head\nHEAD_MAP head.bmp\nBODY_MODEL body\nBODY_MAP body.bmp\nBODY_DEFINITION_END\nDOSSIER_LINE \"unfinished biography\nCHARACTER_END";
        assert_eq!(
            profile_identifier(profile).unwrap().as_deref(),
            Some("Test")
        );
        assert!(
            CharacterProfile::from_bytes(profile)
                .unwrap_err()
                .to_string()
                .contains("незакрытые кавычки")
        );
        assert_eq!(
            profile_identifier(b"CHARACTER_BEGIN\nCHARACTER caf\xe9\n")
                .unwrap()
                .as_deref(),
            Some("café")
        );
        assert!(profile_identifier(b"CHARACTER One\nCHARACTER Two").is_err());
        assert!(profile_identifier(b"CHARACTER \"unfinished").is_err());
        assert!(profile_identifier(b"CHARACTER X\0").is_err());
        assert_eq!(profile_identifier(b";CHARACTER Comment\n").unwrap(), None);
    }
    #[test]
    fn initial_actor_reports_unsupported_multiple_local_characters() {
        let mission = MissionDefinition::from_bytes(
            b"SPECIFY_MTF test.mtf\nSPM_PLAYER_MANAGER_ADD_PLAYER LOCAL_NONSCRIPTED Team 9",
        )
        .unwrap();
        let map = shadow_formats::MtfMap::from_bytes(b"OBJECT Bipedal body One\nOBJECT_POSITION 1 2 3\nOBJECT_ROTATION 0 0 0\nOBJECT_ID 9 -1\nOBJECT Bipedal body Two\nOBJECT_POSITION 4 5 6\nOBJECT_ROTATION 0 0 1\nOBJECT_ID 9 -2").unwrap();
        assert!(initial_actor(&mission, &map.objects).is_err());
    }
    #[test]
    fn home_restores_original_spawn_direction_as_well_as_position() {
        let mut p = player();
        p.spawn_heading = 0.916076;
        p.heading = -1.0;
        p.position = Vec3::ZERO;
        p.step(&input(&[KeyCode::Home]), 0.0, &grid());
        near(p.position, p.spawn);
        assert_eq!(p.heading, p.spawn_heading);
    }
    #[test]
    fn move_order_uses_animation_metres_and_samples_the_slope_then_halts() {
        let mut p = player();
        assert!(p.order(&grid(), Vec3::new(0.5, 0.0, -2.0), false));
        p.step(&input(&[]), 0.5, &grid());
        near(p.position, Vec3::new(0.5, 2.5, -1.0));
        assert_eq!(p.playback.selected, 1);
        p.step(&input(&[KeyCode::KeyH]), 0.5, &grid());
        near(p.position, Vec3::new(0.5, 2.5, -1.0));
        assert_eq!(p.playback.selected, 0);
        assert!(p.destination.is_none());
    }
    #[test]
    fn turning_changes_only_the_next_delta_and_diagonal_is_not_faster() {
        let mut p = player();
        p.order(&grid(), Vec3::new(0.5, 0.0, -2.0), false);
        p.step(&input(&[]), 0.5, &grid());
        p.order(&grid(), Vec3::new(2.0, 0.0, -1.0), false);
        p.step(&input(&[]), 0.5, &grid());
        near(p.position, Vec3::new(1.0, 3.0, -1.0));
        let mut q = player();
        q.order(&grid(), Vec3::new(2.0, 0.0, -2.0), false);
        q.step(&input(&[]), 0.5, &grid());
        let delta = q.position - q.spawn;
        assert!((Vec2::new(delta.x, delta.z).length() - 0.5).abs() < 1e-4);
    }
    #[test]
    fn run_pause_frame_step_reset_and_arrival_do_not_overshoot() {
        let mut p = player();
        p.order(&grid(), Vec3::new(0.5, 0.0, -2.0), true);
        p.step(&input(&[]), 0.5, &grid());
        assert_eq!(p.playback.selected, 2);
        near(p.position, Vec3::new(0.5, 3.5, -1.5));
        p.step(&input(&[KeyCode::ControlLeft, KeyCode::KeyP]), 1.0, &grid());
        let paused = p.position;
        p.step(&input(&[]), 1.0, &grid());
        near(p.position, paused);
        p.step(&input(&[KeyCode::BracketRight]), 0.0, &grid());
        near(p.position, Vec3::new(0.5, 4.5, -2.0));
        assert!(p.destination.is_none());
        assert_eq!(p.playback.selected, 0);
        p.step(&input(&[KeyCode::Home]), 0.0, &grid());
        near(p.position, p.spawn);
        p.order(&grid(), Vec3::new(0.5, 0.0, -0.1), true);
        p.step(
            &input(&[KeyCode::ControlRight, KeyCode::KeyP]),
            5.0,
            &grid(),
        );
        near(p.position, Vec3::new(0.5, 0.7, -0.1));
        assert!(p.destination.is_none());
        assert!(!p.order(&grid(), Vec3::new(-1.0, 0.0, 0.0), false));
        assert!(!p.order(&grid(), Vec3::splat(f32::NAN), false));
    }
    #[test]
    fn original_tracking_and_selection_keys_work_and_space_wasd_shift_do_not_move() {
        let mut p = player();
        p.step(&input(&[KeyCode::KeyT]), 0.5, &grid());
        assert!(p.follow_camera);
        p.selected = false;
        p.step(&input(&[KeyCode::KeyT]), 0.5, &grid());
        assert!(p.follow_camera);
        p.step(&input(&[KeyCode::Digit1]), 0.5, &grid());
        assert!(p.selected);
        p.step(
            &input(&[
                KeyCode::Space,
                KeyCode::KeyW,
                KeyCode::KeyA,
                KeyCode::ShiftLeft,
            ]),
            0.5,
            &grid(),
        );
        near(p.position, p.spawn);
        assert!(!p.playback.paused);
        assert_eq!(p.playback.selected, 0);
        p.preview_action("wave", &grid());
        p.step(&input(&[]), 0.5, &grid());
        assert_eq!(p.playback.selected, 3);
        p.step(&input(&[KeyCode::KeyH]), 0.5, &grid());
        assert_eq!(p.playback.selected, 0);
    }
    #[test]
    fn actual_mouse_system_selects_orders_runs_and_deselects_without_ordering_through_ui() {
        use bevy::camera::{CameraProjection, RenderTargetInfo};
        let mut g = grid();
        g.config.edge = 1000.0;
        g.heights.fill(0.0);
        let mut actor = player();
        actor.position = Vec3::new(8.0, 0.0, -5.0);
        let mut camera = Camera::default();
        camera.computed.target_info = Some(RenderTargetInfo {
            physical_size: UVec2::new(1280, 800),
            ..default()
        });
        camera.computed.clip_from_view = PerspectiveProjection {
            aspect_ratio: 1.6,
            ..default()
        }
        .get_clip_from_view();
        let transform = GlobalTransform::from(
            Transform::from_xyz(10.0, 10.0, 5.0).looking_at(Vec3::new(10.0, 0.0, -10.0), Vec3::Y),
        );
        let target = Vec3::new(12.0, 0.0, -12.0);
        let screen = camera.world_to_viewport(&transform, target).unwrap();
        assert!(screen.x > 315.0);
        let body_screen = camera
            .world_to_viewport(&transform, actor.camera_target())
            .unwrap();
        let mut window = Window {
            resolution: (1280, 800).into(),
            ..default()
        };
        window.set_cursor_position(Some(screen));
        let mut app = App::new();
        app.insert_resource(actor)
            .insert_resource(Time::<()>::default())
            .init_resource::<crate::orders::ClickHistory>()
            .insert_resource(crate::training::Preview {
                scene: None,
                screenshot: None,
                focus: Vec3::ZERO,
                span: 20.0,
                grid: Some(g),
                playable: true,
            });
        let window_entity = app.world_mut().spawn(window).id();
        app.world_mut()
            .spawn((camera, transform, crate::training::MapCamera));
        app.add_systems(Update, crate::training::issue_orders);
        let click = |app: &mut App, button| {
            let mut mouse = ButtonInput::default();
            mouse.press(button);
            app.insert_resource(mouse);
            app.update();
        };
        click(&mut app, MouseButton::Left);
        let p = app.world().resource::<Player>();
        near(p.destination.unwrap(), target);
        assert!(!p.running);
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(std::time::Duration::from_secs_f32(0.2));
        click(&mut app, MouseButton::Left);
        assert!(app.world().resource::<Player>().running);
        click(&mut app, MouseButton::Right);
        assert!(!app.world().resource::<Player>().selected);
        app.world_mut()
            .get_mut::<Window>(window_entity)
            .unwrap()
            .set_cursor_position(Some(body_screen));
        click(&mut app, MouseButton::Left);
        assert!(app.world().resource::<Player>().selected);
        app.world_mut()
            .get_mut::<Window>(window_entity)
            .unwrap()
            .set_cursor_position(Some(Vec2::new(100.0, 100.0)));
        click(&mut app, MouseButton::Left);
        near(
            app.world().resource::<Player>().destination.unwrap(),
            target,
        );
        app.world_mut()
            .resource_mut::<crate::training::Preview>()
            .screenshot = Some("ignored.png".into());
        click(&mut app, MouseButton::Right);
        assert!(app.world().resource::<Player>().selected);
    }
    #[test]
    fn headless_bevy_input_and_pose_system_do_not_conflict_with_root_transforms() {
        let mut app = App::new();
        let mut actor = player();
        actor.order(&grid(), Vec3::new(0.5, 0.0, -2.0), false);
        app.insert_resource(actor).insert_resource(input(&[]));
        let mut time: Time = Time::default();
        time.advance_by(std::time::Duration::from_secs_f32(0.5));
        app.insert_resource(time)
            .insert_resource(crate::training::Preview {
                scene: None,
                screenshot: None,
                focus: Vec3::ZERO,
                span: 2.0,
                grid: Some(grid()),
                playable: true,
            })
            .insert_resource(crate::training::Orbit {
                target: Vec3::ZERO,
                yaw: 0.0,
                pitch: 0.3,
                distance: 6.0,
            });
        app.world_mut().spawn((Root, Transform::default()));
        app.world_mut().spawn((Bone(0), Transform::default()));
        app.world_mut().spawn((Status, Text::default()));
        app.world_mut().resource_mut::<Player>().playback.pose.bones = vec![Transform::default()];
        for clip in &mut app.world_mut().resource_mut::<Player>().clips {
            for pose in &mut clip.frames {
                pose.bones = vec![Transform::default()];
            }
        }
        app.add_systems(Update, crate::training::update_player);
        app.update();
        near(
            app.world().resource::<Player>().position,
            Vec3::new(0.5, 2.5, -1.0),
        );
    }
}
