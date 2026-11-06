use super::*;
use bevy::{
    asset::uuid_handle,
    camera::{Exposure, Hdr},
    core_pipeline::tonemapping::Tonemapping,
    light::{
        Atmosphere, AtmosphereEnvironmentMapLight, NotShadowCaster, SunDisk,
        atmosphere::ScatteringMedium,
    },
    mesh::MeshVertexBufferLayoutRef,
    pbr::{AtmosphereSettings, MaterialPipeline, MaterialPipelineKey},
    post_process::bloom::Bloom,
    render::{
        render_resource::{
            AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
        },
        renderer::RenderAdapterInfo,
    },
    shader::ShaderRef,
};

const SKY_SHADER: Handle<Shader> = uuid_handle!("5d831a76-4b35-431e-9c74-510c5b173800");
#[derive(Clone, Copy, ShaderType, Debug)]
pub(super) struct SkyUniform {
    camera_flash: Vec4,
    sun_day: Vec4,
    cloud: Vec4,
    wind_time: Vec4,
    settings: Vec4,
}
#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub(super) struct SkyMaterial {
    #[uniform(0)]
    data: SkyUniform,
}
impl Material for SkyMaterial {
    fn fragment_shader() -> ShaderRef {
        SKY_SHADER.clone().into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Premultiplied
    }
    fn depth_bias(&self) -> f32 {
        1_000_000.
    }
    fn specialize(
        _: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _: &MeshVertexBufferLayoutRef,
        _: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}
pub(super) struct SkyPlugin;
impl Plugin for SkyPlugin {
    fn build(&self, app: &mut App) {
        app.world_mut()
            .resource_mut::<Assets<Shader>>()
            .insert(
                SKY_SHADER.id(),
                Shader::from_wesl(
                    super::shader_source("sky.wesl", include_str!("sky.wesl")),
                    "opensc/environment/sky.wesl",
                ),
            )
            .expect("stable sky shader ID");
        app.add_plugins(MaterialPlugin::<SkyMaterial>::default());
    }
}
#[derive(Component)]
pub(super) struct WeatherCamera;
#[derive(Component)]
pub(super) struct SkyDome;
#[derive(Component)]
pub(super) struct Moon;
#[derive(Component)]
pub(super) struct LightningLight;
#[derive(Resource)]
pub(super) struct Visuals {
    material: Handle<SkyMaterial>,
    medium: Handle<ScatteringMedium>,
    planet: Option<Entity>,
    quality: Quality,
    software: bool,
}
fn atmosphere_settings(q: Quality) -> AtmosphereSettings {
    let high = q == Quality::High;
    AtmosphereSettings {
        transmittance_lut_size: if high {
            UVec2::new(256, 128)
        } else {
            UVec2::new(128, 64)
        },
        transmittance_lut_samples: if high { 40 } else { 24 },
        multiscattering_lut_size: UVec2::splat(if high { 32 } else { 16 }),
        multiscattering_lut_dirs: if high { 64 } else { 16 },
        multiscattering_lut_samples: if high { 20 } else { 10 },
        sky_view_lut_size: if high {
            UVec2::new(400, 200)
        } else {
            UVec2::new(192, 96)
        },
        sky_view_lut_samples: if high { 16 } else { 10 },
        aerial_view_lut_size: UVec3::splat(if high { 32 } else { 16 }),
        aerial_view_lut_samples: if high { 10 } else { 6 },
        aerial_view_lut_max_distance: 4000.,
        ..default()
    }
}
fn uniform(w: &Weather, camera: Vec3) -> SkyUniform {
    SkyUniform {
        camera_flash: camera.extend(w.flash),
        sun_day: w.sun_to.extend(w.daylight),
        cloud: Vec4::new(w.sample.cloud, w.sample.dust, w.sample.sun, w.illumination),
        wind_time: Vec4::new(
            w.sample.wind.x,
            w.sample.wind.y,
            (w.elapsed % 100_000.) as f32,
            w.hour,
        ),
        settings: Vec4::new(
            w.quality.cloud_steps() as f32,
            if w.quality == Quality::Low { 1. } else { 0. },
            w.sample.visibility,
            w.sample.thunder,
        ),
    }
}
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(super) fn initialize(
    mut commands: Commands,
    weather: Option<ResMut<Weather>>,
    options: Res<Options>,
    cameras: Query<(Entity, &Transform), (With<training::MapCamera>, Without<WeatherCamera>)>,
    adapter: Option<Res<RenderAdapterInfo>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<SkyMaterial>>,
    mut mediums: ResMut<Assets<ScatteringMedium>>,
) {
    let Some(mut w) = weather else {
        return;
    };
    for (camera, transform) in &cameras {
        let software = adapter.as_ref().is_some_and(|a| {
            let n = a.name.to_lowercase();
            n.contains("llvmpipe") || n.contains("software") || n.contains("swiftshader")
        });
        w.quality = options.quality.resolve(software);
        let medium = mediums.add(ScatteringMedium::earth(128, 128));
        let planet = if w.quality == Quality::Low {
            None
        } else {
            Some(
                commands
                    .spawn((player::SceneEntity, Atmosphere::earth(medium.clone())))
                    .id(),
            )
        };
        let material = materials.add(SkyMaterial {
            data: uniform(&w, transform.translation),
        });
        commands.spawn((
            player::SceneEntity,
            SkyDome,
            Mesh3d(meshes.add(Sphere::new(2500.).mesh().uv(48, 24))),
            MeshMaterial3d(material.clone()),
            Transform::from_translation(transform.translation),
            NotShadowCaster,
        ));
        commands.entity(camera).insert((
            WeatherCamera,
            Hdr,
            Exposure { ev100: 13. },
            Tonemapping::AcesFitted,
            Bloom::NATURAL,
            DistanceFog::default(),
        ));
        commands
            .entity(camera)
            .entry::<Projection>()
            .and_modify(|mut p| {
                if let Projection::Perspective(p) = &mut *p {
                    p.far = 12_000.;
                }
            });
        if w.quality != Quality::Low {
            commands.entity(camera).insert((
                atmosphere_settings(w.quality),
                AtmosphereEnvironmentMapLight::default(),
            ));
        }
        commands.spawn((
            player::SceneEntity,
            Moon,
            DirectionalLight {
                illuminance: 0.,
                color: Color::srgb(0.62, 0.72, 1.),
                shadow_maps_enabled: false,
                ..default()
            },
            Transform::default(),
        ));
        commands.spawn((
            player::SceneEntity,
            LightningLight,
            DirectionalLight {
                illuminance: 0.,
                color: Color::srgb(0.78, 0.85, 1.),
                shadow_maps_enabled: false,
                ..default()
            },
            Transform::from_xyz(1., 100., 1.).looking_at(Vec3::ZERO, Vec3::Y),
        ));
        commands.insert_resource(Visuals {
            material,
            medium,
            planet,
            quality: w.quality,
            software,
        });
    }
}
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(super) fn update(
    mut commands: Commands,
    weather: Option<ResMut<Weather>>,
    options: Res<Options>,
    visuals: Option<ResMut<Visuals>>,
    mut materials: ResMut<Assets<SkyMaterial>>,
    mut cameras: Query<
        (
            Entity,
            &Transform,
            &mut Exposure,
            &mut DistanceFog,
            &mut AmbientLight,
        ),
        With<WeatherCamera>,
    >,
    mut domes: Query<
        &mut Transform,
        (
            With<SkyDome>,
            Without<WeatherCamera>,
            Without<crate::vehicle::weather::MissionSun>,
            Without<Moon>,
        ),
    >,
    mut suns: Query<
        (
            Entity,
            &mut Transform,
            &mut DirectionalLight,
            Option<&mut SunDisk>,
        ),
        (
            With<crate::vehicle::weather::MissionSun>,
            Without<WeatherCamera>,
            Without<SkyDome>,
            Without<Moon>,
            Without<LightningLight>,
        ),
    >,
    mut moons: Query<
        (&mut Transform, &mut DirectionalLight),
        (
            With<Moon>,
            Without<WeatherCamera>,
            Without<SkyDome>,
            Without<crate::vehicle::weather::MissionSun>,
            Without<LightningLight>,
        ),
    >,
    mut flash: Query<
        &mut DirectionalLight,
        (
            With<LightningLight>,
            Without<crate::vehicle::weather::MissionSun>,
            Without<Moon>,
        ),
    >,
) {
    let (Some(mut w), Some(mut v)) = (weather, visuals) else {
        return;
    };
    let quality = options.quality.resolve(v.software);
    w.quality = quality;
    for (entity, camera, mut exposure, mut fog, mut ambient) in &mut cameras {
        if v.quality != quality {
            if let Some(planet) = v.planet.take() {
                commands.entity(planet).despawn();
            }
            if quality == Quality::Low {
                commands
                    .entity(entity)
                    .remove::<(AtmosphereSettings, AtmosphereEnvironmentMapLight)>();
            } else {
                v.planet = Some(
                    commands
                        .spawn((player::SceneEntity, Atmosphere::earth(v.medium.clone())))
                        .id(),
                );
                commands.entity(entity).insert((
                    atmosphere_settings(quality),
                    AtmosphereEnvironmentMapLight::default(),
                ));
            }
            v.quality = quality;
        }
        let target_exposure = 6. + w.daylight * 7.;
        exposure.ev100 = target_exposure;
        let dust = Vec3::new(0.44, 0.30, 0.16);
        let ordinary = Vec3::new(0.66, 0.72, 0.78);
        let fog_color = ordinary.lerp(dust, w.sample.dust) * (0.045 + 0.955 * w.daylight);
        fog.color = Color::linear_rgb(fog_color.x, fog_color.y, fog_color.z);
        fog.directional_light_color = Color::srgba(1., 0.9, 0.7, 0.12 * w.sample.sun);
        fog.falloff = FogFalloff::from_visibility(w.sample.visibility);
        ambient.color = Color::linear_rgb(0.65 + 0.35 * w.daylight, 0.75 + 0.25 * w.daylight, 1.);
        ambient.brightness = 15. + w.daylight * (280. + (1. - w.sample.sun) * 250.);
        if let Some(mut m) = materials.get_mut(&v.material) {
            m.data = uniform(&w, camera.translation);
        }
        for mut dome in &mut domes {
            dome.translation = camera.translation;
        }
    }
    for (entity, mut transform, mut light, disk) in &mut suns {
        transform.rotation = Quat::from_rotation_arc(Vec3::NEG_Z, -w.sun_to);
        light.illuminance = 110_000. * w.sample.sun * w.daylight;
        light.color = Color::linear_rgb(w.sun_color.x, w.sun_color.y, w.sun_color.z);
        light.shadow_maps_enabled = w.daylight > 0.01;
        if disk.is_none() {
            commands.entity(entity).insert(SunDisk::EARTH);
        }
    }
    for (mut transform, mut light) in &mut moons {
        transform.rotation = Quat::from_rotation_arc(Vec3::NEG_Z, w.sun_to);
        light.illuminance = 0.25 * (1. - w.daylight) * (1. - w.sample.cloud * 0.85);
    }
    for mut light in &mut flash {
        light.illuminance = w.flash * 80_000.;
    }
}
