//! Tactical orders and direct movement share the player animation and collisions.
use crate::{
    localization::Language,
    map_scene::WORLD_SCALE,
    orders,
    player::Player,
    stance::Posture,
    training::{DIAGNOSTIC_PANEL_WIDTH, MapCamera, Orbit, Preview, actor_clicked},
};
use bevy::prelude::*;
use shadow_formats::TerrainGrid;
use std::collections::VecDeque;

#[derive(Resource, Default)]
pub(crate) struct TacticalInput {
    latched: bool,
    direct: bool,
    previous_follow: bool,
    queued: VecDeque<Vec3>,
    clicks: orders::ClickHistory,
}

impl TacticalInput {
    pub fn direct(&self) -> bool {
        self.direct
    }

    fn set_direct(&mut self, enabled: bool, actor: &mut Player) {
        if self.direct == enabled {
            return;
        }
        stop(actor);
        self.queued.clear();
        self.clicks.0 = None;
        self.direct = enabled;
        if enabled {
            self.previous_follow = actor.follow_camera;
            actor.follow_camera = true;
        } else {
            actor.follow_camera = self.previous_follow;
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
    mut actor: ResMut<Player>,
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
    let halt = keys.just_pressed(KeyCode::KeyH);
    if home || halt {
        state.queued.clear();
        state.clicks.0 = None;
    }
    if home || pause || !window.focused || actor.playback.paused {
        state.latched = false;
        state.set_direct(false, &mut actor);
        state.clicks.0 = None;
        return;
    }
    if keys.just_pressed(KeyCode::End) && actor.selected {
        state.latched = !state.latched;
    }
    let direct = actor.selected && (state.latched || ctrl);
    let entered = direct && !state.direct;
    state.set_direct(direct, &mut actor);
    if entered {
        orbit.target = actor.camera_target();
    }
    if halt {
        stop(&mut actor);
        return;
    }
    if let Some(posture) = step_posture(&keys, actor.posture) {
        actor.request_posture(posture);
    }
    let (camera, transform) = *camera;
    let cursor = window
        .cursor_position()
        .filter(|p| p.x >= DIAGNOSTIC_PANEL_WIDTH);
    let ray = || cursor.and_then(|p| camera.viewport_to_world(transform, p).ok());
    if direct {
        direct_order(&mut actor, grid, &keys, orbit.yaw, ray());
        return;
    }
    if actor.route.failed {
        state.queued.clear();
    } else if actor.destination.is_none()
        && let Some(next) = state.queued.pop_front()
    {
        actor.order(grid, next, false);
    }
    if !mouse.just_pressed(MouseButton::Left) {
        return;
    }
    let Some(cursor) = cursor else { return };
    if actor_clicked(camera, transform, cursor, &actor) {
        actor.selected = true;
        state.clicks.0 = None;
        return;
    }
    if !actor.selected {
        return;
    }
    let Some(point) =
        ray().and_then(|r| orders::terrain_hit(grid, r.origin, r.direction.as_vec3()))
    else {
        return;
    };
    let append = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    if append {
        state.clicks.0 = None;
        if actor.destination.is_some() {
            if state.queued.len() < 16 {
                state.queued.push_back(point);
            }
        } else {
            actor.order(grid, point, false);
        }
    } else {
        state.queued.clear();
        let running = state.clicks.register(time.elapsed_secs_f64(), cursor);
        actor.order(grid, point, running);
    }
}

pub(super) fn status(
    state: Res<TacticalInput>,
    actor: Res<Player>,
    language: Res<Language>,
    mut labels: Query<&mut Text, With<crate::player::Status>>,
) {
    let russian = *language == Language::Russian;
    let mode = match (russian, state.direct) {
        (true, true) => "Прямое управление",
        (true, false) => "Тактические приказы",
        (false, true) => "Direct control",
        (false, false) => "Tactical orders",
    };
    let posture = if russian {
        actor.posture.name()
    } else {
        ["Standing", "Crouched", "Prone"][actor.posture.index()]
    };
    let info = if actor.route.failed {
        if russian {
            "Нет прохода".into()
        } else {
            "No route".into()
        }
    } else {
        let queued = if russian {
            "В очереди"
        } else {
            "Queued"
        };
        format!("{queued} {}", state.queued.len())
    };
    for mut label in &mut labels {
        label.set_if_neq(Text::new(format!("{mode}\n{posture} · {info}")));
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
    fn direct_control_cancels_its_actors_orders_and_restores_camera_follow() {
        let mut actor = crate::player::tests::player();
        actor.destination = Some(Vec3::X);
        let mut input = TacticalInput::default();
        input.queued.push_back(Vec3::Z);
        input.set_direct(true, &mut actor);
        assert!(actor.follow_camera && input.direct());
        assert!(actor.destination.is_none() && input.queued.is_empty());
        let mut keys = ButtonInput::default();
        keys.press(KeyCode::KeyW);
        direct_order(&mut actor, &crate::player::tests::grid(), &keys, 0., None);
        assert!(actor.destination.is_some());
        input.set_direct(false, &mut actor);
        assert!(!actor.follow_camera && !input.direct() && actor.destination.is_none());
    }

    #[test]
    fn m01_input_switches_modes_and_stop_clears_pending_orders() {
        let mut app = App::new();
        app.init_resource::<Time>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<TacticalInput>()
            .insert_resource(crate::player::tests::player())
            .insert_resource(Preview {
                scene: None,
                screenshot: None,
                focus: Vec3::ZERO,
                span: 3.,
                grid: Some(crate::player::tests::grid()),
                playable: true,
                first_area: true,
            })
            .insert_resource(Orbit {
                target: Vec3::ZERO,
                yaw: 0.,
                pitch: 0.8,
                distance: 8.,
            })
            .add_systems(Update, input);
        app.world_mut().spawn(Window {
            focused: true,
            ..default()
        });
        app.world_mut()
            .spawn((Camera::default(), GlobalTransform::default(), MapCamera));
        let press = |app: &mut App, pressed: &[KeyCode]| {
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keys.reset_all();
            for &key in pressed {
                keys.press(key);
            }
        };
        press(&mut app, &[KeyCode::End, KeyCode::KeyW]);
        app.update();
        assert!(app.world().resource::<TacticalInput>().direct());
        assert!(app.world().resource::<Player>().destination.is_some());
        press(&mut app, &[KeyCode::End]);
        app.update();
        assert!(!app.world().resource::<TacticalInput>().direct());
        assert!(app.world().resource::<Player>().destination.is_none());
        press(&mut app, &[KeyCode::ControlLeft]);
        app.update();
        assert!(app.world().resource::<TacticalInput>().direct());
        press(&mut app, &[]);
        app.update();
        assert!(!app.world().resource::<TacticalInput>().direct());

        app.world_mut()
            .resource_mut::<TacticalInput>()
            .queued
            .extend([Vec3::new(1., 0., -1.), Vec3::new(2., 0., -1.)]);
        app.update();
        assert_eq!(app.world().resource::<TacticalInput>().queued.len(), 1);
        assert!(app.world().resource::<Player>().destination.is_some());
        press(&mut app, &[KeyCode::KeyH]);
        app.update();
        assert!(app.world().resource::<TacticalInput>().queued.is_empty());
        assert!(app.world().resource::<Player>().destination.is_none());
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
