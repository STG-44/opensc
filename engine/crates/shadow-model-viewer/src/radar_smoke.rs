//! Native radar regression using the normal UI picking and mouse message path.
use super::input_smoke::{hud, mouse, point};
use super::*;
use crate::{hud::Hud, radar};
use bevy::window::PrimaryWindow;

#[derive(Resource)]
pub(super) struct Smoke {
    path: PathBuf,
    stage: u8,
    frames: u32,
    changed: u32,
    changed_at: std::time::Instant,
    checks: Vec<String>,
    actor: Vec3,
    camera: Vec3,
    expected: Vec3,
    pixels: Vec<u8>,
    world_checks: super::world_smoke::Checks,
}
impl Smoke {
    pub(super) fn new(path: PathBuf) -> Self {
        Self {
            path,
            stage: 0,
            frames: 0,
            changed: 0,
            changed_at: std::time::Instant::now(),
            checks: vec![],
            actor: Vec3::ZERO,
            camera: Vec3::ZERO,
            expected: Vec3::ZERO,
            pixels: vec![],
            world_checks: super::world_smoke::Checks::default(),
        }
    }
    fn advance(&mut self, check: &str) {
        if !check.is_empty() {
            self.checks.push(check.into());
        }
        eprintln!("radar-smoke stage {}: {check}", self.stage);
        self.stage += 1;
        self.changed = self.frames;
        self.changed_at = std::time::Instant::now();
    }
    fn snapshot(&self, world: &mut World, name: &str) {
        world
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(self.path.parent().unwrap().join(name)));
    }
}
fn cursor(world: &mut World, position: Vec2) {
    world
        .resource_mut::<crate::selection::Selection>()
        .test_cursor = Some(position);
    let window = world
        .query_filtered::<Entity, With<PrimaryWindow>>()
        .single(world)
        .unwrap();
    let event = bevy::window::CursorMoved {
        window,
        position,
        delta: None,
    };
    world.write_message(event.clone());
    world.write_message(bevy::window::WindowEvent::CursorMoved(event));
}
fn radar_point(world: &mut World, uv: Vec2) -> Vec3 {
    let h = world.resource::<Hud>();
    let rect = h.fit.rect(h.layout.advice("r_radar").rect);
    let grid = world.resource::<training::Preview>().grid.as_ref().unwrap();
    let view = h.radar.view(world.resource::<training::Orbit>(), grid);
    let expected = view.world(uv, grid);
    cursor(world, rect.min + uv * rect.size());
    expected
}
fn pixels(world: &World) -> Vec<u8> {
    let h = world.resource::<Hud>();
    world
        .resource::<Assets<Image>>()
        .get(h.radar.handle.as_ref().unwrap())
        .unwrap()
        .data
        .clone()
        .unwrap()
}
fn at_actor(world: &World, position: Vec3, color: [u8; 4]) -> bool {
    let h = world.resource::<Hud>();
    let grid = world.resource::<training::Preview>().grid.as_ref().unwrap();
    let view = h.radar.view(world.resource::<training::Orbit>(), grid);
    let image = world
        .resource::<Assets<Image>>()
        .get(h.radar.handle.as_ref().unwrap())
        .unwrap();
    let p = view.project(radar::uv(position, radar::extent(grid)))
        * (image.size().as_vec2() - Vec2::ONE);
    let p = p.round().as_ivec2();
    (-2..=2).any(|y| {
        (-2..=2).any(|x| {
            let p = p + IVec2::new(x, y);
            p.min_element() >= 0
                && p.x < image.width() as i32
                && p.y < image.height() as i32
                && image.data.as_ref().unwrap()
                    [((p.y as u32 * image.width() + p.x as u32) * 4) as usize..][..4]
                    == color
        })
    })
}
fn unchanged_actor(world: &World, smoke: &Smoke) {
    let actor = world.resource::<player::Player>();
    assert!(
        actor.position.distance(smoke.actor) < 0.001,
        "radar click moved actor"
    );
    assert!(
        actor.destination.is_none(),
        "radar leaked a world movement order"
    );
}
pub(super) fn run(world: &mut World) {
    let mut s = world.remove_resource::<Smoke>().unwrap();
    s.frames += 1;
    assert!(s.frames < 30000, "radar test timed out at {}", s.stage);
    if world.contains_resource::<crate::tutorial::Tutorial>() {
        let mut t = world.resource_mut::<crate::tutorial::Tutorial>();
        if t.active.is_some() {
            t.close();
        }
        t.consumed_input = false;
    }
    let ready = s.frames - s.changed >= 10 && s.changed_at.elapsed().as_secs_f32() >= 0.85;
    match s.stage {
        0 if ready && *world.resource::<State<Screen>>().get() == Screen::Main => {
            world.write_message(ActionRequest(Action::Training));
            s.advance("");
        }
        1 if ready
            && world.contains_resource::<Hud>()
            && *world.resource::<State<Screen>>().get() == Screen::Training =>
        {
            s.actor = world.resource::<player::Player>().position;
            let h = world.resource::<Hud>();
            assert!(h.radar.locked && h.radar.zoom == 1.0);
            assert!(at_actor(world, s.actor, radar::FRIEND));
            let combat = world.resource::<crate::combat::Combat>();
            assert!(at_actor(world, combat.guard.position, radar::ENEMY));
            assert!(
                pixels(world)
                    .as_chunks::<4>()
                    .0
                    .contains(&[255, 255, 255, 255])
            );
            s.snapshot(world, "radar-locked.png");
            s.advance("full-map-shows-current-friendly-enemy-and-camera-footprint");
        }
        2 if ready => {
            // Seed an independent actor move; camera stays elsewhere.
            world.resource_mut::<player::Player>().position.x += 8.0;
            s.advance("");
        }
        3 if ready => {
            s.actor = world.resource::<player::Player>().position;
            assert!(at_actor(world, s.actor, radar::FRIEND));
            s.pixels = pixels(world);
            world.resource_mut::<training::Orbit>().target += Vec3::new(15.0, 0.0, 10.0);
            world.resource_mut::<training::Orbit>().yaw += 0.6;
            s.advance("friendly-marker-follows-live-world-position-independently-of-camera");
        }
        4 if ready => {
            assert_ne!(s.pixels, pixels(world));
            assert!(at_actor(world, s.actor, radar::FRIEND));
            let e = hud(world, "b_radar_zoom_in");
            point(world, e);
            s.advance(
                "camera-pan-and-rotation-update-white-footprint-without-moving-friendly-marker",
            );
        }
        5 if ready => {
            mouse(world, true);
            s.advance("");
        }
        6 if ready => {
            mouse(world, false);
            s.advance("");
        }
        7 if ready => {
            assert_eq!(world.resource::<Hud>().radar.zoom, 2.0);
            s.expected = radar_point(world, Vec2::new(0.32, 0.60));
            s.advance("native-zoom-in-button-updates-map");
        }
        8 if ready => {
            mouse(world, true);
            s.advance("");
        }
        9 if ready => {
            let camera = training::camera_focus(world.resource::<training::Orbit>());
            assert!(
                camera.distance(s.expected) < 0.01,
                "camera missed map click {camera} != {}",
                s.expected
            );
            assert!(world.resource::<Hud>().radar.drag.is_some());
            unchanged_actor(world, &s);
            s.camera = camera;
            s.advance("zoomed-map-click-positions-camera-without-ordering-actor");
        }
        10 if ready => {
            assert!(
                training::camera_focus(world.resource::<training::Orbit>()).distance(s.camera)
                    < 0.001,
                "stationary hold caused crop feedback"
            );
            s.expected = radar_point(world, Vec2::new(0.78, 0.28));
            s.advance("stationary-held-pointer-does-not-drift-as-camera-recenters");
        }
        11 if ready => {
            assert!(
                training::camera_focus(world.resource::<training::Orbit>()).distance(s.expected)
                    < 0.01
            );
            unchanged_actor(world, &s);
            cursor(world, Vec2::new(400.0, 250.0));
            mouse(world, false);
            s.advance("held-pointer-drags-camera-using-original-gesture-transform");
        }
        12 if ready => {
            assert!(world.resource::<Hud>().radar.drag.is_none());
            unchanged_actor(world, &s);
            let e = hud(world, "b_radar_mode");
            point(world, e);
            s.advance("release-outside-radar-does-not-issue-world-click");
        }
        13 if ready => {
            mouse(world, true);
            s.advance("");
        }
        14 if ready => {
            mouse(world, false);
            s.advance("");
        }
        15 if ready => {
            assert!(!world.resource::<Hud>().radar.locked);
            let h = world.resource::<Hud>();
            let grid = world.resource::<training::Preview>().grid.as_ref().unwrap();
            let view = h.radar.view(world.resource::<training::Orbit>(), grid);
            assert!(
                view.project(radar::uv(
                    training::camera_focus(world.resource::<training::Orbit>()),
                    radar::extent(grid)
                ))
                .distance(Vec2::splat(0.5))
                    < 1e-5
            );
            s.snapshot(world, "radar-unlocked.png");
            s.expected = radar_point(world, Vec2::new(0.65, 0.65));
            s.advance("lock-button-enables-camera-centered-rotating-map");
        }
        16 if ready => {
            mouse(world, true);
            s.advance("");
        }
        17 if ready => {
            assert!(
                training::camera_focus(world.resource::<training::Orbit>()).distance(s.expected)
                    < 0.01
            );
            unchanged_actor(world, &s);
            mouse(world, false);
            s.advance("rotated-map-click-uses-inverse-map-rotation");
        }
        18 if ready => {
            world.resource_mut::<crate::inventory::Inventory>().open = true;
            s.advance("");
        }
        19 if ready => {
            let e = hud(world, "r_radar");
            assert_eq!(world.get::<Node>(e).unwrap().display, Display::None);
            world.resource_mut::<crate::inventory::Inventory>().open = false;
            let mut h = world.resource_mut::<Hud>();
            h.radar.locked = true;
            h.radar.zoom = 1.0;
            // Isolate one living contact, then remove it through the combat damage path.
            let actor = world.resource::<player::Player>();
            let eye = actor.position + Vec3::Y * actor.body_height() * 0.9;
            let grid = world.resource::<training::Preview>().grid.as_ref().unwrap();
            let geometry = world.resource::<crate::pickup::Pickup>();
            let near = [Vec3::X, -Vec3::X, Vec3::Z, -Vec3::Z]
                .into_iter()
                .find_map(|dir| {
                    let p = actor.position + dir * 2.0;
                    let p = p.with_y(player::surface(grid, p)?);
                    crate::perception::line_of_sight(
                        grid,
                        geometry,
                        eye,
                        p + Vec3::Y * actor.body_height() * 0.9,
                    )
                    .then_some(p)
                })
                .expect("visible nearby hostile fixture");
            world.resource_mut::<crate::combat::Combat>().guard.position = near;
            s.advance("inventory-hides-radar-and-restores-normal-controls-on-close");
        }
        20 if ready => {
            let p = world.resource::<crate::combat::Combat>().guard.position;
            assert!(at_actor(world, p, radar::ENEMY));
            world.resource_scope(|world, mut c: Mut<crate::combat::Combat>| {
                c.blast_damage(
                    0,
                    1_000_000.0,
                    &mut world.resource_mut::<crate::tutorial::Tutorial>(),
                );
            });
            s.advance("");
        }
        21 if ready => {
            let p = world.resource::<crate::combat::Combat>().guard.position;
            assert!(
                !at_actor(world, p, radar::ENEMY),
                "dead guard still displayed as hostile"
            );
            let e = hud(world, "b_radar_zoom_in");
            point(world, e);
            s.advance("combat-death-removes-hostile-marker");
        }
        22 if ready => {
            mouse(world, true);
            s.advance("");
        }
        23 if ready => {
            mouse(world, false);
            s.advance("");
        }
        24 if ready => {
            assert_eq!(world.resource::<Hud>().radar.zoom, 2.0);
            let e = hud(world, "b_radar_zoom_out");
            point(world, e);
            s.advance("");
        }
        25 if ready => {
            mouse(world, true);
            s.advance("");
        }
        26 if ready => {
            mouse(world, false);
            s.advance("");
        }
        27 if ready => {
            assert_eq!(world.resource::<Hud>().radar.zoom, 1.0);
            assert!(at_actor(world, s.actor, radar::FRIEND));
            s.snapshot(world, "radar-final.png");
            s.advance("zoom-out-restores-entire-map-and-current-friendly-position");
        }
        28 if ready => {
            s.expected = radar_point(world, Vec2::splat(0.5));
            s.advance("");
        }
        29 if ready => {
            mouse(world, true);
            s.advance("");
        }
        30 if ready => {
            assert!(world.resource::<Hud>().radar.drag.is_some());
            world.resource_mut::<crate::inventory::Inventory>().open = true;
            s.advance("");
        }
        31 if ready => {
            assert!(world.resource::<Hud>().radar.drag.is_none());
            mouse(world, false);
            world.resource_mut::<crate::inventory::Inventory>().open = false;
            let entity = world
                .query_filtered::<Entity, With<PrimaryWindow>>()
                .single(world)
                .unwrap();
            world
                .get_mut::<Window>(entity)
                .unwrap()
                .resolution
                .set(1440.0, 810.0);
            s.advance("opening-inventory-cancels-held-radar-gesture");
        }
        32 if ready => {
            let entity = hud(world, "r_radar");
            let h = world.resource::<Hud>();
            let image = world
                .resource::<Assets<Image>>()
                .get(h.radar.handle.as_ref().unwrap())
                .unwrap();
            let node = world.get::<ComputedNode>(entity).unwrap();
            assert!((image.size().as_vec2() - node.size()).abs().max_element() <= 1.1);
            assert!(h.fit.screen.distance(Vec2::new(1440.0, 810.0)) < 1.0);
            assert_eq!(world.get::<Node>(entity).unwrap().display, Display::Flex);
            unchanged_actor(world, &s);
            s.snapshot(world, "radar-resized.png");
            s.advance("window-resize-rebuilds-radar-at-physical-ui-size");
        }
        33 if ready && s.world_checks.step(world, s.path.parent().unwrap()) => {
            s.checks.extend(s.world_checks.checks.clone());
            std::fs::write(&s.path, serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","checks":s.checks,"frames":s.frames,"native_ui_picking":true,"mouse_messages_injected":true,"physical_pointer_tested":false,"actor_and_enemy_positions_seeded":true,"enemy_death_uses_combat_damage":true})).unwrap()).unwrap();
            world.write_message(AppExit::Success);
        }
        _ => {}
    }
    world.insert_resource(s);
}
