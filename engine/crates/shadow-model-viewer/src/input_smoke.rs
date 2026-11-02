//! Native regression through Bevy's mouse messages, UI picking and widget events.
use super::*;
use bevy::{
    input::{ButtonState, mouse::MouseButtonInput},
    window::{CursorMoved, PrimaryWindow},
};

#[derive(Resource)]
pub(super) struct Smoke {
    path: PathBuf,
    stage: u8,
    frames: u32,
    changed: u32,
    checks: Vec<String>,
}
impl Smoke {
    pub(super) fn new(path: PathBuf) -> Self {
        Self {
            path,
            stage: 0,
            frames: 0,
            changed: 0,
            checks: Vec::new(),
        }
    }
    fn advance(&mut self, check: &str) {
        if !check.is_empty() {
            self.checks.push(check.into());
        }
        self.stage += 1;
        self.changed = self.frames;
    }
}
fn mouse(world: &mut World, down: bool) {
    let window = world
        .query_filtered::<Entity, With<PrimaryWindow>>()
        .single(world)
        .unwrap();
    let event = MouseButtonInput {
        window,
        button: MouseButton::Left,
        state: if down {
            ButtonState::Pressed
        } else {
            ButtonState::Released
        },
    };
    world.write_message(event);
    world.write_message(bevy::window::WindowEvent::MouseButtonInput(event));
}
fn point(world: &mut World, entity: Entity) {
    let transform = world
        .get::<UiGlobalTransform>(entity)
        .expect("laid-out UI transform");
    let (_, _, position) = transform.to_scale_angle_translation();
    let scale = world
        .get::<ComputedNode>(entity)
        .unwrap()
        .inverse_scale_factor();
    let position = position * scale;
    let window = world
        .query_filtered::<Entity, With<PrimaryWindow>>()
        .single(world)
        .unwrap();
    let w = world.get::<Window>(window).unwrap();
    assert!(
        position.x >= 0.0 && position.y >= 0.0 && position.x < w.width() && position.y < w.height(),
        "button outside window: {position}"
    );
    world
        .resource_mut::<crate::selection::Selection>()
        .test_cursor = Some(position);
    let event = CursorMoved {
        window,
        position,
        delta: None,
    };
    world.write_message(event.clone());
    world.write_message(bevy::window::WindowEvent::CursorMoved(event));
}
fn hud(world: &mut World, id: &str) -> Entity {
    world
        .query::<(Entity, &crate::hud::CommandButton)>()
        .iter(world)
        .find_map(|(e, b)| (b.0 == id).then_some(e))
        .expect("HUD button")
}
fn opened(world: &World) -> bool {
    world.resource::<crate::inventory::Inventory>().open
}

pub(super) fn run(world: &mut World) {
    let mut smoke = world.remove_resource::<Smoke>().unwrap();
    smoke.frames += 1;
    assert!(
        smoke.frames < 6000,
        "input smoke timed out at {}",
        smoke.stage
    );
    if smoke.stage >= 1 && world.contains_resource::<crate::tutorial::Tutorial>() {
        let mut tutorial = world.resource_mut::<crate::tutorial::Tutorial>();
        if tutorial.active.is_some() {
            tutorial.close();
        }
        tutorial.consumed_input = false;
    }
    let ready = smoke.frames - smoke.changed >= 8;
    let screen = *world.resource::<State<Screen>>().get();
    match smoke.stage {
        0 if ready && screen == Screen::Main => {
            world.write_message(ActionRequest(Action::Training));
            smoke.advance("");
        }
        1 if ready
            && screen == Screen::Training
            && world.contains_resource::<crate::hud::Hud>() =>
        {
            let mut tutorial = world.resource_mut::<crate::tutorial::Tutorial>();
            if tutorial.active.is_some() {
                tutorial.close();
            }
            tutorial.consumed_input = false;
            smoke.advance("");
        }
        2 if ready => {
            let e = hud(world, "b_inventory");
            point(world, e);
            smoke.advance("");
        }
        3 if ready => {
            let e = hud(world, "b_inventory");
            let cursor = world
                .query::<&Window>()
                .single(world)
                .unwrap()
                .cursor_position();
            eprintln!(
                "input-smoke before click: hovered={:?} visual={:?} selected={} help={} consumed={} cursor={:?}",
                world.get::<bevy::picking::hover::Hovered>(e),
                world.get::<ButtonVisualState>(e),
                world.resource::<player::Player>().selected,
                world
                    .resource::<crate::tutorial::Tutorial>()
                    .active
                    .is_some(),
                world.resource::<crate::tutorial::Tutorial>().consumed_input,
                cursor
            );
            world
                .spawn(Screenshot::primary_window())
                .observe(save_to_disk(
                    smoke
                        .path
                        .parent()
                        .unwrap()
                        .join("before-inventory-click.png"),
                ));
            mouse(world, true);
            smoke.advance("");
        }
        4 if ready => {
            assert!(opened(world), "HUD mouse click did not open inventory");
            mouse(world, false);
            smoke.advance("mouse-opens-inventory-through-ui-picking");
        }
        5 if ready => {
            let close = world
                .query::<(Entity, &crate::inventory::Slot)>()
                .iter(world)
                .find_map(|(e, s)| (*s == crate::inventory::Slot::Close).then_some(e))
                .unwrap();
            point(world, close);
            smoke.advance("");
        }
        6 if ready => {
            mouse(world, true);
            smoke.advance("");
        }
        7 if ready => {
            mouse(world, false);
            smoke.advance("");
        }
        8 if ready => {
            assert!(!opened(world), "Close button did not close inventory");
            let e = hud(world, "b_inventory");
            assert!(
                world.get::<bevy::ui::Pressed>(e).is_none(),
                "hidden HUD retained stale press"
            );
            let other = hud(world, "b_crouch");
            point(world, other);
            smoke.advance("close-button-works-and-hidden-hud-press-clears");
        }
        9 if ready => {
            mouse(world, true);
            smoke.advance("");
        }
        10 if ready => {
            mouse(world, false);
            smoke.advance("");
        }
        11 if smoke.frames - smoke.changed > 80 => {
            assert!(!opened(world), "posture click reopened inventory");
            assert_eq!(
                world.resource::<player::Player>().posture,
                crate::stance::Posture::Crouch
            );
            let other = hud(world, "b_inventory");
            point(world, other);
            smoke.advance("another-hud-button-changes-posture-without-reopening-inventory");
        }
        12 if ready => {
            mouse(world, true);
            smoke.advance("");
        }
        13 if ready => {
            mouse(world, false);
            smoke.advance("");
        }
        14 if ready => {
            assert!(opened(world), "inventory did not reopen on its own button");
            let window = world
                .query_filtered::<Entity, With<PrimaryWindow>>()
                .single(world)
                .unwrap();
            world.write_message(bevy::input::keyboard::KeyboardInput {
                window,
                key_code: KeyCode::Escape,
                logical_key: bevy::input::keyboard::Key::Escape,
                state: ButtonState::Pressed,
                text: None,
                repeat: false,
            });
            smoke.advance("inventory-button-still-works-after-posture-command");
        }
        15 if ready => {
            assert!(!opened(world), "Escape did not close inventory");
            smoke.advance("escape-closes-inventory-without-opening-pause");
            assert_eq!(*world.resource::<State<Screen>>().get(), Screen::Training);
        }
        16 if ready => {
            let window = world
                .query_filtered::<Entity, With<PrimaryWindow>>()
                .single(world)
                .unwrap();
            world.write_message(bevy::input::keyboard::KeyboardInput {
                window,
                key_code: KeyCode::Escape,
                logical_key: bevy::input::keyboard::Key::Escape,
                state: ButtonState::Released,
                text: None,
                repeat: false,
            });
            world.resource_scope(|world, mut combat: Mut<crate::combat::Combat>| {
                combat.blast_damage(
                    0,
                    1_000_000.0,
                    &mut world.resource_mut::<crate::tutorial::Tutorial>(),
                );
            });
            let body = world.resource::<crate::combat::Combat>().guard.position;
            let near = [Vec3::X, -Vec3::X, Vec3::Z, -Vec3::Z]
                .into_iter()
                .find_map(|direction| {
                    let mut point = body + direction * 0.8;
                    point.y = crate::player::surface(
                        world.resource::<training::Preview>().grid.as_ref().unwrap(),
                        point,
                    )?;
                    world
                        .resource::<crate::pickup::Pickup>()
                        .clear_line(point + Vec3::Y, body + Vec3::Y * 0.3)
                        .then_some(point)
                })
                .expect("reachable body pickup fixture");
            let mut actor = world.resource_mut::<player::Player>();
            actor.position = near;
            actor.posture = crate::stance::Posture::Stand;
            actor.posture_transition = None;
            actor.action = None;
            actor.destination = None;
            actor.selected = true;
            let mut orbit = world.resource_mut::<training::Orbit>();
            orbit.target = body + Vec3::Y * 0.8;
            orbit.distance = 12.0;
            orbit.pitch = 0.85;
            smoke.advance("");
        }
        17 if ready && world.resource::<crate::combat::Combat>().death_finished() => {
            let body = world.resource::<crate::combat::Combat>().guard.position + Vec3::Y * 0.2;
            let (camera, transform) = world
                .query_filtered::<(&Camera, &GlobalTransform), With<training::MapCamera>>()
                .single(world)
                .unwrap();
            let cursor = camera.world_to_viewport(transform, body).unwrap();
            let window = world
                .query_filtered::<Entity, With<PrimaryWindow>>()
                .single(world)
                .unwrap();
            world
                .resource_mut::<crate::selection::Selection>()
                .test_cursor = Some(cursor);
            let event = CursorMoved {
                window,
                position: cursor,
                delta: None,
            };
            world.write_message(event.clone());
            world.write_message(bevy::window::WindowEvent::CursorMoved(event));
            smoke.advance("");
        }
        18 if ready => {
            mouse(world, true);
            smoke.advance("");
        }
        19 if ready => {
            mouse(world, false);
            smoke.advance("");
        }
        20 if ready
            && world.resource::<crate::body_carry::Carry>().phase
                == crate::body_carry::Phase::Carrying(0) =>
        {
            assert!(world.resource::<crate::combat::Combat>().carried);
            let target = world.resource::<player::Player>().camera_target();
            let mut orbit = world.resource_mut::<training::Orbit>();
            orbit.distance = 6.0;
            orbit.target = target;
            smoke.advance("mouse-orders-original-body-pickup-and-callback-attaches-guard");
        }
        21 if ready => {
            let path = smoke.path.parent().unwrap().join("body-carry.png");
            world
                .spawn(Screenshot::primary_window())
                .observe(save_to_disk(path));
            smoke.advance("carried-body-and-carrier-render-with-original-skeletons");
        }
        22 if ready => {
            let window = world
                .query_filtered::<Entity, With<PrimaryWindow>>()
                .single(world)
                .unwrap();
            world.write_message(bevy::input::keyboard::KeyboardInput {
                window,
                key_code: KeyCode::KeyD,
                logical_key: bevy::input::keyboard::Key::Character("d".into()),
                state: ButtonState::Pressed,
                text: None,
                repeat: false,
            });
            smoke.advance("");
        }
        23 if ready => {
            assert_eq!(
                world.resource::<crate::body_carry::Carry>().phase,
                crate::body_carry::Phase::Idle,
                "D did not put down the body"
            );
            assert!(!world.resource::<crate::combat::Combat>().carried);
            assert!(world.resource::<player::Player>().carry_movement.is_none());
            smoke.advance("D-places-body-and-restores-normal-controls");
            if let Some(parent) = smoke.path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&smoke.path,serde_json::to_vec_pretty(&serde_json::json!({
                "status":"passed","checks":smoke.checks,"frames":smoke.frames,
                "native_ui_picking":true,"mouse_messages_injected":true,"physical_pointer_tested":false,
                "dead_guard_state_seeded":true,"tutorial_windows_dismissed_by_harness":true,"body_screenshot":smoke.path.parent().unwrap().join("body-carry.png")
            })).unwrap()).unwrap();
            world.write_message(AppExit::Success);
        }
        _ => {}
    }
    world.insert_resource(smoke);
}
