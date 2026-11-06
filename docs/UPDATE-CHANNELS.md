# Launcher update policy and engine channels

The development launcher 0.4.3 checks for signed launcher and engine updates when
opened and when Play is selected, and silently every five minutes while open.
Unchanged background checks preserve ready text and keep Play available. Failures
back off to ten, twenty and then thirty minutes. Launcher updates precede engine
updates; a running game defers installation. See the [update lifecycle](LAUNCHER-UPDATES.md).
Play checks the selected channel again, completes any
engine update and only then starts the game. Failed or cancelled downloads keep
the previous installation and wait for an explicit retry. When a pre-launch check fails, an already verified release of the selected channel can run offline. A recorded successful channel check allows reusing identical Stable/Test payloads. A release from a different unconfirmed channel is never used as an offline substitute.

**Stable** is the default. **Test** is an explicit opt-in for new features.
Changing the channel saves it immediately and clears the previous check result;
it performs no network request until the next scheduled check, opening or game launch. Returning
to Stable can install an older stable version. Versioned engine packages,
original data, overrides and game preferences remain separate.

Both manifests live beside the existing payloads:

- Stable: `/updates/manifest-stable-v1.json`
- Test: `/updates/manifest-test-v1.json`
- Legacy clients: `/updates/manifest-v1.json`, following Stable

New engine releases are uploaded and verified before moving a channel pointer.
Use `scripts/publish_engine_channel.py` with an explicit channel, the immutable
candidate manifest and the previous pointer SHA-256. Stable promotion also
requires the expected legacy pointer SHA-256. Test publication never changes
Stable or the legacy pointer. Retain previous manifests for rollback.

Launcher self-updates keep their independent Ed25519-signed channel. Choosing a
test engine does not disable signature verification or install an unsigned
launcher.
