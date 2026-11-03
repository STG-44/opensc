#!/usr/bin/env python3
"""Rebuild renamed, static Noto UI font subsets from pinned upstream fonts.

Requires fonttools. The runtime is fully offline; this is an asset-authoring tool.
"""
from pathlib import Path
import argparse
import hashlib
import json
import gzip
import io
import fontTools
import unicodedata
from fontTools import subset
from fontTools.ttLib import TTFont
from fontTools.varLib.instancer import instantiateVariableFont

PROJECT = Path(__file__).resolve().parents[1]
ASSETS = PROJECT / 'engine/crates/opensc-localization/assets'


def build(upstream):
    upstream = Path(upstream)
    records = json.loads((upstream / 'sources.json').read_text())
    text = ''.join((ASSETS / 'locales' / (code + '.json')).read_text() for code in
                   ['en', 'uk', 'de', 'fr', 'es', 'it', 'pt-BR', 'pl', 'cs', 'tr', 'zh-Hans', 'zh-Hant', 'ja', 'ko'])
    text += 'РусскийEnglishУкраїнськаDeutschFrançaisEspañolItalianoPortuguêsBrasilPolskiČeštinaTürkçe简体中文繁體中文日本語한국어'
    # Retain Latin/Cyrillic text and punctuation alongside translated CJK glyphs.
    points = {ord(c) for c in text if unicodedata.category(c)[0] != 'C'} | set(range(0x20, 0x530)) | set(range(0x2000, 0x2070))
    destination = ASSETS / 'fonts'; destination.mkdir(exist_ok=True)
    output = []
    for tag in ['SC', 'TC', 'JP', 'KR']:
        name = 'NotoSans' + tag + '-VF.ttf'
        source = upstream / name
        expected = next(record for record in records['fonts'] if record['file'] == name)
        with source.open('rb') as stream:
            assert hashlib.file_digest(stream, 'sha256').hexdigest() == expected['sha256']
        font = TTFont(source, recalcTimestamp=False)
        options = subset.Options();options.recalc_timestamp=False;options.notdef_outline=True
        sub = subset.Subsetter(options=options);sub.populate(unicodes=points | set(font.getBestCmap()));sub.subset(font)
        font = instantiateVariableFont(font, {'wght': 400}, inplace=True)
        family = 'OpenSC UI ' + tag
        for record in font['name'].names:
            if record.nameID in [1, 4, 6, 16, 17]:
                value = family.replace(' ', '') if record.nameID == 6 else ('Regular' if record.nameID == 17 else family)
                record.string = value.encode(record.getEncoding())
        path = destination / ('OpenSC-UI-' + tag + '.ttf.gz')
        buffer = io.BytesIO();font.save(buffer);raw = buffer.getvalue()
        data = gzip.compress(raw, compresslevel=9, mtime=0);path.write_bytes(data)
        output.append({'file': path.name, 'source': expected, 'sha256': hashlib.sha256(data).hexdigest(),
                       'bytes': len(data), 'uncompressed_sha256': hashlib.sha256(raw).hexdigest(), 'uncompressed_bytes':len(raw), 'family': family, 'weight': 400, 'purpose': 'Bundled UI glyph subset'})
        print('Built '+path.name+' ('+str(len(data))+' bytes)',flush=True)
    (destination / 'OFL.txt').write_bytes((upstream / 'OFL.txt').read_bytes())
    (destination / 'SOURCES.json').write_text(json.dumps({'fonttools_version': fontTools.__version__, 'upstream_commit': records['commit'], 'license_url': records['license_url'], 'fonts': output}, indent=2)+'\n')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--upstream-directory', type=Path, required=True)
    build(parser.parse_args().upstream_directory)
