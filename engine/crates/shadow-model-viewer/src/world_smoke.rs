//! Native integration checks against the player's original scenery.
use super::*;
use crate::{navigation, perception, player};
use bevy::{mesh::VertexAttributeValues, window::PrimaryWindow};
use std::path::Path;

struct Fixture {
    from: Vec3,
    to: Vec3,
    center: Vec3,
    name: String,
}
pub(super) struct Checks {
    phase: u8,
    changed: std::time::Instant,
    fixture: Option<Fixture>,
    previous: Vec3,
    trace: Vec<Vec3>,
    explored: Vec<bool>,
    visible: Vec<bool>,
    generation: u64,
    pub checks: Vec<String>,
}
impl Default for Checks {
    fn default() -> Self {
        Self {
            phase: 0,
            changed: std::time::Instant::now(),
            fixture: None,
            previous: Vec3::ZERO,
            trace: vec![],
            explored: vec![],
            visible: vec![],
            generation: 0,
            checks: vec![],
        }
    }
}
fn fixtures(world: &mut World) -> Vec<Fixture> {
    let mut boxes = Vec::new();
    for (name, transform, children) in world
        .query_filtered::<(&Name, &GlobalTransform, &Children), With<crate::pickup::MapObject>>()
        .iter(world)
    {
        let mut low = Vec3::splat(f32::INFINITY);
        let mut high = Vec3::splat(f32::NEG_INFINITY);
        for child in children.iter() {
            let Some(mesh) = world
                .get::<Mesh3d>(child)
                .and_then(|m| world.resource::<Assets<Mesh>>().get(&m.0))
            else {
                continue;
            };
            let Some(VertexAttributeValues::Float32x3(points)) =
                mesh.attribute(Mesh::ATTRIBUTE_POSITION)
            else {
                continue;
            };
            let matrix = world
                .get::<GlobalTransform>(child)
                .unwrap_or(transform)
                .to_matrix();
            for p in points {
                let p = matrix.transform_point3(Vec3::from_array(*p));
                low = low.min(p);
                high = high.max(p);
            }
        }
        if low.is_finite() && high.is_finite() && high.y - low.y > 0.6 && high.x - low.x < 12.0 {
            boxes.push((name.as_str().to_string(), low, high));
        }
    }
    let grid = world.resource::<training::Preview>().grid.as_ref().unwrap();
    let geometry = world.resource::<crate::pickup::Pickup>();
    let height = world.resource::<player::Player>().body_height();
    let nav = navigation::World {
        grid,
        geometry,
        fence: Some(world.resource::<crate::fence::Fence>()),
        gate: Some(world.resource::<crate::gate::Gate>()),
        height,
    };
    boxes
        .into_iter()
        .filter_map(|(name, low, high)| {
            let center = (low + high) * 0.5;
            let from = Vec3::new(low.x - 1.2, 0.0, center.z);
            let to = Vec3::new(high.x + 1.2, 0.0, center.z);
            let from = from.with_y(player::surface(grid, from)?);
            let to = to.with_y(player::surface(grid, to)?);
            (nav.free(from) && nav.free(to)).then_some(Fixture {
                from,
                to,
                center: center.with_y((from.y + to.y) * 0.5),
                name,
            })
        })
        .collect()
}
fn teleport(actor: &mut player::Player, position: Vec3) {
    actor.position = position;
    actor.destination = None;
    actor.route.clear();
    actor.action = None;
    actor.pickup_seconds = None;
    actor.posture_transition = None;
    actor.posture = crate::stance::Posture::Stand;
    actor.playback.paused = false;
    actor.follow_camera = false;
    actor.playback.select(&actor.clips, actor.idle_clip());
}
fn point(world: &mut World, position: Vec2) {
    world
        .resource_mut::<crate::selection::Selection>()
        .test_cursor = Some(position);
    let window = world
        .query_filtered::<Entity, With<PrimaryWindow>>()
        .single(world)
        .unwrap();
    let e = bevy::window::CursorMoved {
        window,
        position,
        delta: None,
    };
    world.write_message(e.clone());
    world.write_message(bevy::window::WindowEvent::CursorMoved(e));
}
fn snapshot(world: &mut World, directory: &Path, name: &str) {
    world
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(directory.join(name)));
}
impl Checks {
    fn next(&mut self, check: &str) {
        if !check.is_empty() {
            self.checks.push(check.into());
        }
        eprintln!("world-smoke phase {}: {check}", self.phase);
        self.phase += 1;
        self.changed = std::time::Instant::now();
    }
    pub fn step(&mut self, world: &mut World, directory: &Path) -> bool {
        let ready = self.changed.elapsed().as_secs_f32() > 0.9;
        assert!(
            self.changed.elapsed().as_secs() < 180,
            "world regression timed out at {}",
            self.phase
        );
        match self.phase {
            0 => {
                let options = fixtures(world);
                let grid = world.resource::<training::Preview>().grid.as_ref().unwrap();
                let geometry = world.resource::<crate::pickup::Pickup>();
                let nav = navigation::World {
                    grid,
                    geometry,
                    fence: Some(world.resource::<crate::fence::Fence>()),
                    gate: Some(world.resource::<crate::gate::Gate>()),
                    height: world.resource::<player::Player>().body_height(),
                };
                let extent = crate::radar::extent(grid) - Vec2::splat(grid.config.edge * 0.01);
                let fixture = options
                    .into_iter()
                    .filter(|f| {
                        ["crate", "table", "drum"]
                            .iter()
                            .any(|s| f.name.to_ascii_lowercase().contains(s))
                    })
                    .find(|f| {
                        !nav.clear(f.from, f.to)
                            && navigation::find_path(
                                f.from,
                                f.to,
                                extent,
                                |p| player::surface(grid, p),
                                |a, b| nav.clear(a, b),
                            )
                            .is_some()
                    })
                    .expect("original prop with reachable detour");
                self.previous = fixture.from;
                self.trace.push(fixture.from);
                teleport(&mut world.resource_mut::<player::Player>(), fixture.from);
                world.resource_mut::<player::Player>().selected = true;
                world.resource_mut::<crate::party::Party>().jack.selected = false;
                let mut orbit = world.resource_mut::<training::Orbit>();
                orbit.target = fixture.center;
                orbit.yaw = 0.0;
                orbit.pitch = 1.0;
                orbit.distance = 14.0;
                self.fixture = Some(fixture);
                self.next("");
            }
            1 if ready => {
                let goal = self.fixture.as_ref().unwrap().to;
                let (camera, transform) = world
                    .query_filtered::<(&Camera, &GlobalTransform), With<training::MapCamera>>()
                    .single(world)
                    .unwrap();
                let position = camera.world_to_viewport(transform, goal).unwrap();
                assert!(
                    position.y
                        < world.resource::<crate::hud::Hud>().fit.screen.y
                            - 195.0 * world.resource::<crate::hud::Hud>().fit.scale
                );
                point(world, position);
                self.next("");
            }
            2 if ready => {
                super::input_smoke::mouse(world, true);
                self.next("");
            }
            3 if ready => {
                super::input_smoke::mouse(world, false);
                self.next("");
            }
            4 => {
                let actor = world.resource::<player::Player>();
                if let Some(destination) = actor.destination {
                    assert!(destination.distance(self.fixture.as_ref().unwrap().to) < 0.1);
                    assert!(
                        actor.route.points().count() > 1,
                        "obstacle order has no detour"
                    );
                    self.previous = actor.position;
                    self.next("world-mouse-order-plans-detour-around-original-prop");
                }
            }
            5 => {
                let actor = world.resource::<player::Player>();
                let p = actor.position;
                assert!(!actor.route.failed);
                let geometry = world.resource::<crate::pickup::Pickup>();
                assert!(
                    geometry
                        .movement_hit(self.previous, p, actor.body_height(), |id| world
                            .resource::<crate::fence::Fence>()
                            .handles_collision(id)
                            || world.resource::<crate::gate::Gate>().id == id)
                        .is_none(),
                    "detour crossed original geometry"
                );
                self.previous = p;
                self.trace.push(p);
                if actor.destination.is_none() {
                    assert!(
                        p.distance(self.fixture.as_ref().unwrap().to) < 0.1,
                        "John stopped before the destination"
                    );
                    std::fs::write(directory.join("john-route.json"),serde_json::to_vec_pretty(&serde_json::json!({"original_prop":self.fixture.as_ref().unwrap().name,"trace":self.trace.iter().map(|p|p.to_array()).collect::<Vec<_>>()})).unwrap()).unwrap();
                    snapshot(world, directory, "navigation-john.png");
                    let fixture = self.fixture.as_ref().unwrap();
                    let goal = fixture.from;
                    let position = fixture.to;
                    let mut party = world.remove_resource::<crate::party::Party>().unwrap();
                    let grid = world.resource::<training::Preview>().grid.as_ref().unwrap();
                    party.recruited = true;
                    teleport(&mut party.jack, position);
                    party.jack.selected = true;
                    party.jack.order(grid, goal, true);
                    world.insert_resource(party);
                    self.previous = position;
                    self.next("john-follows-detour-to-exact-goal-without-crossing-prop");
                }
            }
            6 => {
                let party = world.resource::<crate::party::Party>();
                let actor = &party.jack;
                assert!(!actor.route.failed);
                assert!(
                    world
                        .resource::<crate::pickup::Pickup>()
                        .movement_hit(
                            self.previous,
                            actor.position,
                            actor.body_height(),
                            |id| world
                                .resource::<crate::fence::Fence>()
                                .handles_collision(id)
                                || world.resource::<crate::gate::Gate>().id == id
                        )
                        .is_none()
                );
                self.previous = actor.position;
                if actor.destination.is_none() {
                    assert!(
                        actor.position.distance(self.fixture.as_ref().unwrap().from) < 0.1,
                        "Jack stopped at obstacle"
                    );
                    snapshot(world, directory, "navigation-jack.png");
                    self.next("jack-follows-independent-running-detour-around-original-prop");
                }
            }
            7 if ready => {
                let options = fixtures(world);
                let grid = world.resource::<training::Preview>().grid.as_ref().unwrap();
                let geometry = world.resource::<crate::pickup::Pickup>();
                let height = world.resource::<player::Player>().body_height();
                let enemy = &world.resource::<crate::combat::Combat>().guard_at(1).guard;
                let target_height = enemy.body_height() * [0.9, 0.55, 0.2][enemy.posture.index()];
                let jack_height = world.resource::<crate::party::Party>().jack.body_height() * 0.9;
                let nav = navigation::World {
                    grid,
                    geometry,
                    fence: Some(world.resource::<crate::fence::Fence>()),
                    gate: Some(world.resource::<crate::gate::Gate>()),
                    height,
                };
                let (fixture, position) = options.into_iter().find_map(|f| {
                    let p = f.to + Vec3::X * 2.0;
                    let p = p.with_y(player::surface(grid,p)?);
                    let target = f.to + Vec3::Y * target_height;
                    (f.from.distance(f.to) < 30.0 && p.distance(f.to) < 4.5 && nav.free(p)
                        && !perception::line_of_sight(grid,geometry,f.from+Vec3::Y*height*0.9,target)
                        && perception::line_of_sight(grid,geometry,p+Vec3::Y*height*0.9,target)
                        && perception::line_of_sight(grid,geometry,p+Vec3::Y*jack_height,target))
                        .then_some((f,p))
                }).expect("original structure blocks sight while both actual character eye heights see from its far side");
                eprintln!(
                    "vision fixture {}: observer={} target={} hidden_side={}",
                    fixture.name, position, fixture.to, fixture.from
                );
                teleport(&mut world.resource_mut::<player::Player>(), position);
                world.resource_mut::<crate::party::Party>().recruited = false;
                world
                    .resource_mut::<crate::combat::Combat>()
                    .guard_mut(1)
                    .guard
                    .position = fixture.to;
                let mut orbit = world.resource_mut::<training::Orbit>();
                orbit.target = fixture.center;
                orbit.distance = 18.0;
                self.fixture = Some(fixture);
                self.next("");
            }
            8 if ready => {
                let actor = world.resource::<player::Player>();
                let guard = world.resource::<crate::combat::Combat>().guard_at(1);
                let eye = actor.position
                    + Vec3::Y * actor.body_height() * [0.9, 0.55, 0.2][actor.posture.index()];
                let target = guard.guard.position
                    + Vec3::Y
                        * guard.guard.body_height()
                        * [0.9, 0.55, 0.2][guard.guard.posture.index()];
                let vis = world.resource::<crate::perception::VisibilityState>();
                let grid = world.resource::<training::Preview>().grid.as_ref().unwrap();
                let geometry = world.resource::<crate::pickup::Pickup>();
                let diagnostic = serde_json::json!({"observer":actor.position.to_array(),"observer_eye":eye.to_array(),"target":guard.guard.position.to_array(),"target_eye":target.to_array(),"distance":actor.position.distance(guard.guard.position),"line_of_sight":perception::line_of_sight(grid,geometry,eye,target),"john_health":world.resource::<crate::medical::Medical>().health,"sample_age_seconds":world.resource::<Time<Real>>().elapsed_secs_f64()-vis.elapsed,"contacts":vis.contacts,"parameters":vis.source});
                std::fs::write(
                    directory.join("visibility-fixture.json"),
                    serde_json::to_vec_pretty(&diagnostic).unwrap(),
                )
                .unwrap();
                assert!(
                    world
                        .resource::<crate::perception::VisibilityState>()
                        .contact(1)
                );
                assert!(
                    world
                        .resource::<crate::combat::Combat>()
                        .guard_at(1)
                        .detected
                );
                let visible = world
                    .query::<(&crate::combat::GuardRoot, &Visibility)>()
                    .iter(world)
                    .find(|(r, _)| r.0 == 1)
                    .unwrap()
                    .1;
                assert_ne!(*visible, Visibility::Hidden);
                self.generation = world
                    .resource::<crate::perception::VisibilityState>()
                    .generation;
                self.explored = world
                    .resource::<crate::perception::VisibilityState>()
                    .explored
                    .clone();
                self.visible = world
                    .resource::<crate::perception::VisibilityState>()
                    .visible
                    .clone();
                world.resource_mut::<training::Orbit>().target = Vec3::new(2.0, 0.0, -2.0);
                self.next("team-detects-visible-enemy-and-renders-its-world-model");
            }
            9 if ready => {
                let vis = world.resource::<crate::perception::VisibilityState>();
                assert_eq!(vis.generation, self.generation);
                assert_eq!(vis.explored, self.explored);
                teleport(
                    &mut world.resource_mut::<player::Player>(),
                    self.fixture.as_ref().unwrap().from,
                );
                self.next("moving-camera-alone-cannot-reveal-fog-or-enemies");
            }
            10 if ready => {
                assert!(
                    !world
                        .resource::<crate::perception::VisibilityState>()
                        .contact(1)
                );
                assert!(
                    !world
                        .resource::<crate::combat::Combat>()
                        .guard_at(1)
                        .detected
                );
                assert_eq!(
                    *world
                        .query::<(&crate::combat::GuardRoot, &Visibility)>()
                        .iter(world)
                        .find(|(r, _)| r.0 == 1)
                        .unwrap()
                        .1,
                    Visibility::Hidden
                );
                let p = self.fixture.as_ref().unwrap().to + Vec3::X * 2.0;
                let y = player::surface(
                    world.resource::<training::Preview>().grid.as_ref().unwrap(),
                    p,
                )
                .unwrap();
                let mut party = world.resource_mut::<crate::party::Party>();
                party.recruited = true;
                teleport(&mut party.jack, p.with_y(y));
                self.next("opaque-original-structure-hides-enemy-contact-and-world-model");
            }
            11 if ready => {
                assert!(
                    world
                        .resource::<crate::perception::VisibilityState>()
                        .contact(1)
                );
                self.explored = world
                    .resource::<crate::perception::VisibilityState>()
                    .explored
                    .clone();
                self.visible = world
                    .resource::<crate::perception::VisibilityState>()
                    .visible
                    .clone();
                world.resource_mut::<crate::party::Party>().recruited = false;
                let p = Vec3::new(2.0, 0.0, -2.0);
                let y = player::surface(
                    world.resource::<training::Preview>().grid.as_ref().unwrap(),
                    p,
                )
                .unwrap();
                teleport(&mut world.resource_mut::<player::Player>(), p.with_y(y));
                self.next("recruited-jack-shares-his-line-of-sight-with-team");
            }
            12 if ready => {
                let vis = world.resource::<crate::perception::VisibilityState>();
                assert!(
                    self.explored
                        .iter()
                        .zip(&vis.explored)
                        .all(|(was, now)| !was || *now)
                );
                assert!(
                    self.visible
                        .iter()
                        .zip(&vis.visible)
                        .any(|(was, now)| *was && !*now)
                );
                snapshot(world, directory, "radar-fog-memory.png");
                self.next("explored-terrain-persists-while-lost-vision-returns-to-fog");
            }
            13 if ready => return true,
            _ => {}
        }
        false
    }
}
