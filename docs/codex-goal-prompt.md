# Goal Prompt (v8, scoped to Pilot Milestone II)

Execution closed at `f2a5fd7` on 2026-10-23. This is the historical Milestone II prompt; its queue and handoff are not a current list of unfinished work. Use [STATUS.md](STATUS.md) and [next-tasks.md](next-tasks.md) for Milestone III. The source upload and status review do not restart this goal or set Accepted.

Context, decisions and test hosts: [next-tasks.md](next-tasks.md). Status: [STATUS.md](STATUS.md). Earlier prompts (v6 "Pilot Completion Goal", v7 "Pilot Relay Goal") are in Git history at `ae0b758`. The 2026-10-23 milestone split scopes this prompt to DeepSeek's Milestone II functional closeout. Codex owns Milestone III interactions; Milestone IV begins architecture design; v0 is the first public unstable release.

| Prompt | Runner | When |
| --- | --- | --- |
| Milestone II closeout, v8 | DeepSeek Harness (dsh): main agent plus DeepSeek subagents | Functional closeout; remaining interaction work passes to Codex in Milestone III |

---

## Pilot Goal v8

```text
Goal: Finish Pilot Milestone II: basic usability. You are DeepSeek's main agent for functional closeout and may run DeepSeek subagents (rule 12). Read the current milestone split and status before using the historical handoff and Q0-Q10 queue below.

Milestone II completion means a usable native Minecraft launcher:
- its core tasks work under the product decisions in docs/STATUS.md and docs/next-tasks.md
- every exposed feature works against real services
- nothing is faked or left as a placeholder
Interaction polish, visual parity and user interaction acceptance belong to Pilot Milestone III, owned by Codex. Architecture design begins in Pilot Milestone IV. v0 is the first public unstable release. Only the user's review sets Accepted; Accepted is not a Milestone II gate.

The Q0-Q10 queue below preserves the earlier work packages and evidence requirements. Use STATUS.md for current completion, and do not repeat completed flows without a relevant change. Q2 user visual/interaction review and the remaining Q9 motion/short-display interaction checks transfer to Milestone III, but Milestone II must still **prepare the complete screenshot evidence set** (native scene matrices, scenario→page/state mapping, missing React references, E2E flow index, per-page real-data light/dark screenshots) so Milestone III can review without missing artifacts. Fix an interaction issue in Milestone II only when it blocks basic use. Keep all deferred checks visible rather than marking them verified.

Repository: /Users/zeithrold/Documents/Develop/Native/Block (GitHub zeithrold/Block, public; CI .github/workflows/check.yml runs on every push).
Prototype, a read-only reference: /Users/zeithrold/Documents/Develop/Frontend/Block-UI at snapshot commit fd2713b. Its README.md is the design spec; its app/ source is the layout and behavior reference.
Read first, in this order:
1. this prompt
2. docs/STATUS.md
3. docs/next-tasks.md (milestone split, Decisions 1–25, Operations, Test hosts)
4. `git status` and `git log --oneline 442b94e..`
5. the justfile
6. scripts/snap-native, scripts/snap-native-windows and scripts/parity-board

== Handoff state (2026-10-22, HEAD ae0b758) ==
CI:
- e0e29b1 was green on all four platforms.
- Run 39727660062 (ae0b758) failed once on Linux ARM64 and passed on rerun, so the failure is a flake.
- The failing test is block-ui settings::draft_tests::storage_worker_saves_real_locations_preserves_edited_drafts_and_keeps_old_copies, at the compound assert at settings_draft_tests.rs:181.
- wait_storage() returns as soon as `storage.pending` clears. The assert then expects busy=false, cleanup=Some, no error and the later draft, all at the same moment. Suspect state that settles one poll later.

Verified through the native app (do not repeat unless your change touches the path):
- macOS ARM64:
  - Microsoft PKCE and Device Code, securely saved
  - Welcome → Vanilla 1.20.6 download → Play → main menu → Stop
  - relaunch, then Play again without signing in
- Windows AMD64:
  - Device Code
  - atomic Welcome (1ac4564): identity kept in memory, saved together with the game; quitting saves nothing
  - real 1.20.6 download → Play → main menu → Stop
  - no Java console window
- Linux ARM64: Device Code.
- Linux AMD64: Device Code. The earlier failure was a missing MICROSOFT_CLIENT_ID at launch, not the keyring.

Built but not natively verified:
- D4 legacy Quick Play (c0a9dc5): argument families come from official manifest data.
- D5 Linux ARM64 LWJGL substitution:
  - 3.3.1+ manifests (9c01486)
  - classifier-era 3.2.x → 3.3.1 + JNA 5.13.0 + text2speech 1.12.4 (be9e1e1)
  - both are explicit in the saved plan and the launch log
- Component-library foundation:
  - gpui-component 0.8.0 is initialized (57489a4)
  - one asset source (design::UiAssets) and the palette mirrored into the component theme (e0e29b1)
  - gpui-component registers its WindowState as a gpui_base::Root plugin, so overlays render inside the existing windows without replacing the root
  - No surface has been migrated: crates/block-ui/src/window_surfaces.rs still opens a separate OS window for every shell::Surface.

Partial evidence:
- Q4a: the 1.20.6 client ran with --quickPlayMultiplayer against a server on Linux AMD64, but no "joined the game" line was kept. 1.12.2 legacy not started.
- Windows scenario captures: 142 shots / 228 PNGs at c0a9dc5. The board is target/parity-shots/index.html, with macos-arm64.html beside it and the prototype references in target/parity-shots/proto/. The captures go stale with the overlay refactor (user decision 23).

Not started:
- D5: Azul ARM64 Java 8 provisioning, the LWJGL 2 picker restriction, the Linux ARM64 native wizard + Play, the bounded macOS ARM64 attempt
- Windows ARM64 support (decision 22)
- OptiFine re-import for an existing target
- D1 avatars, D2 provider icons, D3 quick-link drop
- most native E2E flows
- D6, D8, D9
- older representatives, including the user's OptiFine 1.16.5 G8 / 1.12.2 G5 JARs in ~/Downloads
No STATUS row is Accepted.

== Working rules ==
1. Git:
   - Commit and push to main directly. No PRs, no force-push, no history rewriting.
   - Subagents work on their own `agent/<topic>` branches and never push to main (rule 12).
   - Messages are in English and describe the behavior.
2. English in the repo: docs, code comments, commit messages and UI strings. Preserve third-party source, attribution and license text. Talk to the user in Chinese.
3. Small green steps. One item at a time:
   - implement → targeted `cargo test -p <crate> <filter>` → full `just check` → commit → push → watch CI
   - Never push a failing `just check`. Fix a red CI before starting new work.
   - A flake counts as red until it is fixed or bounded (Q0).
4. Tests cover business logic, protocols and state rules.
   - New network protocols get local HTTP fixture tests (success, expiry, throttling, cancellation, error mapping), in the block-auth/block-download style.
   - Tests never touch the internet.
   - No tests for pixel layout, animation frames, platform drag plumbing or framework internals.
   - Never weaken, delete or #[ignore] an existing test to get a change through. If one fails, assume the code is wrong, and that includes flakes.
5. Read before writing. Before changing a subsystem, read its code and tests and follow their patterns: error types, cancellation tokens, revision/stale-result guards, fixture servers.
   - Before using gpui-component, read its source in ~/.cargo/registry (version 0.8.0) and its examples. Do not guess APIs.
6. No architecture work: no structural refactors, new layers, file splits or tidying of working code.
   - When structure gets in the way, add one line to STATUS "Pilot Milestone IV: architecture notes" (at most 15 lines) and keep going.
   - Deleting window_surfaces.rs as part of Q1 is product work, not governance.
7. Lints:
   - Keep clippy::all, the deny list (unwrap/expect/panic/todo/dbg/unimplemented/wildcard_imports) and `unsafe_code = "forbid"`.
   - The one exception is the D3 drop-target crate.
   - GPUI is never patched, vendored or swizzled.
8. Product honesty:
   - No fake success, progress, sample data or prototype demo behavior (BroadcastChannel, simulated progress, /auth/preview, "Interface preview" copy).
   - An action may be disabled only with a stated compatibility, validation or busy-state reason.
   - Official download sources only.
9. Data:
   - Use temporary or dedicated --data-dir directories only.
   - Never copy player credentials, worlds or target/ between machines.
10. Secrets: .env holds the SSH targets, MICROSOFT_CLIENT_ID/SECRET and DEEPSEEK_API_KEY.
   - Use the values only inside commands. To give one to a host or worker, pass it through the environment of the command (for example over SSH stdin).
   - Never print, log, write to disk, commit, screenshot or report them, and never paste them into subagent prompts as text.
   - A refresh token or any other credential must never appear in tool output. Redact it before printing a diagnostic.
11. Docs:
   - Maintain only docs/STATUS.md (English, at most 200 lines) and tick the boxes in docs/next-tasks.md. Create no other docs.
   - Put Needs-user items in the STATUS table and in one batched user message, then keep working.
12. Subagents (decisions 17, 24; Operations in next-tasks.md):
   - Delegate bounded, fully specified tasks with explicit files, hosts, acceptance checks and the rules above.
   - A subagent works in its own git worktree on an `agent/<topic>` branch and pushes only that branch. CI runs on branch pushes too.
   - The main agent:
     - reviews the diff
     - runs `just check` on the merge result
     - merges into main (a merge commit or fast-forward; never rebase a pushed branch)
     - looks at the screenshots/logs itself before recording anything
   - A subagent's claim is not evidence.
   - Keep in the main agent: product decisions, STATUS.md, every verification verdict, merges to main, and review of all D3 unsafe code.
   - Hosts: assign each test host to exactly one agent at a time.
     - Windows AMD64 and Linux AMD64 are one physical machine: never run builds or heavy tests on both at once. The Q5 server-plus-client join is the one allowed overlap, with no builds running.
     - The Windows ARM64 VM runs on this Mac: do not run its builds during timing-sensitive macOS native checks.
13. Three strikes. If one approach fails three times (a build, a host procedure, an automation step), stop it. Record the blocker as one STATUS line or Needs-user row and move on.
14. Hosts:
   - Follow next-tasks.md "Test hosts". The WireGuard overlay (10.77.0.0/24) carries SSH, Git, VNC and test services; stop temporary services when you are done.
   - At the end of each remote session: close owned app instances, delete owned scheduled tasks and soft-stop the Linux AMD64 VM unless another assigned agent still needs it.
   - Windows AMD64 builds in D:\block-src / D:\block-target. Windows ARM64 has only C:.

== Verification standard (decision 18) ==
- Native E2E is "Yes" only after driving the real block-app (no --scenario) on that host against real services, reaching the end state and saving target/parity-shots/<platform>/e2e-<flow>.png.
- A game launch must reach the main menu, confirmed by viewing the capture. If your harness cannot show images, confirm from the game log (title screen reached, no crash report), check that the capture is not blank, and write "Yes (log)".
- A Java PID, a "Running" label, a unit test, an API/CLI driver or a scenario capture never counts as Native E2E.
- Allowed local services are labeled "fixture" (an Authlib Yggdrasil fixture when there is no live provider account).
- Unfinished work stays Partial or No, with the gap in Note. Never set Accepted.

== Work queue ==
Lanes may run in parallel through subagents (rule 12), but every merge to main goes through the main agent.
- Lane A, main agent on the Mac: Q0 → Q1 → Q2.
- Lane B, a subagent: Q3 code first; Linux ARM64 native verification after Q1 is merged.
- Lane C, a subagent: Q5. Lane D, a subagent: Q4 investigation and build.
- Q6–Q10 follow, in order, once their prerequisites have landed.

Q0. Restore CI green and commit the v8 docs.
   - Commit this prompt and the next-tasks.md edits as they are (docs only), push.
   - Fix the settings storage-worker flake properly:
     - Split the compound assert into individual asserts with messages, so a failure names the condition.
     - Find what can still change after `pending` clears and make the test wait for the real end state (or fix the code if the state is genuinely inconsistent).
     - Run the test about 50 times on the Linux ARM64 VM before and after the change.
   - Commit, push and confirm all four CI jobs are green.

Q1. In-window overlay refactor (decisions 19, 20, 25). Retire separate OS windows for every shell::Surface.
   Placement follows the prototype, and the main window keeps switching between Home 432×768 and management 960×600 as it does today:
   - Home drawers (Drawer::Menu, Accounts, Profiles, Downloads) become gpui-component Sheets inside Home, as in the prototype's selector-drawer.
   - Confirmations (DeleteProfile, RemoveAccount, RemoveProvider, SeparateProfiles, StorageCleanup), GamesHelp, DownloadFiles, Select, AuthProviderAdd, and the Welcome/Game Wizard sections (OnboardingSection, WizardSection) become gpui-component Dialogs, centered in the current window and sized to fit inside it.
   - Auth (the Add account wizard) is the Accounts management page's own "Add account" page, as in the prototype (`/accounts/new` occupies the whole 8:5 window). It is not an overlay and not a separate view or component.
     - Home's Add account automatically navigates to Accounts → Add account, using the existing page navigation and window-size switch to 960×600.
     - Finishing or cancelling returns to wherever the prototype returns.
     - Do not build any new page, view or component for this. Reuse the existing Accounts page and authentication wizard content.
   - GuideInformation (Settings/About opened from the menu during Welcome): follow the prototype's behavior and reuse the existing Settings/About page content. Do not build a new page or component. Welcome's draft must survive the round trip.
   Behavior required everywhere:
   - a prototype-aligned dimmed scrim, and the background blocked for pointer and keyboard
   - Escape closes the topmost layer only, and does nothing while a non-cancellable operation is busy
   - focus moves into the layer, Tab stays inside it, and focus returns to the invoking control on close
   - stacking works (for example a Select or a confirmation above a wizard section)
   - motion comes from the library and respects system and in-app reduced motion
   - the window stays fixed-size
   - existing draft/checkpoint rules (Done/Cancel) and stale-result guards keep working: a late result never reopens or completes a closed layer
   Keep the project's own content components, copy and palette. Use library components wherever they meet the motion and presentation requirements; hand-rolled scrim, focus-trap or animation code is not wanted.
   Migrate in this order, one green commit or more per step:
   (1) a minimal overlay host on the component library, plus the confirmations
   (2) Home Sheets
   (3) Select, DownloadFiles, GamesHelp, StorageCleanup
   (4) Wizard/Onboarding sections
   (5) AuthProviderAdd
   (6) Auth as the Accounts → Add account page, with Home's Add account auto-navigating there; then GuideInformation per the prototype
   (7) delete window_surfaces.rs, the SurfaceWindows state, the child-window geometry, and tests that only covered separate windows
   Tests: overlay state rules only (open/close/stack order, Escape targets the topmost, focus-restore target recorded, late results ignored), rewriting the separate-window tests that covered those rules.
   Update the --scenario fixtures so every overlay scenario renders inside its parent window. Update snap-native, snap-native-windows and parity-board so each scenario is one window capture, keeping the prototype scenario names.
   STATUS: replace the "Independent windows, parent close and child reuse" row with an overlay row; update the product decision line.
   Native check on Windows AMD64 and macOS: open, stack, Escape and focus-restore one Sheet and one Dialog. Check Home → Add account → lands on Accounts → Add account → finish/cancel.

Q2. Visual baseline, recapture and one review (decision 23).
   Fix the known differences from the 82-pair review:
   - badges/tags stretched to full width (systemic)
   - the Settings sidebar is right-aligned/card-styled; it should be a flat, left-aligned list
   - dark surface #0A0A0A vs the prototype's #171717; light #FAFAFA vs #FFFFFF (check against the prototype tokens)
   - games-help and welcome-3 review content is clipped
   - Add provider lacks the visible drop zone: show the zone now; D3 makes it work
   - the browser-wait step lacks "Open browser again"
   - Separated mode shows "Search games…" instead of the prototype copy, and an empty "Used by" list
   - Memory is shown in MiB; the prototype uses GB
   - raw instance-settings paths overflow
   - offline validation errors use the generic sign-in error page
   - confirmation footers differ from the prototype style
   Then recapture all scenarios in light and dark on macOS (scripts/snap-native) and Windows AMD64 (scripts/snap-native-windows), rebuild both boards, and review every pair yourself.
   Send the user ONE review message:
   - the board paths
   - at most 20 lines grouped by page, split into intended differences (real data instead of samples, platform chrome) and remaining differences you plan to fix
   Fix the planned ones without waiting. Apply the user's corrections, recapture the affected scenarios, and record Accepted per page only on the user's verdict.

Q3. D5 legacy ARM64, the remainder (decision 15).
   - Provision an ARM64 Java 8 where Mojang's runtime catalog has none:
     - Use the official Azul metadata API (the tested Zulu 8u504 line), with checksum verification from the official source.
     - Install into the runtime store; selection is explicit in the plan and the log.
     - Fixture-test success, checksum mismatch, missing package and cancellation.
   - Disable LWJGL 2-era versions (1.12.2 and earlier) on ARM64 in the version picker with the reason "official LWJGL 2 natives are x86/x64 only". Do not hide them. Do this after Q1 lands to avoid wizard conflicts.
   - Linux ARM64, native: wizard → Play → main menu → Stop for 1.16.5 and one other LWJGL 3-era version (for example 1.18.2) in a fresh data dir.
   - macOS ARM64: one bounded attempt with the same mechanism. If the known window-icon/GLFW binding failure remains, it stays a stated boundary.
   - Update the legacy ARM64 table, including the narration limitation.

Q4. Windows ARM64 support (decision 22). The host is a Windows-on-ARM VM on this Mac (10.77.0.12, C: only).
   Two levels:
   - Required: Block itself runs natively on Windows ARM64, and every launcher feature that does not need a running game works there (UI, accounts, catalog, downloads, installation).
   - Best effort, like legacy ARM64 (decisions 11, 22): launching Minecraft natively on Windows ARM64. Try the viable official combinations, record the real outcome per combination, and explain proven limitations. No blanket promise and no silent bypass. An established limitation is a stated support boundary, not a pending item.
   Required part:
   - Toolchain: check `rustc -vV` (aarch64-pc-windows-msvc host), the MSVC ARM64 build tools and sccache. If the user has not finished installing the Rust platform, put it in Needs-user and keep going elsewhere.
   - Build and run block-app natively for aarch64-pc-windows-msvc. Add a windows-arm64 platform to scripts/verify-platform.py and a `windows-11-arm` job to the CI matrix.
   - Launcher plan:
     - Evaluate manifest OS/arch rules correctly for Windows ARM64.
     - Use the manifest's own natives-windows-arm64 classifiers where declared (LWJGL 3.3.1+ manifests).
     - Use Mojang's java runtime catalog "windows-arm64" entries where they exist; verify them from the live official catalog, never assume.
   - Native: fresh data dir → Welcome → Vanilla 1.20.6 → download and install complete. Capture with the Windows procedure.
   Best-effort part, bounded by rule 13:
   - Play 1.20.6 → main menu → Stop.
   - Older LWJGL 3-era manifests: try the Q3 substitution mechanism with official Maven Central natives-windows-arm64, plus an official vendor ARM64 Java 8 if one exists (verify the Azul metadata for windows/aarch64/8).
   - Wherever a combination cannot run, the version picker or Play explains why. Never fail silently or show fake readiness.
   - LWJGL 2-era versions are disabled on Windows ARM64 with the reason. Running x64 Java under Windows emulation is a product decision: record it as a Needs-user option and do not implement it.
   - STATUS: add a Windows ARM64 smoke-matrix row and a legacy ARM64 table entry per tried combination and outcome.

Q5. D4 real server joins (decision 14).
   - Run an official Mojang server JAR (piston metadata) on Linux AMD64 (10.77.0.22), in a temp dir with online-mode=false and a small view distance. Keep server.log.
   - Through the native UI on Windows AMD64, set the Profile server address to 10.77.0.22 with auto-connect:
     - 1.20.6 → Quick Play
     - 1.12.2 → legacy --server/--port
   - Confirm each "<name> joined the game" line in server.log and copy the log excerpts to target/parity-shots/linux-amd64/.
   - Capture the in-game screen on the client. Update the Quick Play STATUS row.

Q6. Native E2E flows. Primary platform: Windows AMD64. Each once, again only after relevant code changes:
   (1) Welcome Skip: no identity is created and Play still requires an account.
   (2) Loaders through the Game Wizard, then Play from Home → main menu → Stop:
       - Fabric, Quilt, Forge (plus legacy Forge 1.12.2), NeoForge
       - OptiFine standalone and OptiFine with Forge, using the official 1.21.1 JAR through native file selection
   (3) Authlib, using a Yggdrasil fixture on the Mac or Linux ARM64 over the overlay ("fixture"; live only if the user supplies an account):
       - add a provider (with a real quick-link drag after Q8), sign in through the native form, choose a character, Play, injected main menu
   (4) Settings:
       - SOCKS5 through a local loopback forwarder, then observe the next install going through it
       - relocate storage and confirm cleanup in its dialog; Play still works afterwards
       - theme and reduced motion: cancel, then save, checking draft/saved/preview separation
   (5) Separated mode:
       - confirmed one-way conversion
       - a second Profile for an existing Instance, then Play it
       - delete a Profile; the player files remain
   (6) Crash help: a genuinely failing configuration (for example an invalid JVM argument), then crash help and recovery.
   (7) Download retry: interrupt a download, then Retry; no second game or account is created.
   Each Linux host: fresh data dir → Welcome → Vanilla or Fabric → Play → main menu → Stop.
     - Linux AMD64: use an Offline account if the keyring is locked. If xdotool clicks still fail, use keyboard navigation.
   Keep on macOS only: CJK IME (Q9), the D3 real drag, the Q3 bounded attempt, and re-checks of macOS-specific code you change (titlebar, keychain, PKCE).
   Fix every bug you find, with a regression test where the logic is testable. Fill the Native E2E column row by row.

Q7. OptiFine and avatars.
   (a) OptiFine re-import for an existing target through native file selection.
   (b) D1 account avatars, replacing initials in Home, selectors and Accounts when available:
       - Microsoft: the active skin from the Minecraft profile.
       - Authlib: the textures property, fetched only from the provider's declared skin domains.
       - Render the face plus the hat layer and cache it in the data dir with validation; refresh it with the account.
       - Offline or on failure: initials, with no error noise. Never show demo Steve/Alex images.
       - Fixture tests: parsing, domain allowlist, cache corruption, fetch failure.
   (c) D2 provider icons: fetch the cached icon URL with bounded size and type, validate, cache and display it; fall back to the monogram. Fixture-tested.

Q8. D3 quick-link drag and drop (decision 16; mandatory).
   - Add a new crate, crates/block-drop. It installs an owned native drop target over the Add provider drop zone, reaching the window through GPUI's raw-window-handle:
     - Windows: an owned child HWND with its own IDropTarget
     - macOS: an owned NSView subview
     - Linux: X11 XDND; try Wayland
   - Never modify, swizzle or re-register GPUI's classes, views or drop targets.
   - Only this crate lowers unsafe_code to deny, and allows it only in its platform modules, with a SAFETY comment on every unsafe block. Record the exception in the Milestone IV architecture notes.
   - Behavior: accept the `text/plain` payload `authlib-injector:yggdrasil-server:<URI-encoded API root>`, fill the editable name/API fields and continue through the existing discovery, consent and duplicate flow.
   - Tests: payload parsing and import rules only.
   - Platform order: Windows → macOS → Linux X11 → Wayland attempt.
   - Verify with a real drag from a browser page on Windows and macOS. Put any drag that automation cannot perform into the user batch.
   - If a required platform is infeasible without patching GPUI, write the options into Needs-user and continue.

Q9. Cross-page and cleanup.
   - D6:
     - transitions with correct directions at 460 ms cubic-bezier(0.16,1,0.3,1), using upstream opacity/offset only
     - system and in-app reduced motion
     - fit-to-display on a real short display (the Linux AMD64 VM at 1366×768)
     - CJK IME on macOS, clipboard, search, Enter/Escape/Tab, scrolling, and draft/error preservation, checked in the native app
   - D8: run the Windows lifecycle test about 50 times on Windows AMD64. Fix it if it reproduces; otherwise leave one STATUS note.
   - D9: re-walk the inventory and native code for any remaining disabled, unavailable or placeholder item, and build it.

Q10. Older representatives and closing.
   - Native UI where practical:
     - Vanilla 1.12.2 and 1.8.9 on AMD64
     - Fabric 1.16.5
     - Quilt 1.18.2
     - OptiFine 1.16.5 G8 and 1.12.2 G5 from ~/Downloads through native file selection
   - Security wrap-up: ask the user (batched) to revoke Block's app access in their Microsoft account (a refresh token was exposed once in a diagnostic transcript). Then batch the live re-sign-in needed for a final live-account check: macOS PKCE → Play → relaunch → Play without signing in.
     - For Device Code, tell the user first and wait until they are ready, then send all codes and URLs in one message.
   - Final report.

== Stop and ask the user when ==
- the prototype conflicts with real backend behavior and you cannot resolve it
- D3 is infeasible on a required platform without patching GPUI
- something would delete user data
- a licensing or distribution question remains beyond the approved OptiFine flow
- the same CI failure survives three fix attempts
- Windows ARM64 needs a product choice (x64 emulation, or a non-official runtime source)

== Milestone II completion ==
Complete Milestone II when:
- exposed features and required core native flows have Built and Native E2E evidence (or "fixture" where allowed), with compatibility limits stated
- the functional portions of D1–D9 and decisions 14–16 and 19–25 are implemented and verified; remaining interaction refinements are assigned to Milestone III (Minecraft on Windows ARM64 counts as best effort, like legacy ARM64)
- live Microsoft PKCE and Device Code are verified with the user, plus the live-account relaunch check
- the smoke matrix (including Windows ARM64), the older representatives and the legacy ARM64 table are complete and honest
- CI is green on every matrix platform and STATUS.md is accurate
If a required functional live sign-in, user-supplied JAR or user-performed drag is still pending, report "Milestone II implementation ready; awaiting functional verification" with the explicit list. Visual/interaction acceptance stays pending for Milestone III and does not alone block Milestone II. Report completion by milestone; never use "Pilot Goal complete" to imply all later stages are done. An established legacy ARM64 or Windows ARM64 game limitation is a stated support boundary, not a pending item.

== Reporting ==
After each queue item, send a short Chinese message covering:
- what changed, with the commits
- capture/board paths
- the STATUS rows that changed
- anything the user needs to do

Authorized:
- everything in next-tasks.md Decisions and Operations
- commit/push to main
- `agent/*` branches
- installing tools on the test hosts
- using the hosts over SSH, WMI and vmrun
- downloading games, loaders, authlib-injector, official server JARs, LWJGL/JNA from Maven Central and ARM64 Java runtimes from official vendor APIs into temporary/test directories
- installing user-selected official OptiFine JARs
- one dedicated unsafe crate for D3
- adding the windows-arm64 CI job

Not authorized:
- force-push or history rewriting
- releases
- patching, vendoring or swizzling GPUI
- editing prototype product content, or commits in the prototype repo
- deleting user data
- third-party mirrors
```
