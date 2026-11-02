//! Explicit, previewable reports. Never upload original assets or whole log files.
use crate::{
    config::{self, Config},
    manifest::validate_source,
    updater::Installed,
};
use anyhow::{Context, Result, ensure};
use opensc_report_protocol::{Diagnostics, Receipt, Report};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use url::Url;

pub fn request_id() -> String {
    let seed = format!(
        "{}-{}-{:?}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        std::thread::current().id()
    );
    crate::manifest::hex(&Sha256::digest(seed.as_bytes()))[..32].into()
}

fn truncate(text: &mut String, max: usize) {
    let mut end = text.len().min(max);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
}

pub fn redact(text: &str, root: &Path, config: &Config) -> String {
    let mut result = text.to_owned();
    for (value, replacement) in [
        (config.data_path.clone(), "<game-data>"),
        (config.overrides_path.clone(), "<overrides>"),
        (root.to_string_lossy().into_owned(), "<opensc>"),
        (std::env::var("HOME").unwrap_or_default(), "<home>"),
        (std::env::var("USERPROFILE").unwrap_or_default(), "<home>"),
    ] {
        if !value.is_empty() {
            result = result.replace(&value, replacement);
        }
    }
    // A log may contain paths from dependencies built on another machine.
    for prefix in ["/home/", "/Users/", "C:\\Users\\", "C:/Users/"] {
        let separator = if prefix.contains('\\') { '\\' } else { '/' };
        let mut offset = 0;
        while let Some(start) = result[offset..].find(prefix) {
            let start = offset + start;
            let name_start = start + prefix.len();
            let rest = &result[name_start..];
            let end = name_start
                + rest
                    .find(|c: char| c == separator || c.is_whitespace() || c == '"')
                    .unwrap_or(rest.len());
            result.replace_range(start..end, "<home>");
            offset = start + "<home>".len();
        }
    }
    result.retain(|c| !c.is_control() || ['\n', '\r', '\t'].contains(&c));
    result
}

pub fn diagnostics(
    root: &Path,
    config: &Config,
    installed: Option<&Installed>,
    include_log: bool,
) -> Diagnostics {
    let mut log = String::new();
    if let Ok(mut file) = File::open(root.join("engine.log")) {
        let size = file.metadata().map(|m| m.len()).unwrap_or(0);
        if file
            .seek(SeekFrom::Start(size.saturating_sub(32768)))
            .is_ok()
        {
            let mut bytes = Vec::new();
            if file.take(32768).read_to_end(&mut bytes).is_ok() {
                log = String::from_utf8_lossy(&bytes).into_owned();
            }
        }
    }
    log = redact(&log, root, config);
    let mut graphics = log
        .lines()
        .filter(|line| {
            line.contains("AdapterInfo") || line.contains("Vulkan") || line.contains("Metal")
        })
        .take(2)
        .collect::<Vec<_>>()
        .join("\n");
    truncate(&mut graphics, 512);
    // Keep the most recent bounded tail, on a UTF-8 boundary.
    if log.len() > 24000 {
        let mut start = log.len() - 24000;
        while !log.is_char_boundary(start) {
            start += 1;
        }
        log = log[start..].to_owned();
    }
    let source_sha256 = installed
        .and_then(|i| i.directory(root).ok())
        .and_then(|dir| File::open(dir.join("build-info.json")).ok())
        .and_then(|file| {
            serde_json::from_reader::<_, serde_json::Value>(file.take(128 * 1024)).ok()
        })
        .and_then(|value| {
            value
                .get("source_sha256")
                .and_then(|v| v.as_str())
                .map(str::to_owned)
        })
        .unwrap_or_default();
    Diagnostics {
        launcher_version: env!("CARGO_PKG_VERSION").into(),
        engine_version: installed
            .map(|i| i.release.version.clone())
            .unwrap_or_default(),
        source_sha256,
        platform: crate::platform(),
        os: format!("{} / {}", std::env::consts::OS, std::env::consts::ARCH),
        graphics,
        log_excerpt: if include_log { log } else { String::new() },
    }
}

pub fn save_outbox(root: &Path, report: &Report) -> Result<()> {
    report.validate().map_err(anyhow::Error::msg)?;
    ensure!(
        serde_json::to_vec(report)?.len() <= opensc_report_protocol::MAX_BODY - 4096,
        "Обращение слишком длинное. Сократи текст или отключи журнал"
    );
    config::atomic_json(
        &root
            .join("reports/outbox")
            .join(format!("{}.json", report.request_id)),
        report,
    )
}

pub fn pending(root: &Path) -> Option<Report> {
    let mut files = fs::read_dir(root.join("reports/outbox"))
        .ok()?
        .filter_map(|entry| entry.ok())
        .map(|e| e.path())
        .collect::<Vec<_>>();
    files.sort();
    files.into_iter().find_map(|path| {
        if path.extension()?.to_str()? != "json"
            || path.symlink_metadata().ok()?.len() > opensc_report_protocol::MAX_BODY as u64
        {
            return None;
        }
        let report: Report = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
        report.validate().ok()?;
        Some(report)
    })
}

pub fn send(root: &Path, config: &Config, report: &Report) -> Result<Receipt> {
    save_outbox(root, report)?;
    let source = Url::parse(&config.manifest_url)?;
    validate_source(&source)?;
    let endpoint = source.join(opensc_report_protocol::API_PATH)?;
    let mut response = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none())
        .build()?
        .post(endpoint)
        .header(
            reqwest::header::USER_AGENT,
            concat!("OpenSCLauncher/", env!("CARGO_PKG_VERSION")),
        )
        .json(report)
        .send()
        .context("Сервер недоступен. Обращение сохранено локально; отправь его повторно")?;
    if response.status().as_u16() == 429 {
        anyhow::bail!(
            "Слишком много обращений. Подожди {} секунд и повтори отправку",
            response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|h| h.to_str().ok())
                .unwrap_or("60")
        );
    }
    response
        .error_for_status_ref()
        .context("Сервер не принял обращение; локальная копия сохранена")?;
    let mut bytes = Vec::new();
    response.by_ref().take(4097).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 4096, "Слишком большой ответ сервера");
    let receipt: Receipt = serde_json::from_slice(&bytes)?;
    ensure!(
        receipt.ticket_id.starts_with("OSC-")
            && receipt.ticket_id.len() == 16
            && receipt.ticket_id[4..]
                .bytes()
                .all(|c| c.is_ascii_hexdigit()),
        "Сервер вернул неверный номер обращения"
    );
    config::atomic_json(
        &root
            .join("reports/receipts")
            .join(format!("{}.json", report.request_id)),
        &receipt,
    )?;
    fs::remove_file(
        root.join("reports/outbox")
            .join(format!("{}.json", report.request_id)),
    )?;
    Ok(receipt)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn diagnostics_remove_local_and_build_machine_user_paths() {
        let config = Config {
            data_path: "/games/my secret/Data".into(),
            overrides_path: "/mods/custom".into(),
            ..Config::default()
        };
        let output = redact(
            "/games/my secret/Data/a /mods/custom/b /private/opensc/log /home/builder/src /Users/alice/file C:\\Users\\Bob\\source",
            Path::new("/private/opensc"),
            &config,
        );
        for private in [
            "my secret",
            "custom",
            "builder",
            "alice",
            "Bob",
            "/private/opensc",
        ] {
            assert!(!output.contains(private), "{output}");
        }
        assert!(output.contains("<game-data>/a") && output.contains("<home>/src"));
    }
    #[test]
    fn unicode_truncation_keeps_valid_report_text() {
        let mut text = "Ошибка".repeat(300);
        truncate(&mut text, 513);
        assert!(text.len() <= 513 && !text.is_empty());
        assert_eq!(request_id().len(), 32);
    }
}
