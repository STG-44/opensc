use crate::manifest::{Manifest, Release, validate_source};
use anyhow::{Context, Result, ensure};
use reqwest::{StatusCode, blocking::Client};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use url::Url;

#[derive(Clone, Debug)]
pub struct Available {
    pub source: Url,
    pub release: Release,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Installed {
    pub schema_version: u32,
    pub platform: String,
    pub source: Url,
    pub release: Release,
}

#[derive(Clone, Debug)]
pub struct Progress {
    pub file: String,
    pub downloaded: u64,
    pub total: u64,
}

fn client(source: &Url) -> Result<Client> {
    validate_source(source)?;
    let origin = source.origin();
    Ok(Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(1800))
        .user_agent(concat!("OpenSCLauncher/", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() > 5 || attempt.url().origin() != origin {
                attempt.error("Перенаправление за пределы сервера обновлений")
            } else {
                attempt.follow()
            }
        }))
        .build()?)
}

pub fn check(source: &str) -> Result<Option<Available>> {
    let source = Url::parse(source).context("Некорректный адрес обновлений")?;
    let mut response = client(&source)?
        .get(source.clone())
        .timeout(Duration::from_secs(20))
        .send()?;
    if response.status() == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    response.error_for_status_ref()?;
    let mut bytes = Vec::new();
    response
        .by_ref()
        .take(8 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 8 * 1024 * 1024, "Слишком большой манифест");
    let manifest: Manifest =
        serde_json::from_slice(&bytes).context("Сервер вернул неверный манифест обновлений")?;
    manifest.validate(&source)?;
    Ok(manifest
        .release
        .map(|release| Available { source, release }))
}

impl Installed {
    pub fn load(root: &Path) -> Result<Option<Self>> {
        let path = root.join("active.json");
        if !path.exists() {
            return Ok(None);
        }
        let installed: Self = serde_json::from_slice(&fs::read(path)?)?;
        ensure!(
            installed.schema_version == 1 && installed.platform == crate::platform(),
            "Установка предназначена для другой платформы"
        );
        installed.release.validate(&installed.source)?;
        Ok(Some(installed))
    }

    pub fn directory(&self, root: &Path) -> Result<PathBuf> {
        Ok(root
            .join("versions")
            .join(self.release.directory(&self.platform)?))
    }

    pub fn verify(&self, root: &Path) -> Result<()> {
        let directory = self.directory(root)?;
        for file in self.release.files_for(&self.platform) {
            verify_file(&directory.join(&file.path), file.size, &file.sha256)?;
        }
        Ok(())
    }

    pub fn launch(&self, root: &Path, data_path: &Path) -> Result<Child> {
        ensure!(data_path.is_dir(), "Укажи папку ресурсов оригинальной игры");
        let root = root.canonicalize()?;
        let directory = self.directory(&root)?;
        let config = crate::config::Config::load(&root)?;
        let overrides = if config.overrides_path.trim().is_empty() {
            let path = root.join("overrides");
            fs::create_dir_all(&path)?;
            path
        } else {
            PathBuf::from(&config.overrides_path)
        };
        ensure!(overrides.is_dir(), "Папка улучшенных ресурсов не найдена");
        let game_settings = root.join("game/settings.json");
        fs::create_dir_all(game_settings.parent().context("Нет папки настроек игры")?)?;
        let entrypoint = self
            .release
            .entrypoints
            .get(&self.platform)
            .context("Нет движка для этой платформы")?;
        let file = self
            .release
            .files_for(&self.platform)
            .find(|f| &f.path == entrypoint)
            .context("Нет файла движка")?;
        let executable = directory.join(entrypoint);
        verify_file(&executable, file.size, &file.sha256)?;
        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(root.join("engine.log"))?;
        // A fixed argument contract; no shell and no remote commands from a manifest.
        Command::new(executable)
            .arg("--game-dir")
            .arg(data_path.canonicalize()?)
            .arg("--settings")
            .arg(game_settings)
            .arg("--overrides")
            .arg(overrides.canonicalize()?)
            .current_dir(directory)
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log)
            .spawn()
            .context("Не удалось запустить движок")
    }
}

pub fn verify_file(path: &Path, size: u64, expected: &str) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("Файл не найден: {}", path.display()))?;
    ensure!(
        metadata.is_file() && metadata.len() == size,
        "Неверный размер или тип файла: {}",
        path.display()
    );
    let mut input = File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 128 * 1024];
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    ensure!(
        crate::manifest::hex(&hash.finalize()).eq_ignore_ascii_case(expected),
        "SHA-256 не совпала: {}",
        path.display()
    );
    Ok(())
}

pub fn install(
    available: &Available,
    root: &Path,
    cancel: &AtomicBool,
    mut progress: impl FnMut(Progress),
) -> Result<Installed> {
    available.release.validate(&available.source)?;
    let platform = crate::platform();
    ensure!(
        available.release.entrypoints.contains_key(&platform),
        "Для {platform} ещё нет сборки"
    );
    fs::create_dir_all(root.join("versions"))?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join("update.lock"))?;
    lock.try_lock()
        .context("Другая копия лаунчера уже устанавливает обновление")?;
    let staging = tempfile::Builder::new()
        .prefix(".install-")
        .tempdir_in(root.join("versions"))?;
    let client = client(&available.source)?;
    let total = available
        .release
        .files_for(&platform)
        .try_fold(0u64, |sum, file| {
            sum.checked_add(file.size).context("Слишком большой выпуск")
        })?;
    let mut downloaded = 0u64;
    for file in available.release.files_for(&platform) {
        ensure!(!cancel.load(Ordering::Relaxed), "Установка отменена");
        let path = staging.path().join(&file.path);
        fs::create_dir_all(path.parent().context("Путь без родительской папки")?)?;
        let mut output = File::create(&path)?;
        let mut response = client
            .get(file.download_url(&available.source)?)
            .send()?
            .error_for_status()?;
        let mut size = 0u64;
        let mut hash = Sha256::new();
        let mut buffer = [0; 128 * 1024];
        let mut last_progress = std::time::Instant::now();
        loop {
            ensure!(!cancel.load(Ordering::Relaxed), "Установка отменена");
            let count = response.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            size += count as u64;
            ensure!(
                size <= file.size,
                "Файл больше указанного в манифесте: {}",
                file.path
            );
            hash.update(&buffer[..count]);
            output.write_all(&buffer[..count])?;
            downloaded += count as u64;
            if last_progress.elapsed() >= Duration::from_millis(100) {
                progress(Progress {
                    file: file.path.clone(),
                    downloaded,
                    total,
                });
                last_progress = std::time::Instant::now();
            }
        }
        ensure!(
            size == file.size
                && crate::manifest::hex(&hash.finalize()).eq_ignore_ascii_case(&file.sha256),
            "Файл повреждён: {}",
            file.path
        );
        output.sync_all()?;
        #[cfg(unix)]
        if file.executable {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755))?;
        }
        progress(Progress {
            file: file.path.clone(),
            downloaded,
            total,
        });
    }
    ensure!(!cancel.load(Ordering::Relaxed), "Установка отменена");
    let installed = Installed {
        schema_version: 1,
        platform,
        source: available.source.clone(),
        release: available.release.clone(),
    };
    let destination = installed.directory(root)?;
    if destination.exists() {
        // Never replace an existing directory in place; active games can keep using it.
        installed
            .verify(root)
            .context("Существующая сборка повреждена; убери её папку перед переустановкой")?;
    } else {
        fs::rename(staging.path(), &destination)?;
    }
    crate::config::atomic_json(&root.join("active.json"), &installed)?;
    Ok(installed)
}

pub fn supported(available: &Available) -> bool {
    available
        .release
        .entrypoints
        .contains_key(&crate::platform())
}

pub fn no_release_error() -> anyhow::Error {
    anyhow::anyhow!("Тестовые сборки ещё не опубликованы")
}
