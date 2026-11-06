use anyhow::{Context, Result};
use opensc_localization::Locale;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UpdateChannel {
    #[default]
    Stable,
    Test,
}

impl UpdateChannel {
    pub fn manifest(self) -> &'static str {
        match self {
            Self::Stable => crate::DEFAULT_MANIFEST,
            Self::Test => crate::TEST_MANIFEST,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Stable => "Стабильный",
            Self::Test => "Тестовый",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct Config {
    pub manifest_url: String,
    pub data_path: String,
    pub overrides_path: String,
    pub channel: UpdateChannel,
    pub language: Locale,
    pub confirmed_releases: std::collections::BTreeMap<String, String>,
    #[serde(flatten)]
    pub extra: std::collections::BTreeMap<String, serde_json::Value>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            manifest_url: crate::DEFAULT_MANIFEST.into(),
            data_path: String::new(),
            overrides_path: String::new(),
            channel: UpdateChannel::Stable,
            language: Locale::Russian,
            confirmed_releases: Default::default(),
            extra: Default::default(),
        }
    }
}

pub fn default_root() -> PathBuf {
    if let Some(root) = std::env::var_os("OPENSC_HOME") {
        return root.into();
    }
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support/OpenSC")
    } else if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or(home)
            .join("OpenSC")
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local/share"))
            .join("opensc")
    }
}

impl Config {
    pub fn load(root: &Path) -> Result<Self> {
        let path = root.join("config.json");
        let value: serde_json::Value = if path.exists() {
            serde_json::from_slice(&fs::read(&path)?)?
        } else {
            serde_json::json!({})
        };
        let mut config: Self = serde_json::from_value(value.clone())
            .with_context(|| format!("Не удалось прочитать {}", path.display()))?;
        // Older launchers retained the URL but removed the newer channel field.
        if value.get("channel").is_none() && config.manifest_url == crate::TEST_MANIFEST {
            config.channel = UpdateChannel::Test;
        }
        if value.get("language").is_none()
            && let Ok(bytes) = fs::read(root.join("game/settings.json"))
            && let Ok(settings) = serde_json::from_slice::<serde_json::Value>(&bytes)
            && let Some(code) = settings["language"].as_str()
            && let Ok(language) = Locale::parse(code)
        {
            config.language = language;
        }
        // Migrate the old public channel, but retain explicitly configured local/custom servers.
        if [
            crate::LEGACY_MANIFEST,
            crate::DEFAULT_MANIFEST,
            crate::TEST_MANIFEST,
        ]
        .contains(&config.manifest_url.as_str())
        {
            config.manifest_url = config.channel.manifest().into();
        }
        Ok(config)
    }

    pub fn set_channel(&mut self, channel: UpdateChannel) {
        self.channel = channel;
        self.manifest_url = channel.manifest().into();
    }

    pub fn save(&self, root: &Path) -> Result<()> {
        atomic_json(&root.join("config.json"), self)
    }
}

pub fn atomic_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let parent = path.parent().context("Путь без родительской папки")?;
    fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(&serde_json::to_vec_pretty(value)?)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|e| e.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_game_language_is_adopted_even_without_a_launcher_config() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("game")).unwrap();
        fs::write(
            root.path().join("game/settings.json"),
            r#"{"version":1,"language":"en"}"#,
        )
        .unwrap();
        let config = Config::load(root.path()).unwrap();
        assert_eq!(config.language, Locale::English);
        assert_eq!(config.channel, UpdateChannel::Stable);
    }
    #[test]
    fn old_public_profile_migrates_to_stable_and_retains_resources() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("config.json"), serde_json::json!({
            "manifest_url": crate::LEGACY_MANIFEST, "data_path": "original", "overrides_path": "mods",
            "auto_update": false, "auto_update_launcher": false
        }).to_string()).unwrap();
        let config = Config::load(root.path()).unwrap();
        assert_eq!(config.channel, UpdateChannel::Stable);
        assert_eq!(config.manifest_url, crate::DEFAULT_MANIFEST);
        assert_eq!(config.data_path, "original");
        assert_eq!(config.overrides_path, "mods");
    }

    #[test]
    fn channel_survives_reopening_until_explicitly_changed() {
        let root = tempfile::tempdir().unwrap();
        let mut config = Config::load(root.path()).unwrap();
        assert_eq!(config.channel, UpdateChannel::Stable);
        config.set_channel(UpdateChannel::Test);
        config.save(root.path()).unwrap();
        let mut config = Config::load(root.path()).unwrap();
        assert_eq!(config.channel, UpdateChannel::Test);
        assert_eq!(config.manifest_url, crate::TEST_MANIFEST);
        config.set_channel(UpdateChannel::Stable);
        config.save(root.path()).unwrap();
        assert_eq!(
            Config::load(root.path()).unwrap().manifest_url,
            crate::DEFAULT_MANIFEST
        );
    }

    #[test]
    fn legacy_test_manifest_recovers_the_channel_removed_by_an_old_launcher() {
        let root = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join("config.json"),
            serde_json::json!({
                "manifest_url": crate::TEST_MANIFEST,
                "data_path": "original",
                "overrides_path": "mods"
            })
            .to_string(),
        )
        .unwrap();
        let config = Config::load(root.path()).unwrap();
        assert_eq!(config.channel, UpdateChannel::Test);
        assert_eq!(config.manifest_url, crate::TEST_MANIFEST);
        assert_eq!(config.data_path, "original");
        assert_eq!(config.overrides_path, "mods");
    }

    #[test]
    fn explicit_channel_takes_priority_over_a_legacy_manifest_url() {
        let root = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join("config.json"),
            serde_json::json!({"manifest_url": crate::TEST_MANIFEST, "channel": "stable"})
                .to_string(),
        )
        .unwrap();
        let config = Config::load(root.path()).unwrap();
        assert_eq!(config.channel, UpdateChannel::Stable);
        assert_eq!(config.manifest_url, crate::DEFAULT_MANIFEST);
    }

    #[test]
    fn saving_retains_preferences_written_by_a_newer_launcher() {
        let root = tempfile::tempdir().unwrap();
        let future = serde_json::json!({"download_limit": 2048, "ui": {"scale": 1.5}});
        fs::write(
            root.path().join("config.json"),
            serde_json::json!({"future_preferences": future}).to_string(),
        )
        .unwrap();
        let mut config = Config::load(root.path()).unwrap();
        config.set_channel(UpdateChannel::Test);
        config.save(root.path()).unwrap();
        let saved: serde_json::Value =
            serde_json::from_slice(&fs::read(root.path().join("config.json")).unwrap()).unwrap();
        assert_eq!(saved["future_preferences"], future);
        assert_eq!(saved["channel"], "test");
    }

    #[test]
    fn explicit_custom_server_is_preserved_when_loading_legacy_config() {
        let root = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join("config.json"),
            r#"{"manifest_url":"http://127.0.0.1:45123/custom.json"}"#,
        )
        .unwrap();
        assert_eq!(
            Config::load(root.path()).unwrap().manifest_url,
            "http://127.0.0.1:45123/custom.json"
        );
    }
}
