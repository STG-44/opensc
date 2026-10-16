use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
use walkdir::WalkDir;

#[derive(Clone, Debug, Serialize)]
pub struct Inventory {
    pub root: PathBuf,
    pub files: u64,
    pub bytes: u64,
    pub by_extension: BTreeMap<String, u64>,
    pub case_collisions: Vec<(PathBuf, PathBuf)>,
}

/// Inventory only. Detecting files does not certify compatibility with the engine.
pub fn inspect(root: &Path) -> Result<Inventory> {
    ensure!(
        root.is_dir(),
        "Папка ресурсов не найдена: {}",
        root.display()
    );
    let root = root.canonicalize()?;
    let mut inventory = Inventory {
        root: root.clone(),
        files: 0,
        bytes: 0,
        by_extension: BTreeMap::new(),
        case_collisions: Vec::new(),
    };
    let mut seen = BTreeMap::new();
    for entry in WalkDir::new(&root).follow_links(false).max_depth(64) {
        let entry = entry.context("Не удалось прочитать каталог ресурсов")?;
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path().strip_prefix(&root)?.to_path_buf();
        let key = path.to_string_lossy().replace('\\', "/").to_lowercase();
        if let Some(previous) = seen.insert(key, path.clone()) {
            inventory.case_collisions.push((previous, path));
        }
        inventory.files += 1;
        ensure!(
            inventory.files <= 1_000_000,
            "В каталоге больше миллиона файлов"
        );
        inventory.bytes += entry.metadata()?.len();
        let ext = entry
            .path()
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("(без расширения)")
            .to_ascii_lowercase();
        *inventory.by_extension.entry(ext).or_default() += 1;
    }
    ensure!(inventory.files > 0, "Выбрана пустая папка");
    Ok(inventory)
}

/// Accept either the Data directory itself or the enclosing installation directory.
pub fn resolve_data(root: &Path) -> PathBuf {
    for candidate in [
        root.join("Data"),
        root.join("data"),
        root.join("shadow_company/Data"),
    ] {
        if candidate.is_dir() {
            return candidate;
        }
    }
    root.to_path_buf()
}
