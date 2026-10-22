use shadow_formats::{AssetCatalog, AssetOrigin, GutArchive, normalize_path};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Sandbox(PathBuf);
impl Sandbox {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "shadow-formats-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn write(&self, path: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();
        path
    }
}
impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

// Маленький самостоятельный образец формата; оригинальные ресурсы в тесты не входят.
fn fixture(name: &str, payload: &[u8]) -> Vec<u8> {
    let mut bytes = b"test archive\r\n".to_vec();
    bytes.extend([0x1a, 0xfa, 0x31, 0xc1, 0xde, 0xed, 0x42, 0x13]);
    bytes.extend(1_u32.to_le_bytes());
    let mut archive_name = [0_u8; 32];
    archive_name[..8].copy_from_slice(b"test.gut");
    bytes.extend(archive_name);
    let mut encoded_name = name.as_bytes().to_vec();
    encoded_name.push(0);
    for value in [
        encoded_name.len() as u32,
        payload.len() as u32,
        20 + encoded_name.len() as u32,
        1,
        0x12345678,
    ] {
        bytes.extend(value.to_le_bytes());
    }
    bytes.extend(encoded_name.iter().map(|byte| byte ^ 0xff));
    bytes.extend(payload.iter().map(|byte| byte ^ 0xff));
    bytes
}

#[test]
fn decodes_xor_and_windows_paths_without_changing_archive() {
    let sandbox = Sandbox::new();
    let original = fixture("Textures\\Hello.TXT", b"known original bytes");
    let path = sandbox.write("test.gut", &original);
    let archive = GutArchive::open(&path).unwrap();
    assert_eq!(
        archive.read("textures/HELLO.txt").unwrap(),
        b"known original bytes"
    );
    assert_eq!(archive.entries[0].stored_checksum, 0x12345678);
    assert_eq!(fs::read(path).unwrap(), original);
}

#[test]
fn rejects_truncated_archive_and_offsets_outside_file() {
    let sandbox = Sandbox::new();
    let mut bytes = fixture("textures/a.txt", b"payload");
    let offset_field = b"test archive\r\n".len() + 8 + 4 + 32 + 8;
    bytes[offset_field..offset_field + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(GutArchive::open(sandbox.write("bad-offset.gut", &bytes)).is_err());
    let mut truncated = fixture("textures/a.txt", b"payload");
    truncated.truncate(truncated.len() - 2);
    assert!(GutArchive::open(sandbox.write("truncated.gut", &truncated)).is_err());
}

#[test]
fn rejects_paths_that_escape_the_asset_root() {
    for bad in [
        "../secret",
        "textures/../../secret",
        "C:\\secret",
        "/etc/passwd",
        "\\\\server\\share",
        "",
    ] {
        assert!(normalize_path(bad).is_err(), "accepted {bad:?}");
    }
    let sandbox = Sandbox::new();
    assert!(
        GutArchive::open(sandbox.write("escape.gut", &fixture("../outside", b"data"))).is_err()
    );
}

#[test]
fn loose_files_and_our_overrides_have_the_documented_priority() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "Shadow Company — тест/Data/test.gut",
        &fixture("Textures\\Hello.TXT", b"archive"),
    );
    sandbox.write("Shadow Company — тест/Data/TEXTURES/HELLO.txt", b"loose");
    sandbox.write("Наши ресурсы/textures/hello.txt", b"improved");
    let game = sandbox.0.join("Shadow Company — тест");
    let overlay = sandbox.0.join("Наши ресурсы");
    let base = AssetCatalog::open(&game, None).unwrap();
    assert_eq!(base.read("textures/hello.txt").unwrap(), b"loose");
    let improved = AssetCatalog::open(&game, Some(&overlay)).unwrap();
    assert_eq!(improved.read("TEXTURES\\HELLO.TXT").unwrap(), b"improved");
    assert!(matches!(
        improved.origin("textures/hello.txt").unwrap(),
        AssetOrigin::Override(_)
    ));
    assert_eq!(
        improved
            .resolve_texture("models/unit.smf", "Hello.TXT")
            .unwrap(),
        Some("textures/hello.txt".into())
    );
    assert_eq!(
        fs::read(game.join("Data/TEXTURES/HELLO.txt")).unwrap(),
        b"loose"
    );
}

#[test]
fn rejects_ambiguous_case_collisions_on_case_sensitive_filesystems() {
    let sandbox = Sandbox::new();
    sandbox.write("game/Data/test.gut", &fixture("a.txt", b"archive"));
    sandbox.write("overrides/Textures/hello.txt", b"first");
    sandbox.write("overrides/textures/hello.txt", b"second");
    // Файловая система Windows или macOS может сама объединить эти пути.
    if sandbox.0.join("overrides").read_dir().unwrap().count() == 2 {
        assert!(
            AssetCatalog::open(sandbox.0.join("game"), Some(&sandbox.0.join("overrides"))).is_err()
        );
    }
}
