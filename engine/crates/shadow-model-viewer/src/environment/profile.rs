//! Bounded weather presets, independent of renderer and map dimensions.
use bevy::prelude::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Preset {
    #[default]
    Mission,
    Clear,
    Cloudy,
    Overcast,
    Drizzle,
    Rain,
    Downpour,
    Thunderstorm,
    Fog,
    Snow,
    Sleet,
    Hail,
    DustStorm,
    Blizzard,
}
impl Preset {
    pub const ALL: [Self; 14] = [
        Self::Mission,
        Self::Clear,
        Self::Cloudy,
        Self::Overcast,
        Self::Drizzle,
        Self::Rain,
        Self::Downpour,
        Self::Thunderstorm,
        Self::Fog,
        Self::Snow,
        Self::Sleet,
        Self::Hail,
        Self::DustStorm,
        Self::Blizzard,
    ];
    pub fn key(self) -> &'static str {
        match self {
            Self::Mission => "mission",
            Self::Clear => "clear",
            Self::Cloudy => "cloudy",
            Self::Overcast => "overcast",
            Self::Drizzle => "drizzle",
            Self::Rain => "rain",
            Self::Downpour => "downpour",
            Self::Thunderstorm => "thunderstorm",
            Self::Fog => "fog",
            Self::Snow => "snow",
            Self::Sleet => "sleet",
            Self::Hail => "hail",
            Self::DustStorm => "dust-storm",
            Self::Blizzard => "blizzard",
        }
    }
    pub fn parse(value: &str) -> Result<Self, &'static str> {
        Self::ALL
            .into_iter()
            .find(|p| p.key() == value)
            .ok_or("unknown weather preset")
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Mission => "По сценарию",
            Self::Clear => "Ясно",
            Self::Cloudy => "Облачно",
            Self::Overcast => "Пасмурно",
            Self::Drizzle => "Морось",
            Self::Rain => "Дождь",
            Self::Downpour => "Ливень",
            Self::Thunderstorm => "Гроза",
            Self::Fog => "Туман",
            Self::Snow => "Снег",
            Self::Sleet => "Мокрый снег",
            Self::Hail => "Град",
            Self::DustStorm => "Пылевая буря",
            Self::Blizzard => "Метель",
        }
    }
    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|p| *p == self).unwrap();
        Self::ALL[(i + 1) % Self::ALL.len()]
    }
    pub fn sample(self) -> Sample {
        let mut s = Sample::default();
        match self {
            Self::Mission | Self::Clear => {}
            Self::Cloudy => {
                s.cloud = 0.48;
                s.sun = 0.82;
                s.wind = Vec2::new(2.5, 0.8);
            }
            Self::Overcast => {
                s.cloud = 0.97;
                s.sun = 0.25;
                s.visibility = 1800.;
            }
            Self::Drizzle => {
                s.cloud = 0.88;
                s.rain = 0.18;
                s.sun = 0.4;
                s.visibility = 650.;
            }
            Self::Rain => {
                s.cloud = 0.96;
                s.rain = 0.55;
                s.sun = 0.28;
                s.wind = Vec2::new(4., 1.5);
                s.visibility = 400.;
            }
            Self::Downpour => {
                s.cloud = 1.;
                s.rain = 1.;
                s.sun = 0.12;
                s.wind = Vec2::new(8., 3.);
                s.visibility = 150.;
            }
            Self::Thunderstorm => {
                s.cloud = 1.;
                s.rain = 1.;
                s.sun = 0.08;
                s.wind = Vec2::new(12., 4.);
                s.visibility = 120.;
                s.thunder = 1.;
            }
            Self::Fog => {
                s.cloud = 0.65;
                s.sun = 0.48;
                s.visibility = 35.;
                s.wind = Vec2::new(0.3, 0.15);
            }
            Self::Snow => {
                s.cloud = 0.9;
                s.snow = 0.65;
                s.sun = 0.4;
                s.temperature = -5.;
                s.wind = Vec2::new(2., 0.8);
                s.visibility = 280.;
            }
            Self::Sleet => {
                s.cloud = 0.96;
                s.rain = 0.4;
                s.snow = 0.5;
                s.sun = 0.3;
                s.temperature = 0.5;
                s.visibility = 220.;
            }
            Self::Hail => {
                s.cloud = 1.;
                s.rain = 0.65;
                s.hail = 1.;
                s.sun = 0.15;
                s.wind = Vec2::new(7., 2.);
                s.visibility = 200.;
                s.thunder = 0.35;
            }
            Self::DustStorm => {
                s.cloud = 0.35;
                s.dust = 1.;
                s.sun = 0.22;
                s.wind = Vec2::new(18., 5.);
                s.visibility = 45.;
                s.temperature = 32.;
            }
            Self::Blizzard => {
                s.cloud = 1.;
                s.snow = 1.;
                s.sun = 0.16;
                s.temperature = -14.;
                s.wind = Vec2::new(20., 6.);
                s.visibility = 28.;
            }
        }
        s
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Quality {
    #[default]
    Auto,
    Low,
    Medium,
    High,
}
impl Quality {
    pub fn key(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
    pub fn parse(value: &str) -> Result<Self, &'static str> {
        [Self::Auto, Self::Low, Self::Medium, Self::High]
            .into_iter()
            .find(|p| p.key() == value)
            .ok_or("unknown weather quality")
    }
    pub fn next(self) -> Self {
        match self {
            Self::Auto => Self::Low,
            Self::Low => Self::Medium,
            Self::Medium => Self::High,
            Self::High => Self::Auto,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Автоматически",
            Self::Low => "Низкое",
            Self::Medium => "Среднее",
            Self::High => "Высокое",
        }
    }
    pub fn particles(self) -> usize {
        match self {
            Self::Low => 384,
            Self::High => 3072,
            _ => 1280,
        }
    }
    pub fn cloud_steps(self) -> u32 {
        match self {
            Self::Low => 2,
            Self::High => 16,
            _ => 8,
        }
    }
    pub fn resolve(self, software: bool) -> Self {
        if self == Self::Auto {
            if software { Self::Low } else { Self::Medium }
        } else {
            self
        }
    }
}

#[derive(Resource, Clone, Debug, PartialEq)]
pub(crate) struct Options {
    pub preset: Preset,
    pub quality: Quality,
    pub hour: Option<f32>,
    /// Real minutes per full day; zero keeps the mission's initial time.
    pub day_minutes: f32,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            preset: Preset::Mission,
            quality: Quality::Auto,
            hour: None,
            day_minutes: 120.,
        }
    }
}
impl Options {
    pub fn json(&self) -> serde_json::Value {
        serde_json::json!({"preset":self.preset.key(),"quality":self.quality.key(),"hour":self.hour,"day_minutes":self.day_minutes})
    }
    pub fn from_json(v: &serde_json::Value) -> Result<Self, Box<dyn std::error::Error>> {
        if v.is_null() {
            return Ok(Self::default());
        }
        let preset = Preset::parse(v["preset"].as_str().ok_or("missing weather preset")?)?;
        let quality = Quality::parse(v["quality"].as_str().ok_or("missing weather quality")?)?;
        let hour = if v["hour"].is_null() {
            None
        } else {
            Some(v["hour"].as_f64().ok_or("invalid weather hour")? as f32)
        };
        let day_minutes = v["day_minutes"].as_f64().ok_or("invalid weather clock")? as f32;
        if hour.is_some_and(|h| !h.is_finite() || !(0.0..24.).contains(&h))
            || !day_minutes.is_finite()
            || !(day_minutes == 0. || (15.0..=1440.).contains(&day_minutes))
        {
            return Err("weather clock outside bounds".into());
        }
        Ok(Self {
            preset,
            quality,
            hour,
            day_minutes,
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Sample {
    pub cloud: f32,
    pub rain: f32,
    pub snow: f32,
    pub hail: f32,
    pub dust: f32,
    pub sun: f32,
    pub visibility: f32,
    pub temperature: f32,
    pub wind: Vec2,
    pub thunder: f32,
}
impl Default for Sample {
    fn default() -> Self {
        Self {
            cloud: 0.15,
            rain: 0.,
            snow: 0.,
            hail: 0.,
            dust: 0.,
            sun: 1.,
            visibility: 6000.,
            temperature: 22.,
            wind: Vec2::new(1.3, 0.4),
            thunder: 0.,
        }
    }
}
impl Sample {
    pub fn blend(self, b: Self, t: f32) -> Self {
        let t = t.clamp(0., 1.);
        let f = |a: f32, b: f32| a + (b - a) * t;
        Self {
            cloud: f(self.cloud, b.cloud),
            rain: f(self.rain, b.rain),
            snow: f(self.snow, b.snow),
            hail: f(self.hail, b.hail),
            dust: f(self.dust, b.dust),
            sun: f(self.sun, b.sun),
            // Blend optical density, avoiding a nearly clear midpoint into fog.
            visibility: 1. / f(1. / self.visibility, 1. / b.visibility),
            temperature: f(self.temperature, b.temperature),
            wind: self.wind.lerp(b.wind, t),
            thunder: f(self.thunder, b.thunder),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_presets_have_bounded_physical_parameters_and_roundtrip_options() {
        for preset in Preset::ALL {
            let s = preset.sample();
            for x in [s.cloud, s.rain, s.snow, s.hail, s.dust, s.sun, s.thunder] {
                assert!((0.0..=1.).contains(&x));
            }
            assert!((20.0..=10_000.).contains(&s.visibility));
            assert!(s.wind.length() < 30.);
            let o = Options {
                preset,
                ..default()
            };
            assert_eq!(Options::from_json(&o.json()).unwrap(), o);
        }
        for bad in [
            serde_json::json!({"preset":"unbounded"}),
            serde_json::json!({"preset":"rain","quality":"high","hour":24,"day_minutes":120}),
        ] {
            assert!(Options::from_json(&bad).is_err());
        }
    }
    #[test]
    fn optical_density_transition_is_finite_and_reaches_both_endpoints() {
        let a = Preset::Clear.sample();
        let b = Preset::Fog.sample();
        assert_eq!(a.blend(b, 0.).visibility, a.visibility);
        assert!((a.blend(b, 1.).visibility - b.visibility).abs() < 0.001);
        assert!(a.blend(b, 0.5).visibility < 100.);
    }
}
