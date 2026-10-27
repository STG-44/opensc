#!/usr/bin/env python3
"""Package the native launcher and optional installers, without engine/game data."""
import argparse
import hashlib
import json
import os
import plistlib
import re
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.request
from pathlib import Path

PROJECT = Path(__file__).resolve().parents[1]
APPIMAGETOOL_URL = 'https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage'
APPIMAGETOOL_SHA256 = '95cbe7cce9717fce90c484e34052ee7c7f1d7635b33c12525b4776826a7d29b6'
DESKTOP = '''[Desktop Entry]
Type=Application
Name=OpenSC Launcher
Comment=Launcher for Open Shadow Company
Comment[ru]=Лаунчер Open Shadow Company
Exec=opensc-launcher
Icon=opensc-launcher
Categories=Game;
Terminal=false
StartupWMClass=OpenSC Launcher
'''


def run(*args, **kwargs):
    subprocess.run([str(a) for a in args], check=True, **kwargs)


def copy_executable(source, target):
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source, target)
    target.chmod(0o755)


def mac_app(binary, root, version, platform):
    app = root / 'OpenSC Launcher.app'
    contents = app / 'Contents'
    copy_executable(binary, contents / 'MacOS/opensc-launcher')
    resources = contents / 'Resources'
    resources.mkdir()
    if sys.platform == 'darwin':
        iconset = root / 'opensc.iconset'
        iconset.mkdir()
        for size in [16, 32, 128, 256, 512]:
            for scale in [1, 2]:
                name = f'icon_{size}x{size}' + ('@2x' if scale == 2 else '') + '.png'
                run('sips', '-z', size * scale, size * scale, PROJECT / 'assets/opensc-1024.png', '--out', iconset / name, stdout=subprocess.DEVNULL)
        run('iconutil', '-c', 'icns', iconset, '-o', resources / 'opensc.icns')
    info = dict(CFBundleName='OpenSC Launcher', CFBundleDisplayName='OpenSC Launcher', CFBundleExecutable='opensc-launcher', CFBundleIdentifier='ru.zaikinagroup.opensc.launcher', CFBundleIconFile='opensc.icns', CFBundlePackageType='APPL', CFBundleShortVersionString=version, CFBundleVersion=version, LSMinimumSystemVersion='11.0' if platform == 'macos-aarch64' else '10.15', NSHighResolutionCapable=True, LSApplicationCategoryType='public.app-category.games')
    with (contents / 'Info.plist').open('wb') as stream:
        plistlib.dump(info, stream)
    if sys.platform == 'darwin':
        run('codesign', '--force', '--sign', '-', app)
        run('codesign', '--verify', '--deep', '--strict', app)
    return app


def mac_packages(binary, root, output, stem, args):
    app = mac_app(binary, root, args.version, args.platform)
    shutil.copyfile(args.notices, app / 'Contents/Resources/THIRD-PARTY-NOTICES.txt')
    if sys.platform == 'darwin':
        run('codesign', '--force', '--sign', '-', app)
        run('codesign', '--verify', '--deep', '--strict', app)
    destination = output / (stem + '.zip')
    if sys.platform == 'darwin':
        run('ditto', '-c', '-k', '--sequesterRsrc', '--keepParent', app, destination)
    else:
        portable = root / 'portable'
        portable.mkdir()
        shutil.copytree(app, portable / app.name)
        shutil.make_archive(str(destination.with_suffix('')), 'zip', portable)
    generated = [destination]
    if args.installers:
        if sys.platform != 'darwin':
            raise RuntimeError('DMG creation requires macOS')
        volume = root / 'volume'
        volume.mkdir()
        shutil.copytree(app, volume / app.name)
        (volume / 'Applications').symlink_to('/Applications', target_is_directory=True)
        (volume / 'Установка.txt').write_text('Перетащите OpenSC Launcher.app в Applications.\nОткройте приложение из папки Программы.\nТестовая сборка не заверена Apple (ad-hoc подпись).\nДвижок и игровые данные устанавливаются отдельно.\n')
        dmg = output / (stem + '.dmg')
        run('hdiutil', 'create', '-volname', f'OpenSC {args.version}', '-srcfolder', volume, '-ov', '-format', 'UDZO', dmg)
        generated.append(dmg)
    return generated


def windows_packages(binary, root, output, stem, args):
    portable = root / 'portable'
    portable.mkdir()
    copy_executable(binary, portable / 'opensc-launcher.exe')
    shutil.copyfile(args.notices, portable / 'THIRD-PARTY-NOTICES.txt')
    generated = [Path(shutil.make_archive(str(output / stem), 'zip', portable))]
    if args.installers:
        if sys.platform != 'win32':
            raise RuntimeError('Setup.exe creation requires Windows and Inno Setup 6')
        compiler = shutil.which('iscc')
        if not compiler:
            candidate = Path(os.environ.get('ProgramFiles(x86)', r'C:\Program Files (x86)')) / 'Inno Setup 6/ISCC.exe'
            if candidate.is_file():
                compiler = str(candidate)
        if not compiler:
            raise RuntimeError('Inno Setup compiler ISCC.exe not found')
        run(compiler, f'/DAppVersion={args.version}', f'/DBinaryPath={binary}', f'/DIconPath={PROJECT / "assets/opensc.ico"}', f'/DNoticesPath={args.notices}', f'/DOutputPath={output}', PROJECT / 'packaging/windows/launcher.iss')
        generated.append(output / (stem + '-setup.exe'))
    return generated


def linux_tree(binary, target, notices):
    copy_executable(binary, target / 'usr/bin/opensc-launcher')
    docs = target / 'usr/share/doc/opensc-launcher'
    docs.mkdir(parents=True)
    shutil.copyfile(notices, docs / 'THIRD-PARTY-NOTICES.txt')
    icons = target / 'usr/share/icons/hicolor/256x256/apps'
    icons.mkdir(parents=True)
    shutil.copyfile(PROJECT / 'assets/opensc.png', icons / 'opensc-launcher.png')
    desktop = target / 'usr/share/applications'
    desktop.mkdir(parents=True)
    (desktop / 'opensc-launcher.desktop').write_text(DESKTOP)


def linux_packages(binary, root, output, stem, args):
    portable = root / 'portable'
    portable.mkdir()
    copy_executable(binary, portable / 'opensc-launcher')
    shutil.copyfile(args.notices, portable / 'THIRD-PARTY-NOTICES.txt')
    generated = [Path(shutil.make_archive(str(output / stem), 'gztar', portable))]
    if not args.installers:
        return generated
    if not sys.platform.startswith('linux'):
        raise RuntimeError('Linux installers require Linux packaging tools')
    deb_root = root / 'deb'
    linux_tree(binary, deb_root, args.notices)
    control = deb_root / 'DEBIAN'
    control.mkdir()
    size_kib = (sum(p.stat().st_size for p in deb_root.rglob('*') if p.is_file()) + 1023) // 1024
    (control / 'control').write_text(f'''Package: opensc-launcher
Version: {args.version}
Section: games
Priority: optional
Architecture: amd64
Maintainer: OpenSC <55913434+STG-44@users.noreply.github.com>
Homepage: https://shadow-test.zaikinagroup.ru/
Installed-Size: {size_kib}
Depends: libc6 (>= 2.39), libgcc-s1, libstdc++6, libx11-6, libxkbcommon0, libegl1, libgl1, libwayland-client0, libxcb1
Recommends: xdg-desktop-portal
Description: Native launcher for Open Shadow Company
 Select original game resources and install verified engine updates.
 The game engine and game data are not included.
''')
    deb = output / (stem + '.deb')
    run('dpkg-deb', '--build', '--root-owner-group', deb_root, deb)
    generated.append(deb)

    arch = root / 'arch'
    linux_tree(binary, arch, args.notices)
    size = sum(p.stat().st_size for p in arch.rglob('*') if p.is_file())
    metadata = f'''pkgname = opensc-launcher
pkgbase = opensc-launcher
xdata = pkgtype=pkg
pkgver = {args.version}-1
pkgdesc = Native launcher for Open Shadow Company
url = https://shadow-test.zaikinagroup.ru/
builddate = {int(time.time())}
packager = OpenSC
size = {size}
arch = x86_64
depend = glibc>=2.39
depend = gcc-libs
depend = libx11
depend = libxkbcommon
depend = libglvnd
depend = wayland
depend = libxcb
optdepend = xdg-desktop-portal: native folder picker
'''
    (arch / '.PKGINFO').write_text(metadata)
    arch_package = output / (stem + '.pkg.tar.zst')
    run('tar', '--zstd', '--owner=0', '--group=0', '--numeric-owner', '-cf', arch_package, '-C', arch, '.PKGINFO', 'usr')
    generated.append(arch_package)

    appdir = root / 'OpenSC.AppDir'
    linux_tree(binary, appdir, args.notices)
    (appdir / 'opensc-launcher.desktop').write_text(DESKTOP)
    shutil.copyfile(PROJECT / 'assets/opensc.png', appdir / 'opensc-launcher.png')
    (appdir / 'AppRun').write_text('#!/bin/sh\nappdir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)\nexec "$appdir/usr/bin/opensc-launcher" "$@"\n')
    (appdir / 'AppRun').chmod(0o755)
    tool = root / 'appimagetool.AppImage'
    with urllib.request.urlopen(APPIMAGETOOL_URL, timeout=120) as response, tool.open('wb') as writer:
        shutil.copyfileobj(response, writer)
    if hashlib.sha256(tool.read_bytes()).hexdigest() != APPIMAGETOOL_SHA256:
        raise RuntimeError('appimagetool digest changed; review the official release and update the pinned digest')
    tool.chmod(0o755)
    appimage = output / (stem + '.AppImage')
    run(tool, '--appimage-extract-and-run', appdir, appimage, env=dict(os.environ, ARCH='x86_64'))
    appimage.chmod(0o755)
    generated.append(appimage)
    return generated


def source_hashes():
    paths = [PROJECT / name for name in ['Cargo.toml', 'Cargo.lock', 'build.rs', 'src/launcher-update-key.bin',
             'crates/report-protocol/Cargo.toml', 'assets/opensc.png', 'assets/opensc.ico', 'assets/opensc-1024.png', 'deploy/site/assets/operation-map.jpg']]
    paths += list((PROJECT / 'src').rglob('*.rs'))
    paths += list((PROJECT / 'crates/report-protocol/src').rglob('*.rs'))
    return {p.relative_to(PROJECT).as_posix(): hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(paths)}


def notices(metadata, destination):
    text = ['OpenSC Launcher: third-party notices\n']
    for package in sorted(metadata['packages'], key=lambda p: (p['name'], p['version'])):
        if package['source'] is None:
            continue
        root = Path(package['manifest_path']).parent.resolve()
        text.append(f"\n{'=' * 72}\n{package['name']} {package['version']}\nLicense: {package.get('license') or 'see supplied license file'}\n")
        files = {p for p in root.iterdir() if p.is_file() and p.name.upper().startswith(('LICENSE', 'LICENCE', 'COPYING', 'NOTICE', 'COPYRIGHT'))}
        if package.get('license_file'):
            files.add(root / package['license_file'])
        for path in sorted(files):
            resolved = path.resolve()
            if not resolved.is_relative_to(root) or not resolved.is_file():
                raise ValueError(f"License file outside dependency: {package['name']}")
            text.append(f"\n--- {path.name} ---\n{resolved.read_text(encoding='utf-8')}\n")
    destination.write_text(''.join(text), encoding='utf-8')


def package(args):
    args.binary = args.binary.resolve(strict=True)
    if args.binary.stat().st_size > 128 * 1024 * 1024:
        raise ValueError('Launcher must be a stripped release binary below 128 MiB')
    args.output = args.output.resolve()
    args.output.mkdir(parents=True, exist_ok=True)
    if not re.fullmatch(r'\d+\.\d+\.\d+', args.version):
        raise ValueError('Version must have the form X.Y.Z')
    stem = f'opensc-launcher-{args.version}-{args.platform}'
    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary)
        args.notices = root / 'THIRD-PARTY-NOTICES.txt'
        notices(json.loads(args.cargo_metadata.read_text(encoding='utf-8')), args.notices)
        if args.platform.startswith('macos-'):
            paths = mac_packages(args.binary, root, args.output, stem, args)
        elif args.platform.startswith('windows-'):
            paths = windows_packages(args.binary, root, args.output, stem, args)
        else:
            paths = linux_packages(args.binary, root, args.output, stem, args)
        raw = args.output / (f'opensc-launcher-self-{args.version}-{args.platform}' + ('.exe' if args.platform.startswith('windows-') else ''))
        copy_executable(args.binary, raw)
        if sys.platform == 'darwin':
            run('codesign', '--force', '--sign', '-', raw)
            run('codesign', '--verify', '--strict', raw)
        notice = args.output / f'THIRD-PARTY-NOTICES-{args.platform}.txt'
        shutil.copyfile(args.notices, notice)
        sources = source_hashes()
        info = dict(schema_version=1, project='opensc-launcher', version=args.version, platform=args.platform,
                    source_files=sources, source_sha256=hashlib.sha256(json.dumps(sources, sort_keys=True).encode()).hexdigest(),
                    git_revision=os.environ.get('GITHUB_SHA'), self_binary=dict(file=raw.name,size=raw.stat().st_size,sha256=hashlib.sha256(raw.read_bytes()).hexdigest()),
                    notices=dict(file=notice.name,size=notice.stat().st_size,sha256=hashlib.sha256(notice.read_bytes()).hexdigest()))
        metadata = args.output / f'build-info-{args.platform}.json'
        metadata.write_text(json.dumps(info,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
        paths.extend([raw,notice,metadata])
    for path in paths:
        if not path.is_file():
            raise RuntimeError(f'Expected package was not created: {path}')
        print(path)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cargo-metadata', type=Path, required=True)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--platform', choices=['macos-x86_64', 'macos-aarch64', 'windows-x86_64', 'linux-x86_64'], required=True)
    parser.add_argument('--version', default=re.search(r'^version\s*=\s*"([^"]+)"', (PROJECT / 'Cargo.toml').read_text(), re.M).group(1))
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--installers', action='store_true')
    package(parser.parse_args())
