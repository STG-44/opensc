#!/usr/bin/env python3
"""Promote an already uploaded engine manifest to a channel with SHA-256 guards."""
import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import tempfile


def sha(data):
    return hashlib.sha256(data).hexdigest()


def promote(root, manifest, channel, expected=None, legacy_expected=None):
    root = Path(root).resolve(strict=True)
    data = Path(manifest).read_bytes()
    release = json.loads(data)['release']
    if channel not in ('stable', 'test') or not release or not release['files']:
        raise ValueError('Select an existing engine release and stable or test channel')
    seen = set()
    for item in release['files']:
        url = item['url']
        path = PurePosixPath(url)
        if (path.is_absolute() or '..' in path.parts or '\\' in url or '%' in url
                or not path.parts or path.parts[0] != 'releases' or url in seen):
            raise ValueError('Invalid release payload path')
        seen.add(url)
        payload = root.joinpath(*path.parts)
        if payload.is_symlink() or not payload.resolve().is_relative_to(root):
            raise ValueError('Release payload leaves the public directory')
        with payload.open('rb') as stream:
            digest = hashlib.file_digest(stream, 'sha256').hexdigest()
        if payload.stat().st_size != item['size'] or digest != item['sha256']:
            raise ValueError('Release payload is missing or changed')
    pointers = [(root / f'manifest-{channel}-v1.json', expected)]
    # Older launchers continue to receive the stable release.
    if channel == 'stable':
        pointers.append((root / 'manifest-v1.json', legacy_expected))
    for path, guard in pointers:
        actual = sha(path.read_bytes()) if path.exists() else None
        if actual != guard:
            raise ValueError('Channel changed; inspect it before retrying: ' + path.name)
    for path, _ in pointers:
        with tempfile.NamedTemporaryFile(dir=root, prefix='.channel-', delete=False) as stream:
            temporary = Path(stream.name)
            try:
                stream.write(data)
                stream.flush()
                os.fsync(stream.fileno())
                stream.close()
                temporary.chmod(0o644)
                os.replace(temporary, path)
            finally:
                temporary.unlink(missing_ok=True)
    return {'channel': channel, 'version': release['version'], 'sha256': sha(data)}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--updates-root', type=Path, required=True)
    parser.add_argument('--manifest', type=Path, required=True)
    parser.add_argument('--channel', choices=['stable', 'test'], required=True)
    parser.add_argument('--expected-sha256', help='Required when the channel already exists')
    parser.add_argument('--legacy-expected-sha256', help='Required for the existing stable legacy alias')
    args = parser.parse_args()
    print(json.dumps(promote(args.updates_root, args.manifest, args.channel,
                             args.expected_sha256, args.legacy_expected_sha256)))
