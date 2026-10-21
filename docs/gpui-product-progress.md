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

## Remaining slices

1. Searchable game browser, appearance and launch detail subpages; separated-profile creation flow.
2. Selector search, arrow keys and selection feedback; avatar fallback; distinct downloading/running/stopping states; complete enter/exit, detail, wizard and overlay animations.
3. Real new-game defaults, download/network policies, storage paths and profile appearance; expose each setting only when its backend behavior exists.
4. Account wizard and deletion confirmation; browser authentication cancellation. Device Code, offline identity and Authlib providers need real distinct backend support before they can be advertised. Additional loader tiles must reflect actual install capability.
5. Complete graphical and interaction matrix, packaging and product-level acceptance on all four requested hosts.

## Platform policy

GitHub CI runs Windows AMD64, Linux AMD64 and macOS ARM64. Linux ARM64 is deliberately excluded from GitHub CI per the user's instruction and is tested separately on a native host. `scripts/verify-platform.py` records OS, CPU, Rust host, source fingerprints, exact commands, exit codes and phase logs. It rejects a mismatched host; cross-compilation is not counted as a native platform test.

GPU rendering, window behavior, motion, authenticated gameplay and compiler/framework tests remain separate evidence categories. Prior acceptance in `mvp-progress.md` and `session-supervisor.md` does not establish acceptance of this migration.
