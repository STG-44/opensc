//! Carry a dead training guard using the original shoulder and walking motions.
use crate::{
    combat::Combat,
    inventory::Inventory,
    motion::Clip,
    pickup::Pickup,
    player::Player,
    training::{MapCamera, Preview},
    tutorial::Tutorial,
};
use bevy::prelude::*;
use shadow_formats::{AssetCatalog, BmfMotion, TerrainGrid, config_lines};
use std::error::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Approaching(usize),
    Lifting(usize),
    Carrying(usize),
}
#[derive(Resource)]
pub struct Carry {
    pub phase: Phase,
    lift: usize,
    callback: f32,
    movement: [usize; 3],
    body_clips: Vec<usize>,
    shoulder: Anchor,
    pelvis: Vec<Anchor>,
    saved_weapon: Option<String>,
    pub message: String,
}

/// Preserve the source hierarchy before the render setup takes ownership of the rig.
struct Anchor(Vec<usize>);
impl Anchor {
    fn new(bones: &[crate::rig::BoneDefinition], id: u32) -> Result<Self, Box<dyn Error>> {
        let mut index = bones
            .iter()
            .position(|b| b.id == id)
            .ok_or("carry anchor missing")?;
        let mut chain = vec![index];
        while let Some(parent) = bones[index].parent {
            chain.push(parent);
            index = parent;
        }
        Ok(Self(chain))
    }
    fn world(&self, actor: &Player) -> Vec3 {
        let mut matrix = Mat4::IDENTITY;
        for &index in &self.0 {
            matrix = actor.playback.pose.bones[index].to_matrix() * matrix;
        }
        (actor.transform().to_matrix() * matrix).transform_point3(Vec3::ZERO)
    }
}

fn sequence(
    catalog: &AssetCatalog,
    id: &str,
) -> Result<Vec<shadow_formats::ConfigLine>, Box<dyn Error>> {
    let lines = config_lines(&catalog.read("config/mot_sequencer_defs.txt")?)?;
    let starts = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.words == ["BEGIN_SEQUENCE", id])
        .map(|(i, _)| i)
        .collect::<Vec<_>>();
    let [start] = starts.as_slice() else {
        return Err(format!("one {id} sequence required").into());
    };
    let end = lines[*start + 1..]
        .iter()
        .position(|l| l.words[0] == "END_SEQUENCE")
        .ok_or("unterminated carry sequence")?
        + start
        + 1;
    Ok(lines[*start + 1..end].to_vec())
}
fn load(catalog: &AssetCatalog, name: &str, actor: &Player) -> Result<Clip, Box<dyn Error>> {
    let path = format!("motions/{}.bmf", name.to_ascii_lowercase());
    Clip::new(
        &BmfMotion::from_bytes(&catalog.read(&path)?)?,
        &actor
            .rig
            .as_ref()
            .ok_or("carry preparation needs rig")?
            .bones,
    )
}
impl Carry {
    pub fn prepare(
        catalog: &AssetCatalog,
        actor: &mut Player,
        combat: &mut Combat,
    ) -> Result<Self, Box<dyn Error>> {
        let lift = sequence(catalog, "MSEQ_STAND_PICK_UP_BODY")?;
        let motions = lift
            .iter()
            .filter(|l| l.words[0] == "MOTION")
            .collect::<Vec<_>>();
        if motions.len() != 2 || motions.iter().any(|l| l.words.len() != 2) {
            return Err("unsupported compound body pickup sequence".into());
        }
        let clip = load(catalog, &motions[0].words[1], actor)?;
        let callbacks = lift
            .iter()
            .filter(|l| l.words.get(2).is_some_and(|s| s == "MCB_PICK_UP_ITEM"))
            .collect::<Vec<_>>();
        let [callback] = callbacks.as_slice() else {
            return Err("body pickup needs one callback".into());
        };
        let frame: usize = callback.words[1].parse()?;
        if frame >= clip.frames.len() {
            return Err("body pickup callback outside clip".into());
        }
        let callback = frame as f32 / clip.fps;
        let lift_index = actor.clips.len();
        actor.clips.push(clip);
        let idle = load(catalog, &motions[1].words[1], actor)?;
        let idle_index = actor.clips.len();
        actor.clips.push(idle);
        let mut movement = [idle_index, 0, 0];
        for (index, id) in ["MSEQ_WALK_CARRYING_BODY", "MSEQ_WALK_CARRYING_BODY_FAST"]
            .into_iter()
            .enumerate()
        {
            let (clip, _) =
                crate::combat::sequence(catalog, id, None, &actor.rig.as_ref().unwrap().bones)?;
            movement[index + 1] = actor.clips.len();
            actor.clips.push(clip);
        }
        // ID 51 is the right shoulder in the original character skeleton. The
        // carried motion places its pelvis (ID 1) at the attachment origin.
        let shoulder = Anchor::new(&actor.rig.as_ref().unwrap().bones, 51)?;
        let mut pelvis = Vec::new();
        let mut body_clips = Vec::new();
        for index in 0..combat.guard_count() {
            let guard = &mut combat.guard_mut(index).guard;
            pelvis.push(Anchor::new(
                &guard.rig.as_ref().ok_or("guard rig required")?.bones,
                1,
            )?);
            let (clip, _) = crate::combat::sequence(
                catalog,
                "MSEQ_BODY_BEING_CARRIED",
                None,
                &guard.rig.as_ref().ok_or("guard rig required")?.bones,
            )?;
            body_clips.push(guard.clips.len());
            guard.clips.push(clip);
        }
        actor.source["body_carry"] = serde_json::json!({"pickup_callback_frame":frame,"pickup_callback_seconds":callback,"body_sequence":"MSEQ_BODY_BEING_CARRIED","attachment_original_verified":false});
        Ok(Self {
            phase: Phase::Idle,
            lift: lift_index,
            callback,
            movement,
            body_clips,
            shoulder,
            pelvis,
            saved_weapon: None,
            message: String::new(),
        })
    }

    pub fn order(
        &mut self,
        target: usize,
        actor: &mut Player,
        combat: &Combat,
        grid: &TerrainGrid,
    ) -> bool {
        if target >= combat.guard_count()
            || !combat.guard_at(target).death_finished()
            || actor.carry_movement.is_some()
            || !actor.selected
            || actor.playback.paused
        {
            return false;
        }
        if actor.order(grid, combat.guard_at(target).guard.position, false) {
            self.phase = Phase::Approaching(target);
            self.message = "Подойти и поднять тело".into();
            true
        } else {
            false
        }
    }
    pub fn cancel_approach(&mut self, actor: &mut Player) {
        if matches!(self.phase, Phase::Approaching(_) | Phase::Lifting(_)) {
            if actor.carry_movement.take().is_some() {
                actor.weapon = self.saved_weapon.take();
            }
            actor.action = None;
            self.phase = Phase::Idle;
            self.message.clear();
        }
    }
    pub fn drop_body(
        &mut self,
        actor: &mut Player,
        combat: &mut Combat,
        grid: &TerrainGrid,
        geometry: &Pickup,
    ) -> bool {
        let Phase::Carrying(target) = self.phase else {
            return false;
        };
        let mut point =
            actor.position + Quat::from_rotation_y(actor.heading) * Vec3::new(0.0, 0.0, -1.0);
        let Some(height) = crate::player::surface(grid, point) else {
            self.message = "Нет места, чтобы положить тело".into();
            return false;
        };
        point.y = height;
        if !geometry.clear_line(actor.position + Vec3::Y * 0.4, point + Vec3::Y * 0.4)
            || (height - actor.position.y).abs() > 0.6
        {
            self.message = "Нет места, чтобы положить тело".into();
            return false;
        }
        let guard = combat.guard_mut(target);
        guard.carried = false;
        guard.guard.position = point;
        guard.guard.heading = actor.heading;
        guard.guard.playback.selected = 4;
        guard.guard.playback.pose = guard.guard.clips[4].sample(guard.guard.clips[4].duration());
        actor.carry_movement = None;
        actor.weapon = self.saved_weapon.take();
        actor.action = None;
        actor.destination = None;
        actor.playback.select(&actor.clips, actor.idle_clip());
        self.phase = Phase::Idle;
        self.message = "Тело положено".into();
        true
    }
    pub fn tick(&mut self, actor: &mut Player, combat: &mut Combat, geometry: &Pickup) {
        match self.phase {
            Phase::Idle => {}
            Phase::Approaching(target) => {
                let guard = combat.guard_at(target);
                if !guard.death_finished() {
                    self.cancel_approach(actor);
                    return;
                }
                let delta = (guard.guard.position - actor.position).with_y(0.0);
                if delta.length() <= 1.1 {
                    if !geometry
                        .clear_line(actor.camera_target(), guard.guard.position + Vec3::Y * 0.3)
                    {
                        self.cancel_approach(actor);
                        return;
                    }
                    if actor.posture != crate::stance::Posture::Stand || actor.changing_posture() {
                        actor.request_posture(crate::stance::Posture::Stand);
                        return;
                    }
                    actor.heading = (-delta.x).atan2(-delta.z);
                    self.saved_weapon = actor.weapon.take();
                    actor.carry_movement = Some(self.movement);
                    actor.begin_action(self.lift);
                    self.phase = Phase::Lifting(target);
                } else if actor.destination.is_none() {
                    self.cancel_approach(actor);
                }
            }
            Phase::Lifting(target) => {
                let Some((index, seconds)) = actor.action else {
                    self.cancel_approach(actor);
                    return;
                };
                if index != self.lift {
                    self.cancel_approach(actor);
                    return;
                }
                if seconds >= self.callback {
                    combat.guard_mut(target).carried = true;
                    self.phase = Phase::Carrying(target);
                    self.message = "Джон несёт тело · D: положить".into();
                }
            }
            Phase::Carrying(_) => {
                if actor
                    .action
                    .is_some_and(|(i, t)| i == self.lift && t >= actor.clips[self.lift].duration())
                {
                    actor.action = None;
                    actor.playback.select(&actor.clips, actor.idle_clip());
                }
            }
        }
        if let Phase::Carrying(target) = self.phase {
            let shoulder = self.shoulder.world(actor);
            let guard = combat.guard_mut(target);
            guard.guard.position = actor.position;
            guard.guard.heading = actor.heading;
            guard.guard.playback.selected = self.body_clips[target];
            guard.guard.playback.pose = guard.guard.clips[self.body_clips[target]].sample(0.0);
            guard.guard.position += shoulder - self.pelvis[target].world(&guard.guard);
        }
    }
    pub fn clicked(
        &self,
        combat: &Combat,
        camera: &Camera,
        transform: &GlobalTransform,
        cursor: Vec2,
        geometry: &Pickup,
    ) -> Option<usize> {
        (0..combat.guard_count()).find(|&i| {
            let guard = combat.guard_at(i);
            if !guard.death_finished()
                || guard.carried
                || !geometry.clear_line(
                    transform.translation(),
                    guard.guard.position + Vec3::Y * 0.2,
                )
            {
                return false;
            }
            let Ok(center) =
                camera.world_to_viewport(transform, guard.guard.position + Vec3::Y * 0.2)
            else {
                return false;
            };
            let radius = camera
                .world_to_viewport(transform, guard.guard.position + Vec3::X * 0.9)
                .map(|p| p.distance(center))
                .unwrap_or(10.0)
                .max(12.0);
            center.distance(cursor) <= radius
        })
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn input(
    carry: Option<ResMut<Carry>>,
    actor: Option<ResMut<Player>>,
    combat: Option<ResMut<Combat>>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    preview: Res<Preview>,
    window: Single<&Window>,
    camera: Single<(&Camera, &GlobalTransform), With<MapCamera>>,
    tutorial: Option<Res<Tutorial>>,
    inventory: Option<ResMut<Inventory>>,
    geometry: Option<ResMut<Pickup>>,
    selection: Option<Res<crate::selection::Selection>>,
    actions: (
        Option<ResMut<crate::grenade::Grenade>>,
        Option<ResMut<crate::fence::Fence>>,
        Option<ResMut<crate::smg::Uzi>>,
        Option<ResMut<crate::medical::Medical>>,
        Option<ResMut<crate::armor::Armor>>,
        Option<ResMut<crate::vehicle::Vehicle>>,
    ),
) {
    let (
        Some(mut carry),
        Some(mut actor),
        Some(mut combat),
        Some(grid),
        Some(mut geometry),
        Some(mut inventory),
    ) = (
        carry,
        actor,
        combat,
        preview.grid.as_ref(),
        geometry,
        inventory,
    )
    else {
        return;
    };
    if tutorial.is_some_and(|t| t.pausing() || t.consumed_input)
        || inventory.open
        || inventory.consumed_input
    {
        return;
    }
    if keys.just_pressed(KeyCode::Home) {
        carry.phase = Phase::Idle;
        carry.saved_weapon = None;
        carry.message.clear();
        actor.carry_movement = None;
        for i in 0..combat.guard_count() {
            combat.guard_mut(i).carried = false;
        }
        return;
    }
    if actor.playback.paused || !actor.selected {
        return;
    }
    if keys.just_pressed(KeyCode::KeyD) && matches!(carry.phase, Phase::Carrying(_)) {
        carry.drop_body(&mut actor, &mut combat, grid, &geometry);
        keys.reset(KeyCode::KeyD);
        return;
    }
    if keys.just_pressed(KeyCode::KeyH) {
        carry.cancel_approach(&mut actor);
    }
    if selection.as_ref().is_some_and(|s| s.world_blocked) {
        return;
    }
    if matches!(carry.phase, Phase::Carrying(_)) {
        return;
    }
    let (camera, transform) = *camera;
    let target = selection
        .as_ref()
        .and_then(|s| s.test_cursor)
        .or_else(|| window.cursor_position())
        .and_then(|cursor| carry.clicked(&combat, camera, transform, cursor, &geometry));
    if !mouse.just_pressed(MouseButton::Left) {
        if carry.phase == Phase::Idle {
            carry.message = if target.is_some() {
                "ЛКМ: поднять тело".into()
            } else {
                String::new()
            };
        }
        return;
    }
    carry.cancel_approach(&mut actor);
    let Some(target) = target else {
        return;
    };
    combat.cancel_all(&mut actor);
    inventory.cancel();
    geometry.cancel();
    let (grenade, fence, uzi, medical, armor, vehicle) = actions;
    if let Some(mut g) = grenade {
        g.cancel(&mut actor);
    }
    if let Some(mut f) = fence {
        f.cancel(&mut actor);
    }
    if let Some(mut u) = uzi {
        u.cancel(&mut actor);
    }
    if let Some(mut m) = medical {
        m.cancel(&mut actor);
    }
    if let Some(mut a) = armor {
        a.ground.cancel(&mut actor);
    }
    if let Some(mut v) = vehicle {
        v.cancel_approach(&mut actor);
    }
    if carry.order(target, &mut actor, &combat, grid) {
        mouse.reset(MouseButton::Left);
    }
}

pub fn tick(
    carry: Option<ResMut<Carry>>,
    actor: Option<ResMut<Player>>,
    combat: Option<ResMut<Combat>>,
    preview: Res<Preview>,
    geometry: Option<Res<Pickup>>,
    inventory: Option<Res<Inventory>>,
    tutorial: Option<Res<Tutorial>>,
) {
    let (Some(mut carry), Some(mut actor), Some(mut combat), Some(_grid), Some(geometry)) =
        (carry, actor, combat, preview.grid.as_ref(), geometry)
    else {
        return;
    };
    if actor.playback.paused
        || tutorial.is_some_and(|t| t.pausing())
        || inventory.is_some_and(|i| i.open)
    {
        return;
    }
    carry.tick(&mut actor, &mut combat, &geometry);
}

#[derive(Component)]
pub struct Hint;
pub fn ui(
    mut commands: Commands,
    carry: Option<Res<Carry>>,
    inventory: Option<Res<Inventory>>,
    camera: Single<Entity, With<MapCamera>>,
    mut labels: Query<(&mut Text, &mut Visibility), With<Hint>>,
    mut fonts: ResMut<Assets<Font>>,
) {
    let Some(carry) = carry else {
        return;
    };
    let visible = !carry.message.is_empty() && !inventory.is_some_and(|i| i.open);
    if let Ok((mut text, mut visibility)) = labels.single_mut() {
        text.0.clone_from(&carry.message);
        *visibility = if visible {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    } else {
        let font = fonts.add(Font::from_bytes(
            include_bytes!("../../shadow-viewer/assets/fonts/DejaVuSans.ttf").to_vec(),
        ));
        commands.spawn((
            crate::player::SceneEntity,
            Hint,
            UiTargetCamera(*camera),
            bevy::picking::Pickable::IGNORE,
            GlobalZIndex(60),
            Node {
                position_type: PositionType::Absolute,
                left: px(18),
                bottom: px(164),
                padding: UiRect::all(px(8)),
                ..default()
            },
            Text::new(carry.message.clone()),
            TextFont {
                font: font.into(),
                font_size: bevy::text::FontSize::Px(18.0),
                ..default()
            },
            TextColor(Color::srgb(1.0, 0.88, 0.5)),
            BackgroundColor(Color::srgba(0.02, 0.03, 0.02, 0.85)),
            if visible {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            },
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Carry, Combat, Player, Pickup, Tutorial, TerrainGrid) {
        let (mut combat, mut actor, geometry, tutorial, mut grid) = crate::combat::tests::fixture();
        grid.heights.fill(0.0);
        actor.position.y = 0.0;
        combat.guard.position.y = 0.0;
        let lift = actor.clips.len();
        actor.clips.push(Clip {
            name: "lift".into(),
            fps: 10.0,
            frames: vec![actor.clips[0].frames[0].clone(); 11],
        });
        let carry = Carry {
            phase: Phase::Idle,
            lift,
            callback: 0.5,
            movement: [0, 1, 2],
            body_clips: vec![4],
            shoulder: Anchor(Vec::new()),
            pelvis: vec![Anchor(Vec::new())],
            saved_weapon: None,
            message: String::new(),
        };
        (carry, combat, actor, geometry, tutorial, grid)
    }
    fn kill(combat: &mut Combat, actor: &mut Player, tutorial: &mut Tutorial, geometry: &Pickup) {
        combat.blast_damage(0, 1000.0, tutorial);
        if tutorial.active.is_some() {
            tutorial.close();
        }
        combat.update(actor, tutorial, geometry, 2.0);
        assert!(combat.death_finished());
    }
    #[test]
    fn only_finished_dead_body_can_be_lifted_and_callback_attaches_once() {
        let (mut carry, mut combat, mut actor, geometry, mut tutorial, grid) = fixture();
        assert!(!carry.order(0, &mut actor, &combat, &grid));
        kill(&mut combat, &mut actor, &mut tutorial, &geometry);
        assert!(carry.order(0, &mut actor, &combat, &grid));
        carry.tick(&mut actor, &mut combat, &geometry);
        assert_eq!(carry.phase, Phase::Lifting(0));
        actor.step(&ButtonInput::default(), 0.49, &grid);
        carry.tick(&mut actor, &mut combat, &geometry);
        assert!(!combat.carried);
        actor.step(&ButtonInput::default(), 0.02, &grid);
        carry.tick(&mut actor, &mut combat, &geometry);
        assert!(combat.carried);
        assert_eq!(carry.phase, Phase::Carrying(0));
        assert_eq!(combat.guard.position, actor.position);
        assert!(!actor.request_posture(crate::stance::Posture::Prone));
        assert!(!combat.order_target(0, &mut actor, &grid));
    }
    #[test]
    fn cancel_before_callback_keeps_body_on_ground_and_restores_weapon() {
        let (mut carry, mut combat, mut actor, geometry, mut tutorial, grid) = fixture();
        kill(&mut combat, &mut actor, &mut tutorial, &geometry);
        actor.weapon = Some("knife".into());
        let original = combat.guard.position;
        assert!(carry.order(0, &mut actor, &combat, &grid));
        carry.tick(&mut actor, &mut combat, &geometry);
        assert!(actor.weapon.is_none());
        carry.cancel_approach(&mut actor);
        assert_eq!(carry.phase, Phase::Idle);
        assert!(!combat.carried && actor.carry_movement.is_none());
        assert_eq!(combat.guard.position, original);
        assert_eq!(actor.weapon.as_deref(), Some("knife"));
    }
    #[test]
    fn invalid_drop_keeps_body_attached_and_valid_drop_restores_equipment() {
        let (mut carry, mut combat, mut actor, geometry, mut tutorial, grid) = fixture();
        kill(&mut combat, &mut actor, &mut tutorial, &geometry);
        actor.weapon = Some("knife".into());
        assert!(carry.order(0, &mut actor, &combat, &grid));
        carry.tick(&mut actor, &mut combat, &geometry);
        actor.step(&ButtonInput::default(), 0.6, &grid);
        carry.tick(&mut actor, &mut combat, &geometry);
        let position = actor.position;
        actor.position = Vec3::splat(1_000_000.0);
        assert!(!carry.drop_body(&mut actor, &mut combat, &grid, &geometry));
        assert!(combat.carried && actor.carry_movement.is_some());
        actor.position = position;
        actor.heading = 0.0;
        // The synthetic grid is small; choose a point one metre from its edge.
        actor.position.x = 0.5;
        actor.position.z = -0.5;
        if !carry.drop_body(&mut actor, &mut combat, &grid, &geometry) {
            actor.heading = std::f32::consts::PI;
            assert!(carry.drop_body(&mut actor, &mut combat, &grid, &geometry));
        }
        assert!(!combat.carried && actor.carry_movement.is_none());
        assert_eq!(actor.weapon.as_deref(), Some("knife"));
        assert_eq!(carry.phase, Phase::Idle);
    }
}
