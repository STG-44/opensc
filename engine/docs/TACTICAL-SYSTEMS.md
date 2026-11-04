# Radar, perception and obstacle navigation

These systems use the player's original map and scenery. Their current scope
is the training mission and its two controllable characters.

## Radar

The default view is a stationary north-up map. The lock button switches to
a camera-centered map that rotates to keep the camera facing up. The +/−
buttons select 1×–8× zoom. Left click or dragging moves the camera without
ordering or moving a character.

Green circles show current friendly positions; a white border marks selected
characters. Detected enemies appear red. Jack is blue before recruitment
when the team can see him. The white outline and cross show the camera
footprint and focus independently of the squad.

`radar.rs` uses one invertible transform for texture sampling, markers, camera
geometry and input. A drag retains its initial transform so a stationary held
pointer cannot cause recentering drift. Release outside the HUD is consumed;
opening inventory cancels the gesture. Map-edge input is clamped to the last
terrain sample rather than the texture's larger coverage extent.

The image is regenerated at the current UI/DPI size only when its inputs change.
The camera pose is shared with the renderer. Its footprint is the camera
frustum's intersection with a horizontal plane at focus height; it does not
exactly follow hills or building occlusion.

Map modes and marker colors are based on the player's video and the original
[manual, page 22](https://www.scribd.com/document/777805314/Shadow-Company-Left-for-Dead-Manual-Win-En).
The precise executable implementation is not claimed to be reproduced.

## Team visibility and fog

`perception.rs` reads `config/perception_var_defs.txt` and the characters'
Perception/Infiltration skills through `AssetCatalog`. The supplied parameters
give a detection range of **30 m + Perception × 0.3 m**, with automatic
detection within **5 m** only when line of sight is clear.

Outside that near range, perception is reduced with distance and compared to
the target's infiltration and original stance/movement modifiers. Height
advantage is bounded by the source configuration. Terrain height and opaque
scenery can block sight. Fences with transparent materials do not become
solid opaque walls for this test.

John and recruited Jack share vision. Unrecruited Jack does not reveal areas
for the player. Detected state is used by both radar contacts and enemy world
rendering/picking. Corpses are not displayed as live hostile contacts; a body
being carried remains visible with its carrier.

Radar terrain has three states: currently visible, explored but outside current
sight, and unexplored. Exploration persists for the mission and resets with
the training restart. Camera movement alone does not reveal terrain or enemies.
Visibility refreshes at the source configuration's minimum interval (750 ms in
the original data) and is cached while relevant world state is unchanged.

The fog grid uses approximately 2 m cells. Terrain line tests sample the height
field; opaque scenery uses its actual triangles through a spatial index.
Lighting, night-vision equipment and the complete original AI perception
model remain outside this implementation. Fog darkens the radar, not the full
3D terrain. Enemy AI behavior itself is still incomplete.

## Hearing and noise

`hearing.rs` keeps auditory memory separate from visual contacts and exploration.
An audible event records its emitted position for `audible_duration` (2.5 seconds
in the original configuration). An idle, living training guard turns toward the
strongest remembered hostile sound, then the most recent sound of equal level.
It does not track an unseen character after the sound. Entering the radius of
an earlier shot cannot create a new hearing event. Allies ignore friendly noise.

Footsteps use resolved displacement, including collision and navigation. A blocked
movement order, mission placement, riding in the vehicle or carrying a corpse
does not independently generate footsteps for the blocked/riding/carried actor.
The radius uses the source stance/walk/run values and this provisional formula:

`noise_base + stance_motion_noise * max(stealth_skill_base - Infiltration, 0) * stealth_skill_mod`

Source centimetres are converted to metres. With Infiltration 0, standing walk/run
radii are approximately 4.54/20.63 m; with Infiltration 100, 1.51/6.88 m.
This movement formula is **not yet verified against the original executable**.
Gun radii and levels come directly from `FIRE_NOISE_RADIUS` and `FIRE_NOISE_LEVEL`
in the gun profile: BDA is 36 m/level 3; Uzi is 48 m/level 4. Each combat round
creates a stimulus, including burst rounds without a separate audio sample.
The audio volume setting does not change tactical hearing.

A green outline on the radar shows the strongest active noise radius of a
selected friendly character at the sound's last position. It uses the same
rotation and zoom as the map and disappears when the event expires. Inventory
and world pauses freeze auditory memory; mission restart clears it.

Hearing never reveals enemy models, precise hostile radar contacts or terrain.
Sound currently propagates by distance without wall/material attenuation. Guard
investigation routes, pursuit, return fire and full original AI are not implemented.
The guard's turn is a first reaction, not a complete combat behavior model.

## Ground navigation

`navigation.rs` plans ground routes for John and recruited Jack. The public
destination remains the requested final point; intermediate waypoints guide
animation-driven movement while preserving the selected walk/run mode.

The planner first tests a direct route, then uses an eight-neighbor A* grid
with 0.5 m spacing. Exact endpoints connect to the grid through tested segments.
Path smoothing removes a waypoint only when the entire replacement segment is
clear. The same 0.20 m character radius and stance-dependent height used by
continuous collision are used for route clearance. Diagonal edges cannot pass
through obstacle corners.

Scenery, terrain, closed fences and the current gate pose participate in path
checks. Geometry changes invalidate the spatial index; existing paths are checked
again and replanned if blocked. A destination inside solid geometry or an
unreachable area stops the order and displays a localized notice.

This is ground navigation, not crowd simulation. It does not add friendly-body
avoidance, automatic ladder use, automatic fence cutting/crawling or vehicle
pathfinding. The conservative terrain slope limit is 45 degrees. Searches are
bounded to 30,000 expanded nodes; very narrow passages can fall below grid
resolution. Original routing-table compatibility is not claimed.

## Validation

Unit regressions cover coordinate inversion, map edges, camera frusta,
detours with continuous clearance, unreachable walls, waypoint replacement,
perception range/stance, opaque occlusion and exploration memory.

`--radar-smoke-json` runs a native integration scenario with isolated settings.
It checks HUD picking, both map modes, zoom, camera dragging, release outside
the radar, inventory cancellation and resizing. It also orders John around
an original prop through mouse events, exercises Jack's independent route,
checks occlusion against an original structure, shared team sight and fog memory.
Scene positions and recruitment state are seeded by the harness. Physical
pointer use and Windows/macOS rendering require separate hands-on checks.

`--hearing-smoke-json FILE.json` checks the scheduled perception/combat/HUD path
with isolated settings. It seeds shot events and an original opaque-structure
fixture, verifies hidden enemy models and unchanged fog, guard turning, the radar
noise outline, pause/expiry/reset, and footsteps from an actual animation-driven
running order. This is a native Linux regression; it does not replace interactive
Windows/macOS checks or verification against the original executable.
