//! Персонаж SMF + BMF: позы, ходьба, движение по площадке и смена клипов.
mod asset_mesh;
mod motion;
mod normals;
mod rig;
mod smf_transform;
use bevy::{
    input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll},
    prelude::*,
    render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk},
    text::FontSize,
};
use motion::{Clip, Playback, to_world_delta};
use shadow_formats::{AssetCatalog, BmfMotion};
use std::{env, error::Error, path::PathBuf};
const MOTIONS: [&str; 4] = [
    "motions/bipedal_stand.bmf",
    "motions/bipedal_stand_walk_rifle.bmf",
    "motions/bipedal_stand_run.bmf",
    "motions/bipedal_stand_wave.bmf",
];
struct Config {
    game: PathBuf,
    overrides: Option<PathBuf>,
    motion: String,
    screenshot: Option<PathBuf>,
    pose_time: Option<f32>,
    root_motion: bool,
    pose_json: Option<PathBuf>,
    armed: bool,
}
#[derive(Resource)]
struct Character {
    rig: Option<rig::Rig>,
    clips: Vec<Clip>,
    focus: Vec3,
    basis: Transform,
    config: Config,
}
#[derive(Resource)]
struct Orbit {
    yaw: f32,
    pitch: f32,
    distance: f32,
}
#[derive(Component)]
struct Bone(usize);
#[derive(Component)]
struct SceneRoot;
#[derive(Component)]
struct PreviewCamera;
#[derive(Component)]
struct Status;
#[derive(Component)]
struct Ground;
#[derive(Component)]
struct GridLine {
    index: i32,
    along_x: bool,
}
fn main() {
    if let Err(e) = run() {
        eprintln!("Ошибка: {e}");
        std::process::exit(1);
    }
}
fn run() -> Result<(), Box<dyn Error>> {
    let Some(config) = config()? else {
        return Ok(());
    };
    let catalog = AssetCatalog::open(&config.game, config.overrides.as_deref())?;
    let rig = rig::prepare_appearance(
        &catalog,
        &rig::Appearance {
            body: "models/people/bodies/man3_sp/man3_sp.smf",
            head: "models/people/heads/head_peter/head_peter.smf",
            body_texture: "man3_peter.bmp",
            head_texture: None,
            pack: None,
        },
        config.armed,
    )?;
    let mut paths = MOTIONS.iter().map(|p| (*p).to_owned()).collect::<Vec<_>>();
    if !paths.contains(&config.motion) {
        paths.push(config.motion.clone());
    }
    let clips = paths
        .iter()
        .map(|p| {
            let bytes = catalog.read(p)?;
            let m = BmfMotion::from_bytes(&bytes)?;
            Clip::new(&m, &rig.bones).map_err(|e| format!("{p}: {e}").into())
        })
        .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
    let selected = paths
        .iter()
        .position(|p| p == &config.motion)
        .expect("selected path is indexed");
    let clip = &clips[selected];
    if config.pose_time.is_some_and(|t| t > clip.duration()) {
        return Err("--pose-time выходит за длительность клипа".into());
    }
    let playback = Playback::new(
        &clips,
        selected,
        config.pose_time.unwrap_or(0.0),
        config.pose_time.is_some(),
        config.root_motion,
    );
    println!(
        "Питер Болланд: {} костей, {} частей, {} вершин, {} треугольников; BMF {}: {} кадров, {} FPS",
        rig.bones.len(),
        rig.parts.len(),
        rig.vertices,
        rig.triangles,
        clip.name,
        clip.frames.len(),
        clip.fps
    );
    if let Some(path) = &config.pose_json {
        let mut world_root = rig.root;
        world_root.translation += to_world_delta(&rig.root, playback.travel);
        let weapon_points = rig::weapon_world_vertices(&rig, &playback.pose.bones, world_root);
        let nodes=rig.bones.iter().zip(&playback.pose.bones).map(|(b,t)|serde_json::json!({
            "id":b.id,"name":b.name,"translation":t.translation.to_array(),"rotation_xyzw":t.rotation.to_array(),"scale":t.scale.to_array()
        })).collect::<Vec<_>>();
        let value = serde_json::json!({"motion":paths[selected],"seconds":playback.seconds,"root_motion":playback.root_motion,
            "root_source":playback.pose.root.to_array(),"displacement_source":playback.travel.to_array(),
            "root_world_delta":to_world_delta(&rig.root,playback.travel).to_array(),"presentation_scale":rig.root.scale.to_array(),"bones":nodes,
            "weapon":if rig.armed { Some(rig::WEAPON) } else { None },"weapon_bone_id":if rig.armed { Some(rig::RIGHT_HAND) } else { None },
            "presentation_translation":rig.root.translation.to_array(),"weapon_world_vertices":weapon_points});
        std::fs::write(path, serde_json::to_string_pretty(&value)? + "\n")?;
        return Ok(());
    }
    let focus = rig.focus;
    let basis = rig.root;
    App::new()
        .insert_resource(ClearColor(Color::srgb(0.035, 0.048, 0.045)))
        .insert_resource(Character {
            rig: Some(rig),
            clips,
            focus,
            basis,
            config,
        })
        .insert_resource(playback)
        .insert_resource(Orbit {
            yaw: -2.6,
            pitch: 0.18,
            distance: 6.7,
        })
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Shadow Company · движение / Rust + Bevy".into(),
                resolution: (1280, 800).into(),
                ..default()
            }),
            ..default()
        }))
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (
                playback_controls,
                camera_controls,
                animate,
                move_root,
                move_ground,
                orbit_camera,
                capture,
            )
                .chain(),
        )
        .run();
    Ok(())
}
fn config() -> Result<Option<Config>, Box<dyn Error>> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    if args.iter().any(|a| a == "--help") {
        println!(
            "shadow-character-viewer --game-dir PATH [--overrides PATH] [--weapon ak47|none] [--motion NAME.bmf] [--root-motion on|off] [--pose-time SECONDS] [--pose-json FILE.json] [--screenshot FILE.png]\nW/S/R/G: ходьба/стойка/бег/жест; M: движение по площадке; Home: начало; Space: пауза; [ / ]: кадры; 1/2/3: скорость; ЛКМ/стрелки: камера; колесо: масштаб; Esc: выход."
        );
        return Ok(None);
    }
    let mut config = Config {
        game: PathBuf::new(),
        overrides: None,
        motion: MOTIONS[1].into(),
        screenshot: None,
        pose_time: None,
        root_motion: true,
        pose_json: None,
        armed: true,
    };
    let mut game = env::var_os("SHADOW_GAME_DIR").map(PathBuf::from);
    let mut seen = std::collections::HashSet::new();
    for pair in args.chunks(2) {
        let key = &pair[0];
        if !seen.insert(key) {
            return Err(format!("повтор аргумента {key}").into());
        }
        let value = pair
            .get(1)
            .filter(|v| !v.starts_with("--"))
            .ok_or_else(|| format!("после {key} нужно значение"))?;
        match key.as_str() {
            "--game-dir" => game = Some(value.into()),
            "--overrides" => config.overrides = Some(value.into()),
            "--motion" => config.motion = shadow_formats::normalize_path(value)?,
            "--screenshot" => config.screenshot = Some(value.into()),
            "--pose-json" => config.pose_json = Some(value.into()),
            "--weapon" => {
                config.armed = match value.as_str() {
                    "ak47" => true,
                    "none" => false,
                    _ => return Err("--weapon должен быть ak47 или none".into()),
                }
            }
            "--root-motion" => {
                config.root_motion = match value.as_str() {
                    "on" => true,
                    "off" => false,
                    _ => return Err("--root-motion должен быть on или off".into()),
                }
            }
            "--pose-time" => {
                let t = value.parse::<f32>()?;
                if !t.is_finite() || t < 0.0 {
                    return Err("--pose-time должен быть конечным и >= 0".into());
                }
                config.pose_time = Some(t);
            }
            _ => return Err(format!("неизвестный аргумент {key}").into()),
        }
    }
    config.game = game.ok_or("укажи --game-dir или SHADOW_GAME_DIR")?;
    if config.pose_json.is_some() && config.screenshot.is_some() {
        return Err("выбери --pose-json или --screenshot".into());
    }
    for (path, extension) in [
        (config.screenshot.as_ref(), "png"),
        (config.pose_json.as_ref(), "json"),
    ] {
        if let Some(path) = path {
            if path
                .extension()
                .is_none_or(|e| !e.eq_ignore_ascii_case(extension))
            {
                return Err(format!("выходной файл должен иметь расширение .{extension}").into());
            }
            if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                std::fs::create_dir_all(parent)?;
            }
        }
    }
    Ok(Some(config))
}
fn setup(
    mut commands: Commands,
    mut character: ResMut<Character>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut fonts: ResMut<Assets<Font>>,
) {
    commands.spawn((
        Camera3d::default(),
        PreviewCamera,
        AmbientLight {
            brightness: 350.0,
            ..default()
        },
        Transform::default(),
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 6500.0,
            ..default()
        },
        Transform::from_xyz(4.0, 7.0, -5.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.spawn((
        Ground,
        Mesh3d(meshes.add(Plane3d::default().mesh().size(60.0, 60.0))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.085, 0.11, 0.10),
            perceptual_roughness: 1.0,
            ..default()
        })),
        Transform::from_xyz(0.0, -0.025, 0.0),
    ));
    let grid = meshes.add(Cuboid::new(0.012, 0.004, 10.0));
    let grid_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.16, 0.21, 0.18),
        unlit: true,
        ..default()
    });
    for index in -5..=5 {
        for along_x in [false, true] {
            commands.spawn((
                Ground,
                GridLine { index, along_x },
                Mesh3d(grid.clone()),
                MeshMaterial3d(grid_material.clone()),
                Transform::default().with_rotation(if along_x {
                    Quat::from_rotation_y(std::f32::consts::FRAC_PI_2)
                } else {
                    Quat::IDENTITY
                }),
            ));
        }
    }
    let rig = character.rig.take().expect("startup runs once");
    let root = commands
        .spawn((
            SceneRoot,
            Name::new("Peter Bolland"),
            rig.root,
            Visibility::default(),
        ))
        .id();
    let entities = rig
        .bones
        .iter()
        .enumerate()
        .map(|(i, b)| {
            commands
                .spawn((
                    Name::new(b.name.clone()),
                    Bone(i),
                    b.bind,
                    Visibility::default(),
                ))
                .id()
        })
        .collect::<Vec<_>>();
    for (i, b) in rig.bones.iter().enumerate() {
        commands
            .entity(entities[i])
            .insert(ChildOf(b.parent.map(|p| entities[p]).unwrap_or(root)));
    }
    let textures = rig
        .images
        .into_iter()
        .map(|(n, i)| (n, (images.add(i.image), i.alpha_mode)))
        .collect::<std::collections::BTreeMap<_, _>>();
    for part in rig.parts {
        let material = materials.add(StandardMaterial {
            base_color_texture: Some(textures[&part.texture].0.clone()),
            alpha_mode: textures[&part.texture].1,
            perceptual_roughness: 0.95,
            cull_mode: None,
            ..default()
        });
        commands.spawn((
            Name::new(part.name),
            Mesh3d(meshes.add(part.mesh)),
            MeshMaterial3d(material),
            Transform::default(),
            ChildOf(entities[part.bone]),
        ));
    }
    let font = fonts.add(Font::from_bytes(
        include_bytes!("../../shadow-viewer/assets/fonts/DejaVuSans.ttf").to_vec(),
    ));
    for (text, size, top) in [
        ("SHADOW COMPANY".to_owned(), 30.0, 30.0),
        ("Rust + Bevy · ходьба оригинала".into(), 17.0, 76.0),
        (
            format!(
                "ПИТЕР БОЛЛАНД\n\nТело + голова из SMF\n{} костей · {} вершин\n{} треугольников\n\nОружие: {}\nТекстуры оригинала",
                rig.bones.len(),
                rig.vertices,
                rig.triangles,
                if rig.armed {
                    "AK-47 · правая кисть"
                } else {
                    "нет"
                }
            ),
            15.0,
            133.0,
        ),
    ] {
        commands.spawn((
            Text::new(text),
            TextFont {
                font: font.clone().into(),
                font_size: FontSize::Px(size),
                ..default()
            },
            TextColor(Color::srgb(0.83, 0.87, 0.71)),
            Node {
                position_type: PositionType::Absolute,
                left: px(30),
                top: px(top),
                width: if top < 100.0 { px(600) } else { px(320) },
                ..default()
            },
        ));
    }
    commands.spawn((
        Status,
        Text::default(),
        TextFont {
            font: font.clone().into(),
            font_size: FontSize::Px(15.0),
            ..default()
        },
        TextColor(Color::srgb(0.70, 0.85, 0.76)),
        Node {
            position_type: PositionType::Absolute,
            left: px(30),
            top: px(370),
            width: px(320),
            ..default()
        },
    ));
    commands.spawn((Text::new("W — ходьба     S — стойка     R — бег     G — жест     M — движение     Home — начало\nSpace — пауза     [ / ] — кадр     1 / 2 / 3 — скорость     ЛКМ / стрелки — камера     Колесо — масштаб     Esc — выход"),
        TextFont{font:font.into(),font_size:FontSize::Px(14.0),..default()},TextColor(Color::srgb(0.69,0.75,0.71)),
        Node{position_type:PositionType::Absolute,left:px(30),bottom:px(24),..default()}));
}
fn playback_controls(
    input: Res<ButtonInput<KeyCode>>,
    character: Res<Character>,
    mut playback: ResMut<Playback>,
    mut exit: MessageWriter<AppExit>,
) {
    if input.just_pressed(KeyCode::Escape) {
        exit.write(AppExit::Success);
    }
    if input.just_pressed(KeyCode::Space) {
        playback.paused = !playback.paused;
    }
    if input.just_pressed(KeyCode::KeyM) {
        playback.root_motion = !playback.root_motion;
    }
    if input.just_pressed(KeyCode::Home) {
        playback.reset(&character.clips);
    }
    for (key, selected) in [
        (KeyCode::KeyS, 0),
        (KeyCode::KeyW, 1),
        (KeyCode::KeyR, 2),
        (KeyCode::KeyG, 3),
    ] {
        if input.just_pressed(key) {
            playback.select(&character.clips, selected);
        }
    }
    for (key, speed) in [
        (KeyCode::Digit1, 0.5),
        (KeyCode::Digit2, 1.0),
        (KeyCode::Digit3, 2.0),
    ] {
        if input.just_pressed(key) {
            playback.speed = speed;
        }
    }
    for (key, direction) in [(KeyCode::BracketLeft, -1.0), (KeyCode::BracketRight, 1.0)] {
        if input.just_pressed(key) {
            playback.step_frame(&character.clips, direction);
        }
    }
}
fn camera_controls(
    input: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    time: Res<Time>,
    mut orbit: ResMut<Orbit>,
) {
    let dt = time.delta_secs();
    orbit.yaw += dt
        * (f32::from(input.pressed(KeyCode::ArrowRight))
            - f32::from(input.pressed(KeyCode::ArrowLeft)));
    orbit.pitch += dt
        * (f32::from(input.pressed(KeyCode::ArrowUp))
            - f32::from(input.pressed(KeyCode::ArrowDown)));
    if mouse.pressed(MouseButton::Left) {
        orbit.yaw -= motion.delta.x * 0.006;
        orbit.pitch += motion.delta.y * 0.006;
    }
    orbit.pitch = orbit.pitch.clamp(-0.1, 1.35);
    orbit.distance = (orbit.distance * (-scroll.delta.y * 0.1).exp()).clamp(3.0, 15.0);
}
fn animate(
    character: Res<Character>,
    time: Res<Time>,
    mut playback: ResMut<Playback>,
    mut bones: Query<(&Bone, &mut Transform)>,
    mut status: Single<&mut Text, With<Status>>,
) {
    playback.tick(&character.clips, time.delta_secs());
    for (bone, mut transform) in &mut bones {
        *transform = playback.pose.bones[bone.0];
    }
    let clip = &character.clips[playback.selected];
    let delta = to_world_delta(&character.basis, playback.travel);
    **status = Text::new(format!(
        "{}\n{} кадров · {} FPS\n\n{} · {:.2} / {:.2} с\nСкорость: {:.1}×\n\n{}\nПоложение: {:.2}, {:.2}, {:.2}",
        clip.name,
        clip.frames.len(),
        clip.fps,
        if playback.paused {
            "ПАУЗА"
        } else {
            "ВОСПРОИЗВЕДЕНИЕ"
        },
        playback.seconds,
        clip.duration(),
        playback.speed,
        if playback.root_motion {
            "Движение по площадке"
        } else {
            "Анимация на месте"
        },
        delta.x,
        delta.y,
        delta.z
    ));
}
fn move_root(
    character: Res<Character>,
    playback: Res<Playback>,
    mut root: Single<&mut Transform, With<SceneRoot>>,
) {
    root.translation =
        character.basis.translation + to_world_delta(&character.basis, playback.travel);
}
fn move_ground(
    character: Res<Character>,
    playback: Res<Playback>,
    mut ground: Query<(&mut Transform, Option<&GridLine>), With<Ground>>,
) {
    let delta = to_world_delta(&character.basis, playback.travel);
    let center = Vec3::new(delta.x.round(), 0.0, delta.z.round());
    for (mut transform, line) in &mut ground {
        transform.translation = match line {
            Some(line) => {
                center
                    + if line.along_x {
                        Vec3::new(0.0, -0.02, line.index as f32)
                    } else {
                        Vec3::new(line.index as f32, -0.02, 0.0)
                    }
            }
            None => Vec3::new(delta.x, -0.025, delta.z),
        };
    }
}
fn orbit_camera(
    character: Res<Character>,
    playback: Res<Playback>,
    orbit: Res<Orbit>,
    mut camera: Single<&mut Transform, With<PreviewCamera>>,
) {
    let right = Vec3::new(orbit.yaw.cos(), 0.0, -orbit.yaw.sin());
    let target = character.focus + to_world_delta(&character.basis, playback.travel)
        - right * (orbit.distance * 0.16);
    let direction = Vec3::new(
        orbit.yaw.sin() * orbit.pitch.cos(),
        orbit.pitch.sin(),
        orbit.yaw.cos() * orbit.pitch.cos(),
    );
    **camera = Transform::from_translation(target + direction * orbit.distance)
        .looking_at(target, Vec3::Y);
}
fn capture(mut commands: Commands, character: Res<Character>, mut frames: Local<u32>) {
    *frames += 1;
    if *frames == 120
        && let Some(path) = &character.config.screenshot
    {
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path.clone()))
            .observe(
                |_: On<ScreenshotCaptured>, mut exit: MessageWriter<AppExit>| {
                    exit.write(AppExit::Success);
                },
            );
    }
}

#[cfg(test)]
mod control_tests {
    use super::*;
    use motion::Pose;
    fn press(app: &mut App, key: KeyCode) {
        *app.world_mut().resource_mut::<ButtonInput<KeyCode>>() = ButtonInput::default();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(key);
        app.update();
    }
    #[test]
    fn bevy_input_switches_clips_toggles_motion_and_resets_the_origin() {
        // Исполняем настоящую систему ввода без окна, GPU и ресурсов оригинала.
        let clips = (0..4)
            .map(|i| Clip {
                fps: 10.0,
                name: format!("{i}"),
                frames: vec![
                    Pose {
                        bones: vec![Transform::default()],
                        root: Vec3::ZERO,
                    },
                    Pose {
                        bones: vec![Transform::from_xyz(0.0, 1.0, 0.0)],
                        root: Vec3::Y * 10.0,
                    },
                ],
            })
            .collect::<Vec<_>>();
        let mut playback = Playback::new(&clips, 0, 0.0, false, true);
        playback.travel = Vec3::Z * 50.0;
        let config = Config {
            game: PathBuf::new(),
            overrides: None,
            motion: MOTIONS[0].into(),
            screenshot: None,
            pose_time: None,
            root_motion: true,
            pose_json: None,
            armed: false,
        };
        let mut app = App::new();
        app.add_message::<AppExit>()
            .insert_resource(ButtonInput::<KeyCode>::default())
            .insert_resource(Character {
                rig: None,
                clips,
                focus: Vec3::ZERO,
                basis: Transform::default(),
                config,
            })
            .insert_resource(playback)
            .add_systems(Update, playback_controls);
        for (key, expected) in [
            (KeyCode::KeyW, 1),
            (KeyCode::KeyR, 2),
            (KeyCode::KeyG, 3),
            (KeyCode::KeyS, 0),
        ] {
            press(&mut app, key);
            let p = app.world().resource::<Playback>();
            assert_eq!(p.selected, expected);
            assert_eq!(p.travel, Vec3::Z * 50.0);
        }
        press(&mut app, KeyCode::Space);
        assert!(app.world().resource::<Playback>().paused);
        press(&mut app, KeyCode::BracketRight);
        assert_eq!(app.world().resource::<Playback>().seconds, 0.1);
        press(&mut app, KeyCode::KeyM);
        assert!(!app.world().resource::<Playback>().root_motion);
        press(&mut app, KeyCode::Digit3);
        assert_eq!(app.world().resource::<Playback>().speed, 2.0);
        press(&mut app, KeyCode::Home);
        let p = app.world().resource::<Playback>();
        assert_eq!(p.seconds, 0.0);
        assert_eq!(p.travel, Vec3::ZERO);
    }
}
