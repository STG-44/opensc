//! Sound stimuli and short-lived hearing memory, separate from visual contacts.
use bevy::prelude::*;
use shadow_formats::{AssetCatalog, ConfigLine, config_lines};
use std::{collections::BTreeMap, error::Error};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) enum ActorId {
    John,
    Jack,
    Guard(usize),
}
impl ActorId {
    fn hostile_to(self, other: Self) -> bool {
        matches!(self, Self::Guard(_)) != matches!(other, Self::Guard(_))
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Actor {
    pub id: ActorId,
    pub position: Vec3,
    pub infiltration: f32,
    pub posture: usize,
    pub running: bool,
    pub alive: bool,
    pub footsteps: bool,
}

#[derive(Clone, Copy)]
pub(crate) struct Shot {
    pub source: ActorId,
    pub position: Vec3,
    pub radius: f32,
    pub level: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Kind {
    Movement,
    Shot,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Contact {
    pub source: ActorId,
    pub position: Vec3,
    pub radius: f32,
    pub level: u32,
    kind: Kind,
    emitted: f64,
    expires: f64,
}

struct Settings {
    base: f32,
    skill_base: f32,
    skill_modifier: f32,
    movement: [[f32; 2]; 3],
    duration: f64,
}
impl Settings {
    fn read(lines: &[ConfigLine]) -> Result<Self, Box<dyn Error>> {
        let value = |key: &str| -> Result<f32, Box<dyn Error>> {
            let found: Vec<_> = lines
                .iter()
                .filter(|l| l.words.get(1).is_some_and(|w| w == key))
                .collect();
            let [line] = found.as_slice() else {
                return Err(format!("expected one hearing parameter {key}").into());
            };
            let value: f32 = line.words.get(2).ok_or("missing hearing value")?.parse()?;
            if !value.is_finite() || value < 0.0 {
                return Err(format!("invalid hearing parameter {key}").into());
            }
            Ok(value)
        };
        let mut movement = [[0.0; 2]; 3];
        for (i, posture) in ["stand", "crouch", "prone"].into_iter().enumerate() {
            for (j, motion) in ["move", "run"].into_iter().enumerate() {
                movement[i][j] =
                    value(&format!("noise_{posture}_{motion}"))? * crate::map_scene::WORLD_SCALE;
            }
        }
        let result = Self {
            base: value("noise_base")? * crate::map_scene::WORLD_SCALE,
            skill_base: value("stealth_skill_base")?,
            skill_modifier: value("stealth_skill_mod")?,
            movement,
            duration: f64::from(value("audible_duration")?) / 1000.0,
        };
        if result.skill_base > 1000.0
            || result.skill_modifier > 10.0
            || result.base > 1000.0
            || !(0.05..=60.0).contains(&result.duration)
            || result.movement.iter().flatten().any(|v| *v > 1000.0)
        {
            return Err("invalid hearing parameter ranges".into());
        }
        Ok(result)
    }
    fn radius(&self, actor: &Actor) -> f32 {
        if !actor.infiltration.is_finite() || !(0.0..=100.0).contains(&actor.infiltration) {
            return 0.0;
        }
        let Some(movement) = self.movement.get(actor.posture) else {
            return 0.0;
        };
        (self.base
            + movement[usize::from(actor.running)]
                * (self.skill_base - actor.infiltration).max(0.0)
                * self.skill_modifier)
            .max(0.0)
    }
}

pub(crate) struct HearingState {
    settings: Settings,
    clock: f64,
    previous: BTreeMap<ActorId, Vec3>,
    active: Vec<Contact>,
    memories: BTreeMap<ActorId, Vec<Contact>>,
    pub generation: u64,
    pub source: serde_json::Value,
}
impl HearingState {
    pub fn prepare(catalog: &AssetCatalog) -> Result<Self, Box<dyn Error>> {
        let path = "config/perception_var_defs.txt";
        let settings = Settings::read(&config_lines(&catalog.read(path)?)?)?;
        let source = serde_json::json!({"parameters":path,"origin":catalog.source_label(path)?,
            "audible_memory_seconds":settings.duration,"movement_radius_metres":settings.movement,
            "stealth_skill_base":settings.skill_base,"stealth_skill_modifier":settings.skill_modifier,
            "movement_formula_original_verified":false,"acoustic_geometry_attenuation_implemented":false,
            "sound_reveals_visual_contacts":false,"sound_reveals_terrain":false});
        Ok(Self {
            settings,
            clock: 0.0,
            previous: BTreeMap::new(),
            active: Vec::new(),
            memories: BTreeMap::new(),
            generation: 0,
            source,
        })
    }
    pub fn reset(&mut self) {
        self.clock = 0.0;
        self.previous.clear();
        self.active.clear();
        self.memories.clear();
        self.generation = self.generation.wrapping_add(1);
    }
    pub fn contact(&self, listener: ActorId) -> Option<&Contact> {
        self.memories
            .get(&listener)?
            .iter()
            .max_by(|a, b| a.level.cmp(&b.level).then(a.emitted.total_cmp(&b.emitted)))
    }
    pub fn radius(&self, source: ActorId) -> Option<(Vec3, f32)> {
        self.active
            .iter()
            .filter(|c| c.source == source)
            .max_by(|a, b| a.radius.total_cmp(&b.radius))
            .map(|c| (c.position, c.radius))
    }
    pub fn tick(&mut self, dt: f32, actors: &[Actor], shots: &[Shot], paused: bool) {
        if !dt.is_finite() || dt < 0.0 {
            return;
        }
        if paused {
            self.previous = actors
                .iter()
                .filter(|a| a.alive && a.position.is_finite())
                .map(|a| (a.id, a.position))
                .collect();
            return;
        }
        self.clock += f64::from(dt);
        let old_count = self.active.len();
        self.active.retain(|c| c.expires > self.clock);
        let mut changed = old_count != self.active.len();
        self.memories
            .retain(|id, _| actors.iter().any(|a| a.id == *id && a.alive));
        for memories in self.memories.values_mut() {
            memories.retain(|c| c.expires > self.clock);
        }
        let mut emissions = Vec::new();
        for actor in actors.iter().filter(|a| a.alive && a.position.is_finite()) {
            if let Some(before) = self.previous.get(&actor.id) {
                let distance = actor.position.distance(*before);
                // Use resolved displacement. An order blocked by scenery makes no steps.
                // Discontinuities from mission/reset placement are not walking sounds.
                if actor.footsteps && dt > 0.0 && distance > 0.001 && distance / dt <= 20.0 {
                    let radius = self.settings.radius(actor);
                    if radius > 0.0 {
                        emissions.push(Contact {
                            source: actor.id,
                            position: actor.position,
                            radius,
                            level: 1,
                            kind: Kind::Movement,
                            emitted: self.clock,
                            expires: self.clock + self.settings.duration,
                        });
                    }
                }
            }
        }
        self.previous = actors
            .iter()
            .filter(|a| a.alive && a.position.is_finite())
            .map(|a| (a.id, a.position))
            .collect();
        for shot in shots {
            if shot.position.is_finite()
                && shot.radius.is_finite()
                && shot.radius > 0.0
                && shot.level > 0
            {
                emissions.push(Contact {
                    source: shot.source,
                    position: shot.position,
                    radius: shot.radius,
                    level: shot.level,
                    kind: Kind::Shot,
                    emitted: self.clock,
                    expires: self.clock + self.settings.duration,
                });
            }
        }
        for emission in emissions {
            upsert(&mut self.active, emission, 128);
            changed = true;
            // Hear new emissions only. Entering an old shot's radius is not hearing it.
            for listener in actors.iter().filter(|a| a.alive && a.position.is_finite()) {
                if emission.source.hostile_to(listener.id)
                    && listener.position.distance(emission.position) <= emission.radius
                {
                    upsert(self.memories.entry(listener.id).or_default(), emission, 64);
                }
            }
        }
        if changed {
            self.generation = self.generation.wrapping_add(1);
        }
    }
}
pub(crate) fn turn_towards(heading: f32, from: Vec3, to: Vec3, dt: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    if !heading.is_finite() || !from.is_finite() || !to.is_finite() || !dt.is_finite() || dt <= 0.0
    {
        return heading;
    }
    let direction = (to - from).with_y(0.0);
    if !direction.is_finite() || direction.length_squared() < 0.0001 {
        return heading;
    }
    let current = (heading + PI).rem_euclid(TAU) - PI;
    let wanted = (-direction.x).atan2(-direction.z);
    let difference = (wanted - current + PI).rem_euclid(TAU) - PI;
    let step = (PI * dt).min(PI);
    (current + difference.clamp(-step, step) + PI).rem_euclid(TAU) - PI
}

fn upsert(contacts: &mut Vec<Contact>, value: Contact, capacity: usize) {
    if let Some(old) = contacts
        .iter_mut()
        .find(|c| c.source == value.source && c.kind == value.kind && c.level == value.level)
    {
        *old = value;
    } else {
        if contacts.len() >= capacity {
            contacts.remove(0);
        }
        contacts.push(value);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    const PARAMETERS: &[u8] = b"float noise_base 0\nint stealth_skill_base 150\nfloat stealth_skill_mod 0.011\nfloat noise_stand_move 275\nfloat noise_crouch_move 225\nfloat noise_prone_move 225\nfloat noise_stand_run 1250\nfloat noise_crouch_run 540\nfloat noise_prone_run 490\nint audible_duration 2500\n";
    pub(crate) fn fixture() -> HearingState {
        HearingState {
            settings: Settings::read(&config_lines(PARAMETERS).unwrap()).unwrap(),
            clock: 0.0,
            previous: BTreeMap::new(),
            active: vec![],
            memories: BTreeMap::new(),
            generation: 0,
            source: serde_json::Value::Null,
        }
    }
    fn actor(id: ActorId, x: f32) -> Actor {
        Actor {
            id,
            position: Vec3::new(x, 0.0, 0.0),
            infiltration: 50.0,
            posture: 0,
            running: false,
            alive: true,
            footsteps: true,
        }
    }
    fn shot(source: ActorId, position: Vec3, radius: f32, level: u32) -> Shot {
        Shot {
            source,
            position,
            radius,
            level,
        }
    }
    #[test]
    fn quiet_mod_overrides_and_nonwalking_displacement_do_not_create_steps() {
        let mut s = fixture();
        let mut people = [actor(ActorId::John, 0.0), actor(ActorId::Guard(0), 1.0)];
        s.tick(0.1, &people, &[], false);
        people[0].footsteps = false;
        people[0].position.x = 0.1;
        s.tick(0.1, &people, &[], false);
        assert!(s.radius(ActorId::John).is_none());
        let data = String::from_utf8(PARAMETERS.to_vec())
            .unwrap()
            .replace("float stealth_skill_mod 0.011", "float stealth_skill_mod 0");
        s.settings = Settings::read(&config_lines(data.as_bytes()).unwrap()).unwrap();
        people[0].footsteps = true;
        people[0].position.x = 0.2;
        s.tick(0.1, &people, &[], false);
        assert!(s.contact(ActorId::Guard(0)).is_none());
    }
    #[test]
    fn source_stance_running_and_infiltration_change_movement_radius() {
        let s = fixture();
        let mut john = actor(ActorId::John, 0.0);
        john.infiltration = 0.0;
        let loud = s.settings.radius(&john);
        assert!((loud - 4.5375).abs() < 0.0001);
        john.infiltration = 100.0;
        assert!((s.settings.radius(&john) - 1.5125).abs() < 0.0001);
        john.infiltration = 0.0;
        john.running = true;
        assert!((s.settings.radius(&john) - 20.625).abs() < 0.0001);
        let stand = s.settings.radius(&john);
        john.posture = 1;
        let crouch = s.settings.radius(&john);
        john.posture = 2;
        assert!(stand > crouch && crouch > s.settings.radius(&john));
        john.infiltration = f32::NAN;
        assert_eq!(s.settings.radius(&john), 0.0);
    }
    #[test]
    fn movement_uses_resolved_displacement_and_expires_after_stopping() {
        let mut s = fixture();
        let mut people = [actor(ActorId::John, 0.0), actor(ActorId::Guard(0), 1.0)];
        people[0].running = true;
        s.tick(0.1, &people, &[], false);
        s.tick(0.1, &people, &[], false);
        assert!(s.radius(ActorId::John).is_none() && s.contact(ActorId::Guard(0)).is_none());
        people[0].position.x = 0.1;
        s.tick(0.1, &people, &[], false);
        let heard = s.contact(ActorId::Guard(0)).unwrap().position;
        assert_eq!(heard, people[0].position);
        s.tick(2.4, &people, &[], false);
        assert_eq!(s.contact(ActorId::Guard(0)).unwrap().position, heard);
        s.tick(0.11, &people, &[], false);
        assert!(s.contact(ActorId::Guard(0)).is_none() && s.radius(ActorId::John).is_none());
    }
    #[test]
    fn shots_use_their_source_range_and_do_not_reveal_positions_later() {
        let mut s = fixture();
        let mut people = [actor(ActorId::John, 0.0), actor(ActorId::Guard(0), 40.0)];
        s.tick(
            0.0,
            &people,
            &[shot(ActorId::John, Vec3::ZERO, 36.0, 3)],
            false,
        );
        assert!(s.contact(ActorId::Guard(0)).is_none());
        people[1].position.x = 10.0;
        s.tick(0.1, &people, &[], false);
        assert!(s.contact(ActorId::Guard(0)).is_none());
        people[1].position.x = 40.0;
        s.tick(
            0.0,
            &people,
            &[shot(ActorId::John, Vec3::ZERO, 48.0, 4)],
            false,
        );
        assert_eq!(s.contact(ActorId::Guard(0)).unwrap().position, Vec3::ZERO);
        people[0].position.x = 100.0;
        s.tick(0.1, &people, &[], false);
        assert_eq!(s.contact(ActorId::Guard(0)).unwrap().position, Vec3::ZERO);
    }
    #[test]
    fn allies_ignore_each_others_noise_and_dead_listeners_do_not_hear() {
        let mut s = fixture();
        let mut people = [
            actor(ActorId::John, 0.0),
            actor(ActorId::Jack, 1.0),
            actor(ActorId::Guard(0), 2.0),
        ];
        people[2].alive = false;
        s.tick(
            0.0,
            &people,
            &[shot(ActorId::John, Vec3::ZERO, 36.0, 3)],
            false,
        );
        assert!(s.contact(ActorId::Jack).is_none() && s.contact(ActorId::Guard(0)).is_none());
        assert!(s.radius(ActorId::John).is_some());
    }
    #[test]
    fn hearing_memory_freezes_with_the_world_and_reset_discards_all_stimuli() {
        let mut s = fixture();
        let people = [actor(ActorId::John, 0.0), actor(ActorId::Guard(0), 2.0)];
        s.tick(
            0.0,
            &people,
            &[shot(ActorId::John, Vec3::ZERO, 36.0, 3)],
            false,
        );
        s.tick(20.0, &people, &[], true);
        assert!(s.contact(ActorId::Guard(0)).is_some());
        s.reset();
        assert!(s.contact(ActorId::Guard(0)).is_none() && s.radius(ActorId::John).is_none());
    }
    #[test]
    fn weaker_recent_noise_is_retained_when_an_older_loud_noise_expires() {
        let mut s = fixture();
        let mut people = [actor(ActorId::John, 0.0), actor(ActorId::Guard(0), 1.0)];
        s.tick(
            0.0,
            &people,
            &[shot(ActorId::John, Vec3::ZERO, 36.0, 3)],
            false,
        );
        people[0].position.x = 0.2;
        s.tick(1.0, &people, &[], false);
        assert_eq!(s.contact(ActorId::Guard(0)).unwrap().position, Vec3::ZERO);
        s.tick(1.6, &people, &[], false);
        assert_eq!(
            s.contact(ActorId::Guard(0)).unwrap().position,
            people[0].position
        );
    }
    #[test]
    fn resets_teleports_and_invalid_parameters_cannot_generate_footsteps() {
        let mut s = fixture();
        let mut people = [actor(ActorId::John, 0.0), actor(ActorId::Guard(0), 45.0)];
        s.tick(0.1, &people, &[], false);
        people[0].position.x = 50.0;
        s.tick(0.1, &people, &[], false);
        assert!(s.radius(ActorId::John).is_none());
        for replacement in [
            "float stealth_skill_mod NaN",
            "float stealth_skill_mod -1",
            "float stealth_skill_mod 1001",
        ] {
            let data = String::from_utf8(PARAMETERS.to_vec())
                .unwrap()
                .replace("float stealth_skill_mod 0.011", replacement);
            assert!(Settings::read(&config_lines(data.as_bytes()).unwrap()).is_err());
        }
        let mut lines = config_lines(PARAMETERS).unwrap();
        lines.push(lines[0].clone());
        assert!(Settings::read(&lines).is_err());
    }
    #[test]
    fn continuous_bursts_keep_sound_memory_bounded() {
        let mut s = fixture();
        let people = [actor(ActorId::John, 0.0)];
        let shots: Vec<_> = (0..600)
            .map(|i| shot(ActorId::Guard(i), Vec3::ZERO, 36.0, 3))
            .collect();
        s.tick(0.0, &people, &shots, false);
        assert!(s.active.len() <= 128 && s.memories[&ActorId::John].len() <= 64);
        s.tick(2.6, &people, &[], false);
        assert!(s.contact(ActorId::John).is_none());
    }
    #[test]
    fn turning_takes_the_short_path_and_is_bounded_by_elapsed_time() {
        let a = 179.0f32.to_radians();
        let target = Vec3::new(0.0174524, 0.0, 0.9998477);
        let next = turn_towards(a, Vec3::ZERO, target, 0.01);
        let difference = (next - a + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)
            - std::f32::consts::PI;
        assert!(difference.abs() <= std::f32::consts::PI * 0.01001);
        assert_eq!(turn_towards(a, Vec3::ZERO, Vec3::ZERO, 1.0), a);
    }
}
