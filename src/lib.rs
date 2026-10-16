pub mod config;
pub mod manifest;
pub mod resources;
pub mod updater;

pub const DEFAULT_MANIFEST: &str = "https://shadow-test.zaikinagroup.ru/updates/manifest-v1.json";

pub fn platform() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}
