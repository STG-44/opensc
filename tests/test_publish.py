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


if __name__ == "__main__":
    unittest.main()
