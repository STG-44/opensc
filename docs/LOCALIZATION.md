# Shared launcher and game language

The launcher is the language selector. Its saved language is passed to the game
as a fixed `--language CODE` argument, including command-line launches. Game
settings keep their other values. When started by the launcher, the game displays
the selected language and directs language changes to the launcher. Existing
Russian and English game preferences are migrated when the launcher profile has
no saved language.

Supported codes are `ru`, `en`, `uk`, `de`, `fr`, `es`, `it`, `pt-BR`, `pl`, `cs`,
`tr`, `zh-Hans`, `zh-Hant`, `ja` and `ko`. Simplified and Traditional Chinese are
separate choices. Names appear in their native script.

`engine/crates/opensc-localization` owns the shared codes, native names and offline
UI catalogs. Catalogs use the current Russian source copy as keys. Static phrases
and boundary-aware dynamic labels retain numbers, control names and resource
identifiers. Format templates use indexed `{0}`, `{1}` placeholders in translated
values; preserve their identities when changing word order. All catalogs must
cover the same source keys. User-written bug reports and original resource paths
are never submitted to a runtime translation service.

The additional catalogs were seeded with machine translation. Primary launcher
labels were reviewed; broader translations need fluent-speaker review, especially
context, terminology and grammatical agreement in dynamic labels. Report the
language, screen and offending phrase when suggesting a correction. This covers
the current interface and training hints; original voice/video audio remains in
the player's installation.

CJK fonts are bundled as renamed static regional Noto Sans fonts under SIL OFL 1.1, retaining the upstream regional glyph coverage for user-written text.
The asset directory includes upstream revision, hashes and license. They are compressed in the repository and unpacked locally. Rebuild them
with `scripts/build_ui_fonts.py --upstream-directory DIR` when updating the fonts. The script requires fonttools and the pinned upstream files; no font
or translation download is needed at runtime. The launcher uses DejaVu Sans for
Latin/Cyrillic and locale-specific CJK fallbacks; the engine retains its existing
fonts for Latin/Cyrillic and selects the matching CJK UI font.
