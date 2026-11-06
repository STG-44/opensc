//! Weather response stays in the PBR shader, preserving masks and shadows.
use super::*;
use bevy::{
    asset::uuid_handle,
    pbr::{ExtendedMaterial, MaterialExtension},
    render::render_resource::AsBindGroup,
    shader::ShaderRef,
};
use std::collections::HashMap;
const SURFACE_SHADER: Handle<Shader> = uuid_handle!("cd45ed39-55bb-4724-883e-17215dafc6c0");
#[derive(Component)]
pub(crate) struct ExposedSurface;
#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub(super) struct Response {
    #[uniform(100)]
    state: Vec4,
}
impl MaterialExtension for Response {
    fn fragment_shader() -> ShaderRef {
        SURFACE_SHADER.clone().into()
    }
    fn deferred_fragment_shader() -> ShaderRef {
        SURFACE_SHADER.clone().into()
    }
}
type WeatherMaterial = ExtendedMaterial<StandardMaterial, Response>;
pub(super) struct SurfacePlugin;
impl Plugin for SurfacePlugin {
    fn build(&self, app: &mut App) {
        app.world_mut()
            .resource_mut::<Assets<Shader>>()
            .insert(
                SURFACE_SHADER.id(),
                Shader::from_wesl(
                    super::shader_source("surfaces.wesl", include_str!("surfaces.wesl")),
                    "opensc/environment/surfaces.wesl",
                ),
            )
            .expect("surface shader ID");
        app.add_plugins(MaterialPlugin::<WeatherMaterial>::default());
    }
}
#[derive(Resource, Default)]
pub(super) struct Surfaces {
    materials: HashMap<AssetId<StandardMaterial>, Handle<WeatherMaterial>>,
    last: f64,
}
#[allow(clippy::too_many_arguments)]
pub(super) fn update(
    mut commands: Commands,
    weather: Option<Res<Weather>>,
    state: Option<ResMut<Surfaces>>,
    targets: Query<(Entity, &MeshMaterial3d<StandardMaterial>), With<ExposedSurface>>,
    standard: Res<Assets<StandardMaterial>>,
    mut materials: ResMut<Assets<WeatherMaterial>>,
) {
    let Some(w) = weather else {
        return;
    };
    let Some(mut state) = state else {
        commands.insert_resource(Surfaces {
            last: -10.,
            ..default()
        });
        return;
    };
    let response = Vec4::new(w.wetness, w.snow_cover, w.sample.temperature, 0.);
    for (entity, handle) in &targets {
        let Some(base) = standard.get(handle) else {
            continue;
        };
        if base.unlit || base.perceptual_roughness <= 0.3 {
            commands.entity(entity).remove::<ExposedSurface>();
            continue;
        }
        let coated = state
            .materials
            .entry(handle.id())
            .or_insert_with(|| {
                materials.add(WeatherMaterial {
                    base: base.clone(),
                    extension: Response { state: response },
                })
            })
            .clone();
        commands
            .entity(entity)
            .remove::<MeshMaterial3d<StandardMaterial>>()
            .remove::<ExposedSurface>()
            .insert(MeshMaterial3d(coated));
    }
    if w.elapsed - state.last >= 0.2 {
        for handle in state.materials.values() {
            if let Some(mut m) = materials.get_mut(handle)
                && m.extension.state != response
            {
                m.extension.state = response;
            }
        }
        state.last = w.elapsed;
    }
}
