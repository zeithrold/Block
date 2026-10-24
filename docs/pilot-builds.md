# Pilot release packages

Updated: 2026-10-24. Status: pilot (unsigned, manually dispatched). Nothing is
published to GitHub Releases.

## Workflow

`.github/workflows/pilot-build.yml` runs manually (`workflow_dispatch`) and
produces one downloadable artifact set per platform per run:

| Job | Runner | Artifacts |
| --- | --- | --- |
| macOS universal | `macos-15` | `.app`, `.zip`, `.dmg` (arm64 + x86_64 via `lipo`) |
| Windows amd64 | `windows-2022` | portable `.zip`, `.msi` |
| Windows arm64 | `windows-11-arm` | portable `.zip`, `.msi` |
| Linux amd64 | `ubuntu-24.04` | `.tar.gz`, `.deb`, `.AppImage` |
| Linux arm64 | `ubuntu-24.04-arm` | `.tar.gz`, `.deb`, `.AppImage` |

Each job writes `SHA256SUMS` and `UNSIGNED.txt` next to the packages and
uploads them with `actions/upload-artifact` (14-day retention). There is no
release job: the pilot stage deliberately keeps every run independent.

The macOS artifact is **universal only**. The workflow builds both Apple
architectures and combines them with `lipo`; no per-architecture macOS package
is published. `macos-15-intel` is part of the normal check matrix
(`macos-amd64`) so tests run natively on Intel as well; failures there are
product bugs, not accepted differences.

## Build profile

`Cargo.toml` enables a tuned `[profile.release]`: `strip = "symbols"`,
`lto = "fat"`, `codegen-units = 1`. `panic = "abort"` and `opt-level = "s"` are
deliberately not set because crash reporting and animation quality outrank the
remaining size reduction (see [binary size](binary-size.md)).

Release builds receive `BLOCK_SOURCE_COMMIT` from the workflow, so packaged
`--build-info` output reports the generating commit. `BLOCK_SOURCE_SHA256` is
not populated by the pilot workflow; a source archive digest can be added
later without changing the packages.

## Signing and platform warnings

No package is code-signed or notarized:

- macOS: Gatekeeper blocks the first launch; users must right-click → Open.
  The DMG itself is unsigned.
- Windows: SmartScreen warns on first run of the portable executable and the
  MSI; there is no Authenticode publisher.
- Linux: no repository signing. Verify against `SHA256SUMS`; the `.deb` is a
  plain local install and the AppImage is unsigned.

Signing is intentionally out of scope until release preparation; see
[config and release notes](next-tasks.md) for the v0 gate.

## Known pilot limits

- MSI authoring uses WiX v4 (`dotnet tool`s installed during the workflow) and
  has not yet been validated on a clean Windows host in this repository run.
- AppImage bundling relies on `appimagetool`'s continuous release and on host
  shared libraries; it does not bundle GTK/X11/Wayland/Vulkan.
- The `.deb` dependency list is a pragmatic runtime list; distro-specific
  package versions may still need adjustment.
- Archives are deterministic for the same input, but separately compiled
  binaries are not reproducible.
- Complete transitive dependency notices remain a release prerequisite; the
  pilots ship the notices that packaging already collects.
