# World rendering

OpenSC loads the original SMF geometry and original textures through the asset
catalog. Replacement resources take precedence. The renderer does not simplify
meshes; the map currently chooses the first (near) model in each LOD profile.
The original low polygon counts and small textures remain visible at close
camera distances. Detailed replacement models require separately authored assets.

## Concrete wall ends

The original `AlStWa-concrete01`, `02` and `03` models have open body and coping
panels at the positive-X end. Rendering both sides cannot close a missing face.
The loader adds four triangles on that existing end plane, using the existing
profile corners and wall texture. Source faces, object placement and doorways
are preserved. Picking and collision use the resulting mesh as rendering does.

The repair applies only when the entire SMF matches its original SHA-256.
Authored replacements, including files with the same name, remain untouched.
Model diagnostics identify the added triangles.

## Terrain boundary

The training terrain atlas covers 128 × 128 metres; some original walls stand
between 126 and 128 metres. Drawing only the cells between the 64 × 64 height
samples left an uncovered band there. The outer cell now repeats the last
row/column of heights. Original samples, interior interpolation, UV scale and
object coordinates are unchanged. Mesh generation, height queries, picking,
navigation, movement limits and radar clicks share that boundary. This is an
explicit boundary reconstruction; the original EXE's outer sampling rule has
not been verified. The terrain texture clamps at its borders.

Some wall pillars extend a further 10.6 cm east or 2.8 cm south of the nominal
training boundary. An additional render-only cell around the map covers those
protruding details using clamped edge heights and texture samples. That visual
border does not enlarge terrain picking, movement, navigation or radar bounds.

## Leaves and shadows

South-tree leaf cards use the original RAW alpha mask as a depth-writing cutout
with a 0.5 cutoff. Overlapping cards no longer rely on transparent-object
sorting; their shadows have the leaf mask instead of a solid rectangle. The
source alpha pixels are preserved. Other soft transparency retains blending.

The mission directional light casts shadows using two 1024-pixel cascades,
covering 20 and 100 metres from the camera. The source sunlight direction and
color still come from the campaign time tables. These are modern Bevy shadow
maps, not a reconstruction of the original renderer. Sky layers, continuous
time progression and full environment/fog rendering remain unfinished.

## Reproducing a view

For a fixed diagnostic screenshot, run `shadow-map-viewer` or `shadow-game`
with `--camera-orbit X,Y,Z,YAW,PITCH,DISTANCE --screenshot FILE.png` and the
usual `--game-dir`. Positions and distance use metres; angles use radians.
This option changes the preview camera only and does not write game settings.

## Local environment replacement prototype

The optional environment pack replaces the three South trees and six training fence profiles with generated SMF geometry. It is built locally from installed original dimensions, never written into the installation. Bark and leaf albedo assets and the Rust generator are in [the environment assets directory](../assets/environment/README.md).

Trees use tapered trunks and branches with 280 curved leaf sprays per variant; RGBA leaves use `AlphaMode::Mask(0.5)` to write depth and masked shadows. Chain link, concertina coils and barbs are actual tubes. The steel albedo path selects a metallic material in both map and model viewers, including the post-cut fence.

A matching `.smf.visual.json` sidecar selects the installed original as the interaction proxy. Both original and replacement SHA256 values must match. Picking, movement obstacles, fence-cut targets and gate leaf collision use those proxy parts; drawing uses the enhanced parts. This avoids making each tiny wire an individually clickable or collidable object. Regular user SMF overrides without a sidecar keep their existing behavior. Gate pivots, mesh names, action nodes and cut collision bounds are retained.

The local pack also enhances the Hummer and authors three masonry wall profiles. Vehicle attachment nodes, helper IDs, wheel names and interaction geometry remain installed originals. New wheel geometry (treads, rims, hubs and bolts) stays in one mesh per original wheel, allowing the existing steering and rolling transforms to animate all of it. The Hummer's version 2 sidecar selects the installed `hummer-back01` body as a retained render part. The original silhouette and UVs are preserved, with source-surface-aligned detail meshes added around it. Original body geometry and texture are not distributed in the replacement pack. Model-browser normalization applies to the assembled body, details and interaction proxy together. No cockpit interior or vehicle door animation is added.

The same version 2 sidecar can map a retained part's installed texture to an
override PNG. The Hummer uses a repainted 1254 × 1254 atlas in place of the
128 × 128 original, following its existing UV layout and camouflage markings.
Four UV rectangles identify windshield and door glass. Render triangles are
clipped against these rectangles with interpolated positions, normals and UVs;
their union uses an opaque glass material with roughness 0.16, while paint uses
0.78. The surface stays coplanar and the installed collision mesh is unchanged.
Window reflections in the atlas are painted, with material highlights from
scene lights; live environment reflections and transparent cabin interiors are
not implemented. Missing remap textures, invalid paths and invalid rectangles
are reported as errors instead of silently using a mismatched texture.

For the owned Hummer atlas, the loader reverses only the stencil's UV rectangle
on the positive-X body side, so its letters read normally from both sides.
The remaining door/paint UVs and the installed data are unchanged. Glass uses
reflectance 0.12 as an artistic adjustment for the current uniform ambient
lighting; the default specular lobe washed its albedo into a gray pane.
Headlight bezels are open rings. Their inner/outer contours and the textured
lenses share measured atlas ellipses projected onto the original front faces,
preserving alignment despite unequal atlas spacing and UV stretching. The
calibration file records the texture SHA-256; the generator rejects stale
calibration after a repaint. Headlight beam origins follow the lens geometry.

The enhanced vehicle's textured lamp meshes have separate emission roles.
Darkness enables headlights and dim red position lamps; braking enables brighter
red stop lamps in daylight as well. Physical windshield blades use validated
sidecar pivot/axis/sweep bindings and upper-rail motors, with no painted
duplicates. Their rotation axes follow the windshield plane. Rain drives their
smooth sweep and stopping rain parks them. See [vehicle equipment](VEHICLE-WEATHER.md)
for the shared condition state, native diagnostics and current weather limits.

Masonry profiles are authored closed volumes with blockwork and coping materials. Their installed dimensions and original interaction proxies preserve map openings and the earlier wall-end repairs. Vertical texture coordinates use model-space height so the different node origins do not misalign the masonry courses. New metal, masonry and coping albedos live in the resource pack, with source prompts in the assets directory.

## Visibility scheduling

Radar fog scans run on the async compute pool, with at most one scan in flight.
Each scan owns immutable terrain and geometry snapshots. A geometry revision
invalidates the cached snapshot; moving gates and vehicles cannot mutate one
already in use. The main update polls without waiting, applies completed masks
atomically and unions newly explored cells. Observer and geometry changes are
tracked separately from enemy contact changes, so a pending scan cannot consume
a later observer update. Reset discards pending work and exploration.

Enemy contact checks retain their source-configured cadence and visibility
rules. Boolean geometry rays reuse a thread-local visitation buffer and stop
at the first occluder. Collision clearance uses the same spatial index and
early exit; closest-hit queries still return the closest hit. See
[CPU diagnostics](PERFORMANCE.md) for measurement scope and reproduction.
