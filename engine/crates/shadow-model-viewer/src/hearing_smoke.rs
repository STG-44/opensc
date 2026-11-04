//! Native hearing integration against original scenery and weapon profiles.
use super::*;
use crate::{combat::Combat, hearing::ActorId, hud::Hud, perception::VisibilityState};

#[derive(Resource)]
pub(super) struct Smoke {
    path: PathBuf,
    stage: u8,
    changed: std::time::Instant,
    checks: Vec<String>,
    from: Vec3,
    to: Vec3,
    heading: f32,
    explored: Vec<bool>,
    source: serde_json::Value,
}
impl Smoke {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            stage: 0,
            changed: std::time::Instant::now(),
            checks: vec![],
            from: Vec3::ZERO,
            to: Vec3::ZERO,
            heading: 0.0,
            explored: vec![],
            source: serde_json::Value::Null,
        }
    }
    fn next(&mut self, check: &str) {
        if !check.is_empty() {
            self.checks.push(check.into());
        }
        eprintln!("hearing-smoke stage {}: {check}", self.stage);
        self.stage += 1;
        self.changed = std::time::Instant::now();
    }
}
fn heard(world: &World) -> Option<crate::hearing::Contact> {
    world
        .resource::<VisibilityState>()
        .hearing
        .as_ref()
        .unwrap()
        .contact(ActorId::Guard(0))
        .copied()
}
fn radius(world: &World) -> Option<f32> {
    world
        .resource::<VisibilityState>()
        .hearing
        .as_ref()
        .unwrap()
        .radius(ActorId::John)
        .map(|(_, r)| r)
}
fn ring(world: &World) -> bool {
    let h = world.resource::<Hud>();
    let image = world
        .resource::<Assets<Image>>()
        .get(h.radar.handle.as_ref().unwrap())
        .unwrap();
    image
        .data
        .as_ref()
        .unwrap()
        .as_chunks::<4>()
        .0
        .contains(&[44, 168, 72, 255])
}
fn stop(world: &mut World) {
    let mut actor = world.resource_mut::<player::Player>();
    actor.destination = None;
    actor.route.clear();
}
fn snapshot(world: &mut World, path: PathBuf) {
    world
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(path));
}
pub(super) fn run(world: &mut World) {
    let mut s = world.remove_resource::<Smoke>().unwrap();
    assert!(
        s.changed.elapsed().as_secs() < 180,
        "hearing timed out at {}",
        s.stage
    );
    if let Some(mut t) = world.get_resource_mut::<crate::tutorial::Tutorial>() {
        if t.active.is_some() {
            t.close();
        }
        t.consumed_input = false;
    }
    let ready = s.changed.elapsed().as_secs_f32() > 1.0;
    match s.stage {
        0 if ready && *world.resource::<State<Screen>>().get() == Screen::Main => {
            world.write_message(ActionRequest(Action::Training));
            s.next("");
        }
        1 if ready
            && world.contains_resource::<Hud>()
            && *world.resource::<State<Screen>>().get() == Screen::Training =>
        {
            let options = super::world_smoke::fixtures(world);
            let grid = world.resource::<training::Preview>().grid.as_ref().unwrap();
            let geometry = world.resource::<crate::pickup::Pickup>();
            let eye = world.resource::<player::Player>().body_height() * 0.9;
            let guard = &world.resource::<Combat>().guard;
            let target_eye = guard.body_height() * 0.9;
            let f = options
                .into_iter()
                .find(|f| {
                    f.from.distance(f.to) < 30.0
                        && !crate::perception::line_of_sight(
                            grid,
                            geometry,
                            f.from + Vec3::Y * eye,
                            f.to + Vec3::Y * target_eye,
                        )
                })
                .expect("original opaque structure for hearing test");
            eprintln!("hearing fixture {}: {} -> {}", f.name, f.from, f.to);
            s.from = f.from;
            s.to = f.to;
            super::world_smoke::teleport(&mut world.resource_mut::<player::Player>(), f.from);
            world.resource_mut::<player::Player>().selected = true;
            world.resource_mut::<crate::party::Party>().recruited = false;
            let mut combat = world.resource_mut::<Combat>();
            super::world_smoke::teleport(&mut combat.guard, f.to);
            let d = f.from - f.to;
            s.heading = (-d.x).atan2(-d.z) + std::f32::consts::PI;
            combat.guard.heading = s.heading;
            let mut orbit = world.resource_mut::<training::Orbit>();
            orbit.target = f.center;
            orbit.distance = 18.0;
            world.resource_mut::<VisibilityState>().reset();
            s.next("");
        }
        2 if ready => {
            assert!(!world.resource::<VisibilityState>().contact(0));
            assert!(heard(world).is_none() && radius(world).is_none());
            s.explored = world.resource::<VisibilityState>().explored.clone();
            s.source = world.resource::<VisibilityState>().source.clone();
            let inv = world.resource::<crate::inventory::Inventory>();
            let bda = inv.gun_for(crate::inventory::Weapon::Bda).unwrap();
            let uzi = inv.gun_for(crate::inventory::Weapon::Uzi).unwrap();
            assert!((bda.noise_radius - 36.0).abs() < 0.001 && bda.noise_level == 3);
            assert!((uzi.noise_radius - 48.0).abs() < 0.001 && uzi.noise_level == 4);
            let from = world.resource::<player::Player>().camera_target();
            // Inject the normal per-round combat event, including a non-audio burst round.
            world
                .resource_mut::<Combat>()
                .pending_shots
                .push(crate::combat::Shot {
                    from,
                    to: s.to,
                    weapon: crate::inventory::Weapon::Bda,
                    audio: false,
                });
            s.next("original-profiles-provide-36m-pistol-and-48m-uzi-noise");
        }
        3 => {
            let contact =
                heard(world).expect("shot event must reach hearing before combat drains it");
            assert!(
                contact
                    .position
                    .distance(world.resource::<player::Player>().camera_target())
                    < 0.001
            );
            assert_eq!(contact.level, 3);
            assert_eq!(radius(world), Some(36.0));
            assert!(!world.resource::<VisibilityState>().contact(0));
            assert!(!world.resource::<Combat>().detected);
            assert_eq!(world.resource::<VisibilityState>().explored, s.explored);
            let rendered = world
                .query::<(&crate::combat::GuardRoot, &Visibility)>()
                .iter(world)
                .find(|(r, _)| r.0 == 0)
                .unwrap()
                .1;
            assert_eq!(*rendered, Visibility::Hidden);
            assert!(
                ring(world),
                "selected mercenary noise ring missing from radar"
            );
            assert!((world.resource::<Combat>().guard.heading - s.heading).abs() > 0.001);
            snapshot(world, s.path.parent().unwrap().join("hearing-shot.png"));
            world.resource_mut::<crate::inventory::Inventory>().open = true;
            s.next("shot-through-opaque-scenery-turns-guard-and-shows-noise-ring-without-revealing-enemy-or-fog");
        }
        4 if s.changed.elapsed().as_secs_f32() > 3.0 => {
            assert!(heard(world).is_some());
            world.resource_mut::<crate::inventory::Inventory>().open = false;
            s.next("inventory-pause-preserves-hearing-beyond-normal-memory-duration");
        }
        5 if s.changed.elapsed().as_secs_f32() > 3.0 && radius(world).is_none() => {
            assert!(heard(world).is_none() && radius(world).is_none());
            assert!(!ring(world));
            let grid = world.resource::<training::Preview>().grid.as_ref().unwrap();
            let geometry = world.resource::<crate::pickup::Pickup>();
            let nav = crate::navigation::World {
                grid,
                geometry,
                fence: Some(world.resource::<crate::fence::Fence>()),
                gate: Some(world.resource::<crate::gate::Gate>()),
                height: world.resource::<player::Player>().body_height(),
            };
            let goal = [-Vec3::X, Vec3::Z, -Vec3::Z, Vec3::X]
                .into_iter()
                .find_map(|d| {
                    let p = s.from + d * 4.0;
                    let p = p.with_y(player::surface(grid, p)?);
                    nav.clear(s.from, p).then_some(p)
                })
                .expect("clear original terrain for running test");
            world.resource_scope(|world, mut john: Mut<player::Player>| {
                let grid = world.resource::<training::Preview>().grid.as_ref().unwrap();
                john.senses.infiltration = 0.0;
                assert!(john.order(grid, goal, true));
            });
            world.resource_mut::<Combat>().guard.position = s.from;
            s.next("resuming-expires-memory-and-clears-stationary-noise-ring");
        }
        6 if radius(world).is_some() => {
            assert!((radius(world).unwrap() - 20.625).abs() < 0.001);
            assert!(heard(world).is_some_and(|c| c.level == 1));
            assert!(world.resource::<player::Player>().position.distance(s.from) > 0.001);
            stop(world);
            s.next("animation-driven-resolved-running-emits-source-skill-dependent-footsteps");
        }
        7 if s.changed.elapsed().as_secs_f32() > 3.0 && radius(world).is_none() => {
            assert!(radius(world).is_none());
            assert!(heard(world).is_none());
            assert!(!ring(world));
            let from = world.resource::<player::Player>().camera_target();
            world
                .resource_mut::<Combat>()
                .pending_shots
                .push(crate::combat::Shot {
                    from,
                    to: s.to,
                    weapon: crate::inventory::Weapon::Uzi,
                    audio: false,
                });
            s.next("stopping-real-movement-clears-footstep-memory-and-ring");
        }
        8 => {
            assert_eq!(radius(world), Some(48.0));
            assert_eq!(heard(world).unwrap().level, 4);
            assert!(world.resource::<Combat>().pending_shots.is_empty());
            world.resource_mut::<VisibilityState>().reset();
            assert!(radius(world).is_none() && heard(world).is_none());
            s.next("non-audio-uzi-rounds-still-create-gameplay-noise-and-mission-reset-clears-it");
        }
        9 if ready => {
            assert!(!ring(world));
            std::fs::write(&s.path,serde_json::to_vec_pretty(&serde_json::json!({"status":"passed",
                "checks":s.checks,"source":s.source,"original_geometry":true,"shot_events_seeded":true,
                "movement_uses_actual_animation_and_navigation":true,"physical_pointer_tested":false})).unwrap()).unwrap();
            world.write_message(AppExit::Success);
        }
        _ => {}
    }
    world.insert_resource(s);
}
