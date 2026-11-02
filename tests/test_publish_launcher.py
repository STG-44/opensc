import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import shutil
import struct
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('publish_launcher', ROOT / 'scripts/publish_launcher.py')
publisher = importlib.util.module_from_spec(spec)
spec.loader.exec_module(publisher)

@unittest.skipUnless(sys.platform.startswith('linux') and shutil.which('openssl'), 'Releases are signed on Linux')
class LauncherPublication(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.project = self.root / 'project'
        (self.project / 'src').mkdir(parents=True)
        (self.project / 'engine/scripts').mkdir(parents=True)
        shutil.copyfile(ROOT / 'engine/scripts/package-game.py', self.project / 'engine/scripts/package-game.py')
        self.key = self.root / 'signing.pem'
        subprocess.run(['openssl','genpkey','-algorithm','ED25519','-out',str(self.key)],check=True,capture_output=True)
        self.key.chmod(0o600)
        public = subprocess.check_output(['openssl','pkey','-in',str(self.key),'-pubout','-outform','DER'])
        (self.project / 'src/launcher-update-key.bin').write_bytes(public[-32:])
        self.expected = self.root / 'expected.json'
        sources = {'src/main.rs': '0' * 64}
        expected = dict(source_files=sources,source_sha256=hashlib.sha256(json.dumps(sources,sort_keys=True).encode()).hexdigest())
        self.expected.write_text(json.dumps(expected))
        self.packages = []
        for platform in publisher.TARGETS:
            directory = self.root / platform
            directory.mkdir()
            binary = bytearray(80)
            if platform.startswith('linux-'):
                binary[:6] = b'\x7fELF\x02\x01'; struct.pack_into('<H',binary,18,62)
            elif platform.startswith('windows-'):
                binary[:2] = b'MZ'; struct.pack_into('<I',binary,60,64)
                binary[64:68] = b'PE\0\0'; struct.pack_into('<H',binary,68,0x8664)
            else:
                binary[:4] = b'\xcf\xfa\xed\xfe'
                struct.pack_into('<I',binary,4,0x0100000c if platform.endswith('aarch64') else 0x01000007)
            (directory / 'launcher').write_bytes(binary)
            (directory / 'notices.txt').write_text('Third-party license fixture')
            def record(name):
                file = directory / name
                return dict(file=name,size=file.stat().st_size,sha256=hashlib.sha256(file.read_bytes()).hexdigest())
            info = dict(project='opensc-launcher',platform=platform,version='0.4.0',**expected,
                        self_binary=record('launcher'),notices=record('notices.txt'))
            (directory / f'build-info-{platform}.json').write_text(json.dumps(info))
            self.packages.append(platform + '=' + str(directory))

    def args(self, name='site'):
        return argparse.Namespace(version='0.4.0',package=self.packages,expected_source=self.expected,
                                  signing_key=self.key,output=self.root / name,notes='Test release')

    def test_prepares_all_platforms_and_signature_covers_exact_payload_bytes(self):
        args = self.args()
        with patch.object(publisher,'PROJECT',self.project):
            publisher.prepare(args)
        index = json.loads((args.output / 'launcher-v1.json').read_text())
        payload = json.loads(index['payload'])
        self.assertEqual(len(payload['release']['files']),8)
        self.assertEqual(len(payload['release']['entrypoints']),4)
        public, signature, message = self.root/'public.pem',self.root/'signature',self.root/'message'
        subprocess.run(['openssl','pkey','-in',str(self.key),'-pubout','-out',str(public)],check=True,capture_output=True)
        signature.write_bytes(bytes.fromhex(index['signature']))
        message.write_bytes(publisher.DOMAIN + index['payload'].encode())
        command=['openssl','pkeyutl','-verify','-rawin','-pubin','-inkey',str(public),'-in',str(message),'-sigfile',str(signature)]
        self.assertEqual(subprocess.run(command,capture_output=True).returncode,0)
        message.write_bytes(message.read_bytes().replace(b'0.4.0',b'0.4.1'))
        self.assertNotEqual(subprocess.run(command,capture_output=True).returncode,0)

    def test_changed_artifact_or_missing_platform_never_creates_a_public_index(self):
        (self.root/'linux-x86_64/launcher').write_bytes(b'changed binary')
        with patch.object(publisher,'PROJECT',self.project):
            with self.assertRaises(ValueError): publisher.prepare(self.args())
            args=self.args('missing');args.package=self.packages[1:]
            with self.assertRaises(ValueError): publisher.prepare(args)
        self.assertFalse((self.root/'site/launcher-v1.json').exists())
