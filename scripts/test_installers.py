#!/usr/bin/env python3
"""Check built installers in temporary locations; never use a real game folder."""
import argparse
import os
from pathlib import Path
import plistlib
import re
import subprocess
import tempfile
import time

PROJECT = Path(__file__).resolve().parents[1]


def run(*args, **kwargs):
    return subprocess.run([str(a) for a in args], check=True, **kwargs)


def check_help(binary, version, root):
    env = dict(os.environ, OPENSC_HOME=str(root))
    result = run(binary, '--help', env=env, capture_output=True, text=True, encoding='utf-8')
    assert f'OpenSC Launcher {version}' in result.stdout, result.stdout


def check_mac(directory, stem, version, root):
    result = run('hdiutil', 'attach', '-readonly', '-nobrowse', '-plist', directory / (stem + '.dmg'), capture_output=True)
    info = plistlib.loads(result.stdout)
    mount = next(e['mount-point'] for e in info['system-entities'] if 'mount-point' in e)
    try:
        volume = Path(mount)
        app = volume / 'OpenSC Launcher.app'
        run('codesign', '--verify', '--deep', '--strict', app)
        assert (volume / 'Applications').is_symlink()
        assert (volume / 'Applications').readlink() == Path('/Applications')
        with (app / 'Contents/Info.plist').open('rb') as stream:
            assert plistlib.load(stream)['CFBundleShortVersionString'] == version
        assert (app / 'Contents/Resources/opensc.icns').is_file()
        check_help(app / 'Contents/MacOS/opensc-launcher', version, root / 'profile')
    finally:
        run('hdiutil', 'detach', mount)


def check_windows(directory, stem, version, root):
    install = root / 'installed'
    profile = root / 'profile'
    profile.mkdir()
    sentinel = profile / 'keep-user-data.txt'
    sentinel.write_text('User configuration must survive uninstallation.')
    run(directory / (stem + '-setup.exe'), '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/SP-', f'/DIR={install}')
    binary = install / 'opensc-launcher.exe'
    assert binary.is_file()
    check_help(binary, version, profile)
    run(install / 'unins000.exe', '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART')
    deadline = time.monotonic() + 15
    while binary.exists() and time.monotonic() < deadline:
        time.sleep(0.2)
    assert not binary.exists(), 'Uninstaller did not remove the application'
    assert sentinel.read_text() == 'User configuration must survive uninstallation.'


def check_linux(directory, stem, version, root):
    deb = directory / (stem + '.deb')
    result = run('dpkg-deb', '--field', deb, 'Version', capture_output=True, text=True)
    assert result.stdout.strip() == version
    extracted = root / 'deb'
    run('dpkg-deb', '--extract', deb, extracted)
    binary = extracted / 'usr/bin/opensc-launcher'
    check_help(binary, version, root / 'deb-profile')
    desktop = extracted / 'usr/share/applications/opensc-launcher.desktop'
    assert 'Exec=opensc-launcher' in desktop.read_text()
    assert (extracted / 'usr/share/icons/hicolor/256x256/apps/opensc-launcher.png').is_file()
    result = run('tar', '--zstd', '-xOf', directory / (stem + '.pkg.tar.zst'), '.PKGINFO', capture_output=True, text=True)
    assert f'pkgver = {version}-1' in result.stdout
    assert 'depend = glibc>=2.39' in result.stdout
    check_help_appimage = run(directory / (stem + '.AppImage'), '--appimage-extract-and-run', '--help', env=dict(os.environ, OPENSC_HOME=str(root / 'appimage-profile')), capture_output=True, text=True, encoding='utf-8')
    assert f'OpenSC Launcher {version}' in check_help_appimage.stdout


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--directory', type=Path, required=True)
    parser.add_argument('--platform', required=True, choices=['macos-x86_64', 'macos-aarch64', 'windows-x86_64', 'linux-x86_64'])
    parser.add_argument('--version', default=re.search(r'^version\s*=\s*"([^"]+)"', (PROJECT / 'Cargo.toml').read_text(), re.M).group(1))
    args = parser.parse_args()
    stem = f'opensc-launcher-{args.version}-{args.platform}'
    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary)
        directory = args.directory.resolve(strict=True)
        if args.platform.startswith('macos-'):
            check_mac(directory, stem, args.version, root)
        elif args.platform.startswith('windows-'):
            check_windows(directory, stem, args.version, root)
        else:
            check_linux(directory, stem, args.version, root)
    print(f'Installer checks passed: {args.platform}')
