# Contributing to OpenSC

Keep original data separate from code and overrides, preserve native platform
targets, and describe reconstructed behavior when the original rule is uncertain.

## Repository workflow

Owner-directed development is consolidated in `main`. Commit validated changes
directly to `main`; do not keep a stack of long-lived working branches. Preserve
in-progress checkpoints with an explicit status and validation limits. Automated
launcher and engine checks run for relevant pushes to `main`.

Original assets, private transfer archives and conversation exports must remain
outside Git. Publishing source to GitHub does not promote a server update channel.

## Build and check

The engine/launcher pin Rust 1.99; the server declares its own minimum version.
Use the component's lockfile and toolchain.

```sh
# Engine
cargo fmt --manifest-path engine/Cargo.toml --all --check
cargo test --manifest-path engine/Cargo.toml --workspace --locked
cargo clippy --manifest-path engine/Cargo.toml --workspace --all-targets --locked -- -D warnings

# Launcher
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings

# Server
cargo test --manifest-path server/Cargo.toml --locked
```

Run the game with your own data using the [engine guide](engine/README.md).
For UI/gameplay changes, check the affected native flow with isolated settings.
Record the data set, resolution, language and backend. Distinguish injected
input from physical-input checks; test 16:9 and 21:9 for layout changes.

## Resources and behavior

Prefer original resource definitions, the manual and reproducible video
references over guesses. Keep coordinate systems and units explicit.
Use `AssetCatalog` so original files and overrides follow the same rules.

Do not commit game archives, local installation paths, private report databases
or release credentials. Keep generated packages and test output outside the
source tree unless a small fixture is needed by a regression test.

## Pull requests

Write titles and descriptions in English. Explain the resulting behavior,
validation and material limits. Keep unrelated changes separate. Add tests
for meaningful parser/gameplay regressions rather than restating implementation.

Engine PRs run formatting, workspace tests, Clippy and packaging for Linux
x86-64/ARM64, Windows x86-64 and macOS Intel/Apple Silicon. Build success alone
does not verify a rendered game window on all platforms.

## Bug reports

Use the [issue form](https://github.com/STG-44/opensc/issues/new/choose) or the
launcher's report function. Include versions, platform, reproduction steps and
expected results. For asset problems, name the object/mission and add a screenshot.
Check diagnostic excerpts before posting them publicly.

## Licensing

Project-owned source code and contributions use the [MIT License](LICENSE).
Preserve third-party license notices for dependencies and fonts.
