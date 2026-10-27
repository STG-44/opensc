//! Попадание луча в реальные треугольники рельефа и двойной щелчок.
use crate::map_scene::WORLD_SCALE;
use bevy::prelude::*;
use shadow_formats::TerrainGrid;

#[derive(Resource, Default)]
pub struct ClickHistory(pub Option<(f64, Vec2)>);
impl ClickHistory {
    pub fn register(&mut self, seconds: f64, cursor: Vec2) -> bool {
        let double = self.0.is_some_and(|(previous, point)| {
            (0.0..=0.4).contains(&(seconds - previous)) && cursor.distance(point) <= 8.0
        });
        self.0 = if double {
            None
        } else {
            Some((seconds, cursor))
        };
        double
    }
}
pub fn terrain_hit(grid: &TerrainGrid, origin: Vec3, direction: Vec3) -> Option<Vec3> {
    if !origin.is_finite() || !direction.is_finite() || direction.length_squared() < 1e-12 {
        return None;
    }
    let c = &grid.config;
    let point = |x: usize, y: usize| {
        Vec3::new(
            x as f32 * c.edge,
            grid.heights[y * c.width + x],
            -(y as f32) * c.edge,
        ) * WORLD_SCALE
    };
    let mut nearest = f32::INFINITY;
    for y in 0..c.height - 1 {
        for x in 0..c.width - 1 {
            let a = point(x, y);
            let b = point(x + 1, y);
            let d = point(x + 1, y + 1);
            let e = point(x, y + 1);
            for (a, b, c) in [(a, b, d), (a, d, e)] {
                let e1 = b - a;
                let e2 = c - a;
                let p = direction.cross(e2);
                let det = e1.dot(p);
                if det.abs() < 1e-8 {
                    continue;
                }
                let inv = det.recip();
                let t = origin - a;
                let u = t.dot(p) * inv;
                if !(0.0..=1.0).contains(&u) {
                    continue;
                }
                let q = t.cross(e1);
                let v = direction.dot(q) * inv;
                if v < 0.0 || u + v > 1.0 {
                    continue;
                }
                let distance = e2.dot(q) * inv;
                if distance >= 0.0 {
                    nearest = nearest.min(distance);
                }
            }
        }
    }
    nearest.is_finite().then(|| origin + direction * nearest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shadow_formats::TerrainConfig;
    #[test]
    fn ray_hits_both_sloped_triangles_and_rejects_misses_and_invalid_rays() {
        let grid = TerrainGrid {
            config: TerrainConfig {
                name: "test".into(),
                width: 2,
                height: 2,
                edge: 100.0,
                height_step: 1.0,
                water_index: 0,
            },
            heights: vec![0.0, 0.0, 0.0, 100.0],
        };
        for (x, z) in [
            (0.75, -0.25),
            (0.25, -0.75),
            (0.5, -0.5),
            (0.0, 0.0),
            (1.0, -1.0),
        ] {
            let hit = terrain_hit(&grid, Vec3::new(x, 10.0, z), -Vec3::Y).unwrap();
            assert!((hit.y - grid.height_at(x / 0.01, -z / 0.01).unwrap() * 0.01).abs() < 1e-5);
        }
        assert!(terrain_hit(&grid, Vec3::new(0.5, 0.0, -0.5), Vec3::Y).is_some());
        assert!(terrain_hit(&grid, Vec3::new(0.5, 10.0, -0.5), Vec3::Y).is_none());
        assert!(terrain_hit(&grid, Vec3::new(2.0, 10.0, 0.0), -Vec3::Y).is_none());
        assert!(terrain_hit(&grid, Vec3::splat(f32::NAN), Vec3::Y).is_none());
        assert!(terrain_hit(&grid, Vec3::ZERO, Vec3::ZERO).is_none());
    }
    #[test]
    fn double_click_requires_time_and_position_and_does_not_chain_three_clicks() {
        let mut history = ClickHistory::default();
        assert!(!history.register(1.0, Vec2::ZERO));
        assert!(history.register(1.3, Vec2::new(2.0, 2.0)));
        assert!(!history.register(1.4, Vec2::ZERO));
        assert!(!history.register(2.0, Vec2::ZERO));
        assert!(!history.register(2.1, Vec2::new(30.0, 0.0)));
    }
}
