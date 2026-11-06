//! Reusable local precipitation pools and an exact upward-surface rain shelter.
use super::*;
use bevy::{
    asset::{RenderAssetUsages, uuid_handle},
    light::NotShadowCaster,
    mesh::{Indices, VertexAttributeValues},
    render::render_resource::{AsBindGroup, PrimitiveTopology},
    shader::ShaderRef,
};
use std::collections::HashMap;
const PARTICLE_SHADER: Handle<Shader> = uuid_handle!("22198657-b25b-4d7d-ad42-40c22eecf019");
#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub(super) struct ParticleMaterial {
    #[uniform(0)]
    settings: Vec4,
}
impl Material for ParticleMaterial {
    fn fragment_shader() -> ShaderRef {
        PARTICLE_SHADER.clone().into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }
}
pub(super) struct ParticlePlugin;
impl Plugin for ParticlePlugin {
    fn build(&self, app: &mut App) {
        app.world_mut()
            .resource_mut::<Assets<Shader>>()
            .insert(
                PARTICLE_SHADER.id(),
                Shader::from_wesl(
                    super::shader_source("particles.wesl", include_str!("particles.wesl")),
                    "opensc/environment/particles.wesl",
                ),
            )
            .expect("particle shader ID");
        app.add_plugins(MaterialPlugin::<ParticleMaterial>::default());
    }
}
struct Roof {
    triangle: [Vec3; 3],
    owner: [i32; 2],
}
#[derive(Resource, Default)]
pub(super) struct Shelter {
    roofs: Vec<Roof>,
    bins: HashMap<IVec2, Vec<usize>>,
    global: Vec<usize>,
}
impl Shelter {
    pub fn prepare(scene: &crate::map_scene::MapScene) -> Self {
        let mut result = Self::default();
        for instance in &scene.instances {
            if instance.inventory {
                continue;
            }
            let matrix = Mat4::from_scale(Vec3::splat(crate::map_scene::WORLD_SCALE))
                * instance.transform.to_matrix();
            for part in scene.models[&instance.path].interaction_parts() {
                if part.alpha_mode != AlphaMode::Opaque {
                    continue;
                }
                let Some(VertexAttributeValues::Float32x3(v)) =
                    part.mesh.attribute(Mesh::ATTRIBUTE_POSITION)
                else {
                    continue;
                };
                let Some(indices) = part.mesh.indices() else {
                    continue;
                };
                let ids = indices.iter().collect::<Vec<_>>();
                for face in ids.as_chunks::<3>().0 {
                    let t = face.map(|i| matrix.transform_point3(Vec3::from_array(v[i])));
                    let normal = (t[1] - t[0]).cross(t[2] - t[0]).normalize_or_zero();
                    if normal.y.abs() < 0.25 {
                        continue;
                    }
                    let index = result.roofs.len();
                    let low = (t[0].min(t[1]).min(t[2]) / 4.).floor().as_ivec3();
                    let high = (t[0].max(t[1]).max(t[2]) / 4.).floor().as_ivec3();
                    if i64::from(high.x - low.x + 1) * i64::from(high.z - low.z + 1) > 512 {
                        result.global.push(index);
                    } else {
                        for z in low.z..=high.z {
                            for x in low.x..=high.x {
                                result.bins.entry(IVec2::new(x, z)).or_default().push(index);
                            }
                        }
                    }
                    result.roofs.push(Roof {
                        triangle: t,
                        owner: instance.id,
                    });
                }
            }
        }
        result
    }
    fn height(&self, x: f32, z: f32, ignored: Option<[i32; 2]>) -> Option<f32> {
        let bin = (Vec2::new(x, z) / 4.).floor().as_ivec2();
        self.bins
            .get(&bin)
            .into_iter()
            .flatten()
            .chain(&self.global)
            .filter_map(|i| {
                let r = &self.roofs[*i];
                if Some(r.owner) == ignored {
                    return None;
                }
                roof_height(r.triangle, x, z)
            })
            .max_by(f32::total_cmp)
    }
}
fn roof_height([a, b, c]: [Vec3; 3], x: f32, z: f32) -> Option<f32> {
    let det = (b.x - a.x) * (c.z - a.z) - (b.z - a.z) * (c.x - a.x);
    if det.abs() < 1e-6 {
        return None;
    }
    let u = ((x - a.x) * (c.z - a.z) - (z - a.z) * (c.x - a.x)) / det;
    let v = ((b.x - a.x) * (z - a.z) - (b.z - a.z) * (x - a.x)) / det;
    (u >= 0. && v >= 0. && u + v <= 1.).then_some(a.y + u * (b.y - a.y) + v * (c.y - a.y))
}
struct Particle {
    position: Vec3,
    velocity: Vec3,
    phase: f32,
    bounced: bool,
}
struct Group {
    kind: usize,
    points: Vec<Particle>,
    mesh: Handle<Mesh>,
    entity: Entity,
    quads: Vec<Quad>,
    previous_count: usize,
}
#[derive(Clone, Copy, Default)]
struct Quad {
    positions: [[f32; 3]; 4],
    color: [f32; 4],
}
#[derive(Resource)]
pub(super) struct Particles {
    groups: Vec<Group>,
    rng: u32,
    previous: f64,
    pub active: usize,
    pub impacts: u64,
    bolt_entity: Entity,
    bolt_mesh: Handle<Mesh>,
    bolt_material: Handle<StandardMaterial>,
    generation: u64,
}
fn random(seed: &mut u32) -> f32 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 17;
    *seed ^= *seed << 5;
    *seed as f32 / u32::MAX as f32
}
fn empty_mesh(count: usize) -> Mesh {
    let mut m = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.; 3]; count * 4]);
    m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0., 1., 0.]; count * 4]);
    m.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[0.; 4]; count * 4]);
    m.insert_attribute(
        Mesh::ATTRIBUTE_UV_0,
        (0..count)
            .flat_map(|_| [[0., 0.], [1., 0.], [1., 1.], [0., 1.]])
            .collect::<Vec<_>>(),
    );
    m.insert_indices(Indices::U32(
        (0..count as u32)
            .flat_map(|i| [i * 4, i * 4 + 1, i * 4 + 2, i * 4, i * 4 + 2, i * 4 + 3])
            .collect(),
    ));
    m
}
fn spawn_group(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<ParticleMaterial>,
    kind: usize,
    count: usize,
) -> Group {
    let mesh = meshes.add(empty_mesh(count));
    let material = materials.add(ParticleMaterial {
        settings: Vec4::new(kind as f32, 1., 0., 0.),
    });
    let entity = commands
        .spawn((
            player::SceneEntity,
            Mesh3d(mesh.clone()),
            MeshMaterial3d(material.clone()),
            Transform::default(),
            NotShadowCaster,
            bevy::camera::visibility::NoFrustumCulling,
            Visibility::Hidden,
        ))
        .id();
    Group {
        kind,
        points: (0..count)
            .map(|_| Particle {
                position: Vec3::splat(f32::INFINITY),
                velocity: Vec3::ZERO,
                phase: 0.,
                bounced: false,
            })
            .collect(),
        mesh,
        entity,
        quads: vec![Quad::default(); count],
        previous_count: 0,
    }
}
fn surface(
    x: f32,
    z: f32,
    preview: &training::Preview,
    shelter: Option<&Shelter>,
    vehicle: Option<&crate::vehicle::Vehicle>,
) -> f32 {
    let ground = preview
        .grid
        .as_ref()
        .and_then(|g| player::surface(g, Vec3::new(x, 0., z)))
        .unwrap_or(preview.focus.y);
    let roof = shelter
        .and_then(|s| s.height(x, z, vehicle.map(|v| v.id)))
        .unwrap_or(ground);
    let dynamic = vehicle
        .and_then(|v| v.weather_shelter_height(x, z))
        .unwrap_or(ground);
    ground.max(roof).max(dynamic)
}
#[allow(clippy::too_many_arguments)]
pub(super) fn update(
    mut commands: Commands,
    weather: Option<Res<Weather>>,
    state: Option<ResMut<Particles>>,
    preview: Option<Res<training::Preview>>,
    shelter: Option<Res<Shelter>>,
    vehicle: Option<Res<crate::vehicle::Vehicle>>,
    cameras: Query<&Transform, With<training::MapCamera>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ParticleMaterial>>,
    mut standard: ResMut<Assets<StandardMaterial>>,
    mut visibility: Query<&mut Visibility>,
) {
    let (Some(w), Some(preview), Some(camera)) = (weather, preview, cameras.iter().next()) else {
        return;
    };
    let Some(mut state) = state else {
        let count = Quality::High.particles();
        let groups = (0..4)
            .map(|kind| spawn_group(&mut commands, &mut meshes, &mut materials, kind, count))
            .collect();
        let bolt_mesh = meshes.add(empty_mesh(1));
        let bolt_material = standard.add(StandardMaterial {
            base_color: Color::WHITE,
            emissive: LinearRgba::rgb(10_000., 12_000., 16_000.),
            unlit: true,
            cull_mode: None,
            ..default()
        });
        let bolt_entity = commands
            .spawn((
                player::SceneEntity,
                Mesh3d(bolt_mesh.clone()),
                MeshMaterial3d(bolt_material.clone()),
                Transform::default(),
                Visibility::Hidden,
                NotShadowCaster,
            ))
            .id();
        commands.insert_resource(Particles {
            groups,
            rng: 0x1224_7733,
            previous: w.elapsed,
            active: 0,
            impacts: 0,
            bolt_entity,
            bolt_mesh,
            bolt_material,
            generation: 0,
        });
        return;
    };
    let dt = ((w.elapsed - state.previous) as f32).clamp(0., 0.25);
    state.previous = w.elapsed;
    let forward = *camera.forward();
    let right = *camera.right();
    let up = *camera.up();
    let center = camera.translation + forward * 17.;
    let extent = Vec3::new(29., 23., 29.);
    let wind = Vec3::new(w.sample.wind.x, 0., w.sample.wind.y)
        * (1. + 0.18 * ((w.elapsed as f32) * 0.7).sin());
    let weights = [w.sample.rain, w.sample.snow, w.sample.hail, w.sample.dust];
    let sum = weights.iter().sum::<f32>().max(1.);
    let budget = w.quality.particles();
    let mut seed = state.rng;
    let mut active = 0;
    let mut impacts = 0;
    for group in &mut state.groups {
        let count = ((budget as f32) * weights[group.kind] / sum) as usize;
        active += count;
        if let Ok(mut v) = visibility.get_mut(group.entity) {
            *v = if count == 0 {
                Visibility::Hidden
            } else {
                Visibility::Visible
            };
        }
        if count == 0 {
            continue;
        }
        if count < group.previous_count {
            group.quads[count..group.previous_count].fill(Quad::default());
        }
        group.previous_count = count;
        for (i, p) in group.points.iter_mut().take(count).enumerate() {
            if !p.position.is_finite() || (p.position - center).abs().cmpgt(extent).any() {
                p.position = center
                    + Vec3::new(
                        (random(&mut seed) - 0.5) * extent.x * 2.,
                        (random(&mut seed) - 0.5) * extent.y * 2.,
                        (random(&mut seed) - 0.5) * extent.z * 2.,
                    );
                let y = surface(
                    p.position.x,
                    p.position.z,
                    &preview,
                    shelter.as_deref(),
                    vehicle.as_deref(),
                );
                p.position.y = p.position.y.max(y + random(&mut seed) * 12. + 0.2);
                p.phase = random(&mut seed) * std::f32::consts::TAU;
                p.bounced = false;
                p.velocity = match group.kind {
                    0 => Vec3::NEG_Y * (12. + random(&mut seed) * 8.) + wind * 0.6,
                    1 => Vec3::NEG_Y * (0.8 + random(&mut seed) * 1.2) + wind,
                    2 => Vec3::NEG_Y * (9. + random(&mut seed) * 5.) + wind * 0.5,
                    _ => wind + Vec3::Y * (random(&mut seed) - 0.5),
                };
            }
            p.velocity.x += (wind.x - p.velocity.x) * dt * 0.7;
            p.velocity.z += (wind.z - p.velocity.z) * dt * 0.7;
            if group.kind == 1 {
                p.velocity.x += (w.elapsed as f32 * 1.7 + p.phase).sin() * dt * 0.4;
            }
            if group.kind == 2 {
                p.velocity.y -= 9.81 * dt;
            }
            p.position += p.velocity * dt;
            let ground = surface(
                p.position.x,
                p.position.z,
                &preview,
                shelter.as_deref(),
                vehicle.as_deref(),
            );
            if p.position.y < ground + 0.03 {
                impacts += 1;
                if group.kind == 2 && !p.bounced {
                    p.position.y = ground + 0.03;
                    p.velocity.y = p.velocity.y.abs() * 0.32;
                    p.bounced = true;
                } else {
                    p.position.y = center.y + extent.y;
                    p.bounced = false;
                }
            }
            let brightness = 0.12 + 0.88 * w.daylight + w.flash * 2.;
            let (a, b, color) = match group.kind {
                0 => {
                    let along = p.velocity.normalize_or_zero();
                    let sideways = forward.cross(along).normalize_or_zero();
                    (
                        sideways * 0.007,
                        along * (0.15 + weights[0] * 0.24),
                        [
                            0.62 * brightness,
                            0.75 * brightness,
                            0.86 * brightness,
                            0.32,
                        ],
                    )
                }
                1 => (
                    right * 0.035,
                    up * 0.035,
                    [0.95 * brightness, 0.98 * brightness, brightness, 0.86],
                ),
                2 => (
                    right * 0.028,
                    up * 0.028,
                    [0.82 * brightness, 0.91 * brightness, brightness, 0.95],
                ),
                _ => (
                    right * 0.7,
                    up * 0.7,
                    [
                        0.46 * brightness,
                        0.31 * brightness,
                        0.16 * brightness,
                        0.065,
                    ],
                ),
            };
            let distance = p.position.distance(camera.translation);
            let fade = smooth(0.4, 3., distance) * (1. - smooth(25., 42., distance));
            let mut color = color;
            color[3] *= fade;
            for (j, offset) in [-a - b, a - b, a + b, -a + b].into_iter().enumerate() {
                group.quads[i].positions[j] = (p.position + offset).to_array();
            }
            group.quads[i].color = color;
        }
        if let Some(mut mesh) = meshes.get_mut(&group.mesh) {
            if let Some(VertexAttributeValues::Float32x3(v)) =
                mesh.attribute_mut(Mesh::ATTRIBUTE_POSITION)
            {
                for (q, v) in group.quads.iter().zip(v.as_chunks_mut::<4>().0) {
                    v.copy_from_slice(&q.positions);
                }
            }
            if let Some(VertexAttributeValues::Float32x4(v)) =
                mesh.attribute_mut(Mesh::ATTRIBUTE_COLOR)
            {
                for (q, v) in group.quads.iter().zip(v.as_chunks_mut::<4>().0) {
                    v.fill(q.color);
                }
            }
        }
    }
    state.rng = seed;
    state.active = active;
    state.impacts += impacts;
    if state.generation != w.strike_generation {
        if let Some(strike) = w.strike
            && let Some(mut mesh) = meshes.get_mut(&state.bolt_mesh)
        {
            *mesh = bolt(strike, &mut seed);
        }
        state.generation = w.strike_generation;
    }
    if let Ok(mut v) = visibility.get_mut(state.bolt_entity) {
        *v = if w.flash > 0.01 {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
    if let Some(mut m) = standard.get_mut(&state.bolt_material) {
        let target = LinearRgba::rgb(10_000. * w.flash, 12_000. * w.flash, 16_000. * w.flash);
        if m.emissive != target {
            m.emissive = target;
        }
    }
}
fn bolt(target: Vec3, seed: &mut u32) -> Mesh {
    let mut m = empty_mesh(0);
    let mut vertices = Vec::new();
    let mut ids = Vec::new();
    let top = target + Vec3::new(18., 180., -11.);
    let mut previous = top;
    for i in 1..=14 {
        let t = i as f32 / 14.;
        let p = top.lerp(target, t)
            + Vec3::new(
                (random(seed) - 0.5) * 12. * (1. - t),
                0.,
                (random(seed) - 0.5) * 12. * (1. - t),
            );
        for axis in [Vec3::X, Vec3::Z] {
            let a = vertices.len() as u32;
            let width = axis * (0.10 + (1. - t) * 0.17);
            vertices.extend(
                [previous - width, previous + width, p + width, p - width].map(|p| p.to_array()),
            );
            ids.extend([a, a + 1, a + 2, a, a + 2, a + 3]);
        }
        previous = p;
    }
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, vertices.clone());
    m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0., 1., 0.]; vertices.len()]);
    m.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.; 2]; vertices.len()]);
    m.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[1.; 4]; vertices.len()]);
    m.insert_indices(Indices::U32(ids));
    m
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shelters_follow_actual_sloped_triangles_and_leave_openings() {
        let triangle = [
            Vec3::new(0., 2., 0.),
            Vec3::new(2., 4., 0.),
            Vec3::new(0., 2., 2.),
        ];
        assert_eq!(roof_height(triangle, 0.5, 0.5), Some(2.5));
        assert!(roof_height(triangle, 1.5, 1.5).is_none());
    }
}
