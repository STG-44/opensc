#!/usr/bin/env python3
"""Prepare immutable launcher payloads and an Ed25519-signed update index."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile

PROJECT = Path(__file__).resolve().parents[1]
DOMAIN = b'OPENSC-LAUNCHER-UPDATE-V1\n'
TARGETS = {'linux-x86_64': 'x86_64-unknown-linux-gnu',
           'windows-x86_64': 'x86_64-pc-windows-msvc',
           'macos-x86_64': 'x86_64-apple-darwin',
           'macos-aarch64': 'aarch64-apple-darwin'}

def sha(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()

def prepare(args):
    if not re.fullmatch(r'\d+\.\d+\.\d+', args.version):
        raise ValueError('Launcher version must be major.minor.patch')
    expected = json.loads(args.expected_source.read_text(encoding='utf-8'))
    key = args.signing_key.resolve(strict=True)
    if os.name == 'posix' and key.stat().st_mode & 0o077:
        raise ValueError('Private signing key must have permissions 0600')
    if key.is_relative_to(PROJECT):
        raise ValueError('Keep the signing key outside the repository')
    public = subprocess.check_output(['openssl', 'pkey', '-in', str(key), '-pubout', '-outform', 'DER'])
    if len(public) != 44 or public[-32:] != (PROJECT / 'src/launcher-update-key.bin').read_bytes():
        raise ValueError('Signing key does not match the embedded public key')
    spec = importlib.util.spec_from_file_location('engine_package', PROJECT / 'engine/scripts/package-game.py')
    validator = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(validator)
    packages = {}
    for item in args.package:
        platform, separator, directory = item.partition('=')
        if not separator or platform not in TARGETS or platform in packages:
            raise ValueError('Use one --package PLATFORM=/CI/package per platform')
        root = Path(directory).resolve(strict=True)
        info = json.loads((root / f'build-info-{platform}.json').read_text(encoding='utf-8'))
        if info['project'] != 'opensc-launcher' or info['platform'] != platform or info['version'] != args.version:
            raise ValueError('Wrong launcher build metadata')
        if info['source_files'] != expected['source_files'] or info['source_sha256'] != expected['source_sha256']:
            raise ValueError('Launcher source differs from the approved checkout')
        for field, maximum in [('self_binary', 128 * 1024 * 1024), ('notices', 8 * 1024 * 1024)]:
            file = info[field]
            if Path(file['file']).name != file['file']:
                raise ValueError('Invalid artifact filename')
            source = root / file['file']
            if source.is_symlink() or not source.is_file() or source.stat().st_size > maximum:
                raise ValueError('Invalid launcher payload')
            if source.stat().st_size != file['size'] or sha(source) != file['sha256']:
                raise ValueError('Launcher payload digest mismatch')
        validator.validate_binary(root / info['self_binary']['file'], TARGETS[platform])
        packages[platform] = (root, info)
    if set(packages) != set(TARGETS):
        raise ValueError('All four launcher platforms are required')
    output = args.output.resolve()
    if output.exists():
        raise ValueError('Use a new unpublished output directory')
    output.mkdir(parents=True)
    files, entrypoints = [], {}
    for platform, (root, info) in sorted(packages.items()):
        entrypoint = 'bin/opensc-launcher' + ('.exe' if platform.startswith('windows-') else '')
        entrypoints[platform] = entrypoint
        for field, path in [('self_binary', entrypoint), ('notices', 'THIRD-PARTY-NOTICES.txt')]:
            record = info[field]
            url = f'launcher-releases/{args.version}/{platform}/{path}'
            destination = output / url
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(root / record['file'], destination)
            files.append(dict(path=path, url=url, size=record['size'], sha256=record['sha256'],
                              executable=field == 'self_binary', platform=platform))
    manifest = dict(schema_version=1, release=dict(version=args.version, notes=args.notes,
                    data_directory=None, entrypoints=entrypoints, files=files))
    payload = json.dumps(manifest, ensure_ascii=False, indent=2) + '\n'
    with tempfile.TemporaryDirectory(prefix='opensc-sign-') as temporary:
        message, signature = Path(temporary) / 'message', Path(temporary) / 'signature'
        message.write_bytes(DOMAIN + payload.encode())
        subprocess.run(['openssl', 'pkeyutl', '-sign', '-rawin', '-inkey', str(key),
                        '-in', str(message), '-out', str(signature)], check=True)
        signed = dict(schema_version=1, payload=payload, signature=signature.read_bytes().hex())
    (output / 'launcher-v1.json').write_text(json.dumps(signed, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
    print(json.dumps({'version': args.version, 'platforms': sorted(packages), 'files': len(files),
                      'index_sha256': sha(output / 'launcher-v1.json'), 'source_sha256': expected['source_sha256']}))

if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--version', required=True)
    parser.add_argument('--package', action='append', required=True)
    parser.add_argument('--expected-source', type=Path, required=True)
    parser.add_argument('--signing-key', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--notes', default='Обновление лаунчера OpenSC')
    prepare(parser.parse_args())
