"""Guard against publishing originals or mixing engine architectures."""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("publish_release", Path(__file__).parents[1] / "scripts/publish_release.py")
publish = importlib.util.module_from_spec(spec)
spec.loader.exec_module(publish)


class EnginePublication(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.package = self.root / "Пакет с пробелами"
        self.package.mkdir()
        entry = "bin/x86_64-unknown-linux-gnu/shadow-game"
        contents = {
            entry: b"native binary fixture",
            "README-RU.txt": b"engine only",
            "THIRD-PARTY-NOTICES.txt": b"licenses",
            "docs/CONTROLS-RU.md": b"controls",
            "docs/HUD-RU.md": b"HUD",
            "build-info.json": json.dumps(dict(project="opensc-engine", target="x86_64-unknown-linux-gnu",
                original_data_included=False, entrypoint=entry)).encode(),
        }
        inventory = []
        for name, data in contents.items():
            path = self.package / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
            inventory.append(dict(path=name, size=len(data), sha256=hashlib.sha256(data).hexdigest(), executable=name == entry))
        (self.package / "package-files.json").write_text(json.dumps(inventory))
        self.args = argparse.Namespace(version="0.1.0-test", engine=None,
            package=[f"linux-x86_64={self.package}"], resources=None,
            notes="test", output=self.root / "updates")

    def test_manifest_keeps_original_data_external_and_copies_exact_payload(self):
        publish.build(self.args)
        manifest = json.loads((self.args.output / "manifest-v1.json").read_text())
        release = manifest["release"]
        self.assertIsNone(release["data_directory"])
        self.assertEqual(list(release["entrypoints"]), ["linux-x86_64"])
        self.assertEqual(len(release["files"]), 7)
        for item in release["files"]:
            self.assertEqual(item["platform"], "linux-x86_64")
            self.assertEqual((self.args.output / item["url"]).read_bytes(), (self.package / item["path"]).read_bytes())

    def test_rejects_architecture_mismatch(self):
        self.args.package = [f"windows-x86_64={self.package}"]
        with self.assertRaises(ValueError):
            publish.build(self.args)
        self.assertFalse(self.args.output.exists())

    def test_rejects_original_game_files_before_creating_output(self):
        (self.package / "config.gut").write_bytes(b"do not publish")
        with self.assertRaises(ValueError):
            publish.build(self.args)
        self.assertFalse(self.args.output.exists())

    def test_rejects_changed_payload_and_duplicate_platform(self):
        (self.package / "README-RU.txt").write_text("changed")
        with self.assertRaises(ValueError):
            publish.build(self.args)
        (self.package / "README-RU.txt").write_bytes(b"engine only")
        self.args.package *= 2
        with self.assertRaises(ValueError):
            publish.build(self.args)
        self.assertFalse(self.args.output.exists())

    def test_rejects_symlinks(self):
        link = self.package / "README-RU.txt"
        link.unlink()
        external = self.root / "external"
        external.write_text("outside payload")
        try:
            link.symlink_to(external)
        except OSError:
            self.skipTest("symlinks unavailable")
        with self.assertRaises(ValueError):
            publish.build(self.args)

    def add_owned_pack(self):
        prefix = 'bin/x86_64-unknown-linux-gnu/environment-pack'
        root = self.package / prefix
        model = 'models/opensc-fixture/opensc-fixture.smf'
        files = {
            'environment-pack.json': json.dumps({'version': 1, 'models': [{'path': model}]}).encode(),
            model: b'newly authored geometry fixture',
            model + '.visual.json': json.dumps({'version': 1, 'source_sha256': '0' * 64,
                'visual_sha256': hashlib.sha256(b'newly authored geometry fixture').hexdigest()}).encode(),
        }
        for name in ['bark.png', 'leaves.png', 'galvanized.png', 'masonry.png', 'concrete.png',
                     'hummer-body-v1.png', 'hummer-amber.bmp', 'hummer-glass.bmp', 'hummer-lamp.bmp',
                     'hummer-red.bmp', 'hummer-rubber.bmp', 'hummer-trim.bmp', 'hummer-wheels.bmp']:
            files['textures/opensc-environment/' + name] = b'owned texture fixture'
        for name, data in files.items():
            path = root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
        info_path = self.package / 'build-info.json'
        info = json.loads(info_path.read_text())
        info['environment_pack'] = prefix
        info_path.write_text(json.dumps(info))
        self.refresh_inventory()
        return root, model

    def refresh_inventory(self):
        files = [p for p in sorted(self.package.rglob('*')) if p.is_file() and p.name != 'package-files.json']
        inventory = [{'path': p.relative_to(self.package).as_posix(), 'size': p.stat().st_size,
                      'sha256': hashlib.sha256(p.read_bytes()).hexdigest()} for p in files]
        (self.package / 'package-files.json').write_text(json.dumps(inventory))

    def test_owned_pack_is_versioned_with_engine_and_keeps_original_data_external(self):
        self.add_owned_pack()
        publish.build(self.args)
        release = json.loads((self.args.output / 'manifest-v1.json').read_text())['release']
        self.assertIsNone(release['data_directory'])
        self.assertEqual(len(release['files']), 23)
        for item in release['files']:
            self.assertEqual((self.args.output / item['url']).read_bytes(),
                             (self.package / item['path']).read_bytes())

    def test_owned_pack_rejects_original_archive_and_changed_geometry(self):
        root, model = self.add_owned_pack()
        (root / 'config.gut').write_bytes(b'private original')
        self.refresh_inventory()
        with self.assertRaises(ValueError):
            publish.build(self.args)
        (root / 'config.gut').unlink()
        (root / model).write_bytes(b'changed geometry')
        self.refresh_inventory()
        with self.assertRaises(ValueError):
            publish.build(self.args)
        self.assertFalse(self.args.output.exists())

    def test_owned_pack_cannot_point_outside_executable_sibling(self):
        self.add_owned_pack()
        path = self.package / 'build-info.json'
        info = json.loads(path.read_text()); info['environment_pack'] = '../private'
        path.write_text(json.dumps(info)); self.refresh_inventory()
        with self.assertRaises(ValueError):
            publish.build(self.args)
        self.assertFalse(self.args.output.exists())


if __name__ == "__main__":
    unittest.main()
