use super::*;
use opensc_launcher::manifest::{Manifest, Release, ReleaseFile, hex};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::TcpListener,
    sync::Mutex,
    thread,
};

struct Server {
    source: String,
    release: Arc<Mutex<Release>>,
    downloads: Arc<std::sync::atomic::AtomicUsize>,
    checks: Arc<std::sync::atomic::AtomicUsize>,
    unavailable: Arc<AtomicBool>,
    stopped: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Server {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let source = format!("http://{}/manifest.json", listener.local_addr().unwrap());
        let data = b"# OpenSC updater test fixture\nexit 0\n";
        let release = Arc::new(Mutex::new(Release {
            version: "ui-test-1".into(),
            notes: "Test release".into(),
            data_directory: None,
            entrypoints: BTreeMap::from([(opensc_launcher::platform(), "bin/engine".into())]),
            files: vec![ReleaseFile {
                path: "bin/engine".into(),
                url: "engine".into(),
                size: data.len() as u64,
                sha256: hex(&Sha256::digest(data)),
                executable: true,
                platform: Some(opensc_launcher::platform()),
            }],
        }));
        let downloads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let checks = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let requests = checks.clone();
        let unavailable = Arc::new(AtomicBool::new(false));
        let failures = unavailable.clone();
        let stopped = Arc::new(AtomicBool::new(false));
        let (state, count, done) = (release.clone(), downloads.clone(), stopped.clone());
        let worker = thread::spawn(move || {
            while !done.load(Ordering::Relaxed) {
                if let Ok((mut stream, _)) = listener.accept() {
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    let mut request = Vec::new();
                    let mut buffer = [0; 1024];
                    while request.len() < 65536 && !request.ends_with(b"\r\n\r\n") {
                        let n = stream.read(&mut buffer).unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        request.extend_from_slice(&buffer[..n]);
                    }
                    let request = String::from_utf8_lossy(&request);
                    let body = if request.starts_with("GET /manifest.json ") {
                        requests.fetch_add(1, Ordering::Relaxed);
                        if failures.load(Ordering::Relaxed) {
                            let _ = stream.write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                            continue;
                        }
                        assert!(request.to_lowercase().contains("cache-control: no-cache"));
                        serde_json::to_vec(&Manifest {
                            schema_version: 1,
                            release: Some(state.lock().unwrap().clone()),
                        })
                        .unwrap()
                    } else if request.starts_with("GET /updates/launcher-v1.json ") {
                        let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                        continue;
                    } else {
                        count.fetch_add(1, Ordering::Relaxed);
                        data.to_vec()
                    };
                    let header = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
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
            source,
            release,
            downloads,
            checks,
            unavailable,
            stopped,
            worker: Some(worker),
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Relaxed);
        self.worker.take().unwrap().join().unwrap();
    }
}

fn app(root: &Path, source: &str) -> Launcher {
    Launcher {
        root: root.into(),
        config: Config {
            manifest_url: source.into(),
            ..Config::default()
        },
        installed: None,
        available: None,
        inventory: None,
        receiver: None,
        cancel: Arc::new(AtomicBool::new(false)),
        status: String::new(),
        error: None,
        progress: None,
        activity: Vec::new(),
        settings: false,
        child: None,
        inspected_path: None,
        importing: false,
        hero: None,
        pending_play: false,
        blocked_update: None,
        download_started: None,
        reports: crate::report_ui::Form::default(),
        launcher_receiver: None,
        launcher_ready: None,
        launcher_handoff: None,
        launcher_status: String::new(),
    }
}

fn settle(app: &mut Launcher, ctx: &egui::Context) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        app.tick(ctx);
        app.poll_launcher(ctx);
        if !app.busy() && app.launcher_receiver.is_none() {
            break;
        }
        assert!(Instant::now() < deadline, "{}", app.status);
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn startup_checks_once_and_idle_does_not_fetch_later_publications() {
    let server = Server::new();
    let root = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut app = app(root.path(), &server.source);
    app.check(&ctx);
    settle(&mut app, &ctx);
    assert_eq!(app.installed.as_ref().unwrap().release.version, "ui-test-1");
    server.release.lock().unwrap().version = "ui-test-2".into();
    for _ in 0..1000 {
        app.tick(&ctx);
        app.poll_launcher(&ctx);
    }
    assert_eq!(server.checks.load(Ordering::Relaxed), 1);
    assert_eq!(server.downloads.load(Ordering::Relaxed), 1);
    assert_eq!(app.installed.as_ref().unwrap().release.version, "ui-test-1");
}

fn ready_data(app: &mut Launcher) {
    let root = app.root.join("fixture-data");
    std::fs::create_dir_all(&root).unwrap();
    app.config.data_path = root.to_string_lossy().into_owned();
    app.inspected_path = Some(app.config.data_path.clone());
    app.inventory = Some(Inventory {
        root,
        files: 1,
        bytes: 1,
        by_extension: BTreeMap::new(),
        case_collisions: vec![],
    });
}

// Platforms can reject this non-native fixture at spawn or return a short-lived
// child. Both paths must follow verification and activation.
fn assert_fixture_launch(app: &mut Launcher) {
    assert!(!app.pending_play);
    assert!(app.root.join("engine.log").is_file());
    if let Some(mut child) = app.child.take() {
        let _ = child.wait().unwrap();
    } else {
        assert!(
            app.error
                .as_ref()
                .is_some_and(|e| e.contains("Не удалось запустить движок"))
                || app
                    .activity
                    .iter()
                    .any(|a| a.starts_with("Движок завершился:")),
            "{:?}",
            app.error
        );
    }
}

#[test]
fn play_rechecks_and_installs_latest_before_attempting_to_execute_it() {
    let server = Server::new();
    let root = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut app = app(root.path(), &server.source);
    app.check(&ctx);
    settle(&mut app, &ctx);
    ready_data(&mut app);
    server.release.lock().unwrap().version = "ui-test-2".into();
    app.request_play(&ctx);
    settle(&mut app, &ctx);
    assert_eq!(server.checks.load(Ordering::Relaxed), 2);
    assert_eq!(server.downloads.load(Ordering::Relaxed), 2);
    assert_eq!(app.installed.as_ref().unwrap().release.version, "ui-test-2");
    assert!(!app.pending_play);
    assert_fixture_launch(&mut app);
}

#[test]
fn switching_channel_is_saved_without_a_network_check_or_install() {
    let server = Server::new();
    let root = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut app = app(root.path(), &server.source);
    app.check(&ctx);
    settle(&mut app, &ctx);
    let active = std::fs::read(root.path().join("active.json")).unwrap();
    app.select_channel(UpdateChannel::Test);
    assert_eq!(
        Config::load(root.path()).unwrap().channel,
        UpdateChannel::Test
    );
    assert_eq!(
        Config::load(root.path()).unwrap().manifest_url,
        opensc_launcher::TEST_MANIFEST
    );
    for _ in 0..100 {
        app.tick(&ctx);
    }
    assert_eq!(server.checks.load(Ordering::Relaxed), 1);
    assert_eq!(
        std::fs::read(root.path().join("active.json")).unwrap(),
        active
    );
    assert!(app.available.is_none());
    app.select_channel(UpdateChannel::Stable);
    assert_eq!(
        Config::load(root.path()).unwrap().channel,
        UpdateChannel::Stable
    );
}

#[test]
fn explicit_retry_clears_a_cancelled_download_without_idle_restarts() {
    let server = Server::new();
    let root = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut app = app(root.path(), &server.source);
    app.available = updater::check(&server.source).unwrap();
    app.blocked_update = app.available.as_ref().unwrap().release.fingerprint().ok();
    settle(&mut app, &ctx);
    assert_eq!(server.downloads.load(Ordering::Relaxed), 0);
    ready_data(&mut app);
    app.request_play(&ctx);
    settle(&mut app, &ctx);
    assert_eq!(server.downloads.load(Ordering::Relaxed), 1);
}

#[test]
fn failed_download_preserves_active_release_and_waits_for_explicit_retry() {
    let server = Server::new();
    let root = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut app = app(root.path(), &server.source);
    app.check(&ctx);
    settle(&mut app, &ctx);
    let active = std::fs::read(root.path().join("active.json")).unwrap();
    {
        let mut release = server.release.lock().unwrap();
        release.version = "ui-test-bad".into();
        release.files[0].sha256 = "0".repeat(64);
    }
    app.check(&ctx);
    settle(&mut app, &ctx);
    assert!(app.error.is_some() && app.blocked_update.is_some());
    assert_eq!(
        std::fs::read(root.path().join("active.json")).unwrap(),
        active
    );
    let count = server.downloads.load(Ordering::Relaxed);
    app.check(&ctx);
    settle(&mut app, &ctx);
    assert_eq!(server.downloads.load(Ordering::Relaxed), count);
}

#[test]
fn running_game_defers_installation_until_the_child_exits() {
    let server = Server::new();
    let root = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut app = app(root.path(), &server.source);
    #[cfg(windows)]
    let child = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-Command", "Start-Sleep -Seconds 30"])
        .spawn()
        .unwrap();
    #[cfg(not(windows))]
    let child = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .unwrap();
    app.child = Some(child);
    app.check(&ctx);
    settle(&mut app, &ctx);
    assert!(app.available.is_some() && app.installed.is_none());
    assert_eq!(server.downloads.load(Ordering::Relaxed), 0);
    app.child.as_mut().unwrap().kill().unwrap();
    app.child.as_mut().unwrap().wait().unwrap();
    settle(&mut app, &ctx);
    assert_eq!(server.downloads.load(Ordering::Relaxed), 1);
    assert!(app.installed.is_some());
}

#[test]
fn configured_external_data_is_imported_and_preference_values_are_preserved() {
    let server = Server::new();
    let folder = tempfile::tempdir().unwrap();
    let source = folder.path().join("Original game");
    std::fs::create_dir(&source).unwrap();
    let names = [
        "config/window_bases/main_menu.txt",
        "campaign/training/training.txt",
    ];
    let mut archive = vec![0x1a, 0xfa, 0x31, 0xc1, 0xde, 0xed, 0x42, 0x13];
    archive.extend(2u32.to_le_bytes());
    archive.extend([0; 32]);
    let mut offset = names.iter().map(|s| 20 + s.len() + 1).sum::<usize>();
    for name in names {
        for n in [name.len() as u32 + 1, 7, offset as u32, 0, 0] {
            archive.extend(n.to_le_bytes());
        }
        archive.extend(name.bytes().chain([0]).map(|b| b ^ 0xff));
        offset += 7;
    }
    archive.extend(b"fixturefixture");
    std::fs::write(source.join("core.gut"), &archive).unwrap();
    let root = folder.path().join("profile");
    let ctx = egui::Context::default();
    let mut app = app(&root, &server.source);
    app.config.data_path = source.to_string_lossy().into_owned();
    app.config.channel = UpdateChannel::Test;
    settle(&mut app, &ctx);
    let inventory = app.inventory.as_ref().unwrap();
    assert!(opensc_launcher::importer::is_managed(
        &root,
        &inventory.root
    ));
    assert_eq!(
        std::fs::read(inventory.root.join("core.gut")).unwrap(),
        archive
    );
    assert_eq!(std::fs::read(source.join("core.gut")).unwrap(), archive);
    let saved = Config::load(&root).unwrap();
    assert_eq!(saved.data_path, app.config.data_path);
    assert_eq!(saved.manifest_url, server.source);
    assert_eq!(saved.channel, UpdateChannel::Test);
    assert!(app.blocked_update.is_none());
}

#[test]
fn a_failed_play_check_can_use_the_verified_cached_release_of_the_same_channel() {
    let server = Server::new();
    let root = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut app = app(root.path(), &server.source);
    app.check(&ctx);
    settle(&mut app, &ctx);
    ready_data(&mut app);
    let active = std::fs::read(root.path().join("active.json")).unwrap();
    server.unavailable.store(true, Ordering::Relaxed);
    app.request_play(&ctx);
    settle(&mut app, &ctx);
    assert_fixture_launch(&mut app);
    assert_eq!(server.checks.load(Ordering::Relaxed), 2);
    assert_eq!(server.downloads.load(Ordering::Relaxed), 1);
    assert_eq!(
        std::fs::read(root.path().join("active.json")).unwrap(),
        active
    );
}

#[test]
fn cached_offline_play_rejects_a_different_channel_until_its_payload_is_confirmed() {
    let server = Server::new();
    let root = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut app = app(root.path(), &server.source);
    app.check(&ctx);
    settle(&mut app, &ctx);
    assert!(app.cached_channel_matches());
    app.select_channel(UpdateChannel::Test);
    assert!(!app.cached_channel_matches());
    let fingerprint = app
        .installed
        .as_ref()
        .unwrap()
        .release
        .fingerprint()
        .unwrap();
    app.config
        .confirmed_releases
        .insert(app.config.manifest_url.clone(), fingerprint);
    assert!(app.cached_channel_matches());
    app.save();
    assert_eq!(
        Config::load(root.path()).unwrap().confirmed_releases,
        app.config.confirmed_releases
    );
}

#[test]
fn repeated_play_clicks_while_a_launch_is_pending_do_not_start_extra_checks() {
    let server = Server::new();
    let root = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut app = app(root.path(), &server.source);
    ready_data(&mut app);
    app.pending_play = true;
    app.request_play(&ctx);
    assert!(app.receiver.is_none() && app.launcher_receiver.is_none());
    assert_eq!(server.checks.load(Ordering::Relaxed), 0);
}
