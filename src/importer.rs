//! Original data are copied into a persistent, verified user library before activation.
use crate::{
    config::atomic_json,
    media::Media,
    resources::{self, Inventory},
    updater::Progress,
};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
use walkdir::WalkDir;
const MAX_TOTAL: u64 = 32 * 1024 * 1024 * 1024;
const MAX_FILES: usize = 100_000;
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DataFile {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}
#[derive(Deserialize, Serialize)]
struct Receipt {
    schema_version: u32,
    files: Vec<DataFile>,
    data_directory: String,
}
struct InputFile {
    source: PathBuf,
    relative: String,
    size: u64,
}
pub fn is_managed(root: &Path, path: &Path) -> bool {
    let Some((base, path)) = root
        .join("game-data")
        .canonicalize()
        .ok()
        .zip(path.canonicalize().ok())
    else {
        return false;
    };
    let Ok(relative) = path.strip_prefix(&base) else {
        return false;
    };
    let Some(std::path::Component::Normal(first)) = relative.components().next() else {
        return false;
    };
    first.to_str().is_some_and(|s| s.starts_with("original-"))
        && base.join(first).join("import.json").is_file()
}

fn entries(root: &Path) -> Result<Vec<InputFile>> {
    let mut result = Vec::new();
    let mut names = BTreeMap::new();
    let mut total = 0u64;
    for e in WalkDir::new(root).follow_links(false).max_depth(64) {
        let e = e.context("Не удалось прочитать исходные файлы")?;
        ensure!(
            !e.file_type().is_symlink(),
            "В папке есть ссылка вместо файла: {}",
            e.path().display()
        );
        if !e.file_type().is_file() {
            continue;
        }
        let relative = e
            .path()
            .strip_prefix(root)?
            .to_str()
            .context("Имя файла не представимо в Unicode")?
            .replace('\\', "/");
        crate::manifest::validate_path(&relative)?;
        ensure!(
            names.insert(relative.to_lowercase(), ()).is_none(),
            "Имена файлов отличаются только регистром: {relative}"
        );
        let size = e.metadata()?.len();
        total = total.checked_add(size).context("Слишком много данных")?;
        ensure!(
            total <= MAX_TOTAL && result.len() < MAX_FILES,
            "Исходные файлы больше 32 ГБ или их слишком много"
        );
        result.push(InputFile {
            source: e.path().to_owned(),
            relative,
            size,
        });
    }
    result.sort_by(|a, b| a.relative.cmp(&b.relative));
    ensure!(!result.is_empty(), "Папка пуста");
    Ok(result)
}
fn find(root: &Path, name: &str) -> Option<PathBuf> {
    fs::read_dir(root)
        .ok()?
        .filter_map(|e| e.ok())
        .find_map(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|n| n.eq_ignore_ascii_case(name))
                .then(|| e.path())
        })
}
fn data_root(root: &Path) -> PathBuf {
    find(root, "Data")
        .filter(|p| p.is_dir())
        .unwrap_or_else(|| root.to_owned())
}
fn ready(root: &Path) -> Result<PathBuf> {
    let mut candidates = vec![data_root(root)];
    if let Some(child) = find(root, "shadow_company").filter(|p| p.is_dir()) {
        candidates.push(data_root(&child));
    }
    for data in candidates {
        if let Ok(catalog) = shadow_formats::AssetCatalog::open(&data, None)
            && [
                "config/window_bases/main_menu.txt",
                "campaign/training/training.txt",
            ]
            .iter()
            .all(|name| catalog.read(name).is_ok())
        {
            return Ok(data);
        }
    }
    bail!("В выбранных файлах не найдены полные ресурсы Shadow Company")
}

fn copy(
    root: &Path,
    destination: &Path,
    cancel: &AtomicBool,
    progress: &mut impl FnMut(&str, Progress),
) -> Result<()> {
    let files = entries(root)?;
    let total = files.iter().map(|f| f.size).sum();
    let mut done = 0;
    let mut buffer = vec![0; 256 * 1024];
    let mut last = Instant::now();
    for file in files {
        ensure!(!cancel.load(Ordering::Relaxed), "Импорт отменён");
        let before = fs::metadata(&file.source)?;
        let target = destination.join(&file.relative);
        fs::create_dir_all(target.parent().context("Нет родительской папки")?)?;
        let mut input = File::open(&file.source)?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)?;
        let mut source_hash = Sha256::new();
        let mut size = 0;
        loop {
            ensure!(!cancel.load(Ordering::Relaxed), "Импорт отменён");
            let n = input.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            size += n as u64;
            ensure!(
                size <= file.size,
                "Исходный файл изменился: {}",
                file.relative
            );
            source_hash.update(&buffer[..n]);
            output.write_all(&buffer[..n])?;
            done += n as u64;
            if last.elapsed() >= Duration::from_millis(100) {
                progress(
                    "Копирование игровых файлов",
                    Progress {
                        file: file.relative.clone(),
                        downloaded: done,
                        total,
                    },
                );
                last = Instant::now();
            }
        }
        output.sync_all()?;
        let after = fs::metadata(&file.source)?;
        ensure!(
            size == file.size && before.modified().ok() == after.modified().ok(),
            "Исходный файл изменился во время копирования: {}",
            file.relative
        );
        // Read the destination independently, so a failed copy never becomes active.
        let mut check = File::open(&target)?;
        let mut hash = Sha256::new();
        loop {
            ensure!(!cancel.load(Ordering::Relaxed), "Импорт отменён");
            let n = check.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            hash.update(&buffer[..n]);
        }
        ensure!(
            source_hash.finalize() == hash.finalize(),
            "Проверка копии не прошла: {}",
            file.relative
        );
    }
    progress(
        "Копирование игровых файлов",
        Progress {
            file: "Копирование завершено".into(),
            downloaded: done,
            total,
        },
    );
    Ok(())
}
fn disc(
    root: &Path,
    destination: &Path,
    cancel: &AtomicBool,
    progress: &mut impl FnMut(&str, Progress),
) -> Result<()> {
    let header = find(root, "data1.hdr").context("На диске не найден data1.hdr")?;
    let cabinet = find(root, "data1.cab").context("На диске не найден data1.cab")?;
    let mut media = Media::installshield5(&header, &cabinet)?;
    let mut last = Instant::now();
    media.extract(destination, cancel, |name, done, total| {
        if last.elapsed() >= Duration::from_millis(100) || done == total {
            progress(
                "Распаковка оригинального диска",
                Progress {
                    file: name.into(),
                    downloaded: done,
                    total,
                },
            );
            last = Instant::now();
        }
    })?;
    // Movies are loose files on the original disc, outside data1.cab.
    if let Some(data) = find(root, "data").filter(|p| p.is_dir()) {
        copy(&data, &destination.join("Data"), cancel, progress)?;
    }
    Ok(())
}
fn payload(
    source: &Path,
    destination: &Path,
    cancel: &AtomicBool,
    progress: &mut impl FnMut(&str, Progress),
) -> Result<()> {
    if ready(source).is_ok() {
        copy(source, destination, cancel, progress)
    } else if find(source, "data1.cab").is_some() {
        disc(source, destination, cancel, progress)
    } else {
        bail!(
            "Выбери папку установленной Shadow Company или оригинальный диск с data1.cab и data1.hdr"
        )
    }
}
fn hashes(
    root: &Path,
    cancel: &AtomicBool,
    progress: &mut impl FnMut(&str, Progress),
) -> Result<Vec<DataFile>> {
    let files = entries(root)?;
    let total = files.iter().map(|f| f.size).sum();
    let mut done = 0;
    let mut result = Vec::new();
    let mut buffer = vec![0; 256 * 1024];
    let mut last = Instant::now();
    for file in files {
        let mut input = File::open(file.source)?;
        let mut hash = Sha256::new();
        let mut size = 0;
        loop {
            ensure!(!cancel.load(Ordering::Relaxed), "Импорт отменён");
            let n = input.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            hash.update(&buffer[..n]);
            size += n as u64;
            done += n as u64;
            if last.elapsed() >= Duration::from_millis(100) {
                progress(
                    "Проверка игровых файлов",
                    Progress {
                        file: file.relative.clone(),
                        downloaded: done,
                        total,
                    },
                );
                last = Instant::now();
            }
        }
        ensure!(size == file.size, "Файл изменился во время проверки");
        result.push(DataFile {
            path: file.relative,
            size,
            sha256: crate::manifest::hex(&hash.finalize()),
        });
    }
    Ok(result)
}
/// Source files stay untouched. A failure or cancellation retains the previous data path.
pub fn import(
    root: &Path,
    source: &Path,
    cancel: &AtomicBool,
    mut progress: impl FnMut(&str, Progress),
) -> Result<Inventory> {
    fs::create_dir_all(root)?;
    let root = root.canonicalize()?;
    let source = source.canonicalize().context("Источник игры не найден")?;
    if is_managed(&root, &source) {
        return resources::inspect(&ready(&source)?);
    }
    ensure!(
        !root.starts_with(&source),
        "Нельзя импортировать папку, содержащую сам каталог OpenSC"
    );
    let library = root.join("game-data");
    fs::create_dir_all(&library)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join("data-import.lock"))?;
    lock.try_lock()
        .context("Другая копия лаунчера уже импортирует игру")?;
    let stage = tempfile::Builder::new()
        .prefix(".import-")
        .tempdir_in(&library)?;
    let game = stage.path().join("game");
    fs::create_dir(&game)?;
    if source.is_dir() {
        payload(&source, &game, cancel, &mut progress)?;
    } else {
        let mut media = Media::iso(&source)?;
        if media
            .entries
            .iter()
            .any(|e| e.path.eq_ignore_ascii_case("data1.cab"))
            && media
                .entries
                .iter()
                .any(|e| e.path.eq_ignore_ascii_case("data1.hdr"))
        {
            // The original disc's DirectX and GameSpy installers are not game data.
            media.entries.retain(|e| {
                e.path.eq_ignore_ascii_case("data1.cab")
                    || e.path.eq_ignore_ascii_case("data1.hdr")
                    || e.path.to_ascii_lowercase().starts_with("data/")
            });
        }

        ensure!(
            media
                .entries
                .iter()
                .any(|e| e.path.to_ascii_lowercase().ends_with(".gut"))
                || (media
                    .entries
                    .iter()
                    .any(|e| e.path.eq_ignore_ascii_case("data1.cab"))
                    && media
                        .entries
                        .iter()
                        .any(|e| e.path.eq_ignore_ascii_case("data1.hdr"))),
            "В образе не найдены файлы Shadow Company"
        );
        let extracted = stage.path().join("media");
        fs::create_dir(&extracted)?;
        let mut last = Instant::now();
        media.extract(&extracted, cancel, |name, done, total| {
            if last.elapsed() >= Duration::from_millis(100) || done == total {
                progress(
                    "Чтение образа диска",
                    Progress {
                        file: name.into(),
                        downloaded: done,
                        total,
                    },
                );
                last = Instant::now();
            }
        })?;
        payload(&extracted, &game, cancel, &mut progress)?;
        fs::remove_dir_all(extracted)?;
    }
    let data = ready(&game)?;
    let relative = data
        .strip_prefix(stage.path())?
        .to_str()
        .context("Неверный каталог данных")?
        .replace('\\', "/");
    let files = hashes(&game, cancel, &mut progress)?;
    let receipt = Receipt {
        schema_version: 1,
        files,
        data_directory: relative,
    };
    let id = crate::manifest::hex(&Sha256::digest(serde_json::to_vec(&receipt)?));
    let target = library.join(format!("original-{}", &id[..16]));
    atomic_json(&stage.path().join("import.json"), &receipt)?;
    ensure!(!cancel.load(Ordering::Relaxed), "Импорт отменён");
    if target.exists() {
        let old: Receipt = serde_json::from_slice(&fs::read(target.join("import.json"))?)?;
        ensure!(
            serde_json::to_vec(&old)? == serde_json::to_vec(&receipt)?,
            "Внутренняя копия с таким именем отличается"
        );
        for f in &receipt.files {
            crate::updater::verify_file(&target.join("game").join(&f.path), f.size, &f.sha256)?;
        }
    } else {
        fs::rename(stage.path(), &target)?;
    }
    resources::inspect(&target.join(receipt.data_directory))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::fixtures;
    #[test]
    fn folder_import_copies_all_files_verifies_and_keeps_source_unchanged() {
        let d = tempfile::tempdir().unwrap();
        let source = d.path().join("Источник игры");
        fs::create_dir_all(source.join("Data")).unwrap();
        let archive = fixtures::gut();
        fs::write(source.join("Data/core.gut"), &archive).unwrap();
        fs::write(source.join("Readme.txt"), b"original documentation").unwrap();
        let root = d.path().join("OpenSC data");
        let inventory = import(&root, &source, &AtomicBool::new(false), |_, _| {}).unwrap();
        assert!(is_managed(&root, &inventory.root));
        assert_eq!(fs::read(inventory.root.join("core.gut")).unwrap(), archive);
        assert_eq!(
            fs::read(inventory.root.parent().unwrap().join("Readme.txt")).unwrap(),
            b"original documentation"
        );
        assert_eq!(fs::read(source.join("Data/core.gut")).unwrap(), archive);
        let again = import(&root, &source, &AtomicBool::new(false), |_, _| {}).unwrap();
        assert_eq!(again.root, inventory.root);
    }
    #[test]
    fn cancel_or_invalid_source_keeps_previous_configuration() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("profile");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("config.json"), b"previous settings").unwrap();
        let source = d.path().join("source");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("data.gut"), fixtures::gut()).unwrap();
        assert!(import(&root, &source, &AtomicBool::new(true), |_, _| {}).is_err());
        fs::write(source.join("data.gut"), b"damaged").unwrap();
        assert!(import(&root, &source, &AtomicBool::new(false), |_, _| {}).is_err());
        assert_eq!(
            fs::read(root.join("config.json")).unwrap(),
            b"previous settings"
        );
        assert_eq!(fs::read_dir(root.join("game-data")).unwrap().count(), 0);
    }
    #[test]
    fn imports_joliet_mds_pair_without_mounting() {
        let d = tempfile::tempdir().unwrap();
        let (raw, mds) = fixtures::mdf(&fixtures::iso(true), 2448, 24, 4096);
        fs::write(d.path().join("Образ.mdf"), raw).unwrap();
        fs::write(d.path().join("Образ.mds"), mds).unwrap();
        let result = import(
            &d.path().join("profile"),
            &d.path().join("Образ.mds"),
            &AtomicBool::new(false),
            |_, _| {},
        )
        .unwrap();
        assert_eq!(
            fs::read(result.root.join("CORE.GUT")).unwrap(),
            fixtures::gut()
        );
        assert_eq!(fs::read(result.root.join("Карта.bin")).unwrap(), b"map!");
        fs::remove_file(d.path().join("Образ.mdf")).unwrap();
        fs::remove_file(d.path().join("Образ.mds")).unwrap();
        assert!(
            shadow_formats::AssetCatalog::open(&result.root, None)
                .unwrap()
                .read("config/window_bases/main_menu.txt")
                .is_ok()
        );
    }
    #[test]
    fn disc_import_includes_cabinet_data_and_loose_movies() {
        let d = tempfile::tempdir().unwrap();
        let source = d.path().join("disc");
        fs::create_dir_all(source.join("data/movies")).unwrap();
        let (hdr, cab) = fixtures::cab();
        fs::write(source.join("data1.hdr"), hdr).unwrap();
        fs::write(source.join("data1.cab"), cab).unwrap();
        fs::write(source.join("data/movies/intro.avi"), b"video").unwrap();
        let result = import(
            &d.path().join("profile"),
            &source,
            &AtomicBool::new(false),
            |_, _| {},
        )
        .unwrap();
        assert_eq!(
            fs::read(result.root.join("core.gut")).unwrap(),
            fixtures::gut()
        );
        assert_eq!(
            fs::read(result.root.join("movies/intro.avi")).unwrap(),
            b"video"
        );
    }
}
