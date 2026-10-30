# UI polish verification

## Candidate

Base: `b36681247e7c18686e10b3ef2d2eda2c1c7b5b2b`, with the UI polish
worktree changes. No crate relocation or Router migration is included; those
remain in [the architecture proposal](ui-architecture-proposal.md).

## Executed checks

Host: macOS ARM64.

- `just check`: passed formatting, workspace/all-target/all-feature type checks,
  strict Clippy and workspace tests. UI: 341 unit tests and 3 framework tests;
  launcher compatibility: 8 tests. The offline official manifest corpus and
  audit executable tests also passed.
- `cargo build --locked -p block-app --bin block`: passed.
- The Home queued-launch spacing regression failed without the spacing change
  and passed with it at 360 and 432 logical pixels.
- The selector rendering regression found the informational badge background in
  both account/game selectors in both themes.
- Avatar tests cover face/hat content-box cropping for classic and HD skins,
  missing-skin fallback, loading presentation and provider image insets.
- Drawer tests cover the title-bar gap, top corner paint/clip geometry,
  shortened gesture threshold and existing dismissal/focus behavior.
- Account tests cover collapsed defaults, expansion, alternative selection,
  Back, retained drafts and collapse without hiding the selected alternative.

## Native presentation

The final rebuilt executable was captured with:

```sh
python3 scripts/snap-native --no-build --no-board \
  --scenario accounts-detail --scenario accounts-offline-detail \
  --scenario auth-choose --scenario home-account-drawer \
  --scenario home-profile-drawer --scenario home-downloading
```

All 12 captures passed window-title, dimensions and content assertions. Images
are under ignored `target/parity-shots/native/`. Visual inspection confirmed
clean management avatars, Microsoft-first collapsed choices, colored selector
badges, rounded Drawer tops and separation between launch progress and the
queued-launch action.

Independent source reviews covered visible UI/state behavior, event and motion
preservation, and ARM64 override consolidation. No confirmed findings remained
in those scopes. Review did not substitute for native interaction or gameplay.

## Limitations

- Native screenshots use synthetic identities and initials, not a live skin or
  authenticated account. Skin cropping is covered by implementation-level
  layout tests; real-skin pixel sampling and live Microsoft sign-in were not
  exercised.
- Native pointer interaction is blocked on this host: both
  `AXIsProcessTrusted()` and `CGPreflightPostEventAccess()` return false. The
  disclosure's native expand/Back interaction remains unverified; GPUI control
  interaction tests passed. No system permissions were changed.
- The supported fixed-size native window does not enter the shortened
  full-height Drawer case. Short-window clipping, gap and gesture behavior are
  covered by GPUI layout/paint tests rather than a native short-window capture.
- No Windows/Linux native run or cross-platform matrix was performed. The
  ARM64 override consolidation preserves planning behavior under the workspace
  tests; this is not new Minecraft gameplay compatibility evidence.
- The dependency `block` reports Cargo's future-incompatibility notice; the
  strict project Clippy gate itself passes.
