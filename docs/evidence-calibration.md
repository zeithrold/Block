# E01–E03 evidence calibration

2026-10-24. E01–E03 are complete as evidence-preparation tasks for Milestone III visual review. The Windows source-stamped rebuild, native fixture tests and follow-up captures passed after the user authorized source retrieval by all test hosts. This does not set Accepted or complete the broader Milestone III review.

## E01: correct pages, preserve the originals

The three incorrect light images were replaced at their existing paths. Both themes were refreshed together: four macOS images (`accounts` and the deliberately reused `account-offline-detail`) and two Windows Accounts images. Every replacement passed **process-owned page title + 1152×720 logical viewport + OCR of the seed name, Account details, Offline Mode and the local-identity explanation**. A title or filename alone cannot pass. The current window geometry differs from the historical 960×600 baseline and is recorded in each receipt.

- macOS uses the normal app with a disposable persisted `PageSeed` offline account and an empty catalog, without `--scenario`. Captures identify base commit `9e0f5c9` plus the working-source diff hash and executable hash. The captured source diff is retained at `target/e01-e03/macos-capture-source.diff`.
- Windows uses the normal app with the historical `BlockTester` offline seed copied into a task-owned empty-catalog store. Its refreshed captures use binary SHA-256 `701347713832adb660773ff5bdfa469595d130ecea4dd720a487d2dffd940ef7`, whose immutable `--build-info` records base commit `9e0f5c9` and source fingerprint `cfdc8371ea4957f4969dba55bf496a8fcea60f39f0193cf117fc1d651e5cd499`. All 1,251 source files were hash-checked before building. The capture build used an uncommitted working fix and the fingerprint identifies that exact snapshot; the host checkout's historical `df2db40` base is not substituted for binary identity. The preceding unstamped captures and their receipts were also retained when replaced.
- Originals, including the initially correct dark images, remain under `target/parity-shots/archive/e01-e03-before-20261024/`. The two macOS light originals retain their original Home-image hashes. Refreshed Accounts/list-detail images share an explicit reuse group and do not count as independent pages.
- Each PNG has a `.capture.json` sidecar containing hashes, timestamp, method, data state, dimensions, OCR observations, reduced-motion setting and focus/scroll limitations. Repeated replacement uses immutable image/receipt archives; `replaced_original` identifies the immediately preceding image, and `baseline_original` retains the historical baseline when different. Exact native focus handles are unknown; they are not reconstructed from image appearance.

## E02: audit semantics before comparing visuals

The versioned registry `docs/visual-review-cases.json` describes all 74 current scenario stems. Source-audited correspondence, different tasks/states and native-only states are explicit. No row claims independently verified full-state parity. The generator uses reference candidates from the registry, detects exact-byte reuse across platforms as well as within each board, validates receipt hashes and keeps missing evidence visible. The combined Markdown includes observed per-artifact receipt metadata separately from registry descriptions; unknown archive fields are not backfilled from current sources.

| Case | Correct comparison scope |
| --- | --- |
| `remove-confirmation` | Native game/profile removal versus prototype account removal; separate objects/consequences. |
| `provider-add` | Native creation versus the prototype's existing Sakura provider editor; no creation-state parity. |
| `wizard-loader-picker` | Native Vanilla versus archived React Fabric selection; container comparison only. |
| `wizard-loader-empty` | Native zero compatible builds versus React `zzzz` filtering an existing catalog. The latter is a candidate for `wizard-loader-search-empty`. |
| `auth-error` | Native expired request versus a closed prototype authorization popup; error hierarchy only. Block cannot reliably infer that an external browser was closed. |
| Welcome | Native Game → Account → Review versus combined prototype setup; compare task strategy. Prototype `welcome-about/settings/version-picker` recipes do not reach the corresponding native destinations. |

`accounts-list/accounts-detail` and `games-bundled/games-detail` reuse combined master/detail views. The board labels exact-byte duplicates rather than using the scenario count as independent experience coverage. Archive focus, scroll and capture commits remain unknown unless an individual receipt establishes them. Current-source references are not archive provenance.

## E03: review the actual failure branch

`home-error` now marks the selected profile's actual target `InstallStatus::Failed`. The real Home branch displays **Retry download**, confirmed in light/dark captures on macOS and Windows AMD64. A fixture helper resolves the profile's instance/target instead of assuming array order; its test also checks that an unrelated target stays Ready.

| Scenario | Fixture state / asserted visible result |
| --- | --- |
| `home-error` | Selected target Failed / Retry download + interruption message |
| `wizard-loader-search-empty` | Ready catalog with two builds, unmatched query / No matching builds |
| `wizard-loader-empty` | Ready catalog, zero builds, no search / No compatible builds |
| `wizard-loader-loading` | Loading catalog / Loading official builds |
| `wizard-loader-error` | Failed request / official-catalog failure explanation |

All five were freshly captured in light/dark on both macOS and Windows AMD64: **20 asserted scenario PNGs**, with the old images archived and new search-empty images added on each platform. Each native matrix now contains 144 images. Windows's former incomplete Home fixture is retained only in the archive; current review paths show the actual Failed target. Together with the six account-page captures, **26 current PNGs** have passed title, logical viewport, unique-content and receipt/hash validation. Deliberate macOS account list/detail aliases remain reused visual evidence.

Scenario screenshots force reduced motion and use synthetic identities/initials. Use the archived live-account skins for avatar review and the separate motion recordings/reports for motion review. These fixtures do not verify live authentication, downloads or game launch.

## Reproduce and verify

On macOS, run `python3 scripts/snap-accounts` for ordinary persisted-account pages, or `python3 scripts/snap-native --scenario home-error --scenario wizard-loader-empty --scenario wizard-loader-search-empty --scenario wizard-loader-loading --scenario wizard-loader-error`. Capture tools validate explicit contracts before publishing and archive the original images. `scenario_contracts.py` fails closed for unknown/uncontracted stems. `--no-build` is for a binary whose provenance has already been established by the caller.

On Windows, `scripts/capture-accounts-windows.ps1` runs in an interactive scheduled task against a disposable offline store and emits **candidate** PNG/JSON files. `scripts/capture-scenarios-windows.ps1` captures scenario contracts exported from `scenario_contracts.py` and requires a stamped binary. Transfer PowerShell scripts as UTF-8 with BOM for PowerShell 5.1. After downloading account candidates to the Mac, use `python3 scripts/verify-page-capture <candidate.png> <candidate.json> <destination.png>`; scenario candidates use the same validator with their local scenario contracts. Title, viewport and OCR checks must pass before replacing review images. The run's `target/e01-e03/collect-windows.py` additionally checks every image's source fingerprint and binary hash against the verified build receipt. The legacy host checkout revision is never substituted for immutable binary build identity.

Rebuild the semantic boards with `python3 scripts/parity-board --platform windows-amd64` and `python3 scripts/parity-board --platform macos-arm64`.

Observed validation: Rust formatting, `cargo clippy --locked -p block-ui --all-targets --all-features -- -D warnings`, two fixture tests on macOS and the same two on Windows AMD64, 14 focused Python checks, compiled Swift helper, six account-page assertions and twenty scenario assertions. The Windows executable reports the expected AMD64 architecture and immutable commit/fingerprint. Build/test logs, source verification and the compiled identity receipt are under `target/e01-e03/windows-amd64-stamped/`; the manifest is `target/e01-e03/source-manifest.json`. Screenshots remain local/ignored.

Review entry: `target/parity-shots/e01-e03.html`; machine receipts: `e01-e03-receipts.json`; semantic mapping: `m2-mapping.md` / `m2-semantic.json`.

## Source retrieval and verification

The user explicitly authorized all test hosts to retrieve current source on 2026-10-24, resolving the initial source-transfer approval gate. The Windows host retrieved the task-owned source archive/manifest and build script over the private test overlay; a three-file allowlisted temporary server was used and stopped afterward. The snapshot contains only tracked Cargo/crates/packaging source and licensing files, excluding `.env`, credentials, Git metadata and player data. The remote runner checked every source file against the manifest before building in `D:\block-e01-e03-src`, using the existing `D:\block-target` cache. Immutable build identity and all 12 Windows capture receipts match the staged manifest. The completed capture scheduled task was removed. No source-transfer or E01–E03 recapture gate remains; user visual acceptance and other-platform UI review remain separate work.
