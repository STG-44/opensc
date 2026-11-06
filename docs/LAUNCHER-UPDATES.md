# Launcher update lifecycle

Launcher and engine versions are independent. Stable and Test select the engine
channel; choosing a channel must not change the user's language, original data,
overrides or game preferences. A fresh installation defaults to Stable.

## Startup and Play

1. Before loading preferences or creating a window, check the locally cached
   signed launcher index. A newer compatible cached launcher is used offline
   only after its Ed25519 signature, manifest and installed file hashes match.
2. Forward `--data` as a separate argument and wait for the new window's atomic
   readiness signal. The bootstrap exits without reading or saving preferences.
   A failed handoff leaves the installed launcher available.
3. Check online for a launcher update before checking or installing the engine.
4. Inspect the selected engine channel. Stage and verify required files before
   activating a release. Play always checks the selected channel again.

Every settings change is saved atomically when it occurs. Closing an older
window does not save its stale in-memory profile over a newer window's changes.
Unknown preference fields are retained for compatibility with future versions.
Legacy profiles pointing at Test retain Test when the explicit channel field
was removed by an older launcher.

## Silent periodic checks

While the launcher is open, check for updates every five minutes. Routine checks
have separate receivers from visible operations. An unchanged release does not
clear the current release, replace ready text, reset progress, restart the
window or disable Play. An explicit Play request can take over an ongoing quiet
launcher check, then check the engine before starting it.

Background engine-check failures keep the working release and visible state.
Retry after ten, twenty, then thirty minutes; a successful check restores the
five-minute interval. Results from a previously selected channel are ignored.
New releases may download automatically. Engine installation and launcher
handoff wait until the running game exits. No persistent system service is
installed; closing the launcher ends its checks.

## File analysis, installation and repair

Before downloading, verify reusable files in the target and currently active
releases against the new manifest's size and SHA-256. Copy matching files
independently into staging and download only the remaining files. Reinstalling
an already verified release does not download its payload again.

Activate a new version only after all staged files pass verification. A failed
or cancelled download preserves the previous active pointer and directory.
The file-verification action also repairs damaged files. A completed repair
replaces the damaged directory as a whole and retains it under `repair-backups/`;
a failed replacement restores the old directory. Original game data, overrides
and preferences remain outside release directories.

## Reference behavior

Gaijin documents separate [launcher updates](https://support.gaijin.net/hc/en-us/articles/13599130331025-Stuck-at-Analyzing-files-during-an-update-of-the-game),
[game-file verification and repair](https://support.gaijin.net/hc/en-us/articles/200070331-Error-81110007-File-read-error-Try-to-repair-installation-by-launcher),
and an optional [background updater](https://support.gaijin.net/hc/en-us/articles/115001876905-Gaijin-Net-Updater).
These sources describe observable War Thunder behavior, not its private
implementation. The lifecycle above is OpenSC's implementation of those user
expectations, with in-window periodic checks requested for this project.

## Validation

Run `cargo test --locked`, `cargo fmt --check`, and
`cargo clippy --locked --all-targets -- -D warnings` for the launcher. Tests cover
legacy migration, stale-window shutdown, forward-compatible preferences, signed
offline cache verification, handoff arguments, startup ordering, quiet checks,
backoff, file reuse and failed repairs. Native close-and-reopen checks are also
required for each platform before publishing its package.
