//! Проверенный поднабор terrain_mapping.txt и треугольная сетка высот.
use crate::{Error, PcxImage, Result, config_lines};
use std::collections::BTreeMap;
#[derive(Debug)]
pub struct TerrainConfig {
    pub name: String,
    pub width: usize,
    pub height: usize,
    pub edge: f32,
    pub height_step: f32,
    pub water_index: u8,
}
impl TerrainConfig {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let mut values = BTreeMap::new();
        let keys = [
            "texture_map_base_name",
            "map_dx",
            "map_dy",
            "nominal_edge_size",
            "altitude_map_height_base",
            "min_alt_grad",
            "max_alt_grad",
            "water_level",
        ];
        for line in config_lines(bytes)? {
            let w = line.words;
            if w[0] == "SET"
                && w.get(1).is_some_and(|k| keys.contains(&k.as_str()))
                && (w.len() != 3 || values.insert(w[1].clone(), w[2].clone()).is_some())
            {
                return Err(Error::Invalid(format!(
                    "повтор или неверный SET: строка {}",
                    line.number
                )));
            }
        }
        let field = |key| {
            values
                .get(key)
                .map(String::as_str)
                .ok_or_else(|| Error::Invalid(format!("terrain: отсутствует {key}")))
        };
        let number = |key| {
            field(key)?
                .parse::<f32>()
                .ok()
                .filter(|v| v.is_finite())
                .ok_or_else(|| Error::Invalid(format!("terrain: неверное {key}")))
        };
        let size = |key| {
            field(key)?
                .parse::<usize>()
                .map_err(|_| Error::Invalid(format!("terrain: неверное {key}")))
        };
        let name = field("texture_map_base_name")?.to_lowercase();
        if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
            return Err(Error::Invalid("terrain: недопустимое имя".into()));
        }
        let width = size("map_dx")?;
        let height = size("map_dy")?;
        let edge = number("nominal_edge_size")?;
        let height_step = number("altitude_map_height_base")?;
        if !(2..=2048).contains(&width)
            || !(2..=2048).contains(&height)
            || width * height > 1_048_576
            || edge <= 0.0
            || edge > 1_000_000.0
            || height_step <= 0.0
            || height_step > 1_000_000.0
        {
            return Err(Error::Invalid(
                "terrain: размеры или масштаб вне допустимых границ".into(),
            ));
        }
        if size("min_alt_grad")? != 1 || size("max_alt_grad")? != 1 {
            return Err(Error::Invalid(
                "terrain: случайная таблица высот пока не поддерживается".into(),
            ));
        }
        let water_index = field("water_level")?
            .parse::<u8>()
            .map_err(|_| Error::Invalid("terrain: неверный water_level".into()))?;
        Ok(Self {
            name,
            width,
            height,
            edge,
            height_step,
            water_index,
        })
    }
}
#[derive(Debug)]
pub struct TerrainGrid {
    pub config: TerrainConfig,
    pub heights: Vec<f32>,
}
impl TerrainGrid {
    pub fn new(config: TerrainConfig, image: PcxImage) -> Result<Self> {
        if (config.width, config.height) != (image.width, image.height)
            || image.width.checked_mul(image.height) != Some(image.pixels.len())
        {
            return Err(Error::Invalid(
                "размеры PCX и terrain_mapping.txt различаются".into(),
            ));
        }
        // Светлый индекс PCX означает низкий рельеф: 255 -> 0, 134 -> 1210 при step=10.
        let heights = image
            .pixels
            .iter()
            .map(|&p| f32::from(255 - p) * config.height_step)
            .collect();
        Ok(Self { config, heights })
    }
    /// Диагональ ячейки (0,0)->(1,1). Та же интерполяция, что у mesh просмотрщика.
    pub fn height_at(&self, x: f32, y: f32) -> Option<f32> {
        let c = &self.config;
        let u = x / c.edge;
        let v = y / c.edge;
        if !u.is_finite()
            || !v.is_finite()
            || u < 0.0
            || v < 0.0
            || u > (c.width - 1) as f32
            || v > (c.height - 1) as f32
        {
            return None;
        }
        let a = (u as usize).min(c.width - 2);
        let b = (v as usize).min(c.height - 2);
        let du = u - a as f32;
        let dv = v - b as f32;
        let h00 = self.heights[b * c.width + a];
        let h10 = self.heights[b * c.width + a + 1];
        let h01 = self.heights[(b + 1) * c.width + a];
        let h11 = self.heights[(b + 1) * c.width + a + 1];
        Some(if du >= dv {
            h00 + (h10 - h00) * du + (h11 - h10) * dv
        } else {
            h00 + (h11 - h01) * du + (h01 - h00) * dv
        })
    }
}
