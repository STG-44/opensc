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
