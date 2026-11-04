//! Initial mission sunlight from MTF time and the campaign's hourly TOD tables.
use bevy::prelude::*;
use shadow_formats::{AssetCatalog, ConfigLine, config_lines};
use std::{error::Error, f32::consts::FRAC_PI_2};

struct Tables {
    direction: [[f32; 24]; 3],
    color: [[f32; 24]; 3],
}
impl Tables {
    fn read(lines: &[ConfigLine]) -> Result<Self, Box<dyn Error>> {
        let row = |key: &str, color: bool| -> Result<[f32; 24], Box<dyn Error>> {
            let rows: Vec<_> = lines
                .iter()
                .filter(|l| l.words.first().is_some_and(|w| w == key))
                .collect();
            let [line] = rows.as_slice() else {
                return Err(format!("expected one {key} row").into());
            };
            if line.words.len() != 25 {
                return Err(format!("{key} requires 24 hourly samples").into());
            }
            let mut values = [0.0f32; 24];
            for (value, word) in values.iter_mut().zip(&line.words[1..]) {
                *value = word.parse()?;
                if !value.is_finite()
                    || (color && !(0.0..=1.0).contains(value))
                    || (!color && value.abs() > 100.0)
                {
                    return Err(format!("invalid {key} sample").into());
                }
            }
            Ok(values)
        };
        let result = Self {
            direction: [
                row("TOD_DATA_SUN_X", false)?,
                row("TOD_DATA_SUN_Y", false)?,
                row("TOD_DATA_SUN_Z", false)?,
            ],
            color: [
                row("TOD_DATA_SUN_R", true)?,
                row("TOD_DATA_SUN_G", true)?,
                row("TOD_DATA_SUN_B", true)?,
            ],
        };
        for hour in 0..24 {
            result.sample(hour as f32)?;
        }
        Ok(result)
    }
    fn sample(&self, hour: f32) -> Result<(Vec3, Vec3), Box<dyn Error>> {
        if !hour.is_finite() {
            return Err("nonfinite mission time".into());
        }
        let hour = hour.rem_euclid(24.0);
        let a = hour.floor() as usize;
        let b = (a + 1) % 24;
        let t = hour.fract();
        let sample = |rows: &[[f32; 24]; 3]| {
            Vec3::from_array(std::array::from_fn(|i| {
                rows[i][a] + (rows[i][b] - rows[i][a]) * t
            }))
        };
        // MTF uses Z up; the Bevy world uses Y up and negative Z for source Y.
        let direction = (Quat::from_rotation_x(-FRAC_PI_2) * sample(&self.direction))
            .try_normalize()
            .ok_or("zero sunlight direction")?;
        Ok((direction, sample(&self.color)))
    }
}
fn time(lines: &[ConfigLine]) -> Result<Option<f32>, Box<dyn Error>> {
    let rows: Vec<_> = lines
        .iter()
        .filter(|l| {
            l.words
                .first()
                .is_some_and(|w| w == "GAME_STATE_TIME_OF_DAY")
        })
        .collect();
    if rows.is_empty() {
        return Ok(None);
    }
    let [line] = rows.as_slice() else {
        return Err("duplicate map time".into());
    };
    if line.words.len() != 3 {
        return Err("map time requires hour and minute".into());
    }
    let hour: u8 = line.words[1].parse()?;
    let minute: u8 = line.words[2].parse()?;
    if hour >= 24 || minute >= 60 {
        return Err("map time outside 00:00..23:59".into());
    }
    Ok(Some(f32::from(hour) + f32::from(minute) / 60.0))
}

pub(crate) struct Daylight {
    pub rotation: Quat,
    pub color: Color,
    pub source: serde_json::Value,
}
impl Daylight {
    pub fn prepare(
        catalog: &AssetCatalog,
        map: &str,
        globals: &[ConfigLine],
    ) -> Result<Self, Box<dyn Error>> {
        let initial = time(globals)?;
        let mut matches = Vec::new();
        let basename = map.rsplit('/').next().ok_or("map has no filename")?;
        for path in catalog
            .paths()
            .filter(|p| p.starts_with("campaign/") && p.ends_with(".txt"))
        {
            let pieces: Vec<_> = path.split('/').collect();
            if pieces.len() != 3 || pieces[2].strip_suffix(".txt") != Some(pieces[1]) {
                continue;
            }
            let lines = config_lines(&catalog.read(path)?)?;
            if lines.iter().any(|l| {
                l.words.first().is_some_and(|w| w == "SPECIFY_MTF")
                    && l.words
                        .get(1)
                        .is_some_and(|w| w.eq_ignore_ascii_case(basename))
            }) {
                matches.push((path.to_string(), lines));
            }
        }
        if matches.len() > 1 {
            return Err("ambiguous campaign lighting for map".into());
        }
        if let Some((path, lines)) = matches.pop() {
            let (direction, color) = Tables::read(&lines)?.sample(initial.unwrap_or(12.0))?;
            let source = serde_json::json!({"campaign":path,"campaign_origin":catalog.source_label(&path)?,
                "map":map,"initial_hour":initial,"sample_hour":initial.unwrap_or(12.0),
                "direction":direction.to_array(),"linear_rgb":color.to_array(),
                "hourly_interpolation":"linear, wrapping at midnight","continuous_time_progression":false,
                "original_renderer_photometry_verified":false,"perception_lighting_implemented":false});
            Ok(Self {
                rotation: Quat::from_rotation_arc(Vec3::NEG_Z, direction),
                color: Color::linear_rgb(color.x, color.y, color.z),
                source,
            })
        } else {
            Ok(Self {
                rotation: Transform::from_xyz(40.0, 70.0, -50.0)
                    .looking_at(Vec3::ZERO, Vec3::Y)
                    .rotation,
                color: Color::WHITE,
                source: serde_json::json!({"map":map,"initial_hour":initial,
                    "fallback":"no matching campaign lighting table","original_renderer_photometry_verified":false}),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tables() -> Tables {
        let mut direction = [[0.0; 24]; 3];
        direction[2] = [-1.0; 24];
        let mut color = [[0.0; 24]; 3];
        for row in &mut color {
            row[0] = 1.0;
            row[23] = 0.2;
        }
        Tables { direction, color }
    }
    #[test]
    fn midnight_interpolation_wraps_and_source_down_becomes_world_down() {
        let t = tables();
        let (direction, color) = t.sample(23.5).unwrap();
        assert!(direction.distance(Vec3::NEG_Y) < 0.0001);
        assert!(color.distance(Vec3::splat(0.6)) < 0.0001);
        assert_eq!(t.sample(24.0).unwrap(), t.sample(0.0).unwrap());
        let rotation = Quat::from_rotation_arc(Vec3::NEG_Z, direction);
        assert!((rotation * Vec3::NEG_Z).distance(Vec3::NEG_Y) < 0.0001);
    }
    #[test]
    fn map_time_uses_minutes_and_rejects_invalid_or_ambiguous_values() {
        assert!(
            (time(&config_lines(b"GAME_STATE_TIME_OF_DAY 12 4").unwrap())
                .unwrap()
                .unwrap()
                - (12.0 + 4.0 / 60.0))
                .abs()
                < 0.0001
        );
        assert!(time(&[]).unwrap().is_none());
        for data in [
            "GAME_STATE_TIME_OF_DAY 24 0",
            "GAME_STATE_TIME_OF_DAY 3 60",
            "GAME_STATE_TIME_OF_DAY NaN 0",
            "GAME_STATE_TIME_OF_DAY 0 0\nGAME_STATE_TIME_OF_DAY 1 0",
        ] {
            assert!(time(&config_lines(data.as_bytes()).unwrap()).is_err());
        }
    }
    #[test]
    fn color_tables_require_unique_complete_finite_samples() {
        let data = ["X", "Y", "Z", "R", "G", "B"]
            .map(|axis| {
                let value = if axis == "Z" { "-1" } else { "0" };
                format!("TOD_DATA_SUN_{axis} {}", [value; 24].join(" "))
            })
            .join("\n");
        let lines = config_lines(data.as_bytes()).unwrap();
        assert!(Tables::read(&lines).is_ok());
        let mut duplicate = lines.clone();
        duplicate.push(lines[0].clone());
        assert!(Tables::read(&duplicate).is_err());
        for replacement in ["NaN", "-1", "2"] {
            let mut invalid = lines.clone();
            invalid[3].words[1] = replacement.into();
            assert!(Tables::read(&invalid).is_err());
        }
        let mut short = lines;
        short[0].words.pop();
        assert!(Tables::read(&short).is_err());
        let mut zero = tables();
        zero.direction = [[0.0; 24]; 3];
        assert!(zero.sample(12.0).is_err());
    }
}
