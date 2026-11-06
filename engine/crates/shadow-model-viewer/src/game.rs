//! Главный экран и обучение живут в одном Bevy App и одном окне.
mod armor;
mod asset_mesh;
mod body_carry;
mod combat;
mod daylight;
mod environment;
mod evacuation;
mod fence;
mod frontend;
mod gate;
mod grenade;
mod hearing;
mod hud;
mod hud_layout;
mod inventory;
mod inventory_ui;
mod item_icon;
mod localization;
mod map_scene;
mod medical;
mod menu_motion;
mod motion;
mod movie;
mod navigation;
mod normals;
mod obstacles;
mod orders;
mod party;
mod perception;
mod pickup;
mod player;
mod preferences;
mod radar;
mod rig;
mod roster;
mod scene;
mod selection;
mod smf_transform;
mod smg;
mod stance;
mod training;
mod tutorial;
mod vehicle;
fn main() {
    // Числовые снимки и прямой запуск карты сохраняют интерфейс прежней витрины.
    let standalone = std::env::args().any(|a| {
        matches!(
            a.as_str(),
            "--player"
                | "--environment-preview"
                | "--vehicle-weather-json"
                | "--weather"
                | "--weather-quality"
                | "--time-of-day"
                | "--day-length"
                | "--weather-time"
                | "--weather-json"
                | "--weather-flash"
                | "--cpu-profile-json"
                | "--player-json"
                | "--scene-json"
                | "--map"
                | "--terrain"
                | "--camera-orbit"
        )
    });
    let result = if standalone {
        training::run()
    } else {
        frontend::run()
    };
    if let Err(error) = result {
        eprintln!("Ошибка: {error}");
        std::process::exit(1);
    }
}

mod ui_controls;
