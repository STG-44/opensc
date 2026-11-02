use opensc_launcher::{
    manifest::{Manifest, Release, ReleaseFile, hex, validate_path},
    resources,
    updater::{self, Available, Installed},
};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    net::TcpListener,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};
use url::Url;

struct Server {
    url: Url,
    done: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Server {
    fn new(body: Vec<u8>, status: u16) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = Url::parse(&format!(
            "http://{}/updates/manifest-v1.json",
            listener.local_addr().unwrap()
        ))
        .unwrap();
        listener.set_nonblocking(true).unwrap();
        let done = Arc::new(AtomicBool::new(false));
        let worker_done = done.clone();
        let worker = thread::spawn(move || {
            while !worker_done.load(Ordering::Relaxed) {
                if let Ok((mut stream, _)) = listener.accept() {
                    // BSD/macOS and Windows may inherit the listener's
                    // nonblocking mode; Linux accept() does not.
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    // TCP can split headers across reads. Closing the socket
                    // with unread request bytes can reset it on Windows.
                    let mut request = Vec::new();
                    let mut chunk = [0u8; 4096];
                    while request.len() <= 65536 && !request.windows(4).any(|w| w == b"\r\n\r\n") {
                        let count = stream.read(&mut chunk).unwrap_or(0);
                        if count == 0 {
                            break;
                        }
                        request.extend_from_slice(&chunk[..count]);
                    }
                    if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&request[..end]);
                        let length = headers
                            .lines()
                            .find_map(|line| {
                                line.split_once(':')
                                    .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                                    .and_then(|(_, value)| value.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        while request.len() < end + 4 + length && request.len() <= 65536 {
                            let count = stream.read(&mut chunk).unwrap_or(0);
                            if count == 0 {
                                break;
                            }
                            request.extend_from_slice(&chunk[..count]);
                        }
                    }
                    let header = format!(
                        "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(header.as_bytes());
                    let _ = stream.write_all(&body);
                } else {
                    thread::sleep(Duration::from_millis(5));
                }
            }
        });
        Self {
            url,
            done,
            worker: Some(worker),
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.done.store(true, Ordering::Relaxed);
        self.worker.take().unwrap().join().unwrap();
    }
}

#[test]
fn bug_report_outbox_survives_network_failure_and_success_keeps_a_receipt() {
    use opensc_launcher::{config::Config, reports};
    use opensc_report_protocol::{Diagnostics, Report};
    let root = tempfile::tempdir().unwrap();
    let report = Report {
        schema_version: 1,
        request_id: "f".repeat(32),
        category: "interface".into(),
        title: "Inventory input freeze".into(),
        steps: "Open training and click the inventory button".into(),
        expected: "Controls work".into(),
        actual: "Inventory stays open".into(),
        diagnostics: Diagnostics {
            launcher_version: "0.4.0".into(),
            platform: opensc_launcher::platform(),
            ..Diagnostics::default()
        },
    };
    let unavailable = Server::new(b"temporarily unavailable".to_vec(), 503);
    let mut config = Config {
        manifest_url: unavailable.url.to_string(),
        ..Config::default()
    };
    assert!(reports::send(root.path(), &config, &report).is_err());
    assert_eq!(
        reports::pending(root.path()).unwrap().request_id,
        report.request_id
    );
    let available = Server::new(
        br#"{"ticket_id":"OSC-123456ABCDEF","status":"new","duplicate":true}"#.to_vec(),
        201,
    );
    config.manifest_url = available.url.to_string();
    let receipt = reports::send(root.path(), &config, &report).unwrap();
    assert_eq!(receipt.ticket_id, "OSC-123456ABCDEF");
    assert!(receipt.duplicate && reports::pending(root.path()).is_none());
    assert!(
        root.path()
            .join("reports/receipts")
            .join(format!("{}.json", report.request_id))
            .is_file()
    );
}

#[test]
fn throttled_download_honors_retry_after_and_installs_without_manual_restart() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let source = Url::parse(&format!(
        "http://{}/manifest.json",
        listener.local_addr().unwrap()
    ))
    .unwrap();
    let worker = thread::spawn(move || {
        for attempt in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 1024];
            while !request.ends_with(b"\r\n\r\n") {
                let n = stream.read(&mut buffer).unwrap();
                if n == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..n]);
            }
            let response = if attempt == 0 {
                "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 1\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            } else {
                "HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\nengine"
            };
            stream.write_all(response.as_bytes()).unwrap();
        }
    });
    let root = tempfile::tempdir().unwrap();
    let start = std::time::Instant::now();
    let installed = updater::install(
        &Available {
            source,
            release: release(b"engine"),
        },
        root.path(),
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    assert!(start.elapsed() >= Duration::from_secs(1));
    installed.verify(root.path()).unwrap();
    worker.join().unwrap();
}

#[test]
fn fixture_waits_for_complete_http_headers() {
    let server = Server::new(b"complete response".to_vec(), 200);
    let address = format!("127.0.0.1:{}", server.url.port().unwrap());
    let mut stream = std::net::TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_millis(100)))
        .unwrap();
    stream
        .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n")
        .unwrap();
    let mut byte = [0];
    let error = stream.read(&mut byte).unwrap_err();
    assert!(matches!(
        error.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    ));
    stream.write_all(b"\r\n").unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    assert!(response.ends_with("complete response"));
}

fn release(bytes: &[u8]) -> Release {
    Release {
        version: "0.1.0-test".into(),
        notes: "Integration test fixture".into(),
        data_directory: None,
        entrypoints: BTreeMap::from([(opensc_launcher::platform(), "bin/opensc-engine".into())]),
        files: vec![ReleaseFile {
            path: "bin/opensc-engine".into(),
            url: "engine".into(),
            size: bytes.len() as u64,
            sha256: hex(&Sha256::digest(bytes)),
            executable: true,
            platform: Some(opensc_launcher::platform()),
        }],
    }
}

#[test]
fn rejects_paths_unsafe_on_windows_and_unix() {
    for path in [
        "../escape",
        "/absolute",
        "a/../b",
        "a\\b",
        "C:/file",
        "a//b",
        "a/./b",
        "bin/NUL.txt",
        "bin/com1.exe",
        "bad.",
        "a:b",
        "a\0b",
    ] {
        assert!(validate_path(path).is_err(), "accepted {path:?}");
    }
    validate_path("data/Textures/ground.pcx").unwrap();
}

#[test]
fn rejects_duplicate_paths_cross_origin_and_directory_conflicts() {
    let source = Url::parse("https://example.com/updates/manifest.json").unwrap();
    let mut r = release(b"engine");
    let mut duplicate = r.files[0].clone();
    duplicate.path = "BIN/OPENSC-ENGINE".into();
    r.files.push(duplicate);
    assert!(r.validate(&source).is_err());
    r.files.pop();
    r.files[0].url = "https://other.example/engine".into();
    assert!(r.validate(&source).is_err());
    r.files[0].url = "engine".into();
    let mut conflict = r.files[0].clone();
    conflict.path = "bin".into();
    r.files.push(conflict);
    assert!(r.validate(&source).is_err());
}

#[test]
fn no_release_handles_missing_manifest_and_empty_release() {
    let missing = Server::new(Vec::new(), 404);
    assert!(updater::check(missing.url.as_str()).unwrap().is_none());
    let empty = Server::new(br#"{"schema_version":1,"release":null}"#.to_vec(), 200);
    assert!(updater::check(empty.url.as_str()).unwrap().is_none());
    let placeholder = Server::new(b"<html>Not a manifest</html>".to_vec(), 200);
    assert!(updater::check(placeholder.url.as_str()).is_err());
}

#[test]
fn installs_and_detects_local_file_corruption() {
    let bytes = b"engine fixture";
    let server = Server::new(bytes.to_vec(), 200);
    let available = Available {
        source: server.url.clone(),
        release: release(bytes),
    };
    let root = tempfile::tempdir().unwrap();
    let installed =
        updater::install(&available, root.path(), &AtomicBool::new(false), |_| {}).unwrap();
    installed.verify(root.path()).unwrap();
    assert!(Installed::load(root.path()).unwrap().is_some());
    fs::write(
        installed
            .directory(root.path())
            .unwrap()
            .join("bin/opensc-engine"),
        b"bad",
    )
    .unwrap();
    assert!(installed.verify(root.path()).is_err());
}

#[test]
fn damaged_download_keeps_previous_active_release() {
    let server = Server::new(b"good".to_vec(), 200);
    let available = Available {
        source: server.url.clone(),
        release: release(b"good"),
    };
    let root = tempfile::tempdir().unwrap();
    updater::install(&available, root.path(), &AtomicBool::new(false), |_| {}).unwrap();
    let before = fs::read(root.path().join("active.json")).unwrap();
    let bad_server = Server::new(b"evil".to_vec(), 200);
    let mut next = available.clone();
    next.source = bad_server.url.clone();
    next.release.version = "0.2.0-test".into();
    assert!(updater::install(&next, root.path(), &AtomicBool::new(false), |_| {}).is_err());
    assert_eq!(before, fs::read(root.path().join("active.json")).unwrap());
    Installed::load(root.path())
        .unwrap()
        .unwrap()
        .verify(root.path())
        .unwrap();
    assert_eq!(
        fs::read_dir(root.path().join("versions")).unwrap().count(),
        1
    );
}

#[test]
fn cancellation_does_not_publish_partial_installation() {
    let server = Server::new(b"good".to_vec(), 200);
    let available = Available {
        source: server.url.clone(),
        release: release(b"good"),
    };
    let root = tempfile::tempdir().unwrap();
    assert!(updater::install(&available, root.path(), &AtomicBool::new(true), |_| {}).is_err());
    assert!(!root.path().join("active.json").exists());
    assert_eq!(
        fs::read_dir(root.path().join("versions")).unwrap().count(),
        0
    );
}

#[test]
fn discovers_manifest_and_rejects_unknown_schema() {
    let manifest = Manifest {
        schema_version: 1,
        release: Some(release(b"fixture")),
    };
    let server = Server::new(serde_json::to_vec(&manifest).unwrap(), 200);
    assert!(updater::check(server.url.as_str()).unwrap().is_some());
    let bad = Manifest {
        schema_version: 2,
        release: None,
    };
    assert!(bad.validate(&server.url).is_err());
}

#[test]
fn inventories_without_following_symlinks() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("Data")).unwrap();
    fs::write(root.path().join("Data/texture.pcx"), b"pcx fixture").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(root.path(), root.path().join("Data/loop")).unwrap();
    let data = resources::resolve_data(root.path());
    let inventory = resources::inspect(&data).unwrap();
    assert_eq!(inventory.files, 1);
    assert_eq!(inventory.by_extension.get("pcx"), Some(&1));
}

#[cfg(unix)]
#[test]
fn launches_with_data_path_as_one_argument() {
    let script = b"#!/bin/sh\nprintf '%s\\n' \"$@\" > launch-args.txt\n";
    let server = Server::new(script.to_vec(), 200);
    let available = Available {
        source: server.url.clone(),
        release: release(script),
    };
    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("Data with spaces and $literal");
    fs::create_dir(&data).unwrap();
    let installed =
        updater::install(&available, root.path(), &AtomicBool::new(false), |_| {}).unwrap();
    assert!(
        installed
            .launch(root.path(), &data)
            .unwrap()
            .wait()
            .unwrap()
            .success()
    );
    let arguments = fs::read_to_string(
        installed
            .directory(root.path())
            .unwrap()
            .join("launch-args.txt"),
    )
    .unwrap();
    assert_eq!(
        arguments,
        format!(
            "--game-dir\n{}\n--settings\n{}\n--overrides\n{}\n",
            data.canonicalize().unwrap().display(),
            root.path()
                .canonicalize()
                .unwrap()
                .join("game/settings.json")
                .display(),
            root.path()
                .canonicalize()
                .unwrap()
                .join("overrides")
                .display()
        )
    );
}

#[test]
fn old_launcher_configuration_keeps_defaults_for_game_integration() {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("config.json"),
        br#"{"data_path":"existing/Data"}"#,
    )
    .unwrap();
    let config = opensc_launcher::config::Config::load(root.path()).unwrap();
    assert_eq!(config.data_path, "existing/Data");
    assert!(config.overrides_path.is_empty());
    assert!(config.auto_update);
}

#[cfg(unix)]
#[test]
fn updating_engine_preserves_settings_and_passes_unicode_overrides_without_shell() {
    let script = b"#!/bin/sh\nprintf '%s\\n' \"$@\" > launch-args.txt\n";
    let server = Server::new(script.to_vec(), 200);
    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("Оригинал с пробелами; $literal");
    let overrides = root.path().join("Улучшения с пробелами; $literal");
    fs::create_dir(&data).unwrap();
    fs::create_dir(&overrides).unwrap();
    fs::write(data.join("original.gut"), b"original unchanged").unwrap();
    let config = opensc_launcher::config::Config {
        overrides_path: overrides.to_string_lossy().into_owned(),
        ..Default::default()
    };
    config.save(root.path()).unwrap();
    fs::create_dir(root.path().join("game")).unwrap();
    fs::write(root.path().join("game/settings.json"), b"user preferences").unwrap();
    let mut available = Available {
        source: server.url.clone(),
        release: release(script),
    };
    let old = updater::install(&available, root.path(), &AtomicBool::new(false), |_| {}).unwrap();
    available.release.version = "0.1.1-test".into();
    let new = updater::install(&available, root.path(), &AtomicBool::new(false), |_| {}).unwrap();
    assert!(old.directory(root.path()).unwrap().is_dir());
    assert!(
        new.launch(root.path(), &data)
            .unwrap()
            .wait()
            .unwrap()
            .success()
    );
    let arguments =
        fs::read_to_string(new.directory(root.path()).unwrap().join("launch-args.txt")).unwrap();
    assert_eq!(
        arguments,
        format!(
            "--game-dir\n{}\n--settings\n{}\n--overrides\n{}\n",
            data.canonicalize().unwrap().display(),
            root.path()
                .canonicalize()
                .unwrap()
                .join("game/settings.json")
                .display(),
            overrides.canonicalize().unwrap().display()
        )
    );
    assert_eq!(
        fs::read(root.path().join("game/settings.json")).unwrap(),
        b"user preferences"
    );
    assert_eq!(
        fs::read(data.join("original.gut")).unwrap(),
        b"original unchanged"
    );
    assert_eq!(
        opensc_launcher::config::Config::load(root.path())
            .unwrap()
            .overrides_path,
        config.overrides_path
    );
}
