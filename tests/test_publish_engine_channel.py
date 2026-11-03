import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('channels', Path(__file__).resolve().parents[1] / 'scripts/publish_engine_channel.py')
channels = importlib.util.module_from_spec(spec)
spec.loader.exec_module(channels)


class Channels(unittest.TestCase):
    def test_test_publication_cannot_move_stable_or_legacy(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root / 'manifest-stable-v1.json').write_bytes(b'stable')
            (root / 'manifest-v1.json').write_bytes(b'stable')
            source = self.release(root)
            channels.promote(root, source, 'test')
            self.assertEqual((root / 'manifest-test-v1.json').read_bytes(), source.read_bytes())
            self.assertEqual((root / 'manifest-stable-v1.json').read_bytes(), b'stable')
            self.assertEqual((root / 'manifest-v1.json').read_bytes(), b'stable')

    def release(self, root):
        payload = root / 'releases/test/bin/game'
        payload.parent.mkdir(parents=True)
        payload.write_bytes(b'engine')
        source = root / 'candidate.json'
        source.write_text(json.dumps({'schema_version': 1, 'release': {'version': 'test', 'files': [
            {'url': 'releases/test/bin/game', 'size': 6, 'sha256': channels.sha(b'engine')}
        ]}}))
        return source

    def test_stable_promotion_is_guarded_and_updates_legacy(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder);source = self.release(root)
            (root / 'manifest-v1.json').write_bytes(b'old')
            with self.assertRaises(ValueError):
                channels.promote(root, source, 'stable')
            self.assertFalse((root / 'manifest-stable-v1.json').exists())
            channels.promote(root, source, 'stable', legacy_expected=channels.sha(b'old'))
            self.assertEqual((root / 'manifest-v1.json').read_bytes(), source.read_bytes())
            self.assertEqual((root / 'manifest-stable-v1.json').read_bytes(), source.read_bytes())
            with self.assertRaises(ValueError):
                channels.promote(root, source, 'stable', expected='0' * 64, legacy_expected=channels.sha(source.read_bytes()))

    def test_changed_payload_does_not_publish(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder);source = self.release(root)
            (root / 'releases/test/bin/game').write_bytes(b'broken')
            with self.assertRaises(ValueError):channels.promote(root, source, 'test')
            self.assertFalse((root / 'manifest-test-v1.json').exists())
