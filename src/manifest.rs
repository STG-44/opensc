use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use url::Url;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema_version: u32,
    pub release: Option<Release>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Release {
    pub version: String,
    pub notes: String,
    #[serde(default)]
    pub data_directory: Option<String>,
    pub entrypoints: BTreeMap<String, String>,
    pub files: Vec<ReleaseFile>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseFile {
    pub path: String,
    pub url: String,
    pub size: u64,
    pub sha256: String,
    #[serde(default)]
    pub executable: bool,
    /// None marks shared files, including resource packs.
    #[serde(default)]
    pub platform: Option<String>,
}

/// Accept a portable, relative file path on every supported operating system.
pub fn validate_path(path: &str) -> Result<()> {
    ensure!(
        !path.is_empty() && path.len() <= 240,
        "Некорректный путь: {path}"
    );
    ensure!(
        !path.contains(['\\', ':', '\0']) && !path.starts_with('/'),
        "Небезопасный путь: {path}"
    );
    for part in path.split('/') {
        ensure!(
            !part.is_empty() && part != "." && part != "..",
            "Небезопасный путь: {path}"
        );
        ensure!(
            !part.ends_with(['.', ' '])
                && !part
                    .chars()
                    .any(|c| c.is_control() || "<>\"|?*".contains(c)),
            "Непереносимый путь: {path}"
        );
        let base = part
            .split('.')
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase();
        ensure!(
            !matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
                && !(1..=9).any(|n| base == format!("COM{n}") || base == format!("LPT{n}")),
            "Зарезервированное имя: {path}"
        );
    }
    Ok(())
}

pub fn validate_source(source: &Url) -> Result<()> {
    let local = matches!(source.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    ensure!(
        source.scheme() == "https" || (source.scheme() == "http" && local),
        "Для обновлений требуется HTTPS"
    );
    ensure!(
        source.username().is_empty() && source.password().is_none() && source.fragment().is_none(),
        "Некорректный адрес обновлений"
    );
    Ok(())
}

impl Manifest {
    pub fn validate(&self, source: &Url) -> Result<()> {
        validate_source(source)?;
        ensure!(
            self.schema_version == 1,
            "Неизвестная версия манифеста: {}",
            self.schema_version
        );
        if let Some(release) = &self.release {
            release.validate(source)?;
        }
        Ok(())
    }
}

impl Release {
    pub fn files_for<'a>(&'a self, platform: &'a str) -> impl Iterator<Item = &'a ReleaseFile> {
        self.files
            .iter()
            .filter(move |file| file.platform.as_deref().is_none_or(|p| p == platform))
    }

    pub fn fingerprint(&self) -> Result<String> {
        Ok(hex(&Sha256::digest(serde_json::to_vec(self)?)))
    }

    pub fn directory(&self, platform: &str) -> Result<String> {
        Ok(format!(
            "{}-{}-{}",
            self.version,
            platform,
            &self.fingerprint()?[..16]
        ))
    }

    pub fn validate(&self, source: &Url) -> Result<()> {
        ensure!(
            !self.version.is_empty()
                && self.version.len() <= 64
                && self
                    .version
                    .starts_with(|c: char| c.is_ascii_alphanumeric())
                && self
                    .version
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c)),
            "Некорректная версия выпуска"
        );
        ensure!(
            !self.entrypoints.is_empty() && self.files.len() <= 100_000,
            "Некорректный список файлов выпуска"
        );
        if let Some(data) = &self.data_directory {
            validate_path(data)?;
            ensure!(
                self.files
                    .iter()
                    .any(|f| f.platform.is_none() && f.path.starts_with(&format!("{data}/"))),
                "В выпуске нет общего пакета ресурсов"
            );
        }
        for (platform, entrypoint) in &self.entrypoints {
            ensure!(
                platform
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
                "Некорректная платформа"
            );
            validate_path(entrypoint)?;
            let mut paths = BTreeSet::new();
            let mut entrypoint_found = false;
            for file in self.files_for(platform) {
                validate_path(&file.path)?;
                ensure!(
                    paths.insert(file.path.to_ascii_lowercase()),
                    "Повторяющийся путь: {}",
                    file.path
                );
                ensure!(
                    file.size <= 16 * 1024 * 1024 * 1024,
                    "Слишком большой файл: {}",
                    file.path
                );
                ensure!(
                    file.sha256.len() == 64 && file.sha256.bytes().all(|c| c.is_ascii_hexdigit()),
                    "Некорректная SHA-256: {}",
                    file.path
                );
                file.download_url(source)?;
                if file.path == *entrypoint {
                    ensure!(file.executable, "Файл движка не отмечен как исполняемый");
                    entrypoint_found = true;
                }
            }
            for path in &paths {
                let mut prefix = String::new();
                let parts: Vec<_> = path.split('/').collect();
                for part in &parts[..parts.len() - 1] {
                    if !prefix.is_empty() {
                        prefix.push('/');
                    }
                    prefix.push_str(part);
                    ensure!(!paths.contains(&prefix), "Конфликт файла и папки: {prefix}");
                }
            }
            ensure!(
                entrypoint_found,
                "Движок {entrypoint} отсутствует в выпуске для {platform}"
            );
        }
        for file in &self.files {
            if let Some(platform) = &file.platform {
                ensure!(
                    self.entrypoints.contains_key(platform),
                    "Файл с неизвестной платформой: {platform}"
                );
            }
        }
        Ok(())
    }
}

impl ReleaseFile {
    pub fn download_url(&self, source: &Url) -> Result<Url> {
        let url = source.join(&self.url).context("Некорректный URL файла")?;
        validate_source(&url)?;
        if url.origin() != source.origin() {
            bail!("Файл должен находиться на сервере обновлений: {}", self.url);
        }
        Ok(url)
    }
}

pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8] = b"0123456789abcdef";
    bytes
        .iter()
        .flat_map(|b| {
            [
                DIGITS[(b >> 4) as usize] as char,
                DIGITS[(b & 15) as usize] as char,
            ]
        })
        .collect()
}
