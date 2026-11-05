"""Validate the bounded, owned environment pack before shipping it."""
import hashlib
import json
from pathlib import Path
import re


def owned_pack_files(root):
    root = Path(root)
    manifest = json.loads((root / 'environment-pack.json').read_text(encoding='utf-8'))
    if manifest.get('version') != 1 or not 1 <= len(manifest.get('models', [])) <= 128:
        raise ValueError('Invalid owned environment pack')
    allowed = {'environment-pack.json'}
    for model in manifest['models']:
        path = model['path']
        if not re.fullmatch(r'models/([a-z0-9_-]+)/\1\.smf', path) or path in allowed:
            raise ValueError('Invalid or duplicate owned model path')
        visual = root / path
        sidecar = root / (path + '.visual.json')
        metadata = json.loads(sidecar.read_text(encoding='utf-8'))
        actual = hashlib.sha256(visual.read_bytes()).hexdigest()
        if (metadata.get('version') not in (1, 2)
                or metadata.get('visual_sha256') != actual
                or not re.fullmatch(r'[0-9a-f]{64}', metadata.get('source_sha256', ''))
                or metadata['source_sha256'] == actual):
            raise ValueError('Changed replacement mesh or original model in owned pack')
        allowed.update((path, path + '.visual.json'))
    textures = {
        'bark.png', 'leaves.png', 'galvanized.png', 'masonry.png', 'concrete.png',
        'hummer-body-v1.png', 'hummer-amber.bmp', 'hummer-glass.bmp',
        'hummer-lamp.bmp', 'hummer-red.bmp', 'hummer-rubber.bmp',
        'hummer-trim.bmp', 'hummer-wheels.bmp',
    }
    allowed.update('textures/opensc-environment/' + name for name in textures)
    found = set()
    for path in root.rglob('*'):
        if path.is_symlink():
            raise ValueError('Symlink in owned environment pack')
        if path.is_file():
            found.add(path.relative_to(root).as_posix())
            if not 0 < path.stat().st_size <= 32 * 1024 * 1024:
                raise ValueError('Owned resource exceeds size bounds')
    if found != allowed:
        raise ValueError('Unexpected or missing files in owned environment pack')
    return sorted(allowed)
