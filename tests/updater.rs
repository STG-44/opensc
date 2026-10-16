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
                    stream
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    let mut request = [0u8; 4096];
                    let _ = stream.read(&mut request);
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
    let script = b"#!/bin/sh\nprintf '%s\\n' \"$1\" \"$2\" > launch-args.txt\n";
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
        format!("--data\n{}\n", data.canonicalize().unwrap().display())
    );
}
