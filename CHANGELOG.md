# Changelog

OpenSC uses the website's test update channel. Engine and launcher versions
are independent.

## Launcher 0.4.2 and engine 0.1.0-test.20261103.3 — 2026-11-03

- Check updates on launcher opening and before Play, with no periodic polling.
- Save an explicit Stable/Test engine channel; default new and migrated profiles to Stable.
- Complete the selected channel update before starting the game, including switching back to an older stable release.
- Simplify launcher settings and remove duplicate main-screen actions.
- Share a saved language between the launcher and game across 15 UI locales, including Ukrainian, Simplified Chinese and Traditional Chinese.
- Bundle CJK UI fonts and preserve dynamic values and controls in translated text.

See [channel policy](docs/UPDATE-CHANNELS.md) and [translation scope](docs/LOCALIZATION.md).

## Engine 0.1.0-test.20261103.2 — 2026-11-03

- Restore live team markers, detected enemies and the camera footprint on the radar.
- Add north-up and camera-relative views, zoom and camera dragging with shared coordinates.
- Add obstacle detours for John and recruited Jack, continuous clearance and path revalidation.
- Detect enemies using original perception parameters, skills, stance, height and line of sight.
- Preserve explored radar terrain, hide lost contacts and prevent camera-only exploration.
- Provide English project/contributor documentation alongside Russian guides.

See [tactical systems](engine/docs/TACTICAL-SYSTEMS.md) for exact limits.

## Engine 0.1.0-test.20261103.1 — 2026-11-03

- Restore the original inventory layout, character browsing and skills.
- Show equipped weapons, rounds and grenades below portraits.
- Fill the backpack grid to the right frame on wide screens.
- Preserve running during interactions and use the original walking cycle.
- Add body placement animation and collision with crates, tables and barrels.
- Repair mirrored rear labels on original GRANITE and weapon crates.

## Launcher 0.4.1

- Import game folders, MDF/MDS and ISO into a managed library.
- Check for engine updates automatically and install verified versioned packages.
- Preserve original data and settings independently of executable updates.

Earlier notes remain in the [Russian project history](README.ru.md).
