# Config storage: `block-global.toml` and `block.toml`

Updated: 2026-10-24. This is a **breaking** configuration change. The previous
JSON configuration documents are no longer read or written, and no migration is
performed. Existing `preferences.json`, `accounts.json`, `providers.json`,
`storage.json` and root `catalog.json` files are left untouched on disk but are
ignored entirely; the launcher starts from defaults on the new files.

## Data root

| Platform | Data root |
| --- | --- |
| macOS | `~/Library/Application Support/one.ztd.block/` |
| Windows | `%APPDATA%\Block\` |
| Linux | `~/.local/share/Block/` |

`BLOCK_DATA_DIR` still overrides the data root for the desktop app, and the
CLI's `--data-dir` option selects an external root. The platform application
folder holds the automatic global game library
(`crates/block-ui/src/setup_storage.rs::global_application_directory`),
used by installed builds; portable/writable builds keep `.minecraft` beside the
application instead. Neither location is user-selectable.

## Files

Exactly two user-configuration documents exist:

### `block-global.toml` (block-ui owns it)

Written by `crates/block-ui/src/global_config.rs`. Sections:

```toml
[preferences]
appearance = "Dark"
reduce_motion = true
onboarding_complete = true

[preferences.game]
memory_mib = 2048
fullscreen = false

[preferences.downloads]
max_concurrent = 8

[[accounts]]
provider = "offline"
player_name = "Player"
# … account tables …

[[providers]]
name = "Community server"
api_url = "https://community.example.test/api/"
# … provider tables …
```

Write path: load the whole document, apply the change, re-encode and atomically
replace it (same-directory temp file, `sync_all`, persist). An in-process mutex
serializes read-modify-write cycles; `Preferences::save` additionally holds the
data-root lease. Writes only own `[preferences]`, `[[accounts]]` and
`[[providers]]`, so they cannot erase each other or foreign sections.

### `block.toml` (block-launcher owns it)

Written by `crates/block-launcher/src/config.rs`. Sections: `[storage]`
(library/cache/generation) and `[catalog]` (catalog schema version and the
installation snapshot). Callers must already hold the data-root lease
(`storage::acquire`); catalog writes and storage relocation satisfy this, which
serializes read-modify-write cycles across processes and keeps
`[storage]`/`[catalog]` from dropping each other. Unknown top-level sections are
preserved verbatim.

```toml
[storage]
cache = "/data/cache"
library = "/data/games"

[catalog]
schema_version = 1

[catalog.snapshot]
profile_mode = "bundled"
# … instances/profiles …
```

`None` fields are omitted because TOML has no `null`; values are converted
through `serde_json` with null object members stripped on write.

## Deliberate exceptions (not user configuration)

- `storage-migration.json`: transient crash-recovery journal for an in-flight
  relocation. It is removed when the migration resolves and stays JSON on
  purpose.
- `catalog.lock`, `storage.lock`, `.block-game.lock`, `.block-session.json` and
  session reservation files: runtime locks/markers.
- Per-instance files under the game folders (`installed.json`, manifests,
  downloaded resources): game data, not launcher configuration.

## Known limitations

- Cross-process account/provider writes rely on the single-instance assumption
  for the launcher UI; only `Preferences::save` holds the data-root lease.
- A malformed section fails strict reads of the whole document (the file is
  preserved for hand repair). This is stricter than the old independent JSON
  documents.
- Storage relocation no longer rebases the legacy `preferences.json` `java`
  string. Catalog-owned runtime paths (`[catalog]`) are still rebased; the
  user-provided custom Java path in `[preferences].game.java` is not.

## Verification

`just check` (formatting, workspace check, strict Clippy with `-D warnings`,
workspace tests) passes. Focused coverage: `block-launcher` 147 lib tests and
all 15 integration suites, `block-ui` full lib suite (327 tests) plus the new
`global_config` round-trip tests, and native onboarding captures after the
migration. The old JSON filenames only appear in intentional "legacy files are
ignored" regressions and historical notes.
