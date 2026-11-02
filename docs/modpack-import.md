# Local modpack import

Choose **Add Game**, then **Import pack**, or use the pack shortcut in the
existing game-directory import drawer. Block inspects the selected archive
before creating a new Instance. Review the original pack name, game or Instance
name, target and optional files; required files can be expanded. The new name
does not change the original pack name. The first Profile uses the current
defaults; its launch settings can be changed after import.

## Supported inputs

| Input | Behavior |
| --- | --- |
| Modrinth `.mrpack` v1 | Uses the declared Minecraft and Loader versions, client file requirements, optional choices and download sources. Verifies size, SHA-1 and SHA-512, then applies common overrides followed by client overrides. |
| CurseForge manifest ZIP v1 | Reads the target, overrides and project/file references. Required or selected ID-only references remain blocked because they do not provide verifiable file identities. |
| Prism/MultiMC standard instance ZIP | Maps supported components and copies the identified game-content root. Unsupported components, patches and nonempty command hooks are rejected rather than silently discarded. |
| Plain Minecraft ZIP | Detects a game-content root at the archive root, a named Minecraft directory or one outer wrapper. Uses reliable version metadata where available; uncertain Minecraft/Loader targets require explicit selection. Ambiguous roots are rejected. |

Standard format markers take precedence over Plain detection. A broken or
unsupported marked pack does not become a Plain archive. The importer does not
infer a target from an archive filename or a mod filename.

CurseForge import uses no API key or automatic CF file resolution. The project
link opens a project page, not a guaranteed exact file page. Selecting an entry
or supplying a similarly named file cannot verify a project/file ID. A package
whose required references cannot be verified cannot finish installation.

Prism settings use Block's defaults rather than copying machine-local Java paths
or arbitrary launch settings. Importing a format does not establish that every
Minecraft version, Loader and CPU combination runs; see [current platform
status](STATUS.md).

## Installation and recovery

The shared launcher core owns inspection, installation and launch checks.
Archives are read-only inputs. ZIP paths, links, duplicate entries, conflicting
destinations and extraction bounds are checked before publishing content.
Installation does not execute archive scripts or copy launcher credentials.

Pack metadata and the original content baseline live under the Instance's fixed
`.block-pack/` directory. Download URLs remain transient. The baseline is
separate from the Profile's writable game directory: initial Profile preparation
uses it without cloning another Profile's played worlds or configuration.
Existing game data is not overwritten during later preparation or policy changes.
Ordinary Repair continues to repair the base game and Loader, not reset pack
configuration or worlds.

An unfinished pack remains Pending and is rejected by the core launch entry
point. Ready requires the selected files, overlays and base target to complete.
Stopping leaves the installation unfinished. Leaving during installation asks
for confirmation before stopping it.
Retry reuses the same Instance. After creation, its saved target, name and
optional choices are locked; retry restores those choices rather than silently
changing the installation. To make different choices, start a new import.

Keep the original archive for recovery. After restarting Block or moving the
archive, select it again to resume; its declarations must match the saved pack.
The already-snapshotted overrides remain the baseline for that installation.

## Verification

The Game Wizard integration passed `just check` on macOS ARM64, including
534 UI tests and 3 GPUI framework tests, plus strict UI Clippy with
`--features tracing/log`. It also passed the complete
`scripts/verify-platform.py --platform linux-arm64` sequence on Debian 13 ARM64,
including 238 launcher tests, 534 UI tests, 3 GPUI framework tests and the desktop
build. Windows AMD64 passed strict UI Clippy with `--features tracing/log`,
533 UI tests and the desktop build.
Automated tests exercise real Catalog, Store, Library and launch-plan
preparation paths using synthetic archives and local HTTP data.

Native Game Wizard checks installed Simply Optimized `1.20.1-12.3`
(Minecraft `1.20.1`, Fabric `0.16.0`) to Ready on Windows AMD64 and Linux ARM64.
The Windows walkthrough covered source choice, the system picker, failed-file
recovery, renaming, the required-file list, installation and completion.
The Linux walkthrough additionally covered returning to source choice without
losing the name, stopping and resuming the same Pending Instance.
On both hosts, installation completed while the stop-and-leave dialog was open:
the dialog closed, Complete remained visible, and Space activated the live Done
button to open the resulting Instance.

Linux ARM64 Japanese dark-theme checks at 360×540 covered the compact source
page, wrapped CurseForge recovery actions and Plain target selectors.
Tab dismissed the version popup without selecting a value and moved to the
Loader control; Shift+Tab returned to the version trigger, Space reopened it,
and Escape closed only the popup. These supplemental CF and Plain checks used
synthetic archives, not successful third-party installations.

The earlier native import observations used KDE Wayland, an Xwayland Block window and an
isolated D-Bus session with the GTK file-chooser portal. The system's portal
configuration was not changed. Native checks cover real archive selection,
picker cancellation/reopening, CF missing-file blocking, Modrinth optional
selection and source fallback, explicit Plain target selection, and failed
download recovery with the same Instance and saved optional selection.
Prism inspection accepted the standard fixture and rejected a disabled Loader
component. Long version lists revealed the active item during keyboard traversal.
Synthetic Vanilla 1.20.1 installations used the normal official base-game
installation path; their test payloads are not playable mods.
Japanese dark-theme checks at 360×540 confirmed that a 64-character CJK pack
name stays in scrollable details, the instance name remains editable and the
footer remains reachable. Renaming the Instance preserves the full original
pack name.

Generate the small original fixtures with:

```sh
python3 scripts/modpack-fixtures.py /tmp/block-modpack-fixtures --port 8123
python3 -m http.server 8123 --bind 127.0.0.1 \
  --directory /tmp/block-modpack-fixtures/http
```

The generator prints each input's expected behavior and the retry-file command.
Use a new or empty output directory. The operator owns and stops the loopback
server.

### Evidence limits

No real third-party modpack gameplay, new Loader/platform compatibility matrix,
Microsoft login or macOS native import session was exercised. The walkthroughs
do not establish a full keyboard-only import or native IME acceptance. Linux
file-chooser interaction was verified with the isolated GTK portal, not automated
through the host KDE portal. Automated parsing, installation or launch-plan
success is not evidence of entering a world, exiting normally or relaunching.
The existing shared field-error row can clip long validation copy at compact
widths; wrapping that row belongs to the shared field control follow-up.
