# OpenSC engine

The **Rust + Bevy** runtime for OpenSC loads the player's original Shadow
Company: Left for Dead resources and optional replacement assets.

[Project](../README.md) · [Contributing](../CONTRIBUTING.md) ·
[Tactical systems](docs/TACTICAL-SYSTEMS.md) · [Русский](README.ru.md)

The current gameplay development branch is
[`codex/original-inventory`](https://github.com/STG-44/opensc/tree/codex/original-inventory)
([PR #2](https://github.com/STG-44/opensc/pull/2)). Switch to that branch before
building the latest test runtime.

The workspace pins **Rust 1.99.0** and **Bevy 0.20.0**. Its current playable
scope is the training mission: the original map, characters, animation,
tutorial events, combat, inventory, vehicles and evacuation. The complete
campaign is still in development.

## Run from source

From the repository root with the pinned Rust toolchain installed:

```sh
git switch codex/launcher-channels
cargo run --manifest-path engine/Cargo.toml --locked \
  -p shadow-model-viewer --bin shadow-game -- \
  --game-dir "/path/to/Shadow Company"
```

The same arguments accept Windows paths. Add `--overrides "/path/to/replacements"`
to replace assets; preserve the internal directory structure. Use
`--settings "/path/to/test-settings.json"` for isolated settings.

The UI supports 15-language (`--language ru|en`), HD and wide windows
(`--window 1920x1080`). See `--help` for inspection and native-check modes.
Linux needs the platform libraries installed by the [engine workflow](../.github/workflows/engine.yml).

## Controls

| Action | Input |
| --- | --- |
| Select / add to selection | Left click / Ctrl + left click |
| Move / run | Left click terrain / double click terrain |
| Select John / Jack / both | 1 / 2 / F11 |
| Stand / crouch / prone | S / C / P |
| Stop / inventory / tracking | H / Space / T |
| Pause / return | Esc |
| Camera position through radar | Left click or drag |
| Radar scale / orientation | + / − / lock buttons |
| Diagnostics | F3 |

Context actions depend on the character and equipped item. The detailed
original control map is in the [Russian guide](README.ru.md).

## Workspace

| Crate | Responsibility |
| --- | --- |
| `shadow-formats` | Validated resource readers and override catalog |
| `shadow-tools` | Asset extraction and structured inspection |
| `shadow-viewer` | Texture inspection |
| `shadow-model-viewer` | Character/map viewers and `shadow-game` |

`navigation.rs` plans paths
against character collision geometry. `perception.rs`
shares team visibility with enemy rendering and the radar. `hearing.rs` records
short-lived sound stimuli and idle guard reactions independently of sight.
`radar.rs` uses one invertible transform
for the map, markers, camera outline and mouse input.

## Validate

```sh
cargo fmt --manifest-path engine/Cargo.toml --all --check
cargo test --manifest-path engine/Cargo.toml --workspace --locked
cargo clippy --manifest-path engine/Cargo.toml --workspace --all-targets --locked -- -D warnings
```

Native scenarios require original data and a working graphics/display setup.
Create a separate output directory and settings file:

```sh
shadow-game --game-dir "/path/to/game" --settings "/tmp/opensc-check/settings.json" \
  --window 1280x720 --radar-smoke-json "/tmp/opensc-check/result.json"
```

The radar scenario exercises UI picking, coordinates, dragging, resizing,
original-scenery detours, visibility and exploration memory.
`--input-smoke-json` exercises inventory and body carrying;
`--hearing-smoke-json` checks sound, guard turning, pause/expiry and the noise ring. These scenarios
inject input events; they do not certify physical input or every driver.

## Packaging and limits

The [engine workflow](../.github/workflows/engine.yml) builds five native targets.
`scripts/package-game.py` records source/binary hashes, validates architecture
and includes dependency notices. The repository's `scripts/publish_release.py`
creates the versioned update channel from verified packages.
Original game archives are excluded.

Current limits include incomplete campaign/AI behavior, several HUD commands,
lighting perception modifiers, complete hearing-driven AI, mission saves and
full executable compatibility. The camera outline uses a horizontal reference plane; radar fog
does not darken the complete 3D terrain. See [tactical systems](docs/TACTICAL-SYSTEMS.md).

Project-owned engine source uses the [MIT License](LICENSE); dependency, font
and original-game asset terms remain separate.

The launcher selects and passes the interface language. See [shared language details](../docs/LOCALIZATION.md), including translation review and bundled font scope.
