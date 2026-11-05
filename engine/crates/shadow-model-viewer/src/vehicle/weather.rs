//! Vehicle equipment follows mission lighting and shared precipitation state.
use crate::{
    inventory::Inventory,
    player,
    scene::{LampRole, WiperBinding},
    tutorial::Tutorial,
};
use bevy::prelude::*;
pub(crate) struct EquipmentPlugin;
impl Plugin for EquipmentPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (animate, probe).chain().after(super::visuals));
    }
}

#[derive(Resource, Clone, Debug)]
pub(crate) struct Conditions {
    pub illumination: f32,
    pub rain: f32,
}
impl Conditions {
    pub fn from_source(source: &serde_json::Value) -> Self {
        let light = source["linear_rgb"]
            .as_array()
            .filter(|v| v.len() == 3)
            .map(|v| {
                [0.2126, 0.7152, 0.0722]
                    .iter()
                    .zip(v)
                    .map(|(w, v)| *w * v.as_f64().unwrap_or(1.) as f32)
                    .sum()
            })
            .unwrap_or(1.);
        let mut result = Self {
            illumination: light,
            rain: 0.,
        };
        if let Some(mode) = source["environment_preview"].as_str() {
            result.illumination = if mode.starts_with("night-") { 0.3 } else { 1. };
            result.rain = if mode.ends_with("-rain") { 1. } else { 0. };
        }
        result
    }
    pub fn dark(&self) -> bool {
        self.illumination.is_finite() && self.illumination < 0.55
    }
    pub fn sun(&self) -> f32 {
        if self.dark() { 35. } else { 6500. }
    }
    pub fn ambient(&self) -> f32 {
        if self.dark() { 25. } else { 500. }
    }
}
#[derive(Resource, Default)]
pub(crate) struct EquipmentState {
    phase: f32,
    brake_hold: f32,
    previous_speed: f32,
}
impl EquipmentState {
    pub(super) fn braking(&self) -> bool {
        self.brake_hold > 0.
    }
    fn advance(&mut self, dt: f32, rain: f32, braking: bool, paused: bool) {
        if paused || !dt.is_finite() || dt < 0. {
            return;
        }
        self.brake_hold = if braking {
            0.3
        } else {
            (self.brake_hold - dt).max(0.)
        };
        let wet = rain.is_finite() && rain > 0.01;
        if wet || self.phase > 0. {
            let next = self.phase + dt * 1.2;
            self.phase = if wet {
                next.rem_euclid(1.)
            } else if next >= 1. {
                0.
            } else {
                next
            };
        }
    }
    fn sweep(&self) -> f32 {
        0.5 - 0.5 * (self.phase * std::f32::consts::TAU).cos()
    }
}
#[derive(Component)]
pub(crate) struct Wiper(pub WiperBinding);
#[derive(Component)]
pub(crate) struct Lamp(pub LampRole);
#[derive(Component)]
pub(crate) struct Beam;
#[derive(Component)]
pub(crate) struct MissionSun;

pub(crate) fn attach_beams(commands: &mut Commands, parent: Entity, centers: &[Vec3]) {
    for center in centers {
        commands.spawn((
            player::SceneEntity,
            Beam,
            Name::new("Hummer headlight beam"),
            SpotLight {
                intensity: 0.,
                color: Color::srgb(1., 0.91, 0.73),
                range: 32.,
                radius: 0.05,
                inner_angle: 0.23,
                outer_angle: 0.42,
                shadow_maps_enabled: false,
                ..default()
            },
            Transform::from_translation(*center + Vec3::NEG_Z * 5.)
                .with_rotation(Quat::from_rotation_x(-0.065)),
            ChildOf(parent),
        ));
    }
}

/// Independent lens components give beam origins from the actual authored
/// geometry, avoiding a second approximate set of headlight coordinates.
pub(crate) fn lamp_centers(mesh: &Mesh) -> Vec<Vec3> {
    use bevy::mesh::VertexAttributeValues;
    let Some(VertexAttributeValues::Float32x3(points)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION)
    else {
        return Vec::new();
    };
    let Some(indices) = mesh.indices() else {
        return Vec::new();
    };
    let mut parents = (0..points.len()).collect::<Vec<_>>();
    fn root(p: &[usize], mut i: usize) -> usize {
        while p[i] != i {
            i = p[i];
        }
        i
    }
    let ids = indices.iter().collect::<Vec<_>>();
    for tri in ids.as_chunks::<3>().0 {
        let a = root(&parents, tri[0]);
        for &i in &tri[1..] {
            let b = root(&parents, i);
            parents[b] = a;
        }
    }
    let mut groups = std::collections::BTreeMap::<usize, (Vec3, usize)>::new();
    for (i, p) in points.iter().enumerate() {
        let g = groups.entry(root(&parents, i)).or_default();
        g.0 += Vec3::from_array(*p);
        g.1 += 1;
    }
    groups.values().map(|(p, n)| *p / (*n as f32)).collect()
}
fn emission(role: LampRole, dark: bool, braking: bool) -> LinearRgba {
    match role {
        LampRole::Head if dark => LinearRgba::rgb(16., 13., 9.),
        LampRole::Tail if braking => LinearRgba::rgb(12., 1., 0.3),
        LampRole::Tail if dark => LinearRgba::rgb(1.5, 0.05, 0.01),
        _ => LinearRgba::BLACK,
    }
}
fn brake_requested(
    keys: &ButtonInput<KeyCode>,
    speed: f32,
    occupied: bool,
    selected: bool,
) -> bool {
    occupied
        && selected
        && ((keys.pressed(KeyCode::ArrowDown) && speed > 0.05)
            || (keys.pressed(KeyCode::ArrowUp) && speed < -0.05)
            || (keys.just_pressed(KeyCode::KeyH) && speed.abs() > 0.05))
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn animate(
    time: Res<Time>,
    conditions: Option<Res<Conditions>>,
    state: Option<ResMut<EquipmentState>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut wipers: Query<(&Wiper, &mut Transform)>,
    lamps: Query<(&Lamp, &MeshMaterial3d<StandardMaterial>)>,
    mut beams: Query<&mut SpotLight, With<Beam>>,
    mut sun: Query<&mut DirectionalLight, With<MissionSun>>,
    mut ambient: Query<&mut AmbientLight, With<crate::training::MapCamera>>,
    v: Option<Res<super::Vehicle>>,
    p: Option<Res<player::Player>>,
    inv: Option<Res<Inventory>>,
    t: Option<Res<Tutorial>>,
    keys: Res<ButtonInput<KeyCode>>,
) {
    let (Some(conditions), Some(mut state)) = (conditions, state) else {
        return;
    };
    if keys.just_pressed(KeyCode::Home) {
        *state = EquipmentState::default();
    }
    let paused = p.as_ref().is_some_and(|p| p.playback.paused)
        || inv.is_some_and(|i| i.open)
        || t.is_some_and(|t| t.pausing());
    let braking = brake_requested(
        &keys,
        state.previous_speed,
        v.as_ref().is_some_and(|v| v.occupied),
        p.as_ref().is_some_and(|p| p.selected),
    );
    state.advance(time.delta_secs(), conditions.rain, braking, paused);
    if !paused {
        state.previous_speed = v.map_or(0., |v| v.speed);
    }
    for (wiper, mut transform) in &mut wipers {
        let rotation = Quat::from_axis_angle(wiper.0.axis, wiper.0.sweep * state.sweep());
        let translation = wiper.0.pivot - rotation * wiper.0.pivot;
        if transform.rotation != rotation || transform.translation != translation {
            transform.rotation = rotation;
            transform.translation = translation;
        }
    }
    for (lamp, handle) in &lamps {
        let target = emission(lamp.0, conditions.dark(), state.brake_hold > 0.);
        if let Some(mut material) = materials.get_mut(handle)
            && material.emissive != target
        {
            material.emissive = target;
        }
    }
    for mut light in &mut beams {
        let target = if conditions.dark() { 100_000. } else { 0. };
        if light.intensity != target {
            light.intensity = target;
        }
    }
    for mut light in &mut sun {
        let target = conditions.sun();
        if light.illuminance != target {
            light.illuminance = target;
        }
    }
    for mut light in &mut ambient {
        let target = conditions.ambient();
        if light.brightness != target {
            light.brightness = target;
        }
    }
}

#[derive(Resource)]
pub(crate) struct Probe {
    pub path: std::path::PathBuf,
    frames: u32,
    low: f32,
    high: f32,
    done: bool,
}
impl Probe {
    pub fn new(path: std::path::PathBuf) -> Self {
        Self {
            path,
            frames: 0,
            low: f32::INFINITY,
            high: f32::NEG_INFINITY,
            done: false,
        }
    }
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn probe(
    probe: Option<ResMut<Probe>>,
    conditions: Option<Res<Conditions>>,
    state: Option<Res<EquipmentState>>,
    wipers: Query<(&Wiper, &Transform)>,
    beams: Query<(&SpotLight, &GlobalTransform), With<Beam>>,
    lamps: Query<(&Lamp, &MeshMaterial3d<StandardMaterial>)>,
    materials: Res<Assets<StandardMaterial>>,
) {
    let (Some(mut probe), Some(c), Some(s)) = (probe, conditions, state) else {
        return;
    };
    if probe.done {
        return;
    }
    probe.frames += 1;
    probe.low = probe.low.min(s.sweep());
    probe.high = probe.high.max(s.sweep());
    if probe.frames < 110 {
        return;
    }
    assert_eq!(
        wipers.iter().count(),
        2,
        "enhanced Hummer must have two physical wipers"
    );
    assert_eq!(beams.iter().count(), 2, "two headlight beams");
    assert_eq!(lamps.iter().count(), 2, "head and tail materials");
    for (light, _) in &beams {
        assert_eq!(light.intensity > 0., c.dark());
    }
    for (lamp, handle) in &lamps {
        assert_eq!(
            materials.get(handle).unwrap().emissive,
            emission(lamp.0, c.dark(), false)
        );
    }
    if c.rain > 0.01 {
        assert!(probe.high - probe.low > 0.1, "rain must move wipers");
    } else {
        assert_eq!(probe.high, 0.);
    }
    let report = serde_json::json!({"status":"passed","dark":c.dark(),"rain":c.rain,"frames":probe.frames,"headlight_beams":beams.iter().map(|(b,t)|serde_json::json!({"intensity":b.intensity,"position_metres":t.translation().to_array(),"range":b.range})).collect::<Vec<_>>(),"wiper_sweep_range":[probe.low,probe.high],"physical_wipers":wipers.iter().map(|(w,t)|serde_json::json!({"part":w.0.part,"rotation":t.rotation.to_array(),"pivot_cm":w.0.pivot.to_array()})).collect::<Vec<_>>()});
    std::fs::write(
        &probe.path,
        serde_json::to_string_pretty(&report).unwrap() + "\n",
    )
    .expect("write equipment probe");
    probe.done = true;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn headlights_require_darkness_but_brakes_work_in_daylight() {
        for dark in [false, true] {
            for rain in [0., 1.] {
                let c = Conditions {
                    illumination: if dark { 0.3 } else { 1. },
                    rain,
                };
                assert_eq!(c.dark(), dark);
                assert_eq!(
                    emission(LampRole::Head, dark, false) == LinearRgba::BLACK,
                    !dark
                );
                assert!(
                    emission(LampRole::Tail, dark, true).red
                        > emission(LampRole::Tail, dark, false).red
                );
            }
        }
    }
    #[test]
    fn wipers_only_sweep_in_rain_pause_and_return_to_park() {
        let mut s = EquipmentState::default();
        s.advance(0.2, 0., false, false);
        assert_eq!(s.sweep(), 0.);
        s.advance(0.2, 1., false, false);
        assert!(s.sweep() > 0.);
        let old = s.phase;
        s.advance(1., 1., false, true);
        assert_eq!(s.phase, old);
        for _ in 0..100 {
            s.advance(0.02, 0., false, false);
        }
        assert_eq!(s.sweep(), 0.);
    }
    #[test]
    fn mission_sun_color_sets_darkness_and_preview_is_explicit() {
        assert!(Conditions::from_source(&serde_json::json!({"linear_rgb":[0.4,0.4,0.414]})).dark());
        assert!(!Conditions::from_source(&serde_json::json!({"linear_rgb":[1.,1.,1.]})).dark());
        let c = Conditions::from_source(&serde_json::json!({"environment_preview":"day-rain"}));
        assert!(!c.dark());
        assert_eq!(c.rain, 1.);
    }
    #[test]
    fn brake_pedal_does_not_confuse_reverse_acceleration_or_coasting() {
        let mut keys = ButtonInput::default();
        assert!(!brake_requested(&keys, 4., true, true));
        keys.press(KeyCode::ArrowDown);
        assert!(brake_requested(&keys, 4., true, true));
        assert!(!brake_requested(&keys, -4., true, true));
        assert!(!brake_requested(&keys, 4., false, true));
        keys.reset_all();
        keys.press(KeyCode::ArrowUp);
        assert!(brake_requested(&keys, -4., true, true));
    }
    #[test]
    fn equipment_plugin_runs_without_player_and_parks_when_rain_stops() {
        let mut app = App::new();
        let mut materials = Assets::<StandardMaterial>::default();
        let handle = materials.add(StandardMaterial::default());
        let mut time = Time::<()>::default();
        time.advance_by(std::time::Duration::from_secs_f32(0.1));
        app.insert_resource(time)
            .insert_resource(materials)
            .insert_resource(ButtonInput::<KeyCode>::default())
            .insert_resource(Conditions {
                illumination: 0.3,
                rain: 1.,
            })
            .init_resource::<EquipmentState>()
            .add_plugins(EquipmentPlugin);
        let pivot = Vec3::new(1., 2., 3.);
        let wiper = app
            .world_mut()
            .spawn((
                Wiper(WiperBinding {
                    part: "test".into(),
                    pivot,
                    axis: Vec3::Y,
                    sweep: 1.,
                }),
                Transform::default(),
            ))
            .id();
        app.world_mut()
            .spawn((Lamp(LampRole::Head), MeshMaterial3d(handle.clone())));
        app.update();
        let pose = app.world().get::<Transform>(wiper).unwrap();
        assert_ne!(pose.rotation, Quat::IDENTITY);
        assert!(pose.transform_point(pivot).distance(pivot) < 1e-5);
        assert_eq!(
            app.world()
                .resource::<Assets<StandardMaterial>>()
                .get(&handle)
                .unwrap()
                .emissive
                .red,
            16.
        );
        *app.world_mut().resource_mut::<Conditions>() = Conditions {
            illumination: 1.,
            rain: 0.,
        };
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(std::time::Duration::from_secs(1));
        app.update();
        assert_eq!(
            app.world().get::<Transform>(wiper).unwrap().rotation,
            Quat::IDENTITY
        );
        assert_eq!(
            app.world()
                .resource::<Assets<StandardMaterial>>()
                .get(&handle)
                .unwrap()
                .emissive,
            LinearRgba::BLACK
        );
    }
    #[test]
    fn beams_use_disconnected_lens_geometry_centers() {
        use bevy::{
            asset::RenderAssetUsages, mesh::Indices, render::render_resource::PrimitiveTopology,
        };
        let mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        )
        .with_inserted_attribute(
            Mesh::ATTRIBUTE_POSITION,
            vec![
                [-1., 0., 0.],
                [-0.5, 1., 0.],
                [-1.5, 1., 0.],
                [1., 0., 0.],
                [1.5, 1., 0.],
                [0.5, 1., 0.],
            ],
        )
        .with_inserted_indices(Indices::U32(vec![0, 1, 2, 3, 4, 5]));
        let centers = lamp_centers(&mesh);
        assert_eq!(centers.len(), 2);
        assert!(centers[0].distance(Vec3::new(-1., 2. / 3., 0.)) < 1e-5);
        assert!(centers[1].distance(Vec3::new(1., 2. / 3., 0.)) < 1e-5);
    }
}
