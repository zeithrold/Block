# M3 UI refinement: first package

## Outcome and scope

Make game creation and account selection easier to understand without changing
launch execution. This package uses `d446d84` as the execution baseline and
extracts only the existing, independently useful UI work. Final visual and
product acceptance belongs to the user.

- The ordinary game wizard keeps the name first, then Version, Loader,
  OptiFine and Configuration as two columns of content-height cards. Values
  wrap instead of being truncated. First-run's existing square tiles are not
  redesigned.
- Review leads with identity and the consequence of Create. Effective custom
  Java, JVM arguments, server, automatic connection and game-files path remain
  visible even when inherited; unchanged defaults are folded. The ordinary
  wizard has one outer body scroll, with a fixed, reachable footer.
- The account selector is for choosing an account; removal remains in account
  management. Clear appears for nonempty input and returns focus to the input.
  Settings says “No unsaved changes” when clean.
- Preserve first-run Create as save-only and Finish as preparation, ordinary
  Create as save plus preparation, and Recommended as direct submission.
  Preserve drafts, Back, Cancel, Separated mode and OptiFine JAR validation.

Welcome's existing geometry, pointer behavior, reversible fill, circular hit
area and staged 170 ms out / hidden size snap / 190 ms in are inherited; this
package changes neither Welcome nor its motion. Do not alter the Home skeleton,
frame, Drawer, global palette or fonts. Downloads, Home and Queue experiments
are reference material, not part of this package. Security, Stop, concurrency
and resource-lifecycle changes are explicitly out of scope; report existing
issues rather than treating them as an open-ended prerequisite.

The [product decisions](next-tasks.md) and
[platform follow-up](onboarding-platform-followup.md) remain authoritative.
New design directions or any UI item requiring a new message, state owner or
execution guarantee must be raised with Main before implementation.

## Ownership and checks

Wizard and its tests, selector and Settings, and the shared `lib.rs` UI-layout
regression have separate owners during implementation. The coordinator owns
this plan, integration and verification; the integrated diff must contain no
other production files or unrelated tests.

- Exercise the real wizard and selector with focused
  `cargo test --locked -p block-ui` tests, then run the repository's `just check`
  on the integrated candidate. Check failures are recorded rather than hidden.
- On Linux ARM64, inspect the candidate's native UI in both themes, including
  default and long values, a short viewport, keyboard use and focus. Reuse
  existing scenario and GPUI facilities; old captures are references only.
  Native evidence does not establish Windows, macOS, live authentication or
  actual game-launch behavior.
- The Mac is source-edit/read-only for this package. Use an isolated remote
  source snapshot without `.env`, Git metadata, player data or `target/`; do
  not overwrite a dirty checkout. One builder owns any shared build target.
  Windows AMD64 and Linux AMD64 heavy work, if requested later, is serialized.
- No motion is changed, so no motion recording is required. Any future motion
  proposal needs Main's design decision and presented-frame, reversal and
  reduced-motion evidence rather than static images.

Freeze the exact Delta frontier and identify files, checks, native captures and
untested boundaries for Main's review. No push or merge to a shared branch is
authorized.
