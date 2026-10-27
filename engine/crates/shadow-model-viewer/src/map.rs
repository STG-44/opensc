//! Первая карта: MTF-расстановка, PCX-рельеф, JPG-поверхность и SMF-объекты.
mod armor;
mod asset_mesh;
mod body_carry;
mod combat;
mod evacuation;
mod fence;
mod gate;
mod grenade;
mod hud;
mod hud_layout;
mod inventory;
mod localization;
mod map_scene;
mod medical;
mod motion;
mod normals;
mod orders;
mod party;
mod pickup;
mod player;
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
    if let Err(error) = training::run() {
        eprintln!("Ошибка: {error}");
        std::process::exit(1);
    }
}

mod ui_controls;
