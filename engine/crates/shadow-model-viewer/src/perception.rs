//! Shared team visibility, source perception parameters, and persistent radar fog.
use bevy::prelude::*;
use bevy::tasks::{AsyncComputeTaskPool, Task, TaskPool, futures::check_ready};
use shadow_formats::{AssetCatalog, ConfigLine, TerrainGrid, config_lines};
use std::{error::Error, sync::Arc, time::Instant};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Skills {
    pub perception: f32,
    pub infiltration: f32,
}
impl Skills {
    pub fn from_profile(lines: &[ConfigLine]) -> Result<Self, Box<dyn Error>> {
        let skill = |name: &str| -> Result<f32, Box<dyn Error>> {
            let values: Vec<_> = lines
                .iter()
                .filter(|l| {
                    l.words.first().is_some_and(|s| s == "SKILL")
                        && l.words.get(1).is_some_and(|s| s.eq_ignore_ascii_case(name))
                })
                .collect();
            let [line] = values.as_slice() else {
                return Err(format!("expected one {name} skill").into());
            };
            let v: f32 = line.words.get(2).ok_or("missing skill value")?.parse()?;
            if !v.is_finite() || !(0.0..=100.0).contains(&v) {
                return Err(format!("invalid {name} skill").into());
            }
            Ok(v)
        };
        Ok(Self {
            perception: skill("Perception")?,
            infiltration: skill("Infiltration")?,
        })
    }
}
impl Default for Skills {
    fn default() -> Self {
        Self {
            perception: 68.0,
            infiltration: 45.0,
        }
    }
}

#[derive(Clone)]
struct Settings {
    base: f32,
    modifier: f32,
    near: f32,
    factor: f32,
    drop: f32,
    height_multiplier: f32,
    height_min: f32,
    height_max: f32,
    evade: [[f32; 3]; 3],
    period: f64,
}
impl Settings {
    fn read(lines: &[ConfigLine]) -> Result<Self, Box<dyn Error>> {
        let value = |key: &str| -> Result<f32, Box<dyn Error>> {
            let values: Vec<_> = lines
                .iter()
                .filter(|l| l.words.get(1).is_some_and(|s| s == key))
                .collect();
            let [line] = values.as_slice() else {
                return Err(format!("expected one perception parameter {key}").into());
            };
            let v: f32 = line
                .words
                .get(2)
                .ok_or("missing perception value")?
                .parse()?;
            if !v.is_finite() {
                return Err("nonfinite perception parameter".into());
            }
            Ok(v)
        };
        let mut evade = [[0.0; 3]; 3];
        for (stance, name) in ["stand", "crouch", "prone"].into_iter().enumerate() {
            for (motion, suffix) in ["still", "walk", "run"].into_iter().enumerate() {
                evade[stance][motion] = value(&format!("stance_evade_{name}_{suffix}"))?;
            }
        }
        let settings = Self {
            base: value("oli_base_range")? * 0.01,
            modifier: value("oli_perc_mod_range")? * 0.01,
            near: value("auto_seen_range")? * 0.01,
            factor: value("perc_range_factor_max")?,
            drop: value("perc_range_factor_drop")?,
            height_multiplier: value("perc_height_adv_mult")? / 0.01,
            height_min: value("perc_height_adv_min")?,
            height_max: value("perc_height_adv_max")?,
            evade,
            period: f64::from(value("oli_gen_period_min")?) / 1000.0,
        };
        if settings.base <= 0.0
            || settings.modifier < 0.0
            || settings.near < 0.0
            || settings.factor <= 0.0
            || settings.drop < 0.0
            || settings.height_min > settings.height_max
            || !(0.05..=10.0).contains(&settings.period)
        {
            return Err("invalid perception parameter ranges".into());
        }
        Ok(settings)
    }
    fn range(&self, perception: f32) -> f32 {
        self.base + perception * self.modifier
    }
    fn detects(&self, observer: &Person, target: &Person) -> bool {
        let distance = observer.position.distance(target.position);
        let range = self.range(observer.skills.perception);
        if distance > range {
            return false;
        }
        if distance <= self.near {
            return true;
        }
        let advantage = ((observer.eye().y - target.eye().y) * self.height_multiplier)
            .clamp(self.height_min, self.height_max);
        let score =
            observer.skills.perception * (self.factor - (distance / range) * self.drop) + advantage;
        let evade = if target.alive {
            target.skills.infiltration + self.evade[target.posture][target.motion]
        } else {
            0.0
        };
        score >= evade
    }
}
#[derive(Clone, Debug, PartialEq)]
struct Person {
    position: Vec3,
    height: f32,
    skills: Skills,
    posture: usize,
    motion: usize,
    alive: bool,
}
impl Person {
    fn actor(p: &crate::player::Player, alive: bool) -> Self {
        Self {
            position: p.position,
            height: p.body_height(),
            skills: p.senses,
            posture: p.posture.index(),
            motion: if p.destination.is_none() {
                0
            } else if p.running {
                2
            } else {
                1
            },
            alive,
        }
    }
    fn eye(&self) -> Vec3 {
        self.position + Vec3::Y * self.height * [0.9, 0.55, 0.2][self.posture]
    }
}
#[derive(Clone, PartialEq)]
struct Key {
    observers: Vec<Person>,
    targets: Vec<Person>,
    neutral: Person,
    geometry: u64,
}
#[derive(Clone, PartialEq)]
struct FogKey {
    observers: Vec<Person>,
    geometry: u64,
}

struct FogResult {
    size: UVec2,
    visible: Vec<bool>,
    worker_ms: f64,
}

fn scan_fog(
    grid: &TerrainGrid,
    geometry: &crate::pickup::SightSnapshot,
    settings: &Settings,
    observers: &[Person],
    size: UVec2,
    extent: Vec2,
) -> FogResult {
    let start = Instant::now();
    let mut visible = vec![false; (size.x * size.y) as usize];
    for observer in observers {
        let eye = observer.eye();
        let radius = settings.range(observer.skills.perception);
        let center = crate::radar::uv(observer.position, extent) * size.as_vec2();
        let cells = Vec2::splat(radius) / extent * size.as_vec2();
        let low = (center - cells).floor().max(Vec2::ZERO).as_uvec2();
        let high = (center + cells)
            .ceil()
            .min((size - UVec2::ONE).as_vec2())
            .max(Vec2::ZERO)
            .as_uvec2();
        for y in low.y..=high.y {
            for x in low.x..=high.x {
                let uv = (Vec2::new(x as f32, y as f32) + Vec2::splat(0.5)) / size.as_vec2();
                let p = Vec3::new(uv.x * extent.x, 0., -uv.y * extent.y);
                let Some(height) = crate::player::surface(grid, p) else {
                    continue;
                };
                let target = p.with_y(height + 0.2);
                let i = (y * size.x + x) as usize;
                if !visible[i]
                    && target.distance(eye) <= radius
                    && terrain_clear(grid, eye, target)
                    && geometry.line_of_sight(eye, target)
                {
                    visible[i] = true;
                }
            }
        }
    }
    FogResult {
        size,
        visible,
        worker_ms: start.elapsed().as_secs_f64() * 1000.,
    }
}

#[derive(Resource)]
pub(crate) struct VisibilityState {
    settings: Settings,
    pub hearing: Option<crate::hearing::HearingState>,
    pub size: UVec2,
    pub explored: Vec<bool>,
    pub visible: Vec<bool>,
    pub contacts: Vec<bool>,
    pub neutral: bool,
    pub generation: u64,
    pub source: serde_json::Value,
    key: Option<Key>,
    pub elapsed: f64,
    extent: Vec2,
    fog: Option<Task<FogResult>>,
    fog_key: Option<FogKey>,
    terrain: Option<Arc<TerrainGrid>>,
    geometry_cache: Option<(u64, Arc<crate::pickup::SightSnapshot>)>,
    last_worker_ms: f64,
}
impl VisibilityState {
    pub fn prepare(catalog: &AssetCatalog) -> Result<Self, Box<dyn Error>> {
        let path = "config/perception_var_defs.txt";
        let settings = Settings::read(&config_lines(&catalog.read(path)?)?)?;
        let hearing = crate::hearing::HearingState::prepare(catalog)?;
        let source = serde_json::json!({"hearing":hearing.source.clone(),"parameters":path,"origin":catalog.source_label(path)?,"base_range_metres":settings.base,"perception_range_modifier_metres":settings.modifier,"auto_seen_range_metres":settings.near,"refresh_seconds":settings.period,"terrain_line_of_sight":true,"opaque_geometry_line_of_sight":true,"lighting_modifiers_implemented":false,"hearing_implemented":true,"original_exe_formula_verified":false});
        Ok(Self {
            settings,
            hearing: Some(hearing),
            size: UVec2::ZERO,
            explored: vec![],
            visible: vec![],
            contacts: vec![],
            neutral: false,
            generation: 0,
            source,
            key: None,
            elapsed: f64::NEG_INFINITY,
            extent: Vec2::ONE,
            fog: None,
            fog_key: None,
            terrain: None,
            geometry_cache: None,
            last_worker_ms: 0.,
        })
    }
    pub fn contact(&self, i: usize) -> bool {
        self.contacts.get(i).copied().unwrap_or(false)
    }
    pub fn shade(&self, uv: Vec2) -> f32 {
        if self.size == UVec2::ZERO {
            return 0.06;
        }
        let p = (uv.clamp(Vec2::ZERO, Vec2::ONE) * self.size.as_vec2())
            .as_uvec2()
            .min(self.size - UVec2::ONE);
        let i = (p.y * self.size.x + p.x) as usize;
        if self.visible[i] {
            1.0
        } else if self.explored[i] {
            0.45
        } else {
            0.06
        }
    }
    pub fn reset(&mut self) {
        if let Some(hearing) = &mut self.hearing {
            hearing.reset();
        }
        self.explored.fill(false);
        self.visible.fill(false);
        self.contacts.fill(false);
        self.neutral = false;
        self.key = None;
        self.fog = None;
        self.fog_key = None;
        self.terrain = None;
        self.geometry_cache = None;
        self.elapsed = f64::NEG_INFINITY;
        self.generation = self.generation.wrapping_add(1);
    }
    fn poll_fog(&mut self) {
        let Some(result) = self.fog.as_mut().and_then(check_ready) else {
            return;
        };
        self.fog = None;
        if result.size != self.size {
            return;
        }
        for (known, seen) in self.explored.iter_mut().zip(&result.visible) {
            *known |= *seen;
        }
        self.visible = result.visible;
        self.last_worker_ms = result.worker_ms;
        self.generation = self.generation.wrapping_add(1);
    }
    fn observe(
        &mut self,
        now: f64,
        key: Key,
        grid: &TerrainGrid,
        geometry: &crate::pickup::Pickup,
    ) {
        // Poll without waiting, even if no new contact refresh is due yet.
        self.poll_fog();
        if now - self.elapsed < self.settings.period {
            return;
        }
        let fog_key = FogKey {
            observers: key.observers.clone(),
            geometry: key.geometry,
        };
        let contacts_changed = self.key.as_ref() != Some(&key);
        let fog_changed = self.fog_key.as_ref() != Some(&fog_key);
        if !contacts_changed && !fog_changed {
            return;
        }
        self.elapsed = now;
        self.extent = crate::radar::extent(grid);
        let size = (self.extent / 2.0).ceil().as_uvec2().max(UVec2::ONE);
        if self.size != size {
            self.fog = None;
            self.fog_key = None;
            self.terrain = None;
            self.geometry_cache = None;
            self.size = size;
            self.explored = vec![false; (size.x * size.y) as usize];
            self.visible = self.explored.clone();
        }
        let clear = |a, b| line_of_sight(grid, geometry, a, b);
        // At most one scan is in flight. It owns immutable input; moving a car
        // or gate cannot mutate a worker's geometry or starve it of results.
        if self.fog.is_none() && fog_changed {
            let terrain = self
                .terrain
                .get_or_insert_with(|| Arc::new(grid.clone()))
                .clone();
            if self
                .geometry_cache
                .as_ref()
                .is_none_or(|(rev, _)| *rev != key.geometry)
            {
                self.geometry_cache = Some((key.geometry, Arc::new(geometry.sight_snapshot())));
            }
            let world = self.geometry_cache.as_ref().unwrap().1.clone();
            let observers = key.observers.clone();
            let settings = self.settings.clone();
            let extent = self.extent;
            self.fog_key = Some(fog_key);
            self.fog = Some(
                AsyncComputeTaskPool::get_or_init(TaskPool::new).spawn(async move {
                    scan_fog(&terrain, &world, &settings, &observers, size, extent)
                }),
            );
        }
        if !contacts_changed {
            return;
        }
        self.contacts = key
            .targets
            .iter()
            .map(|target| {
                key.observers.iter().any(|observer| {
                    self.settings.detects(observer, target) && clear(observer.eye(), target.eye())
                })
            })
            .collect();
        self.neutral = key.observers.iter().any(|observer| {
            self.settings.detects(observer, &key.neutral)
                && clear(observer.eye(), key.neutral.eye())
        });
        self.key = Some(key);
        self.generation = self.generation.wrapping_add(1);
    }
}
pub(crate) fn line_of_sight(
    grid: &TerrainGrid,
    geometry: &crate::pickup::Pickup,
    from: Vec3,
    to: Vec3,
) -> bool {
    terrain_clear(grid, from, to) && geometry.line_of_sight(from, to)
}
fn terrain_clear(grid: &TerrainGrid, from: Vec3, to: Vec3) -> bool {
    if !from.is_finite() || !to.is_finite() {
        return false;
    }
    let distance = from.distance(to);
    let steps = (distance / 0.5).ceil().max(1.0) as usize;
    for i in 1..steps {
        let p = from.lerp(to, i as f32 / steps as f32);
        if crate::player::surface(grid, p).is_none_or(|y| y > p.y + 0.02) {
            return false;
        }
    }
    true
}

/// CPU-only diagnostics against installed data; excludes renderer and GPU time.
/// Keep the ordinary visibility scheduling intact so this measures the burst
/// experienced by a moving player, rather than a synthetic empty scene.
pub(crate) fn profile_cpu(
    catalog: &AssetCatalog,
    grid: &TerrainGrid,
    geometry: &crate::pickup::Pickup,
    actor: &crate::player::Player,
) -> Result<serde_json::Value, Box<dyn Error>> {
    let mut state = VisibilityState::prepare(catalog)?;
    let mut times = Vec::new();
    let mut workers = Vec::new();
    let mut max_poll_ms = 0f64;
    for frame in 0..8 {
        let mut person = Person::actor(actor, true);
        person.position.x += frame as f32 * 0.1;
        let key = Key {
            observers: vec![person.clone()],
            targets: Vec::new(),
            neutral: person,
            geometry: geometry.revision(),
        };
        let start = Instant::now();
        state.observe(frame as f64, key, grid, geometry);
        times.push(start.elapsed().as_secs_f64() * 1000.);
        while state.fog.is_some() {
            let poll = Instant::now();
            state.poll_fog();
            max_poll_ms = max_poll_ms.max(poll.elapsed().as_secs_f64() * 1000.);
            if state.fog.is_some() {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
        workers.push(state.last_worker_ms);
    }
    let clips=actor.clips.iter().map(|clip| {
        let a=&clip.frames[0];let b=clip.frames.last().unwrap();
        let seam=a.bones.iter().zip(&b.bones).map(|(a,b)|a.rotation.angle_between(b.rotation).abs().to_degrees()).fold(0f32,f32::max);
        serde_json::json!({"name":clip.name,"fps":clip.fps,"duration_seconds":clip.duration(),"loop_seam_degrees":seam})
    }).collect::<Vec<_>>();
    Ok(
        serde_json::json!({"cpu_only":true,"renderer_measured":false,"visibility_update_ms":times,
        "fog_cells":state.size.to_array(),"refresh_seconds":state.settings.period,
        "visibility_worker_ms":workers,"maximum_poll_ms":max_poll_ms,"background_visibility":true,
        "actual_standing_walk":actor.postures.as_ref().map(|p|&actor.clips[p.walk[0]].name),
        "actual_standing_run":actor.postures.as_ref().map(|p|&actor.clips[p.run[0]].name),"animation_clips":clips}),
    )
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn update(
    visibility: Option<ResMut<VisibilityState>>,
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    actor: Option<Res<crate::player::Player>>,
    party: Option<Res<crate::party::Party>>,
    combat: Option<ResMut<crate::combat::Combat>>,
    medical: Option<Res<crate::medical::Medical>>,
    geometry: Option<Res<crate::pickup::Pickup>>,
    preview: Res<crate::training::Preview>,
    inventory: Option<Res<crate::inventory::Inventory>>,
    vehicle: Option<Res<crate::vehicle::Vehicle>>,
    tutorial: Option<Res<crate::tutorial::Tutorial>>,
) {
    let (Some(mut vis), Some(actor), Some(party), Some(mut combat), Some(geometry), Some(grid)) =
        (visibility, actor, party, combat, geometry, &preview.grid)
    else {
        return;
    };
    if keys.just_pressed(KeyCode::Home)
        && inventory
            .as_ref()
            .is_none_or(|i| !i.open && !i.consumed_input)
        && tutorial
            .as_ref()
            .is_none_or(|t| t.active.is_none() && !t.consumed_input)
    {
        vis.reset();
    }
    let mut observers = Vec::new();
    if medical.as_ref().is_none_or(|m| m.health > 0.0) {
        observers.push(Person::actor(&actor, true));
    }
    if party.recruited {
        observers.push(Person::actor(&party.jack, true));
    }
    let targets = (0..combat.guard_count())
        .map(|i| {
            let g = combat.guard_at(i);
            Person::actor(&g.guard, g.alive())
        })
        .collect();
    let key = Key {
        observers,
        targets,
        neutral: Person::actor(&party.jack, true),
        geometry: geometry.revision(),
    };
    vis.observe(time.elapsed_secs_f64(), key, grid, &geometry);
    let paused = actor.playback.paused
        || tutorial.as_ref().is_some_and(|t| t.pausing())
        || inventory.as_ref().is_some_and(|i| i.open);
    if let Some(hearing) = &mut vis.hearing {
        use crate::hearing::{Actor, ActorId, Shot};
        let sample = |id, p: &crate::player::Player, alive, footsteps| Actor {
            id,
            position: p.position,
            infiltration: p.senses.infiltration,
            posture: p.posture.index(),
            running: p.running,
            alive,
            footsteps,
        };
        let mut people = vec![sample(
            ActorId::John,
            &actor,
            medical.as_ref().is_none_or(|m| m.health > 0.0),
            vehicle.as_ref().is_none_or(|v| !v.occupied),
        )];
        if party.recruited {
            people.push(sample(ActorId::Jack, &party.jack, true, true));
        }
        for i in 0..combat.guard_count() {
            let guard = combat.guard_at(i);
            people.push(sample(
                ActorId::Guard(i),
                &guard.guard,
                guard.alive(),
                !guard.carried,
            ));
        }
        let shots: Vec<_> = combat
            .pending_shots
            .iter()
            .filter_map(|shot| {
                let gun = inventory.as_ref()?.gun_for(shot.weapon)?;
                Some(Shot {
                    source: ActorId::John,
                    position: shot.from,
                    radius: gun.noise_radius,
                    level: gun.noise_level,
                })
            })
            .collect();
        hearing.tick(time.delta_secs(), &people, &shots, paused);
        if !paused {
            for i in 0..combat.guard_count() {
                let Some(contact) = hearing.contact(ActorId::Guard(i)) else {
                    continue;
                };
                let target = contact.position;
                let guard = combat.guard_mut(i);
                if guard.alive()
                    && !guard.carried
                    && !guard.ordered
                    && guard.guard.destination.is_none()
                    && guard.guard.action.is_none()
                {
                    guard.guard.heading = crate::hearing::turn_towards(
                        guard.guard.heading,
                        guard.guard.position,
                        target,
                        time.delta_secs(),
                    );
                }
            }
        }
    }

    for i in 0..combat.guard_count() {
        combat.guard_mut(i).detected = vis.contact(i);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn settings() -> Settings {
        Settings {
            base: 30.0,
            modifier: 0.3,
            near: 5.0,
            factor: 1.52,
            drop: 1.0,
            height_multiplier: 1.0,
            height_min: -15.0,
            height_max: 15.0,
            evade: [[0.0, -5.0, -20.0], [10.0, -5.0, -5.0], [20.0, -5.0, -5.0]],
            period: 0.75,
        }
    }
    #[test]
    fn source_ranges_skills_stance_and_near_detection_affect_contact_visibility() {
        let (_, p, _, _) = crate::pickup::tests::fixture();
        let mut observer = Person::actor(&p, true);
        observer.position = Vec3::ZERO;
        let mut target = observer.clone();
        target.position.x = 55.0;
        assert!(!settings().detects(&observer, &target));
        target.position.x = 40.0;
        target.skills.infiltration = 60.0;
        assert!(!settings().detects(&observer, &target));
        target.position.x = 20.0;
        target.skills.infiltration = 60.0;
        assert!(settings().detects(&observer, &target));
        target.posture = 2;
        assert!(!settings().detects(&observer, &target));
        target.position.x = 4.0;
        assert!(settings().detects(&observer, &target));
    }
    #[test]
    fn automatic_near_detection_never_sees_through_an_opaque_wall() {
        let (mut geometry, _, _, grid) = crate::pickup::tests::fixture();
        crate::pickup::tests::add_wall(&mut geometry);
        assert!(!line_of_sight(
            &grid,
            &geometry,
            Vec3::new(0.5, 2.0, -0.5),
            Vec3::new(1.5, 2.0, -0.5)
        ));
    }
    #[test]
    fn explored_cells_remain_known_after_the_team_moves_away_and_reset_clears_them() {
        let (geometry, p, _, _) = crate::pickup::tests::fixture();
        let grid = TerrainGrid {
            config: shadow_formats::TerrainConfig {
                name: "visibility-test".into(),
                width: 65,
                height: 65,
                edge: 100.0,
                height_step: 1.0,
                water_index: 0,
            },
            heights: vec![0.0; 65 * 65],
        };
        let mut config = settings();
        config.base = 10.0;
        config.modifier = 0.0;
        let mut v = VisibilityState {
            settings: config,
            hearing: None,
            size: UVec2::ZERO,
            explored: vec![],
            visible: vec![],
            contacts: vec![],
            neutral: false,
            generation: 0,
            source: serde_json::Value::Null,
            key: None,
            elapsed: f64::NEG_INFINITY,
            extent: Vec2::ONE,
            fog: None,
            fog_key: None,
            terrain: None,
            geometry_cache: None,
            last_worker_ms: 0.,
        };
        let mut observer = Person::actor(&p, true);
        observer.position = Vec3::new(4.0, 0.0, -4.0);
        let key = Key {
            observers: vec![observer.clone()],
            targets: vec![],
            neutral: observer.clone(),
            geometry: 0,
        };
        v.observe(0.0, key.clone(), &grid, &geometry);
        finish_scan(&mut v);
        let old = Vec2::new(4.0, 4.0) / crate::radar::extent(&grid);
        assert_eq!(v.shade(old), 1.0);
        let generation = v.generation;
        v.observe(1.0, key.clone(), &grid, &geometry);
        assert!(v.fog.is_none());
        assert_eq!(v.generation, generation);
        // Simulate an older scan still in flight while the observer moves.
        v.fog = Some(AsyncComputeTaskPool::get().spawn(std::future::pending()));
        let mut key2 = key;
        key2.observers[0].position = Vec3::new(60.0, 0.0, -60.0);
        v.observe(2.0, key2.clone(), &grid, &geometry);
        assert!(v.fog.is_some());
        v.fog = None; // older scan drains; its contact key must not eat this update
        v.observe(3.0, key2, &grid, &geometry);
        finish_scan(&mut v);
        assert_eq!(v.shade(old), 0.45);
        assert_eq!(
            v.shade(Vec2::new(60., 60.) / crate::radar::extent(&grid)),
            1.
        );
        v.fog = Some(AsyncComputeTaskPool::get().spawn(async {
            FogResult {
                size: UVec2::splat(33),
                visible: vec![true; 33 * 33],
                worker_ms: 0.,
            }
        }));
        v.reset();
        v.poll_fog();
        assert_eq!(v.shade(old), 0.06);
        assert!(!v.contact(0));
    }
    fn finish_scan(v: &mut VisibilityState) {
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        while v.fog.is_some() {
            v.poll_fog();
            assert!(Instant::now() < deadline, "fog worker did not finish");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
}
