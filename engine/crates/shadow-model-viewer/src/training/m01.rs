//! First-area terrain playtest using the existing map renderer and locomotion.
use super::*;
use crate::{daylight::Daylight, map_scene::WORLD_SCALE};
use bevy::{
    asset::RenderAssetUsages,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};
use shadow_formats::{MtfMap, TerrainConfig, TerrainGrid};
use std::{collections::HashSet, fs::File, io::Read, path::Path};

const MAGIC: &[u8] = b"OpenSC heightfield 1\n";
const HEADER_BYTES: usize = 20;
const MAX_SAMPLES: usize = 1_048_576;
const MAX_FILE_BYTES: usize = MAGIC.len() + HEADER_BYTES + MAX_SAMPLES * 4;

pub(super) fn validate_arguments(seen: &HashSet<&String>, config: &Config) -> Result<(), String> {
    let supported = [
        "--m01-terrain",
        "--game-dir",
        "--overrides",
        "--window",
        "--language",
        "--scene-json",
        "--screenshot",
        "--camera-orbit",
        "--weather",
        "--weather-quality",
        "--time-of-day",
        "--day-length",
        "--weather-json",
        "--weather-time",
        "--weather-flash",
        "--player-json",
        "--player-action",
        "--player-seconds",
    ];
    if let Some(argument) = seen.iter().find(|key| !supported.contains(&key.as_str())) {
        return Err(format!("{argument} is unavailable for --m01-terrain"));
    }
    if !matches!(config.player_action.as_str(), "idle" | "walk" | "run") {
        return Err("M01 locomotion diagnostics support idle, walk or run".into());
    }
    if config.player_seconds.is_some() && config.player_json.is_none() {
        return Err("M01 simulated movement requires --player-json".into());
    }
    Ok(())
}

fn decode(bytes: &[u8]) -> Result<(TerrainGrid, Vec2), &'static str> {
    let body = bytes
        .strip_prefix(MAGIC)
        .ok_or("unsupported terrain format")?;
    let (header, samples) = body
        .split_at_checked(HEADER_BYTES)
        .ok_or("incomplete terrain header")?;
    let word = |index: usize| {
        let at = index * 4;
        <[u8; 4]>::try_from(&header[at..at + 4]).unwrap()
    };
    let width = u32::from_le_bytes(word(0)) as usize;
    let height = u32::from_le_bytes(word(1)) as usize;
    let step = f32::from_le_bytes(word(2));
    let spawn = Vec2::new(f32::from_le_bytes(word(3)), f32::from_le_bytes(word(4)));
    if !(2..=2048).contains(&width)
        || !(2..=2048).contains(&height)
        || width * height > MAX_SAMPLES
        || !(0.25..=64.0).contains(&step)
        || samples.len() != width * height * 4
    {
        return Err("invalid terrain dimensions, spacing or sample count");
    }
    let extent = Vec2::new(width as f32 * step, height as f32 * step);
    if !spawn.is_finite() || !spawn.cmpge(Vec2::ZERO).all() || !spawn.cmple(extent).all() {
        return Err("terrain spawn is outside the map");
    }
    let heights = samples
        .as_chunks::<4>()
        .0
        .iter()
        .map(|sample| {
            let metres = f32::from_le_bytes(*sample);
            if (-10_000.0..=10_000.0).contains(&metres) {
                Ok(metres / WORLD_SCALE)
            } else {
                Err("invalid terrain elevation")
            }
        })
        .collect::<Result<_, _>>()?;
    let grid = TerrainGrid {
        config: TerrainConfig {
            name: "m01".into(),
            width,
            height,
            edge: step / WORLD_SCALE,
            height_step: 1.0,
            water_index: 0,
        },
        heights,
    };
    Ok((grid, spawn))
}

fn prepare(path: &Path) -> Result<(map_scene::MapScene, Vec2), Box<dyn Error>> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take((MAX_FILE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    let (grid, spawn) = decode(&bytes).map_err(|error| format!("{}: {error}", path.display()))?;
    let min = grid.heights.iter().copied().reduce(f32::min).unwrap();
    let max = grid.heights.iter().copied().reduce(f32::max).unwrap();
    let extent = Vec2::from_array(grid.extent()) * WORLD_SCALE;
    let daylight = Daylight {
        rotation: Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2),
        color: Color::WHITE,
        source: serde_json::json!({"initial_hour": 12.0}),
        tables: None,
    };
    let c = &grid.config;
    let report = serde_json::json!({
        "map": "M01",
        "terrain": {
            "extent_metres": extent.to_array(),
            "samples": [c.width, c.height],
            "spacing_metres": c.edge * WORLD_SCALE,
            "height_range_metres": [min * WORLD_SCALE, max * WORLD_SCALE],
            "triangles": (c.width + 2) * (c.height + 2) * 2,
        },
        "spawn_east_north_metres": spawn.to_array(),
        "daylight": daylight.source,
        "mission_logic": false,
    });
    // A height tint makes the imported shapes readable during terrain tests.
    let pixels = grid
        .heights
        .iter()
        .flat_map(|height| {
            let t = (height - min) / (max - min).max(1.);
            [
                36.0f32.lerp(129., t) as u8,
                65.0f32.lerp(145., t) as u8,
                70.0f32.lerp(77., t) as u8,
                255,
            ]
        })
        .collect();
    let mut texture = Image::new(
        Extent3d {
            width: c.width as u32,
            height: c.height as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        pixels,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    texture.sampler = bevy::image::ImageSampler::linear();
    Ok((
        map_scene::MapScene {
            models: default(),
            instances: Vec::new(),
            terrain: map_scene::terrain_mesh(&grid),
            texture,
            grid,
            daylight,
            actors: Vec::new(),
            focus: Vec3::new(
                extent.x * 0.5,
                (min + max) * 0.5 * WORLD_SCALE,
                -extent.y * 0.5,
            ),
            span: extent.max_element(),
            report,
        },
        spawn,
    ))
}

pub(super) fn run(
    config: &Config,
    catalog: &AssetCatalog,
    path: &Path,
) -> Result<(), Box<dyn Error>> {
    let (scene, spawn) = prepare(path)?;
    if let Some(output) = &config.scene_json {
        std::fs::write(output, serde_json::to_string_pretty(&scene.report)? + "\n")?;
        return Ok(());
    }
    let mission = training_definition(catalog)?;
    let map = MtfMap::from_bytes(&catalog.read(&format!("maps/{}", mission.map))?)?;
    let template = player::initial_actor(&mission, &map.objects)?;
    let actor = shadow_formats::MtfObject {
        inventory: false,
        class: template.class.clone(),
        model: template.model.clone(),
        name: template.name.clone(),
        position: Some([spawn.x / WORLD_SCALE, spawn.y / WORLD_SCALE, 0.0]),
        rotation: Some([0.; 3]),
        id: template.id,
        properties: Vec::new(),
    };
    let mut actor = player::prepare(catalog, &scene.grid, &actor)?;
    if let Some(output) = &config.player_json {
        let seconds = config.player_seconds.unwrap_or(0.);
        actor.preview_action(&config.player_action, &scene.grid);
        let input = ButtonInput::default();
        let mut trace = vec![actor.position.to_array()];
        for _ in 0..(seconds * 60.).round() as usize {
            actor.step(&input, 1.0 / 60.0, &scene.grid);
            trace.push(actor.position.to_array());
        }
        let report =
            serde_json::json!({"map": "M01", "player": actor.report(), "trace_metres": trace});
        std::fs::write(output, serde_json::to_string_pretty(&report)? + "\n")?;
        return Ok(());
    }
    let orbit = config
        .camera_orbit
        .clone()
        .unwrap_or_else(|| player_view(actor.camera_target()));
    let title = if config.language == crate::localization::Language::Russian {
        "OpenSC · M01 Тихая вода"
    } else {
        "OpenSC · M01 Still Water"
    };
    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: title.into(),
            resolution: config.window.into(),
            ..default()
        }),
        ..default()
    }))
    .insert_resource(config.language)
    .add_plugins(crate::localization::LocalizationPlugin)
    .insert_resource(config.environment.clone())
    .add_plugins(crate::environment::EnvironmentPlugin)
    .insert_resource(Preview {
        focus: scene.focus,
        span: scene.span,
        scene: Some(scene),
        screenshot: config.screenshot.clone(),
        grid: None,
        playable: true,
        first_area: true,
    })
    .insert_resource(orbit)
    .insert_resource(actor)
    .init_resource::<orders::ClickHistory>()
    .add_systems(Startup, (setup, player::setup).chain())
    .add_systems(
        Update,
        (
            controls,
            issue_orders,
            update_player,
            update_markers,
            camera.in_set(CameraUpdate),
            capture,
        )
            .chain(),
    );
    if config.environment_json.is_some() || config.environment_time.is_some() {
        app.insert_resource(crate::environment::Probe::new(
            config.environment_json.clone(),
            config.environment_time,
            config.environment_flash,
        ));
    }
    if !app.run().is_success() {
        return Err("M01 terrain playtest failed; see the game log".into());
    }
    Ok(())
}

fn grounded_camera(orbit: &mut Orbit, grid: &TerrainGrid) -> Transform {
    let [east, north] = grid.extent().map(|units| units * WORLD_SCALE);
    orbit.target.x = orbit.target.x.clamp(0., east);
    orbit.target.z = orbit.target.z.clamp(-north, 0.);
    if let Some(height) = player::surface(grid, orbit.target) {
        orbit.target.y = height + 1.4;
    }
    let mut pose = camera_pose(orbit);
    if let Some(height) = player::surface(grid, pose.translation)
        && pose.translation.y < height + 1.4
    {
        pose.translation.y = height + 1.4;
        pose.look_at(camera_focus(orbit), Vec3::Y);
    }
    pose
}

fn camera(
    mut orbit: ResMut<Orbit>,
    preview: Res<Preview>,
    mut camera: Single<&mut Transform, With<MapCamera>>,
) {
    if let Some(grid) = &preview.grid {
        **camera = grounded_camera(&mut orbit, grid);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<u8> {
        let mut bytes = MAGIC.to_vec();
        bytes.extend(2u32.to_le_bytes());
        bytes.extend(2u32.to_le_bytes());
        for value in [4.0f32, 3., 5., 16., 20., 24., 28.] {
            bytes.extend(value.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn imported_heightfield_shares_metres_between_rays_and_movement() {
        let (grid, spawn) = decode(&fixture()).unwrap();
        assert_eq!(spawn, Vec2::new(3., 5.));
        assert_eq!(grid.extent().map(|n| n * WORLD_SCALE), [8., 8.]);
        let point = Vec3::new(2., 40., -2.);
        let hit = orders::terrain_hit(&grid, point, Vec3::NEG_Y).unwrap();
        assert!((hit.y - 22.).abs() < 1e-4);
        assert!((hit.y - player::surface(&grid, point).unwrap()).abs() < 1e-4);
    }

    #[test]
    fn rejects_truncated_or_nonfinite_terrain_and_out_of_bounds_spawn() {
        let mut bytes = fixture();
        assert!(decode(&bytes[..bytes.len() - 1]).is_err());
        let sample = MAGIC.len() + HEADER_BYTES;
        bytes[sample..sample + 4].copy_from_slice(&f32::NAN.to_le_bytes());
        assert!(decode(&bytes).is_err());
        let mut bytes = fixture();
        let spawn_x = MAGIC.len() + 12;
        bytes[spawn_x..spawn_x + 4].copy_from_slice(&9.0f32.to_le_bytes());
        assert!(decode(&bytes).is_err());
    }

    #[test]
    fn camera_stays_inside_the_map_and_above_an_upslope() {
        let (grid, _) = decode(&fixture()).unwrap();
        let mut orbit = Orbit {
            target: Vec3::new(3., 0., 1.),
            yaw: std::f32::consts::PI,
            pitch: 0.1,
            distance: 3.,
        };
        let pose = grounded_camera(&mut orbit, &grid);
        assert_eq!(orbit.target.z, 0.);
        let height = player::surface(&grid, pose.translation).unwrap();
        assert!(pose.translation.y - height >= 1.39);
    }
}
