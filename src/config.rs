use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct Config {
    pub manifest_url: String,
    pub data_path: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            manifest_url: crate::DEFAULT_MANIFEST.into(),
            data_path: String::new(),
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
        if !path.exists() {
            return Ok(Self::default());
        }
        serde_json::from_slice(&fs::read(&path)?)
            .with_context(|| format!("Не удалось прочитать {}", path.display()))
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
