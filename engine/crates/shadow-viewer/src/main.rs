//! Первый Bevy-прототип: показываем настоящие изображения из ресурсов игры.
//! Здесь можно изучить App, Plugin, Resource, Component и системы ECS.
use bevy::{
    asset::RenderAssetUsages,
    image::{CompressedImageFormats, ImageSampler, ImageType},
    prelude::*,
    render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk},
    text::FontSize,
};
use shadow_formats::AssetCatalog;
use std::{env, error::Error, path::PathBuf};

struct Config {
    game_dir: PathBuf,
    overrides: Option<PathBuf>,
    initial_asset: Option<String>,
    screenshot: Option<PathBuf>,
}

/// Resource хранит общее состояние приложения. Его передают системам через Res.
#[derive(Resource)]
struct Browser {
    catalog: AssetCatalog,
    names: Vec<String>,
    selected: usize,
    dirty: bool,
    dimensions: Vec2,
    current_image: Option<Handle<Image>>,
    config: Config,
}

// Компоненты-маркеры позволяют найти нужную сущность через With<T>.
#[derive(Component)]
struct Preview;
#[derive(Component)]
struct SelectionLabel;

struct ResourceBrowserPlugin;

impl Plugin for ResourceBrowserPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup).add_systems(
            Update,
            (keyboard, show_preview, fit_preview, capture_preview).chain(),
        );
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Ошибка: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let Some(config) = parse_config()? else {
        return Ok(());
    };
    let catalog = AssetCatalog::open(&config.game_dir, config.overrides.as_deref())?;
    let names = image_names(&catalog);
    if names.is_empty() {
        return Err("в ресурсах не найдены поддерживаемые изображения".into());
    }
    let preferred = config
        .initial_asset
        .as_deref()
        .unwrap_or("textures/interface/loadscrn.jpg");
    let normalized = shadow_formats::normalize_path(preferred)?;
    let selected = if config.initial_asset.is_some() {
        names
            .iter()
            .position(|name| name == &normalized)
            .ok_or("изображение --asset не найдено")?
    } else {
        names
            .iter()
            .position(|name| name == &normalized)
            .unwrap_or(0)
    };
    App::new()
        .insert_resource(ClearColor(Color::srgb(0.035, 0.048, 0.045)))
        .insert_resource(Browser {
            catalog,
            names,
            selected,
            dirty: true,
            dimensions: Vec2::ZERO,
            current_image: None,
            config,
        })
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Shadow Company · Rust + Bevy".into(),
                resolution: (1280, 800).into(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(ResourceBrowserPlugin)
        .run();
    Ok(())
}

fn image_names(catalog: &AssetCatalog) -> Vec<String> {
    catalog
        .paths()
        .filter(|name| {
            matches!(
                name.rsplit('.').next(),
                Some("bmp" | "jpg" | "jpeg" | "png")
            )
        })
        .map(str::to_owned)
        .collect()
}

fn parse_config() -> Result<Option<Config>, Box<dyn Error>> {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--help") {
        println!(
            "shadow-viewer --game-dir PATH [--overrides PATH] [--asset NAME] [--screenshot FILE.png]\n←/→: изображение; R: перечитать каталог; Esc: выход. --screenshot сохраняет кадр и завершает приложение."
        );
        return Ok(None);
    }
    let mut game_dir = env::var_os("SHADOW_GAME_DIR").map(PathBuf::from);
    let mut config = Config {
        game_dir: PathBuf::new(),
        overrides: None,
        initial_asset: None,
        screenshot: None,
    };
    let mut index = 0;
    while index < args.len() {
        let value = args
            .get(index + 1)
            .filter(|value| !value.starts_with("--"))
            .ok_or_else(|| format!("после {} нужно значение", args[index]))?;
        match args[index].as_str() {
            "--game-dir" => game_dir = Some(value.into()),
            "--overrides" => config.overrides = Some(value.into()),
            "--asset" => config.initial_asset = Some(value.clone()),
            "--screenshot" => config.screenshot = Some(value.into()),
            unknown => return Err(format!("неизвестный аргумент {unknown}").into()),
        }
        index += 2;
    }
    config.game_dir = game_dir.ok_or("укажи --game-dir или переменную SHADOW_GAME_DIR")?;
    if let Some(path) = &config.screenshot {
        if path
            .extension()
            .is_none_or(|extension| !extension.eq_ignore_ascii_case("png"))
        {
            return Err("снимок нужно сохранять в файл .png".into());
        }
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)?;
        }
    }
    Ok(Some(config))
}

fn setup(mut commands: Commands, mut fonts: ResMut<Assets<Font>>) {
    commands.spawn(Camera2d);
    commands.spawn((Sprite::default(), Transform::default(), Preview));
    let font = fonts.add(Font::from_bytes(
        include_bytes!("../assets/fonts/DejaVuSans.ttf").to_vec(),
    ));
    let labels = [
        ("SHADOW COMPANY", 30.0, 30.0, 30.0),
        (
            "Rust + Bevy  ·  просмотр ресурсов оригинала",
            17.0,
            30.0,
            76.0,
        ),
    ];
    for (text, size, left, top) in labels {
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
                left: px(left),
                top: px(top),
                ..default()
            },
        ));
    }
    commands.spawn((
        Text::default(),
        SelectionLabel,
        TextFont {
            font: font.clone().into(),
            font_size: FontSize::Px(16.0),
            ..default()
        },
        TextColor(Color::srgb(0.76, 0.81, 0.77)),
        Node {
            position_type: PositionType::Absolute,
            left: px(30),
            top: px(133),
            width: px(280),
            ..default()
        },
    ));
    commands.spawn((
        Text::new("← / →  —  выбрать текстуру      R  —  перечитать данные      Esc  —  выйти"),
        TextFont {
            font: font.into(),
            font_size: FontSize::Px(16.0),
            ..default()
        },
        TextColor(Color::srgb(0.65, 0.71, 0.67)),
        Node {
            position_type: PositionType::Absolute,
            left: px(30),
            bottom: px(28),
            ..default()
        },
    ));
}

fn keyboard(
    input: Res<ButtonInput<KeyCode>>,
    mut browser: ResMut<Browser>,
    mut exit: MessageWriter<AppExit>,
) {
    if input.just_pressed(KeyCode::Escape) {
        exit.write(AppExit::Success);
    }
    if input.just_pressed(KeyCode::ArrowRight) {
        browser.selected = (browser.selected + 1) % browser.names.len();
        browser.dirty = true;
    }
    if input.just_pressed(KeyCode::ArrowLeft) {
        browser.selected = (browser.selected + browser.names.len() - 1) % browser.names.len();
        browser.dirty = true;
    }
    if input.just_pressed(KeyCode::KeyR) {
        match AssetCatalog::open(
            &browser.config.game_dir,
            browser.config.overrides.as_deref(),
        ) {
            Ok(catalog) => {
                let selected_name = browser.names[browser.selected].clone();
                let names = image_names(&catalog);
                if names.is_empty() {
                    error!("В обновлённом каталоге нет изображений");
                    return;
                }
                browser.selected = names
                    .iter()
                    .position(|name| name == &selected_name)
                    .unwrap_or(0);
                browser.names = names;
                browser.catalog = catalog;
                browser.dirty = true;
            }
            Err(error) => error!("Не удалось перечитать каталог: {error}"),
        }
    }
}

fn show_preview(
    mut browser: ResMut<Browser>,
    mut images: ResMut<Assets<Image>>,
    mut sprite: Single<&mut Sprite, With<Preview>>,
    mut label: Single<&mut Text, With<SelectionLabel>>,
) {
    if !browser.dirty {
        return;
    }
    browser.dirty = false;
    let name = browser.names[browser.selected].clone();
    let decoded = (|| -> Result<Image, Box<dyn Error>> {
        let bytes = browser.catalog.read(&name)?;
        let extension = name
            .rsplit('.')
            .next()
            .ok_or("нет расширения изображения")?;
        Ok(Image::from_buffer(
            &bytes,
            ImageType::Extension(extension),
            CompressedImageFormats::NONE,
            true,
            ImageSampler::linear(),
            RenderAssetUsages::default(),
        )?)
    })();
    match decoded {
        Ok(image) => {
            browser.dimensions = Vec2::new(image.width() as f32, image.height() as f32);
            let handle = images.add(image);
            sprite.image = handle.clone();
            sprite.color = Color::WHITE;
            if let Some(previous) = browser.current_image.replace(handle) {
                images.remove(previous.id());
            }
            **label = Text::new(format!(
                "ИЗОБРАЖЕНИЕ {} / {}\n\n{}\n\nИсточник\n{}\n\nРазмер\n{} × {}\n\nАрхивов: {}\n\nДоступно\n3D-модели SMF в отдельном просмотрщике\n\nСледующий этап\nАнимации .bmf\nКарты .mtf",
                browser.selected + 1,
                browser.names.len(),
                name,
                browser.catalog.source_label(&name).unwrap_or_default(),
                browser.dimensions.x as u32,
                browser.dimensions.y as u32,
                browser.catalog.archives.len()
            ));
        }
        Err(error) => {
            sprite.color = Color::NONE;
            **label = Text::new(format!(
                "{name}\n\nОшибка загрузки:\n{error}\n\n→ — следующее изображение"
            ));
        }
    }
}

fn fit_preview(
    browser: Res<Browser>,
    window: Single<&Window>,
    mut preview: Single<(&mut Sprite, &mut Transform), With<Preview>>,
) {
    if browser.dimensions.min_element() <= 0.0 {
        return;
    }
    let available = Vec2::new(
        (window.width() - 400.0).max(1.0),
        (window.height() - 190.0).max(1.0),
    );
    let scale = (available / browser.dimensions).min_element();
    let (sprite, transform) = &mut *preview;
    sprite.custom_size = Some(browser.dimensions * scale);
    transform.translation = Vec3::new(165.0, -20.0, 0.0);
}

fn capture_preview(mut commands: Commands, browser: Res<Browser>, mut frames: Local<u32>) {
    *frames += 1;
    if *frames == 60
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
