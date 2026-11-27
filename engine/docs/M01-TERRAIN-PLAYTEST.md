# M01 Still Water terrain playtest

The first area uses a 1,000 × 1,000 m cut inside the planned 4 × 4 km map.
The playtest renders its heightfield and uses that same surface for ground
orders, walking, running and camera clearance. The test squad uses two temporary
characters from the local game installation when there is room at the spawn.

## Prepare the terrain

The offline exporter needs Python, Rasterio and NumPy. It accepts an unrotated,
north-up GeoTIFF with metre coordinates and one elevation band. The input and
its source manifest stay with the local map assets.

```sh
python3 engine/scripts/export_heightfield.py /path/to/m01-core-1km.tif /path/to/m01.heightfield \
  --spacing 4 --height-origin 420 --spawn 180 450
```

The sample site uses the LVermGeo Sachsen-Anhalt DGM1 data attributed to
© GeoBasis-DE / LVermGeo ST under Datenlizenz Deutschland – Namensnennung 2.0.
Keep the dataset attribution, source query and conversion details alongside
any derived terrain you distribute.

The heightfield starts with the bytes `OpenSC heightfield 1\n`, then two
little-endian `u32` dimensions and three little-endian `f32` values for sample
spacing, spawn easting and spawn northing in metres. The remainder is
little-endian `f32` elevations relative to the chosen vertical origin. Samples
run west to east as columns and south to north as rows. The playable extent is
the sample count times the spacing. The outermost cell repeats the last sample.

## Run the playtest

```sh
shadow-game --game-dir /path/to/local/game --m01-terrain /path/to/m01.heightfield \
  --m01-layout engine/assets/m01/blockout.json
```

Click a character or drag a selection box. Shift adds the box to the current
selection and toggles a clicked character. Use 1 or 2 to select one member,
or 3 to select both. To appoint the commander, select one member and press K.
The gold ring marks the current commander. Selection alone leaves the
commander in place.

Click the ground to walk, double-click to run and Shift-click to queue a move.
A group order goes to the commander. Members in formation follow the route
the commander actually takes, including through doors. Giving the other
member an individual order takes that member out of formation. G brings
the squad back together. A plain ground click replaces the selected orders.
H stops the selection and, when the commander stops, any members following
the commander. With the layout loaded, orders report unreachable ground
such as the lake. T follows the selected character with the camera.

End switches between tactical orders and direct control. Holding Ctrl uses
direct control temporarily. WASD or the arrow keys move relative to the
camera and Shift runs. With both members selected, direct control goes to
the commander. The other member follows or continues an individual order.
PgUp and PgDn step between stances. Space toggles prone and standing.

In tactical mode, pan with WASD or a right drag, rotate and tilt with the
arrow keys, and zoom with the wheel. Home returns to the spawn. F6 opens the
weather panel. R toggles the headquarters roof, which clears automatically
while the character is inside.

The movement and direct-control conventions draw on the
[Faces of War manual](https://shared.fastly.steamstatic.com/store_item_assets/steam/apps/289300/manuals/manual%20ENG.pdf?t=1759490193).
The next controls stages on M01 cover contextual actions for objects and
cover, plus direct aiming, fire and vehicle controls as the corresponding
scenario content becomes ready.

Use `--scene-json /tmp/m01-scene.json` for map bounds and sample size, or
`--player-json /tmp/m01-walk.json --player-action walk --player-seconds 25`
for a movement trace. Add `--m01-order 272,880` to target the headquarters
interior from the default blockout arrival point. This diagnostic reports the
traces for both members when the test squad can spawn. The terrain preview uses
the spawn stored in the heightfield when no layout is supplied.
Use `--weather fog --time-of-day 5.75 --day-length 0` to inspect the location
in a pre-dawn setup. The layout marks the fictional village approach and
headquarters. Locations are local east and north metres, with elevations
relative to the heightfield origin. The current building and road are
prototypes for testing the CH01 arrival route.
