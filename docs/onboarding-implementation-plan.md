# First-run onboarding implementation

Date: 2026-10-24 (Asia/Taipei). Baseline: `f117471`.

The user authorized this entire implementation after reviewing the onboarding
proposal and interactive import Drawer. Luna subagents may handle independent
work. This document is the execution plan and acceptance record; implementation,
automated validation, native review and user acceptance remain separate.

## Agreed behavior

- Begin at the fitted Home size (preferred 360 × 720), with bold `Welcome Back`,
  `Add an account and set up a game.`, a 160 px circular button with a solid Play
  icon, and a small `Skip setup` action at the bottom.
  Hide application navigation and Home business controls; preserve window drag,
  minimize and close. The Play button grows slightly with a small shadow and a
  clipped color-inversion wave originating at the pointer's entry point. Reverse
  continuously on exit; respect the existing reduced-motion policy.
- Until completion or explicit skip, navigation remains inside onboarding and
  its invoked tasks. Closing before completion resumes onboarding. Completion
  and skip are durable; upgrades do not automatically replay it.
- Detect both an adjacent `.minecraft` and Minecraft Launcher installations.
  Offer the two sources together in one bottom Drawer, including empty states.
  Count identifiable launch configurations, not worlds or resource directories.
- For one detected game, show its name/version/loader. For several, show the
  count and a `View games` link. That link opens a checkbox list by horizontal
  navigation inside the same Drawer. Its header changes text without horizontal
  travel; keep the footer outside scrolling content and the bottom edge fixed.
  Animate content-dependent height, cap to the available window and scroll lists.
  Preserve selections when returning or switching sources.
- Existing games are **copied** into independent Block directories. Never move,
  edit or delete source data, import launcher/account credentials, follow unsafe
  links or claim unsupported metadata is supported. Each successful imported
  game is saved independently; failures can be retried without duplicating prior
  successes. Cancellation retains completed imports and removes only unfinished
  destination work owned by this operation. Check source/destination overlap.
- Use the fitted Manage size (preferred 1152 × 720) for the setup overview and
  tasks. Show `Account` and `Game` task cards. Continue enters an unfinished task,
  preferring Account when both are incomplete; allow either card to be chosen.
  Imported games satisfy Game. Both complete changes Continue to Finish.
- Every account/game Wizard saves when it completes. Returning, cancelling the
  next task, skipping or restarting must retain completed work. Finish records
  onboarding completion and returns Home; it does not wait for downloads and
  does not imply that an unfinished installation is playable.
- Reuse the existing account/game Wizard content and validation for onboarding
  and normal management. In onboarding, Microsoft is the primary account path;
  Authlib and Offline belong under Other sign-in options. Game setup offers
  Recommended setup (stable Vanilla, automatic compatible Java, pinned version)
  and Custom setup (the existing detailed creation flow).
- Wizard bodies enter from below the fixed 56 px title area. Keep internal
  entrance travel small and staggered; steps use bounded horizontal motion and
  reverse direction for Back. Replace the title-area menu with a down arrow that
  cancels the current task and returns to its invoking page. Remove global
  Back to Home buttons; retain step Back. Disable the task exit only during a
  short noninterruptible save; keep normal waits cancellable.
- Installed/system/unwritable/temporary application locations default to the
  platform user-data directory. A writable portable distribution may default to
  adjacent `.minecraft`. Expose the actual game destination under More options,
  using the shared task container without a Step rail. Default location changes
  do not move existing games. Keep account secrets in the existing secure store.
- Use literal task copy without slogans. Detailed account/game Wizard editorial
  refinements remain a subsequent pass after this onboarding increment.

## Execution order

- [x] Record the approved scope and validation plan before implementation.
- [x] Implement bounded discovery and safe, cancellable copy import with fixture
  tests for both sources, loaders, overlap, credentials, partial failure/retry and
  source preservation. (Independent Luna work.)
- [x] Replace the first-run presentation and completion state with Welcome plus
  a durable Account/Game overview, shared Wizards and per-Wizard save semantics.
- [x] Integrate import discovery, source/list Drawer states, batch progress,
  cancellation and results with the UI engine and current catalog.
- [x] Implement the reusable Wizard presentation/title-area controls and motion,
  and the circular welcome button interaction.
- [x] Integrate storage defaults and More options, preserve existing settings
  and account/game operations, and update obsolete tests and documentation.
- [x] Run focused meaningful regressions, then the repository verification gate.
- [x] Build and inspect native Welcome, import, overview and Wizard states in
  both themes; record what was actually exercised and remaining platform limits.

## Verification and acceptance

Use temporary fixture directories and synthetic accounts for automated checks;
never inspect or copy `.env` or live account-bearing launcher files into evidence.
Only explicit source-selection actions can start copies. Discovery is read-only.
Check counts and filtering, a real filesystem copy with unchanged source hashes,
safe path handling, per-game publication and interruption, saved-account/game
restart behavior, completed/skipped first-run persistence, unfinished-task routing,
late-result isolation, window sizes and return destinations, Drawer height/list
behavior, title-area controls, continuous/reduced motion and download state truth.

Required local gate: `cargo fmt --all -- --check`, workspace all-target/all-feature
check, strict Clippy, workspace tests and native build. Reuse existing native
scenario/capture tooling where possible. Cross-platform compilation/CI and real
interactive platform acceptance must be reported separately; do not mark user
acceptance complete from tests or screenshots.

## Initial implementation and validation

- Planning: recorded before code changes. Existing working tree was clean.
- UI and backend integration are implemented. The final review extended adjacent
  multi-version discovery, source-root metadata resolution and precise copy
  filtering. Current year-based versions and legacy versions are recognized;
  corrupt latest-version aliases cannot recurse. Unknown launch components are
  shown as unsupported rather than silently converted to Vanilla.
- Copying preserves mutable game data, including worlds whose names contain
  words such as `accounts` or `secrets`, and mods such as `token-helper.jar`.
  Credentials, `.env` files and unrelated root directories are excluded by
  explicit rules. Fixture tests cover unchanged source bytes, cancellation,
  overlap, duplicate profiles and custom game-directory metadata.
- The final workspace gate passed **627 tests across 39 suites**, with no failed
  or ignored tests. Formatting, workspace all-target/all-feature check, strict
  Clippy (`-D warnings`) and the native `block` build passed. This includes
  independent save/restart routing, import selection refresh, filesystem copy
  filtering, real hover enter/leave dispatch, outgoing click-barrier retirement
  and circular reveal geometry regressions.
- The final native build was captured in nine modern `setup-*` scenarios in
  both themes (18 PNGs), and all receipt hashes match the final Rust sources
  and binary.
  All required page labels passed OCR assertions. Welcome/import windows were
  360 × 720; overview/account/game/storage windows were 1152 × 720. Synthetic
  accounts, games and paths are explicit; no live authentication or game-launch
  result is implied. Screenshot receipts contain the binary hash and an all-Rust
  source-tree hash that includes new files.
- Evidence is local and ignored: `target/onboarding-validation/`. The initial
  sandboxed supervisor tests failed because the child could not start; the
  full tests passed when rerun with approved local-process access.
- After the user unlocked macOS, three foreground fixture runs passed, with
  **1,038 complete compositor frames** captured through ScreenCaptureKit's
  `desktopIndependentWindow` filter for the exact owned window. These replace
  the earlier locked-desktop attempts as accepted native evidence. Static
  reduced-motion screenshots remain separate from animated captures.
- Native input checks cover source/list navigation, changing four selections to
  three and retaining them on return/reopen; Wizard cancellation and subsequent
  Continue; Microsoft-first secondary-option disclosure; More options entry and
  cancellation; Custom setup, Review and Back with the draft retained; and quick
  welcome hover reversal/reentry from different sides. Window position and size
  remain unchanged during each run, all post-warmup paints are active, and page
  motion settles without retaining an outgoing input barrier.
- Frame analysis observes the Drawer header at a constant horizontal position
  (within a 2 px OCR tolerance), while its list moves 150 px across four sampled
  intermediate frames. The Wizard sidebar enters vertically to its fixed 56 px
  content origin. The welcome fill has 18 observed intermediate states; pixel
  checks verify inversion inside the circle, no spill into square corners and
  white fill restored after leaving the control.
- Physical checks found and fixed missing first-paint acknowledgement on task
  page transitions, an animation request made from an input callback, use of a
  window-exit event for element hover, and rectangular clipping of a custom
  circular fill. They also corrected synthetic scenario release timestamps so
  modern fixture versions are not classified as LWJGL 2-era versions. The game
  review now uses literal task copy and explains that first-run creation saves
  before Finish begins resource preparation.
- Native interaction receipts, frame measurements and three MP4s are in
  `target/onboarding-validation/interactions-unlocked/`; their provenance matches
  the final Rust sources and binary. The capture contains synthetic data and
  does not assert a live authentication, installation or game-launch result.
- Windows/Linux interactive validation and live Microsoft sign-in have not been
  repeated for this increment. Existing provider behavior is reused; automated
  account save tests use an isolated offline identity.
- Changes remain local on `main`; no commit, push or new CI result is implied.
- User acceptance: pending.
- [x] Repeat foreground source/list/back and Wizard-exit checks after macOS is unlocked.

## Welcome refinement after user review — 2026-10-24

- Move `Skip setup` out of the centered content into a bottom action: 12 px
  normal-weight muted text, a 28 px control and a 24 px bottom inset.
- Increase the base circle from 80 to 112 px. Retain the fixed hit area,
  proportional hover growth, shadow and continuous inversion/reversal.
- Use a solid Play glyph. Clip the inverted glyph as a filled polygon so its
  shape remains identical throughout the reveal.
- This increment passes eight focused Welcome regressions, strict block-ui
  all-target/all-feature Clippy, formatting and native build. Native light/dark
  screenshots and a foreground hover run (216 compositor frames) confirm the
  circle, bottom action, solid white/black glyph, circular mask and exit restore.
  Receipts match the current source tree and binary. Updated evidence is in
  `target/onboarding-validation/welcome-refinement/`; the full-workspace and
  broader interaction results above belong to the preceding implementation gate.

## Welcome layout after screenshot review — 2026-10-24

- Change the main heading to `Welcome Back` with bold weight. Use a 6 px
  title/subtitle gap and a 32 px gap above the button.
- Increase the base circle from 112 to 160 px and the solid icon from 32 to
  48 px. Anchor the button independently at 56% of the content area's height;
  in the 360 × 720 window its center is (180, 428). Text sits above this anchor
  rather than determining the button's position. Keep Skip setup at the bottom.
- Eight focused Welcome regressions, strict block-ui Clippy, native build and
  both theme captures pass. Foreground hover, fast reversal/reentry, circular
  clipping and solid-icon inversion pass with 232 compositor frames. Evidence
  is in `target/onboarding-validation/welcome-layout/`.
- Open a new working desktop preview of this Welcome page. The preceding
  preview's in-progress setup remains preserved in its original data directory.

## Exit-point revision and four-platform continuation — 2026-10-24

- Shrink toward the pointer's circle exit crossing, including off-axis paths;
  retain the currently visible fill during fast reversals and reentry.
- Move the central anchor from 56% to 52% of content height, about 27 px upward
  at the preferred Home size. Keep the 160 px button and existing typography.
- Continue Windows AMD64/ARM64 and Linux AMD64/ARM64 native gates and desktop
  checks with Luna workers; serialize the two AMD64 phases. Temporary Mac
  activity inhibition is authorized for the run.
- Use the final frozen Round 2 snapshot after correcting the compositor-observed
  clipping flicker. See [platform progress and evidence](onboarding-platform-followup.md)
  for source identity, current results and validation boundaries.
- The final native gates pass on all five hosts: 631 macOS tests and 629 on each
  Windows/Linux architecture, with no failed or ignored tests. Desktop replay
  verifies the welcome exit/reentry, source selection and Wizard flows within
  the recorded synthetic scope. Linux normal-engine checks additionally cover
  Skip/restart, storage persistence and three-game batch copying without changing
  source bytes. The Linux AMD64 ENOSPC attempt, approved cache recovery and final
  successful retry are retained separately.
- Temporary Mac activity inhibition has ended; the updated normal desktop
  preview remains available with isolated review data. Visual acceptance and the
  subsequent detailed account/game Wizard polish remain pending.

## Welcome icon and hover refinement after user review — 2026-10-24

- Replace the hand-drawn Play triangle with the solid Lucide `play` glyph from
  the bundled 24 × 24 outline. The glyph is sampled into one convex polygon so
  the inverted hover reveal keeps a single, identical silhouette.
- Raise the hover growth from 5% to 8% of the 160 px circle and the hover shadow
  opacity from 0.12 to 0.22. The fixed hit area, the 48 px icon scale and the
  continuous inversion/reversal behavior are unchanged.
- Eleven focused Welcome regressions and strict block-ui Clippy pass, and the
  native binary rebuilds. Isolated 360 × 720 light/dark captures measure the
  resting circle at 160 px, the hover disc at 171 px (5% growth previously
  measured 166 px), a 32 × 36 px solid glyph whose rest and inverted silhouettes
  match, a deeper hover shadow and an exact pixel restore after the pointer
  leaves. Evidence is in
  `target/onboarding-validation/welcome-lucide-hover/`.
- Visual and user acceptance remain pending; no other page changed.

## Welcome start without an empty import flash — 2026-10-24

- The Welcome start button now scans for importable games before presenting the
  source Drawer. The Drawer opens only when the report contains games; an empty
  report continues straight to the Account/Game overview. Explicit imports from
  the overview still open the Drawer immediately, and a missing data directory
  still surfaces its error in the Drawer.
- On a host without an adjacent `.minecraft` or an official launcher folder the
  empty scan previously flashed the Drawer for about 150 ms before the overview.
  The recorded replacement shows no Drawer frame: the normal Welcome → overview
  page transition begins after the report resolves.
- Three focused regressions cover the deferred empty scan, the deferred populated
  scan and the immediate explicit import. All 314 block-ui tests, strict block-ui
  Clippy, formatting and the native rebuild pass. Frame evidence is in
  `target/onboarding-validation/welcome-lucide-hover/repro-frames-fixed/`.
- User confirmation of the revised flow remains pending.

## Onboarding layout and storage review — 2026-10-24

- One shared heading treatment now drives every wizard, editor and management
  page: bold 20 px title, muted subtitle and a 6 px title/subtitle gap
  (`patterns::page_heading` / `page_title`). Welcome keeps its larger 2xl title.
- Overview heading copy is `Two steps and you're ready`; the Account/Game card
  titles are larger and bold with a 6 px gap to their descriptions. Cards are
  auto-height with `p_6` padding, two equal halves of the same 620 px column as
  the heading, and equal height with top-aligned content.
- The Overview footer is one row: More options left, Continue/Finish right. The
  `Next: …` hint and the Overview Skip setup button are removed (Welcome's
  bottom Skip setup remains). A chrome back button returns to the Home-sized
  Welcome page without deleting saved setup progress; the existing top-left
  close-setup chevron now shares the Ghost window-control style (no border,
  32×32, 16 px muted icon) with minimize/close.
- Game storage (More options) is two mutually exclusive read-only cards:
  Global storage (fixed app-data `.minecraft`, no folder picker) and Local
  storage (executable-adjacent `.minecraft`, path display plus Choose Folder).
  Save is the primary action in the pinned footer and persists only the game
  library path through the new `block.toml` `[storage]` section.
- The config migration that backs these screens is documented separately in
  [config storage](config-storage.md): `block-global.toml` plus `block.toml`,
  the new platform data root and the deliberate no-migration policy.
- Welcome and the Account/Game overview now share the standard horizontal
  slide/fade when the play button opens the overview and when the chrome back
  button returns to Welcome. The page key includes the first-run stage, the
  incoming overview slides while the window frame morphs, and leaving slides
  the outgoing overview away instead of only clipping it with the shrinking
  window. A navigation regression asserts both scene poses and the rendered
  `navigation-current-0` / `navigation-outgoing-0` offsets.
- Verification: `just check` (formatting, workspace check, strict Clippy,
  workspace tests) passes, including the new first-run regressions for the back
  action, single-row footer, read-only mutually exclusive storage cards and
  `Locations::save`. Native captures of the Overview (light/dark), Game storage
  and Create game screens are in
  `target/onboarding-validation/ui-refinement/`. User acceptance remains
  pending.
