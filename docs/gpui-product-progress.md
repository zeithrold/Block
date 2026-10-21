# GPUI product migration

Reference: the current working files in sibling `Frontend/Block-UI`, including its uncommitted interface work. The reference is a browser prototype: authentication, installation and launch progress are simulated there. Native Block keeps its existing real launcher, protected Microsoft credentials, verified installation and detached session supervision.

The goal remains active. This ledger distinguishes delivered slices from remaining parity work; a passing build is not graphical acceptance.

## Stage 1: native product shell and library model

- Portrait Home (432×768 content) and management windows (960×600), real Account/Profile selection, native Play/Stop and download state entry.
- Embedded unchanged `Block-Icon/icon-v4` brand assets; source SHA-256 receipts live beside the assets.
- Home / Games / Profiles / Accounts / Settings / About navigation and native app-window sizing.
- Light, Dark, System appearance; reduced-motion switch; settings draft cancellation and atomic saved preferences.
- Independent page and drawer entrance animation keys, 460 ms easing, keyboard buttons, Escape dismissal and focus restoration. Full reference exit transitions, detail and wizard scopes remain subsequent work.
- New libraries default to Bundled (one Instance and one Profile). Existing catalogs missing a mode retain Separated behavior. An explicitly confirmed, one-way catalog transaction enables multiple Profiles. Deleting a bundled game removes metadata while preserving all files.
- Windows crash diagnostics now hold a directory identity handle rather than comparing a reusable creation timestamp.

Local macOS ARM64 `just check` completed with exit 0: format, all-target/all-feature type checks, strict Clippy, and 162 passing tests. GitHub CI run [39543608491](https://github.com/zeithrold/Block/actions/runs/39543608491) passed Windows AMD64, Linux AMD64 and macOS ARM64 for commit `574fe53`. Independent Linux ARM64 native verification also passed 162 workspace tests, five vendor tests/doctests, strict Clippy and an aarch64 ELF desktop build. Native macOS visual inspection used an isolated empty test library and captured the actual 432×768 rendered Home. No player credentials or worlds were copied.

## Stage 2: first run, game creation and package identity

- First launch opens a friendly three-page 960×600 guide: Welcome, Appearance, Next steps. Completion and theme are saved atomically; restoring defaults preserves completion.
- The native window uses custom controls and a drag region, with no system titlebar. A platform may fall back to the system titlebar if real window operation proves unreliable; this must be tested per host.
- Product website: `https://block.ztd.one`; application/bundle ID: `one.ztd.block`. Existing user-data and credential locations are preserved.
- New game uses Setup → Review; version, loader and installation-location subpages preserve local drafts, cancel creates nothing, and final confirmation calls the real installer. Bundled game names match their first Profile.
- Reproducible packaging script creates a macOS `.app`, Linux prefix archive, or Windows portable ZIP. macOS uses unchanged original 1024 px artwork from Block-Icon. Packaging verifies binary architecture and emits file/hash receipts.
- macOS full `just check`: exit 0, 165 tests. Native QA confirmed a 960×600 guide, working custom drag (position moved from 427/168 to 500/204), immediate dark preview, completion persistence, transition to 432×768 Home native close, and fresh-process reopening directly into dark portrait Home. Screenshot capture initially paused while the desktop was locked, then resumed after user unlock. Automated input initially used stale coordinates; the live accessibility bounds resolved that test-tool issue.
- The macOS package plist, executable help, file hashes and deterministic archive checks passed. Other package formats and all-platform window/animation acceptance remain separate pending gates.

## Stage 3: reference-based setup and effective preferences

The six reference routes were captured directly from the running Block-UI development server. All returned HTTP 200 with no page errors. The shared native wizard now follows its 32% step rail, four square component tiles, independent right-pane scrolling and fixed 72 px footer. A native screenshot exposed a primary-action alignment issue; the footer context was corrected and a layout assertion now guards the right edge.

The user rejected the early native guide. The newly redrawn `/welcome` reference was captured from the development server (HTTP 200, no page errors). Its dense Instance/Account design is a reference, not a specification to copy wholesale. Native first launch now has its own 8:5 brand composition and **Your game → Your account → Ready to go** flow: one task per page, initial name and version only, optional loader/folder disclosure, and a three-row final summary. Java, memory and file paths are not required beginner decisions. The compact layout removes the brand side panel and keeps the footer actionable.

Microsoft sign-in remains real and cancellable. “I’ll sign in later” advances only the account step, while “Skip setup” finishes the complete guide without creating a game. Neither creates an identity or bypasses the Play gate. “Create and download” calls the real installer, enters Home after catalog creation and continues background downloads. Home offers Download/Retry download for incomplete/failed targets, without requiring an account or creating another instance. Returning from authentication keeps the draft; a failed preference save can retry completion while the same game downloads. Native macOS screenshots verified the new composition and exposed a Microsoft mark sizing issue, which was corrected before the final build. These changes supersede the earlier informational guide and rejected shared-rail first run.

Game defaults (Java, memory, JVM arguments, resolution, fullscreen and snapshot visibility) are validated and applied only to new games. Downloads use actual shared concurrency, retry, aggregate object-body rate limits and System/Direct/HTTP(S) proxy policies; local HTTP tests verify behavior. Settings keep a draft, preserve it on validation failure, show the specific error and open the relevant section. Storage relocation, SOCKS proxies and unsupported loaders are not offered as functioning features.

Window fallback follows the user's permission: **Windows and Linux X11 use system decorations**, after native custom dragging failed. macOS keeps its custom controls and verified drag behavior. Native Wayland keeps the custom path pending separate compositor acceptance. Windows also builds as a GUI subsystem application to avoid an unwanted console window.

`--build-info` exposes compile-time product, platform and source identity. The platform verifier refreshes only source-file mtimes, confirms unchanged bytes, runs checks with a source stamp, executes the binary to compare its compiled identity, and rejects source changes during verification. This protects shared Cargo caches from stale source-archive timestamps. Manual unstamped builds report null source identity.

Local Stage 3 `just check` passed with exit 0 and 183 tests after the first-run redesign, effective preference wiring and Home download recovery. Regression coverage includes compact footer bounds, draft persistence, preventing exit during catalog creation, completion-save retry during background download, and retrying a failed download without a second game. New-source graphical acceptance and CI results are tracked separately.

Stage 2 CI [39545112536](https://github.com/zeithrold/Block/actions/runs/39545112536) passed all three jobs. Separate Windows verification of Stage 1 passed 152 platform-specific tests and a native build. Stage 2 GUI acceptance passed initial/portrait sizing, minimize/restore/close, first-run completion and reopening on Windows AMD64 and Linux ARM64. Windows motion frames confirmed fade/slide progression. Both hosts' custom drag failures remain recorded; fallback acceptance belongs to the newer build. Linux ARM64 used XWayland and CPU llvmpipe; its short desktop clamped Home height to 756 px. These older receipts do not establish acceptance of the newer Account/Game/Review flow.


## Short-display follow-up

Independent Stage 3 graphical acceptance found that the 432×768 Home client plus system decorations could exceed an Xfce 1280×800 desktop. Home startup, navigation and onboarding completion now fit the portrait size to the current display and window position, reserving space for desktop panels and decorations. Large screens retain 432×768; the minimum usable size remains 360×540. Geometry and rendered-control regression tests cover the measured short display, negative monitor coordinates and both navigation/completion paths. Local full `just check` passed 189 tests with strict Clippy. Native verification of the follow-up is recorded separately from the earlier full source-stamped receipts.

GPUI exposes full display bounds rather than the desktop work area, and cannot reposition a window through its public API. The policy therefore uses conservative margins; an unusually small display or an already off-screen window can still require manual repositioning.

## Remaining slices

1. Searchable game browser, appearance and launch detail subpages; separated-profile creation flow.
2. Selector search, arrow keys and selection feedback; avatar fallback; distinct downloading/running/stopping states; complete enter/exit, detail, wizard and overlay animations.
3. Storage paths and profile appearance; expose each setting only when its backend behavior exists.
4. Account wizard and deletion confirmation; browser authentication cancellation. Device Code, offline identity and Authlib providers need real distinct backend support before they can be advertised. Additional loader tiles must reflect actual install capability.
5. Complete graphical and interaction matrix, packaging and product-level acceptance on all four requested hosts.

## Platform policy

GitHub CI runs Windows AMD64, Linux AMD64 and macOS ARM64. Linux ARM64 is deliberately excluded from GitHub CI per the user's instruction and is tested separately on a native host. `scripts/verify-platform.py` records OS, CPU, Rust host, source fingerprints, exact commands, exit codes and phase logs. It rejects a mismatched host; cross-compilation is not counted as a native platform test.

GPU rendering, window behavior, motion, authenticated gameplay and compiler/framework tests remain separate evidence categories. Prior acceptance in `mvp-progress.md` and `session-supervisor.md` does not establish acceptance of this migration.
