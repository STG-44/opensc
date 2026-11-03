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
pub(super) fn mouse(world: &mut World, down: bool) {
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
pub(super) fn point(world: &mut World, entity: Entity) {
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
pub(super) fn hud(world: &mut World, id: &str) -> Entity {
    world
        .query::<(Entity, &crate::hud::CommandButton)>()
        .iter(world)
        .find_map(|(e, b)| (b.0 == id).then_some(e))
        .expect("HUD button")
}
fn opened(world: &World) -> bool {
    world.resource::<crate::inventory::Inventory>().open
}

fn slot(world: &mut World, kind: crate::inventory::Slot) -> Entity {
    world
        .query::<(Entity, &crate::inventory::Slot)>()
        .iter(world)
        .find_map(|(e, s)| (*s == kind).then_some(e))
        .expect("original inventory slot")
}
fn snapshot(world: &mut World, smoke: &Smoke, name: &str) {
    world
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(smoke.path.parent().unwrap().join(name)));
}
fn verify_grid_frame(world: &mut World) {
    let fit = world.resource::<crate::hud::Hud>().fit;
    let mut right = 0.0f32;
    for (slot, node, tr) in world
        .query::<(&crate::inventory::Slot, &ComputedNode, &UiGlobalTransform)>()
        .iter(world)
    {
        if *slot == crate::inventory::Slot::Backpack {
            let (_, _, center) = tr.to_scale_angle_translation();
            right = right.max((center.x + node.size().x * 0.5) * node.inverse_scale_factor());
        }
    }
    assert!(
        (right - (fit.screen.x - 184.0 * fit.scale)).abs() < 1.1,
        "inventory right gap: {right}"
    );
}
fn verify_world_props(world: &mut World, smoke: &mut Smoke) {
    use bevy::mesh::VertexAttributeValues;
    let mut objects = Vec::new();
    let mut query = world.query::<(
        &crate::pickup::MapObject,
        &Name,
        &GlobalTransform,
        &Children,
    )>();
    for (id, name, tr, children) in query.iter(world) {
        let lower_name = name.as_str().to_ascii_lowercase();
        if !["crate", "table", "drum"]
            .iter()
            .any(|tag| lower_name.contains(tag))
        {
            continue;
        }
        let mut low = Vec3::splat(f32::INFINITY);
        let mut high = Vec3::splat(f32::NEG_INFINITY);
        for child in children.iter() {
            let Some(mesh) = world.get::<Mesh3d>(child) else {
                continue;
            };
            let Some(mesh) = world.resource::<Assets<Mesh>>().get(&mesh.0) else {
                continue;
            };
            let Some(VertexAttributeValues::Float32x3(points)) =
                mesh.attribute(Mesh::ATTRIBUTE_POSITION)
            else {
                continue;
            };
            let matrix = world.get::<GlobalTransform>(child).unwrap().to_matrix();
            for point in points {
                let point = matrix.transform_point3(Vec3::from_array(*point));
                low = low.min(point);
                high = high.max(point);
            }
        }
        let center = (low + high) * 0.5;
        let ground = player::surface(
            world.resource::<training::Preview>().grid.as_ref().unwrap(),
            center,
        );
        if low.is_finite()
            && (high - low).max_element() < 5.0
            && ground.is_some_and(|y| high.y > y + 0.2 && low.y < y + 1.5)
        {
            objects.push((id.0, lower_name, low, high, tr.to_matrix()));
        }
    }
    for tag in ["crate", "table", "drum"] {
        let (id, name, low, high, _) = objects
            .iter()
            .find(|(_, name, _, _, _)| name.contains(tag))
            .expect("representative original prop");
        let center = (*low + *high) * 0.5;
        let grid = world.resource::<training::Preview>().grid.as_ref().unwrap();
        let mut from = Vec3::new(low.x - 0.8, 0.0, center.z);
        let mut to = Vec3::new(high.x + 0.8, 0.0, center.z);
        from.y = player::surface(grid, from).unwrap();
        to.y = player::surface(grid, to).unwrap();
        world.resource_scope(|world, mut actor: Mut<player::Player>| {
            let saved = (actor.position, actor.destination, actor.posture);
            actor.position = to;
            actor.destination = Some(to);
            actor.posture = crate::stance::Posture::Stand;
            world.resource::<crate::pickup::Pickup>().guard_movement(
                &mut actor,
                from,
                world.resource::<training::Preview>().grid.as_ref().unwrap(),
                |other| other != *id,
            );
            assert!(
                actor.position.x < high.x && actor.destination.is_none(),
                "walked through original {name}: bounds={low:?}/{high:?}, sweep={from:?}/{to:?}, result={:?}",actor.position
            );
            actor.position = saved.0;
            actor.destination = saved.1;
            actor.posture = saved.2;
        });
        smoke
            .checks
            .push(format!("original-{tag}-mesh-blocks-character-sweep"));
    }
    let (target_id, _, low, high, matrix) = objects
        .iter()
        .find(|(_, name, _, _, _)| name.contains("granitecrate") && !name.contains("dam"))
        .expect("GRANITE crate");
    let others = world
        .query::<(Entity, &crate::pickup::MapObject)>()
        .iter(world)
        .filter_map(|(entity, object)| (object.0 != *target_id).then_some(entity))
        .collect::<Vec<_>>();
    for entity in others {
        world.entity_mut(entity).insert(Visibility::Hidden);
    }
    let normal = matrix.transform_vector3(-Vec3::Z).normalize();
    let mut orbit = world.resource_mut::<training::Orbit>();
    orbit.target = (*low + *high) * 0.5;
    orbit.yaw = normal.x.atan2(normal.z);
    orbit.pitch = 0.23;
    orbit.distance = 4.5;
}
fn primary_ammo(world: &mut World) -> String {
    let screen = world
        .query::<&Window>()
        .single(world)
        .unwrap()
        .resolution
        .size();
    let mut primary = None;
    for (caption, text, node, transform) in world
        .query::<(
            &crate::roster::Caption,
            &Text,
            &ComputedNode,
            &UiGlobalTransform,
        )>()
        .iter(world)
    {
        if caption.0 != 1 || text.0.is_empty() {
            continue;
        }
        let (_, _, center) = transform.to_scale_angle_translation();
        let max = (center + node.size() * 0.5) * node.inverse_scale_factor();
        assert!(
            max.x <= screen.x + 0.5 && max.y <= screen.y + 0.5,
            "portrait weapon/ammunition row clipped by window: {max:?}/{screen:?}"
        );
        if !caption.1 {
            primary = Some(text.0.clone());
        }
    }
    primary.expect("John primary ammo caption")
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
            if matches!(
                world.resource::<crate::body_carry::Carry>().phase,
                crate::body_carry::Phase::Dropping(_)
            ) {
                if !smoke
                    .checks
                    .iter()
                    .any(|c| c == "D-starts-animated-body-release")
                {
                    smoke.checks.push("D-starts-animated-body-release".into());
                    world
                        .spawn(Screenshot::primary_window())
                        .observe(save_to_disk(
                            smoke.path.parent().unwrap().join("body-drop.png"),
                        ));
                }
                world.insert_resource(smoke);
                return;
            }
            assert_eq!(
                world.resource::<crate::body_carry::Carry>().phase,
                crate::body_carry::Phase::Idle,
                "D did not put down the body"
            );
            assert!(!world.resource::<crate::combat::Combat>().carried);
            assert!(world.resource::<player::Player>().carry_movement.is_none());
            smoke.advance("D-places-body-and-restores-normal-controls");
        }
        24 if ready => {
            let mut inv = world.resource_mut::<crate::inventory::Inventory>();
            inv.owns_knife = true;
            inv.collected = true;
            inv.owns_uzi = true;
            inv.owns_cutters = true;
            inv.rounds = 7;
            inv.uzi_rounds = 23;
            let model = inv.gun.model.clone();
            world.resource_mut::<player::Player>().weapon = Some(model);
            let mut g = world.resource_mut::<crate::grenade::Grenade>();
            g.collected = true;
            g.secondary = true;
            g.count = 3;
            let mut m = world.resource_mut::<crate::medical::Medical>();
            m.ground.collected = true;
            m.uses = 4;
            m.health = 230.;
            let mut a = world.resource_mut::<crate::armor::Armor>();
            a.ground.collected = true;
            a.worn = true;
            world.resource_mut::<crate::party::Party>().recruited = true;
            smoke.advance("");
        }
        25 if ready => {
            assert_eq!(primary_ammo(world), "7");
            snapshot(world, &smoke, "hud-bda-7.png");
            world
                .resource_mut::<crate::inventory::Inventory>()
                .take_round_for(crate::inventory::Weapon::Bda);
            smoke.advance("portrait-shows-selected-BDA-magazine");
        }
        26 if ready => {
            assert_eq!(primary_ammo(world), "6");
            let model = world
                .resource::<crate::inventory::Inventory>()
                .uzi
                .as_ref()
                .unwrap()
                .model
                .clone();
            world.resource_mut::<player::Player>().weapon = Some(model);
            smoke.advance("portrait-ammo-decreases-after-shot");
        }
        27 if ready => {
            assert_eq!(primary_ammo(world), "23");
            snapshot(world, &smoke, "hud-uzi-23.png");
            world.resource_mut::<crate::inventory::Inventory>().open = true;
            smoke.advance("portrait-switches-to-UZI-magazine");
        }
        28 if ready => {
            let e = slot(
                world,
                crate::inventory::Slot::Stored(crate::inventory::Weapon::Bda),
            );
            point(world, e);
            smoke.advance("");
        }
        29 if ready => {
            mouse(world, true);
            smoke.advance("");
        }
        30 if ready => {
            assert_eq!(
                world.resource::<crate::inventory::Inventory>().drag,
                Some(crate::inventory::Weapon::Bda)
            );
            let e = slot(world, crate::inventory::Slot::Primary);
            point(world, e);
            smoke.advance("");
        }
        31 if ready => {
            mouse(world, false);
            smoke.advance("");
        }
        32 if ready => {
            let inv = world.resource::<crate::inventory::Inventory>();
            assert_eq!(
                inv.weapon(world.resource::<player::Player>()),
                Some(crate::inventory::Weapon::Bda)
            );
            assert_eq!((inv.rounds, inv.uzi_rounds), (6, 23));
            snapshot(world, &smoke, "inventory-bda-equipped.png");
            let e = slot(world, crate::inventory::Slot::Next);
            point(world, e);
            smoke.advance("original-pack-drag-equips-weapon-and-retains-both-magazines");
        }
        33 if ready => {
            mouse(world, true);
            smoke.advance("");
        }
        34 if ready => {
            mouse(world, false);
            smoke.advance("");
        }
        35 if ready => {
            assert_eq!(
                world.resource::<crate::inventory::Inventory>().view_member,
                2
            );
            assert_eq!(
                world
                    .query::<&crate::inventory_ui::ItemImage>()
                    .iter(world)
                    .count(),
                0
            );
            snapshot(world, &smoke, "inventory-jack-empty.png");
            let e = slot(world, crate::inventory::Slot::Next);
            point(world, e);
            smoke.advance("next-mercenary-shows-Jack-without-Johns-equipment");
        }
        36 if ready => {
            mouse(world, true);
            smoke.advance("");
        }
        37 if ready => {
            mouse(world, false);
            smoke.advance("");
        }
        38 if ready => {
            assert_eq!(
                world.resource::<crate::inventory::Inventory>().view_member,
                1
            );
            assert!(
                world
                    .query::<&crate::inventory_ui::ItemImage>()
                    .iter(world)
                    .any(|i| i.0 == crate::inventory_ui::Item::Bda)
            );
            smoke.advance("return-to-John-restores-equipment-view");
            let e = slot(world, crate::inventory::Slot::Skills);
            point(world, e);
        }
        39 if ready => {
            mouse(world, true);
            smoke.advance("");
        }
        40 if ready => {
            mouse(world, false);
            smoke.advance("");
        }
        41 if ready => {
            assert!(world.resource::<crate::inventory::Inventory>().skills);
            if *world.resource::<crate::localization::Language>()
                == crate::localization::Language::English
            {
                assert!(
                    world
                        .query::<&Text>()
                        .iter(world)
                        .all(|t| !crate::localization::has_cyrillic(&t.0))
                );
            }
            snapshot(world, &smoke, "inventory-skills.png");
            smoke.advance("original-query-button-opens-localized-mercenary-skills");
        }
        42 if ready => {
            verify_grid_frame(world);
            smoke
                .checks
                .push("inventory-grid-reaches-right-frame".into());
            {
                let mut inv = world.resource_mut::<crate::inventory::Inventory>();
                inv.open = false;
                inv.skills = false;
            }
            verify_world_props(world, &mut smoke);
            smoke.advance("");
        }
        43 if ready => {
            snapshot(world, &smoke, "granite-crate-rear-label.png");
            world.resource_mut::<training::Orbit>().yaw += std::f32::consts::PI;
            smoke.advance("");
        }
        44 if ready => {
            snapshot(world, &smoke, "granite-crate-front-label.png");
            smoke.advance("");
        }
        45 if ready => {
            if let Some(parent) = smoke.path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&smoke.path,serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","checks":smoke.checks,"frames":smoke.frames,"native_ui_picking":true,"mouse_messages_injected":true,"physical_pointer_tested":false,"dead_guard_state_seeded":true,"inventory_states_seeded":true,"tutorial_windows_dismissed_by_harness":true,"body_screenshot":smoke.path.parent().unwrap().join("body-carry.png")})).unwrap()).unwrap();
            world.write_message(AppExit::Success);
        }
        _ => {}
    }
    world.insert_resource(smoke);
}
