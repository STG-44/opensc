# Environment resource prototype

Locally authored visual replacements for the three South trees, six metal fence profiles, three masonry wall sections and the training Hummer. The game still needs an installed original copy. No original SMF, BMP or RAW is included here.

Trees have tapered trunks, branched crowns and 280 small curved foliage sprays per variant. Leaf surfaces use depth-writing alpha cutouts and cast masked shadows. Fence diamonds, tension wires, posts, coils and barbs are solid geometry. Their new galvanized steel albedo uses a physical metal material. Masonry walls have closed ends, consistent 40 × 20 cm block courses, and separate concrete coping. The Hummer retains its installed original body shape and UVs, including the roof taper. A repainted atlas follows its original camouflage layout and markings, with clear door seams, handles, hinges, glass gaskets and lenses. Authored round wheels, tread blocks, hubs, grille, lamp bezels, mirrors and hood vents add detail. The four original wheel mesh names and pivots remain intact so steering and rolling move every wheel detail together.

Build the private override pack from your installed data:

```sh
cargo run -p shadow-tools -- environment --game-dir /path/to/original/game --textures assets/environment/textures --output /path/to/new-environment-pack
cargo run -p shadow-model-viewer --bin shadow-game -- --game-dir /path/to/original/game --overrides /path/to/new-environment-pack
```

The output directory must not exist. Generated SMFs retain original attachment names, node IDs, transforms and bounds for cutting and opening gates. They contain newly generated meshes. A fingerprinted `.smf.visual.json` sidecar makes the engine read collision and picking geometry from the installed original rather than from each individual wire or leaf. The Hummer uses version 2 metadata with `render_base_parts: ["hummer-back01"]`: the engine draws that body directly from the installed original, alongside the owned detail meshes. Neither the original body nor its texture is copied into this pack. Its optional `texture_overrides` mapping selects the new PNG and four normalized UV glass regions. These regions subdivide the render surface along existing faces, preserving positions, UVs and collision, and use a smoother opaque glass material. Cabin transparency, interior geometry and live environment reflections remain unimplemented. Use an engine supporting both retained parts and texture remapping; rebuild the pack when the original data or replacement SMF changes. A custom pack without these sidecars retains the normal replacement collision behavior.

The generated output is private, derived from the installed models’ dimensions and attachment metadata. Keep it outside the repository. This first pass covers `AlScTr-South1..3`, `AlStFn-Consertinafence`, `_END`, `_CUT_END`, `ConsertinaFenceGate`, `BarbFence` and `BarbFence_CUT_END`, plus `AlStWa-Concrete01..03` and `AlVhQd-Hummer`. Other vegetation and damaged fence variants remain original. Distance LOD, wind animation and texture normal maps are future work. Geometry is deliberately bounded for the software renderer used during local verification.

The owned Hummer atlas also corrects the legacy mirrored stencil on the positive-X body side. Only the stencil UV rectangle is reversed; door and camouflage layout elsewhere is preserved. Glass uses reduced reflectance (0.12) with the current uniform ambient lighting so its painted reflections and tint remain visible. Open headlight bezels and textured lenses follow the same measured atlas ellipses, projected onto the original body to account for UV stretching. [hummer-optics.json](hummer-optics.json) records this calibration and the PNG SHA-256; the generator rejects a mismatched atlas until its optics are remeasured. Headlight beams originate at the lens geometry. Two physical wipers mount on the upper windshield rail and have validated pivot/axis/sweep bindings in the visual sidecar; their painted duplicates are removed from the atlas.

Vehicle equipment follows shared environmental conditions: headlights and dim rear position lamps activate only in darkness, while braking produces brighter rear stop lamps in daylight too. Wipers sweep during rain, pause with gameplay, and finish their current cycle before parking when rain stops. Source TOD luminance determines initial darkness; the current training map starts at 12:04 in dry conditions. The weather controller can update the shared precipitation value. A complete rain renderer, procedural weather changes and continuous mission-time progression are separate unfinished features. See [vehicle weather diagnostics](../../docs/VEHICLE-WEATHER.md).

## Versioned engine resources

`pack/` contains the validated owned output distributed with the engine. It
contains newly authored replacement meshes, generated textures and visual
sidecars; installed original body geometry and original texture atlases are
absent. Packaging validates the file allowlist, mesh fingerprints and inventory.
The engine reads this pack beside its executable, then applies the player's
overrides on top. `read_base` still reads the original installation, preserving
interaction geometry. Updating or rolling back the engine updates this owned
pack together with its binary.

## Texture provenance

`bark.png` and `leaves.png` were generated with the built-in image generation tool on 2026-11-04, then copied here without pixel edits. The leaf atlas preserves generated RGBA transparency. Geometry is authored in Rust in `shadow-tools/src/environment.rs` and its submodules. Galvanized steel, masonry and coping concrete albedos were also made with the built-in generator. `camouflage.png` is an unused earlier prototype; it is no longer included by the pack generator. `hummer-body-v1.png` was repainted with the built-in generator on 2026-11-05 using the private original atlas as a UV layout reference. The generated 1254 × 1254 PNG is copied unchanged; the original atlas is 128 × 128. Vehicle trim, lamps and wheels use constant material color swatches made by the Rust tool; their shape detail is geometry. Cabin interiors and opening vehicle doors are not implemented. Cosmetic attachments use the original vehicle collision proxy.

Prompts are recorded in [generation-prompts.json](generation-prompts.json), [vehicle-wall-prompts.json](vehicle-wall-prompts.json), [hummer-texture-prompt.json](hummer-texture-prompt.json), [hummer-glass-refinement-prompt.json](hummer-glass-refinement-prompt.json) and [hummer-wiper-cleanup-prompt.json](hummer-wiper-cleanup-prompt.json). The glass refinement increases the painted sky/tint contrast so it remains visible in the game. The final cleanup removes painted windshield wipers so only their animated geometry remains. These replacement assets use the project MIT license; installed original data remains separate and private.

## Vehicle validation

The interactive vehicle smoke requires the preceding training lessons to be prepared first, as documented in `engine/docs/VEHICLE-RU.md`:

```sh
cargo run -p shadow-model-viewer --bin shadow-game -- --game-dir /path/to/original/game --overrides /path/to/new-environment-pack --player on --player-action armor --player-seconds 420 --vehicle-smoke-json /tmp/vehicle-input.json --screenshot /tmp/vehicle-input.png
```

Starting this fixture from the initial `idle` spawn does not prepare the boarding objective or the path out of the initial training area. The fixture now reports a rejected approach after allowing the mouse release to be processed instead of indefinitely retrying clicks. This validation change does not alter normal gameplay input.
