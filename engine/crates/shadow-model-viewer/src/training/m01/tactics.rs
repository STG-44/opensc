//! Tactical orders and direct movement share the player animation and collisions.
use super::Squad;
use crate::{
    localization::Language,
    map_scene::WORLD_SCALE,
    orders,
    player::Player,
    selection::Gesture,
    stance::Posture,
    training::{DIAGNOSTIC_PANEL_WIDTH, MapCamera, Orbit, Preview, actor_clicked},
};
use bevy::prelude::*;
use shadow_formats::TerrainGrid;

#[derive(Resource, Default)]
pub(crate) struct TacticalInput {
    latched: bool,
    direct: Option<usize>,
    previous_follow: Option<usize>,
    gesture: Option<Gesture>,
    clicks: orders::ClickHistory,
}

impl TacticalInput {
    pub fn direct(&self) -> bool {
        self.direct.is_some()
    }

    pub(super) fn direct_actor(&self) -> Option<usize> {
        self.direct
    }

    pub(crate) fn gesture(&self) -> Option<Gesture> {
        self.gesture
    }

    fn set_direct(&mut self, target: Option<usize>, squad: &mut Squad, orbit: &mut Orbit) {
        if self.direct == target {
            return;
        }
        if let Some(index) = self.direct {
            stop(&mut squad.members[index].actor);
            squad.following = self.previous_follow.take();
        }
        self.gesture = None;
        self.clicks.0 = None;
        self.direct = target;
        if let Some(index) = target {
            let member = &mut squad.members[index];
            member.clear_orders();
            self.previous_follow = squad.following;
            squad.following = target;
            orbit.target = member.actor.camera_target();
        }
    }
}

fn stop(actor: &mut Player) {
    actor.destination = None;
    actor.route.clear();
}

fn pressed(keys: &ButtonInput<KeyCode>, a: KeyCode, b: KeyCode) -> f32 {
    f32::from(keys.pressed(a) || keys.pressed(b))
}

fn direction(keys: &ButtonInput<KeyCode>, yaw: f32) -> Vec3 {
    let x = pressed(keys, KeyCode::KeyD, KeyCode::ArrowRight)
        - pressed(keys, KeyCode::KeyA, KeyCode::ArrowLeft);
    let y = pressed(keys, KeyCode::KeyW, KeyCode::ArrowUp)
        - pressed(keys, KeyCode::KeyS, KeyCode::ArrowDown);
    let (sin, cos) = yaw.sin_cos();
    let right = Vec3::new(cos, 0., -sin);
    let forward = Vec3::new(-sin, 0., -cos);
    (right * x + forward * y).normalize_or_zero()
}

fn step_posture(keys: &ButtonInput<KeyCode>, current: Posture) -> Option<Posture> {
    if keys.just_pressed(KeyCode::Space) {
        return Some(if current == Posture::Prone {
            Posture::Stand
        } else {
            Posture::Prone
        });
    }
    let up = keys.just_pressed(KeyCode::PageUp);
    let down = keys.just_pressed(KeyCode::PageDown);
    match (current, up, down) {
        (Posture::Crouch, true, false) => Some(Posture::Stand),
        (Posture::Crouch, false, true) => Some(Posture::Prone),
        (Posture::Stand, false, true) | (Posture::Prone, true, false) => Some(Posture::Crouch),
        _ => None,
    }
}

fn direct_order(
    actor: &mut Player,
    grid: &TerrainGrid,
    keys: &ButtonInput<KeyCode>,
    yaw: f32,
    aim: Option<Ray3d>,
) {
    let direction = direction(keys, yaw);
    if direction == Vec3::ZERO {
        if actor.destination.is_some() {
            stop(actor);
        }
        if let Some(ray) = aim
            && ray.direction.y < -0.001
        {
            let distance = (actor.position.y - ray.origin.y) / ray.direction.y;
            let facing = (ray.get_point(distance) - actor.position).xz();
            if distance > 0. && facing.length_squared() > 0.04 {
                actor.heading = (-facing.x).atan2(-facing.y);
            }
        }
        return;
    }
    let running = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    let [east, north] = grid.extent().map(|units| units * WORLD_SCALE);
    let point = actor.position + direction * 2.5;
    let point = Vec3::new(point.x.clamp(0., east), point.y, point.z.clamp(-north, 0.));
    actor.order(grid, point, running);
}

#[allow(clippy::too_many_arguments)]
pub(super) fn input(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    time: Res<Time>,
    window: Single<&Window>,
    camera: Single<(&Camera, &GlobalTransform), With<MapCamera>>,
    preview: Res<Preview>,
    mut squad: ResMut<Squad>,
    mut state: ResMut<TacticalInput>,
    mut orbit: ResMut<Orbit>,
) {
    if preview.screenshot.is_some() {
        return;
    }
    let Some(grid) = &preview.grid else { return };
    let ctrl = keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]);
    let home = keys.just_pressed(KeyCode::Home);
    let pause = keys.just_pressed(KeyCode::Escape) || (ctrl && keys.just_pressed(KeyCode::KeyP));
    if home || pause || !window.focused {
        state.latched = false;
        state.set_direct(None, &mut squad, &mut orbit);
        state.gesture = None;
        state.clicks.0 = None;
        if home && window.focused {
            squad.reset(&mut orbit);
        }
        if pause && window.focused {
            squad.paused = !squad.paused;
        }
        return;
    }
    if squad.paused {
        return;
    }

    let command_actor = squad.command_actor();
    if command_actor.is_some() && keys.just_pressed(KeyCode::End) {
        state.latched = !state.latched;
    }
    let direct = command_actor.filter(|_| state.latched || ctrl);
    state.set_direct(direct, &mut squad, &mut orbit);
    if keys.just_pressed(KeyCode::KeyH) {
        squad.halt(direct);
        state.gesture = None;
        state.clicks.0 = None;
        return;
    }
    for (index, member) in squad.members.iter_mut().enumerate() {
        if direct.map_or(member.actor.selected, |target| target == index)
            && let Some(posture) = step_posture(&keys, member.actor.posture)
        {
            member.actor.request_posture(posture);
        }
    }

    let (camera, transform) = *camera;
    let in_world = |p: Vec2| {
        p.is_finite()
            && p.x >= DIAGNOSTIC_PANEL_WIDTH
            && p.x <= window.width()
            && p.y >= 0.
            && p.y <= window.height()
    };
    let cursor = window.cursor_position();
    if let Some(index) = direct {
        let aim = cursor
            .filter(|&p| in_world(p))
            .and_then(|p| camera.viewport_to_world(transform, p).ok());
        direct_order(&mut squad.members[index].actor, grid, &keys, orbit.yaw, aim);
        return;
    }

    for (key, mask) in [
        (KeyCode::Digit1, 1),
        (KeyCode::Digit2, 2),
        (KeyCode::Digit3, u8::MAX),
    ] {
        if keys.just_pressed(key) {
            squad.select(mask);
            state.clicks.0 = None;
        }
    }
    if keys.just_pressed(KeyCode::KeyK) {
        squad.promote();
    }
    if keys.just_pressed(KeyCode::KeyG) {
        squad.regroup();
    }
    if keys.just_pressed(KeyCode::KeyT)
        && let Some(index) = squad.command_actor()
    {
        squad.following = (squad.following != Some(index)).then_some(index);
        if squad.following.is_some() {
            orbit.target = squad.members[index].actor.camera_target();
        }
    }
    if mouse.just_pressed(MouseButton::Right) || cursor.is_none() {
        state.gesture = None;
        return;
    }
    let cursor = cursor.unwrap();
    if state.gesture.is_none() && mouse.just_pressed(MouseButton::Left) && in_world(cursor) {
        let add = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
        state.gesture = Some(Gesture::new(cursor, add, squad.mask(), false));
    }
    let Some(gesture) = &mut state.gesture else {
        return;
    };
    gesture.update(cursor);
    if mouse.pressed(MouseButton::Left) {
        return;
    }
    let gesture = state.gesture.take().unwrap();
    if gesture.dragged {
        let inside = squad.actors().enumerate().fold(0, |mask, (index, actor)| {
            let contains = camera
                .world_to_viewport(transform, actor.camera_target())
                .is_ok_and(|point| in_world(point) && gesture.rect().contains(point));
            mask | (u8::from(contains) << index)
        });
        squad.select(gesture.result(inside));
        state.clicks.0 = None;
        return;
    }
    if !in_world(cursor) {
        state.clicks.0 = None;
        return;
    }
    if let Some((index, _)) = squad
        .actors()
        .enumerate()
        .filter(|(_, actor)| actor_clicked(camera, transform, gesture.start, actor))
        .min_by(|(_, a), (_, b)| {
            transform
                .translation()
                .distance_squared(a.position)
                .total_cmp(&transform.translation().distance_squared(b.position))
        })
    {
        let bit = 1 << index;
        let mask = if gesture.additive {
            squad.mask() ^ bit
        } else {
            bit
        };
        squad.select(mask);
        state.clicks.0 = None;
    } else if squad.mask() != 0
        && let Ok(ray) = camera.viewport_to_world(transform, gesture.start)
        && let Some(point) = orders::terrain_hit(grid, ray.origin, ray.direction.as_vec3())
    {
        let running = if gesture.additive {
            state.clicks.0 = None;
            false
        } else {
            state
                .clicks
                .register(time.elapsed_secs_f64(), gesture.start)
        };
        squad.order(grid, point, running, gesture.additive);
    }
}

pub(super) fn status(
    state: Res<TacticalInput>,
    squad: Res<Squad>,
    language: Res<Language>,
    mut labels: Query<&mut Text, With<crate::player::Status>>,
) {
    let russian = *language == Language::Russian;
    let mode = match (russian, squad.paused, state.direct()) {
        (true, true, _) => "Пауза",
        (false, true, _) => "Paused",
        (true, false, true) => "Прямое управление",
        (false, false, true) => "Direct control",
        (true, false, false) => "Тактические приказы",
        (false, false, false) => "Tactical orders",
    };
    let commander = if russian {
        "Командир"
    } else {
        "Commander"
    };
    let name = &squad.members[squad.commander].actor.display_name;
    let mut text = format!("{mode}\n{commander} · {name}");
    for (i, member) in squad.members.iter().enumerate() {
        if !state
            .direct_actor()
            .map_or(member.actor.selected, |index| index == i)
        {
            continue;
        }
        let actor = &member.actor;
        let posture = if russian {
            actor.posture.name()
        } else {
            ["Standing", "Crouched", "Prone"][actor.posture.index()]
        };
        let detail = if actor.route.failed {
            if russian {
                "Нет прохода".into()
            } else {
                "No route".into()
            }
        } else if member.in_formation && state.direct_actor() != Some(i) {
            if russian {
                "Следует за командиром".into()
            } else {
                "Following the commander".into()
            }
        } else {
            let queued = if russian {
                "В очереди"
            } else {
                "Queued"
            };
            format!("{queued} {}", member.queued.len())
        };
        text.push_str(&format!(
            "
{} · {}
{posture} · {detail}",
            i + 1,
            actor.display_name
        ));
    }
    for mut label in &mut labels {
        label.set_if_neq(Text::new(text.clone()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_movement_is_camera_relative_and_diagonals_are_normalized() {
        let mut keys = ButtonInput::default();
        keys.press(KeyCode::KeyW);
        assert_eq!(direction(&keys, 0.), Vec3::NEG_Z);
        let rotated = direction(&keys, std::f32::consts::FRAC_PI_2);
        assert!(rotated.distance(Vec3::NEG_X) < 1e-6);
        keys.press(KeyCode::ArrowRight);
        assert!((direction(&keys, 0.).length() - 1.).abs() < 1e-6);
    }

    #[test]
    fn direct_control_preserves_the_other_members_active_order_and_queue() {
        let mut squad = Squad::new([
            crate::player::tests::player(),
            crate::player::tests::player(),
        ]);
        let grid = crate::player::tests::grid();
        squad.select(1);
        squad.order(&grid, Vec3::new(1.9, 0., -0.9), false, false);
        squad.select(2);
        squad.order(&grid, Vec3::new(1.9, 0., -0.9), false, false);
        squad.order(&grid, Vec3::new(1.8, 0., -1.6), false, true);
        squad.select(3);
        let other = squad.members[1].actor.destination.unwrap();
        let mut state = TacticalInput::default();
        let mut orbit = crate::training::player_view(Vec3::ZERO);
        state.set_direct(Some(0), &mut squad, &mut orbit);
        assert_eq!(squad.following, Some(0));
        assert!(squad.members[0].actor.destination.is_none());
        assert!(squad.members[0].queued.is_empty());
        assert_eq!(squad.members[1].actor.destination, Some(other));
        assert_eq!(squad.members[1].queued.len(), 1);
        let before = squad.members[1].actor.position;
        squad.step(&grid, None, state.direct_actor(), 0.2);
        assert_ne!(squad.members[1].actor.position, before);
        state.set_direct(None, &mut squad, &mut orbit);
        assert!(squad.following.is_none());
        assert_eq!(squad.members[1].actor.destination, Some(other));
    }

    #[test]
    fn selection_preserves_orders_and_does_not_reassign_the_commander() {
        let mut squad = Squad::new([
            crate::player::tests::player(),
            crate::player::tests::player(),
        ]);
        squad.select(1);
        squad.members[0].actor.destination = Some(Vec3::X);
        let mut rectangle = Gesture::new(Vec2::ZERO, true, squad.mask(), false);
        rectangle.update(Vec2::splat(10.));
        assert!(rectangle.dragged);
        squad.select(rectangle.result(2));
        assert_eq!(squad.mask(), 3);
        squad.select(2);
        assert_eq!(squad.command_actor(), Some(1));
        assert_eq!(squad.commander, 0);
        assert_eq!(squad.members[0].actor.destination, Some(Vec3::X));
        squad.promote();
        assert_eq!(squad.commander, 1);
        assert!(squad.members[0].in_formation);
        assert!(!squad.members[1].in_formation);
        assert!(squad.members[0].actor.destination.is_none());
    }

    #[test]
    fn mouse_box_and_ground_click_use_the_actual_squad_input_system() {
        use bevy::camera::{CameraProjection, RenderTargetInfo};
        let mut grid = crate::player::tests::grid();
        grid.config.edge = 1000.;
        grid.heights.fill(0.);
        let mut members = [
            crate::player::tests::player(),
            crate::player::tests::player(),
        ];
        members[0].position = Vec3::new(8., 0., -5.);
        members[1].position = Vec3::new(12., 0., -5.);
        let mut squad = Squad::new(members);
        squad.select(1);
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
        let view = GlobalTransform::from(
            Transform::from_xyz(10., 10., 5.).looking_at(Vec3::new(10., 0., -10.), Vec3::Y),
        );
        let positions = squad
            .actors()
            .map(|actor| {
                camera
                    .world_to_viewport(&view, actor.camera_target())
                    .unwrap()
            })
            .collect::<Vec<_>>();
        let ground = camera
            .world_to_viewport(&view, Vec3::new(10., 0., -13.))
            .unwrap();
        let from = positions[0].min(positions[1]) - Vec2::splat(20.);
        let to = positions[0].max(positions[1]) + Vec2::splat(20.);
        let mut app = App::new();
        app.insert_resource(Time::<()>::default())
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<TacticalInput>()
            .insert_resource(squad)
            .insert_resource(Preview {
                scene: None,
                screenshot: None,
                focus: Vec3::ZERO,
                span: 30.,
                grid: Some(grid),
                playable: true,
                first_area: true,
            })
            .insert_resource(crate::training::player_view(Vec3::ZERO))
            .add_systems(Update, input);
        let window = app
            .world_mut()
            .spawn(Window {
                focused: true,
                resolution: (1280, 800).into(),
                ..default()
            })
            .id();
        app.world_mut().spawn((camera, view, MapCamera));

        let point = |app: &mut App, pos| {
            app.world_mut()
                .get_mut::<Window>(window)
                .unwrap()
                .set_cursor_position(Some(pos));
        };
        point(&mut app, from);
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app.update();
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .clear_just_pressed(MouseButton::Left);
        point(&mut app, to);
        app.update();
        assert!(
            app.world()
                .resource::<TacticalInput>()
                .gesture
                .unwrap()
                .dragged
        );
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .release(MouseButton::Left);
        app.update();
        assert_eq!(app.world().resource::<Squad>().mask(), 3);
        assert!(
            app.world()
                .resource::<Squad>()
                .actors()
                .all(|actor| actor.destination.is_none())
        );

        point(&mut app, ground);
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app.update();
        assert!(
            app.world()
                .resource::<Squad>()
                .actors()
                .all(|actor| actor.destination.is_none())
        );
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .release(MouseButton::Left);
        app.update();
        let squad = app.world().resource::<Squad>();
        assert!(squad.members[0].actor.destination.is_some());
        assert!(squad.members[1].actor.destination.is_none());
        assert!(squad.members[1].in_formation);

        let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        keys.press(KeyCode::Digit2);
        app.update();
        assert_eq!(app.world().resource::<Squad>().commander, 0);
        let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        keys.reset_all();
        keys.press(KeyCode::KeyK);
        app.update();
        assert_eq!(app.world().resource::<Squad>().commander, 1);
    }

    #[test]
    fn attached_member_follows_direct_movement_until_the_squad_halts() {
        let mut grid = crate::player::tests::grid();
        grid.config.edge = 1_000.;
        grid.heights.fill(0.);
        let mut leader = crate::player::tests::player();
        leader.position = Vec3::new(10., 0., -10.);
        let mut follower = crate::player::tests::player();
        follower.position = Vec3::new(10., 0., -7.);
        let mut squad = Squad::new([leader, follower]);
        let mut state = TacticalInput::default();
        let mut orbit = crate::training::player_view(Vec3::ZERO);
        state.set_direct(Some(0), &mut squad, &mut orbit);
        let mut keys = ButtonInput::default();
        keys.press(KeyCode::KeyW);
        for _ in 0..240 {
            direct_order(&mut squad.members[0].actor, &grid, &keys, 0., None);
            squad.step(&grid, None, state.direct_actor(), 0.025);
        }
        assert!(squad.members[1].actor.position.z < -12.);
        assert!(squad.members[1].actor.position.z > squad.members[0].actor.position.z + 1.);
        squad.halt(state.direct_actor());
        let stopped = squad.members[1].actor.position;
        squad.step(&grid, None, state.direct_actor(), 0.025);
        assert_eq!(squad.members[1].actor.position, stopped);
    }

    #[test]
    fn posture_shortcuts_step_and_toggle() {
        let mut keys = ButtonInput::default();
        keys.press(KeyCode::PageDown);
        assert_eq!(step_posture(&keys, Posture::Stand), Some(Posture::Crouch));
        assert_eq!(step_posture(&keys, Posture::Crouch), Some(Posture::Prone));
        keys.reset_all();
        keys.press(KeyCode::Space);
        assert_eq!(step_posture(&keys, Posture::Prone), Some(Posture::Stand));
    }
}
