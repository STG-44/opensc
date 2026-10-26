//! Общая учебная сцена для витрины и игры с меню.
use crate::{combat, fence, grenade, inventory, map_scene, orders, pickup, player, tutorial};
use bevy::{
    input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll},
    prelude::*,
    render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk},
    text::FontSize,
};
use shadow_formats::{AssetCatalog, HelpDefinitions, MissionDefinition, normalize_path};
use std::{env, error::Error, path::PathBuf};
struct Config {
    game: PathBuf,
    overrides: Option<PathBuf>,
    map: String,
    terrain: String,
    screenshot: Option<PathBuf>,
    scene_json: Option<PathBuf>,
    camp: bool,
    player: bool,
    player_action: String,
    player_seconds: Option<f32>,
    player_json: Option<PathBuf>,
    smoke: Option<PathBuf>,
    medical_smoke: Option<PathBuf>,
    armor_smoke: Option<PathBuf>,
    vehicle_smoke: Option<PathBuf>,
    gate_smoke: Option<PathBuf>,
    jack_smoke: Option<PathBuf>,
    evacuation_smoke: Option<PathBuf>,
    selection_smoke: Option<PathBuf>,
    combat_smoke: bool,
    bda_smoke: bool,
    grenade_smoke: bool,
    fence_smoke: bool,
    smg_smoke: bool,
    window: (u32, u32),
    language: crate::localization::Language,
}
#[derive(Resource)]
pub(crate) struct Preview {
    pub(crate) scene: Option<map_scene::MapScene>,
    pub(crate) screenshot: Option<PathBuf>,
    pub(crate) focus: Vec3,
    pub(crate) span: f32,
    pub(crate) grid: Option<shadow_formats::TerrainGrid>,
    pub(crate) playable: bool,
}
#[derive(Resource)]
pub(crate) struct Orbit {
    pub(crate) target: Vec3,
    pub(crate) yaw: f32,
    pub(crate) pitch: f32,
    pub(crate) distance: f32,
}
#[derive(Component)]
pub(crate) struct MapCamera;
#[derive(Resource)]
pub(crate) struct CameraOptions {
    pub speed: f32,
    pub edge_scroll: bool,
}
pub(crate) fn run() -> Result<(), Box<dyn Error>> {
    let Some(config) = config()? else {
        return Ok(());
    };
    let catalog = AssetCatalog::open(&config.game, config.overrides.as_deref())?;
    let scene = map_scene::prepare(&catalog, &config.map, &config.terrain)?;
    println!(
        "Карта {}: {} статических объектов, {} моделей, рельеф {} треугольников",
        config.map,
        scene.instances.len(),
        scene.models.len(),
        scene.report["terrain"]["triangles"]
    );
    if let Some(path) = config.scene_json {
        std::fs::write(path, serde_json::to_string_pretty(&scene.report)? + "\n")?;
        return Ok(());
    }
    let focus = scene.focus;
    let span = scene.span;
    let mut actor = if config.player {
        let mission = training_definition(&catalog)?;
        if normalize_path(&config.map)? != format!("maps/{}", mission.map) {
            return Err("управляемый режим пока поддерживает карту сценария обучения".into());
        }
        Some(player::prepare(
            &catalog,
            &scene.grid,
            player::initial_actor(&mission, &scene.actors)?,
        )?)
    } else {
        None
    };
    let mut item = actor
        .as_mut()
        .map(|a| pickup::Pickup::prepare(&catalog, &scene, a))
        .transpose()?;
    let mut combat = actor
        .as_mut()
        .map(|a| combat::Combat::prepare(&catalog, &scene, &training_definition(&catalog)?, a))
        .transpose()?;
    let mut inventory = actor
        .as_mut()
        .map(|p| inventory::Inventory::prepare(&catalog, p))
        .transpose()?;
    let mut grenade = actor
        .as_mut()
        .map(|p| grenade::Grenade::prepare(&catalog, p))
        .transpose()?;
    let mut fence = if let (Some(p), Some(inv)) = (&mut actor, &mut inventory) {
        Some(fence::Fence::prepare(&catalog, &scene, p, inv)?)
    } else {
        None
    };
    if let Some(p) = &mut actor {
        crate::stance::prepare(&catalog, p)?;
    }
    let mut uzi = if let Some(inv) = &inventory {
        Some(crate::smg::Uzi::prepare(&catalog, &scene, inv)?)
    } else {
        None
    };
    let mut medical = if let Some(p) = &mut actor {
        Some(crate::medical::Medical::prepare(&catalog, &scene, p)?)
    } else {
        None
    };
    let mut armor = if let Some(p) = &mut actor {
        Some(crate::armor::Armor::prepare(&catalog, &scene, p)?)
    } else {
        None
    };
    let mut vehicle = actor
        .as_ref()
        .map(|_| crate::vehicle::Vehicle::prepare(&catalog, &scene))
        .transpose()?;
    let mut party = actor
        .as_ref()
        .map(|_| crate::party::Party::prepare(&catalog, &scene, &training_definition(&catalog)?))
        .transpose()?;
    let roster = if let (Some(john), Some(party)) = (&actor, &party) {
        Some(crate::roster::Roster::prepare(&catalog, john, &party.jack)?)
    } else {
        None
    };
    let hud = actor
        .as_ref()
        .map(|_| crate::hud::Hud::prepare(&catalog))
        .transpose()?;
    let evacuation = if let (Some(john), Some(party)) = (&mut actor, &mut party) {
        Some(crate::evacuation::Evacuation::prepare(
            &catalog,
            &training_definition(&catalog)?,
            john,
            party,
        )?)
    } else {
        None
    };
    let gate = actor
        .as_ref()
        .map(|_| crate::gate::Gate::prepare(&catalog, &scene))
        .transpose()?;
    if let (Some(v), Some(g)) = (&mut vehicle, &gate) {
        v.replace_obstacles(g.id, g.vehicle_bounds());
    }
    let mut tutorial = if actor.is_some() {
        Some(tutorial::Tutorial::new(
            training_definition(&catalog)?,
            HelpDefinitions::from_bytes(&catalog.read("config/help_window_defs.txt")?)?,
        )?)
    } else {
        None
    };
    if let Some(actor) = &mut actor {
        let seconds = config.player_seconds.unwrap_or(0.0);
        if [
            "pickup", "stab", "shoot", "grenade", "fence", "smg", "medical", "armor", "vehicle",
        ]
        .contains(&config.player_action.as_str())
        {
            item.as_mut().unwrap().order(actor, &scene.grid);
        } else {
            actor.preview_action(&config.player_action, &scene.grid);
        }
        let mut trace = vec![actor.position.to_array()];
        let steps = (seconds * 60.0).ceil() as usize;
        for i in 0..steps {
            let dt = (seconds - i as f32 / 60.0).clamp(0.0, 1.0 / 60.0);
            let t = tutorial.as_mut().unwrap();
            // Числовой пример автоматически подтверждает окна; интерактивный запуск — кнопкой OK.
            if t.active.is_some() {
                t.close();
            }
            let previous = actor.position;
            if !vehicle.as_ref().unwrap().occupied {
                actor.step(&ButtonInput::default(), dt, &scene.grid);
                fence.as_mut().unwrap().guard_movement(actor, previous);
            }
            item.as_mut().unwrap().update(actor, t);
            if [
                "stab", "shoot", "grenade", "fence", "smg", "medical", "armor", "vehicle",
            ]
            .contains(&config.player_action.as_str())
                && item.as_ref().unwrap().collected
                && actor.pickup_seconds.is_none()
                && combat.as_ref().unwrap().alive()
                && !combat.as_ref().unwrap().ordered
            {
                combat.as_mut().unwrap().order(actor, &scene.grid);
            }
            let inv = inventory.as_mut().unwrap();
            let c = combat.as_mut().unwrap();
            inv.observe(item.as_ref().unwrap(), c);
            inv.update(actor, t, item.as_ref().unwrap().callback_seconds);
            if [
                "shoot", "grenade", "fence", "smg", "medical", "armor", "vehicle",
            ]
            .contains(&config.player_action.as_str())
                && !c.alive()
                && actor.action.is_none()
            {
                if !inv.collected && !inv.ordered {
                    inv.order(actor, &scene.grid);
                }
                if inv.collected
                    && actor.pickup_seconds.is_none()
                    && c.guard_at(1).alive()
                    && !c.has_order()
                {
                    inv.equip(inventory::Weapon::Bda, actor);
                    c.order_target(1, actor, &scene.grid);
                }
            }
            c.update_with_inventory(actor, t, item.as_ref().unwrap(), inv, dt);
            let g = grenade.as_mut().unwrap();
            g.observe(c);
            if ["grenade", "fence", "smg", "medical", "armor", "vehicle"]
                .contains(&config.player_action.as_str())
                && !c.guard_at(1).alive()
                && actor.action.is_none()
            {
                if !g.collected && !g.pickup_ordered {
                    g.order_pickup(actor, &scene.grid);
                }
                if g.collected
                    && actor.pickup_seconds.is_none()
                    && c.guard_at(2).alive()
                    && !g.has_order()
                    && g.flight.is_none()
                {
                    g.order_throw(
                        c.guard_at(2).guard.position,
                        Some(2),
                        false,
                        actor,
                        &scene.grid,
                    );
                }
            }
            g.update(
                actor,
                c,
                t,
                item.as_ref().unwrap(),
                inv,
                &scene.grid,
                dt,
                item.as_ref().unwrap().callback_seconds,
            );
            let f = fence.as_mut().unwrap();
            if ["fence", "smg", "medical", "armor", "vehicle"]
                .contains(&config.player_action.as_str())
                && !c.guard_at(2).alive()
                && actor.action.is_none()
                && actor.pickup_seconds.is_none()
                && !f.has_order()
            {
                if !f.collected {
                    f.order_pickup(actor, &scene.grid);
                } else if !f.cut || f.crossings == 0 {
                    f.order_fence(actor, &scene.grid);
                }
            }
            f.update(actor, inv, t, item.as_ref().unwrap(), &scene.grid, dt);
            let u = uzi.as_mut().unwrap();
            if ["smg", "medical", "armor", "vehicle"].contains(&config.player_action.as_str())
                && f.crossings > 0
                && !f.has_order()
                && actor.action.is_none()
                && actor.pickup_seconds.is_none()
            {
                if !u.collected {
                    if !u.ordered {
                        actor.request_posture(crate::stance::Posture::Crouch);
                        u.order(actor, &scene.grid);
                    }
                } else if !actor.changing_posture() {
                    inv.equip(inventory::Weapon::Uzi, actor);
                    if !c.has_order()
                        && let Some(index) = (3..c.guard_count()).find(|&i| c.guard_at(i).alive())
                    {
                        c.order_target(index, actor, &scene.grid);
                    }
                }
            }
            u.update(actor, inv, t, item.as_ref().unwrap().callback_seconds);
            let m = medical.as_mut().unwrap();
            if ["medical", "armor", "vehicle"].contains(&config.player_action.as_str())
                && c.smg.iter().all(|g| !g.alive())
                && !c.has_order()
                && actor.action.is_none()
                && actor.pickup_seconds.is_none()
                && !actor.changing_posture()
            {
                if !m.ground.collected {
                    if !m.ground.ordered {
                        m.ground.order(actor, &scene.grid);
                    }
                } else if m.arm() {
                    m.treat(actor);
                }
            }
            m.update(actor, inv, t, item.as_ref().unwrap().callback_seconds);
            let a = armor.as_mut().unwrap();
            if ["armor", "vehicle"].contains(&config.player_action.as_str())
                && m.health == m.maximum
                && m.ground.collected
                && m.treatments > 0
                && !m.ground.ordered
                && actor.action.is_none()
                && actor.pickup_seconds.is_none()
                && !actor.changing_posture()
                && !a.ground.collected
                && !a.ground.ordered
            {
                a.ground.order(actor, &scene.grid);
            }
            a.update(actor, inv, t, item.as_ref().unwrap().callback_seconds);
            let v = vehicle.as_mut().unwrap();
            if config.player_action == "vehicle"
                && a.ground.collected
                && !a.ground.ordered
                && actor.action.is_none()
                && actor.pickup_seconds.is_none()
                && !actor.changing_posture()
                && !v.occupied
                && !v.approaching
                && v.entries == 0
            {
                v.order(actor, &scene.grid);
            }
            v.board(actor, t, &scene.grid);
            t.tick(f64::from(dt), false);
            trace.push(actor.position.to_array());
        }
        combat.as_mut().unwrap().pending_shots.clear();
        grenade.as_mut().unwrap().suppress_pending_audio();
        if let Some(m) = &mut medical {
            m.suppress_audio();
        }
        actor.playback.paused = config.player_seconds.is_some();
        actor.follow_camera = false;
        if let Some(path) = &config.player_json {
            let mut player_report = actor.report();
            player_report["obstacle_collisions"] =
                serde_json::json!("Structure_Fence and dynamic swing gate");
            let value = serde_json::json!({"map":config.map,"player":player_report,"action":config.player_action,
                "simulated_seconds":seconds,"step_seconds":1.0/60.0,"trace_metres":trace,"pickup":item.as_ref().map(|i|i.report()),"tutorial":tutorial.as_ref().map(|t|t.report()),"combat":combat.as_ref().map(|c|c.report()),"inventory":inventory.as_ref().map(|inv|inv.report(actor)),"grenade":grenade.as_ref().map(|g|g.report()),"fence":fence.as_ref().map(|f|f.report()),"uzi_pickup":uzi.as_ref().map(|u|u.report()),"medical":medical.as_ref().map(|m|m.report()),"armor":armor.as_ref().map(|a|a.report()),"vehicle":vehicle.as_ref().map(|v|v.report()),"gate":gate.as_ref().map(|g|g.report()),"party":party.as_ref().map(|g|g.report(actor)),"evacuation":evacuation.as_ref().map(|e|e.report()),"roster":roster.as_ref().map(|r|r.report()),"hud":hud.as_ref().map(|h|h.report())});
            std::fs::write(path, serde_json::to_string_pretty(&value)? + "\n")?;
            return Ok(());
        }
    }
    let orbit = if config.camp {
        camp_view()
    } else if let Some(actor) = &actor {
        player_view(actor.camera_target())
    } else {
        overview(focus, span)
    };
    let mut app = App::new();
    app.insert_resource(ClearColor(Color::srgb(0.07, 0.10, 0.085)))
        .insert_resource(Preview {
            scene: Some(scene),
            screenshot: if config.smoke.is_some()
                || config.medical_smoke.is_some()
                || config.armor_smoke.is_some()
                || config.vehicle_smoke.is_some()
                || config.gate_smoke.is_some()
                || config.jack_smoke.is_some()
                || config.evacuation_smoke.is_some()
                || config.selection_smoke.is_some()
            {
                None
            } else {
                config.screenshot.clone()
            },
            focus,
            span,
            grid: None,
            playable: actor.is_some(),
        })
        .insert_resource(orbit)
        .init_resource::<orders::ClickHistory>()
        .init_resource::<crate::selection::Selection>()
        .add_plugins(
            DefaultPlugins.set(WindowPlugin {
                primary_window: Some(Window {
                    title: if config.language == crate::localization::Language::English {
                        "Shadow Company · training map / Rust + Bevy"
                    } else {
                        "Shadow Company · учебная карта / Rust + Bevy"
                    }
                    .into(),
                    resolution: config.window.into(),
                    ..default()
                }),
                ..default()
            }),
        )
        .insert_resource(config.language)
        .add_plugins(crate::localization::LocalizationPlugin)
        .add_systems(
            Startup,
            (
                setup,
                player::setup,
                combat::setup,
                inventory::setup,
                grenade::setup,
                fence::setup,
                crate::smg::setup,
                crate::medical::setup,
                crate::armor::setup,
                crate::vehicle::setup,
                crate::party::setup,
                crate::hud::setup,
                crate::roster::setup,
                crate::selection::setup,
                crate::evacuation::setup,
            )
                .chain(),
        )
        .add_systems(
            Update,
            (
                (
                    (
                        crate::ui_controls::sync,
                        pickup::smoke_input,
                        crate::medical::native_smoke,
                        crate::armor::native_smoke,
                        crate::vehicle::native_smoke,
                    )
                        .chain(),
                    (
                        crate::gate::native_smoke,
                        crate::party::native_smoke,
                        crate::evacuation::native_smoke,
                        crate::selection::native_smoke,
                    )
                        .chain(),
                    (
                        tutorial::input,
                        crate::selection::input,
                        crate::hud::input,
                        crate::roster::input,
                        crate::evacuation::input,
                    )
                        .chain(),
                    inventory::input
                        .run_if(crate::vehicle::on_foot_or_reset)
                        .run_if(crate::party::john_controls),
                    crate::party::input,
                    crate::vehicle::input,
                    crate::armor::input
                        .run_if(crate::vehicle::on_foot_or_reset)
                        .run_if(crate::party::john_controls),
                    crate::medical::input
                        .run_if(crate::vehicle::on_foot_or_reset)
                        .run_if(crate::party::john_controls),
                    grenade::input
                        .run_if(crate::vehicle::on_foot_or_reset)
                        .run_if(crate::party::john_controls),
                    fence::input
                        .run_if(crate::vehicle::on_foot_or_reset)
                        .run_if(crate::party::john_controls),
                    crate::smg::input
                        .run_if(crate::vehicle::on_foot_or_reset)
                        .run_if(crate::party::john_controls),
                    crate::stance::input
                        .run_if(crate::vehicle::on_foot_or_reset)
                        .run_if(crate::party::john_controls),
                    pickup::input
                        .run_if(crate::vehicle::on_foot_or_reset)
                        .run_if(crate::party::john_controls),
                    combat::input
                        .run_if(crate::vehicle::on_foot_or_reset)
                        .run_if(crate::party::john_controls),
                    controls,
                    issue_orders,
                    update_player,
                )
                    .chain(),
                (
                    (crate::gate::tick, crate::vehicle::tick).chain(),
                    pickup::sync,
                    combat::traces,
                    combat::sync,
                    inventory::sync,
                    grenade::sync,
                    fence::sync,
                    crate::smg::sync,
                    crate::medical::sync,
                    crate::armor::sync,
                    crate::medical::hud,
                    (
                        tutorial::advance,
                        crate::party::tick,
                        crate::party::audio,
                        crate::evacuation::advance,
                        crate::evacuation::observe_end,
                    )
                        .chain(),
                    tutorial::ui,
                    inventory::ui,
                    pickup::ring,
                    update_markers,
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
                    crate::hud::diagnostics,
                    orbit_camera,
                    capture,
                )
                    .chain(),
            )
                .chain(),
        );
    app.add_systems(Last, crate::hud::release_keys);
    if let Some(h) = hud {
        app.insert_resource(h);
    }
    if let Some(r) = roster {
        app.insert_resource(r);
    }
    if let Some(e) = evacuation {
        app.insert_resource(e);
    }
    if let Some(party) = party {
        app.insert_resource(party);
    }
    if let Some(g) = gate {
        app.insert_resource(g);
    }
    if let Some(v) = vehicle {
        app.insert_resource(v);
    }
    if let Some(a) = armor {
        app.insert_resource(a);
    }
    if let Some(m) = medical {
        app.insert_resource(m);
    }
    if let Some(actor) = actor {
        app.insert_resource(actor);
    }
    if let Some(item) = item {
        app.insert_resource(item);
    }
    if let Some(combat) = combat {
        app.insert_resource(combat);
    }
    if let Some(inventory) = inventory {
        app.insert_resource(inventory);
    }
    if let Some(tutorial) = tutorial {
        app.insert_resource(tutorial);
    }
    if let Some(g) = grenade {
        app.insert_resource(g);
    }
    if let Some(f) = fence {
        app.insert_resource(f);
    }
    if let Some(u) = uzi {
        app.insert_resource(u);
    }
    if let Some(path) = config.selection_smoke {
        app.insert_resource(crate::selection::NativeSmoke::new(
            path,
            config.screenshot.clone(),
        ));
    }
    if let Some(path) = config.evacuation_smoke {
        app.insert_resource(crate::evacuation::NativeSmoke::new(
            path,
            config.screenshot.clone(),
            false,
        ));
    }
    if let Some(path) = config.jack_smoke {
        app.insert_resource(crate::party::NativeSmoke::new(
            path,
            config.screenshot.clone(),
        ));
    }
    if let Some(path) = config.gate_smoke {
        app.insert_resource(crate::gate::NativeSmoke::new(
            path,
            config.screenshot.clone(),
        ));
    }
    if let Some(path) = config.vehicle_smoke {
        app.insert_resource(crate::vehicle::NativeSmoke::new(
            path,
            config.screenshot.clone(),
        ));
    }
    if let Some(path) = config.armor_smoke {
        app.insert_resource(crate::armor::NativeSmoke::new(
            path,
            config.screenshot.clone(),
        ));
    }
    if let Some(path) = config.medical_smoke {
        app.insert_resource(crate::medical::NativeSmoke::new(
            path,
            config.screenshot.clone(),
        ));
    }
    if let Some(path) = config.smoke {
        let mut smoke = pickup::Smoke::new(path, config.screenshot);
        smoke.combat = config.combat_smoke;
        smoke.bda = config.bda_smoke;
        smoke.grenade = config.grenade_smoke;
        smoke.fence = config.fence_smoke;
        smoke.smg = config.smg_smoke;
        app.insert_resource(smoke);
    }
    if !app.run().is_success() {
        return Err("ошибка учебной сцены; см. журнал".into());
    }
    Ok(())
}
fn config() -> Result<Option<Config>, Box<dyn Error>> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    if args.iter().any(|a| a == "--help") {
        println!(
            "shadow-map-viewer --tutorial-smoke-json FILE.json | --combat-smoke-json FILE.json | --bda-smoke-json FILE.json | --grenade-smoke-json FILE.json | --fence-smoke-json FILE.json | --smg-smoke-json FILE.json | --medical-smoke-json FILE.json | --armor-smoke-json FILE.json | --vehicle-smoke-json FILE.json | --gate-smoke-json FILE.json | --jack-smoke-json FILE.json | --evacuation-smoke-json FILE.json | --selection-smoke-json FILE.json [--window 1280x720] [--language ru|en] --game-dir PATH [--overrides PATH] [--map NAME.mtf] [--terrain NAME] [--camera overview|camp] [--player on|off] [--player-action idle|walk|run|wave|pickup|stab|shoot|grenade|fence|smg|medical|armor|vehicle --player-seconds SECONDS] [--player-json FILE.json] [--scene-json FILE.json] [--screenshot FILE.png]\nБоец: ЛКМ — выбор/идти; двойной ЛКМ — бег; ПКМ — снять выбор, удержание — камера; H — остановить; T — слежение; Numpad — камера; Ctrl+P — пауза; Esc — учебная пауза; Ctrl+X — выход. Учебные: Home — начало, [ / ] — кадр. Просмотр карты: WASD — камера, ЛКМ/стрелки — вращение, C — площадка, Esc — выход."
        );
        return Ok(None);
    }
    let mut config = Config {
        game: PathBuf::new(),
        overrides: None,
        map: "maps/training_final.mtf".into(),
        terrain: "training".into(),
        screenshot: None,
        scene_json: None,
        camp: false,
        player: false,
        player_action: "idle".into(),
        player_seconds: None,
        player_json: None,
        smoke: None,
        medical_smoke: None,
        armor_smoke: None,
        vehicle_smoke: None,
        gate_smoke: None,
        jack_smoke: None,
        evacuation_smoke: None,
        selection_smoke: None,
        combat_smoke: false,
        bda_smoke: false,
        grenade_smoke: false,
        fence_smoke: false,
        smg_smoke: false,
        window: (1280, 720),
        language: crate::localization::Language::Russian,
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
            "--language" => config.language = crate::localization::Language::parse(value)?,

            "--game-dir" => game = Some(value.into()),
            "--overrides" => config.overrides = Some(value.into()),
            "--map" => config.map = shadow_formats::normalize_path(value)?,
            "--terrain" => config.terrain = value.to_lowercase(),
            "--screenshot" => config.screenshot = Some(value.into()),
            "--scene-json" => config.scene_json = Some(value.into()),
            "--player-json" => config.player_json = Some(value.into()),
            "--selection-smoke-json" => {
                config.selection_smoke = Some(value.into());
                config.player = true;
            }
            "--evacuation-smoke-json" => {
                config.evacuation_smoke = Some(value.into());
                config.player = true;
            }
            "--jack-smoke-json" => {
                config.jack_smoke = Some(value.into());
                config.player = true;
            }
            "--gate-smoke-json" => {
                config.gate_smoke = Some(value.into());
                config.player = true;
            }
            "--vehicle-smoke-json" => {
                config.vehicle_smoke = Some(value.into());
                config.player = true;
            }
            "--armor-smoke-json" => {
                config.armor_smoke = Some(value.into());
                config.player = true;
            }
            "--medical-smoke-json" => {
                config.medical_smoke = Some(value.into());
                config.player = true;
            }
            "--smg-smoke-json"
            | "--fence-smoke-json"
            | "--grenade-smoke-json"
            | "--bda-smoke-json"
            | "--combat-smoke-json"
            | "--tutorial-smoke-json" => {
                config.combat_smoke = key != "--tutorial-smoke-json";
                config.bda_smoke = [
                    "--bda-smoke-json",
                    "--grenade-smoke-json",
                    "--fence-smoke-json",
                    "--smg-smoke-json",
                ]
                .contains(&key.as_str());
                config.grenade_smoke = [
                    "--grenade-smoke-json",
                    "--fence-smoke-json",
                    "--smg-smoke-json",
                ]
                .contains(&key.as_str());
                config.fence_smoke =
                    ["--fence-smoke-json", "--smg-smoke-json"].contains(&key.as_str());
                config.smg_smoke = key == "--smg-smoke-json";
                config.smoke = Some(value.into());
            }
            "--window" => {
                let (w, h) = value.split_once('x').ok_or("нужен --window WIDTHxHEIGHT")?;
                config.window = (w.parse()?, h.parse()?);
                if config.window.0 < 1280
                    || config.window.1 < 720
                    || config.window.0 > 7680
                    || config.window.1 > 4320
                {
                    return Err("размер окна от 1280x720 до 7680x4320".into());
                }
            }
            "--player" => {
                config.player = match value.as_str() {
                    "on" => true,
                    "off" => false,
                    _ => return Err("--player должен быть on или off".into()),
                }
            }
            "--player-action" => {
                if ![
                    "idle", "walk", "run", "wave", "pickup", "stab", "shoot", "grenade", "fence",
                    "smg", "medical", "armor", "vehicle",
                ]
                .contains(&value.as_str())
                {
                    return Err(
                        "--player-action должен быть idle, walk, run, wave, pickup, stab, shoot, grenade, fence, smg, medical, armor или vehicle"
                            .into(),
                    );
                }
                config.player_action = value.clone();
            }
            "--player-seconds" => {
                let t = value.parse::<f32>()?;
                if !t.is_finite() || !(0.0..=480.0).contains(&t) {
                    return Err("--player-seconds должен быть от 0 до 480".into());
                }
                config.player_seconds = Some(t);
            }
            "--camera" => {
                config.camp = match value.as_str() {
                    "overview" => false,
                    "camp" => true,
                    _ => return Err("--camera должен быть overview или camp".into()),
                }
            }
            _ => return Err(format!("неизвестный аргумент {key}").into()),
        }
    }
    if config.smoke.is_some() {
        config.player = true;
    }
    config.game = game.ok_or("укажи --game-dir или SHADOW_GAME_DIR")?;
    if config.terrain.is_empty()
        || !config
            .terrain
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        return Err("недопустимое имя terrain".into());
    }
    if !config.map.ends_with(".mtf") {
        return Err("--map должен быть .mtf".into());
    }
    if usize::from(config.screenshot.is_some())
        + usize::from(config.scene_json.is_some())
        + usize::from(config.player_json.is_some())
        > 1
    {
        return Err(
            "выбери один выходной файл: --screenshot, --scene-json или --player-json".into(),
        );
    }
    if (config.player_json.is_some()
        || config.player_seconds.is_some()
        || config.player_action != "idle")
        && !config.player
    {
        return Err("для параметров бойца укажи --player on".into());
    }
    if config.player_action != "idle" && config.player_seconds.is_none() {
        return Err("для --player-action укажи --player-seconds".into());
    }
    for (path, ext) in [
        (config.screenshot.as_ref(), "png"),
        (config.scene_json.as_ref(), "json"),
        (config.player_json.as_ref(), "json"),
    ] {
        if let Some(path) = path {
            if path
                .extension()
                .is_none_or(|e| !e.eq_ignore_ascii_case(ext))
            {
                return Err(format!("выходной файл должен быть .{ext}").into());
            }
            if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                std::fs::create_dir_all(parent)?;
            }
        }
    }
    Ok(Some(config))
}
fn overview(focus: Vec3, span: f32) -> Orbit {
    Orbit {
        target: focus,
        yaw: -2.7,
        pitch: 0.8,
        distance: span * 1.3,
    }
}
fn camp_view() -> Orbit {
    Orbit {
        target: Vec3::new(82.0, 12.1, -107.0),
        yaw: -2.5,
        pitch: 0.72,
        distance: 35.0,
    }
}
fn player_view(target: Vec3) -> Orbit {
    Orbit {
        target,
        yaw: -2.6,
        pitch: 0.65,
        distance: 10.0,
    }
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn setup(
    mut commands: Commands,
    mut preview: ResMut<Preview>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut fonts: ResMut<Assets<Font>>,
    integrated: Option<Res<IntegratedSession>>,
    actor: Option<Res<player::Player>>,
    dynamic: (
        Option<Res<crate::vehicle::Vehicle>>,
        Option<Res<crate::gate::Gate>>,
    ),
) {
    let (vehicle, gate) = dynamic;
    let camera = commands
        .spawn((
            Camera3d::default(),
            MapCamera,
            player::SceneEntity,
            AmbientLight {
                brightness: 500.0,
                ..default()
            },
            Transform::default(),
        ))
        .id();
    commands.spawn((
        player::SceneEntity,
        DirectionalLight {
            illuminance: 6500.0,
            ..default()
        },
        Transform::from_xyz(40.0, 70.0, -50.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    let scene = preview.scene.take().expect("startup runs once");
    preview.grid = Some(scene.grid);
    let root = commands
        .spawn((
            player::SceneEntity,
            Name::new("Map · original units -> metres"),
            Transform::from_scale(Vec3::splat(map_scene::WORLD_SCALE)),
            Visibility::default(),
        ))
        .id();
    let terrain_texture = images.add(scene.texture);
    commands.spawn((
        Name::new("Training terrain"),
        Mesh3d(meshes.add(scene.terrain)),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color_texture: Some(terrain_texture),
            perceptual_roughness: 1.0,
            ..default()
        })),
        Transform::default(),
        ChildOf(root),
    ));
    let mut textures = std::collections::BTreeMap::new();
    let mut models = std::collections::BTreeMap::new();
    for (path, model) in scene.models {
        for (name, image) in model.images {
            textures
                .entry(name)
                .or_insert_with(|| images.add(image.image));
        }
        let parts = model
            .parts
            .into_iter()
            .map(|part| {
                let texture = part.texture.and_then(|t| textures.get(&t).cloned());
                let material = materials.add(StandardMaterial {
                    base_color: if texture.is_some() {
                        Color::WHITE
                    } else {
                        Color::srgb(1.0, 0.12, 0.55)
                    },
                    base_color_texture: texture,
                    alpha_mode: part.alpha_mode,
                    perceptual_roughness: 0.95,
                    cull_mode: None,
                    ..default()
                });
                (part.name, meshes.add(part.mesh), material)
            })
            .collect::<Vec<_>>();
        models.insert(path, parts);
    }
    let knife_position = scene
        .instances
        .iter()
        .find(|i| i.inventory && i.name.eq_ignore_ascii_case("Knife"))
        .map(|i| i.transform.translation * map_scene::WORLD_SCALE);
    for instance in scene.instances {
        let entity = commands
            .spawn((
                pickup::MapObject(instance.id),
                Name::new(instance.name),
                instance.transform,
                Visibility::default(),
                ChildOf(root),
            ))
            .id();
        for (name, mesh, material) in &models[&instance.path] {
            let mut part = commands.spawn((
                Name::new(name.clone()),
                Mesh3d(mesh.clone()),
                MeshMaterial3d(material.clone()),
                Transform::default(),
                ChildOf(entity),
            ));
            if let Some(g) = &gate
                && instance.id == g.id
                && g.parts.contains(name)
            {
                part.insert(crate::gate::Leaf);
            }
            if let Some(v) = &vehicle
                && instance.id == v.id
                && let Some(wheel) = v.wheels.iter().find(|w| &w.name == name)
            {
                part.insert(crate::vehicle::Tire(wheel.clone()));
            }
        }
    }
    if let Some(mut position) = knife_position {
        if let Some(grid) = &preview.grid {
            position.y = player::surface(grid, position).unwrap_or(position.y) + 0.02;
        }
        commands.spawn((
            player::SceneEntity,
            pickup::ItemRing,
            Mesh3d(meshes.add(Torus::new(0.16, 0.19))),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: Color::srgb(0.96, 0.8, 0.28),
                unlit: true,
                ..default()
            })),
            Transform::from_translation(position),
            Visibility::default(),
        ));
    }
    let r = &scene.report;
    let font = fonts.add(Font::from_bytes(
        include_bytes!("../../shadow-viewer/assets/fonts/DejaVuSans.ttf").to_vec(),
    ));
    if preview.playable {
        commands.spawn((
            player::SceneEntity,
            UiTargetCamera(camera),
            crate::hud::Diagnostic,
            crate::selection::UiBlock,
            Node {
                position_type: PositionType::Absolute,
                left: px(0),
                top: px(0),
                bottom: px(0),
                width: px(315),
                ..default()
            },
            BackgroundColor(Color::srgba(0.015, 0.03, 0.025, 0.94)),
        ));
    }
    let mut label = if preview.playable {
        format!(
            "SHADOW COMPANY\n\n{}\nОбучение: снаряжение\n\nЛКМ — выбрать / идти\nДвойной ЛКМ — бег\nПКМ — выбор / граната\nH — остановить\nT — следить за бойцом\n1 / F11 — выбрать\n\nNumpad — камера\nУдержание ПКМ — камера\nКрая экрана — прокрутка\nКолесо — приближение\nCtrl+P / Esc — пауза\nCtrl+X — выход\n\nУчебные: Home — начало\n[ / ] — кадр\n\nПредмет — ЛКМ: подобрать\nВраг — ЛКМ: атака\nSpace — инвентарь\nПрототип: без ответного ИИ",
            actor
                .as_ref()
                .map(|a| a.display_name.to_uppercase())
                .unwrap_or_default()
        )
    } else {
        format!(
            "SHADOW COMPANY\n\nУчебная карта оригинала\nRust + Bevy\n\n{} объектов\n{} моделей\n\nРельеф: {} × {}\n{} треугольников\n\nMTF · PCX · JPG · SMF\n\n{} бойцов пока пропущены\n\nWASD — камера\nЛКМ / стрелки — поворот\nКолесо — приближение\nHome — вся карта\nC — учебная площадка\nEsc — выход",
            r["static_instances"],
            models.len(),
            r["terrain"]["samples"][0],
            r["terrain"]["samples"][1],
            r["terrain"]["triangles"],
            r["soldiers_deferred"]
        )
    };
    if integrated.is_some() {
        label = label.replace("Ctrl+P / Esc — пауза", "Ctrl+P — пауза\nEsc — меню");
    }
    let mut label_entity = commands.spawn((
        player::SceneEntity,
        UiTargetCamera(camera),
        GlobalZIndex(10),
        Text::new(label),
        TextFont {
            font: font.into(),
            // Панель и статус бойца помещаются без наложения при высоте HD 720.
            font_size: FontSize::Px(15.0),
            ..default()
        },
        TextColor(Color::srgb(0.84, 0.89, 0.78)),
        Node {
            position_type: PositionType::Absolute,
            left: px(25),
            top: px(25),
            width: px(275),
            ..default()
        },
    ));
    if preview.playable {
        label_entity.insert(crate::hud::Diagnostic);
    }
}
#[allow(clippy::too_many_arguments)] // Ресурсы внедряет Bevy.
pub(crate) fn controls(
    input: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    time: Res<Time>,
    window: Single<&Window>,
    preview: Res<Preview>,
    player: Option<ResMut<player::Player>>,
    mut orbit: ResMut<Orbit>,
    mut exit: MessageWriter<AppExit>,
    integrated: Option<Res<IntegratedSession>>,
    tutorial: Option<Res<tutorial::Tutorial>>,
    inventory: Option<Res<inventory::Inventory>>,
    native_selection: Option<Res<crate::selection::NativeSmoke>>,
    pointer: Option<Res<crate::selection::Selection>>,
    camera_options: Option<Res<CameraOptions>>,
) {
    if inventory.is_some_and(|inv| inv.open || inv.consumed_input) {
        return;
    }
    if tutorial
        .as_ref()
        .is_some_and(|t| t.pausing() || t.consumed_input)
    {
        return;
    }
    if preview.screenshot.is_some() {
        return;
    }
    let playable = player.is_some();
    let camera_speed = camera_options.as_ref().map_or(1.0, |p| p.speed);
    let ctrl = input.pressed(KeyCode::ControlLeft) || input.pressed(KeyCode::ControlRight);
    if (!playable && integrated.is_none() && input.just_pressed(KeyCode::Escape))
        || (ctrl && input.just_pressed(KeyCode::KeyX))
    {
        exit.write(AppExit::Success);
    }
    if !playable && input.just_pressed(KeyCode::Home) {
        *orbit = overview(preview.focus, preview.span);
    }
    if !playable && input.just_pressed(KeyCode::KeyC) {
        *orbit = camp_view();
    }
    if playable
        && camera_options.as_ref().is_none_or(|p| p.edge_scroll)
        && input.just_pressed(KeyCode::Home)
        && let Some(player) = &player
    {
        *orbit = player_view(player.camera_target());
    }
    let pointer_blocked = pointer.as_ref().is_some_and(|s| s.world_blocked);
    let dt = time.delta_secs();
    let (left, right, up, down) = if playable {
        (
            KeyCode::Numpad7,
            KeyCode::Numpad9,
            KeyCode::Numpad0,
            KeyCode::Numpad2,
        )
    } else {
        (
            KeyCode::ArrowLeft,
            KeyCode::ArrowRight,
            KeyCode::ArrowUp,
            KeyCode::ArrowDown,
        )
    };
    let alternative_left = playable && input.pressed(KeyCode::Numpad1);
    let alternative_right = playable && input.pressed(KeyCode::Numpad3);
    orbit.yaw += dt
        * (f32::from(input.pressed(right) || alternative_right)
            - f32::from(input.pressed(left) || alternative_left));
    orbit.pitch += dt * (f32::from(input.pressed(up)) - f32::from(input.pressed(down)));
    if !playable && mouse.pressed(MouseButton::Left) {
        orbit.yaw -= motion.delta.x * 0.006;
        orbit.pitch += motion.delta.y * 0.006;
    }
    orbit.pitch = orbit.pitch.clamp(0.08, 1.5);
    let keyboard_zoom = if playable {
        f32::from(input.pressed(KeyCode::NumpadSubtract))
            - f32::from(input.pressed(KeyCode::NumpadAdd))
    } else {
        0.0
    };
    orbit.distance = (orbit.distance
        * (-if pointer_blocked { 0.0 } else { scroll.delta.y } * 0.1 + keyboard_zoom * dt).exp())
    .clamp(if playable { 2.5 } else { 5.0 }, preview.span * 3.0);
    let forward = Vec3::new(-orbit.yaw.sin(), 0.0, -orbit.yaw.cos());
    let right = Vec3::new(orbit.yaw.cos(), 0.0, -orbit.yaw.sin());
    let (f, b, l, r) = if playable {
        (
            KeyCode::Numpad8,
            KeyCode::Numpad5,
            KeyCode::Numpad4,
            KeyCode::Numpad6,
        )
    } else {
        (KeyCode::KeyW, KeyCode::KeyS, KeyCode::KeyA, KeyCode::KeyD)
    };
    let mut pan = Vec2::new(
        f32::from(input.pressed(r)) - f32::from(input.pressed(l)),
        f32::from(input.pressed(f)) - f32::from(input.pressed(b)),
    );
    if playable
        && !pointer_blocked
        && native_selection.is_none()
        && preview.screenshot.is_none()
        && let Some(cursor) = window.cursor_position()
    {
        if cursor.x < 12.0 {
            pan.x -= 1.0;
        }
        if cursor.x > window.width() - 12.0 {
            pan.x += 1.0;
        }
        if cursor.y < 12.0 {
            pan.y += 1.0;
        }
        if cursor.y > window.height() - 12.0 {
            pan.y -= 1.0;
        }
    }
    let distance = orbit.distance;
    orbit.target +=
        (right * pan.x + forward * pan.y).normalize_or_zero() * distance * 0.3 * dt * camera_speed;
    if playable && !pointer_blocked && mouse.pressed(MouseButton::Right) {
        orbit.target += (right * (-motion.delta.x) + forward * motion.delta.y)
            * distance
            * 0.002
            * camera_speed;
    }
}
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(crate) fn issue_orders(
    mouse: Res<ButtonInput<MouseButton>>,
    time: Res<Time>,
    window: Single<&Window>,
    camera: Single<(&Camera, &GlobalTransform), With<MapCamera>>,
    preview: Res<Preview>,
    player: Option<ResMut<player::Player>>,
    mut history: ResMut<orders::ClickHistory>,
    tutorial: Option<Res<tutorial::Tutorial>>,
    item: Option<ResMut<pickup::Pickup>>,
    smoke: Option<Res<pickup::Smoke>>,
    combat: Option<ResMut<combat::Combat>>,
    inventory: Option<ResMut<inventory::Inventory>>,
    grenade: Option<ResMut<grenade::Grenade>>,
    keys: Option<Res<ButtonInput<KeyCode>>>,
    fence: Option<ResMut<fence::Fence>>,
    extras: (
        Option<ResMut<crate::smg::Uzi>>,
        Option<ResMut<crate::medical::Medical>>,
        Option<ResMut<crate::armor::Armor>>,
        Option<ResMut<crate::vehicle::Vehicle>>,
        Option<ResMut<crate::gate::Gate>>,
        Option<ResMut<crate::party::Party>>,
        Option<Res<crate::selection::Selection>>,
    ),
) {
    if tutorial
        .as_ref()
        .is_some_and(|t| t.active.is_some() || t.consumed_input)
    {
        history.0 = None;
        return;
    }
    if extras.6.as_ref().is_some_and(|s| s.world_blocked) {
        return;
    }
    if extras.5.as_ref().is_some_and(|p| p.consumed_input) {
        return;
    }
    if preview.screenshot.is_some()
        || inventory
            .as_ref()
            .is_some_and(|inv| inv.open || inv.consumed_input)
    {
        return;
    }
    let Some(mut player) = player else {
        return;
    };
    if !mouse.just_pressed(MouseButton::Left) && !mouse.just_pressed(MouseButton::Right) {
        return;
    }
    let cursor = extras.6.as_ref().and_then(|s| s.click_cursor).or_else(|| {
        extras
            .5
            .as_ref()
            .and_then(|p| p.test_cursor)
            .or_else(|| extras.4.as_ref().and_then(|g| g.test_cursor))
            .or_else(|| {
                extras
                    .3
                    .as_ref()
                    .and_then(|v| v.test_cursor)
                    .or_else(|| extras.2.as_ref().and_then(|a| a.test_cursor))
                    .or_else(|| {
                        extras.1.as_ref().and_then(|m| m.test_cursor).or_else(|| {
                            smoke
                                .as_ref()
                                .and_then(|s| s.cursor)
                                .or_else(|| window.cursor_position())
                        })
                    })
            })
    });
    let (camera, transform) = *camera;
    let mut item = item;
    let mut combat = combat;
    let mut inventory = inventory;
    let mut grenade = grenade;
    let mut fence = fence;
    let (mut uzi, mut medical, mut armor, mut vehicle, mut gate, mut party, _) = extras;
    if let Some(g) = &mut gate {
        g.cancel();
    }
    if let Some(v) = &mut vehicle
        && v.occupied
    {
        if !v.consumed_input
            && mouse.just_pressed(MouseButton::Left)
            && v.exit_armed
            && let (Some(cursor), Some(grid)) = (cursor, &preview.grid)
            && cursor.x >= 0.0
            && let Ok(ray) = camera.viewport_to_world(transform, cursor)
            && let Some(point) = orders::terrain_hit(grid, ray.origin, ray.direction.as_vec3())
        {
            let cut = fence.as_ref().filter(|f| f.cut).map(|f| f.id);
            v.exit(point, &mut player, grid, cut);
        }
        history.0 = None;
        return;
    }
    if mouse.just_pressed(MouseButton::Left)
        && player.selected
        && !player.playback.paused
        && let (Some(party), Some(grid), Some(cursor)) = (&mut party, &preview.grid, cursor)
        && !party.recruited
        && cursor.x >= 0.0
        && actor_clicked(camera, transform, cursor, &party.jack)
        && item
            .as_ref()
            .is_some_and(|i| i.clear_line(transform.translation(), party.jack.camera_target()))
    {
        if let Some(c) = &mut combat {
            c.cancel_all(&mut player);
        }
        if let Some(inv) = &mut inventory {
            inv.cancel();
        }
        if let Some(g) = &mut grenade {
            g.cancel(&mut player);
        }
        if let Some(f) = &mut fence {
            f.cancel(&mut player);
        }
        if let Some(u) = &mut uzi {
            u.cancel(&mut player);
        }
        if let Some(m) = &mut medical {
            m.cancel(&mut player);
        }
        if let Some(a) = &mut armor {
            a.ground.cancel(&mut player);
        }
        if let Some(item) = &mut item {
            item.cancel();
        }
        if let Some(v) = &mut vehicle {
            v.cancel_approach(&mut player);
        }
        let direction = (player.position - party.jack.position)
            .with_y(0.0)
            .normalize_or_zero();
        player.order(grid, party.jack.position + direction * 1.0, false);
        history.0 = None;
        return;
    }
    if mouse.just_pressed(MouseButton::Left)
        && player.selected
        && !player.playback.paused
        && let (Some(gate), Some(geometry), Some(grid), Some(cursor)) =
            (&mut gate, &item, &preview.grid, cursor)
        && cursor.x >= 0.0
        && gate.clicked(camera, transform, cursor, geometry)
    {
        if let Some(c) = &mut combat {
            c.cancel_all(&mut player);
        }
        if let Some(i) = &mut inventory {
            i.cancel();
        }
        if let Some(g) = &mut grenade {
            g.cancel(&mut player);
        }
        if let Some(f) = &mut fence {
            f.cancel(&mut player);
        }
        if let Some(u) = &mut uzi {
            u.cancel(&mut player);
        }
        if let Some(m) = &mut medical {
            m.cancel(&mut player);
        }
        if let Some(a) = &mut armor {
            a.ground.cancel(&mut player);
        }
        if let Some(v) = &mut vehicle {
            v.cancel_approach(&mut player);
        }
        gate.order(&mut player, grid);
        if let Some(i) = &mut item {
            i.cancel();
        }
        history.0 = None;
        return;
    }
    if let Some(v) = &mut vehicle {
        if mouse.just_pressed(MouseButton::Left)
            && player.selected
            && cursor.is_some_and(|cursor| {
                cursor.x >= 0.0
                    && item
                        .as_ref()
                        .is_some_and(|i| v.clicked(camera, transform, cursor, i))
            })
            && let Some(grid) = &preview.grid
        {
            if let Some(i) = &mut item {
                i.cancel();
            }
            if let Some(c) = &mut combat {
                c.cancel_all(&mut player);
            }
            if let Some(inv) = &mut inventory {
                inv.cancel();
            }
            if let Some(g) = &mut grenade {
                g.cancel(&mut player);
            }
            if let Some(f) = &mut fence {
                f.cancel(&mut player);
            }
            if let Some(u) = &mut uzi {
                u.cancel(&mut player);
            }
            if let Some(m) = &mut medical {
                m.cancel(&mut player);
            }
            if let Some(a) = &mut armor {
                a.ground.cancel(&mut player);
            }
            v.order(&mut player, grid);
            history.0 = None;
            return;
        }
        v.cancel_approach(&mut player);
    }
    if mouse.just_pressed(MouseButton::Right) {
        if let Some(a) = &mut armor {
            a.ground.cancel(&mut player);
        }
        if let Some(m) = &mut medical {
            m.cancel(&mut player);
        }
        if player.selected
            && let (Some(g), Some(c), Some(geometry), Some(grid), Some(cursor)) =
                (&mut grenade, &mut combat, &item, &preview.grid, cursor)
            && cursor.x >= 0.0
        {
            let target = c.clicked_target(camera, transform, cursor, geometry);
            let forced = keys
                .as_ref()
                .is_some_and(|k| k.pressed(KeyCode::ShiftLeft) || k.pressed(KeyCode::ShiftRight));
            let point = target.map(|i| c.guard_at(i).guard.position).or_else(|| {
                if forced {
                    camera
                        .viewport_to_world(transform, cursor)
                        .ok()
                        .and_then(|r| orders::terrain_hit(grid, r.origin, r.direction.as_vec3()))
                } else {
                    None
                }
            });
            if let Some(point) = point {
                if g.order_throw(point, target, forced, &mut player, grid) {
                    if let Some(u) = &mut uzi {
                        u.cancel(&mut player);
                    }
                    if let Some(f) = &mut fence {
                        f.cancel(&mut player);
                    }
                    c.cancel_all(&mut player);
                    if let Some(item) = &mut item {
                        item.cancel();
                    }
                    if let Some(inv) = &mut inventory {
                        inv.cancel();
                    }
                }
                history.0 = None;
                return;
            }
        }
        player.selected = false;
        player.follow_camera = false;
        history.0 = None;
        return;
    }
    let Some(cursor) = cursor else {
        return;
    };
    if cursor.x < 0.0 {
        return;
    }
    if medical.as_ref().is_some_and(|m| m.armed) {
        if let Some(a) = &mut armor {
            a.ground.cancel(&mut player);
        }
        if actor_clicked(camera, transform, cursor, &player) {
            if let Some(c) = &mut combat {
                c.cancel_all(&mut player);
            }
            if let Some(g) = &mut grenade {
                g.cancel(&mut player);
            }
            if let Some(f) = &mut fence {
                f.cancel(&mut player);
            }
            if let Some(u) = &mut uzi {
                u.cancel(&mut player);
            }
            if let Some(i) = &mut item {
                i.cancel();
            }
            if let Some(inv) = &mut inventory {
                inv.cancel();
            }
            medical.as_mut().unwrap().treat(&mut player);
        }
        history.0 = None;
        return;
    }
    if actor_clicked(camera, transform, cursor, &player) {
        player.selected = true;
        history.0 = None;
        return;
    }
    if !player.selected {
        return;
    }
    if let Some(a) = &mut armor {
        let clicked = item
            .as_ref()
            .is_some_and(|i| a.ground.clicked(camera, transform, cursor, i));
        a.ground.cancel(&mut player);
        if clicked && let Some(grid) = &preview.grid {
            if let Some(m) = &mut medical {
                m.cancel(&mut player);
            }
            if let Some(u) = &mut uzi {
                u.cancel(&mut player);
            }
            if let Some(c) = &mut combat {
                c.cancel_all(&mut player);
            }
            if let Some(g) = &mut grenade {
                g.cancel(&mut player);
            }
            if let Some(f) = &mut fence {
                f.cancel(&mut player);
            }
            if let Some(i) = &mut item {
                i.cancel();
            }
            if let Some(inv) = &mut inventory {
                inv.cancel();
            }
            a.ground.order(&mut player, grid);
            history.0 = None;
            return;
        }
    }
    if let Some(m) = &mut medical {
        let clicked = item
            .as_ref()
            .is_some_and(|i| m.ground.clicked(camera, transform, cursor, i));
        m.cancel(&mut player);
        if clicked && let Some(grid) = &preview.grid {
            if let Some(c) = &mut combat {
                c.cancel_all(&mut player);
            }
            if let Some(g) = &mut grenade {
                g.cancel(&mut player);
            }
            if let Some(f) = &mut fence {
                f.cancel(&mut player);
            }
            if let Some(u) = &mut uzi {
                u.cancel(&mut player);
            }
            if let Some(i) = &mut item {
                i.cancel();
            }
            if let Some(inv) = &mut inventory {
                inv.cancel();
            }
            m.ground.order(&mut player, grid);
            history.0 = None;
            return;
        }
    }
    if let (Some(u), Some(geometry), Some(grid)) = (uzi.as_mut(), &item, &preview.grid) {
        let clicked = u.clicked(camera, transform, cursor, geometry);
        u.cancel(&mut player);
        if clicked {
            if let Some(c) = &mut combat {
                c.cancel_all(&mut player);
            }
            if let Some(g) = &mut grenade {
                g.cancel(&mut player);
            }
            if let Some(f) = &mut fence {
                f.cancel(&mut player);
            }
            if let Some(item) = &mut item {
                item.cancel();
            }
            if let Some(inv) = &mut inventory {
                inv.cancel();
            }
            u.order(&mut player, grid);
            history.0 = None;
            return;
        }
    }
    if let (Some(f), Some(geometry), Some(grid)) = (&mut fence, &item, &preview.grid) {
        let tool = f.tool_clicked(camera, transform, cursor, geometry);
        let boundary = f.fence_clicked(camera, transform, cursor, geometry);
        if tool || boundary {
            if let Some(c) = &mut combat {
                c.cancel_all(&mut player);
            }
            if let Some(g) = &mut grenade {
                g.cancel(&mut player);
            }
            if let Some(item) = &mut item {
                item.cancel();
            }
            if let Some(inv) = &mut inventory {
                inv.cancel();
            }
            if tool {
                f.order_pickup(&mut player, grid);
            } else {
                f.order_fence(&mut player, grid);
            }
            history.0 = None;
            return;
        }
    }
    if let Some(f) = &mut fence {
        f.cancel(&mut player);
    }
    if let Some(ref mut item) = item
        && let Some(grid) = &preview.grid
        && item.clicked(camera, transform, cursor)
    {
        if let Some(g) = &mut grenade {
            g.cancel(&mut player);
        }
        item.order(&mut player, grid);
        if let Some(inv) = &mut inventory {
            inv.cancel();
        }
        if let Some(c) = &mut combat {
            c.cancel_all(&mut player);
        }
        history.0 = None;
        return;
    }
    if let (Some(g), Some(geometry), Some(grid)) = (&mut grenade, &item, &preview.grid)
        && g.clicked(camera, transform, cursor, geometry)
    {
        if g.order_pickup(&mut player, grid) {
            if let Some(c) = &mut combat {
                c.cancel_all(&mut player);
            }
            if let Some(item) = &mut item {
                item.cancel();
            }
            if let Some(inv) = &mut inventory {
                inv.cancel();
            }
        }
        history.0 = None;
        return;
    }
    if let (Some(inv), Some(item), Some(grid)) = (&mut inventory, &item, &preview.grid)
        && inv.clicked(camera, transform, cursor, item)
    {
        if let Some(g) = &mut grenade {
            g.cancel(&mut player);
        }
        if inv.order(&mut player, grid)
            && let Some(c) = &mut combat
        {
            c.cancel_all(&mut player);
        }
        history.0 = None;
        return;
    }
    if let (Some(c), Some(geometry), Some(grid)) = (&mut combat, &item, &preview.grid)
        && let Some(target) = c.clicked_target(camera, transform, cursor, geometry)
    {
        if let Some(g) = &mut grenade {
            g.cancel(&mut player);
        }
        if c.order_target(target, &mut player, grid)
            && let Some(item) = &mut item
        {
            item.cancel();
        }
        if let Some(inv) = &mut inventory {
            inv.cancel();
        }
        history.0 = None;
        return;
    }
    if let Some(grid) = &preview.grid
        && let Ok(ray) = camera.viewport_to_world(transform, cursor)
        && let Some(point) = orders::terrain_hit(grid, ray.origin, ray.direction.as_vec3())
    {
        let running = history.register(time.elapsed_secs_f64(), cursor);
        if let Some(g) = &mut grenade {
            g.cancel(&mut player);
        }
        if let Some(party) = &mut party {
            party.order_group(&player, point, running, grid);
        }
        if player.order(grid, point, running)
            && let Some(item) = &mut item
        {
            item.cancel();
            if let Some(c) = &mut combat {
                c.cancel_all(&mut player);
            }
            if let Some(inv) = &mut inventory {
                inv.cancel();
            }
        }
    }
}
pub(crate) fn actor_clicked(
    camera: &Camera,
    transform: &GlobalTransform,
    cursor: Vec2,
    actor: &player::Player,
) -> bool {
    if let (Ok(feet), Ok(head)) = (
        camera.world_to_viewport(transform, actor.position),
        camera.world_to_viewport(
            transform,
            actor.position + (actor.camera_target() - actor.position) * 2.0,
        ),
    ) {
        let width = ((feet.y - head.y).abs() * 0.3).max(8.0);
        (cursor.x - (feet.x + head.x) * 0.5).abs() < width
            && cursor.y >= head.y.min(feet.y) - 5.0
            && cursor.y <= head.y.max(feet.y) + 5.0
    } else {
        false
    }
}
pub(crate) fn update_markers(
    player: Option<Res<player::Player>>,
    mut markers: Query<(&player::Marker, &mut Transform, &mut Visibility)>,
) {
    let Some(player) = player else {
        return;
    };
    for (marker, mut transform, mut visibility) in &mut markers {
        let point = if marker.0 {
            player.selected.then_some(player.position)
        } else {
            player.destination
        };
        if let Some(point) = point {
            transform.translation = point + Vec3::Y * 0.03;
            *visibility = Visibility::Visible;
        } else {
            *visibility = Visibility::Hidden;
        }
    }
}
#[allow(clippy::too_many_arguments)] // Ресурсы и независимые запросы внедряет Bevy.
pub(crate) fn update_player(
    input: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    preview: Res<Preview>,
    player: Option<ResMut<player::Player>>,
    mut orbit: ResMut<Orbit>,
    mut bones: Query<(&player::Bone, &mut Transform), Without<player::Root>>,
    mut roots: Query<&mut Transform, With<player::Root>>,
    mut status: Query<&mut Text, With<player::Status>>,
    integrated: Option<Res<IntegratedSession>>,
    tutorial: Option<ResMut<tutorial::Tutorial>>,
    combat: Option<Res<combat::Combat>>,
    inventory: Option<ResMut<inventory::Inventory>>,
    grenade: Option<Res<grenade::Grenade>>,
    fence: Option<ResMut<fence::Fence>>,
    item: Option<Res<pickup::Pickup>>,
    dynamic: (
        Option<Res<crate::vehicle::Vehicle>>,
        Option<ResMut<crate::gate::Gate>>,
    ),
) {
    let (vehicle, mut gate) = dynamic;
    let Some(mut player) = player else {
        return;
    };
    let Some(grid) = &preview.grid else {
        return;
    };
    let mut game_input = input.clone();
    if integrated.is_some() {
        game_input.reset(KeyCode::Escape);
    }
    let ignored_input = ButtonInput::default();
    let input = if preview.screenshot.is_some() {
        &ignored_input
    } else {
        &game_input
    };
    let blocked = tutorial.as_ref().is_some_and(|t| t.pausing())
        || inventory.as_ref().is_some_and(|inv| inv.open);
    let input = if inventory
        .as_ref()
        .is_some_and(|inv| inv.open || inv.consumed_input)
        || tutorial
            .as_ref()
            .is_some_and(|t| t.active.is_some() || t.consumed_input)
    {
        &ignored_input
    } else {
        input
    };
    let mut inventory = inventory;
    let mut tutorial = tutorial;
    let mut fence = fence;
    let previous = player.position;
    let followed = player.follow_camera;
    if !blocked {
        player.step(input, time.delta_secs(), grid);
    }
    if let Some(f) = &mut fence
        && !vehicle.as_ref().is_some_and(|v| v.occupied)
    {
        if !input.just_pressed(KeyCode::Home) {
            f.guard_movement(&mut player, previous);
        }
        if let (Some(inv), Some(t), Some(item)) = (&mut inventory, &mut tutorial, &item) {
            f.update(&mut player, inv, t, item, grid, time.delta_secs());
        }
    }
    if let Some(g) = &mut gate
        && !vehicle.as_ref().is_some_and(|v| v.occupied)
        && !input.just_pressed(KeyCode::Home)
    {
        g.guard_movement(&mut player, previous);
    }
    for (bone, mut transform) in &mut bones {
        *transform = player.playback.pose.bones[bone.0];
    }
    for mut transform in &mut roots {
        *transform = player.transform();
    }
    if player.follow_camera {
        if followed {
            orbit.target += player.position - previous;
        } else {
            orbit.target = player.camera_target();
        }
    }
    if input.just_pressed(KeyCode::Home) {
        *orbit = player_view(player.camera_target());
    }
    let clip = &player.clips[player.playback.selected];
    for mut text in &mut status {
        *text = Text::new(format!(
            "Оружие: {}\nСтойка: {} · S/C/P\n{}\n{:.2} / {:.2} с · {}\nПозиция: {:.2}, {:.2}, {:.2} м\nВыбор: {}\n{}",
            inventory
                .as_ref()
                .and_then(|inv| inv.weapon(&player))
                .map(|w| w.name())
                .unwrap_or("нет"),
            player.posture.name(),
            clip.name,
            player.playback.seconds,
            clip.duration(),
            if player.playback.paused || blocked {
                "ПАУЗА"
            } else {
                "ДВИЖЕНИЕ"
            },
            player.position.x,
            player.position.y,
            player.position.z,
            if player.selected {
                &player.name
            } else {
                "нет"
            },
            inventory
                .as_ref()
                .map(|inv| {
                    let ammo = if inv.collected {
                        format!("BDA: {}/{} патронов\n", inv.rounds, inv.gun.magazine)
                    } else {
                        String::new()
                    };
                    let ammo = if inv.owns_uzi {
                        format!(
                            "{ammo}UZI: {}/{} патронов\n",
                            inv.uzi_rounds,
                            inv.uzi.as_ref().unwrap().magazine
                        )
                    } else {
                        ammo
                    };
                    let target = combat
                        .as_ref()
                        .map(|c| {
                            if c.alive() {
                                format!("Цель 1: {:.0}/75", c.health)
                            } else if c.guard_at(1).alive() {
                                format!("Цель 2: {:.0}/75 · {}", c.guard_at(1).health, c.message)
                            } else if c.guard_at(2).alive() {
                                format!("Цель 3: {:.0}/75", c.guard_at(2).health)
                            } else if inventory.as_ref().is_some_and(|inv| inv.owns_uzi) {
                                format!(
                                    "Охрана: {}/6 · {}",
                                    c.smg.iter().filter(|g| !g.alive()).count(),
                                    c.message
                                )
                            } else {
                                fence
                                    .as_ref()
                                    .map(|f| {
                                        if f.cut {
                                            format!("Проход открыт · переходов: {}", f.crossings)
                                        } else if f.collected {
                                            "Кусачки: ЛКМ по сетке".into()
                                        } else {
                                            "Кусачки: ЛКМ по предмету".into()
                                        }
                                    })
                                    .unwrap_or_else(|| "Первые три задачи выполнены".into())
                            }
                        })
                        .unwrap_or_default();
                    format!(
                        "{ammo}{target}{}",
                        grenade
                            .as_ref()
                            .filter(|g| g.collected)
                            .map(|g| format!(" · гранат: {}", g.count))
                            .unwrap_or_default()
                    )
                })
                .unwrap_or_default()
        ));
    }
}
pub(crate) fn orbit_camera(orbit: Res<Orbit>, mut camera: Single<&mut Transform, With<MapCamera>>) {
    let direction = Vec3::new(
        orbit.yaw.sin() * orbit.pitch.cos(),
        orbit.pitch.sin(),
        orbit.yaw.cos() * orbit.pitch.cos(),
    );
    let right = Vec3::new(orbit.yaw.cos(), 0.0, -orbit.yaw.sin());
    let target = orbit.target - right * orbit.distance * 0.12;
    **camera = Transform::from_translation(target + direction * orbit.distance)
        .looking_at(target, Vec3::Y);
}
pub(crate) fn capture(mut commands: Commands, preview: Res<Preview>, mut frames: Local<u32>) {
    *frames += 1;
    if *frames == 120
        && let Some(path) = &preview.screenshot
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

#[derive(Resource)]
pub(crate) struct IntegratedSession;
#[derive(Resource)]
#[allow(dead_code)]
pub(crate) struct SessionReady;
#[allow(dead_code)] // Интерфейс используется бинарником shadow-game.
pub(crate) struct Session {
    preview: Preview,
    orbit: Orbit,
    actor: player::Player,
    item: pickup::Pickup,
    combat: combat::Combat,
    inventory: inventory::Inventory,
    grenade: grenade::Grenade,
    fence: fence::Fence,
    uzi: crate::smg::Uzi,
    medical: crate::medical::Medical,
    armor: crate::armor::Armor,
    vehicle: crate::vehicle::Vehicle,
    gate: crate::gate::Gate,
    party: crate::party::Party,
    roster: crate::roster::Roster,
    hud: crate::hud::Hud,
    evacuation: crate::evacuation::Evacuation,
    tutorial: tutorial::Tutorial,
}
#[allow(dead_code)] // Интерфейс используется бинарником shadow-game.
pub(crate) fn prepare_session(catalog: &AssetCatalog) -> Result<Session, Box<dyn Error>> {
    let mission = training_definition(catalog)?;
    let scene = map_scene::prepare(catalog, &format!("maps/{}", mission.map), "training")?;
    let mut actor = player::prepare(
        catalog,
        &scene.grid,
        player::initial_actor(&mission, &scene.actors)?,
    )?;
    let item = pickup::Pickup::prepare(catalog, &scene, &mut actor)?;
    let combat = combat::Combat::prepare(catalog, &scene, &mission, &mut actor)?;
    let mut inventory = inventory::Inventory::prepare(catalog, &mut actor)?;
    let grenade = grenade::Grenade::prepare(catalog, &mut actor)?;
    let fence = fence::Fence::prepare(catalog, &scene, &mut actor, &mut inventory)?;
    crate::stance::prepare(catalog, &mut actor)?;
    let uzi = crate::smg::Uzi::prepare(catalog, &scene, &inventory)?;
    let medical = crate::medical::Medical::prepare(catalog, &scene, &mut actor)?;
    let armor = crate::armor::Armor::prepare(catalog, &scene, &mut actor)?;
    let mut party = crate::party::Party::prepare(catalog, &scene, &mission)?;
    let roster = crate::roster::Roster::prepare(catalog, &actor, &party.jack)?;
    let hud = crate::hud::Hud::prepare(catalog)?;
    let evacuation =
        crate::evacuation::Evacuation::prepare(catalog, &mission, &mut actor, &mut party)?;
    let gate = crate::gate::Gate::prepare(catalog, &scene)?;
    let mut vehicle = crate::vehicle::Vehicle::prepare(catalog, &scene)?;
    vehicle.replace_obstacles(gate.id, gate.vehicle_bounds());
    let tutorial = tutorial::Tutorial::new(
        mission,
        HelpDefinitions::from_bytes(&catalog.read("config/help_window_defs.txt")?)?,
    )?;
    let orbit = player_view(actor.camera_target());
    Ok(Session {
        preview: Preview {
            focus: scene.focus,
            span: scene.span,
            scene: Some(scene),
            screenshot: None,
            grid: None,
            playable: true,
        },
        orbit,
        actor,
        item,
        combat,
        inventory,
        grenade,
        fence,
        uzi,
        medical,
        armor,
        vehicle,
        gate,
        party,
        roster,
        hud,
        evacuation,
        tutorial,
    })
}
fn training_definition(catalog: &AssetCatalog) -> Result<MissionDefinition, Box<dyn Error>> {
    Ok(MissionDefinition::from_bytes(
        &catalog.read("campaign/training/training.txt")?,
    )?)
}
#[allow(dead_code)] // Интерфейс используется бинарником shadow-game.
impl Session {
    pub(crate) fn install(self, commands: &mut Commands) {
        commands.insert_resource(self.preview);
        commands.insert_resource(self.orbit);
        commands.insert_resource(self.actor);
        commands.insert_resource(self.item);
        commands.insert_resource(self.combat);
        commands.insert_resource(self.inventory);
        commands.insert_resource(self.grenade);
        commands.insert_resource(self.fence);
        commands.insert_resource(self.uzi);
        commands.insert_resource(self.medical);
        commands.insert_resource(self.armor);
        commands.insert_resource(self.vehicle);
        commands.insert_resource(self.gate);
        commands.insert_resource(self.party);
        commands.insert_resource(self.roster);
        commands.insert_resource(self.hud);
        commands.insert_resource(crate::selection::Selection::default());
        commands.insert_resource(self.evacuation);
        commands.insert_resource(self.tutorial);
        commands.insert_resource(orders::ClickHistory::default());
        commands.insert_resource(IntegratedSession);
        commands.insert_resource(SessionReady);
    }
}
#[allow(dead_code)] // Интерфейс используется бинарником shadow-game.
pub(crate) fn finish_setup(mut commands: Commands) {
    commands.remove_resource::<SessionReady>();
}
#[allow(dead_code)] // Интерфейс используется бинарником shadow-game.
pub(crate) fn cleanup(mut commands: Commands, roots: Query<Entity, With<player::SceneEntity>>) {
    for entity in &roots {
        commands.entity(entity).despawn();
    }
    commands.remove_resource::<Preview>();
    commands.remove_resource::<Orbit>();
    commands.remove_resource::<player::Player>();
    commands.remove_resource::<pickup::Pickup>();
    commands.remove_resource::<combat::Combat>();
    commands.remove_resource::<inventory::Inventory>();
    commands.remove_resource::<grenade::Grenade>();
    commands.remove_resource::<fence::Fence>();
    commands.remove_resource::<crate::smg::Uzi>();
    commands.remove_resource::<crate::medical::Medical>();
    commands.remove_resource::<crate::armor::Armor>();
    commands.remove_resource::<crate::vehicle::Vehicle>();
    commands.remove_resource::<crate::gate::Gate>();
    commands.remove_resource::<crate::party::Party>();
    commands.remove_resource::<crate::roster::Roster>();
    commands.remove_resource::<crate::hud::Hud>();
    commands.remove_resource::<crate::selection::Selection>();
    commands.remove_resource::<crate::evacuation::Evacuation>();
    commands.remove_resource::<tutorial::Tutorial>();
    commands.remove_resource::<orders::ClickHistory>();
    commands.remove_resource::<IntegratedSession>();
    commands.remove_resource::<SessionReady>();
}
