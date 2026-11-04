//! Ground A* with continuous clearance checks and conservative path smoothing.
use bevy::prelude::*;
use shadow_formats::TerrainGrid;
use std::{
    cmp::Reverse,
    collections::{BinaryHeap, HashMap, VecDeque},
};

#[derive(Default)]
pub(crate) struct Route {
    goal: Option<Vec3>,
    points: VecDeque<Vec3>,
    revision: u64,
    height: f32,
    pub failed: bool,
}
impl Route {
    pub fn clear(&mut self) {
        *self = Self::default();
    }
    pub fn next(&mut self, position: Vec3, destination: Option<Vec3>) -> Option<Vec3> {
        if self.goal != destination {
            self.points.clear();
        }
        while self
            .points
            .front()
            .is_some_and(|p| Vec2::new(p.x - position.x, p.z - position.z).length() < 0.025)
        {
            self.points.pop_front();
        }
        self.points.front().copied().or(destination)
    }
    pub fn points(&self) -> impl Iterator<Item = Vec3> + '_ {
        self.points.iter().copied()
    }
}

pub(crate) struct World<'a> {
    pub grid: &'a TerrainGrid,
    pub geometry: &'a crate::pickup::Pickup,
    pub fence: Option<&'a crate::fence::Fence>,
    pub gate: Option<&'a crate::gate::Gate>,
    pub height: f32,
}
impl World<'_> {
    fn ignored(&self, id: [i32; 2]) -> bool {
        self.fence.is_some_and(|f| f.handles_collision(id)) || self.gate.is_some_and(|g| g.id == id)
    }
    pub fn clear(&self, from: Vec3, to: Vec3) -> bool {
        let length = Vec2::new(to.x - from.x, to.z - from.z).length();
        if !from.is_finite() || !to.is_finite() {
            return false;
        }
        // Follow the terrain between endpoints instead of drawing a chord through a hill.
        let steps = (length / 0.4).ceil().max(1.0) as usize;
        let mut a = from;
        for i in 1..=steps {
            let b = from.lerp(to, i as f32 / steps as f32);
            let Some(y) = crate::player::surface(self.grid, b) else {
                return false;
            };
            let b = b.with_y(y);
            let run = Vec2::new(b.x - a.x, b.z - a.z).length();
            if (b.y - a.y).abs() > run * 1.0 + 0.02
                || self
                    .geometry
                    .movement_hit(a, b, self.height, |id| self.ignored(id))
                    .is_some()
                || self.fence.is_some_and(|f| f.movement_hit(a, b).is_some())
                || self.gate.is_some_and(|g| g.movement_hit(a, b).is_some())
            {
                return false;
            }
            a = b;
        }
        true
    }
    pub fn free(&self, p: Vec3) -> bool {
        !self
            .geometry
            .occupied(p, self.height, |id| self.ignored(id))
            && self.fence.is_none_or(|f| {
                f.movement_hit(p - Vec3::X * 0.001, p + Vec3::X * 0.001)
                    .is_none()
            })
            && self.gate.is_none_or(|g| {
                g.movement_hit(p - Vec3::X * 0.001, p + Vec3::X * 0.001)
                    .is_none()
            })
    }
}

pub(crate) fn prepare(actor: &mut crate::player::Player, world: &World<'_>) {
    let Some(goal) = actor.destination else {
        actor.route.points.clear();
        return;
    };
    let revision = world
        .geometry
        .revision()
        .wrapping_add(u64::from(world.fence.is_some_and(|f| f.cut)))
        .wrapping_add(
            world
                .gate
                .map_or(0, |g| u64::from(g.angle.to_bits()).rotate_left(32)),
        );
    let changed = actor.route.goal != Some(goal) || actor.route.height != world.height;
    let obstructed = actor.route.revision != revision
        && !{
            let mut from = actor.position;
            actor.route.points.iter().copied().chain([goal]).all(|p| {
                let clear = world.clear(from, p);
                from = p;
                clear
            })
        };
    if !changed && !obstructed {
        actor.route.revision = revision;
        return;
    }
    actor.route.goal = Some(goal);
    actor.route.height = world.height;
    actor.route.revision = revision;
    let ext = Vec2::from_array(world.grid.extent()) * crate::map_scene::WORLD_SCALE;
    let points = world
        .free(goal)
        .then(|| {
            find_path(
                actor.position,
                goal,
                ext,
                |p| crate::player::surface(world.grid, p),
                |a, b| world.clear(a, b),
            )
        })
        .flatten();
    actor.route.failed = points.is_none();
    actor.route.points = points.unwrap_or_default().into();
    if actor.route.failed {
        actor.destination = None;
        actor.playback.select(&actor.clips, actor.idle_clip());
    }
}

/// The exact endpoints remain off-grid. Every connector and simplified edge is
/// tested with the same radius/geometry as movement; diagonals cannot cut corners.
pub(crate) fn find_path(
    start: Vec3,
    goal: Vec3,
    extent: Vec2,
    ground: impl Fn(Vec3) -> Option<f32>,
    clear: impl Fn(Vec3, Vec3) -> bool,
) -> Option<Vec<Vec3>> {
    if !start.is_finite() || !goal.is_finite() || !extent.is_finite() || extent.min_element() <= 0.0
    {
        return None;
    }
    if clear(start, goal) {
        return Some(vec![goal]);
    }
    const CELL: f32 = 0.5;
    let dims = (extent / CELL).ceil().as_uvec2() + UVec2::ONE;
    let count = dims.x as usize * dims.y as usize;
    if count > 2_000_000 {
        return None;
    }
    let point = |id: usize| {
        let p = Vec3::new(
            (id as u32 % dims.x) as f32 * CELL,
            0.0,
            -((id as u32 / dims.x) as f32 * CELL),
        );
        (p.x <= extent.x && -p.z <= extent.y)
            .then(|| ground(p).map(|y| p.with_y(y)))
            .flatten()
    };
    let heuristic =
        |p: Vec3| (Vec2::new(p.x - goal.x, p.z - goal.z).length() / CELL * 1000.0).floor() as i32;
    let mut costs = vec![i32::MAX; count];
    let mut parents = vec![usize::MAX; count];
    let mut queue = BinaryHeap::new();
    let seed = IVec2::new(
        (start.x / CELL).round() as i32,
        (-start.z / CELL).round() as i32,
    );
    for y in -1..=1 {
        for x in -1..=1 {
            let p = seed + IVec2::new(x, y);
            if p.min_element() < 0 || p.x >= dims.x as i32 || p.y >= dims.y as i32 {
                continue;
            }
            let id = p.y as usize * dims.x as usize + p.x as usize;
            if let Some(p) = point(id)
                && clear(start, p)
            {
                let cost = (start.distance(p) / CELL * 1000.0).ceil() as i32;
                costs[id] = cost;
                queue.push(Reverse((cost + heuristic(p), cost, id)));
            }
        }
    }
    let mut edges = HashMap::new();
    let mut visited = 0;
    while let Some(Reverse((_, cost, id))) = queue.pop() {
        if costs[id] != cost {
            continue;
        }
        visited += 1;
        if visited > 30_000 {
            return None;
        }
        let p = point(id)?;
        if Vec2::new(p.x - goal.x, p.z - goal.z).length() <= CELL * 2.0 && clear(p, goal) {
            let mut path = vec![goal, p];
            let mut cursor = id;
            while parents[cursor] != usize::MAX {
                cursor = parents[cursor];
                path.push(point(cursor)?);
            }
            path.push(start);
            path.reverse();
            let mut result = Vec::new();
            let mut from = 0;
            while from < path.len() - 1 {
                let next = (from + 1..path.len())
                    .rev()
                    .find(|&i| clear(path[from], path[i]))?;
                result.push(path[next]);
                from = next;
            }
            return Some(result);
        }
        let cell = IVec2::new(id as i32 % dims.x as i32, id as i32 / dims.x as i32);
        for y in -1i32..=1 {
            for x in -1i32..=1 {
                if x == 0 && y == 0 {
                    continue;
                }
                let n = cell + IVec2::new(x, y);
                if n.min_element() < 0 || n.x >= dims.x as i32 || n.y >= dims.y as i32 {
                    continue;
                }
                let next = n.y as usize * dims.x as usize + n.x as usize;
                let next_cost = cost + if x != 0 && y != 0 { 1415 } else { 1000 };
                if next_cost >= costs[next] {
                    continue;
                }
                let Some(q) = point(next) else {
                    continue;
                };
                let pass = *edges.entry((id, next)).or_insert_with(|| clear(p, q));
                if pass {
                    costs[next] = next_cost;
                    parents[next] = id;
                    queue.push(Reverse((next_cost + heuristic(q), next_cost, next)));
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    fn box_clear(a: Vec3, b: Vec3, low: Vec2, high: Vec2) -> bool {
        let tri = [
            [
                Vec3::new(low.x, 0.8, -low.y),
                Vec3::new(high.x, 0.8, -low.y),
                Vec3::new(high.x, 0.8, -high.y),
            ],
            [
                Vec3::new(low.x, 0.8, -low.y),
                Vec3::new(high.x, 0.8, -high.y),
                Vec3::new(low.x, 0.8, -high.y),
            ],
        ];
        crate::obstacles::sweep(a, b, 1.7, tri.into_iter()).is_none()
    }
    #[test]
    fn detours_around_table_and_every_smoothed_edge_keeps_character_clearance() {
        let start = Vec3::new(1.1, 0.0, -5.1);
        let goal = Vec3::new(8.9, 0.0, -5.1);
        let clear = |a, b| box_clear(a, b, Vec2::new(4.0, 3.0), Vec2::new(6.0, 7.0));
        let path = find_path(start, goal, Vec2::splat(10.0), |_| Some(0.0), clear).unwrap();
        assert!(path.len() >= 3 && path.len() < 8);
        assert_eq!(*path.last().unwrap(), goal);
        let mut from = start;
        for p in path {
            assert!(clear(from, p));
            from = p;
        }
    }
    #[test]
    fn sealed_wall_is_unreachable_and_a_door_becomes_traversable_when_opened() {
        let start = Vec3::new(1.0, 0.0, -5.0);
        let goal = Vec3::new(9.0, 0.0, -5.0);
        let closed = |a, b| box_clear(a, b, Vec2::new(4.0, -1.0), Vec2::new(6.0, 11.0));
        assert!(find_path(start, goal, Vec2::splat(10.0), |_| Some(0.0), closed).is_none());
        assert_eq!(
            find_path(start, goal, Vec2::splat(10.0), |_| Some(0.0), |_, _| true).unwrap(),
            vec![goal]
        );
    }
    #[test]
    fn route_waypoints_keep_final_destination_and_new_order_replaces_old_route() {
        let a = Vec3::X;
        let b = Vec3::X * 2.0;
        let mut r = Route {
            goal: Some(b),
            points: [a, b].into(),
            ..default()
        };
        assert_eq!(r.next(Vec3::ZERO, Some(b)), Some(a));
        assert_eq!(r.next(a, Some(b)), Some(b));
        assert_eq!(r.next(a, Some(Vec3::Z)), Some(Vec3::Z));
        assert!(r.points.is_empty());
    }
}
