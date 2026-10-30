# Pilot release packages

Updated: 2026-10-24. Status: pilot (unsigned, built after checks or on
dispatch). Nothing is published to GitHub Releases.

## Workflow

`.github/workflows/desktop-build.yml` runs only after the
**Cross-platform checks** workflow succeeds for a push to `main` (via
`workflow_run`), and on manual dispatch. A manual dispatch also requires a
successful completed check run for the dispatched commit; a `gate` job fails
the workflow otherwise, so failing checks can never produce packages. Each package type is uploaded as its own artifact; a newer push cancels an
in-flight run for the same ref:

| Job | Runner | Artifacts |
| --- | --- | --- |
| macOS universal | `macos-15` | `block-macos-universal-dmg`, `-zip` (arm64 + x86_64 via `lipo`) |
| Windows amd64 | `windows-2022` | `block-windows-amd64-zip`, `-msi` |
| Windows arm64 | `windows-11-arm` | `block-windows-arm64-zip`, `-msi` |
| Linux amd64 | `ubuntu-24.04` | `block-linux-amd64-tar-gz`, `-deb`, `-appimage` |
| Linux arm64 | `ubuntu-24.04-arm` | `block-linux-arm64-tar-gz`, `-deb`, `-appimage` |

Each job also uploads a `-metadata` artifact with the package manifest,
`SHA256SUMS` and `UNSIGNED.txt` (14-day retention). Artifacts are deliberately
separated by package type, so downloading the MSI does not pull in the portable
ZIP, DEB, AppImage or metadata. There is no release job: the pilot stage keeps
every run independent.

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
