//! Настоящие SMF-модели и текстуры оригинала в нативном окне Bevy.
mod asset_mesh;
mod normals;
mod scene;
mod smf_transform;
use bevy::{
    input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll},
    prelude::*,
    render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk},
    text::FontSize,
};
use shadow_formats::AssetCatalog;
use std::{collections::BTreeMap, env, error::Error, path::PathBuf};

const DEFAULT_MODEL: &str = "models/alscba-oildrum_alt/alscba-oildrum_alt.smf";

struct Config {
    game: PathBuf,
    overrides: Option<PathBuf>,
    model: String,
    screenshot: Option<PathBuf>,
}
#[derive(Resource)]
struct Browser {
    config: Config,
    catalog: AssetCatalog,
    names: Vec<String>,
    selected: usize,
    dirty: bool,
    initial: Option<scene::PreparedModel>,
    meshes: Vec<Handle<Mesh>>,
    materials: Vec<Handle<StandardMaterial>>,
    images: Vec<Handle<Image>>,
    focus: Vec3,
}
#[derive(Resource)]
struct Orbit {
    yaw: f32,
    pitch: f32,
    distance: f32,
    rotating: bool,
}
impl Default for Orbit {
    fn default() -> Self {
        Self {
            yaw: 0.65,
            pitch: 0.42,
            distance: 7.5,
            rotating: false,
        }
    }
}
#[derive(Component)]
struct ModelPart;
#[derive(Component)]
struct InfoLabel;
#[derive(Component)]
struct PreviewCamera;

fn main() {
    if let Err(error) = run() {
        eprintln!("Ошибка: {error}");
        std::process::exit(1);
    }
}
fn run() -> Result<(), Box<dyn Error>> {
    let Some(config) = config()? else {
        return Ok(());
    };
    let catalog = AssetCatalog::open(&config.game, config.overrides.as_deref())?;
    let names: Vec<_> = catalog
        .paths()
        .filter(|p| p.ends_with(".smf"))
        .map(str::to_owned)
        .collect();
    let selected = names
        .iter()
        .position(|p| *p == config.model)
        .ok_or("модель --model не найдена")?;
    let initial = scene::prepare(&catalog, &config.model, true)?;
    App::new()
        .insert_resource(ClearColor(Color::srgb(0.035, 0.048, 0.045)))
        .insert_resource(Browser {
            config,
            catalog,
            names,
            selected,
            dirty: true,
            initial: Some(initial),
            meshes: Vec::new(),
            materials: Vec::new(),
            images: Vec::new(),
            focus: Vec3::ZERO,
        })
        .init_resource::<Orbit>()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Shadow Company · SMF / Rust + Bevy".into(),
                resolution: (1280, 800).into(),
                ..default()
            }),
            ..default()
        }))
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (
                camera_controls,
                resource_controls,
                load_model,
                orbit_camera,
                capture,
            )
                .chain(),
        )
        .run();
    Ok(())
}
fn config() -> Result<Option<Config>, Box<dyn Error>> {
    let args: Vec<_> = env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help") {
        println!(
            "shadow-model-viewer --game-dir PATH [--overrides PATH] [--model NAME.smf] [--screenshot FILE.png]\nPgUp/PgDn: модель; стрелки / ЛКМ: камера; +/- / колесо: масштаб; Space: вращение; R: перечитать; Esc: выход."
        );
        return Ok(None);
    }
    let mut game = env::var_os("SHADOW_GAME_DIR").map(PathBuf::from);
    let mut config = Config {
        game: PathBuf::new(),
        overrides: None,
        model: DEFAULT_MODEL.into(),
        screenshot: None,
    };
    let mut seen = std::collections::HashSet::new();
    let mut index = 0;
    while index < args.len() {
        let key = args[index].as_str();
        if !seen.insert(key) {
            return Err(format!("повторяющийся аргумент {key}").into());
        }
        let value = args
            .get(index + 1)
            .filter(|v| !v.starts_with("--"))
            .ok_or_else(|| format!("после {key} нужно значение"))?;
        match key {
            "--game-dir" => game = Some(value.into()),
            "--overrides" => config.overrides = Some(value.into()),
            "--model" => config.model = shadow_formats::normalize_path(value)?,
            "--screenshot" => config.screenshot = Some(value.into()),
            _ => return Err(format!("неизвестный аргумент {key}").into()),
        }
        index += 2;
    }
    config.game = game.ok_or("укажи --game-dir или SHADOW_GAME_DIR")?;
    if let Some(path) = &config.screenshot {
        if path
            .extension()
            .is_none_or(|e| !e.eq_ignore_ascii_case("png"))
        {
            return Err("снимок должен иметь расширение .png".into());
        }
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
    }
    Ok(Some(config))
}
fn setup(
    mut commands: Commands,
    mut fonts: ResMut<Assets<Font>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        Camera3d::default(),
        PreviewCamera,
        AmbientLight {
            brightness: 300.0,
            ..default()
        },
        Transform::default(),
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 6000.0,
            ..default()
        },
        Transform::from_xyz(4.0, 7.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(12.0, 12.0))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.085, 0.11, 0.10),
            perceptual_roughness: 1.0,
            ..default()
        })),
        Transform::from_xyz(0.0, -0.025, 0.0),
    ));
    let grid_mesh = meshes.add(Cuboid::new(0.012, 0.004, 10.0));
    let grid_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.15, 0.20, 0.17),
        unlit: true,
        ..default()
    });
    for i in -5..=5 {
        commands.spawn((
            Mesh3d(grid_mesh.clone()),
            MeshMaterial3d(grid_material.clone()),
            Transform::from_xyz(i as f32, -0.02, 0.0),
        ));
        commands.spawn((
            Mesh3d(grid_mesh.clone()),
            MeshMaterial3d(grid_material.clone()),
            Transform::from_xyz(0.0, -0.02, i as f32)
                .with_rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2)),
        ));
    }
    let font = fonts.add(Font::from_bytes(
        include_bytes!("../../shadow-viewer/assets/fonts/DejaVuSans.ttf").to_vec(),
    ));
    for (label, size, top) in [
        ("SHADOW COMPANY", 30.0, 30.0),
        ("Rust + Bevy  ·  модели оригинала в 3D", 17.0, 76.0),
    ] {
        commands.spawn((
            Text::new(label),
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
                ..default()
            },
        ));
    }
    commands.spawn((
        Text::default(),
        InfoLabel,
        TextFont {
            font: font.clone().into(),
            font_size: FontSize::Px(15.0),
            ..default()
        },
        TextColor(Color::srgb(0.77, 0.82, 0.78)),
        Node {
            position_type: PositionType::Absolute,
            left: px(30),
            top: px(133),
            width: px(300),
            ..default()
        },
    ));
    commands.spawn((Text::new("PgUp / PgDn — модель     ЛКМ / стрелки — камера     Колесо — масштаб\nSpace — вращение     R — перечитать ресурсы     Esc — выйти"),
        TextFont {font:font.into(),font_size:FontSize::Px(15.0),..default()}, TextColor(Color::srgb(0.69,0.75,0.71)),
        Node { position_type:PositionType::Absolute,left:px(30),bottom:px(24),..default() }));
}
fn camera_controls(
    input: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mouse: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    mut orbit: ResMut<Orbit>,
) {
    if input.just_pressed(KeyCode::Space) {
        orbit.rotating = !orbit.rotating;
    }
    let step = time.delta_secs();
    if input.pressed(KeyCode::ArrowLeft) {
        orbit.yaw -= step;
    }
    if input.pressed(KeyCode::ArrowRight) || orbit.rotating {
        orbit.yaw += step;
    }
    if input.pressed(KeyCode::ArrowUp) {
        orbit.pitch += step;
    }
    if input.pressed(KeyCode::ArrowDown) {
        orbit.pitch -= step;
    }
    if mouse.pressed(MouseButton::Left) {
        orbit.yaw -= motion.delta.x * 0.006;
        orbit.pitch += motion.delta.y * 0.006;
    }
    if input.pressed(KeyCode::Equal) || input.pressed(KeyCode::NumpadAdd) {
        orbit.distance -= step * 3.0;
    }
    if input.pressed(KeyCode::Minus) || input.pressed(KeyCode::NumpadSubtract) {
        orbit.distance += step * 3.0;
    }
    orbit.distance = (orbit.distance * (-scroll.delta.y * 0.10).exp()).clamp(2.0, 20.0);
    orbit.pitch = orbit.pitch.clamp(-0.1, 1.35);
}
fn resource_controls(
    input: Res<ButtonInput<KeyCode>>,
    mut browser: ResMut<Browser>,
    mut exit: MessageWriter<AppExit>,
) {
    if input.just_pressed(KeyCode::Escape) {
        exit.write(AppExit::Success);
    }
    if input.just_pressed(KeyCode::PageDown) {
        browser.selected = (browser.selected + 1) % browser.names.len();
        browser.dirty = true;
    }
    if input.just_pressed(KeyCode::PageUp) {
        browser.selected = (browser.selected + browser.names.len() - 1) % browser.names.len();
        browser.dirty = true;
    }
    if input.just_pressed(KeyCode::KeyR) {
        match AssetCatalog::open(&browser.config.game, browser.config.overrides.as_deref()) {
            Ok(catalog) => {
                let selected = browser.names[browser.selected].clone();
                let names: Vec<_> = catalog
                    .paths()
                    .filter(|p| p.ends_with(".smf"))
                    .map(str::to_owned)
                    .collect();
                if !names.is_empty() {
                    browser.selected = names.iter().position(|p| *p == selected).unwrap_or(0);
                    browser.names = names;
                    browser.catalog = catalog;
                    browser.initial = None;
                    browser.dirty = true;
                }
            }
            Err(error) => error!("Не удалось перечитать каталог: {error}"),
        }
    }
}
fn load_model(
    mut commands: Commands,
    mut browser: ResMut<Browser>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    parts: Query<Entity, With<ModelPart>>,
    mut label: Single<&mut Text, With<InfoLabel>>,
) {
    if !browser.dirty {
        return;
    }
    browser.dirty = false;
    let path = browser.names[browser.selected].clone();
    let initial = browser
        .initial
        .take()
        .filter(|_| path == browser.config.model);
    let prepared = initial
        .map(Ok)
        .unwrap_or_else(|| scene::prepare(&browser.catalog, &path, true));
    let prepared = match prepared {
        Ok(model) => model,
        Err(error) => {
            **label = Text::new(format!(
                "{path}\n\nОшибка: {error}\n\nПоказана предыдущая модель.\nPgDn — следующая."
            ));
            return;
        }
    };
    for entity in &parts {
        commands.entity(entity).despawn();
    }
    for handle in browser.meshes.drain(..) {
        meshes.remove(handle.id());
    }
    for handle in browser.materials.drain(..) {
        materials.remove(handle.id());
    }
    for handle in browser.images.drain(..) {
        images.remove(handle.id());
    }
    let mut textures = BTreeMap::new();
    let mut texture_labels = Vec::new();
    for (name, image) in prepared.images {
        texture_labels.push(format!(
            "{}\n{}",
            name,
            browser.catalog.source_label(&name).unwrap_or_default()
        ));
        let handle = images.add(image.image);
        browser.images.push(handle.clone());
        textures.insert(name, handle);
    }
    let summary = format!(
        "МОДЕЛЬ {} / {}\n\n{}\n{}\n\n{}\n\nЧастей: {}\nВершин: {}\nТреугольников: {}\nНормалей восстановлено: {}\n\nТЕКСТУРЫ\n{}{}",
        browser.selected + 1,
        browser.names.len(),
        prepared.name,
        prepared.version,
        browser.catalog.source_label(&path).unwrap_or_default(),
        prepared.parts.len(),
        prepared.vertices,
        prepared.triangles,
        prepared.repaired_normals,
        texture_labels
            .iter()
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n\n"),
        if prepared.notes.is_empty() {
            String::new()
        } else {
            format!("\n\n{}", prepared.notes.join("\n"))
        }
    );
    **label = Text::new(summary);
    browser.focus = prepared.focus;
    for part in prepared.parts {
        let texture = part.texture.and_then(|name| textures.get(&name).cloned());
        let material = materials.add(StandardMaterial {
            base_color: if texture.is_some() {
                Color::WHITE
            } else {
                Color::srgb(0.85, 0.28, 0.65)
            },
            base_color_texture: texture,
            alpha_mode: part.alpha_mode,
            perceptual_roughness: 0.9,
            cull_mode: None,
            ..default()
        });
        let mesh = meshes.add(part.mesh);
        browser.meshes.push(mesh.clone());
        browser.materials.push(material.clone());
        commands.spawn((
            Name::new(part.name),
            ModelPart,
            Mesh3d(mesh),
            MeshMaterial3d(material),
            Transform::default(),
        ));
    }
}
fn orbit_camera(
    orbit: Res<Orbit>,
    browser: Res<Browser>,
    mut camera: Single<&mut Transform, With<PreviewCamera>>,
) {
    let right = Vec3::new(orbit.yaw.cos(), 0.0, -orbit.yaw.sin());
    // Центр модели зависит от её высоты; сдвиг оставляет место панели слева.
    let target = browser.focus - right * (orbit.distance * 0.15);
    let direction = Vec3::new(
        orbit.yaw.sin() * orbit.pitch.cos(),
        orbit.pitch.sin(),
        orbit.yaw.cos() * orbit.pitch.cos(),
    );
    **camera = Transform::from_translation(target + direction * orbit.distance)
        .looking_at(target, Vec3::Y);
}
fn capture(mut commands: Commands, browser: Res<Browser>, mut frames: Local<u32>) {
    *frames += 1;
    if *frames == 120
        && let Some(path) = &browser.config.screenshot
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
