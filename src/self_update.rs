//! Signed launcher updates live in user data; the installed launcher is retained.
use crate::{
    config::atomic_json,
    manifest::{Manifest, validate_source},
    updater::{self, Available},
};
use anyhow::{Context, Result, ensure};
use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};
use url::Url;

const DOMAIN: &[u8] = b"OPENSC-LAUNCHER-UPDATE-V1\n";
const PUBLIC_KEY: &[u8; 32] = include_bytes!("launcher-update-key.bin");
const MAX_INDEX: u64 = 256 * 1024;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedIndex {
    pub schema_version: u32,
    pub payload: String,
    pub signature: String,
}

pub struct Prepared {
    pub version: String,
    executable: PathBuf,
    size: u64,
    sha256: String,
}
pub struct Handoff {
    child: Child,
    ready: PathBuf,
    version: String,
    started: Instant,
}

fn version(value: &str) -> Result<[u32; 3]> {
    let values = value
        .split('.')
        .map(str::parse::<u32>)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    values
        .try_into()
        .map_err(|_| anyhow::anyhow!("Нужна версия лаунчера major.minor.patch"))
}
fn verify(index: &SignedIndex, source: &Url, key: &[u8; 32]) -> Result<Available> {
    ensure!(
        index.schema_version == 1 && index.payload.len() <= MAX_INDEX as usize,
        "Неверный индекс лаунчера"
    );
    ensure!(
        index.signature.len() == 128 && index.signature.bytes().all(|b| b.is_ascii_hexdigit()),
        "Неверная подпись лаунчера"
    );
    let signature = (0..128)
        .step_by(2)
        .map(|i| u8::from_str_radix(&index.signature[i..i + 2], 16))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let signature = Signature::try_from(signature.as_slice())?;
    let message = [DOMAIN, index.payload.as_bytes()].concat();
    VerifyingKey::from_bytes(key)?
        .verify_strict(&message, &signature)
        .context("Подпись обновления лаунчера не прошла проверку")?;
    let manifest: Manifest = serde_json::from_str(&index.payload)?;
    manifest.validate(source)?;
    let release = manifest.release.context("В индексе нет выпуска лаунчера")?;
    version(&release.version)?;
    ensure!(
        release.data_directory.is_none() && release.files.len() == 2 * release.entrypoints.len(),
        "Недопустимый пакет лаунчера"
    );
    for (platform, entrypoint) in &release.entrypoints {
        let expected = if platform.starts_with("windows-") {
            "bin/opensc-launcher.exe"
        } else {
            "bin/opensc-launcher"
        };
        let files = release.files_for(platform).collect::<Vec<_>>();
        ensure!(
            entrypoint == expected
                && files.len() == 2
                && files
                    .iter()
                    .any(|f| f.path == *entrypoint && f.executable && f.size <= 128 * 1024 * 1024)
                && files.iter().any(|f| f.path == "THIRD-PARTY-NOTICES.txt"
                    && !f.executable
                    && f.size <= 8 * 1024 * 1024),
            "Неверный файл лаунчера"
        );
    }
    Ok(Available {
        source: source.clone(),
        release,
    })
}

pub fn verify_index(bytes: &[u8], source: &Url) -> Result<Available> {
    validate_source(source)?;
    ensure!(
        bytes.len() <= MAX_INDEX as usize,
        "Индекс лаунчера слишком большой"
    );
    let index: SignedIndex = serde_json::from_slice(bytes)?;
    verify(&index, source, PUBLIC_KEY)
}

pub fn prepare(root: &Path, game_source: &str) -> Result<Option<Prepared>> {
    let source = Url::parse(game_source)?.join("/updates/launcher-v1.json")?;
    validate_source(&source)?;
    let origin = source.origin();
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() > 3 || attempt.url().origin() != origin {
                attempt.error("Cross-origin launcher redirect")
            } else {
                attempt.follow()
            }
        }))
        .build()?;
    let mut response = client
        .get(source.clone())
        .header(reqwest::header::CACHE_CONTROL, "no-cache")
        .send()?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    response.error_for_status_ref()?;
    let mut bytes = Vec::new();
    response
        .by_ref()
        .take(MAX_INDEX + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= MAX_INDEX as usize,
        "Индекс лаунчера слишком большой"
    );
    let index: SignedIndex = serde_json::from_slice(&bytes)?;
    let available = verify(&index, &source, PUBLIC_KEY)?;
    prepare_verified(root, &available, &index, env!("CARGO_PKG_VERSION"))
}

fn prepare_verified(
    root: &Path,
    available: &Available,
    index: &SignedIndex,
    current: &str,
) -> Result<Option<Prepared>> {
    if version(&available.release.version)? <= version(current)? || !updater::supported(available) {
        return Ok(None);
    }
    let store = root.join("launcher-update");
    let installed = updater::install(available, &store, &AtomicBool::new(false), |_| {})?;
    let platform = crate::platform();
    let file = available
        .release
        .files_for(&platform)
        .find(|file| file.executable)
        .context("Нет файла для этой платформы")?;
    let executable = installed.directory(&store)?.join(&file.path);
    updater::verify_file(&executable, file.size, &file.sha256)?;
    atomic_json(&store.join("signed-index.json"), index)?;
    Ok(Some(Prepared {
        version: available.release.version.clone(),
        executable,
        size: file.size,
        sha256: file.sha256.clone(),
    }))
}

fn token_path(root: &Path, token: &str) -> Result<PathBuf> {
    ensure!(
        token.len() == 32 && token.bytes().all(|b| b.is_ascii_hexdigit()),
        "Неверный токен запуска лаунчера"
    );
    Ok(root
        .join("launcher-update/handoff")
        .join(format!("{token}.json")))
}
pub fn signal_ready(root: &Path) -> Result<()> {
    if let Ok(token) = std::env::var("OPENSC_LAUNCHER_HANDOFF") {
        atomic_json(
            &token_path(root, &token)?,
            &serde_json::json!({"version":env!("CARGO_PKG_VERSION")}),
        )?;
    }
    Ok(())
}
impl Prepared {
    pub fn start(self, root: &Path) -> Result<Handoff> {
        updater::verify_file(&self.executable, self.size, &self.sha256)?;
        let token = crate::reports::request_id();
        let ready = token_path(root, &token)?;
        if let Some(parent) = ready.parent() {
            fs::create_dir_all(parent)?;
        }
        ensure!(!ready.exists(), "Токен запуска уже существует");
        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(root.join("launcher-update/launcher.log"))?;
        let child = Command::new(&self.executable)
            .env("OPENSC_HOME", root)
            .env("OPENSC_LAUNCHER_HANDOFF", &token)
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log)
            .spawn()
            .context("Не удалось запустить новый лаунчер")?;
        Ok(Handoff {
            child,
            ready,
            version: self.version,
            started: Instant::now(),
        })
    }
}
impl Handoff {
    /// Close the old window only after the new version has created its GUI.
    pub fn cancel(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_file(&self.ready);
    }
    pub fn poll(&mut self) -> Result<bool> {
        if self.ready.is_file() {
            let data: serde_json::Value = serde_json::from_slice(&fs::read(&self.ready)?)?;
            let _ = fs::remove_file(&self.ready);
            if data["version"].as_str() == Some(&self.version) {
                return Ok(true);
            }
            let _ = self.child.kill();
            let _ = self.child.wait();
            anyhow::bail!("Версия нового лаунчера не совпала; текущий оставлен открытым");
        }
        ensure!(
            self.child.try_wait()?.is_none(),
            "Новый лаунчер завершился; текущий оставлен открытым"
        );
        if self.started.elapsed() > Duration::from_secs(20) {
            let _ = self.child.kill();
            let _ = self.child.wait();
            anyhow::bail!("Новый лаунчер не открыл окно; текущий оставлен открытым");
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    #[test]
    fn launcher_signature_rejects_tampering_and_wrong_signer() {
        let key = SigningKey::from_bytes(&[7; 32]);
        let source = Url::parse("https://example.com/updates/launcher-v1.json").unwrap();
        let payload = serde_json::json!({"schema_version":1,"release":{"version":"0.5.0","notes":"test","data_directory":null,
            "entrypoints":{"linux-x86_64":"bin/opensc-launcher"},"files":[{"path":"bin/opensc-launcher","url":"launcher-releases/0.5.0/linux-x86_64/bin/opensc-launcher",
            "size":4,"sha256":"0".repeat(64),"executable":true,"platform":"linux-x86_64"},{"path":"THIRD-PARTY-NOTICES.txt","url":"launcher-releases/0.5.0/linux-x86_64/THIRD-PARTY-NOTICES.txt","size":1,"sha256":"0".repeat(64),"executable":false,"platform":"linux-x86_64"}]}}).to_string();
        let signature = key.sign(&[DOMAIN, payload.as_bytes()].concat());
        let mut index = SignedIndex {
            schema_version: 1,
            payload,
            signature: crate::manifest::hex(&signature.to_bytes()),
        };
        assert!(verify(&index, &source, &key.verifying_key().to_bytes()).is_ok());
        let original = index.payload.clone();
        index.payload = index.payload.replace("0.5.0", "0.6.0");
        assert!(verify(&index, &source, &key.verifying_key().to_bytes()).is_err());
        index.payload = original;
        assert!(
            verify(
                &index,
                &source,
                &SigningKey::from_bytes(&[8; 32]).verifying_key().to_bytes()
            )
            .is_err()
        );
    }
    #[test]
    fn versions_do_not_downgrade_and_handoff_tokens_are_local_names() {
        assert!(version("0.4.1").unwrap() > version("0.4.0").unwrap());
        assert!(version("0.10.0").unwrap() > version("0.9.9").unwrap());
        assert!(version("0.4.0-beta").is_err());
        assert!(token_path(Path::new("/tmp/root"), "../escape").is_err());
    }

    #[test]
    #[cfg(unix)]
    fn handoff_waits_for_ready_and_failed_child_preserves_the_profile() {
        use sha2::{Digest, Sha256};
        use std::os::unix::fs::PermissionsExt;
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("profile with spaces");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("keep-settings"), b"user settings").unwrap();
        let executable = root.join("new launcher");
        let script = "#!/bin/sh\nmkdir -p \"$OPENSC_HOME/launcher-update/handoff\"\nprintf '%s' '{\"version\":\"0.5.0\"}' > \"$OPENSC_HOME/launcher-update/handoff/$OPENSC_LAUNCHER_HANDOFF.json\"\nexec sleep 5\n";
        fs::write(&executable, script).unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
        let prepared = Prepared {
            version: "0.5.0".into(),
            executable: executable.clone(),
            size: script.len() as u64,
            sha256: crate::manifest::hex(&Sha256::digest(script.as_bytes())),
        };
        let mut handoff = prepared.start(&root).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !handoff.poll().unwrap() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        handoff.cancel();
        let script = "#!/bin/sh\nexit 7\n";
        fs::write(&executable, script).unwrap();
        let prepared = Prepared {
            version: "0.5.0".into(),
            executable,
            size: script.len() as u64,
            sha256: crate::manifest::hex(&Sha256::digest(script.as_bytes())),
        };
        let mut handoff = prepared.start(&root).unwrap();
        loop {
            match handoff.poll() {
                Ok(false) => std::thread::sleep(Duration::from_millis(10)),
                Err(_) => break,
                Ok(true) => panic!("failed child signalled ready"),
            }
        }
        assert_eq!(
            fs::read(root.join("keep-settings")).unwrap(),
            b"user settings"
        );
    }
}
