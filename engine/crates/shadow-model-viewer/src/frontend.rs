//! Меню оригинала, современный текст и переходы в учебную сцену.
use crate::ui_controls::{ButtonVisualState, UiButton};
use crate::{
    asset_mesh, menu_motion, movie, player,
    preferences::{self, Preferences},
    training,
};
use bevy::{
    picking::Pickable,
    prelude::*,
    render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk},
    tasks::{AsyncComputeTaskPool, Task, block_on, poll_once},
    text::FontSize,
    window::{MonitorSelection, WindowMode, WindowResizeConstraints},
};
use shadow_formats::{AssetCatalog, MenuLayout};
use std::{collections::HashSet, error::Error, path::PathBuf, sync::Arc, time::Duration};

#[path = "localization_smoke.rs"]
mod locale_smoke;
#[path = "menu_pages.rs"]
mod pages;

const MENU_PATH: &str = "config/window_bases/main_menu.txt";
const RESOLUTIONS: &[[u32; 2]] = &[
    [1280, 720],
    [1366, 768],
    [1600, 900],
    [1920, 1080],
    [2560, 1440],
    [3840, 2160],
    [5120, 2880],
    [7680, 4320],
    [2560, 1080],
    [3440, 1440],
    [3840, 1600],
    [5120, 2160],
    [7680, 3240],
];
#[derive(States, Default, Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Screen {
    #[default]
    Main,
    Options,
    Loading,
    Training,
    Pause,
    Movie,
    NewGame,
    LoadGame,
    Multiplayer,
    Quit,
}
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Training,
    Options,
    Exit,
    Language,
    Window,
    Resolution,
    TextSize,
    Background,
    Apply,
    Back,
    Resume,
    Main,
    Pause,
    Notice,
    Intro,
    NewGame,
    LoadGame,
    Multiplayer,
    ConfirmExit,
    DisplayTab,
    AudioTab,
    MouseTab,
    KeyboardTab,
    MasterVolume,
    CameraSpeed,
    EdgeScroll,
    BriefingTab,
    TeamTab,
    EquipmentTab,
    PreviousMission,
    NextMission,
    RefreshSaves,
}
#[derive(Message)]
struct ActionRequest(Action);
#[derive(Resource)]
struct Context {
    catalog: Arc<AssetCatalog>,
    layout: MenuLayout,
    settings_path: PathBuf,
    saved: Preferences,
    draft: Preferences,
    notice: String,
    frame: usize,
    options_return: Screen,
}
#[derive(Resource)]
struct MenuAssets {
    backgrounds: Vec<Handle<Image>>,
    bullet_size: UVec2,
    font: Handle<Font>,
    title_font: Handle<Font>,
}
#[derive(Resource, Default)]
struct Focus(Option<Action>, bool); // true: выбор мышью, гаснет после ухода указателя.
#[derive(Resource)]
struct Loading(Task<Result<training::Session, String>>);
#[derive(Component)]
struct MenuCamera;
#[derive(Component)]
struct Canvas;
#[derive(Component)]
struct Background(bool); // нижнее изображение и верхний слой перехода.
#[derive(Component)]
struct TitleGhost;
#[derive(Component)]
struct TitleBlur(Vec2, f32);
#[derive(Component)]
struct TitleOutline(Vec2);
#[derive(Component)]
struct ButtonCaption;
#[derive(Component)]
struct BrassRing(Color);
#[derive(Component)]
struct TitleBullet;
#[derive(Resource, Default)]
struct AnimationPreview {
    seconds: Option<f64>,
    hover: Option<(Action, f32)>,
}
#[derive(Component)]
struct Choice {
    action: Option<Action>,
    order: usize,
}
#[derive(Component)]
struct Placement {
    position: Vec2,
    size: Vec2,
}
#[derive(Component, Clone, Copy, Default)]
enum Anchor {
    #[default]
    Left,
    Right,
    Center,
    Stretch,
}
#[derive(Component)]
struct Caption {
    ru: String,
    en: String,
    size: f32,
    action: Option<Action>,
}
#[derive(Resource)]
struct Capture {
    target: Screen,
    path: Option<PathBuf>,
    frames: u32,
    boot_pause: bool,
    fixed_frame: Option<usize>,
    ignore_input: bool,
}
struct Config {
    game: PathBuf,
    overrides: Option<PathBuf>,
    settings: PathBuf,
    size: Option<[u32; 2]>,
    screenshot: Option<PathBuf>,
    report: Option<PathBuf>,
    smoke: Option<PathBuf>,
    screen: Screen,
    frame: Option<usize>,
    english: Option<bool>,
    movie: Option<String>,
    movie_time: Option<Duration>,
    movie_report: Option<PathBuf>,
    movie_smoke: Option<PathBuf>,
    evacuation_smoke: Option<PathBuf>,
    preview: AnimationPreview,
    options_tab: usize,
    campaign_tab: usize,
    locale_smoke: Option<PathBuf>,
}

pub(crate) fn run() -> Result<(), Box<dyn Error>> {
    let Some(config) = config()? else {
        return Ok(());
    };
    let catalog = Arc::new(AssetCatalog::open(
        &config.game,
        config.overrides.as_deref(),
    )?);
    if let Some(path) = &config.movie_report {
        return movie::write_report(
            &catalog,
            config.movie.as_deref().unwrap_or("movies/intro.avi"),
            path,
        );
    }
    let layout = MenuLayout::from_bytes(&catalog.read(MENU_PATH)?)?;
    let (mut saved, notice) = match Preferences::load(&config.settings) {
        Ok(p) => (p, String::new()),
        Err(e) => (
            Preferences::default(),
            format!("Настройки не прочитаны: {e}"),
        ),
    };
    if let Some(english) = config.english {
        saved.english = english;
    }
    if let Some(size) = config.size {
        saved.window_size = size;
    }
    let backgrounds = layout
        .backgrounds
        .iter()
        .map(|p| asset_mesh::image(&catalog, p))
        .collect::<Result<Vec<_>, _>>()?;
    let mut bullet = asset_mesh::image(&catalog, menu_motion::BULLET_PATH)?.image;
    // UI-атлас не повторяется за границами, в отличие от мировых материалов.
    bullet.sampler = bevy::image::ImageSampler::linear();
    let bullet_size = bullet.size();
    if bullet_size.x < 2 || bullet_size.y < 2 {
        return Err("атлас значков меню должен содержать четыре клетки".into());
    }
    if let Some(path) = &config.report {
        let report = serde_json::json!({"menu":MENU_PATH,"source":catalog.source_label(MENU_PATH)?,"base_size":layout.size,"text_offset":layout.text_offset,"shadow_offset":layout.shadow_offset,"animation":{"background_step_seconds":menu_motion::BACKGROUND_STEP,"order":"frame1 → frame5 → frame1","hover_enter_seconds":menu_motion::HOVER_ENTER,"hover_leave_seconds":menu_motion::HOVER_LEAVE,"bullet":{"path":menu_motion::BULLET_PATH,"source":catalog.source_label(menu_motion::BULLET_PATH)?,"size":bullet_size.to_array()}},"buttons":layout.buttons.iter().map(|b|serde_json::json!({"id":b.id,"position":b.position,"enabled":main_action(&b.id).is_some()})).collect::<Vec<_>>(),"backgrounds":layout.backgrounds.iter().zip(&backgrounds).map(|(p,t)|serde_json::json!({"path":p,"source":catalog.source_label(p).unwrap_or_default(),"size":[t.image.texture_descriptor.size.width,t.image.texture_descriptor.size.height]})).collect::<Vec<_>>(),"fonts":{"body":"DejaVu Sans (Unicode)","titles":"Russo One (Unicode, OFL)","outline":"native UI geometry"},"preferences":saved.json()});
        std::fs::write(path, serde_json::to_string_pretty(&report)? + "\n")?;
        return Ok(());
    }
    let start = match config.screen {
        Screen::Training | Screen::Pause => Screen::Loading,
        s => s,
    };
    let mut app = App::new();
    let first_frame = config.frame.unwrap_or(layout.backgrounds.len() - 1);
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "Shadow Company: Left for Dead".into(),
            resolution: saved.window_size.into(),
            resize_constraints: WindowResizeConstraints {
                min_width: 1280.0,
                min_height: 720.0,
                ..default()
            },
            mode: window_mode(saved.fullscreen),
            ..default()
        }),
        ..default()
    }))
    .insert_state(start)
    .insert_resource(ClearColor(Color::BLACK))
    .insert_resource(Context {
        catalog,
        layout,
        settings_path: config.settings,
        saved: saved.clone(),
        draft: saved.clone(),
        notice,
        frame: first_frame,
        options_return: Screen::Main,
    })
    .init_resource::<Focus>()
    .insert_resource(pages::Navigation {
        options: config.options_tab,
        campaign: config.campaign_tab,
        ..default()
    })
    .insert_resource(saved.clone())
    .insert_resource(crate::localization::Language::from_english(saved.english))
    .add_plugins(crate::localization::LocalizationPlugin)
    .insert_resource(training::CameraOptions {
        speed: saved.camera_speed,
        edge_scroll: saved.edge_scroll,
    })
    .insert_resource(config.preview)
    .insert_resource(Capture {
        target: config.screen,
        path: config.screenshot.clone(),
        frames: 0,
        boot_pause: config.screen == Screen::Pause,
        fixed_frame: config.frame,
        ignore_input: config.screenshot.is_some()
            || config.smoke.is_some()
            || config.movie_smoke.is_some()
            || config.evacuation_smoke.is_some()
            || config.locale_smoke.is_some(),
    })
    .add_message::<ActionRequest>();
    movie::install(&mut app);
    if config.screen == Screen::Movie {
        app.insert_resource(movie::Request {
            catalog: app.world().resource::<Context>().catalog.clone(),
            path: config
                .movie
                .clone()
                .unwrap_or_else(|| "movies/intro.avi".into()),
            snapshot: config.movie_time,
        });
    }
    let font = Font::from_bytes(
        include_bytes!("../../shadow-viewer/assets/fonts/DejaVuSans.ttf").to_vec(),
    );
    let font = app.world_mut().resource_mut::<Assets<Font>>().add(font);
    let title_font = app
        .world_mut()
        .resource_mut::<Assets<Font>>()
        .add(Font::from_bytes(
            include_bytes!("../../shadow-viewer/assets/fonts/RussoOne-Regular.ttf").to_vec(),
        ));
    let backgrounds = backgrounds
        .into_iter()
        .map(|t| app.world_mut().resource_mut::<Assets<Image>>().add(t.image))
        .collect();
    app.insert_resource(MenuAssets {
        font,
        backgrounds,
        bullet_size,
        title_font,
    });
    // Начальный OnEnter исполняется до PreStartup. Камера нужна уже здесь.
    install_menu_camera(&mut app);
    add_screens(&mut app);
    app.add_systems(Last, crate::hud::release_keys);
    app.add_systems(
        Update,
        (
            crate::ui_controls::sync,
            suppress_captured_input,
            keyboard,
            movie_click,
            mouse_actions,
            handle_actions,
            sync_language,
            pages::refresh,
            pages::apply_audio,
            pages::scroll_text,
            poll_loading,
            movie::update.run_if(in_state(Screen::Movie)),
            movie_finished,
            apply_camera,
            animate_background,
            fit_ui,
            movie::fit.run_if(in_state(Screen::Movie)),
            update_captions,
            style_buttons,
            capture,
        )
            .chain(),
    )
    .add_systems(
        Update,
        crate::tutorial::input
            .run_if(in_state(Screen::Training))
            .before(keyboard)
            .after(suppress_captured_input),
    )
    .add_systems(
        Update,
        (
            crate::selection::input,
            crate::hud::input,
            crate::roster::input,
        )
            .chain()
            .run_if(in_state(Screen::Training))
            .run_if(training_accepts_input)
            .after(crate::tutorial::input)
            .before(crate::inventory::input)
            .before(keyboard),
    )
    .add_systems(
        Update,
        crate::inventory::input
            .run_if(crate::vehicle::on_foot_or_reset)
            .run_if(crate::party::john_controls)
            .run_if(in_state(Screen::Training))
            .run_if(training_accepts_input)
            .after(crate::tutorial::input)
            .before(keyboard),
    )
    .add_systems(
        Update,
        (
            (
                crate::evacuation::input,
                crate::party::input,
                crate::vehicle::input,
                (
                    crate::grenade::input,
                    crate::fence::input,
                    crate::smg::input,
                    crate::medical::input,
                    crate::armor::input,
                    crate::stance::input,
                    crate::pickup::input,
                    crate::combat::input,
                )
                    .chain()
                    .run_if(crate::vehicle::on_foot_or_reset)
                    .run_if(crate::party::john_controls),
                training::controls,
                training::issue_orders,
            )
                .chain()
                .run_if(training_accepts_input),
            training::update_player,
            (crate::gate::tick, crate::vehicle::tick).chain(),
            crate::pickup::sync,
            crate::combat::traces,
            crate::combat::sync,
            crate::inventory::sync,
            crate::grenade::sync,
            crate::fence::sync,
            crate::smg::sync,
            crate::medical::sync,
            crate::armor::sync,
            crate::medical::hud,
            (
                crate::tutorial::advance,
                crate::party::tick,
                crate::party::audio,
                crate::evacuation::advance,
                crate::evacuation::observe_end,
            )
                .chain(),
            crate::tutorial::ui,
            crate::inventory::ui,
            crate::pickup::ring,
            training::update_markers,
            (
                crate::vehicle::visuals,
                crate::gate::visuals,
                crate::party::visuals,
                crate::hud::visuals,
                crate::roster::visuals,
                crate::selection::visuals,
                crate::evacuation::visuals,
            )
                .chain(),
            (crate::hud::diagnostics, training::orbit_camera).chain(),
        )
            .chain()
            .run_if(in_state(Screen::Training))
            .after(handle_actions),
    );
    app.add_systems(
        Update,
        training_finished
            .run_if(in_state(Screen::Training))
            .after(crate::evacuation::observe_end),
    );
    if let Some(path) = config.evacuation_smoke {
        app.insert_resource(crate::evacuation::NativeSmoke::new(path, None, true))
            .add_systems(
                Update,
                crate::evacuation::native_smoke
                    .run_if(in_state(Screen::Training))
                    .before(crate::tutorial::input)
                    .after(suppress_captured_input),
            )
            .add_systems(Update, evacuation_smoke_result.after(training_finished));
    }
    if let Some(path) = config.locale_smoke {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        app.insert_resource(locale_smoke::Smoke::new(path))
            .add_systems(Last, locale_smoke::run);
    }
    if let Some(path) = config.smoke {
        app.insert_resource(Smoke::new(path))
            .add_systems(Update, smoke.after(capture));
    }
    if let Some(path) = config.movie_smoke {
        app.insert_resource(MovieSmoke::new(
            path,
            config.movie.unwrap_or_else(|| "movies/intro.avi".into()),
        ))
        .add_systems(Update, movie_smoke.after(capture));
    }
    if !app.run().is_success() {
        return Err("приложение завершилось с ошибкой; подробности в журнале".into());
    }
    Ok(())
}
fn add_screens(app: &mut App) {
    app.add_systems(
        OnEnter(Screen::Main),
        (training::cleanup, reset_input, spawn_main).chain(),
    )
    .add_systems(
        OnEnter(Screen::Options),
        (reset_input, crate::selection::cancel, spawn_options).chain(),
    )
    .add_systems(
        OnEnter(Screen::NewGame),
        (reset_input, pages::enter).chain(),
    )
    .add_systems(
        OnEnter(Screen::LoadGame),
        (reset_input, pages::enter).chain(),
    )
    .add_systems(
        OnEnter(Screen::Multiplayer),
        (reset_input, pages::enter).chain(),
    )
    .add_systems(OnEnter(Screen::Quit), (reset_input, pages::enter).chain())
    .add_systems(
        OnEnter(Screen::Loading),
        (reset_input, spawn_loading, start_loading).chain(),
    )
    .add_systems(
        OnEnter(Screen::Training),
        (
            reset_input,
            (
                training::setup,
                player::setup,
                crate::combat::setup,
                crate::inventory::setup,
                crate::grenade::setup,
                crate::fence::setup,
                crate::smg::setup,
                crate::medical::setup,
                crate::armor::setup,
                crate::vehicle::setup,
                crate::party::setup,
                crate::hud::setup,
                crate::roster::setup,
                crate::selection::setup,
                crate::evacuation::setup,
                training::finish_setup,
            )
                .chain()
                .run_if(resource_exists::<training::SessionReady>),
        )
            .chain(),
    )
    .add_systems(
        Update,
        (
            crate::party::pause_audio.run_if(in_state(Screen::Pause)),
            crate::party::pause_audio.run_if(in_state(Screen::Options)),
        ),
    )
    .add_systems(
        OnEnter(Screen::Pause),
        (
            reset_input,
            crate::selection::cancel,
            crate::party::pause_audio,
            spawn_pause,
        )
            .chain(),
    )
    .add_systems(
        OnEnter(Screen::Movie),
        (reset_input, spawn_movie, movie::start).chain(),
    )
    .add_systems(OnExit(Screen::Training), crate::medical::reset_cursor)
    .add_systems(OnExit(Screen::Movie), movie::cleanup);
}
fn window_mode(fullscreen: bool) -> WindowMode {
    if fullscreen {
        WindowMode::BorderlessFullscreen(MonitorSelection::Current)
    } else {
        WindowMode::Windowed
    }
}
fn spawn_movie(
    mut commands: Commands,
    context: Res<Context>,
    assets: Res<MenuAssets>,
    camera: Single<Entity, With<MenuCamera>>,
    mut focus: ResMut<Focus>,
) {
    focus.0 = None;
    let parent = root(
        &mut commands,
        Screen::Movie,
        *camera,
        &assets,
        context.frame,
        false,
    );
    commands.spawn((
        movie::Picture,
        Node {
            position_type: PositionType::Absolute,
            ..default()
        },
        ChildOf(parent),
    ));
    let w = context.layout.size[0];
    let label = text(
        &mut commands,
        parent,
        &assets,
        Vec2::new(24.0, 210.0),
        Vec2::new(w - 48.0, 40.0),
        "ЗАГРУЗКА РОЛИКА…",
        "LOADING MOVIE…",
        20.0,
        None,
    );
    commands
        .entity(label)
        .insert((Anchor::Stretch, movie::LoadingLabel));
    let hint = text(
        &mut commands,
        parent,
        &assets,
        Vec2::new(24.0, 454.0),
        Vec2::new(w - 48.0, 25.0),
        "Esc / Enter / пробел / левая кнопка — пропустить",
        "Esc / Enter / Space / left click — skip",
        10.0,
        None,
    );
    commands.entity(hint).insert(Anchor::Stretch);
}
fn movie_finished(
    mut messages: MessageReader<movie::Finished>,
    state: Res<State<Screen>>,
    mut context: ResMut<Context>,
    mut next: ResMut<NextState<Screen>>,
    capture: Res<Capture>,
    mut exit: MessageWriter<AppExit>,
) {
    for movie::Finished(error) in messages.read() {
        if *state.get() != Screen::Movie {
            continue;
        }
        context.notice = error
            .as_ref()
            .map(|e| format!("Ролик не воспроизведён: {e}"))
            .unwrap_or_default();
        if error.is_some() && capture.ignore_input {
            eprintln!("{}", context.notice);
            exit.write(AppExit::error());
        }
        next.set(Screen::Main);
    }
}
fn config() -> Result<Option<Config>, Box<dyn Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.iter().any(|a| a == "--help") {
        println!("--data PATH — совместимость с OpenSC Launcher 0.2.0.");
        println!(
            "shadow-game --game-dir PATH [--overrides PATH] [--settings FILE.json] [--language ru|en] [--window WIDTHxHEIGHT] [--screen main|options|new-game|load-game|multiplayer|quit|training|pause|movie --screenshot FILE.png] [--frame 1..5] [--menu-time SECONDS --menu-hover b_training:0..1 --screenshot FILE.png] [--menu-json FILE.json] [--smoke-json FILE.json] [--locale-smoke-json FILE.json]\nВидео: --movie movies/intro.avi; --movie-json FILE.json — сведения, PCM hash и три PNG; --movie-time SECONDS --screenshot FILE.png — неподвижный снимок; --movie-smoke-json FILE.json — пропуск, повтор и окончание короткого ролика.\nРазрешения от HD (1280x720), включая 16:9 и 21:9. Мышь или ↑/↓/Tab и Enter. Esc — назад/меню паузы, Ctrl+X — выход. Ролик: Esc/Enter/пробел/левая кнопка — пропустить. --player on сохраняет прямой запуск прежней учебной витрины."
        );
        return Ok(None);
    }
    let mut game = std::env::var_os("SHADOW_GAME_DIR").map(PathBuf::from);
    let mut config = Config {
        game: PathBuf::new(),
        overrides: None,
        settings: PathBuf::new(),
        size: None,
        screenshot: None,
        report: None,
        smoke: None,
        screen: Screen::Main,
        frame: None,
        english: None,
        movie: None,
        movie_time: None,
        movie_report: None,
        movie_smoke: None,
        evacuation_smoke: None,
        preview: AnimationPreview::default(),
        options_tab: 0,
        campaign_tab: 0,
        locale_smoke: None,
    };
    let mut seen = HashSet::new();
    for pair in args.chunks(2) {
        let key = if pair[0] == "--data" {
            "--game-dir"
        } else {
            pair[0].as_str()
        };
        if !seen.insert(key) {
            return Err(format!("повтор аргумента {key}").into());
        }
        let value = pair
            .get(1)
            .filter(|v| !v.starts_with("--"))
            .ok_or_else(|| format!("после {key} нужно значение"))?;
        match key {
            "--game-dir" => game = Some(value.into()),
            "--overrides" => config.overrides = Some(value.into()),
            "--settings" => config.settings = value.into(),
            "--screenshot" => config.screenshot = Some(value.into()),
            "--menu-json" => config.report = Some(value.into()),
            "--smoke-json" => config.smoke = Some(value.into()),
            "--locale-smoke-json" => config.locale_smoke = Some(value.into()),
            "--movie" => config.movie = Some(shadow_formats::normalize_path(value)?),
            "--movie-json" => config.movie_report = Some(value.into()),
            "--evacuation-smoke-json" => config.evacuation_smoke = Some(value.into()),
            "--movie-smoke-json" => config.movie_smoke = Some(value.into()),
            "--menu-time" => {
                let seconds: f64 = value.parse()?;
                if !seconds.is_finite() || !(0.0..=86400.0).contains(&seconds) {
                    return Err("неверное время анимации меню".into());
                }
                config.preview.seconds = Some(seconds);
            }
            "--menu-hover" => {
                let (id, progress) = value
                    .split_once(':')
                    .ok_or("--menu-hover: b_training:0.0..1.0")?;
                let action = main_action(id)
                    .ok_or("для --menu-hover нужна доступная кнопка главного меню")?;
                let progress: f32 = progress.parse()?;
                if !progress.is_finite() || !(0.0..=1.0).contains(&progress) {
                    return Err("фаза наведения должна быть 0..1".into());
                }
                config.preview.hover = Some((action, progress));
            }
            "--movie-time" => {
                let seconds: f64 = value.parse()?;
                if !seconds.is_finite() || !(0.0..=86400.0).contains(&seconds) {
                    return Err("неверное время ролика".into());
                }
                config.movie_time = Some(Duration::from_secs_f64(seconds));
            }
            "--language" => {
                config.english = Some(match value.as_str() {
                    "ru" => false,
                    "en" => true,
                    _ => return Err("язык должен быть ru или en".into()),
                })
            }
            "--screen" => {
                config.screen = match value.as_str() {
                    "main" => Screen::Main,
                    "options" => Screen::Options,
                    "training" => Screen::Training,
                    "pause" => Screen::Pause,
                    "movie" => Screen::Movie,
                    "new-game" => Screen::NewGame,
                    "load-game" => Screen::LoadGame,
                    "multiplayer" => Screen::Multiplayer,
                    "quit" => Screen::Quit,
                    _ => return Err("неверный --screen".into()),
                }
            }
            "--options-tab" => {
                config.options_tab = match value.as_str() {
                    "preferences" => 0,
                    "audio-video" => 1,
                    "mouse" => 2,
                    "keyboard" => 3,
                    _ => return Err("--options-tab: preferences|audio-video|mouse|keyboard".into()),
                }
            }
            "--campaign-tab" => {
                config.campaign_tab = match value.as_str() {
                    "planning" => 0,
                    "personnel" => 1,
                    "equipment" => 2,
                    _ => return Err("--campaign-tab: planning|personnel|equipment".into()),
                }
            }
            "--frame" => {
                let n = value.parse::<usize>()?;
                if !(1..=5).contains(&n) {
                    return Err("номер кадра должен быть 1..5".into());
                }
                config.frame = Some(5 - n);
            }
            "--window" => {
                let (w, h) = value
                    .split_once('x')
                    .ok_or("окно должно иметь формат WIDTHxHEIGHT")?;
                let size = [w.parse()?, h.parse()?];
                preferences::validate_window_size(size)?;
                config.size = Some(size);
            }
            _ => return Err(format!("неизвестный аргумент {key}").into()),
        }
    }
    config.game = game.ok_or("укажи --game-dir или SHADOW_GAME_DIR")?;
    if config.settings.as_os_str().is_empty() {
        config.settings = preferences::default_path()?;
    }
    if config.movie.is_some() || config.movie_smoke.is_some() {
        config.screen = Screen::Movie;
    }
    if config.movie_time.is_some()
        && (config.screen != Screen::Movie || config.screenshot.is_none())
    {
        return Err("--movie-time требует --movie и --screenshot".into());
    }
    if (config.preview.seconds.is_some() || config.preview.hover.is_some())
        && (config.screenshot.is_none() || !matches!(config.screen, Screen::Main | Screen::Options))
    {
        return Err("снимок анимации меню требует --screenshot и --screen main|options".into());
    }
    if config.preview.seconds.is_some() && config.frame.is_some() {
        return Err("выбери --menu-time или --frame".into());
    }
    if config.preview.hover.is_some() && config.screen != Screen::Main {
        return Err("--menu-hover относится к главному меню".into());
    }
    if config.screen == Screen::Movie && config.screenshot.is_some() && config.movie_time.is_none()
    {
        config.movie_time = Some(Duration::ZERO);
    }
    if usize::from(config.screenshot.is_some())
        + usize::from(config.report.is_some())
        + usize::from(config.smoke.is_some())
        + usize::from(config.movie_report.is_some())
        + usize::from(config.movie_smoke.is_some())
        + usize::from(config.evacuation_smoke.is_some())
        + usize::from(config.locale_smoke.is_some())
        > 1
    {
        return Err("выбери один выходной файл".into());
    }
    if config.locale_smoke.is_some() && config.screen != Screen::Main {
        return Err("--locale-smoke-json starts at --screen main".into());
    }
    if config.locale_smoke.is_some() {
        if !seen.contains("--settings") {
            return Err(
                "--locale-smoke-json requires --settings with an isolated test file".into(),
            );
        }
        config.english = Some(true);
    }
    Ok(Some(config))
}
fn install_menu_camera(app: &mut App) {
    app.world_mut().spawn((Camera2d, MenuCamera));
}
fn reset_input(
    mut keyboard: ResMut<ButtonInput<KeyCode>>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
) {
    keyboard.reset_all();
    mouse.reset_all();
}
fn suppress_captured_input(
    capture: Res<Capture>,
    keyboard: ResMut<ButtonInput<KeyCode>>,
    mouse: ResMut<ButtonInput<MouseButton>>,
) {
    if capture.ignore_input {
        reset_input(keyboard, mouse);
    }
}
fn training_accepts_input(
    capture: Res<Capture>,
    smoke: Option<Res<crate::evacuation::NativeSmoke>>,
) -> bool {
    !capture.ignore_input || smoke.is_some()
}
fn main_action(id: &str) -> Option<Action> {
    match id {
        "b_new_game" => Some(Action::NewGame),
        "b_load_game" => Some(Action::LoadGame),
        "b_multiplayer" => Some(Action::Multiplayer),
        "b_training" => Some(Action::Training),
        "b_options" => Some(Action::Options),
        "b_exit" => Some(Action::Exit),
        "b_intro" => Some(Action::Intro),
        _ => None,
    }
}
fn main_label(id: &str) -> (&str, &str) {
    match id {
        "b_new_game" => ("НОВАЯ ИГРА", "NEW GAME"),
        "b_load_game" => ("ЗАГРУЗИТЬ ИГРУ", "LOAD GAME"),
        "b_training" => ("ОБУЧЕНИЕ", "TRAINING"),
        "b_options" => ("НАСТРОЙКИ", "OPTIONS"),
        "b_intro" => ("ВСТУПЛЕНИЕ", "VIEW INTRO"),
        "b_multiplayer" => ("СЕТЕВАЯ ИГРА", "MULTIPLAYER"),
        "b_exit" => ("ВЫХОД", "EXIT"),
        _ => ("НЕИЗВЕСТНЫЙ РАЗДЕЛ", "UNKNOWN SECTION"),
    }
}
fn root(
    commands: &mut Commands,
    screen: Screen,
    camera: Entity,
    assets: &MenuAssets,
    frame: usize,
    background: bool,
) -> Entity {
    let id = commands
        .spawn((
            Canvas,
            DespawnOnExit(screen),
            UiTargetCamera(camera),
            Node {
                position_type: PositionType::Absolute,
                ..default()
            },
        ))
        .id();
    if background {
        for top in [false, true] {
            commands.spawn((
                Background(top),
                ImageNode {
                    color: Color::WHITE.with_alpha(if top { 0.0 } else { 1.0 }),
                    ..ImageNode::new(assets.backgrounds[frame % assets.backgrounds.len()].clone())
                },
                Node {
                    position_type: PositionType::Absolute,
                    width: percent(100),
                    height: percent(100),
                    ..default()
                },
                Pickable::IGNORE,
                ChildOf(id),
            ));
        }
    }
    id
}
#[allow(clippy::too_many_arguments)] // Координаты и два перевода одной надписи.
fn text(
    commands: &mut Commands,
    parent: Entity,
    assets: &MenuAssets,
    position: Vec2,
    size: Vec2,
    ru: &str,
    en: &str,
    font_size: f32,
    action: Option<Action>,
) -> Entity {
    commands
        .spawn((
            Text::new(ru),
            Caption {
                ru: ru.into(),
                en: en.into(),
                size: font_size,
                action,
            },
            TextFont {
                font: assets.font.clone().into(),
                font_size: FontSize::Px(font_size),
                ..default()
            },
            TextColor(Color::srgb(0.87, 0.87, 0.80)),
            Node {
                position_type: PositionType::Absolute,
                ..default()
            },
            Placement { position, size },
            Anchor::Left,
            ChildOf(parent),
        ))
        .id()
}
#[allow(clippy::too_many_arguments)]
fn button(
    commands: &mut Commands,
    parent: Entity,
    assets: &MenuAssets,
    position: Vec2,
    size: Vec2,
    offset: Vec2,
    ru: &str,
    en: &str,
    action: Option<Action>,
    order: usize,
    title: bool,
) -> Entity {
    let id = commands
        .spawn((
            Choice { action, order },
            Node {
                position_type: PositionType::Absolute,
                ..default()
            },
            Placement { position, size },
            Anchor::Left,
            BackgroundColor(Color::NONE),
            ChildOf(parent),
        ))
        .id();
    if action.is_some() {
        commands.entity(id).insert(UiButton);
    }
    let caption = text(
        commands,
        id,
        assets,
        offset,
        size - offset - Vec2::new(8.0, 0.0),
        ru,
        en,
        if title { 19.0 } else { 14.0 },
        action,
    );
    commands.entity(caption).insert(TextFont {
        font: assets.title_font.clone().into(),
        font_size: FontSize::Px(if title { 19.0 } else { 14.0 }),
        ..default()
    });
    commands.entity(caption).insert((
        ButtonCaption,
        ZIndex(1),
        Pickable::IGNORE,
        TextShadow {
            offset: Vec2::splat(1.0),
            color: Color::BLACK.with_alpha(0.8),
        },
    ));
    id
}
fn spawn_main(
    mut commands: Commands,
    mut context: ResMut<Context>,
    assets: Res<MenuAssets>,
    camera: Single<Entity, With<MenuCamera>>,
    mut focus: ResMut<Focus>,
) {
    commands.remove_resource::<Loading>();
    context.draft = context.saved.clone();
    focus.0 = None;
    focus.1 = false;
    let root = root(
        &mut commands,
        Screen::Main,
        *camera,
        &assets,
        context.frame,
        true,
    );
    for (order, b) in context.layout.buttons.iter().enumerate() {
        let (ru, en) = main_label(&b.id);
        let entity = button(
            &mut commands,
            root,
            &assets,
            Vec2::from_array(b.position),
            Vec2::new(context.layout.size[0] - b.position[0] - 10.0, 40.0),
            Vec2::from_array(context.layout.text_offset),
            ru,
            en,
            main_action(&b.id),
            order,
            true,
        );
        commands.entity(entity).insert(Anchor::Right);
        commands
            .entity(entity)
            .insert(menu_motion::Hover::default());
        // Новые надписи строятся из TTF, а не из низкого растрового атласа.
        // Восемь смещённых контуров дают исходную красную обводку при любом DPI.
        for direction in [
            (-1., -1.),
            (0., -1.),
            (1., -1.),
            (-1., 0.),
            (1., 0.),
            (-1., 1.),
            (0., 1.),
            (1., 1.),
        ] {
            let outline = text(
                &mut commands,
                entity,
                &assets,
                Vec2::from_array(context.layout.text_offset),
                Vec2::new(context.layout.size[0] - b.position[0] - 10.0, 40.0)
                    - Vec2::from_array(context.layout.text_offset)
                    - Vec2::new(8.0, 0.0),
                ru,
                en,
                19.0,
                main_action(&b.id),
            );
            commands.entity(outline).insert((
                TitleOutline(Vec2::new(direction.0, direction.1) * 1.2),
                ButtonCaption,
                TextFont {
                    font: assets.title_font.clone().into(),
                    font_size: FontSize::Px(19.0),
                    ..default()
                },
                ZIndex(0),
                Pickable::IGNORE,
            ));
        }
        // Мягкий ореол исходных кнопок пересобирается из Unicode-текста.
        // Слои не требуют размытого низкого атласа и сближаются при наведении.
        for (x, y, alpha) in [
            (0., 0., 0.09),
            (-1.4, 0., 0.07),
            (1.4, 0., 0.07),
            (0., -1.4, 0.07),
            (0., 1.4, 0.07),
            (-1.4, -1.4, 0.04),
            (-1.4, 1.4, 0.04),
            (1.4, -1.4, 0.04),
            (1.4, 1.4, 0.04),
        ] {
            let ghost = text(
                &mut commands,
                entity,
                &assets,
                Vec2::from_array(context.layout.shadow_offset),
                Vec2::new(context.layout.size[0] - b.position[0] - 10., 40.)
                    - Vec2::from_array(context.layout.text_offset)
                    - Vec2::new(8., 0.),
                ru,
                en,
                19.,
                main_action(&b.id),
            );
            commands.entity(ghost).insert((
                TitleGhost,
                TitleBlur(Vec2::new(x, y), alpha),
                ButtonCaption,
                Pickable::IGNORE,
                ZIndex(-1),
                TextFont {
                    font: assets.title_font.clone().into(),
                    font_size: FontSize::Px(19.),
                    ..default()
                },
            ));
        }
        // Латунное донце патрона: новые круги UI без масштабирования BMP.
        for (size, color) in [
            (23., Color::srgb(0.09, 0.08, 0.03)),
            (20., Color::srgb(0.55, 0.44, 0.19)),
            (17., Color::srgb(0.15, 0.16, 0.13)),
            (14., Color::srgb(0.72, 0.61, 0.32)),
            (11., Color::srgb(0.29, 0.27, 0.16)),
            (6., Color::srgb(0.87, 0.79, 0.49)),
        ] {
            commands.spawn((
                BrassRing(color),
                BackgroundColor(color),
                Node {
                    position_type: PositionType::Absolute,
                    border_radius: BorderRadius::MAX,
                    ..default()
                },
                Placement {
                    position: Vec2::new(13.0, 17.0) - Vec2::splat(size * 0.5),
                    size: Vec2::splat(size),
                },
                Anchor::Left,
                Pickable::IGNORE,
                ChildOf(entity),
            ));
        }
    }
    let w = context.layout.size[0];
    let footer = text(
        &mut commands,
        root,
        &assets,
        Vec2::new(22.0, 407.0),
        Vec2::new(w - 44.0, 45.0),
        "OpenSC · ТЕСТОВАЯ ВЕРСИЯ",
        "OpenSC · TEST BUILD",
        11.0,
        None,
    );
    commands.entity(footer).insert(Anchor::Stretch);
    let notice = text(
        &mut commands,
        root,
        &assets,
        Vec2::new(22.0, 450.0),
        Vec2::new(w - 44.0, 25.0),
        &context.notice,
        &context.notice,
        10.0,
        Some(Action::Notice),
    );
    commands.entity(notice).insert(Anchor::Stretch);
}
fn spawn_options(
    mut commands: Commands,
    context: Res<Context>,
    assets: Res<MenuAssets>,
    camera: Single<Entity, With<MenuCamera>>,
    mut focus: ResMut<Focus>,
    mut navigation: ResMut<pages::Navigation>,
) {
    let root = root(
        &mut commands,
        Screen::Options,
        *camera,
        &assets,
        context.frame,
        true,
    );
    navigation.dirty = false;
    pages::draw(
        &mut commands,
        root,
        Screen::Options,
        &context,
        &assets,
        &navigation,
        &mut focus,
    );
}
fn spawn_loading(
    mut commands: Commands,
    context: Res<Context>,
    assets: Res<MenuAssets>,
    camera: Single<Entity, With<MenuCamera>>,
    mut focus: ResMut<Focus>,
) {
    focus.0 = None;
    let root = root(
        &mut commands,
        Screen::Loading,
        *camera,
        &assets,
        context.frame,
        true,
    );
    pages::draw(
        &mut commands,
        root,
        Screen::Loading,
        &context,
        &assets,
        &pages::Navigation::default(),
        &mut focus,
    );
}
fn spawn_pause(
    mut commands: Commands,
    context: Res<Context>,
    assets: Res<MenuAssets>,
    camera: Single<Entity, With<training::MapCamera>>,
    mut focus: ResMut<Focus>,
) {
    focus.0 = Some(Action::Resume);
    focus.1 = false;
    let root = root(
        &mut commands,
        Screen::Pause,
        *camera,
        &assets,
        context.frame,
        false,
    );
    pages::draw(
        &mut commands,
        root,
        Screen::Pause,
        &context,
        &assets,
        &pages::Navigation::default(),
        &mut focus,
    );
}
fn start_loading(mut commands: Commands, context: Res<Context>) {
    let catalog = context.catalog.clone();
    commands.insert_resource(Loading(AsyncComputeTaskPool::get().spawn(async move {
        training::prepare_session(&catalog).map_err(|e| e.to_string())
    })));
}
fn poll_loading(
    mut commands: Commands,
    loading: Option<ResMut<Loading>>,
    mut next: ResMut<NextState<Screen>>,
    mut context: ResMut<Context>,
    state: Res<State<Screen>>,
) {
    if *state.get() != Screen::Loading {
        return;
    }
    let Some(mut task) = loading else {
        return;
    };
    if let Some(result) = block_on(poll_once(&mut task.0)) {
        commands.remove_resource::<Loading>();
        match result {
            Ok(session) => {
                context.notice.clear();
                session.install(&mut commands);
                next.set(Screen::Training);
            }
            Err(error) => {
                context.notice = format!("Не удалось загрузить обучение: {error}");
                next.set(Screen::Main);
            }
        }
    }
}
fn ordered_actions(choices: impl Iterator<Item = (usize, Option<Action>)>) -> Vec<Action> {
    let mut choices = choices
        .filter_map(|(i, a)| a.map(|a| (i, a)))
        .collect::<Vec<_>>();
    choices.sort_by_key(|c| c.0);
    choices.into_iter().map(|c| c.1).collect()
}
fn move_focus(actions: &[Action], current: Option<Action>, backwards: bool) -> Option<Action> {
    if actions.is_empty() {
        return None;
    }
    let current = current.and_then(|a| actions.iter().position(|v| *v == a));
    let index = match current {
        None => {
            if backwards {
                actions.len() - 1
            } else {
                0
            }
        }
        Some(i) => {
            if backwards {
                (i + actions.len() - 1) % actions.len()
            } else {
                (i + 1) % actions.len()
            }
        }
    };
    Some(actions[index])
}
fn keyboard(
    input: Res<ButtonInput<KeyCode>>,
    state: Res<State<Screen>>,
    capture: Res<Capture>,
    choices: Query<&Choice>,
    mut focus: ResMut<Focus>,
    mut requests: MessageWriter<ActionRequest>,
) {
    if capture.ignore_input {
        return;
    }
    let ctrl = input.pressed(KeyCode::ControlLeft) || input.pressed(KeyCode::ControlRight);
    if ctrl && input.just_pressed(KeyCode::KeyX) {
        requests.write(ActionRequest(Action::ConfirmExit));
        return;
    }
    if input.just_pressed(KeyCode::Escape) {
        requests.write(ActionRequest(match state.get() {
            Screen::Main => Action::Exit,
            Screen::Training => Action::Pause,
            Screen::Pause => Action::Resume,
            _ => Action::Back,
        }));
        return;
    }
    if *state.get() == Screen::Movie {
        if input.just_pressed(KeyCode::Enter) || input.just_pressed(KeyCode::Space) {
            requests.write(ActionRequest(Action::Back));
        }
        return;
    }
    if *state.get() == Screen::Training {
        return;
    }
    if input.just_pressed(KeyCode::ArrowDown)
        || input.just_pressed(KeyCode::Tab)
        || input.just_pressed(KeyCode::ArrowUp)
    {
        let backward = input.just_pressed(KeyCode::ArrowUp)
            || (input.just_pressed(KeyCode::Tab)
                && (input.pressed(KeyCode::ShiftLeft) || input.pressed(KeyCode::ShiftRight)));
        let actions = ordered_actions(choices.iter().map(|c| (c.order, c.action)));
        focus.0 = move_focus(&actions, focus.0, backward);
        focus.1 = false;
    }
    if (input.just_pressed(KeyCode::Enter) || input.just_pressed(KeyCode::Space))
        && let Some(action) = focus.0
    {
        requests.write(ActionRequest(action));
    }
}
fn movie_click(
    mouse: Res<ButtonInput<MouseButton>>,
    capture: Res<Capture>,
    state: Res<State<Screen>>,
    mut requests: MessageWriter<ActionRequest>,
) {
    if !capture.ignore_input
        && *state.get() == Screen::Movie
        && mouse.just_pressed(MouseButton::Left)
    {
        requests.write(ActionRequest(Action::Back));
    }
}
fn mouse_actions(
    mouse: Res<ButtonInput<MouseButton>>,
    capture: Res<Capture>,
    choices: Query<(&Choice, &ButtonVisualState), Changed<ButtonVisualState>>,
    mut focus: ResMut<Focus>,
    mut requests: MessageWriter<ActionRequest>,
) {
    if capture.ignore_input {
        return;
    }
    for (choice, interaction) in &choices {
        let Some(action) = choice.action else {
            continue;
        };
        if *interaction == ButtonVisualState::Hovered {
            focus.0 = Some(action);
            focus.1 = true;
        }
        if *interaction == ButtonVisualState::Pressed && mouse.just_pressed(MouseButton::Left) {
            requests.write(ActionRequest(action));
        }
    }
}
#[allow(clippy::too_many_arguments)] // Ресурсы и запросы состояния одного ECS-обработчика.
fn handle_actions(
    mut requests: MessageReader<ActionRequest>,
    mut context: ResMut<Context>,
    state: Res<State<Screen>>,
    mut next: ResMut<NextState<Screen>>,
    mut window: Single<&mut Window>,
    mut exit: MessageWriter<AppExit>,
    mut commands: Commands,
    mut navigation: ResMut<pages::Navigation>,
) {
    for ActionRequest(action) in requests.read() {
        match action {
            Action::NewGame if *state.get() == Screen::Main => {
                context.notice.clear();
                next.set(Screen::NewGame);
            }
            Action::LoadGame if *state.get() == Screen::Main => {
                context.notice.clear();
                next.set(Screen::LoadGame);
            }
            Action::Multiplayer if *state.get() == Screen::Main => {
                context.notice.clear();
                next.set(Screen::Multiplayer);
            }
            Action::DisplayTab | Action::AudioTab | Action::MouseTab | Action::KeyboardTab
                if *state.get() == Screen::Options =>
            {
                navigation.options = match action {
                    Action::AudioTab => 1,
                    Action::MouseTab => 2,
                    Action::KeyboardTab => 3,
                    _ => 0,
                };
                navigation.dirty = true;
            }
            Action::BriefingTab | Action::TeamTab | Action::EquipmentTab
                if *state.get() == Screen::NewGame =>
            {
                navigation.campaign = match action {
                    Action::TeamTab => 1,
                    Action::EquipmentTab => 2,
                    _ => 0,
                };
                navigation.dirty = true;
            }
            Action::PreviousMission | Action::NextMission if *state.get() == Screen::NewGame => {
                let count = pages::mission_count(&context.catalog);
                navigation.mission = if *action == Action::NextMission {
                    (navigation.mission + 1) % count
                } else {
                    (navigation.mission + count - 1) % count
                };
                navigation.dirty = true;
            }
            Action::RefreshSaves if *state.get() == Screen::LoadGame => {
                context.notice = if context.saved.english {
                    "List refreshed: no compatible saves."
                } else {
                    "Список обновлён: совместимых сохранений нет."
                }
                .into();
            }
            Action::Intro if *state.get() == Screen::Main => {
                context.notice.clear();
                commands.insert_resource(movie::Request {
                    catalog: context.catalog.clone(),
                    path: "movies/intro.avi".into(),
                    snapshot: None,
                });
                next.set(Screen::Movie);
            }
            Action::Training if matches!(state.get(), Screen::Main | Screen::NewGame) => {
                context.notice.clear();
                next.set(Screen::Loading);
            }
            Action::Options if matches!(state.get(), Screen::Main | Screen::Pause) => {
                context.options_return = *state.get();
                context.draft = context.saved.clone();
                context.notice.clear();
                next.set(Screen::Options);
            }
            Action::MasterVolume if *state.get() == Screen::Options => {
                context.draft.master_volume = if context.draft.master_volume >= 1.0 {
                    0.0
                } else {
                    (context.draft.master_volume + 0.1).min(1.0)
                };
            }
            Action::CameraSpeed if *state.get() == Screen::Options => {
                context.draft.camera_speed = if context.draft.camera_speed >= 2.0 {
                    0.5
                } else {
                    (context.draft.camera_speed + 0.25).min(2.0)
                };
            }
            Action::EdgeScroll if *state.get() == Screen::Options => {
                context.draft.edge_scroll = !context.draft.edge_scroll
            }
            Action::Language if *state.get() == Screen::Options => {
                context.draft.english = !context.draft.english
            }
            Action::Window if *state.get() == Screen::Options => {
                context.draft.fullscreen = !context.draft.fullscreen
            }
            Action::TextSize if *state.get() == Screen::Options => {
                context.draft.text_scale = match context.draft.text_scale {
                    1.0 => 1.25,
                    1.25 => 1.5,
                    _ => 1.0,
                }
            }
            Action::Background if *state.get() == Screen::Options => {
                context.draft.animate_background = !context.draft.animate_background
            }
            Action::Resolution if *state.get() == Screen::Options => {
                let index = RESOLUTIONS
                    .iter()
                    .position(|s| *s == context.draft.window_size);
                context.draft.window_size =
                    RESOLUTIONS[index.map_or(0, |i| (i + 1) % RESOLUTIONS.len())];
            }
            Action::Apply if *state.get() == Screen::Options => {
                match context.draft.save(&context.settings_path) {
                    Ok(()) => {
                        context.saved = context.draft.clone();
                        commands.insert_resource(context.saved.clone());
                        commands.insert_resource(training::CameraOptions {
                            speed: context.saved.camera_speed,
                            edge_scroll: context.saved.edge_scroll,
                        });
                        window.resolution.set(
                            context.saved.window_size[0] as f32,
                            context.saved.window_size[1] as f32,
                        );
                        window.mode = window_mode(context.saved.fullscreen);
                        context.notice = if context.saved.english {
                            "Settings saved"
                        } else {
                            "Настройки сохранены"
                        }
                        .into();
                    }
                    Err(e) => context.notice = format!("Не удалось сохранить настройки: {e}"),
                }
            }
            Action::Back => match state.get() {
                Screen::Options => next.set(context.options_return),
                Screen::Loading => {
                    next.set(Screen::Main);
                    commands.remove_resource::<Loading>();
                }
                Screen::Movie => next.set(Screen::Main),
                Screen::NewGame | Screen::LoadGame | Screen::Multiplayer => next.set(Screen::Main),
                Screen::Quit => next.set(navigation.quit_return.unwrap_or(Screen::Main)),
                _ => {}
            },
            Action::Resume if *state.get() == Screen::Pause => next.set(Screen::Training),
            Action::Pause if *state.get() == Screen::Training => next.set(Screen::Pause),
            Action::Main if *state.get() == Screen::Pause => {
                context.notice.clear();
                next.set(Screen::Main);
            }
            Action::Exit => {
                navigation.quit_return = Some(*state.get());
                context.notice.clear();
                next.set(Screen::Quit);
            }
            Action::ConfirmExit => {
                exit.write(AppExit::Success);
            }
            _ => {}
        }
    }
}
fn sync_language(
    context: Res<Context>,
    state: Res<State<Screen>>,
    mut language: ResMut<crate::localization::Language>,
) {
    let english = if *state.get() == Screen::Options {
        context.draft.english
    } else {
        context.saved.english
    };
    language.set_if_neq(crate::localization::Language::from_english(english));
}
fn apply_camera(
    state: Res<State<Screen>>,
    mut cameras: Query<(
        &mut Camera,
        Option<&MenuCamera>,
        Option<&training::MapCamera>,
    )>,
) {
    let training = matches!(state.get(), Screen::Training | Screen::Pause);
    for (mut camera, menu, map) in &mut cameras {
        if menu.is_some() && camera.is_active == training {
            camera.is_active = !training;
        }
        if map.is_some() && camera.is_active != training {
            camera.is_active = training;
        }
    }
}
// Фильтры явно разделяют изменяемые Node-запросы корней, элементов и фона.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn fit_ui(
    window: Single<&Window>,
    context: Res<Context>,
    state: Res<State<Screen>>,
    mut roots: Query<&mut Node, (With<Canvas>, Without<Placement>)>,
    mut children: Query<(&Placement, &Anchor, &mut Node), (Without<Canvas>, Without<Background>)>,
    mut backgrounds: Query<
        (&ImageNode, &mut Node),
        (With<Background>, Without<Canvas>, Without<Placement>),
    >,
    images: Res<Assets<Image>>,
    mut fonts: Query<(
        &Caption,
        &mut TextFont,
        Option<&ButtonCaption>,
        Option<&Placement>,
    )>,
    mut previous: Local<Option<([f32; 2], f32, Screen, usize)>>,
) {
    let text_scale = if *state.get() == Screen::Options {
        context.draft.text_scale
    } else {
        context.saved.text_scale
    };
    let metrics = (
        [window.width(), window.height()],
        text_scale,
        *state.get(),
        context.frame,
    );
    // Два слоя могут иметь разные размеры в overrides; подгоняем каждый отдельно.
    // Это делается и внутри перехода, когда номер основного кадра ещё не изменился.
    for (image_node, mut node) in &mut backgrounds {
        if let Some(image) = images.get(&image_node.image) {
            let source = image.size().as_vec2();
            let scale = (window.width() / source.x).max(window.height() / source.y);
            let size = source * scale;
            node.left = px((window.width() - size.x) * 0.5);
            node.top = px((window.height() - size.y) * 0.5);
            node.width = px(size.x);
            node.height = px(size.y);
        }
    }
    let fresh = children.iter_mut().any(|(_, _, node)| node.is_added());
    if previous.as_ref() == Some(&metrics) && !fresh {
        return;
    }
    *previous = Some(metrics);
    let Some(fit) = context.layout.fit(window.width(), window.height()) else {
        return;
    };
    for mut root in &mut roots {
        root.left = px(fit.origin[0]);
        root.top = px(fit.origin[1]);
        root.width = px(fit.size[0]);
        root.height = px(fit.size[1]);
    }
    let extra_width = window.width() - context.layout.size[0] * fit.scale;
    for (placement, anchor, mut node) in &mut children {
        let offset = match anchor {
            Anchor::Left | Anchor::Stretch => 0.0,
            Anchor::Right => extra_width,
            Anchor::Center => extra_width * 0.5,
        };
        node.left = px(placement.position.x * fit.scale + offset);
        node.top = px(placement.position.y * fit.scale);
        node.width = px(placement.size.x * fit.scale
            + if matches!(anchor, Anchor::Stretch) {
                extra_width
            } else {
                0.0
            });
        node.height = px(placement.size.y * fit.scale);
    }
    for (caption, mut font, button, placement) in &mut fonts {
        let mut size = caption.size * text_scale;
        if button.is_some()
            && let Some(placement) = placement
        {
            // Основной и контурные слои получают одинаковую ограниченную кегль.
            // Длинные русские подписи и масштаб 150% не выходят из кнопки.
            let chars = caption
                .ru
                .chars()
                .count()
                .max(caption.en.chars().count())
                .max(1);
            if chars > 1 {
                size = size.min(placement.size.x / (chars as f32 * 0.75));
            }
        }
        font.font_size = FontSize::Px(size * fit.scale);
    }
}
fn update_captions(
    context: Res<Context>,
    state: Res<State<Screen>>,
    mut captions: Query<(&Caption, &mut Text)>,
) {
    let p = if *state.get() == Screen::Options {
        &context.draft
    } else {
        &context.saved
    };
    for (caption, mut text) in &mut captions {
        let value = match caption.action {
            Some(Action::Notice)
                if matches!(
                    context.notice.as_str(),
                    "Настройки сохранены" | "Settings saved"
                ) =>
            {
                if p.english {
                    "Settings saved"
                } else {
                    "Настройки сохранены"
                }
                .into()
            }
            Some(Action::Notice) => context.notice.clone(),
            Some(Action::Language) => {
                if p.english {
                    "MENU LANGUAGE: ENGLISH".into()
                } else {
                    "ЯЗЫК МЕНЮ: РУССКИЙ".into()
                }
            }
            Some(Action::MasterVolume) => {
                format!(
                    "{}: {:.0}%",
                    if p.english {
                        "MASTER VOLUME"
                    } else {
                        "ОБЩАЯ ГРОМКОСТЬ"
                    },
                    p.master_volume * 100.
                )
            }
            Some(Action::CameraSpeed) => {
                format!(
                    "{}: {:.0}%",
                    if p.english {
                        "CAMERA SPEED"
                    } else {
                        "СКОРОСТЬ КАМЕРЫ"
                    },
                    p.camera_speed * 100.
                )
            }
            Some(Action::EdgeScroll) => {
                format!(
                    "{}: {}",
                    if p.english {
                        "SCREEN EDGE SCROLLING"
                    } else {
                        "ПРОКРУТКА У КРАЯ ЭКРАНА"
                    },
                    if p.edge_scroll {
                        if p.english { "ON" } else { "ВКЛ" }
                    } else if p.english {
                        "OFF"
                    } else {
                        "ВЫКЛ"
                    }
                )
            }
            Some(Action::Window) => {
                if p.english {
                    format!(
                        "DISPLAY: {}",
                        if p.fullscreen {
                            "FULLSCREEN"
                        } else {
                            "WINDOWED"
                        }
                    )
                } else {
                    format!(
                        "ЭКРАН: {}",
                        if p.fullscreen {
                            "ПОЛНЫЙ ЭКРАН"
                        } else {
                            "ОКНО"
                        }
                    )
                }
            }
            Some(Action::TextSize) => {
                if p.english {
                    format!("TEXT SIZE: {:.0}%", p.text_scale * 100.0)
                } else {
                    format!("РАЗМЕР ТЕКСТА: {:.0}%", p.text_scale * 100.0)
                }
            }
            Some(Action::Resolution) => {
                let label = if p.english {
                    "RESOLUTION"
                } else {
                    "РАЗРЕШЕНИЕ"
                };
                format!("{label}: {} × {}", p.window_size[0], p.window_size[1])
            }
            Some(Action::Background) => {
                if p.english {
                    format!(
                        "ANIMATED BACKGROUND: {}",
                        if p.animate_background { "ON" } else { "OFF" }
                    )
                } else {
                    format!(
                        "СМЕНА ФОНОВ: {}",
                        if p.animate_background {
                            "ВКЛ"
                        } else {
                            "ВЫКЛ"
                        }
                    )
                }
            }
            Some(Action::Apply) => {
                if p.english {
                    "APPLY".into()
                } else {
                    "ПРИМЕНИТЬ".into()
                }
            }
            _ => {
                if p.english {
                    caption.en.clone()
                } else {
                    caption.ru.clone()
                }
            }
        };
        if text.0 != value {
            text.0 = value;
        }
    }
}
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn style_buttons(
    time: Res<Time<Real>>,
    window: Single<&Window>,
    context: Res<Context>,
    focus: Res<Focus>,
    preview: Option<Res<AnimationPreview>>,
    assets: Res<MenuAssets>,
    mut choices: Query<(
        Entity,
        &Choice,
        Option<&ButtonVisualState>,
        Option<&mut menu_motion::Hover>,
    )>,
    mut background: Query<&mut BackgroundColor>,
    mut captions: Query<
        (
            &ChildOf,
            &mut TextColor,
            &mut Node,
            Option<&TitleGhost>,
            Option<&TitleOutline>,
            Option<&TitleBlur>,
        ),
        Without<TitleBullet>,
    >,
    mut bullets: Query<(&ChildOf, &mut ImageNode), With<TitleBullet>>,
    rings: Query<(Entity, &ChildOf, &BrassRing)>,
) {
    let scale = window.height() / context.layout.size[1];
    let snapshot = preview.as_ref().and_then(|p| p.hover);
    for (entity, choice, interaction, mut hover) in &mut choices {
        let selected = choice.action.is_some()
            && if let Some((action, _)) = snapshot {
                choice.action == Some(action)
            } else if focus.1 {
                interaction.is_some_and(|i| *i != ButtonVisualState::None)
            } else {
                choice.action == focus.0
            };
        let pressed = interaction.is_some_and(|i| *i == ButtonVisualState::Pressed);
        let title = hover.is_some();
        let progress = if let Some(h) = hover.as_mut() {
            if let Some((action, progress)) = snapshot {
                h.progress = if choice.action == Some(action) {
                    progress
                } else {
                    0.0
                };
            } else {
                h.advance(selected, time.delta_secs());
            }
            h.progress
        } else {
            if selected { 1.0 } else { 0.0 }
        };
        if let Ok(mut color) = background.get_mut(entity) {
            color.0 = if title {
                Color::NONE
            } else if pressed {
                Color::srgba(0.55, 0.2, 0.04, 0.6)
            } else if selected {
                Color::srgba(0.30, 0.32, 0.15, 0.6)
            } else {
                Color::NONE
            };
        }
        for (parent, mut color, mut node, ghost, outline, blur) in &mut captions {
            if parent.parent() == entity {
                color.0 = if ghost.is_some() {
                    let position = hover.as_ref().unwrap().offset(
                        Vec2::from_array(context.layout.text_offset),
                        Vec2::from_array(context.layout.shadow_offset),
                    );
                    let position = position + blur.map_or(Vec2::ZERO, |b| b.0) * (1.0 - progress);
                    node.left = px(position.x * scale);
                    node.top = px(position.y * scale);
                    Color::srgba(
                        0.94,
                        0.33,
                        0.015,
                        blur.map_or(0.3, |b| b.1) * (1.0 - progress),
                    )
                } else if let Some(outline) = outline {
                    let position = Vec2::from_array(context.layout.text_offset) + outline.0;
                    node.left = px(position.x * scale);
                    node.top = px(position.y * scale);
                    Color::srgb(0.80 + progress * 0.2, 0.16, 0.01)
                } else if choice.action.is_none() {
                    Color::srgb(0.43, 0.43, 0.41)
                } else if title && pressed {
                    Color::srgb(0.65, 0.30, 0.04)
                } else if title {
                    Color::srgb(1.0, 0.85 + 0.12 * progress, 0.08 + 0.32 * progress)
                } else if selected {
                    Color::srgb(1.0, 0.72, 0.40)
                } else {
                    Color::srgb(0.90, 0.90, 0.83)
                };
            }
        }
        for (ring_entity, parent, ring) in &rings {
            if parent.parent() == entity
                && let Ok(mut color) = background.get_mut(ring_entity)
            {
                color.0 = ring.0.with_alpha(if pressed { 0.6 } else { 1.0 });
            }
        }
        for (parent, mut image) in &mut bullets {
            if parent.parent() == entity {
                image.rect = Some(menu_motion::bullet_rect(assets.bullet_size, pressed));
                image.color =
                    Color::WHITE.with_alpha(if choice.action.is_some() { 1.0 } else { 0.30 });
            }
        }
    }
}
fn animate_background(
    time: Res<Time<Real>>,
    capture: Res<Capture>,
    mut context: ResMut<Context>,
    assets: Res<MenuAssets>,
    preview: Option<Res<AnimationPreview>>,
    mut elapsed: Local<f64>,
    mut images: Query<(&Background, &mut ImageNode)>,
) {
    if images.is_empty() {
        return;
    }
    let snapshot = preview.as_ref().and_then(|p| p.seconds);
    if context.saved.animate_background && capture.fixed_frame.is_none() && snapshot.is_none() {
        *elapsed += time.delta_secs_f64();
    }
    let blend = if let Some(frame) = capture.fixed_frame {
        let frame = frame % assets.backgrounds.len();
        menu_motion::Blend {
            from: frame,
            to: frame,
            alpha: 0.0,
        }
    } else {
        menu_motion::background(snapshot.unwrap_or(*elapsed), assets.backgrounds.len())
    };
    context.frame = blend.from;
    for (layer, mut image) in &mut images {
        image.image = assets.backgrounds[if layer.0 { blend.to } else { blend.from }].clone();
        image.color = Color::WHITE.with_alpha(if layer.0 { blend.alpha } else { 1.0 });
    }
}
fn capture(
    mut commands: Commands,
    mut capture: ResMut<Capture>,
    state: Res<State<Screen>>,
    mut next: ResMut<NextState<Screen>>,
    playback: Option<Res<movie::Playback>>,
) {
    if capture.boot_pause && *state.get() == Screen::Training {
        capture.boot_pause = false;
        next.set(Screen::Pause);
        return;
    }
    if *state.get() != capture.target {
        return;
    }
    if capture.target == Screen::Movie && playback.is_none() {
        return;
    }
    capture.frames += 1;
    if capture.frames == 120
        && let Some(path) = capture.path.clone()
    {
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path))
            .observe(
                |_: On<ScreenshotCaptured>, mut exit: MessageWriter<AppExit>| {
                    exit.write(AppExit::Success);
                },
            );
    }
}

#[derive(Resource)]
struct MovieSmoke {
    path: PathBuf,
    movie: String,
    stage: usize,
    frames: u32,
    ids: Option<(AssetId<Image>, Option<AssetId<movie::MovieAudio>>)>,
    cycles: Vec<serde_json::Value>,
}
impl MovieSmoke {
    fn new(path: PathBuf, movie: String) -> Self {
        Self {
            path,
            movie,
            stage: 0,
            frames: 0,
            ids: None,
            cycles: vec![],
        }
    }
}
#[allow(clippy::too_many_arguments)]
fn movie_smoke(
    mut smoke: ResMut<MovieSmoke>,
    state: Res<State<Screen>>,
    playback: Option<Res<movie::Playback>>,
    images: Res<Assets<Image>>,
    audio: Res<Assets<movie::MovieAudio>>,
    context: Res<Context>,
    cameras: Query<&Camera>,
    sounds: Query<&AudioPlayer<movie::MovieAudio>>,
    mut requests: MessageWriter<ActionRequest>,
    mut next: ResMut<NextState<Screen>>,
    mut commands: Commands,
    mut exit: MessageWriter<AppExit>,
    mut previous: Local<Option<(usize, Screen, Option<bool>)>>,
) {
    smoke.frames += 1;
    assert!(
        !(smoke.stage == 0 && *state.get() == Screen::Main),
        "movie ended before the skip check"
    );
    let step = (
        smoke.stage,
        *state.get(),
        playback.as_ref().map(|p| p.audio_started),
    );
    if previous.as_ref() != Some(&step)
        || (*state.get() == Screen::Movie && smoke.frames.is_multiple_of(6))
        || smoke.frames.is_multiple_of(120)
    {
        eprintln!(
            "movie smoke: stage={} screen={:?} frames={} video={:?}",
            smoke.stage,
            state.get(),
            smoke.frames,
            playback.as_ref().map(|p| p.diagnostics())
        );
        *previous = Some(step);
    }
    assert!(smoke.frames < 10000, "movie smoke timeout");
    if matches!(smoke.stage, 0 | 2)
        && let Some(p) = playback
    {
        smoke.ids = Some((p.image.id(), p.audio.as_ref().map(Handle::id)));
        if smoke.stage == 0 && p.frame >= 4 {
            smoke.cycles.push(serde_json::json!({"reason":"skip","frame":p.frame,"decoded":p.decoded,"audio_started":p.audio_started}));
            requests.write(ActionRequest(Action::Back));
            smoke.stage = 1;
            smoke.frames = 0;
        } else if smoke.stage == 2 && smoke.cycles.len() == 1 {
            smoke.cycles.push(
                serde_json::json!({"reason":"end","replayed":true,"audio_started":p.audio_started}),
            );
        } else if smoke.stage == 2 && p.audio_started {
            smoke.cycles[1]["audio_started"] = serde_json::json!(true);
        }
        if smoke.stage == 2 && smoke.cycles.len() == 2 {
            smoke.cycles[1]["frame"] = serde_json::json!(p.frame);
            smoke.cycles[1]["decoded"] = serde_json::json!(p.decoded);
        }
    }
    if matches!(smoke.stage, 1 | 2) && *state.get() == Screen::Main && smoke.frames >= 20 {
        let (image, sound) = smoke.ids.expect("movie was loaded");
        assert!(images.get(image).is_none());
        assert!(sound.is_none_or(|id| audio.get(id).is_none()));
        assert!(sounds.is_empty());
        assert_eq!(cameras.iter().filter(|c| c.is_active).count(), 1);
        if smoke.stage == 1 {
            commands.insert_resource(movie::Request {
                catalog: context.catalog.clone(),
                path: smoke.movie.clone(),
                snapshot: None,
            });
            next.set(Screen::Movie);
            smoke.stage = 2;
            smoke.frames = 0;
        } else {
            let report = serde_json::json!({"status":"passed","cycles":smoke.cycles,"images_released":true,"audio_released":true,"audio_entities_released":true,"active_cameras":1});
            std::fs::write(
                &smoke.path,
                serde_json::to_string_pretty(&report).unwrap() + "\n",
            )
            .unwrap();
            smoke.stage = 3;
            exit.write(AppExit::Success);
        }
    }
}

#[derive(Resource)]
struct Smoke {
    path: PathBuf,
    stage: usize,
    frames: u32,
    cycles: Vec<serde_json::Value>,
    roots: usize,
    meshes: Vec<AssetId<Mesh>>,
    materials: Vec<AssetId<StandardMaterial>>,
    images: Vec<AssetId<Image>>,
    sounds: Vec<AssetId<AudioSource>>,
    pause_snapshot: Option<serde_json::Value>,
}
impl Smoke {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            stage: 0,
            frames: 0,
            cycles: Vec::new(),
            roots: 0,
            meshes: Vec::new(),
            materials: Vec::new(),
            images: Vec::new(),
            sounds: Vec::new(),
            pause_snapshot: None,
        }
    }
}
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn smoke(
    mut smoke: ResMut<Smoke>,
    state: Res<State<Screen>>,
    roots: Query<
        (Entity, Has<crate::medical::HealthLabel>),
        (
            With<player::SceneEntity>,
            Without<crate::tutorial::HelpPanel>,
            Without<crate::inventory::InventoryPanel>,
        ),
    >,
    help_panels: Query<Entity, With<crate::tutorial::HelpPanel>>,
    tutorial: Option<Res<crate::tutorial::Tutorial>>,
    mut pickup: Option<ResMut<crate::pickup::Pickup>>,
    battle: (
        Option<Res<crate::combat::Combat>>,
        Option<ResMut<crate::grenade::Grenade>>,
        Option<ResMut<crate::fence::Fence>>,
        Option<ResMut<crate::smg::Uzi>>,
        Option<ResMut<crate::medical::Medical>>,
        Option<ResMut<crate::armor::Armor>>,
        Option<ResMut<crate::vehicle::Vehicle>>,
        Option<Res<crate::gate::Gate>>,
        Option<Res<crate::party::Party>>,
        Option<Res<crate::evacuation::Evacuation>>,
        Option<Res<crate::roster::Roster>>,
        Option<Res<crate::selection::Selection>>,
        Option<Res<crate::hud::Hud>>,
    ),
    mut inventory: Option<ResMut<crate::inventory::Inventory>>,
    mesh_components: Query<(&Mesh3d, &MeshMaterial3d<StandardMaterial>)>,
    meshes: Res<Assets<Mesh>>,
    materials: Res<Assets<StandardMaterial>>,
    assets: (Res<Assets<Image>>, Res<Assets<AudioSource>>),
    mut player: Option<ResMut<player::Player>>,
    cameras: (
        Query<&Camera>,
        Query<&bevy::window::CursorIcon, With<Window>>,
    ),
    mut requests: MessageWriter<ActionRequest>,
    mut exit: MessageWriter<AppExit>,
) {
    let (images, sounds) = assets;
    let (cameras, cursors) = cameras;
    let (
        combat,
        mut grenade,
        mut fence,
        mut uzi,
        mut medical,
        mut armor,
        vehicle,
        gate,
        party,
        evacuation,
        roster,
        selection,
        hud,
    ) = battle;
    smoke.frames += 1;
    let active = cameras.iter().filter(|c| c.is_active).count();
    let ready = smoke.frames >= 20;
    let snapshot = serde_json::json!({"party":party.as_ref().and_then(|g|player.as_ref().map(|p|g.report(p))),"gate":gate.as_ref().map(|g|g.report()),"vehicle":vehicle.as_ref().map(|v|v.report()),"armor":armor.as_ref().map(|a|a.report()),"medical":medical.as_ref().map(|m|m.report()),"clock":tutorial.as_ref().map(|t|t.clock),"help":tutorial.as_ref().and_then(|t|t.active.as_ref()),
        "combat":combat.as_ref().map(|c|c.report()),"inventory":inventory.as_ref().and_then(|inv|player.as_ref().map(|p|inv.report(p))),"grenade":grenade.as_ref().map(|g|g.report()),"fence":fence.as_ref().map(|f|f.report()),"collected":pickup.as_ref().map(|p|p.collected),"weapon":player.as_ref().and_then(|p|p.weapon.as_ref()),"posture":player.as_ref().map(|p|p.posture.name()),"uzi_pickup":uzi.as_ref().map(|u|u.report())});
    match smoke.stage {
        0 if *state.get() == Screen::Main && ready => {
            requests.write(ActionRequest(Action::Training));
            smoke.stage = 1;
            smoke.frames = 0;
        }
        1 if *state.get() == Screen::Training
            && ready
            && roots.iter().any(|(_, health)| health) =>
        {
            // Контролируемая фикстура проверяет паузу с экипировкой и потраченными патронами.
            if let (Some(inv), Some(p), Some(item)) = (&mut inventory, &mut player, &mut pickup) {
                item.collected = true;
                inv.owns_knife = true;
                inv.collected = true;
                inv.rounds = 7;
                inv.shots = 6;
                inv.open = true;
                inv.owns_uzi = true;
                inv.uzi_rounds = 23;
                inv.uzi_shots = 9;
                inv.equip(crate::inventory::Weapon::Uzi, p);
                let p = &mut **p;
                p.posture = crate::stance::Posture::Crouch;
                let idle = p.idle_clip();
                p.playback.select(&p.clips, idle);
                smoke.sounds = inv
                    .sound_handle
                    .as_ref()
                    .map(|h| vec![h.id()])
                    .unwrap_or_default();
            }
            if let Some(u) = &mut uzi {
                u.collected = true;
            }
            if let Some(inv) = &inventory
                && let Some(h) = &inv.uzi_sound_handle
            {
                smoke.sounds.push(h.id());
            }
            if let Some(g) = &mut grenade {
                g.collected = true;
                g.count = 2;
                g.secondary = true;
                if let Some(h) = &g.sound_handle {
                    smoke.sounds.push(h.id());
                }
            }
            if let (Some(f), Some(inv)) = (&mut fence, &mut inventory) {
                f.collected = true;
                f.cut = true;
                f.cuts = 1;
                f.crossings = 1;
                inv.owns_cutters = true;
            }
            if let Some(m) = &mut medical {
                m.ground.collected = true;
                m.health = 230.0;
                m.uses = 4;
                m.armed = true;
                if let Some(handle) = &m.sound_handle {
                    smoke.sounds.push(handle.id());
                }
            }
            if let Some(a) = &mut armor {
                a.ground.collected = true;
                a.worn = true;
            }
            if let Some(sound) = party.as_ref().and_then(|p| p.sound_handle.as_ref()) {
                smoke.sounds.push(sound.id());
            }
            smoke.roots = roots.iter().count();
            assert_eq!(active, 1, "training camera count");
            assert!(smoke.roots > 0);
            let mut mesh_ids = HashSet::new();
            let mut material_ids = HashSet::new();
            let mut image_ids = HashSet::new();
            for (mesh, material) in &mesh_components {
                mesh_ids.insert(mesh.0.id());
                material_ids.insert(material.0.id());
                if let Some(texture) = materials
                    .get(&material.0)
                    .and_then(|m| m.base_color_texture.as_ref())
                {
                    image_ids.insert(texture.id());
                }
            }
            smoke.meshes = mesh_ids.into_iter().collect();
            smoke.materials = material_ids.into_iter().collect();
            if let Some(a) = &armor {
                image_ids.extend(a.image_handles().map(|handle| handle.id()));
            }
            if let Some(r) = &roster {
                image_ids.extend(r.image_handles().map(|h| h.id()));
            }
            if let Some(h) = &hud {
                image_ids.extend(h.image_handles().map(|h| h.id()));
            }
            smoke.images = image_ids.into_iter().collect();
            requests.write(ActionRequest(Action::Pause));
            smoke.stage = 2;
            smoke.frames = 0;
        }
        2 if *state.get() == Screen::Pause && ready => {
            smoke.pause_snapshot = Some(snapshot.clone());
            assert_eq!(active, 1);
            assert_eq!(roots.iter().count(), smoke.roots);
            requests.write(ActionRequest(Action::Options));
            smoke.stage = 3;
            smoke.frames = 0;
        }
        3 if *state.get() == Screen::Options && ready => {
            assert_eq!(
                Some(&snapshot),
                smoke.pause_snapshot.as_ref(),
                "options must preserve paused tutorial/equipment"
            );
            assert_eq!(active, 1);
            assert_eq!(roots.iter().count(), smoke.roots);
            requests.write(ActionRequest(Action::Back));
            smoke.stage = 4;
            smoke.frames = 0;
        }
        4 if *state.get() == Screen::Pause && ready => {
            assert_eq!(cursors.iter().count(), 1);
            assert!(
                cursors.iter().all(|c| *c
                    == bevy::window::CursorIcon::System(bevy::window::SystemCursorIcon::Default))
            );
            assert_eq!(
                Some(&snapshot),
                smoke.pause_snapshot.as_ref(),
                "pause must stop tutorial time"
            );
            requests.write(ActionRequest(Action::Resume));
            smoke.stage = 5;
            smoke.frames = 0;
        }
        5 if *state.get() == Screen::Training && ready => {
            assert!(cursors.iter().all(|c| *c
                == bevy::window::CursorIcon::System(bevy::window::SystemCursorIcon::Crosshair)));
            assert_eq!(active, 1);
            assert_eq!(
                roots.iter().count(),
                smoke.roots,
                "resume duplicated the scene"
            );
            requests.write(ActionRequest(Action::Pause));
            smoke.stage = 6;
            smoke.frames = 0;
        }
        6 if *state.get() == Screen::Pause && ready => {
            requests.write(ActionRequest(Action::Main));
            smoke.stage = 7;
            smoke.frames = 0;
        }
        7 if *state.get() == Screen::Main && ready => {
            let clean = roots.is_empty()
                && help_panels.is_empty()
                && tutorial.is_none()
                && pickup.is_none()
                && combat.is_none()
                && inventory.is_none()
                && grenade.is_none()
                && fence.is_none()
                && uzi.is_none()
                && medical.is_none()
                && armor.is_none()
                && vehicle.is_none()
                && gate.is_none()
                && party.is_none()
                && evacuation.is_none()
                && roster.is_none()
                && selection.is_none()
                && hud.is_none()
                && player.is_none()
                && smoke.meshes.iter().all(|id| !meshes.contains(*id))
                && smoke.materials.iter().all(|id| !materials.contains(*id))
                && smoke.images.iter().all(|id| !images.contains(*id))
                && smoke.sounds.iter().all(|id| !sounds.contains(*id));
            assert!(clean, "scene resources survived return to menu");
            assert!(
                cursors.iter().all(|c| *c
                    == bevy::window::CursorIcon::System(bevy::window::SystemCursorIcon::Default))
            );
            assert_eq!(active, 1);
            let value = serde_json::json!({"scene_roots_loaded":smoke.roots,"tracked_meshes_released":smoke.meshes.len(),"tracked_materials_released":smoke.materials.len(),"tracked_textures_released":smoke.images.len(),"tracked_audio_sources_released":smoke.sounds.len(),"inventory_fixture":"UZI primary with 23 rounds, crouched, knife/BDA/cutters stored, BDA 7 rounds, 2 grenades, cut fence and one crossing, medkit 4 uses and 230 HP with healing selected, Flak Jacket worn, inventory open","resume_kept_scene":true,"pause_kept_tutorial_and_equipment":true,"tutorial_resources_released":true,"scene_resources_released":clean,"party_resource_released":party.is_none(),"evacuation_resource_released":evacuation.is_none(),"gate_resource_released":gate.is_none(),"paused_gate_state_checked":true,"cursor_restored_for_menu":true,"healing_cursor_resumed":true,"active_menu_cameras":active});
            smoke.cycles.push(value);
            if smoke.cycles.len() == 2 {
                std::fs::write(&smoke.path,serde_json::to_string_pretty(&serde_json::json!({"status":"passed","cycles":smoke.cycles,"loading_async":true,"single_window":true})) .unwrap()).unwrap();
                exit.write(AppExit::Success);
            } else {
                smoke.stage = 0;
                smoke.frames = 0;
            }
        }
        _ => {}
    }
}

fn training_finished(
    tutorial: Option<Res<crate::tutorial::Tutorial>>,
    mut context: ResMut<Context>,
    mut next: ResMut<NextState<Screen>>,
) {
    if tutorial.is_some_and(|t| t.end_campaign.as_deref() == Some("EOC_RETURN_TO_MAIN_MENU")) {
        context.notice = if context.saved.english {
            "Training complete"
        } else {
            "Обучение завершено"
        }
        .into();
        next.set(Screen::Main);
    }
}
#[allow(clippy::too_many_arguments)]
fn evacuation_smoke_result(
    s: Option<ResMut<crate::evacuation::NativeSmoke>>,
    state: Res<State<Screen>>,
    roots: Query<Entity, With<player::SceneEntity>>,
    e: Option<Res<crate::evacuation::Evacuation>>,
    party: Option<Res<crate::party::Party>>,
    tutorial: Option<Res<crate::tutorial::Tutorial>>,
    cameras: Query<&Camera>,
    context: Res<Context>,
    mut requests: MessageWriter<ActionRequest>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(mut s) = s else {
        return;
    };
    if !s.menu_started && *state.get() == Screen::Main {
        s.menu_started = true;
        requests.write(ActionRequest(Action::Training));
    }
    if s.stage == 8 && *state.get() == Screen::Main {
        assert!(roots.is_empty() && e.is_none() && party.is_none() && tutorial.is_none());
        assert_eq!(cameras.iter().filter(|c| c.is_active).count(), 1);
        assert!(
            context.notice.contains("Обучение завершено")
                || context.notice.contains("Training complete")
        );
        s.checks.push(serde_json::json!({"automatic_return_to_main_menu":true,"evacuation_and_party_resources_released":true,"training_scene_entities_released":true,"single_active_camera":true,"success_notice":true}));
        std::fs::write(&s.path,serde_json::to_string_pretty(&serde_json::json!({"status":"passed","checks":s.checks,"before_exit":s.before_exit,"returned_to_main_menu":true,"physical_pointer_tested":false})).unwrap()+"\n").unwrap();
        s.stage = 10;
        exit.write(AppExit::Success);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn animation_test_app(name: &str) -> (App, PathBuf) {
        let path = std::env::temp_dir().join(format!("shadow-menu-{name}-{}", std::process::id()));
        std::fs::create_dir_all(path.join("Data")).unwrap();
        let mut gut = b"test\x1a\xfa\x31\xc1\xde\xed\x42\x13".to_vec();
        gut.extend(0_u32.to_le_bytes());
        gut.extend([0; 32]);
        std::fs::write(path.join("Data/test.gut"), gut).unwrap();
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<Assets<Image>>()
            .init_resource::<Focus>()
                    .init_resource::<pages::Navigation>()
            .init_resource::<ButtonInput<MouseButton>>()
            .add_message::<ActionRequest>()
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(0.125)))
            .insert_resource(Capture { target: Screen::Main, path: None, frames: 0, boot_pause: false, fixed_frame: None, ignore_input: false })
            .insert_resource(Context {
                catalog: Arc::new(AssetCatalog::open(&path, None).unwrap()),
                layout: MenuLayout::from_bytes(b"WINDOW_BASE_DX 854\nWINDOW_BASE_DY 480\nDEFINE_USER_IVAR button_offset_x 30\nDEFINE_USER_IVAR button_offset_y 5\nDEFINE_BUTTON_ADVICE b_training 529 160 0 0\nDEFINE_BUTTON_ADVICE b_exit 509 320 0 0\nGEOMETRY_JPG_NAME frame1\n").unwrap(),
                settings_path: path.join("settings.json"), saved: Preferences::default(), draft: Preferences::default(), notice: String::new(), frame: 4, options_return: Screen::Main,
            });
        let backgrounds = (0..5)
            .map(|_| {
                app.world_mut()
                    .resource_mut::<Assets<Image>>()
                    .add(Image::default())
            })
            .collect();
        app.insert_resource(MenuAssets {
            backgrounds,
            bullet_size: UVec2::splat(128),
            font: Handle::default(),
            title_font: Handle::default(),
        });
        app.world_mut().spawn(Window {
            resolution: [1680, 720].into(),
            ..default()
        });
        (app, path)
    }
    #[test]
    fn pointer_leave_releases_the_ghost_and_keyboard_focus_animates_again() {
        let (mut app, path) = animation_test_app("hover");
        app.add_systems(Update, (mouse_actions, style_buttons).chain());
        let button = app
            .world_mut()
            .spawn((
                Choice {
                    action: Some(Action::Training),
                    order: 0,
                },
                ButtonVisualState::None,
                menu_motion::Hover::default(),
                BackgroundColor(Color::NONE),
            ))
            .id();
        let ghost = app
            .world_mut()
            .spawn((
                TitleGhost,
                ChildOf(button),
                Node::default(),
                TextColor(Color::WHITE),
            ))
            .id();
        let bullet = app
            .world_mut()
            .spawn((TitleBullet, ChildOf(button), ImageNode::default()))
            .id();
        app.update();
        app.world_mut()
            .entity_mut(button)
            .insert(ButtonVisualState::Hovered);
        app.update();
        assert_eq!(
            app.world()
                .get::<menu_motion::Hover>(button)
                .unwrap()
                .progress,
            0.5
        );
        assert_eq!(app.world().get::<Node>(ghost).unwrap().left, px(36.5 * 1.5));
        app.world_mut()
            .entity_mut(button)
            .insert(ButtonVisualState::None);
        app.update();
        assert!(
            app.world()
                .get::<menu_motion::Hover>(button)
                .unwrap()
                .progress
                < 0.5
        );
        for _ in 0..3 {
            app.update();
        }
        assert_eq!(
            app.world()
                .get::<menu_motion::Hover>(button)
                .unwrap()
                .progress,
            0.0
        );
        app.world_mut().resource_mut::<Focus>().1 = false;
        app.update();
        app.update();
        assert_eq!(
            app.world()
                .get::<menu_motion::Hover>(button)
                .unwrap()
                .progress,
            1.0
        );
        app.world_mut()
            .entity_mut(button)
            .insert(ButtonVisualState::Pressed);
        app.update();
        assert_eq!(
            app.world().get::<ImageNode>(bullet).unwrap().rect,
            Some(Rect::new(0.0, 64.0, 63.0, 127.0))
        );
        assert_eq!(
            app.world().get::<BackgroundColor>(button).unwrap().0,
            Color::NONE
        );
        std::fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn background_setting_freezes_both_layers_and_resumes_without_a_jump() {
        let (mut app, path) = animation_test_app("fade");
        app.add_systems(Update, animate_background);
        let bottom = app
            .world_mut()
            .spawn((Background(false), ImageNode::default()))
            .id();
        let top = app
            .world_mut()
            .spawn((Background(true), ImageNode::default()))
            .id();
        app.update();
        for _ in 0..10 {
            app.update();
        }
        assert_eq!(
            app.world().get::<ImageNode>(top).unwrap().color.alpha(),
            0.5
        );
        let background = &app.world().resource::<MenuAssets>().backgrounds;
        assert_eq!(
            app.world().get::<ImageNode>(bottom).unwrap().image,
            background[4]
        );
        assert_eq!(
            app.world().get::<ImageNode>(top).unwrap().image,
            background[3]
        );
        app.world_mut()
            .resource_mut::<Context>()
            .saved
            .animate_background = false;
        for _ in 0..30 {
            app.update();
        }
        assert_eq!(
            app.world().get::<ImageNode>(top).unwrap().color.alpha(),
            0.5
        );
        app.world_mut()
            .resource_mut::<Context>()
            .saved
            .animate_background = true;
        app.update();
        assert!((app.world().get::<ImageNode>(top).unwrap().color.alpha() - 0.55).abs() < 0.00001);
        app.world_mut().resource_mut::<Capture>().fixed_frame = Some(2);
        app.update();
        assert_eq!(
            app.world().get::<ImageNode>(top).unwrap().color.alpha(),
            0.0
        );
        assert_eq!(
            app.world().get::<ImageNode>(bottom).unwrap().image,
            app.world().resource::<MenuAssets>().backgrounds[2]
        );
        std::fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn movie_skip_keys_and_mouse_do_not_activate_menu_focus() {
        for key in [
            Some(KeyCode::Escape),
            Some(KeyCode::Enter),
            Some(KeyCode::Space),
            None,
        ] {
            let mut app = App::new();
            app.add_plugins((MinimalPlugins, bevy::state::app::StatesPlugin))
                .insert_state(Screen::Movie)
                .init_resource::<ButtonInput<KeyCode>>()
                .init_resource::<ButtonInput<MouseButton>>()
                .insert_resource(Focus(Some(Action::Training), false))
                .insert_resource(Capture {
                    target: Screen::Movie,
                    path: None,
                    frames: 0,
                    boot_pause: false,
                    fixed_frame: None,
                    ignore_input: false,
                })
                .add_message::<ActionRequest>()
                .add_systems(Update, (keyboard, movie_click).chain());
            app.update();
            if let Some(key) = key {
                app.world_mut()
                    .resource_mut::<ButtonInput<KeyCode>>()
                    .press(key);
            } else {
                app.world_mut()
                    .resource_mut::<ButtonInput<MouseButton>>()
                    .press(MouseButton::Left);
            }
            app.update();
            let messages = app.world().resource::<Messages<ActionRequest>>();
            let mut cursor = messages.get_cursor();
            let actions = cursor.read(messages).map(|m| m.0).collect::<Vec<_>>();
            assert_eq!(actions, [Action::Back]);
            app.world_mut().resource_mut::<Capture>().ignore_input = true;
            app.update();
            assert_eq!(
                cursor
                    .read(app.world().resource::<Messages<ActionRequest>>())
                    .count(),
                0
            );
        }
    }
    #[test]
    fn actual_camera_switching_leaves_one_active_view_after_state_changes() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::state::app::StatesPlugin))
            .init_state::<Screen>()
            .add_systems(Update, apply_camera);
        let menu = app.world_mut().spawn((Camera::default(), MenuCamera)).id();
        let map = app
            .world_mut()
            .spawn((Camera::default(), training::MapCamera))
            .id();
        for screen in [
            Screen::Main,
            Screen::Loading,
            Screen::Training,
            Screen::Pause,
            Screen::Options,
            Screen::Pause,
            Screen::Training,
            Screen::Main,
        ] {
            app.world_mut()
                .resource_mut::<NextState<Screen>>()
                .set(screen);
            app.update();
            let training = matches!(screen, Screen::Training | Screen::Pause);
            assert_eq!(app.world().get::<Camera>(map).unwrap().is_active, training);
            assert_eq!(
                app.world().get::<Camera>(menu).unwrap().is_active,
                !training
            );
            assert_eq!(
                app.world_mut()
                    .query::<&Camera>()
                    .iter(app.world())
                    .filter(|c| c.is_active)
                    .count(),
                1
            );
        }
    }
    #[test]
    fn initial_screens_exist_before_startup_and_adapt_to_modern_resolutions() {
        let path = std::env::temp_dir().join(format!("shadow-menu-startup-{}", std::process::id()));
        std::fs::create_dir_all(path.join("Data")).unwrap();
        let mut gut = b"test\x1a\xfa\x31\xc1\xde\xed\x42\x13".to_vec();
        gut.extend(0_u32.to_le_bytes());
        gut.extend([0; 32]);
        std::fs::write(path.join("Data/test.gut"), gut).unwrap();
        let catalog = Arc::new(AssetCatalog::open(&path, None).unwrap());
        for width in [640, 854] {
            let mut source = format!(
                "WINDOW_BASE_DX {width}\nWINDOW_BASE_DY 480\nDEFINE_USER_IVAR button_offset_x 30\nDEFINE_USER_IVAR button_offset_y 5\nGEOMETRY_JPG_NAME frame1\n"
            );
            for (i, name) in [
                "b_new_game",
                "b_load_game",
                "b_training",
                "b_options",
                "b_intro",
                "b_multiplayer",
                "b_exit",
            ]
            .iter()
            .enumerate()
            {
                source.push_str(&format!(
                    "DEFINE_BUTTON_ADVICE {name} {} {} 0 0\n",
                    width - 315 - i * 5,
                    80 + i * 40
                ));
            }
            for screen in [
                Screen::Main,
                Screen::Options,
                Screen::NewGame,
                Screen::LoadGame,
                Screen::Multiplayer,
                Screen::Quit,
            ] {
                let mut app = App::new();
                app.add_plugins((MinimalPlugins, bevy::state::app::StatesPlugin))
                    .insert_state(screen)
                    .init_resource::<ButtonInput<KeyCode>>()
                    .init_resource::<ButtonInput<MouseButton>>()
                    .init_resource::<Focus>()
                    .init_resource::<pages::Navigation>()
                    .init_resource::<Assets<Image>>()
                    .insert_resource(Context {
                        catalog: catalog.clone(),
                        layout: MenuLayout::from_bytes(source.as_bytes()).unwrap(),
                        settings_path: path.join("settings.json"),
                        saved: Preferences::default(),
                        draft: Preferences::default(),
                        notice: String::new(),
                        frame: 0,
                        options_return: Screen::Main,
                    });
                let image = app
                    .world_mut()
                    .resource_mut::<Assets<Image>>()
                    .add(Image::default());
                app.insert_resource(MenuAssets {
                    backgrounds: vec![image.clone()],
                    bullet_size: UVec2::splat(128),
                    font: Handle::default(),
                    title_font: Handle::default(),
                });
                let window = app
                    .world_mut()
                    .spawn(Window {
                        resolution: [1280, 720].into(),
                        ..default()
                    })
                    .id();
                install_menu_camera(&mut app);
                add_screens(&mut app);
                app.add_systems(Update, fit_ui);
                app.update();
                assert_eq!(
                    app.world_mut()
                        .query_filtered::<Entity, With<Canvas>>()
                        .iter(app.world())
                        .count(),
                    1
                );
                assert_eq!(
                    app.world_mut().query::<&Choice>().iter(app.world()).count(),
                    match screen {
                        Screen::Options => 9,
                        Screen::LoadGame | Screen::Quit => 2,
                        Screen::Multiplayer => 1,
                        _ => 7,
                    }
                );
                let enabled = app
                    .world_mut()
                    .query_filtered::<Entity, With<UiButton>>()
                    .iter(app.world())
                    .count();
                assert_eq!(
                    enabled,
                    match screen {
                        Screen::Options => 9,
                        Screen::LoadGame | Screen::Quit => 2,
                        Screen::Multiplayer => 1,
                        _ => 7,
                    }
                );
                for &[w, h] in RESOLUTIONS {
                    app.world_mut()
                        .get_mut::<Window>(window)
                        .unwrap()
                        .resolution
                        .set(w as f32, h as f32);
                    app.update();
                    for node in app
                        .world_mut()
                        .query_filtered::<&Node, With<Canvas>>()
                        .iter(app.world())
                    {
                        assert_eq!(
                            (node.left, node.top, node.width, node.height),
                            (px(0), px(0), px(w as f32), px(h as f32))
                        );
                    }
                    for (_choice, node) in app
                        .world_mut()
                        .query::<(&Choice, &Node)>()
                        .iter(app.world())
                    {
                        let (Val::Px(left), Val::Px(top), Val::Px(size), Val::Px(height)) =
                            (node.left, node.top, node.width, node.height)
                        else {
                            panic!("pixel layout expected")
                        };
                        assert!(
                            left >= 0.0
                                && top >= 0.0
                                && left + size <= w as f32 + 0.01
                                && top + height <= h as f32 + 0.01
                        );
                        if screen == Screen::Main {
                            assert!(
                                (w as f32 - left - size - 10.0 * h as f32 / 480.0).abs() < 0.01
                            );
                        }
                    }
                }
            }
        }
        std::fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn keyboard_navigation_skips_disabled_entries_and_wraps() {
        let actions = ordered_actions(
            [
                (6, Some(Action::Exit)),
                (0, None),
                (2, Some(Action::Training)),
                (3, Some(Action::Options)),
                (5, None),
            ]
            .into_iter(),
        );
        assert_eq!(actions, [Action::Training, Action::Options, Action::Exit]);
        assert_eq!(
            move_focus(&actions, Some(Action::Exit), false),
            Some(Action::Training)
        );
        assert_eq!(
            move_focus(&actions, Some(Action::Training), true),
            Some(Action::Exit)
        );
        assert_eq!(move_focus(&[], None, false), None);
    }
    #[test]
    fn mouse_requires_a_new_press_and_screenshot_ignores_live_input() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<Focus>()
            .init_resource::<pages::Navigation>()
            .add_message::<ActionRequest>()
            .insert_resource(Capture {
                target: Screen::Main,
                path: None,
                frames: 0,
                boot_pause: false,
                fixed_frame: None,
                ignore_input: false,
            })
            .add_systems(Update, mouse_actions);
        let button = app
            .world_mut()
            .spawn((
                Choice {
                    action: Some(Action::Training),
                    order: 0,
                },
                ButtonVisualState::Pressed,
            ))
            .id();
        app.update();
        assert_eq!(app.world().resource::<Messages<ActionRequest>>().len(), 0);
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app.world_mut()
            .entity_mut(button)
            .insert(ButtonVisualState::None);
        app.update();
        app.world_mut()
            .entity_mut(button)
            .insert(ButtonVisualState::Pressed);
        app.update();
        let messages = app.world().resource::<Messages<ActionRequest>>();
        let mut cursor = messages.get_cursor();
        assert_eq!(cursor.read(messages).next().unwrap().0, Action::Training);
        // Зажатая кнопка над вновь созданной UI-кнопкой не повторяет действие.
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .clear();
        app.world_mut().spawn((
            Choice {
                action: Some(Action::Options),
                order: 1,
            },
            ButtonVisualState::Pressed,
        ));
        app.update();
        assert_eq!(
            cursor
                .read(app.world().resource::<Messages<ActionRequest>>())
                .count(),
            0
        );
        app.world_mut().resource_mut::<Capture>().ignore_input = true;
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .reset_all();
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app.world_mut()
            .entity_mut(button)
            .insert(ButtonVisualState::None);
        app.update();
        app.world_mut()
            .entity_mut(button)
            .insert(ButtonVisualState::Pressed);
        app.update();
        // Проверяем новые сообщения, а не срок хранения прежнего сообщения в буфере.
        assert_eq!(
            cursor
                .read(app.world().resource::<Messages<ActionRequest>>())
                .count(),
            0
        );
    }
    #[test]
    fn actual_keyboard_system_traverses_only_available_menu_actions() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::state::app::StatesPlugin))
            .init_state::<Screen>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<Focus>()
            .init_resource::<pages::Navigation>()
            .add_message::<ActionRequest>()
            .insert_resource(Capture {
                target: Screen::Main,
                path: None,
                frames: 0,
                boot_pause: false,
                fixed_frame: None,
                ignore_input: false,
            })
            .add_systems(Update, keyboard);
        for (order, action) in [
            None,
            Some(Action::Training),
            None,
            Some(Action::Options),
            Some(Action::Exit),
        ]
        .into_iter()
        .enumerate()
        {
            app.world_mut().spawn(Choice { order, action });
        }
        app.world_mut().resource_mut::<Focus>().0 = Some(Action::Training);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::ArrowDown);
        app.update();
        assert_eq!(app.world().resource::<Focus>().0, Some(Action::Options));
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .reset_all();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Enter);
        app.update();
        let actions = app.world().resource::<Messages<ActionRequest>>();
        let mut cursor = actions.get_cursor();
        assert_eq!(cursor.read(actions).next().unwrap().0, Action::Options);
    }
    #[test]
    fn actual_options_handler_saves_applied_changes_and_cancels_draft() {
        let path = std::env::temp_dir().join(format!("shadow-menu-actions-{}", std::process::id()));
        std::fs::create_dir_all(path.join("Data")).unwrap();
        let mut gut = b"test\x1a\xfa\x31\xc1\xde\xed\x42\x13".to_vec();
        gut.extend(0_u32.to_le_bytes());
        gut.extend([0; 32]);
        std::fs::write(path.join("Data/test.gut"), gut).unwrap();
        let catalog = Arc::new(AssetCatalog::open(&path, None).unwrap());
        let layout=MenuLayout::from_bytes(b"WINDOW_BASE_DX 640\nWINDOW_BASE_DY 480\nDEFINE_BUTTON_ADVICE b_training 100 100 0 0\nDEFINE_BUTTON_ADVICE b_exit 100 200 0 0\nGEOMETRY_JPG_NAME frame1\n").unwrap();
        let settings = path.join("settings.json");
        let saved = Preferences::default();
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::state::app::StatesPlugin))
            .init_state::<Screen>()
            .add_message::<ActionRequest>()
            .insert_resource(Context {
                catalog,
                layout,
                settings_path: settings.clone(),
                saved: saved.clone(),
                draft: saved,
                notice: String::new(),
                frame: 0,
                options_return: Screen::Main,
            })
            .init_resource::<pages::Navigation>()
            .add_systems(Update, handle_actions);
        app.world_mut().spawn(Window::default());
        let action = |app: &mut App, action| {
            app.world_mut()
                .resource_mut::<Messages<ActionRequest>>()
                .write(ActionRequest(action));
            app.update();
            app.update();
        };
        for (route, screen) in [
            (Action::NewGame, Screen::NewGame),
            (Action::LoadGame, Screen::LoadGame),
            (Action::Multiplayer, Screen::Multiplayer),
            (Action::Exit, Screen::Quit),
        ] {
            action(&mut app, route);
            assert_eq!(*app.world().resource::<State<Screen>>().get(), screen);
            action(&mut app, Action::Back);
            assert_eq!(*app.world().resource::<State<Screen>>().get(), Screen::Main);
        }
        action(&mut app, Action::Options);
        assert_eq!(
            *app.world().resource::<State<Screen>>().get(),
            Screen::Options
        );
        action(&mut app, Action::Language);
        action(&mut app, Action::TextSize);
        action(&mut app, Action::Resolution);
        action(&mut app, Action::Window);
        action(&mut app, Action::AudioTab);
        assert_eq!(app.world().resource::<pages::Navigation>().options, 1);
        action(&mut app, Action::MasterVolume);
        action(&mut app, Action::MouseTab);
        action(&mut app, Action::CameraSpeed);
        action(&mut app, Action::EdgeScroll);
        action(&mut app, Action::Apply);
        let saved = Preferences::load(&settings).unwrap();
        assert_eq!(saved.master_volume, 0.0);
        assert_eq!(saved.camera_speed, 1.25);
        assert!(!saved.edge_scroll);
        assert_eq!(app.world().resource::<Preferences>(), &saved);
        assert!(saved.english && saved.fullscreen);
        assert_eq!(saved.text_scale, 1.25);
        assert_eq!(saved.window_size, [1366, 768]);
        assert_eq!(app.world().resource::<Context>().saved, saved);
        action(&mut app, Action::Language);
        action(&mut app, Action::Back);
        assert_eq!(*app.world().resource::<State<Screen>>().get(), Screen::Main);
        assert_eq!(Preferences::load(&settings).unwrap(), saved);
        action(&mut app, Action::Training);
        assert_eq!(
            *app.world().resource::<State<Screen>>().get(),
            Screen::Loading
        );
        action(&mut app, Action::Back);
        assert_eq!(*app.world().resource::<State<Screen>>().get(), Screen::Main);
        std::fs::remove_dir_all(path).unwrap();
    }
}
