#!/usr/bin/env python3
"""Build a static update directory; publish manifest-v1.json last on the server."""
import argparse
import hashlib
import json
import re
from pathlib import Path
from urllib.parse import quote

TARGETS = {
    "linux-x86_64": "x86_64-unknown-linux-gnu",
    "linux-aarch64": "aarch64-unknown-linux-gnu",
    "windows-x86_64": "x86_64-pc-windows-msvc",
    "macos-x86_64": "x86_64-apple-darwin",
    "macos-aarch64": "aarch64-apple-darwin",
}


def read_package(spec):
    platform, separator, directory = spec.partition("=")
    if not separator or platform not in TARGETS:
        raise ValueError("Use --package PLATFORM=/path/to/staged/package")
    root = Path(directory).resolve(strict=True)
    info = json.loads((root / "build-info.json").read_text(encoding="utf-8"))
    if (info.get("project") != "opensc-engine" or info.get("target") != TARGETS[platform]
            or info.get("original_data_included") is not False):
        raise ValueError("Package does not match the selected engine platform")
    entrypoint = "bin/" + TARGETS[platform] + "/shadow-game" + (".exe" if platform.startswith("windows-") else "")
    if info.get("entrypoint") != entrypoint:
        raise ValueError("Unexpected engine entrypoint")
    allowed = {entrypoint, "build-info.json", "README-RU.txt", "THIRD-PARTY-NOTICES.txt",
               "docs/CONTROLS-RU.md", "docs/HUD-RU.md", "package-files.json"}
    found = set()
    for path in root.rglob("*"):
        if path.is_symlink():
            raise ValueError(f"Symlinks are not allowed: {path}")
        if path.is_file():
            found.add(path.relative_to(root).as_posix())
    if found != allowed:
        raise ValueError("Unexpected or missing files in engine package")
    inventory = json.loads((root / "package-files.json").read_text(encoding="utf-8"))
    if {f["path"] for f in inventory} != allowed - {"package-files.json"} or len(inventory) != len(allowed) - 1:
        raise ValueError("Invalid package inventory")
    for file in inventory:
        source = root / file["path"]
        with source.open("rb") as stream:
            actual = hashlib.file_digest(stream, "sha256").hexdigest()
        if source.stat().st_size != file["size"] or actual != file["sha256"]:
            raise ValueError(f"Changed package file: {file['path']}")
    return platform, root, entrypoint, sorted(allowed)


def build(args):
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]{0,63}", args.version):
        raise ValueError("Invalid version")
    output = args.output.resolve()
    if output.exists():
        raise ValueError("Output directory must not exist; use a new staging directory")
    platforms = {}
    packages = [read_package(spec) for spec in (args.package or [])]
    if packages and args.resources:
        raise ValueError("Bevy packages use the player's original installation; select overrides in the launcher")
    for spec in args.engine or []:
        platform, separator, filename = spec.partition("=")
        if not separator or platform not in {"macos-x86_64", "macos-aarch64", "windows-x86_64", "linux-x86_64", "linux-aarch64"}:
            raise ValueError("Use --engine PLATFORM=/path/to/engine")
        if platform in platforms:
            raise ValueError("Duplicate platform")
        path = Path(filename).resolve(strict=True)
        if not path.is_file():
            raise ValueError("Engine must be a file")
        platforms[platform] = path
    package_platforms = [p[0] for p in packages]
    if len(package_platforms) != len(set(package_platforms)) or set(package_platforms) & platforms.keys():
        raise ValueError("Duplicate engine platform")
    if not platforms and not packages:
        raise ValueError("Specify at least one --engine or --package")
    if args.resources:
        resources = args.resources.resolve(strict=True)
        if not resources.is_dir() or output == resources or resources in output.parents:
            raise ValueError("Resource directory and output must be separate")
    else:
        resources = None
    files = []
    entrypoints = {}
    output.mkdir(parents=True)

    def copy(source, relative, platform=None, executable=False):
        download = f"releases/{args.version}/{platform or 'shared'}/{relative}"
        destination = output / download
        destination.parent.mkdir(parents=True, exist_ok=True)
        digest = hashlib.sha256()
        size = 0
        with source.open("rb") as src, destination.open("xb") as dst:
            while chunk := src.read(1024 * 1024):
                dst.write(chunk)
                digest.update(chunk)
                size += len(chunk)
        files.append(dict(path=relative, url=quote(download, safe="/"), size=size, sha256=digest.hexdigest(), executable=executable, platform=platform))

    for platform, source in sorted(platforms.items()):
        name = "bin/opensc-engine.exe" if platform.startswith("windows-") else "bin/opensc-engine"
        copy(source, name, platform, True)
        entrypoints[platform] = name
    for platform, root, entrypoint, paths in packages:
        for path in paths:
            copy(root / path, path, platform, path == entrypoint)
        entrypoints[platform] = entrypoint
    if resources:
        for source in sorted(resources.rglob("*")):
            if source.is_symlink():
                raise ValueError(f"Symlinks are not allowed: {source}")
            if source.is_file() and source.name != ".DS_Store":
                copy(source, "data/" + source.relative_to(resources).as_posix())
    manifest = dict(schema_version=1, release=dict(version=args.version, notes=args.notes, data_directory="data" if resources else None, entrypoints=entrypoints, files=files))
    (output / "manifest-v1.json").write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"Prepared {len(files)} files in {output}")
    print("Copy the releases/ directory into the existing /updates/ document root first.")
    print("Replace /updates/manifest-v1.json atomically after all files have arrived.")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", required=True)
    parser.add_argument("--engine", action="append", help="Legacy single-file engine: PLATFORM=/path/to/engine")
    parser.add_argument("--package", action="append", help="PLATFORM=/path/to/staged/engine; repeat for several platforms")
    parser.add_argument("--resources", type=Path, help="Legacy data package; do not publish original game assets")
    parser.add_argument("--notes", default="Тестовая сборка")
    parser.add_argument("--output", type=Path, required=True)
    build(parser.parse_args())
