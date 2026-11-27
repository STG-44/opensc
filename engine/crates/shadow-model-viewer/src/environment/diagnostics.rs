use super::*;
use std::path::PathBuf;
#[derive(Resource)]
pub(crate) struct Probe {
    pub path: Option<PathBuf>,
    pub time: Option<f32>,
    pub force_flash: bool,
    frames: u32,
    peak_flash: f32,
    peak_particles: usize,
    done: bool,
}
impl Probe {
    pub fn new(path: Option<PathBuf>, time: Option<f32>, force_flash: bool) -> Self {
        Self {
            path,
            time,
            force_flash,
            frames: 0,
            peak_flash: 0.,
            peak_particles: 0,
            done: false,
        }
    }
}
pub(super) fn probe(
    probe: Option<ResMut<Probe>>,
    weather: Option<Res<Weather>>,
    options: Res<Options>,
    particles: Option<Res<particles::Particles>>,
) {
    let (Some(mut p), Some(w)) = (probe, weather) else {
        return;
    };
    if p.done {
        return;
    };
    p.frames += 1;
    p.peak_flash = p.peak_flash.max(w.flash);
    if let Some(state) = &particles {
        p.peak_particles = p.peak_particles.max(state.active);
    }
    if p.frames < 110 {
        return;
    }
    let sample = w.sample();
    assert!(sample.hour.is_finite() && (0.0..24.).contains(&sample.hour));
    assert!(sample.sun_to.is_finite() && (sample.sun_to.length() - 1.).abs() < 0.001);
    assert!(sample.profile.visibility >= 20.);
    assert!(p.peak_particles <= w.quality.particles());
    let precipitation =
        sample.profile.rain + sample.profile.snow + sample.profile.hail + sample.profile.dust;
    if precipitation > 0.1 {
        assert!(p.peak_particles > 0, "precipitation must render particles");
    }
    if let Some(path) = &p.path {
        let value = serde_json::json!({"status":"passed","frames":p.frames,"preset":options.preset.key(),"quality":w.quality.key(),"hour":sample.hour,"elapsed_seconds":w.elapsed,"fixed_snapshot":p.time,"forced_lightning_snapshot":p.force_flash,
            "illumination":sample.illumination,"sun_direction":sample.sun_to.to_array(),"cloud_coverage":sample.profile.cloud,"visibility_metres":sample.profile.visibility,"wind_metres_per_second":sample.profile.wind.to_array(),"rain":sample.profile.rain,"snow":sample.profile.snow,"hail":sample.profile.hail,"dust":sample.profile.dust,
            "wetness":sample.wetness,"snow_cover":sample.snow_cover,"peak_particles":p.peak_particles,"particle_budget":w.quality.particles(),"peak_lightning_flash":p.peak_flash,"lightning_strikes":w.strike_generation,"thunder_events":w.thunder_generation,"precipitation_impacts":particles.as_ref().map_or(0,|p|p.impacts)});
        std::fs::write(path, serde_json::to_string_pretty(&value).unwrap() + "\n")
            .expect("write weather diagnostics");
    }
    p.done = true;
}
