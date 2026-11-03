//! Continuous character collision against the visible world geometry.
use bevy::prelude::*;
use shadow_formats::TerrainGrid;

fn flat(p: Vec3) -> Vec2 {
    Vec2::new(p.x, p.z)
}
fn clip_y(points: &[Vec3], y: f32, above: bool) -> Vec<Vec3> {
    let mut out = Vec::new();
    for i in 0..points.len() {
        let a = points[i];
        let b = points[(i + 1) % points.len()];
        let inside = |p: Vec3| if above { p.y >= y } else { p.y <= y };
        if inside(a) {
            out.push(a);
        }
        if inside(a) != inside(b) {
            out.push(a.lerp(b, (y - a.y) / (b.y - a.y)));
        }
    }
    out
}
fn distance_edge(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let edge = b - a;
    p.distance(a + edge * ((p - a).dot(edge) / edge.length_squared().max(1e-12)).clamp(0.0, 1.0))
}
fn inside(p: Vec2, poly: &[Vec2]) -> bool {
    let mut positive = false;
    let mut negative = false;
    for i in 0..poly.len() {
        let cross = (poly[(i + 1) % poly.len()] - poly[i]).perp_dot(p - poly[i]);
        positive |= cross > 1e-7;
        negative |= cross < -1e-7;
    }
    positive != negative
}
fn polygon_hit(from: Vec2, to: Vec2, poly: &[Vec2], radius: f32) -> Option<f32> {
    if poly.is_empty() {
        return None;
    }
    // Permit an actor placed inside scenery to move out, rather than trapping it.
    if inside(from, poly)
        || (0..poly.len())
            .any(|i| distance_edge(from, poly[i], poly[(i + 1) % poly.len()]) < radius - 1e-5)
    {
        return None;
    }
    let delta = to - from;
    let speed = delta.length_squared();
    if speed < 1e-12 {
        return None;
    }
    let mut hit = 1.1f32;
    for i in 0..poly.len() {
        let a = poly[i];
        let b = poly[(i + 1) % poly.len()];
        let edge = b - a;
        let length = edge.length();
        if length > 1e-6 {
            let normal = Vec2::new(-edge.y, edge.x) / length;
            let velocity = delta.dot(normal);
            if velocity.abs() > 1e-7 {
                for side in [-radius, radius] {
                    let t = (side - (from - a).dot(normal)) / velocity;
                    let along = (from + delta * t - a).dot(edge) / (length * length);
                    if (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&along) {
                        hit = hit.min(t);
                    }
                }
            }
        }
        let offset = from - a;
        let half_b = offset.dot(delta);
        let c = offset.length_squared() - radius * radius;
        let discriminant = half_b * half_b - speed * c;
        if discriminant >= 0.0 {
            let t = (-half_b - discriminant.sqrt()) / speed;
            if (0.0..=1.0).contains(&t) {
                hit = hit.min(t);
            }
        }
    }
    (hit <= 1.0).then_some(hit)
}
pub fn sweep(
    from: Vec3,
    to: Vec3,
    height: f32,
    triangles: impl Iterator<Item = [Vec3; 3]>,
) -> Option<f32> {
    if !from.is_finite() || !to.is_finite() || flat(to - from).length_squared() < 1e-10 {
        return None;
    }
    let radius = 0.20;
    let low = from.min(to) - Vec3::new(radius, -0.12, radius);
    let high = from.max(to) + Vec3::new(radius, height, radius);
    triangles
        .filter_map(|triangle| {
            let min = triangle[0].min(triangle[1]).min(triangle[2]);
            let max = triangle[0].max(triangle[1]).max(triangle[2]);
            if min.cmpgt(high).any() || max.cmplt(low).any() {
                return None;
            }
            let points = clip_y(&clip_y(&triangle, low.y, true), high.y, false);
            let polygon = points.into_iter().map(flat).collect::<Vec<_>>();
            polygon_hit(flat(from), flat(to), &polygon, radius)
        })
        .min_by(f32::total_cmp)
}
pub(crate) fn occupied(
    point: Vec3,
    height: f32,
    mut triangles: impl Iterator<Item = [Vec3; 3]>,
) -> bool {
    triangles.any(|triangle| {
        let points = clip_y(
            &clip_y(&triangle, point.y + 0.12, true),
            point.y + height,
            false,
        );
        let poly: Vec<_> = points.into_iter().map(flat).collect();
        !poly.is_empty()
            && (inside(flat(point), &poly)
                || (0..poly.len()).any(|i| {
                    distance_edge(flat(point), poly[i], poly[(i + 1) % poly.len()]) < 0.20
                }))
    })
}
pub fn guard(
    actor: &mut crate::player::Player,
    previous: Vec3,
    grid: &TerrainGrid,
    triangles: impl Iterator<Item = [Vec3; 3]>,
) {
    let height = actor.body_height() * [1.0, 0.65, 0.25][actor.posture.index()];
    if let Some(t) = sweep(previous, actor.position, height, triangles) {
        let distance = flat(actor.position - previous).length();
        actor.position = previous.lerp(actor.position, (t - 0.002 / distance).max(0.0));
        actor.position.y = crate::player::surface(grid, actor.position).unwrap_or(previous.y);
        actor.destination = None;
        actor.playback.select(&actor.clips, actor.idle_clip());
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn table(y: f32) -> [[Vec3; 3]; 2] {
        let [a, b, c, d] = [
            Vec3::new(-1.0, y, -1.0),
            Vec3::new(1.0, y, -1.0),
            Vec3::new(1.0, y, 1.0),
            Vec3::new(-1.0, y, 1.0),
        ];
        [[a, b, c], [a, c, d]]
    }
    #[test]
    fn tabletop_blocks_chest_in_both_directions_including_long_steps() {
        for (from, to) in [
            (Vec3::new(-4.0, 0.0, 0.0), Vec3::new(4.0, 0.0, 0.0)),
            (Vec3::new(4.0, 0.0, 0.0), Vec3::new(-4.0, 0.0, 0.0)),
        ] {
            let t = sweep(from, to, 1.7, table(0.8).into_iter()).unwrap();
            assert!((t - 0.35).abs() < 0.0001);
        }
    }
    #[test]
    fn floor_low_debris_and_overhead_geometry_do_not_block() {
        let from = Vec3::new(-4.0, 0.0, 0.0);
        let to = Vec3::new(4.0, 0.0, 0.0);
        for y in [0.0, 0.08, 2.5] {
            assert!(sweep(from, to, 1.7, table(y).into_iter()).is_none());
        }
        assert!(sweep(from, to, 0.4, table(0.8).into_iter()).is_none());
    }
    #[test]
    fn narrow_table_leg_and_radius_near_miss_are_distinguished() {
        let leg = [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(0.0, 1.0, 0.04),
        ];
        assert!(
            sweep(
                Vec3::new(-2.0, 0.0, 0.19),
                Vec3::new(2.0, 0.0, 0.19),
                1.7,
                [leg].into_iter()
            )
            .is_some()
        );
        assert!(
            sweep(
                Vec3::new(-2.0, 0.0, 0.3),
                Vec3::new(2.0, 0.0, 0.3),
                1.7,
                [leg].into_iter()
            )
            .is_none()
        );
    }
    #[test]
    fn tilted_overhead_face_is_clipped_to_character_height() {
        let ramp = [
            Vec3::new(-2.0, 0.0, -2.0),
            Vec3::new(2.0, 4.0, -2.0),
            Vec3::new(2.0, 4.0, 2.0),
        ];
        assert!(
            sweep(
                Vec3::new(1.5, 0.0, -3.0),
                Vec3::new(1.5, 0.0, 3.0),
                1.7,
                [ramp].into_iter()
            )
            .is_none()
        );
    }
}
