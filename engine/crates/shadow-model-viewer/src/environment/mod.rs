//! Shared sky/weather presentation. Original installation data is read-only.
use crate::{daylight, inventory::Inventory, player, training, tutorial::Tutorial};
use bevy::prelude::*;
mod audio;
mod diagnostics;
mod particles;
pub(crate) mod profile;
mod sky;
mod surfaces;
pub(crate) use audio::Gain;
pub(crate) use diagnostics::Probe;
pub(crate) use profile::{Options, Preset, Quality, Sample};
pub(crate) use surfaces::ExposedSurface;
pub(crate) fn prepare_shelter(commands: &mut Commands, scene: &crate::map_scene::MapScene) {
    commands.insert_resource(particles::Shelter::prepare(scene));
}
fn shader_source(name: &str, embedded: &'static str) -> std::borrow::Cow<'static, str> {
    #[cfg(debug_assertions)]
    if let Some(root) = std::env::var_os("OPENSC_WEATHER_SHADER_DIR") {
        let path = std::path::PathBuf::from(root).join(name);
        if let Ok(text) = std::fs::read_to_string(path) {
            return std::borrow::Cow::Owned(text);
        }
    }
    let _ = name;
    std::borrow::Cow::Borrowed(embedded)
}

pub(crate) struct EnvironmentPlugin;
impl Plugin for EnvironmentPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Options>()
            .init_resource::<audio::Gain>()
            .add_plugins((
                sky::SkyPlugin,
                particles::ParticlePlugin,
                surfaces::SurfacePlugin,
            ))
            .add_systems(
                Update,
                (
                    sky::initialize,
                    control,
                    sky::update,
                    particles::update,
                    surfaces::update,
                    audio::update,
                    diagnostics::probe,
                )
                    .chain()
                    .after(training::CameraUpdate)
                    .before(crate::vehicle::weather::animate),
            );
    }
}

#[derive(Resource)]
pub(crate) struct Weather {
    pub sample: Sample,
    from: Sample,
    target: Sample,
    transition: f32,
    selected: Preset,
    pub quality: Quality,
    pub elapsed: f64,
    clock_elapsed: f64,
    clock_options: (Option<f32>, f32),
    initial_hour: f32,
    pub hour: f32,
    pub sun_to: Vec3,
    pub sun_color: Vec3,
    pub daylight: f32,
    pub illumination: f32,
    pub wetness: f32,
    pub snow_cover: f32,
    pub flash: f32,
    pub strike: Option<Vec3>,
    pub strike_generation: u64,
    pub thunder_generation: u64,
    pub thunder_distance: f32,
    lightning_time: f32,
    lightning_age: f32,
    thunder_delay: Option<f32>,
    rng: u32,
    tables: Option<daylight::Tables>,
    fallback_sun: Vec3,
    pub fixed_time: Option<f32>,
    pub paused: bool,
    legacy_preview: Option<String>,
}
impl Weather {
    pub fn new(daylight: &daylight::Daylight, options: &Options) -> Self {
        let preview = daylight.source["environment_preview"]
            .as_str()
            .map(str::to_string);
        let initial_hour = daylight.source["sample_hour"].as_f64().unwrap_or(12.) as f32;
        let preset = if options.preset == Preset::Mission
            && preview.as_deref().is_some_and(|s| s.ends_with("-rain"))
        {
            Preset::Rain
        } else {
            options.preset
        };
        let sample = preset.sample();
        let mut result = Self {
            sample,
            from: sample,
            target: sample,
            transition: 1.,
            selected: options.preset,
            quality: options.quality,
            elapsed: 0.,
            clock_elapsed: 0.,
            clock_options: (options.hour, options.day_minutes),
            initial_hour,
            hour: initial_hour,
            sun_to: Vec3::Y,
            sun_color: Vec3::ONE,
            daylight: 1.,
            illumination: 1.,
            wetness: 0.,
            snow_cover: 0.,
            flash: 0.,
            strike: None,
            strike_generation: 0,
            thunder_generation: 0,
            thunder_distance: 0.,
            lightning_time: 8.,
            lightning_age: 10.,
            thunder_delay: None,
            rng: 0x5343_2026,
            tables: daylight.tables.clone(),
            fallback_sun: -(daylight.rotation * Vec3::NEG_Z),
            fixed_time: None,
            paused: false,
            legacy_preview: preview,
        };
        result.light(options);
        result
    }
    pub fn prepare_snapshot(&mut self, probe: &Probe, options: &Options, focus: Vec3) {
        if let Some(seconds) = probe.time {
            let mut left = seconds;
            while left > 0. {
                let step = left.min(0.1);
                self.advance(step, options, focus);
                left -= step;
            }
            self.fixed_time = Some(seconds);
            if probe.force_flash {
                self.strike = Some(focus + Vec3::new(-65., 0., 45.));
                self.strike_generation += 1;
                self.lightning_age = 0.001;
                self.flash = flash_envelope(self.lightning_age) * self.sample.thunder;
            }
        }
    }
    fn light(&mut self, options: &Options) {
        if self.clock_options != (options.hour, options.day_minutes) {
            self.clock_options = (options.hour, options.day_minutes);
            self.clock_elapsed = 0.;
        }
        let legacy_hour = self
            .legacy_preview
            .as_deref()
            .map(|p| if p.starts_with("night-") { 0. } else { 12. });
        let start = options.hour.or(legacy_hour).unwrap_or(self.initial_hour);
        let delta = if options.day_minutes > 0. {
            self.clock_elapsed as f32 * 24. / (options.day_minutes * 60.)
        } else {
            0.
        };
        self.hour = (start + delta).rem_euclid(24.);
        if let Some(tables) = &self.tables {
            let (direction, color) = tables
                .sample(self.hour)
                .expect("validated mission sunlight");
            self.sun_to = -direction;
            self.sun_color = color;
        } else {
            let angle = (self.hour - self.initial_hour) * std::f32::consts::TAU / 24.;
            self.sun_to = Quat::from_rotation_z(angle) * self.fallback_sun;
            self.sun_color = Vec3::ONE;
        }
        // Real-time overrides need a sun that actually rises and sets even
        // when legacy night tables keep a positive ambient sun contribution.
        if options.hour.is_some() || self.legacy_preview.is_some() {
            let a = (self.hour - 6.) * std::f32::consts::PI / 12.;
            self.sun_to = Vec3::new(-a.cos(), a.sin() * 0.9, a.sin() * 0.4).normalize();
            self.sun_color = Vec3::ONE;
        }
        self.daylight = smooth(-0.08, 0.18, self.sun_to.y);
        self.illumination = self.daylight * (0.48 + 0.52 * self.sample.sun);
    }
    fn random(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng as f32 / u32::MAX as f32
    }
    fn advance(&mut self, dt: f32, options: &Options, focus: Vec3) {
        if self.selected != options.preset {
            self.from = self.sample;
            self.target = options.preset.sample();
            self.transition = 0.;
            self.selected = options.preset;
        }
        self.transition = (self.transition + dt / 12.).min(1.);
        self.sample = self
            .from
            .blend(self.target, smooth(0., 1., self.transition));
        self.elapsed += f64::from(dt);
        self.clock_elapsed += f64::from(dt);
        self.wetness = (self.wetness
            + dt * (self.sample.rain * 0.05 + self.sample.hail * 0.01 - self.sample.sun * 0.0015))
            .clamp(0., 1.);
        self.snow_cover = (self.snow_cover
            + dt * (self.sample.snow * 0.004
                - ((self.sample.temperature - 1.).max(0.) * 0.0004 + self.sample.rain * 0.002)))
            .clamp(0., 1.);
        self.light(options);
        self.lightning_age += dt;
        self.flash = flash_envelope(self.lightning_age) * self.sample.thunder;
        if let Some(delay) = &mut self.thunder_delay {
            *delay -= dt;
            if *delay <= 0. {
                self.thunder_generation += 1;
                self.thunder_delay = None;
            }
        }
        if self.sample.thunder > 0.05 {
            self.lightning_time -= dt * self.sample.thunder;
            if self.lightning_time <= 0. {
                let angle = self.random() * std::f32::consts::TAU;
                let distance = 65. + self.random() * 180.;
                self.strike =
                    Some(focus + Vec3::new(angle.cos() * distance, 0., angle.sin() * distance));
                self.thunder_distance = distance;
                self.thunder_delay = Some(distance / 343.);
                self.strike_generation += 1;
                self.lightning_age = 0.;
                self.flash = self.sample.thunder;
                self.lightning_time = 9. + self.random() * 20.;
            }
        }
    }
}
pub(crate) fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0., 1.);
    t * t * (3. - 2. * t)
}
fn flash_envelope(age: f32) -> f32 {
    if age < 0.08 {
        1. - age / 0.08
    } else if (0.15..0.24).contains(&age) {
        (1. - (age - 0.15) / 0.09) * 0.65
    } else {
        0.
    }
}
#[allow(clippy::too_many_arguments)]
fn control(
    time: Res<Time>,
    options: Res<Options>,
    weather: Option<ResMut<Weather>>,
    mut conditions: Option<ResMut<crate::vehicle::weather::Conditions>>,
    p: Option<Res<player::Player>>,
    inv: Option<Res<Inventory>>,
    tutorial: Option<Res<Tutorial>>,
    orbit: Option<Res<training::Orbit>>,
) {
    let Some(mut w) = weather else {
        return;
    };
    let paused = p.is_some_and(|p| p.playback.paused)
        || inv.is_some_and(|i| i.open)
        || tutorial.is_some_and(|t| t.pausing());
    w.paused = paused;
    let focus = orbit.map_or(Vec3::ZERO, |o| o.target);
    if !paused {
        let dt = if w.fixed_time.is_some() {
            0.
        } else {
            time.delta_secs().clamp(0., 0.25)
        };
        w.advance(dt, &options, focus);
    }
    if let Some(c) = conditions.as_mut() {
        c.illumination = w.illumination;
        c.rain = w.sample.rain;
    }
}

pub(crate) fn cleanup(commands: &mut Commands) {
    commands.remove_resource::<Weather>();
    commands.remove_resource::<sky::Visuals>();
    commands.remove_resource::<particles::Particles>();
    commands.remove_resource::<particles::Shelter>();
    commands.remove_resource::<surfaces::Surfaces>();
    commands.remove_resource::<audio::WeatherAudio>();
    commands.remove_resource::<diagnostics::Probe>();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lightning_is_bounded_and_does_not_flash_continuously() {
        assert_eq!(flash_envelope(0.), 1.);
        assert_eq!(flash_envelope(0.1), 0.);
        for i in 0..1000 {
            assert!((0.0..=1.).contains(&flash_envelope(i as f32 / 1000.)));
        }
        assert_eq!(flash_envelope(10.), 0.);
    }
}
