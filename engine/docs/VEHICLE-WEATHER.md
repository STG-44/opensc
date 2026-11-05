# Hummer lighting and wipers

The enhanced Hummer has separate textured front and rear lamp surfaces and two
top-mounted physical wipers. Lenses and open bezels use measured atlas ellipses
projected onto the installed body's UVs. The calibration in
`assets/environment/hummer-optics.json` is tied to the PNG's SHA-256; repainting
the atlas requires measuring it again. Beam origins come from the two lens
meshes. Collision, seats, wheel pivots and
driving physics still use the installed original model.

The shared environmental resource contains relative illumination and rain
intensity. Initial illumination comes from the mission's sampled TOD sun color;
rain defaults to zero. Luminance below 0.55 selects darkness. These thresholds,
night photometry, lamp strengths and wiper speed are modern presentation rules,
not verified reconstructions of the original EXE.

| Conditions | Headlights | Rear position lamps | Wipers |
| --- | --- | --- | --- |
| Bright, dry | Off | Off | Parked |
| Bright, rain | Off | Off | Sweeping |
| Dark, dry | On | Dim red | Parked |
| Dark, rain | On | Dim red | Sweeping |

Braking activates brighter red stop lamps in every lighting condition. Opposite
throttle while moving and the halt command trigger braking; coasting does not.
The brief brake hold keeps an instantaneous halt visible. These are combined
position/stop lenses with distinct brightness states. Reverse lamps, turn
signals and vehicle ignition are not implemented.

Wiper motors sit on the upper windshield rail. Their rotation axes follow the
glass plane, sampled inside the pane rather than on the adjacent roof face.
Wiper parts rotate around their sidecar's source-space pivot and axis. The
loader converts source Z-up coordinates to Bevy coordinates and normalizes the
pivot with the mesh in the model browser. Rain starts a smooth 1.2 Hz cycle;
stopping rain finishes the cycle and returns the blades to the parked pose.
Gameplay pause freezes the cycle. Painted windshield blades are absent from
the replacement atlas, preventing duplicate wipers.

Two forward spot lights follow the vehicle hierarchy and use a 32 m range.
The lenses emit only in darkness. Night sun/ambient strengths are reduced so
the lamps can illuminate the terrain. Spot lights currently do not add shadow
passes. Glass uses painted environment reflections and direct-light highlights;
live reflections and a transparent cabin interior are not implemented.

## Reproduction

Diagnostic preview flags do not change game settings or the original data:

```sh
shadow-game --game-dir /path/to/original/game --overrides /path/to/environment-pack --environment-preview night-rain --camera-orbit 39.84,13.4,-102.5,0.04,0.25,6.5 --screenshot /tmp/hummer-night-rain.png --vehicle-weather-json /tmp/hummer-night-rain.json
```

Available preview conditions: `day-dry`, `night-dry`, `day-rain`, `night-rain`.
The optional JSON report requires a screenshot. It checks two physical wipers,
two headlight beams, both lamp material roles, correct day/night emission, and
wet sweep versus dry parking across 110 real application frames.

The regular vehicle input fixture also checks that real driving/halt input
activates the stop-lamp state. Four-condition unit tests cover lighting and
braking, and rain-stop/pause tests cover wiper motion.

The training mission currently starts at 12:04 with no precipitation setting.
The shared conditions can be updated by a future weather controller. Procedural
weather, rain particles, water on glass and continuous mission time remain
unfinished; diagnostic rain supplies precipitation state without claiming a
complete rain renderer.
