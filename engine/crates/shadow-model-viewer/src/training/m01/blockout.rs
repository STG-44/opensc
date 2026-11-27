//! Authored waterfront and headquarters for the M01 playtest.
use crate::{environment, obstacles, player, training::Preview};
use bevy::{
    asset::RenderAssetUsages, mesh::Indices, prelude::*, render::render_resource::PrimitiveTopology,
};
use serde::Deserialize;
use shadow_formats::TerrainGrid;
use std::{error::Error, fs::File, io::Read, path::Path};

use crate::map_scene::WORLD_SCALE;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Layout {
    version: u8,
    arrival: [f32; 2],
    water_height: f32,
    road_width: f32,
    road: Vec<[f32; 2]>,
    headquarters: Headquarters,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Headquarters {
    centre: [f32; 2],
    size: [f32; 2],
    wall_height: f32,
    door_width: f32,
}

#[derive(Resource)]
pub(crate) struct Blockout {
    pub arrival: Vec2,
    centre: Vec3,
    size: Vec2,
    wall_height: f32,
    door_width: f32,
    water: f32,
    road_width: f32,
    road: Vec<Vec2>,
    walls: Vec<Wall>,
    triangles: Vec<[Vec3; 3]>,
}

const WALL_DEPTH: f32 = 0.28;
const DOOR_HEIGHT: f32 = 2.3;

struct Wall {
    centre: Vec3,
    size: Vec3,
}
impl Wall {
    fn vertical_faces(&self) -> [[Vec3; 3]; 8] {
        let low = self.centre - self.size * 0.5;
        let high = self.centre + self.size * 0.5;
        let corners = [
            Vec3::new(low.x, low.y, low.z),
            Vec3::new(high.x, low.y, low.z),
            Vec3::new(high.x, low.y, high.z),
            Vec3::new(low.x, low.y, high.z),
        ];
        std::array::from_fn(|index| {
            let a = corners[index / 2];
            let b = corners[(index / 2 + 1) % 4];
            let up = Vec3::Y * self.size.y;
            if index % 2 == 0 {
                [a, b, b + up]
            } else {
                [a, b + up, a + up]
            }
        })
    }
}

fn ground(grid: &TerrainGrid, point: Vec2) -> Option<Vec3> {
    let position = Vec3::new(point.x, 0., -point.y);
    player::surface(grid, position).map(|height| position.with_y(height))
}

impl Blockout {
    pub fn load(path: &Path, grid: &mut TerrainGrid) -> Result<Self, Box<dyn Error>> {
        let mut bytes = Vec::new();
        File::open(path)?.take(16_385).read_to_end(&mut bytes)?;
        if bytes.len() > 16_384 {
            return Err("M01 layout exceeds 16 KiB".into());
        }
        Self::prepare(serde_json::from_slice(&bytes)?, grid)
    }

    fn prepare(layout: Layout, grid: &mut TerrainGrid) -> Result<Self, Box<dyn Error>> {
        let Headquarters {
            centre,
            size,
            wall_height,
            door_width,
        } = layout.headquarters;
        let centre = Vec2::from_array(centre);
        let size = Vec2::from_array(size);
        let arrival = Vec2::from_array(layout.arrival);
        let extent = Vec2::from_array(grid.extent()) * WORLD_SCALE;
        let road = layout
            .road
            .into_iter()
            .map(Vec2::from_array)
            .collect::<Vec<_>>();
        let within = |point: Vec2, margin: Vec2| {
            point.cmpge(margin).all() && point.cmple(extent - margin).all()
        };
        if layout.version != 1
            || !(-10_000.0..=10_000.0).contains(&layout.water_height)
            || !(1.0..=8.0).contains(&layout.road_width)
            || !(2..=32).contains(&road.len())
            || !road
                .iter()
                .all(|&p| within(p, Vec2::splat(layout.road_width)))
            || road.windows(2).any(|p| p[0].distance(p[1]) < 1.0)
            || !within(arrival, Vec2::ZERO)
            || !(8.0..=24.0).contains(&size.x)
            || !(6.0..=16.0).contains(&size.y)
            || !(2.6..=5.0).contains(&wall_height)
            || !(1.2..=size.y - 1.0).contains(&door_width)
            || !within(centre, size * 0.5 + Vec2::splat(9.))
            || (arrival - centre)
                .abs()
                .cmple(size * 0.5 + Vec2::splat(0.3))
                .all()
        {
            return Err("invalid M01 layout dimensions, positions or version".into());
        }
        let floor = ground(grid, centre).ok_or("headquarters lies outside terrain")?;
        if floor.y < layout.water_height + 0.45 {
            return Err("M01 headquarters must be above the waterline".into());
        }
        if road.windows(2).map(|p| p[0].distance(p[1])).sum::<f32>() > 2_000.0 {
            return Err("M01 approach exceeds the blockout road budget".into());
        }
        // The pad changes the same samples used by rendering, picks and walking.
        let step = grid.config.edge * WORLD_SCALE;
        let pad = size * 0.5 + Vec2::splat(step * 1.5);
        for (index, value) in grid.heights.iter_mut().enumerate() {
            let position = Vec2::new(
                (index % grid.config.width) as f32,
                (index / grid.config.width) as f32,
            ) * step;
            let distance = ((position - centre).abs() - pad).max(Vec2::ZERO).length();
            let weight = 1.0 - environment::smooth(0., 8., distance);
            *value = value.lerp(floor.y / WORLD_SCALE, weight);
        }
        if ground(grid, arrival).is_none_or(|p| p.y <= layout.water_height)
            || road
                .iter()
                .any(|&p| ground(grid, p).is_none_or(|p| p.y <= layout.water_height))
        {
            return Err("M01 arrival and road points must be on land".into());
        }
        let hx = size.x / 2.;
        let hz = size.y / 2.;
        let wall = |x, z, width, depth, bottom, height| Wall {
            centre: floor + Vec3::new(x, bottom + height / 2., z),
            size: Vec3::new(width, height, depth),
        };
        let wall_base = -0.22;
        let full_height = wall_height - wall_base;
        let door_side = (size.y - door_width) / 2.;
        let walls = vec![
            wall(-hx, 0., WALL_DEPTH, size.y, wall_base, full_height),
            wall(
                0.,
                -hz,
                size.x + WALL_DEPTH,
                WALL_DEPTH,
                wall_base,
                full_height,
            ),
            wall(
                0.,
                hz,
                size.x + WALL_DEPTH,
                WALL_DEPTH,
                wall_base,
                full_height,
            ),
            wall(
                hx,
                -(hz + door_width / 2.) / 2.,
                WALL_DEPTH,
                door_side,
                wall_base,
                full_height,
            ),
            wall(
                hx,
                (hz + door_width / 2.) / 2.,
                WALL_DEPTH,
                door_side,
                wall_base,
                full_height,
            ),
            wall(
                hx,
                0.,
                WALL_DEPTH,
                door_width,
                DOOR_HEIGHT,
                wall_height - DOOR_HEIGHT,
            ),
        ];
        let triangles = walls.iter().flat_map(Wall::vertical_faces).collect();
        Ok(Self {
            arrival,
            centre: floor,
            size,
            wall_height,
            door_width,
            water: layout.water_height,
            road_width: layout.road_width,
            road,
            walls,
            triangles,
        })
    }

    pub fn report(&self) -> serde_json::Value {
        serde_json::json!({
            "arrival_east_north_metres": self.arrival.to_array(),
            "road_east_north_metres": self.road.iter().map(|p| p.to_array()).collect::<Vec<_>>(),
            "water_height_metres": self.water,
            "headquarters": {
                "centre_metres": self.centre.to_array(),
                "size_metres": self.size.to_array(),
                "door_east_metres": self.centre.x + self.size.x / 2.,
                "door_width_metres": self.door_width,
            }
        })
    }

    pub(crate) fn free(&self, point: Vec3, height: f32) -> bool {
        point.y > self.water && !obstacles::occupied(point, height, self.triangles.iter().copied())
    }

    pub(crate) fn blocks_step(
        &self,
        from: Vec3,
        to: Vec3,
        height: f32,
        grid: &TerrainGrid,
    ) -> bool {
        obstacles::sweep(from, to, height, self.triangles.iter().copied()).is_some()
            || self.last_dry(from, to, grid).is_some()
    }

    pub fn guard(&self, actor: &mut player::Player, previous: Vec3, grid: &TerrainGrid) {
        if previous == actor.position {
            return;
        }
        obstacles::guard(actor, previous, grid, self.triangles.iter().copied());
        if let Some(dry) = self.last_dry(previous, actor.position, grid) {
            actor.position = dry;
            actor.destination = None;
            actor.route.clear();
            actor.playback.select(&actor.clips, actor.idle_clip());
        }
    }

    fn last_dry(&self, from: Vec3, to: Vec3, grid: &TerrainGrid) -> Option<Vec3> {
        let steps = (from.xz().distance(to.xz()) / 0.24).ceil() as usize;
        let mut dry = from;
        for i in 1..=steps {
            let point = from.lerp(to, i as f32 / steps as f32);
            let Some(height) = player::surface(grid, point) else {
                break;
            };
            if height <= self.water {
                return Some(dry);
            }
            dry = point.with_y(height);
        }
        None
    }

    pub fn roof_triangles(&self) -> [[Vec3; 3]; 6] {
        let x = self.size.x / 2. + 0.38;
        let z = self.size.y / 2. + 0.38;
        let y = self.wall_height + 0.05;
        let a = self.centre + Vec3::new(-x, y, -z);
        let b = self.centre + Vec3::new(x, y, -z);
        let c = self.centre + Vec3::new(x, y, z);
        let d = self.centre + Vec3::new(-x, y, z);
        let e = self.centre + Vec3::new(-x, y + 1.55, 0.);
        let f = self.centre + Vec3::new(x, y + 1.55, 0.);
        [
            [a, e, f],
            [a, f, b],
            [e, d, c],
            [e, c, f],
            [a, d, e],
            [b, f, c],
        ]
    }

    fn road_mesh(&self, grid: &TerrainGrid) -> Mesh {
        let mut points = Vec::new();
        let mut indices = Vec::new();
        for (segment, pair) in self.road.windows(2).enumerate() {
            let steps = (pair[0].distance(pair[1]) / 0.6).ceil() as usize;
            let across = |i: usize| {
                let before = self.road[i.saturating_sub(1)];
                let after = self.road[(i + 1).min(self.road.len() - 1)];
                (after - before).perp().normalize_or_zero()
            };
            for i in 0..steps {
                let t = i as f32 / steps as f32;
                let p = pair[0].lerp(pair[1], t);
                let normal = across(segment).lerp(across(segment + 1), t);
                append_road_row(grid, &mut points, p, normal * self.road_width / 2.);
            }
        }
        let end = self.road.len() - 1;
        let normal = (self.road[end] - self.road[end - 1]).perp().normalize();
        append_road_row(
            grid,
            &mut points,
            self.road[end],
            normal * self.road_width / 2.,
        );
        for row in 0..points.len() / 5 - 1 {
            for col in 0..4 {
                let i = (row * 5 + col) as u32;
                indices.extend([i, i + 6, i + 1, i, i + 5, i + 6]);
            }
        }
        make_mesh(points, indices)
    }
}

fn append_road_row(grid: &TerrainGrid, points: &mut Vec<[f32; 3]>, centre: Vec2, across: Vec2) {
    for column in -2..=2 {
        let sample = centre + across * (column as f32 * 0.5);
        let pos = ground(grid, sample).expect("validated road stays inside the playable area");
        points.push((pos + Vec3::Y * 0.085).to_array());
    }
}

fn make_mesh(points: Vec<[f32; 3]>, indices: Vec<u32>) -> Mesh {
    let normals = crate::normals::reconstructed_normals(&points, &indices);
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, points)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_indices(Indices::U32(indices))
}

#[derive(Component)]
pub(super) struct Roof;

pub(super) fn spawn(
    mut commands: Commands,
    site: Res<Blockout>,
    preview: Res<Preview>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let grid = preview
        .grid
        .as_ref()
        .expect("terrain startup precedes blockout");
    let [x, z] = grid.extent().map(|n| n * WORLD_SCALE);
    let water = materials.add(StandardMaterial {
        base_color: Color::srgb(0.052, 0.18, 0.22),
        perceptual_roughness: 0.32,
        reflectance: 0.26,
        ..default()
    });
    commands.spawn((
        Name::new("M01 reservoir"),
        Mesh3d(meshes.add(Plane3d::default().mesh().size(x, z))),
        MeshMaterial3d(water),
        Transform::from_xyz(x / 2., site.water, -z / 2.),
    ));
    let mut material = |color| {
        materials.add(StandardMaterial {
            base_color: color,
            perceptual_roughness: 0.91,
            cull_mode: None,
            ..default()
        })
    };
    commands.spawn((
        Name::new("Waldbrück approach"),
        environment::ExposedSurface,
        Mesh3d(meshes.add(site.road_mesh(grid))),
        MeshMaterial3d(material(Color::srgb(0.29, 0.26, 0.208))),
    ));
    let plaster = material(Color::srgb(0.71, 0.63, 0.47));
    for wall in &site.walls {
        commands.spawn((
            Name::new("Headquarters wall"),
            environment::ExposedSurface,
            Mesh3d(meshes.add(Cuboid::from_size(wall.size))),
            MeshMaterial3d(plaster.clone()),
            Transform::from_translation(wall.centre),
        ));
    }
    let floor = material(Color::srgb(0.33, 0.21, 0.13));
    commands.spawn((
        Name::new("Headquarters floor"),
        Mesh3d(meshes.add(Cuboid::new(site.size.x, 0.07, site.size.y))),
        MeshMaterial3d(floor),
        Transform::from_translation(site.centre - Vec3::Y * 0.024),
    ));
    let roof = site
        .roof_triangles()
        .iter()
        .flatten()
        .map(|p| p.to_array())
        .collect::<Vec<_>>();
    let indices = (0..roof.len() as u32).collect();
    commands.spawn((
        Name::new("Headquarters roof"),
        Roof,
        environment::ExposedSurface,
        Mesh3d(meshes.add(make_mesh(roof, indices))),
        MeshMaterial3d(material(Color::srgb(0.41, 0.16, 0.105))),
    ));
    let tint = material(Color::srgb(0.12, 0.21, 0.23));
    for x in [-3.6, 0., 3.6] {
        commands.spawn((
            Name::new("Headquarters south window"),
            Mesh3d(meshes.add(Cuboid::new(1.15, 0.92, 0.035))),
            MeshMaterial3d(tint.clone()),
            Transform::from_translation(
                site.centre + Vec3::new(x, 1.55, (site.size.y + WALL_DEPTH) / 2.),
            ),
        ));
    }
}

pub(super) fn cutaway(
    keys: Res<ButtonInput<KeyCode>>,
    squad: Res<super::Squad>,
    site: Res<Blockout>,
    mut manual: Local<bool>,
    mut roof: Single<&mut Visibility, With<Roof>>,
) {
    if keys.just_pressed(KeyCode::KeyR) {
        *manual = !*manual;
    }
    let inside = squad.actors().any(|actor| {
        (actor.position - site.centre)
            .xz()
            .abs()
            .cmplt(site.size / 2.)
            .all()
    });
    roof.set_if_neq(if inside || *manual {
        Visibility::Hidden
    } else {
        Visibility::Visible
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use shadow_formats::TerrainConfig;

    fn site() -> (Blockout, TerrainGrid) {
        let mut grid = TerrainGrid {
            config: TerrainConfig {
                name: "test".into(),
                width: 500,
                height: 500,
                edge: 200.,
                height_step: 1.,
                water_index: 0,
            },
            heights: vec![700.; 500 * 500],
        };
        let layout =
            serde_json::from_str(include_str!("../../../../../assets/m01/blockout.json")).unwrap();
        let site = Blockout::prepare(layout, &mut grid).unwrap();
        (site, grid)
    }

    #[test]
    fn authored_door_is_passable_and_adjacent_wall_stops_a_long_step() {
        let (site, _) = site();
        let traverse = |z| {
            obstacles::sweep(
                site.centre + Vec3::new(12., 0., z),
                site.centre + Vec3::new(0., 0., z),
                1.75,
                site.triangles.iter().copied(),
            )
        };
        assert!(traverse(0.).is_none());
        assert!(traverse(2.).is_some_and(|t| (0.4..0.55).contains(&t)));
    }

    #[test]
    fn shore_stops_a_step_before_the_sunken_surface() {
        let (site, mut grid) = site();
        for row in grid.heights.as_chunks_mut::<500>().0.iter_mut().skip(11) {
            row.fill(200.);
        }
        let from = Vec3::new(20., 7., -19.);
        let to = Vec3::new(20., 2., -24.);
        let dry = site.last_dry(from, to, &grid).unwrap();
        assert!(dry.z > -22.);
        assert!(player::surface(&grid, dry).unwrap() > site.water);
    }
}
