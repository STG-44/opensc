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
    stopped: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Server {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let source = format!("http://{}/manifest.json", listener.local_addr().unwrap());
        let data = b"UI updater test; never executed";
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
                        assert!(request.to_lowercase().contains("cache-control: no-cache"));
                        serde_json::to_vec(&Manifest {
                            schema_version: 1,
                            release: Some(state.lock().unwrap().clone()),
                        })
                        .unwrap()
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
        hero: None,
        next_check: Instant::now(),
        blocked_update: None,
        download_started: None,
        reports: crate::report_ui::Form::default(),
        launcher_receiver: None,
        launcher_ready: None,
        launcher_handoff: None,
        launcher_status: String::new(),
        next_launcher_check: Instant::now() + Duration::from_secs(3600),
    }
}

fn settle(app: &mut Launcher, ctx: &egui::Context) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        app.tick(ctx);
        if !app.busy() {
            break;
        }
        assert!(Instant::now() < deadline, "{}", app.status);
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn open_launcher_downloads_and_detects_a_later_publication_without_reopening() {
    let server = Server::new();
    let root = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut app = app(root.path(), &server.source);
    settle(&mut app, &ctx);
    assert_eq!(app.installed.as_ref().unwrap().release.version, "ui-test-1");
    app.installed.as_ref().unwrap().verify(root.path()).unwrap();
    assert_eq!(server.downloads.load(Ordering::Relaxed), 1);
    app.next_check = Instant::now();
    settle(&mut app, &ctx);
    assert_eq!(
        server.downloads.load(Ordering::Relaxed),
        1,
        "unchanged manifest must not redownload"
    );
    server.release.lock().unwrap().version = "ui-test-2".into();
    app.next_check = Instant::now();
    settle(&mut app, &ctx);
    assert_eq!(app.installed.as_ref().unwrap().release.version, "ui-test-2");
    assert_eq!(server.downloads.load(Ordering::Relaxed), 2);
}

#[test]
fn disabled_autoload_and_pause_do_not_restart_downloads_on_the_next_check() {
    let server = Server::new();
    let root = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut app = app(root.path(), &server.source);
    app.config.auto_update = false;
    settle(&mut app, &ctx);
    assert!(app.available.is_some() && app.installed.is_none());
    assert_eq!(server.downloads.load(Ordering::Relaxed), 0);
    app.blocked_update = app.available.as_ref().unwrap().release.fingerprint().ok();
    app.config.auto_update = true;
    app.next_check = Instant::now();
    settle(&mut app, &ctx);
    assert_eq!(server.downloads.load(Ordering::Relaxed), 0);
    app.blocked_update = None;
    settle(&mut app, &ctx);
    assert_eq!(server.downloads.load(Ordering::Relaxed), 1);
}

#[test]
fn failed_download_preserves_active_release_and_waits_for_explicit_retry() {
    let server = Server::new();
    let root = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut app = app(root.path(), &server.source);
    settle(&mut app, &ctx);
    let active = std::fs::read(root.path().join("active.json")).unwrap();
    {
        let mut release = server.release.lock().unwrap();
        release.version = "ui-test-bad".into();
        release.files[0].sha256 = "0".repeat(64);
    }
    app.next_check = Instant::now();
    settle(&mut app, &ctx);
    assert!(app.error.is_some() && app.blocked_update.is_some());
    assert_eq!(
        std::fs::read(root.path().join("active.json")).unwrap(),
        active
    );
    let count = server.downloads.load(Ordering::Relaxed);
    app.next_check = Instant::now();
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
    settle(&mut app, &ctx);
    assert!(app.available.is_some() && app.installed.is_none());
    assert_eq!(server.downloads.load(Ordering::Relaxed), 0);
    app.child.as_mut().unwrap().kill().unwrap();
    app.child.as_mut().unwrap().wait().unwrap();
    settle(&mut app, &ctx);
    assert_eq!(server.downloads.load(Ordering::Relaxed), 1);
    assert!(app.installed.is_some());
}
