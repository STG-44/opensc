#!/usr/bin/env python3
"""Stage a built engine for OpenSC; never include the player's original assets.

This creates payload files and build provenance, not an updater manifest. The
launcher repository owns the update protocol and publication step.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import struct
import subprocess
import tempfile

PROJECT = Path(__file__).resolve().parents[1]
TARGETS = {
    "x86_64-unknown-linux-gnu": ("elf", 62),
    "aarch64-unknown-linux-gnu": ("elf", 183),
    "x86_64-pc-windows-msvc": ("pe", 0x8664),
    "x86_64-apple-darwin": ("macho", 0x01000007),
    "aarch64-apple-darwin": ("macho", 0x0100000C),
}


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def validate_binary(path, target):
    """Reject accidentally publishing a binary for another OS/architecture."""
    kind, machine = TARGETS[target]
    with path.open("rb") as stream:
        head = stream.read(64)
        if kind == "elf":
            valid = (len(head) == 64 and head[:6] == b"\x7fELF\x02\x01"
                     and struct.unpack_from("<H", head, 18)[0] == machine)
        elif kind == "macho":
            valid = (len(head) >= 8 and head[:4] == b"\xcf\xfa\xed\xfe"
                     and struct.unpack_from("<I", head, 4)[0] == machine)
        else:
            valid = False
            if len(head) == 64 and head[:2] == b"MZ":
                offset = struct.unpack_from("<I", head, 60)[0]
                if 64 <= offset <= 1024 * 1024:
                    stream.seek(offset)
                    pe = stream.read(6)
                    valid = (len(pe) == 6 and pe[:4] == b"PE\0\0"
                             and struct.unpack_from("<H", pe, 4)[0] == machine)
        if not valid:
            raise ValueError(f"binary does not match {target}")


def source_hashes():
    paths = [PROJECT / name for name in ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml")]
    paths += list((PROJECT / "crates").rglob("*.rs"))
    paths += list((PROJECT / "crates").rglob("Cargo.toml"))
    paths += list((PROJECT / "crates").rglob("*.ttf"))
    paths += list((PROJECT / "crates/shadow-model-viewer/assets/locales").glob("*.json"))
    return {p.relative_to(PROJECT).as_posix(): digest(p) for p in sorted(paths)}


def notices(metadata, destination):
    """Carry dependency declarations and their supplied license/notice files."""
    text = ["OpenSC engine: third-party notices\n",
            "Dependency metadata for the selected native target.\n"]
    for package in sorted(metadata["packages"], key=lambda p: (p["name"], p["version"])):
        if package["source"] is None:
            continue
        root = Path(package["manifest_path"]).parent
        text.append(f"\n{'=' * 72}\n{package['name']} {package['version']}\n")
        text.append(f"License: {package.get('license') or 'see supplied license file'}\n")
        if package.get("repository"):
            text.append(f"Source: {package['repository']}\n")
        files = {p for p in root.iterdir() if p.is_file()
                 and p.name.upper().startswith(("LICENSE", "LICENCE", "COPYING", "NOTICE", "COPYRIGHT"))}
        if package.get("license_file"):
            files.add(root / package["license_file"])
        for path in sorted(files):
            # Metadata may name a license outside the crate. Do not package
            # arbitrary files from the build computer.
            resolved = path.resolve()
            if not resolved.is_relative_to(root.resolve()) or not resolved.is_file():
                raise ValueError(f"license unavailable inside crate: {package['name']}")
            text.append(f"\n--- {path.name} ---\n{resolved.read_text(encoding='utf-8')}\n")
    font = PROJECT / "crates/shadow-viewer/assets/fonts/LICENSE-DejaVu.txt"
    text.append(f"\n{'=' * 72}\nEmbedded DejaVu Sans\n{font.read_text(encoding='utf-8')}")
    font = PROJECT / "crates/shadow-viewer/assets/fonts/LICENSE-RussoOne.txt"
    text.append(f"\n{'=' * 72}\nEmbedded Russo One\n{font.read_text(encoding='utf-8')}")
    destination.write_text("".join(text), encoding="utf-8")


def glibc_requirement(binary):
    result = subprocess.run(["readelf", "--version-info", str(binary)],
                            check=True, capture_output=True, text=True)
    versions = re.findall(r"\bGLIBC_(\d+(?:\.\d+)+)\b", result.stdout)
    return max(versions, key=lambda v: tuple(map(int, v.split(".")))) if versions else None


def stage(args):
    binary = args.binary.resolve(strict=True)
    validate_binary(binary, args.target)
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]{0,95}", args.build_id):
        raise ValueError("build-id must be a simple version label")
    metadata = json.loads(args.cargo_metadata.read_text(encoding="utf-8"))
    package = next(p for p in metadata["packages"] if p["name"] == "shadow-model-viewer")
    output = args.output.absolute()
    output.parent.mkdir(parents=True, exist_ok=True)
    if output.exists():
        raise FileExistsError(f"refusing to overwrite {output}")
    with tempfile.TemporaryDirectory(prefix=".opensc-stage-", dir=output.parent) as temp:
        root = Path(temp)
        name = "shadow-game.exe" if "windows" in args.target else "shadow-game"
        relative = Path("bin") / args.target / name
        executable = root / relative
        executable.parent.mkdir(parents=True)
        if args.strip_tool:
            # -o never modifies Cargo's cached executable or its hard links.
            subprocess.run([args.strip_tool, "-o", str(executable), str(binary)], check=True)
        else:
            shutil.copyfile(binary, executable)
        executable.chmod(0o755)
        validate_binary(executable, args.target)
        if executable.stat().st_size > 512 * 1024 * 1024:
            raise ValueError("engine exceeds 512 MiB; strip debug symbols before packaging")
        (root / "docs").mkdir()
        for source in ("CONTROLS-RU.md", "HUD-RU.md"):
            shutil.copyfile(PROJECT / "docs" / source, root / "docs" / source)
        notices(metadata, root / "THIRD-PARTY-NOTICES.txt")
        (root / "README-RU.txt").write_text(
            "OpenSC — тестовая сборка движка Shadow Company: Left for Dead\n\n"
            "Содержит текущее главное меню, видеоролики из установки игрока и обучение.\n"
            "Полная кампания и часть игровых механик ещё разрабатываются.\n"
            "Rust и Wine для запуска этой нативной сборки не нужны.\n\n"
            "Лаунчер должен запускать bin/<target>/shadow-game отдельными аргументами:\n"
            "--game-dir <каталог оригинала> --settings <файл вне каталога обновления>\n"
            "Необязательно: --overrides <наши ресурсы вне каталога оригинала>.\n"
            "Оригинальные архивы, настройки и сохранения в пакет не входят.\n"
            "Требования конкретной сборки указаны в build-info.json.\n",
            encoding="utf-8")
        sources = source_hashes()
        info = {
            "schema_version": 1,
            "project": "opensc-engine",
            "version": package["version"],
            "build_id": args.build_id,
            "target": args.target,
            "profile": args.profile,
            "entrypoint": relative.as_posix(),
            "source_sha256": hashlib.sha256(json.dumps(sources, sort_keys=True).encode()).hexdigest(),
            "source_files": sources,
            "requires_original_data": True,
            "original_data_included": False,
            "minimum_glibc": glibc_requirement(executable) if "linux" in args.target else None,
            "launch_arguments": ["--game-dir", "<original-directory>", "--settings", "<user-settings-file>"],
        }
        (root / "build-info.json").write_text(json.dumps(info, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        inventory = [{"path": p.relative_to(root).as_posix(), "size": p.stat().st_size,
                      "sha256": digest(p), "executable": p == executable}
                     for p in sorted(root.rglob("*")) if p.is_file()]
        # A local packaging inventory, deliberately not manifest-v1.json.
        (root / "package-files.json").write_text(json.dumps(inventory, indent=2) + "\n", encoding="utf-8")
        os.rename(root, output)
    return {"package": str(output), "build_id": args.build_id,
            "target": args.target, "files": len(inventory) + 1,
            "bytes": sum(p.stat().st_size for p in output.rglob("*") if p.is_file()),
            "minimum_glibc": info["minimum_glibc"]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--cargo-metadata", type=Path, required=True)
    parser.add_argument("--target", choices=TARGETS, required=True)
    parser.add_argument("--profile", choices=("dev", "release"), required=True)
    parser.add_argument("--build-id", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--strip-tool", help="native strip tool; writes a separate copy")
    print(json.dumps(stage(parser.parse_args()), ensure_ascii=False))


if __name__ == "__main__":
    main()
