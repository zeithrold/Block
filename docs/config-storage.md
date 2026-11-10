# Config storage: `block-global.toml` and `block.toml`

Updated: 2026-11-09. These are **breaking** configuration changes. The previous
JSON configuration documents are no longer read or written, and no migration is
performed. Existing `preferences.json`, `accounts.json`, `providers.json`,
`storage.json` and root `catalog.json` files are left untouched on disk but are
ignored entirely; the launcher starts from defaults on the new files.
An existing TOML catalog with schema version 1 is different: it is rejected
explicitly and preserved, without migration, reset or removal of old sections.

## Data root

| Platform | Data root |
| --- | --- |
| macOS | `~/Library/Application Support/one.ztd.block/` |
| Windows | `%APPDATA%\Block\` |
| Linux | `~/.local/share/Block/` |

`BLOCK_DATA_DIR` still overrides the data root for the desktop app, and the
CLI's `--data-dir` option selects an external root. The platform application
folder holds the automatic global game library
(`crates/block-ui/src/setup/setup_storage.rs::global_application_directory`),
used by installed builds; portable/writable builds keep `.minecraft` beside the
application instead. Neither location is user-selectable.

## Files

Exactly two user-configuration documents exist:

### `block-global.toml` (block-ui owns it)

Written by `crates/block-ui/src/settings/global_config.rs`. Sections:

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

Written by `crates/block-launcher/src/persistence/config.rs`. Sections: `[storage]`
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
schema_version = 2

[catalog.snapshot]
instances = []
```

Nonempty catalogs use `[[catalog.snapshot.instances]]` tables. Each Instance
stores its settings directly, including resource directory, filesystem ownership
(`managed` or `registered`), actual game directory, data policy, targets and
selected target, runtime, memory, arguments, account reference, appearance,
server and window settings, compatibility and optional pack attachment.
There is no separate product Profile, mode or inheritance object.
Catalog documents, snapshots and Instances reject unknown fields. Schema 1 and
unsupported versions fail without rewriting `block.toml`; accounts and storage
settings are not reset.

`None` fields are omitted because TOML has no `null`; values are converted
through `serde_json` with null object members stripped on write.

## Game and cache boundaries

`resource_directory` identifies installation resources and version metadata.
`game_directory` is the actual Java working directory and `--gameDir`. New
Managed Instances default to both paths being `<library>/<instance-id>`, a
self-contained Isolated game root. Shared requires an explicit existing game
directory. Registered Instances point to their source resources and selected
actual game directory; registration creates no source directories and copies
or rewrites no Minecraft data. Changing policy or location does not migrate
files; an active accepted session prevents changing its directory or policy.
Deleting an Instance removes only its catalog entry.

Registration uses explicit launch-configuration evidence, not the presence of
`saves`, `mods` or `config` folders:

| Evidence | Shared choice | Isolated choice | Default |
| --- | --- | --- | --- |
| Shared at `G` | `G` | `<resource>/versions/<version-id>` | Shared |
| Isolated at `G` | `<resource>` | `G`, including supported external locations | Isolated |
| Unknown | `<resource>` | `<resource>/versions/<version-id>` | Isolated |

If the proposed Isolated path equals the known Shared path, the offered
Isolated location is its `.minecraft` child; unsafe aliases are rejected.
The Drawer displays the selected actual path before registration. Shared
retains a warning about shared saves/mods/config. Unknown warns that old data
may not appear at the isolated path; explicitly choosing Isolated acknowledges
the choice while retaining a reminder that files stayed in place. Both warnings
open the policy choices within the same Drawer. Scanning and registration do
not create the selected game directory.

The cache contains verified objects, metadata, views, managed runtimes and
`preparations/<instance-id>/`. Preparation locks and secret-free receipts live
there, with run-private natives under `runs/<run-id>/natives`. Standalone
recipe execution also captures logs under `runs/<run-id>/logs`; catalog session
logs remain under the locked actual game directory's `.block-runs/<run-id>`.
Preparation state is separate from a managed Instance's game root.

Cache relocation moves the `preparations` tree with the other cache trees.
Managed-game relocation moves the entire owned resource root, including mutable
game data inside it. An external selected game directory remains external.
Registered source directories are protected from relocation, even when their
names resemble managed IDs; relocation fails if a selected move tree would
contain a registered resource or game directory.

Public recipe compilation retains its Store-root-relative
`instances/<recipe-instance>/game` fallback. Catalog compilation omits creation
of that synthetic game directory and binds the Instance's locked actual path.
See [recipes](recipes.md) for the separate development interface.

## Deliberate exceptions (not user configuration)

- `storage-migration.json`: transient crash-recovery journal for an in-flight
  relocation. It is removed when the migration resolves and stays JSON on
  purpose.
- `catalog.lock`, `storage.lock`, `.block-game.lock`, `.block-session.json` and
  session reservation files: runtime locks/markers.
- Installation receipts, manifests, downloaded resources and preparation
  receipts: derived game/cache data, not user configuration.

## Known limitations

- Cross-process account/provider writes rely on the single-instance assumption
  for the launcher UI; only `Preferences::save` holds the data-root lease.
- A malformed section fails strict reads of the whole document (the file is
  preserved for hand repair). This is stricter than the old independent JSON
  documents.
- Storage relocation no longer rebases the legacy `preferences.json` `java`
  string. Catalog-owned runtime paths (`[catalog]`) are still rebased; the
  user-provided custom Java path in `[preferences].game.java` is not.

## Historical verification

The 2026-10-24 TOML migration record below predates the Instance-only catalog
schema and preparation-root changes. It is not current redesign acceptance;
the native acceptance matrix for that contract remains in progress.

At that migration snapshot, `just check` (formatting, workspace check, strict Clippy with `-D warnings`,
workspace tests) passes. Focused coverage: `block-launcher` 147 lib tests and
all 15 integration suites, `block-ui` full lib suite (327 tests) plus the new
`global_config` round-trip tests, and native onboarding captures after the
migration. The old JSON filenames only appear in intentional "legacy files are
ignored" regressions and historical notes.
