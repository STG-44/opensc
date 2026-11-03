pub mod config;
pub mod importer;
pub mod manifest;
mod media;
pub mod reports;
pub mod resources;
pub mod self_update;
pub mod updater;

pub const LEGACY_MANIFEST: &str = "https://shadow-test.zaikinagroup.ru/updates/manifest-v1.json";
pub const DEFAULT_MANIFEST: &str =
    "https://shadow-test.zaikinagroup.ru/updates/manifest-stable-v1.json";
pub const TEST_MANIFEST: &str = "https://shadow-test.zaikinagroup.ru/updates/manifest-test-v1.json";

pub fn platform() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}
