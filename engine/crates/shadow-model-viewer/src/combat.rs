//! Первый часовой обучения. Координаты, профиль и клипы — из ресурсов;
//! урон и дистанция пока являются параметрами прототипа, а не восстановленной формулой.
use crate::{
    inventory::{Inventory, Weapon},
    map_scene::MapScene,
    motion::Clip,
    pickup::Pickup,
    player, rig,
    tutorial::Tutorial,
};
use bevy::prelude::*;
use shadow_formats::{
    AssetCatalog, BmfMotion, CampaignEventType as Event, MissionDefinition, config_lines,
};
use std::error::Error;

const REACH: f32 = 1.1;
const DAMAGE: f32 = 40.0;
#[derive(Resource)]
pub struct Combat {
    pub guard: player::Player,
    pub health: f32,
    pub maximum: f32,
    pub ordered: bool,
    pub hits: u32,
    pub source: serde_json::Value,
    strike: usize,
    callback: f32,
    hit_applied: bool,
    death_seconds: Option<f32>,
    manager: String,
    pub second: Option<Box<Combat>>,
    pub third: Option<Box<Combat>>,
    pub smg: Vec<Combat>,
    smg_completed: bool,
    uzi_model: Option<String>,
    finishing_melee: bool,
    fire: Option<FireOrder>,
    aim_clip: usize,
    fire_clip: usize,
    gun_model: String,
    pub pending_shots: Vec<Shot>,
    pub message: String,
}
#[derive(Component)]
pub struct GuardBone(usize, usize);
#[derive(Component)]
pub struct GuardRoot;
#[derive(Component)]
pub struct GuardRing(usize);
#[derive(Component)]
pub struct GuardWeapon(usize);
#[derive(Component)]
pub struct ShotTrace {
    remaining: f32,
}
#[derive(Clone, Copy)]
enum FirePhase {
    Approach,
    Aim,
    Firing,
}
struct FireOrder {
    target: usize,
    phase: FirePhase,
    cooldown: f32,
    weapon: Weapon,
    burst_remaining: u32,
    started: bool,
    aim_clip: usize,
    fire_clip: usize,
}
pub struct Shot {
    pub from: Vec3,
    pub to: Vec3,
    pub weapon: Weapon,
    pub audio: bool,
}

fn field<'a>(
    lines: &'a [shadow_formats::ConfigLine],
    key: &str,
) -> Result<&'a str, Box<dyn Error>> {
    let values: Vec<_> = lines.iter().filter(|l| l.words[0] == key).collect();
    match values.as_slice() {
        [line] if line.words.len() == 2 => Ok(&line.words[1]),
        _ => Err(format!("неверное или неоднозначное поле {key}").into()),
    }
}
/// Ограниченная последовательность: один IMMEDIATE-клип, без смешивания нескольких MOTION.
pub(crate) fn sequence(
    catalog: &AssetCatalog,
    name: &str,
    callback: Option<&str>,
    bones: &[rig::BoneDefinition],
) -> Result<(Clip, Option<usize>), Box<dyn Error>> {
    sequence_with_cog(catalog, name, callback, bones, None)
}
pub(crate) fn sequence_with_cog(
    catalog: &AssetCatalog,
    name: &str,
    callback: Option<&str>,
    bones: &[rig::BoneDefinition],
    cog: Option<Vec3>,
) -> Result<(Clip, Option<usize>), Box<dyn Error>> {
    let lines = config_lines(&catalog.read("config/mot_sequencer_defs.txt")?)?;
    let starts: Vec<_> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.words == ["BEGIN_SEQUENCE", name])
        .map(|(i, _)| i)
        .collect();
    let [start] = starts.as_slice() else {
        return Err(format!("нужна одна последовательность {name}").into());
    };
    let end = lines[start + 1..]
        .iter()
        .position(|l| l.words[0] == "END_SEQUENCE")
        .map(|i| start + 1 + i)
        .ok_or("последовательность не завершена")?;
    let body = &lines[start + 1..end];
    let motions: Vec<_> = body.iter().filter(|l| l.words[0] == "MOTION").collect();
    let [motion] = motions.as_slice() else {
        return Err("составная последовательность пока не поддерживается".into());
    };
    if !(2..=3).contains(&motion.words.len())
        || (motion.words.len() == 3 && motion.words[2] != "IMMEDIATE")
    {
        return Err("режим последовательности пока не поддерживается".into());
    }
    let clip = Clip::with_default_cog(
        &BmfMotion::from_bytes(&catalog.read(&format!(
            "motions/{}.bmf",
            motion.words[1].to_ascii_lowercase()
        ))?)?,
        bones,
        cog,
    )
    .map_err(|e| format!("{name}: {e}"))?;
    let frame = if let Some(callback) = callback {
        let values: Vec<_> = body
            .iter()
            .filter(|l| l.words.get(2).is_some_and(|v| v == callback))
            .collect();
        let [line] = values.as_slice() else {
            return Err("нужен один callback удара".into());
        };
        if line.words.len() != 3 {
            return Err("неверный callback удара".into());
        }
        let frame = line.words[1].parse::<usize>()?;
        if frame >= clip.frames.len() {
            return Err("callback удара за пределами BMF".into());
        }
        Some(frame)
    } else {
        None
    };
    Ok((clip, frame))
}
impl Combat {
    pub fn prepare(
        catalog: &AssetCatalog,
        scene: &MapScene,
        mission: &MissionDefinition,
        actor: &mut player::Player,
    ) -> Result<Self, Box<dyn Error>> {
        let mut first = Self::prepare_for(catalog, scene, mission, actor, "AI_Stab", None, 0)?;
        let second = Self::prepare_for(
            catalog,
            scene,
            mission,
            actor,
            "AI_Shoot",
            Some((first.strike, first.callback)),
            0,
        )?;
        first.second = Some(Box::new(second));
        first.third = Some(Box::new(Self::prepare_for(
            catalog,
            scene,
            mission,
            actor,
            "AI_Grenade",
            Some((first.strike, first.callback)),
            0,
        )?));
        for member in 0..6 {
            first.smg.push(Self::prepare_for(
                catalog,
                scene,
                mission,
                actor,
                "AI_SMG",
                Some((first.strike, first.callback)),
                member,
            )?);
        }
        first.uzi_model = Some(crate::inventory::definition(catalog, "UZI", "CSTYLE_RIFLE")?.model);
        let bones = &actor.rig.as_ref().unwrap().bones;
        let (aim, _) = sequence(catalog, "MSEQ_STAND_INTO_FIRE_PISTOL", None, bones)?;
        let (fire, _) = sequence(catalog, "MSEQ_STAND_FIRE_PISTOL", None, bones)?;
        first.aim_clip = actor.clips.len();
        actor.clips.push(aim);
        first.fire_clip = actor.clips.len();
        actor.clips.push(fire);
        first.source["pistol_aim_sequence"] = serde_json::json!("MSEQ_STAND_INTO_FIRE_PISTOL");
        first.source["pistol_fire_sequence"] = serde_json::json!("MSEQ_STAND_FIRE_PISTOL");
        Ok(first)
    }
    fn prepare_for(
        catalog: &AssetCatalog,
        scene: &MapScene,
        mission: &MissionDefinition,
        actor: &mut player::Player,
        manager_name: &str,
        shared_strike: Option<(usize, f32)>,
        member_index: usize,
    ) -> Result<Self, Box<dyn Error>> {
        let managers: Vec<_> = mission
            .players
            .iter()
            .filter(|p| {
                !p.multiplayer
                    && p.name.eq_ignore_ascii_case(manager_name)
                    && p.kind.eq_ignore_ascii_case("LOCAL_SCRIPTED")
            })
            .collect();
        let [manager] = managers.as_slice() else {
            return Err("нужен один менеджер часового".into());
        };
        let ai: Vec<_> = mission
            .ai
            .iter()
            .filter(|a| a.player.eq_ignore_ascii_case(&manager.name))
            .collect();
        let [ai] = ai.as_slice() else {
            return Err("нужен один файл ИИ часового".into());
        };
        let ai_path = format!("campaign/training/{}", ai.file);
        let lines = config_lines(&catalog.read(&ai_path)?)?;
        let members: Vec<_> = lines
            .iter()
            .filter(|l| l.words[0] == "MCP_GROUP_MEMBER")
            .collect();
        let expected_count = if manager_name == "AI_SMG" { 6 } else { 1 };
        if members.len() != expected_count {
            return Err(format!("нужно {expected_count} бойцов в {manager_name}").into());
        }
        let member = members
            .get(member_index)
            .ok_or("неверный индекс бойца группы")?;
        let mut ids = std::collections::BTreeSet::new();
        for m in &members {
            if m.words.len() != 4 || !ids.insert((&m.words[2], &m.words[3])) {
                return Err("повторный или неверный MCP_GROUP_MEMBER".into());
            }
        }
        if member.words.len() != 4 {
            return Err("неверный MCP_GROUP_MEMBER".into());
        }
        let id = [
            member.words[2].parse::<i32>()?,
            member.words[3].parse::<i32>()?,
        ];
        if id[0] != manager.id {
            return Err("менеджер часового не совпадает с MCP_GROUP_MEMBER".into());
        }
        let matches: Vec<_> = scene
            .actors
            .iter()
            .filter(|a| a.id == id && a.name.eq_ignore_ascii_case(&member.words[1]))
            .collect();
        let [mtf] = matches.as_slice() else {
            return Err("часовой из ИИ не найден однозначно в MTF".into());
        };
        let (_, profile) = player::profile_for(catalog, &mtf.name)?;
        let equipped = manager_name == "AI_Stab";
        let expected = if equipped {
            vec!["GIVE_OBJECT", "BDA", "EQUIP"]
        } else if manager_name == "AI_Shoot" {
            vec!["GIVE_OBJECT", "Frag Grenade"]
        } else {
            Vec::new()
        };
        if (expected.is_empty() && !profile.equipment.is_empty())
            || (!expected.is_empty()
                && (profile.equipment.len() != 1 || profile.equipment[0].words != expected))
        {
            return Err(
                format!("стартовая экипировка {manager_name} пока не поддерживается").into(),
            );
        }
        let maximum: f32 = field(&profile.properties, "HEALTH_MAX")?.parse()?;
        if !maximum.is_finite() || maximum <= 0.0 {
            return Err("неверный запас здоровья часового".into());
        }
        let mut guard = player::prepare_actor(catalog, &scene.grid, mtf, true)?;
        let gun = crate::inventory::gun_definition(catalog)?;
        let guard_rig = guard.rig.as_mut().unwrap();
        if equipped {
            rig::prepare_weapon(catalog, guard_rig, &gun.model, "BDA")?;
            guard.weapon = Some(gun.model.clone());
        }
        let (death, _) = sequence(
            catalog,
            "MSEQ_STAND_GET_SHOT_DEAD_1",
            None,
            &guard_rig.bones,
        )?;
        guard.clips.push(death);
        let (strike_index, callback, frame) = if let Some((index, seconds)) = shared_strike {
            (
                index,
                seconds,
                (seconds * actor.clips[index].fps).round() as usize,
            )
        } else {
            let bones = &actor
                .rig
                .as_ref()
                .ok_or("боевые клипы готовятся до создания сцены")?
                .bones;
            let (strike, frame) = sequence(
                catalog,
                "MSEQ_STAND_STAB",
                Some("MCB_PERFORM_ACTION"),
                bones,
            )?;
            let frame = frame.unwrap();
            let seconds = frame as f32 / strike.fps;
            let index = actor.clips.len();
            actor.clips.push(strike);
            (index, seconds, frame)
        };
        let strike = &actor.clips[strike_index];
        let source = serde_json::json!({"manager":manager.name,"ai":ai_path,"ai_origin":catalog.source_label(&ai_path)?,"actor_id":id,
            "profile":guard.source,"equipment":if equipped {"BDA"} else if manager_name=="AI_Shoot" {"Frag Grenade (inventory)"} else {"none"},"weapon_bone_id":equipped.then_some(rig::RIGHT_HAND),"health_max":maximum,
            "strike_sequence":"MSEQ_STAND_STAB","strike_motion":strike.name,"callback_frame":frame,"fps":strike.fps,
            "death_sequence":"MSEQ_STAND_GET_SHOT_DEAD_1","death_selection_original_verified":false,
            "damage_per_hit":DAMAGE,"reach_metres":REACH,"damage_formula_original_verified":false,"reach_original_verified":false,
            "enemy_ai_implemented":false,"navigation":false,"loot_inventory_implemented":equipped});
        Ok(Self {
            guard,
            health: maximum,
            maximum,
            ordered: false,
            hits: 0,
            strike: strike_index,
            callback,
            hit_applied: false,
            death_seconds: None,
            manager: manager.name.clone(),
            source,
            second: None,
            third: None,
            smg: Vec::new(),
            smg_completed: false,
            uzi_model: None,
            finishing_melee: false,
            fire: None,
            aim_clip: 0,
            fire_clip: 0,
            gun_model: gun.model,
            pending_shots: Vec::new(),
            message: String::new(),
        })
    }
    pub fn death_finished(&self) -> bool {
        self.death_seconds
            .is_some_and(|s| s >= self.guard.clips[4].duration())
    }
    pub fn alive(&self) -> bool {
        self.health > 0.0
    }
    pub fn target(&self) -> Vec3 {
        self.guard.camera_target()
    }
    pub fn order(
        &mut self,
        actor: &mut player::Player,
        grid: &shadow_formats::TerrainGrid,
    ) -> bool {
        if !self.alive()
            || actor.weapon.is_none()
            || actor.weapon != actor.prepared_weapon
            || actor.playback.paused
        {
            return false;
        }
        if actor.order(grid, self.guard.position, false) {
            self.ordered = true;
            self.hit_applied = false;
            true
        } else {
            false
        }
    }
    pub fn cancel(&mut self, actor: &mut player::Player) {
        self.ordered = false;
        self.hit_applied = false;
        self.finishing_melee = false;
        self.fire = None;
        if actor.action.is_some() {
            actor.action = None;
            actor.playback.select(&actor.clips, actor.idle_clip());
        }
    }
    pub fn restart(&mut self, actor: &mut player::Player) {
        self.cancel_all(actor);
        if let Some(second) = &mut self.second {
            second.restart(actor);
        }
        if let Some(third) = &mut self.third {
            third.restart(actor);
        }
        for guard in &mut self.smg {
            guard.restart(actor);
        }
        self.smg_completed = false;
        self.pending_shots.clear();
        self.message.clear();
        self.health = self.maximum;
        self.hits = 0;
        self.death_seconds = None;
        self.guard.playback.select(&self.guard.clips, 0);
        self.guard.playback.reset(&self.guard.clips);
    }
    pub fn update(
        &mut self,
        actor: &mut player::Player,
        t: &mut Tutorial,
        geometry: &Pickup,
        dt: f32,
    ) {
        self.update_melee(actor, t, geometry, dt);
        if let Some(second) = &mut self.second {
            second.update_melee(actor, t, geometry, dt);
        }
        if let Some(third) = &mut self.third {
            third.update_melee(actor, t, geometry, dt);
        }
        for guard in &mut self.smg {
            guard.update_melee(actor, t, geometry, dt);
        }
        self.complete_smg(t);
    }
    fn update_melee(
        &mut self,
        actor: &mut player::Player,
        t: &mut Tutorial,
        geometry: &Pickup,
        dt: f32,
    ) {
        if t.pausing()
            || actor.playback.paused
            || actor.changing_posture()
            || !dt.is_finite()
            || dt < 0.0
        {
            return;
        }
        if let Some(seconds) = self.death_seconds {
            let clip = &self.guard.clips[4];
            let seconds = (seconds + dt).min(clip.duration());
            self.death_seconds = Some(seconds);
            self.guard.playback.selected = 4;
            self.guard.playback.seconds = seconds;
            self.guard.playback.pose = clip.sample(seconds);
            if self.finishing_melee
                && actor.action.is_some_and(|(index, seconds)| {
                    index == self.strike && seconds >= actor.clips[self.strike].duration()
                })
            {
                self.cancel(actor);
            }
            return;
        }
        self.guard.playback.tick(&self.guard.clips, dt);
        if !self.ordered {
            return;
        }
        if actor.weapon.is_none() || (actor.action.is_none() && actor.destination.is_none()) {
            self.cancel(actor);
            return;
        }
        let delta = self.guard.position - actor.position;
        let distance = Vec2::new(delta.x, delta.z).length();
        let in_reach = distance <= REACH && delta.y.abs() <= 0.8;
        let visible = geometry.clear_line(actor.camera_target(), self.target());
        if actor.action.is_none() && in_reach {
            if !visible {
                actor.destination = None;
                self.cancel(actor);
                return;
            }
            actor.heading = (-delta.x).atan2(-delta.z);
            actor.begin_action(self.strike);
            self.hit_applied = false;
        }
        if let Some((index, seconds)) = actor.action {
            if index != self.strike {
                self.cancel(actor);
                return;
            }
            if !self.hit_applied && seconds >= self.callback {
                self.hit_applied = true;
                if in_reach && visible {
                    self.hits += 1;
                    self.health = (self.health - DAMAGE).max(0.0);
                    if !self.alive() {
                        self.ordered = false;
                        self.death_seconds = Some(0.0);
                        self.finishing_melee = true;
                        self.guard.playback.selected = 4;
                        self.guard.playback.seconds = 0.0;
                        self.guard.playback.pose = self.guard.clips[4].sample(0.0);
                        t.fire(Event::HumanKilled, &self.guard.name);
                        // Ресурс загружает и проверяет весь поддержанный состав AI_Stab (один часовой).
                        if !self.manager.eq_ignore_ascii_case("AI_SMG") {
                            t.fire(Event::AllPlayerInfantryKilled, &self.manager);
                        }
                        t.tick(0.0, false);
                    }
                }
            }
            if self.alive() && seconds >= actor.clips[self.strike].duration() {
                actor.begin_action(self.strike);
                self.hit_applied = false;
            }
        }
    }
    pub fn clicked(
        &self,
        camera: &Camera,
        transform: &GlobalTransform,
        cursor: Vec2,
        geometry: &Pickup,
    ) -> bool {
        if !self.alive() || !geometry.clear_line(transform.translation(), self.target()) {
            return false;
        }
        let (Ok(feet), Ok(head)) = (
            camera.world_to_viewport(transform, self.guard.position),
            camera.world_to_viewport(transform, self.guard.position + Vec3::Y * 1.8),
        ) else {
            return false;
        };
        let width = ((feet.y - head.y).abs() * 0.35).max(8.0);
        (cursor.x - (feet.x + head.x) * 0.5).abs() <= width
            && cursor.y >= head.y.min(feet.y) - 4.0
            && cursor.y <= head.y.max(feet.y) + 4.0
    }
    pub fn report(&self) -> serde_json::Value {
        serde_json::json!({"name":self.guard.name,"position_metres":self.guard.position.to_array(),"heading_radians":self.guard.heading,
            "health":self.health,"alive":self.alive(),"ordered":self.ordered,"hits":self.hits,"death_seconds":self.death_seconds,
            "motion":self.guard.clips[self.guard.playback.selected].name,"source":self.source,"second":self.second.as_ref().map(|s|s.report()),"third":self.third.as_ref().map(|s|s.report()),"smg_guards":self.smg.iter().map(|g|g.report()).collect::<Vec<_>>(),"smg_group_complete":self.smg_completed,"gun_order_target":self.fire.as_ref().map(|f|f.target),"message":self.message})
    }
}
// Свои компоненты костей: обновление Джона никогда не меняет позу часового.
pub fn setup(
    mut commands: Commands,
    combat: Option<ResMut<Combat>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let Some(mut combat) = combat else {
        return;
    };
    spawn_guard(
        &mut commands,
        &mut combat,
        0,
        &mut meshes,
        &mut materials,
        &mut images,
    );
    if let Some(third) = &mut combat.third {
        spawn_guard(
            &mut commands,
            third,
            2,
            &mut meshes,
            &mut materials,
            &mut images,
        );
    }
    if let Some(second) = &mut combat.second {
        spawn_guard(
            &mut commands,
            second,
            1,
            &mut meshes,
            &mut materials,
            &mut images,
        );
    }
    for (index, guard) in combat.smg.iter_mut().enumerate() {
        spawn_guard(
            &mut commands,
            guard,
            index + 3,
            &mut meshes,
            &mut materials,
            &mut images,
        );
    }
}
fn spawn_guard(
    commands: &mut Commands,
    combat: &mut Combat,
    owner: usize,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
) {
    let rig = combat.guard.rig.take().expect("guard setup runs once");
    let root = commands
        .spawn((
            player::SceneEntity,
            GuardRoot,
            Name::new(combat.guard.display_name.clone()),
            combat.guard.transform(),
            Visibility::default(),
        ))
        .id();
    let bones: Vec<_> = rig
        .bones
        .iter()
        .enumerate()
        .map(|(i, b)| {
            commands
                .spawn((
                    GuardBone(owner, i),
                    Name::new(b.name.clone()),
                    combat.guard.playback.pose.bones[i],
                    Visibility::default(),
                ))
                .id()
        })
        .collect();
    for (i, b) in rig.bones.iter().enumerate() {
        commands
            .entity(bones[i])
            .insert(ChildOf(b.parent.map(|p| bones[p]).unwrap_or(root)));
    }
    let textures = rig
        .images
        .into_iter()
        .map(|(name, texture)| (name, (images.add(texture.image), texture.alpha_mode)))
        .collect::<std::collections::BTreeMap<_, _>>();
    let first_weapon = rig.weapon_first_part;
    for (index, part) in rig.parts.into_iter().enumerate() {
        let mut part_entity = commands.spawn((
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
        if first_weapon.is_some_and(|first| index >= first) {
            part_entity.insert(GuardWeapon(owner));
        }
    }
    commands.spawn((
        player::SceneEntity,
        GuardRing(owner),
        Mesh3d(meshes.add(Torus::new(0.45, 0.5))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.22, 0.1),
            unlit: true,
            ..default()
        })),
        Transform::from_translation(combat.guard.position + Vec3::Y * 0.04),
        Visibility::default(),
    ));
}
pub fn input(
    keys: Res<ButtonInput<KeyCode>>,
    combat: Option<ResMut<Combat>>,
    actor: Option<ResMut<player::Player>>,
    t: Option<Res<Tutorial>>,
    inventory: Option<Res<Inventory>>,
) {
    if inventory.is_some_and(|inv| inv.open || inv.consumed_input) {
        return;
    }
    if t.is_some_and(|t| t.active.is_some() || t.consumed_input) {
        return;
    }
    if let (Some(mut c), Some(mut p)) = (combat, actor) {
        if keys.just_pressed(KeyCode::Home) {
            c.restart(&mut p);
        } else if keys.just_pressed(KeyCode::KeyH) && p.selected {
            c.cancel_all(&mut p);
        }
    }
}
#[allow(clippy::too_many_arguments)]
pub fn sync(
    mut commands: Commands,
    combat: Option<ResMut<Combat>>,
    actor: Option<ResMut<player::Player>>,
    t: Option<ResMut<Tutorial>>,
    item: Option<Res<Pickup>>,
    inventory: Option<ResMut<Inventory>>,
    time: Res<Time>,
    mut bones: Query<(&GuardBone, &mut Transform)>,
    mut rings: Query<(&GuardRing, &mut Visibility), Without<GuardWeapon>>,
    mut weapons: Query<(&GuardWeapon, &mut Visibility), Without<GuardRing>>,
    equipment: Query<(&crate::pickup::EquippedPart, &GlobalTransform)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let (Some(mut c), Some(mut p), Some(mut t), Some(item), Some(mut inv)) =
        (combat, actor, t, item, inventory)
    else {
        return;
    };
    c.update_with_inventory(&mut p, &mut t, &item, &mut inv, time.delta_secs());
    inv.observe(&item, &c);
    for (b, mut transform) in &mut bones {
        *transform = c.guard_at(b.0).guard.playback.pose.bones[b.1];
    }
    for (ring, mut visible) in &mut rings {
        *visible = if c.guard_at(ring.0).alive() {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
    for (w, mut visible) in &mut weapons {
        *visible = if c.guard_at(w.0).alive() {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
    for shot in c.pending_shots.drain(..) {
        let from = equipment
            .iter()
            .find(|(part, _)| part.0 == inv.gun_for(shot.weapon).unwrap().model)
            .map(|(_, transform)| transform.translation())
            .unwrap_or(shot.from);
        let length = from.distance(shot.to);
        if length.is_finite() && length > 0.001 {
            commands.spawn((
                player::SceneEntity,
                ShotTrace { remaining: 0.09 },
                Mesh3d(meshes.add(Cuboid::new(0.012, 0.012, length))),
                MeshMaterial3d(materials.add(StandardMaterial {
                    base_color: Color::srgb(1.0, 0.88, 0.48),
                    unlit: true,
                    ..default()
                })),
                Transform::from_translation((from + shot.to) * 0.5).looking_at(shot.to, Vec3::Y),
            ));
        }
        let sound = if shot.weapon == Weapon::Uzi {
            &inv.uzi_sound_handle
        } else {
            &inv.sound_handle
        };
        if let Some(sound) = sound.as_ref().filter(|_| shot.audio) {
            commands.spawn((
                player::SceneEntity,
                AudioPlayer(sound.clone()),
                PlaybackSettings::DESPAWN,
            ));
        }
    }
}
pub fn traces(
    mut commands: Commands,
    time: Res<Time>,
    mut traces: Query<(Entity, &mut ShotTrace)>,
) {
    for (e, mut trace) in &mut traces {
        trace.remaining -= time.delta_secs();
        if trace.remaining <= 0.0 {
            commands.entity(e).despawn();
        }
    }
}
impl Combat {
    pub fn has_order(&self) -> bool {
        self.ordered
            || self.fire.is_some()
            || self.second.as_ref().is_some_and(|s| s.ordered)
            || self.third.as_ref().is_some_and(|s| s.ordered)
            || self.smg.iter().any(|s| s.ordered)
    }
    pub fn guard_count(&self) -> usize {
        1 + usize::from(self.second.is_some()) + usize::from(self.third.is_some()) + self.smg.len()
    }
    pub fn guard_at(&self, index: usize) -> &Combat {
        match index {
            0 => self,
            1 => self.second.as_ref().expect("second prepared"),
            2 => self.third.as_ref().expect("third prepared"),
            _ => &self.smg[index - 3],
        }
    }
    pub fn guard_mut(&mut self, index: usize) -> &mut Combat {
        match index {
            0 => self,
            1 => self.second.as_mut().expect("second prepared"),
            2 => self.third.as_mut().expect("third prepared"),
            _ => &mut self.smg[index - 3],
        }
    }
    pub fn cancel_all(&mut self, p: &mut player::Player) {
        self.cancel(p);
        if let Some(second) = &mut self.second {
            second.cancel(p);
        }
        if let Some(third) = &mut self.third {
            third.cancel(p);
        }
        for guard in &mut self.smg {
            guard.cancel(p);
        }
    }
    fn complete_smg(&mut self, t: &mut Tutorial) {
        if !self.smg.is_empty() && !self.smg_completed && self.smg.iter().all(|g| !g.alive()) {
            self.smg_completed = true;
            t.fire(Event::AllPlayerInfantryKilled, "AI_SMG");
            t.tick(0.0, false);
        }
    }
    pub fn blast_damage(&mut self, index: usize, damage: f32, t: &mut Tutorial) {
        if index >= self.guard_count() || !damage.is_finite() || damage <= 0.0 {
            return;
        }
        let guard = self.guard_mut(index);
        if !guard.alive() {
            return;
        }
        guard.hits += 1;
        guard.health = (guard.health - damage).max(0.0);
        if !guard.alive() {
            guard.ordered = false;
            guard.death_seconds = Some(0.0);
            guard.finishing_melee = false;
            guard.guard.playback.selected = 4;
            guard.guard.playback.seconds = 0.0;
            guard.guard.playback.pose = guard.guard.clips[4].sample(0.0);
            t.fire(Event::HumanKilled, &guard.guard.name);
            if !guard.manager.eq_ignore_ascii_case("AI_SMG") {
                t.fire(Event::AllPlayerInfantryKilled, &guard.manager);
            }
            t.tick(0.0, false);
        }
        self.complete_smg(t);
    }
    pub fn order_target(
        &mut self,
        index: usize,
        p: &mut player::Player,
        grid: &shadow_formats::TerrainGrid,
    ) -> bool {
        if index >= self.guard_count() || !self.guard_at(index).alive() || p.playback.paused {
            return false;
        }
        let rifle = self
            .uzi_model
            .as_ref()
            .is_some_and(|model| p.weapon.as_ref() == Some(model));
        let gun = p.weapon.as_ref() == Some(&self.gun_model) || rifle;
        if !gun && p.weapon != p.prepared_weapon {
            return false;
        }
        self.cancel_all(p);
        self.message.clear();
        if gun {
            if p.order(grid, self.guard_at(index).guard.position, false) {
                self.fire = Some(FireOrder {
                    target: index,
                    phase: FirePhase::Approach,
                    cooldown: 0.0,
                    weapon: if rifle { Weapon::Uzi } else { Weapon::Bda },
                    burst_remaining: 0,
                    started: false,
                    aim_clip: p.fire_clips(rifle).map(|v| v.0).unwrap_or(self.aim_clip),
                    fire_clip: p.fire_clips(rifle).map(|v| v.1).unwrap_or(self.fire_clip),
                });
                true
            } else {
                false
            }
        } else {
            self.guard_mut(index).order(p, grid)
        }
    }
    pub fn clicked_target(
        &self,
        camera: &Camera,
        transform: &GlobalTransform,
        cursor: Vec2,
        item: &Pickup,
    ) -> Option<usize> {
        (0..self.guard_count()).find(|&i| self.guard_at(i).clicked(camera, transform, cursor, item))
    }
    pub fn update_with_inventory(
        &mut self,
        p: &mut player::Player,
        t: &mut Tutorial,
        item: &Pickup,
        inv: &mut Inventory,
        dt: f32,
    ) {
        if inv.open
            || t.pausing()
            || p.playback.paused
            || p.changing_posture()
            || !dt.is_finite()
            || dt < 0.0
        {
            return;
        }
        self.update(p, t, item, dt);
        let Some(mut order) = self.fire.take() else {
            return;
        };
        if !self.guard_at(order.target).alive()
            || inv.weapon(p) != Some(order.weapon)
            || !inv.owns(order.weapon)
        {
            self.cancel_all(p);
            return;
        }
        let target = self.guard_at(order.target).target();
        let delta = self.guard_at(order.target).guard.position - p.position;
        let gun = inv.gun_for(order.weapon).unwrap();
        let (range, delay, damage, burst_count, round_delay) = (
            gun.range,
            gun.delay,
            gun.damage(),
            gun.burst_count,
            gun.burst_round_delay,
        );
        let in_range = delta.length() <= range;
        let visible = item.clear_line(p.camera_target(), target);
        match order.phase {
            FirePhase::Approach => {
                if in_range && visible {
                    p.heading = (-delta.x).atan2(-delta.z);
                    p.begin_action(order.aim_clip);
                    order.phase = FirePhase::Aim;
                } else if p.destination.is_none() {
                    self.message = "Нет линии огня".into();
                    self.cancel_all(p);
                    return;
                }
            }
            FirePhase::Aim => {
                if p.action.is_none() {
                    self.cancel_all(p);
                    return;
                }
                if p.action
                    .is_some_and(|(_, s)| s >= p.clips[order.aim_clip].duration())
                {
                    p.begin_action(order.fire_clip);
                    order.phase = FirePhase::Firing;
                }
            }
            FirePhase::Firing => {
                if p.action.is_none() || !in_range || !visible {
                    self.message = "Нет линии огня".into();
                    self.cancel_all(p);
                    return;
                }
                if order.started {
                    order.cooldown -= dt;
                } else {
                    order.started = true;
                }
                while order.cooldown <= 0.0 {
                    if !inv.take_round_for(order.weapon) {
                        self.message = format!("{}: магазин пуст", order.weapon.name());
                        self.cancel_all(p);
                        return;
                    }
                    p.begin_action(order.fire_clip);
                    let audio = order.burst_remaining == 0;
                    if order.burst_remaining == 0 {
                        order.burst_remaining = burst_count;
                    }
                    order.burst_remaining -= 1;
                    order.cooldown += if order.burst_remaining > 0 {
                        round_delay
                    } else {
                        delay
                    };
                    self.pending_shots.push(Shot {
                        from: p.camera_target(),
                        to: target,
                        weapon: order.weapon,
                        audio,
                    });
                    let guard = self.guard_mut(order.target);
                    guard.hits += 1;
                    guard.health = (guard.health - damage).max(0.0);
                    if !guard.alive() {
                        guard.death_seconds = Some(0.0);
                        guard.guard.playback.selected = 4;
                        guard.guard.playback.seconds = 0.0;
                        guard.guard.playback.pose = guard.guard.clips[4].sample(0.0);
                        t.fire(Event::HumanKilled, &guard.guard.name);
                        if !guard.manager.eq_ignore_ascii_case("AI_SMG") {
                            t.fire(Event::AllPlayerInfantryKilled, &guard.manager);
                        }
                        t.tick(0.0, false);
                        self.complete_smg(t);
                        self.cancel_all(p);
                        return;
                    }
                }
            }
        }
        self.fire = Some(order);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    pub(crate) fn fixture() -> (
        Combat,
        player::Player,
        Pickup,
        Tutorial,
        shadow_formats::TerrainGrid,
    ) {
        let (item, mut actor, _, grid) = crate::pickup::tests::fixture();
        let (mission, helps) = (
            MissionDefinition::from_bytes(b"SPECIFY_MTF training.mtf\nSPM_PLAYER_MANAGER_ADD_PLAYER LOCAL_SCRIPTED AI_Stab 3\nDEFINE_OBJECTIVE true first \"Stab\"\nDEFINE_OBJECTIVE true second \"Shoot\"\nADD_OBJECTIVE_ENABLE first second\nCER_DEFINITION\nCER_FOR_TYPE CET_IMMEDIATE\nCER_BODY_DEFINITION\nCER_BODY_TYPE CER_BODY_ENABLE_OBJECTIVE\nCER_BODY_IDENTIFIER first\nCER_DEFINITION\nCER_FOR_TYPE CET_ALL_PLAYERS_INFANTRY_KILLED\nCER_FOR AI_Stab\nCER_BODY_DEFINITION\nCER_BODY_TYPE CER_BODY_NOTIFY_OBJECTIVE_COMPLETE\nCER_BODY_IDENTIFIER first\nCER_BODY_DEFINITION\nCER_BODY_TYPE CER_BODY_SHOW_CONTEXT_HELP\nCER_BODY_IDENTIFIER after\nCER_BODY_MS_DELAY 1000\nCER_BODY_DEFINITION\nCER_BODY_TYPE CER_BODY_REMOVE_CER\n").unwrap(),
            shadow_formats::HelpDefinitions::from_bytes(b"HELP_DEF\nHELP_ID after\nHELP_CONTEXT\nHELP_BODY \"Done\"\n").unwrap());
        let t = Tutorial::new(mission, helps).unwrap();
        actor.prepared_weapon = Some("knife".into());
        actor.weapon = Some("knife".into());
        actor.clips.push(Clip {
            name: "stab".into(),
            fps: 10.0,
            frames: vec![actor.clips[0].frames[0].clone(); 11],
        });
        let mut guard = player::tests::player();
        guard.name = "guard".into();
        guard.position = actor.position + Vec3::X * 0.8;
        guard.clips.push(Clip {
            name: "death".into(),
            fps: 10.0,
            frames: vec![guard.clips[0].frames[0].clone(); 11],
        });
        let c = Combat {
            guard,
            health: 75.0,
            maximum: 75.0,
            ordered: false,
            hits: 0,
            source: serde_json::Value::Null,
            strike: 5,
            callback: 0.6,
            hit_applied: false,
            death_seconds: None,
            manager: "AI_Stab".into(),
            second: None,
            third: None,
            smg: Vec::new(),
            smg_completed: false,
            uzi_model: None,
            finishing_melee: false,
            fire: None,
            aim_clip: 5,
            fire_clip: 5,
            gun_model: "bda".into(),
            pending_shots: Vec::new(),
            message: String::new(),
        };
        (c, actor, item, t, grid)
    }
    #[test]
    fn callback_applies_once_per_strike_and_death_completes_linked_objective() {
        let (mut c, mut p, item, mut t, grid) = fixture();
        assert!(c.order(&mut p, &grid));
        c.update(&mut p, &mut t, &item, 0.0);
        p.step(&ButtonInput::default(), 0.59, &grid);
        c.update(&mut p, &mut t, &item, 0.59);
        assert_eq!(c.health, 75.0);
        p.step(&ButtonInput::default(), 0.02, &grid);
        c.update(&mut p, &mut t, &item, 0.02);
        assert_eq!(c.health, 35.0);
        c.update(&mut p, &mut t, &item, 0.0);
        assert_eq!(c.hits, 1);
        assert!(t.completed.is_empty());
        p.step(&ButtonInput::default(), 0.4, &grid);
        c.update(&mut p, &mut t, &item, 0.4);
        p.step(&ButtonInput::default(), 0.6, &grid);
        c.update(&mut p, &mut t, &item, 0.6);
        assert_eq!(c.health, 0.0);
        assert_eq!(c.hits, 2);
        assert!(t.completed.contains("first"));
        assert!(t.objectives.contains("second"));
        assert_eq!(
            t.events
                .iter()
                .filter(|e| e.starts_with("CET_ALL_PLAYERS_INFANTRY_KILLED"))
                .count(),
            1
        );
        t.tick(0.99, false);
        assert!(t.active.is_none());
        t.tick(0.01, false);
        assert_eq!(t.active.as_deref(), Some("after"));
        t.close();
        p.step(&ButtonInput::default(), 1.0, &grid);
        c.update(&mut p, &mut t, &item, 10.0);
        assert!(c.death_finished());
        assert_eq!(c.death_seconds, Some(1.0));
        assert!(p.action.is_none());
        assert!(!c.order(&mut p, &grid));
    }
    #[test]
    fn unarmed_and_wrong_equipment_cannot_attack() {
        let (mut c, mut p, _, _, grid) = fixture();
        p.weapon = None;
        assert!(!c.order(&mut p, &grid));
        p.weapon = Some("BDA".into());
        assert!(!c.order(&mut p, &grid));
        assert_eq!(c.health, 75.0);
    }
    #[test]
    fn cancellation_and_new_movement_before_callback_prevent_damage() {
        let (mut c, mut p, item, mut t, grid) = fixture();
        c.order(&mut p, &grid);
        c.update(&mut p, &mut t, &item, 0.0);
        p.step(&ButtonInput::default(), 0.4, &grid);
        c.cancel(&mut p);
        p.step(&ButtonInput::default(), 1.0, &grid);
        c.update(&mut p, &mut t, &item, 1.0);
        assert_eq!(c.hits, 0);
        assert!(p.action.is_none());
        c.order(&mut p, &grid);
        c.update(&mut p, &mut t, &item, 0.0);
        p.order(&grid, Vec3::new(0.5, 0.0, -1.8), false);
        c.cancel(&mut p);
        assert!(p.destination.is_some());
        assert_eq!(c.health, 75.0);
    }
    #[test]
    fn pause_and_invalid_time_preserve_attack_and_health() {
        let (mut c, mut p, item, mut t, grid) = fixture();
        c.order(&mut p, &grid);
        c.update(&mut p, &mut t, &item, 0.0);
        p.playback.paused = true;
        p.step(&ButtonInput::default(), 5.0, &grid);
        c.update(&mut p, &mut t, &item, 5.0);
        assert_eq!(p.action, Some((5, 0.0)));
        assert_eq!(c.hits, 0);
        p.playback.paused = false;
        for dt in [f32::NAN, -1.0] {
            p.step(&ButtonInput::default(), dt, &grid);
            c.update(&mut p, &mut t, &item, dt);
        }
        assert_eq!(p.action, Some((5, 0.0)));
        assert_eq!(c.health, 75.0);
        t.active = Some("after".into());
        c.update(&mut p, &mut t, &item, 5.0);
        assert_eq!(c.hits, 0);
    }
    #[test]
    fn distance_is_rechecked_at_callback() {
        let (mut c, mut p, item, mut t, grid) = fixture();
        c.order(&mut p, &grid);
        c.update(&mut p, &mut t, &item, 0.0);
        c.guard.position.x += 5.0;
        p.step(&ButtonInput::default(), 0.6, &grid);
        c.update(&mut p, &mut t, &item, 0.6);
        assert_eq!(c.hits, 0);
        assert_eq!(c.health, 75.0);
    }
    #[test]
    fn opaque_geometry_blocks_melee_even_inside_reach() {
        let (mut c, mut p, mut item, mut t, grid) = fixture();
        crate::pickup::tests::add_wall(&mut item);
        assert!(c.order(&mut p, &grid));
        c.update(&mut p, &mut t, &item, 0.0);
        p.step(&ButtonInput::default(), 1.0, &grid);
        c.update(&mut p, &mut t, &item, 1.0);
        assert_eq!(c.health, 75.0);
        assert!(!c.ordered);
        assert!(p.action.is_none());
    }
    #[test]
    fn restart_restores_guard_and_removes_pending_strike() {
        let (mut c, mut p, _, mut t, grid) = fixture();
        c.health = 0.0;
        c.hits = 2;
        c.death_seconds = Some(1.0);
        p.begin_action(5);
        c.restart(&mut p);
        t.restart();
        assert_eq!(c.health, 75.0);
        assert_eq!(c.hits, 0);
        assert!(c.death_seconds.is_none());
        assert!(p.action.is_none());
        assert!(t.completed.is_empty());
        assert!(!t.objectives.contains("second"));
        assert!(c.order(&mut p, &grid));
    }
}

#[cfg(test)]
mod gun_tests {
    use super::*;
    fn ready() -> (
        Combat,
        player::Player,
        Pickup,
        Tutorial,
        shadow_formats::TerrainGrid,
        Inventory,
    ) {
        let (c, mut p, item, t, grid) = super::tests::fixture();
        let mut inv = crate::inventory::tests::inventory();
        inv.collected = true;
        inv.equip(Weapon::Bda, &mut p);
        (c, p, item, t, grid, inv)
    }
    fn aim(
        c: &mut Combat,
        p: &mut player::Player,
        item: &Pickup,
        t: &mut Tutorial,
        grid: &shadow_formats::TerrainGrid,
        inv: &mut Inventory,
        index: usize,
    ) {
        assert!(c.order_target(index, p, grid));
        c.update_with_inventory(p, t, item, inv, 0.0);
        p.step(&ButtonInput::default(), 1.0, grid);
        c.update_with_inventory(p, t, item, inv, 1.0);
    }
    #[test]
    fn ranged_hits_spend_rounds_obey_delay_and_complete_only_after_death() {
        let (mut c, mut p, item, mut t, grid, mut inv) = ready();
        aim(&mut c, &mut p, &item, &mut t, &grid, &mut inv, 0);
        assert_eq!(inv.shots, 0);
        c.update_with_inventory(&mut p, &mut t, &item, &mut inv, 0.0);
        assert_eq!(inv.rounds, 12);
        assert!((c.health - 50.7).abs() < 0.01);
        assert!(t.completed.is_empty());
        c.update_with_inventory(&mut p, &mut t, &item, &mut inv, 0.29);
        assert_eq!(inv.shots, 1);
        c.update_with_inventory(&mut p, &mut t, &item, &mut inv, 0.011);
        assert_eq!(inv.shots, 2);
        for _ in 0..2 {
            c.update_with_inventory(&mut p, &mut t, &item, &mut inv, 0.3);
        }
        assert!(!c.alive());
        assert_eq!(inv.shots, 4);
        assert_eq!(inv.rounds, 9);
        assert_eq!(c.pending_shots.len(), 4);
        assert!(t.completed.contains("first"));
        c.update_with_inventory(&mut p, &mut t, &item, &mut inv, 1.0);
        assert_eq!(inv.shots, 4);
    }
    #[test]
    fn empty_magazine_stops_without_damage_or_negative_ammo() {
        let (mut c, mut p, item, mut t, grid, mut inv) = ready();
        inv.rounds = 0;
        aim(&mut c, &mut p, &item, &mut t, &grid, &mut inv, 0);
        c.update_with_inventory(&mut p, &mut t, &item, &mut inv, 0.0);
        assert_eq!(c.health, 75.0);
        assert_eq!(inv.rounds, 0);
        assert_eq!(inv.shots, 0);
        assert!(!c.has_order());
        assert_eq!(c.message, "BDA: магазин пуст");
    }
    #[test]
    fn opaque_obstacle_between_aim_and_fire_prevents_damage_and_ammo_use() {
        let (mut c, mut p, mut item, mut t, grid, mut inv) = ready();
        aim(&mut c, &mut p, &item, &mut t, &grid, &mut inv, 0);
        crate::pickup::tests::add_wall(&mut item);
        c.update_with_inventory(&mut p, &mut t, &item, &mut inv, 0.0);
        assert_eq!(c.health, 75.0);
        assert_eq!(inv.rounds, 13);
        assert_eq!(inv.shots, 0);
        assert!(!c.has_order());
    }
    #[test]
    fn range_and_inventory_pause_block_firing() {
        let (mut c, mut p, item, mut t, grid, mut inv) = ready();
        inv.gun.range = 0.1;
        c.order_target(0, &mut p, &grid);
        c.update_with_inventory(&mut p, &mut t, &item, &mut inv, 1.0);
        assert!(p.action.is_none());
        assert_eq!(inv.shots, 0);
        inv.gun.range = 20.0;
        aim(&mut c, &mut p, &item, &mut t, &grid, &mut inv, 0);
        inv.open = true;
        c.update_with_inventory(&mut p, &mut t, &item, &mut inv, 10.0);
        assert_eq!(inv.rounds, 13);
        assert_eq!(c.health, 75.0);
        inv.open = false;
        for dt in [f32::NAN, -1.0] {
            c.update_with_inventory(&mut p, &mut t, &item, &mut inv, dt);
        }
        assert_eq!(inv.shots, 0);
    }
    #[test]
    fn ownership_and_equipped_weapon_are_required_for_a_shot() {
        let (mut c, mut p, item, mut t, grid, mut inv) = ready();
        inv.collected = false;
        aim(&mut c, &mut p, &item, &mut t, &grid, &mut inv, 0);
        c.update_with_inventory(&mut p, &mut t, &item, &mut inv, 0.0);
        assert_eq!(inv.shots, 0);
        assert_eq!(c.health, 75.0);
        p.weapon = None;
        assert!(!c.order_target(0, &mut p, &grid));
    }
    #[test]
    fn dead_first_guard_does_not_cancel_second_guard_aim() {
        let (mut c, mut p, item, mut t, grid, mut inv) = ready();
        let (mut second, _, _, _, _) = super::tests::fixture();
        second.manager = "AI_Shoot".into();
        c.second = Some(Box::new(second));
        c.health = 0.0;
        c.death_seconds = Some(0.0);
        c.finishing_melee = true;
        aim(&mut c, &mut p, &item, &mut t, &grid, &mut inv, 1);
        assert!(p.action.is_some());
        c.update_with_inventory(&mut p, &mut t, &item, &mut inv, 0.0);
        assert_eq!(c.health, 0.0);
        assert!((c.guard_at(1).health - 50.7).abs() < 0.01);
        assert_eq!(c.hits, 0);
        assert_eq!(inv.shots, 1);
    }
}

#[cfg(test)]
mod smg_tests {
    use super::*;
    fn ready() -> (
        Combat,
        player::Player,
        Pickup,
        Tutorial,
        shadow_formats::TerrainGrid,
        Inventory,
    ) {
        let (mut c, mut p, item, _, grid) = super::tests::fixture();
        c.second = Some(Box::new(super::tests::fixture().0));
        c.third = Some(Box::new(super::tests::fixture().0));
        for _ in 0..6 {
            let mut guard = super::tests::fixture().0;
            guard.manager = "AI_SMG".into();
            c.smg.push(guard);
        }
        c.uzi_model = Some("uzi".into());
        let mut inv = crate::smg::tests::inventory();
        inv.owns_uzi = true;
        inv.equip(Weapon::Uzi, &mut p);
        (c, p, item, crate::smg::tests::tutorial(), grid, inv)
    }
    fn start(
        c: &mut Combat,
        p: &mut player::Player,
        item: &Pickup,
        t: &mut Tutorial,
        grid: &shadow_formats::TerrainGrid,
        inv: &mut Inventory,
    ) {
        assert!(c.order_target(3, p, grid));
        c.update_with_inventory(p, t, item, inv, 0.0);
        p.step(&ButtonInput::default(), 1.0, grid);
        c.update_with_inventory(p, t, item, inv, 0.0);
        c.update_with_inventory(p, t, item, inv, 0.0);
    }
    #[test]
    fn three_round_burst_uses_round_delay_and_one_audio_sample() {
        let (mut c, mut p, item, mut t, grid, mut inv) = ready();
        c.smg[0].health = 300.0;
        start(&mut c, &mut p, &item, &mut t, &grid, &mut inv);
        assert_eq!(inv.uzi_shots, 1);
        c.update_with_inventory(&mut p, &mut t, &item, &mut inv, 0.079);
        assert_eq!(inv.uzi_shots, 1);
        c.update_with_inventory(&mut p, &mut t, &item, &mut inv, 0.002);
        assert_eq!(inv.uzi_shots, 2);
        c.update_with_inventory(&mut p, &mut t, &item, &mut inv, 0.08);
        assert_eq!(inv.uzi_shots, 3);
        c.update_with_inventory(&mut p, &mut t, &item, &mut inv, 0.248);
        assert_eq!(inv.uzi_shots, 3);
        c.update_with_inventory(&mut p, &mut t, &item, &mut inv, 0.002);
        assert_eq!(inv.uzi_shots, 4);
        assert_eq!(c.pending_shots.iter().filter(|s| s.audio).count(), 2);
        assert_eq!(inv.rounds, 9);
    }
    #[test]
    fn burst_timing_matches_small_and_large_frame_deltas() {
        let mut results = Vec::new();
        for (count, dt) in [(100, 0.01), (4, 0.25)] {
            let (mut c, mut p, item, mut t, grid, mut inv) = ready();
            c.smg[0].health = 300.0;
            start(&mut c, &mut p, &item, &mut t, &grid, &mut inv);
            for _ in 0..count {
                c.update_with_inventory(&mut p, &mut t, &item, &mut inv, dt);
            }
            results.push((
                inv.uzi_shots,
                inv.uzi_rounds,
                c.pending_shots.iter().filter(|s| s.audio).count(),
            ));
        }
        assert_eq!(results[0], results[1]);
        assert_eq!(results[0], (9, 23, 3));
    }
    #[test]
    fn empty_magazine_stops_partway_through_burst_without_negative_ammo() {
        let (mut c, mut p, item, mut t, grid, mut inv) = ready();
        inv.uzi_rounds = 2;
        start(&mut c, &mut p, &item, &mut t, &grid, &mut inv);
        c.update_with_inventory(&mut p, &mut t, &item, &mut inv, 0.08);
        c.update_with_inventory(&mut p, &mut t, &item, &mut inv, 0.08);
        assert_eq!(inv.uzi_rounds, 0);
        assert_eq!(inv.uzi_shots, 2);
        assert!(c.smg[0].alive());
        assert!(!c.has_order());
        assert_eq!(inv.rounds, 9);
    }
    #[test]
    fn group_event_is_emitted_only_after_all_six_die_and_once() {
        let (mut c, _, _, mut t, _, _) = ready();
        for index in 3..8 {
            c.blast_damage(index, 100.0, &mut t);
            assert!(!t.completed.contains("guards"));
        }
        c.blast_damage(8, 100.0, &mut t);
        assert!(t.completed.contains("guards"));
        c.blast_damage(8, 100.0, &mut t);
        c.complete_smg(&mut t);
        assert_eq!(
            t.events
                .iter()
                .filter(|e| e.as_str() == "CET_ALL_PLAYERS_INFANTRY_KILLED:AI_SMG")
                .count(),
            1
        );
        t.tick(1.0, false);
        assert_eq!(t.active.as_deref(), Some("after"));
    }
    #[test]
    fn ownership_pause_and_weapon_change_prevent_more_rounds() {
        let (mut c, mut p, item, mut t, grid, mut inv) = ready();
        start(&mut c, &mut p, &item, &mut t, &grid, &mut inv);
        inv.open = true;
        c.update_with_inventory(&mut p, &mut t, &item, &mut inv, 1.0);
        assert_eq!(inv.uzi_shots, 1);
        inv.open = false;
        inv.equip(Weapon::Bda, &mut p);
        c.update_with_inventory(&mut p, &mut t, &item, &mut inv, 1.0);
        assert_eq!(inv.uzi_shots, 1);
        assert!(!c.has_order());
    }
    #[test]
    fn stand_crouch_and_prone_select_their_source_rifle_aim_and_fire_clips() {
        for posture in [
            crate::stance::Posture::Stand,
            crate::stance::Posture::Crouch,
            crate::stance::Posture::Prone,
        ] {
            let (mut c, mut p, item, mut t, grid, mut inv) = ready();
            crate::stance::tests::add(&mut p);
            p.posture = posture;
            let (aim, fire) = p.fire_clips(true).unwrap();
            assert!(c.order_target(3, &mut p, &grid));
            c.update_with_inventory(&mut p, &mut t, &item, &mut inv, 0.0);
            assert_eq!(p.action.unwrap().0, aim);
            p.step(&ButtonInput::default(), 2.0, &grid);
            c.update_with_inventory(&mut p, &mut t, &item, &mut inv, 0.0);
            assert_eq!(p.action.unwrap().0, fire);
            c.update_with_inventory(&mut p, &mut t, &item, &mut inv, 0.0);
            assert_eq!(inv.uzi_shots, 1);
        }
    }
    #[test]
    fn home_restart_revives_whole_group_and_clears_pending_burst() {
        let (mut c, mut p, item, mut t, grid, mut inv) = ready();
        start(&mut c, &mut p, &item, &mut t, &grid, &mut inv);
        for i in 3..9 {
            c.blast_damage(i, 100.0, &mut t);
        }
        c.restart(&mut p);
        assert!(!c.smg_completed);
        assert!(c.smg.iter().all(|g| g.alive() && g.hits == 0));
        assert!(c.pending_shots.is_empty());
        assert!(!c.has_order());
    }
}
