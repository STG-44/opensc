#!/usr/bin/env python3
"""Package a prebuilt launcher without game data or an engine."""
import argparse
import plistlib
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path


def package(args):
    binary = args.binary.resolve(strict=True)
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary)
        if args.platform.startswith("macos-"):
            app = root / "OpenSC Launcher.app"
            contents = app / "Contents"
            executable = contents / "MacOS/opensc-launcher"
            executable.parent.mkdir(parents=True)
            shutil.copy2(binary, executable)
            executable.chmod(0o755)
            with (contents / "Info.plist").open("wb") as stream:
                plistlib.dump(dict(CFBundleName="OpenSC Launcher", CFBundleDisplayName="OpenSC Launcher", CFBundleExecutable="opensc-launcher", CFBundleIdentifier="ru.zaikinagroup.opensc.launcher", CFBundlePackageType="APPL", CFBundleShortVersionString=args.version, CFBundleVersion=args.version, LSMinimumSystemVersion="10.15", NSHighResolutionCapable=True), stream)
            if sys.platform == "darwin":
                subprocess.run(["codesign", "--force", "--sign", "-", str(app)], check=True)
            destination = output / f"opensc-launcher-{args.version}-{args.platform}.zip"
            if sys.platform == "darwin":
                subprocess.run(["ditto", "-c", "-k", "--sequesterRsrc", "--keepParent", str(app), str(destination)], check=True)
            else:
                shutil.make_archive(str(destination.with_suffix("")), "zip", root)
        else:
            name = "opensc-launcher.exe" if args.platform.startswith("windows-") else "opensc-launcher"
            shutil.copy2(binary, root / name)
            (root / name).chmod(0o755)
            extension = "zip" if args.platform.startswith("windows-") else "gztar"
            destination = Path(shutil.make_archive(str(output / f"opensc-launcher-{args.version}-{args.platform}"), extension, root))
    print(destination)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--platform", required=True)
    parser.add_argument("--version", default="0.1.0")
    parser.add_argument("--output", type=Path, required=True)
    package(parser.parse_args())
