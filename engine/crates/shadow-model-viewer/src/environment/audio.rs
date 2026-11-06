//! Small procedural ambience sources; no original or external recordings.
use super::*;
use bevy::audio::{AudioSink, AudioSinkPlayback, Volume};
use std::io::Cursor;
#[derive(Resource)]
pub(crate) struct Gain(pub f32);
impl Default for Gain {
    fn default() -> Self {
        Self(1.)
    }
}
#[derive(Component, Clone, Copy)]
pub(super) enum Sound {
    Rain,
    Wind,
    Thunder(f32),
}
#[derive(Resource)]
pub(super) struct WeatherAudio {
    rain: Handle<AudioSource>,
    wind: Handle<AudioSource>,
    thunder: Handle<AudioSource>,
    generation: u64,
}
fn source(kind: u8) -> AudioSource {
    let rate = 24_000u32;
    let seconds = if kind == 2 { 6 } else { 4 };
    let mut cursor = Cursor::new(Vec::new());
    {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::new(&mut cursor, spec).expect("weather WAV buffer");
        let mut rng = 0x5a33_9171u32;
        let mut low = 0.;
        let mut lower = 0.;
        for i in 0..rate * seconds {
            rng ^= rng << 13;
            rng ^= rng >> 17;
            rng ^= rng << 5;
            let white = (rng as f32 / u32::MAX as f32) * 2. - 1.;
            let t = i as f32 / rate as f32;
            low += 0.035 * (white - low);
            lower += 0.004 * (white - lower);
            let sound = match kind {
                0 => white * 0.12 + low * 0.8,
                1 => (low * 0.8 + lower * 2.) * (0.65 + 0.2 * (t * 1.57).sin()),
                _ => {
                    (low * 1.6 + (t * 43.).sin() * lower * 2.5)
                        * (1. - (-t * 9.).exp())
                        * (-t * 0.65).exp()
                }
            };
            let fade = (t * 20.).min(1.) * ((seconds as f32 - t) * 20.).min(1.);
            writer
                .write_sample((sound * fade * 0.85 * 32767.).clamp(-32767., 32767.) as i16)
                .expect("weather sample");
        }
        writer.finalize().expect("weather WAV header");
    }
    AudioSource {
        bytes: cursor.into_inner().into(),
    }
}
pub(super) fn update(
    mut commands: Commands,
    weather: Option<Res<Weather>>,
    state: Option<ResMut<WeatherAudio>>,
    gain: Res<Gain>,
    mut sources: ResMut<Assets<AudioSource>>,
    mut sinks: Query<(&Sound, &mut AudioSink)>,
) {
    let Some(w) = weather else {
        return;
    };
    let Some(mut state) = state else {
        let rain = sources.add(source(0));
        let wind = sources.add(source(1));
        let thunder = sources.add(source(2));
        for (role, handle) in [(Sound::Rain, rain.clone()), (Sound::Wind, wind.clone())] {
            commands.spawn((
                player::SceneEntity,
                role,
                AudioPlayer(handle),
                PlaybackSettings::LOOP.with_volume(Volume::Linear(0.)),
            ));
        }
        commands.insert_resource(WeatherAudio {
            rain,
            wind,
            thunder,
            generation: w.thunder_generation,
        });
        return;
    };
    // Hold strong handles for the looping sources until mission cleanup.
    let _ = (&state.rain, &state.wind);
    let master = gain.0.clamp(0., 1.);
    for (role, mut sink) in &mut sinks {
        let level = match role {
            Sound::Rain => w.sample.rain * 0.48 + w.sample.hail * 0.1,
            Sound::Wind => (w.sample.wind.length() / 24.).clamp(0., 1.) * 0.5,
            Sound::Thunder(level) => *level,
        };
        sink.set_volume(Volume::Linear(level * master));
        if w.paused {
            sink.pause();
        } else {
            sink.play();
        }
    }
    if state.generation != w.thunder_generation {
        let level = 0.9 / (1. + w.thunder_distance / 200.);
        commands.spawn((
            player::SceneEntity,
            Sound::Thunder(level),
            AudioPlayer(state.thunder.clone()),
            PlaybackSettings::DESPAWN.with_volume(Volume::Linear(level * master)),
        ));
        state.generation = w.thunder_generation;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn procedural_ambience_is_valid_bounded_pcm_with_nonzero_energy() {
        for kind in 0..3 {
            let s = source(kind);
            let mut reader = hound::WavReader::new(Cursor::new(&s.bytes)).unwrap();
            assert_eq!(reader.spec().sample_rate, 24_000);
            assert_eq!(reader.spec().channels, 1);
            let samples = reader
                .samples::<i16>()
                .map(Result::unwrap)
                .collect::<Vec<_>>();
            assert!(samples.len() >= 96_000);
            assert!(samples.iter().any(|s| s.unsigned_abs() > 100));
            assert!(samples.iter().all(|s| s.unsigned_abs() < 32768));
        }
    }
}
