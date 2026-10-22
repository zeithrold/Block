# Goal Prompts (v7 relay, v6 specification)

Context and decisions: [next-tasks.md](next-tasks.md). The v5 prompt (P0–P3 and most of the P4 backend) is in Git history.

| Prompt | Runner | When |
| --- | --- | --- |
| Pilot Relay Goal | DeepSeek (dsh), main agent | Now (2026-10-22); Codex's v6 run stopped mid-way |
| Pilot Completion Goal (v6) | Codex | Superseded as a runner prompt; still the binding detailed specification for the relay |

---

## Pilot Relay Goal

```text
Goal: Take over the Block pilot from Codex and drive it first to "implementation ready; awaiting user verification", then to completion with the user. You are the main agent for this relay. The v6 "Pilot Completion Goal" later in this file remains the binding specification (definition of complete, working rules 1–10, sections B–F, Stop-and-ask, Completion, Authorized / Not authorized). This prompt records the handoff state, overrides order and platform priority, and adds rules. Where the two conflict, this prompt wins.

Repository: /Users/zeithrold/Documents/Develop/Native/Block
Prototype (read-only reference): /Users/zeithrold/Documents/Develop/Frontend/Block-UI, snapshot commit fd2713b; its README.md is the design spec.
Read first, in this order: this prompt; docs/STATUS.md; docs/next-tasks.md (Decisions and Test hosts); the v6 prompt below; `git status`; `git log --oneline 78f1595..`; the justfile.

== Handoff state (2026-10-22, HEAD 442b94e, CI green) ==
Verified through the native app:
- macOS ARM64: Microsoft browser PKCE and Device Code, securely saved. Welcome → Vanilla 1.20.6 real download → Home → Play → main menu → Stop. Quit, relaunch, Play again without signing in (v6 B4 and macOS E2 are done).
- Windows AMD64: Device Code securely saved. Welcome creates a game, real download, 1.20.6 main menu, native Stop; the Java process exited.
- Linux ARM64: Device Code securely saved.
- Linux AMD64: the user created and unlocked a persistent keyring; an independent read/write/delete check passed. Native sign-in not done yet.
- Prototype reference screenshots exist in target/parity-shots/proto/ (86 images); scripts/parity-board pairs them with native captures.
Uncommitted work left by Codex (its full `just check` passed; not pushed):
- CREATE_NO_WINDOW for every Java child process on Windows (block-launcher command.rs, the executor Java probe, the NeoForge/Forge and OptiFine installers). Still needs a real Windows check: no console window during loader install or Play.
- Platform-specific copy for a locked/unavailable secure credential store (block-ui authentication.rs), with a test.
- scripts/parity-board --platform (Windows board at index.html, macOS board at macos-arm64.html).
- docs/STATUS.md updates.
Open defects and gaps:
- Welcome is not atomic: the new account is saved before the final game creation (STATUS row "Failed (macOS)"). This violates a product decision.
- Linux AMD64: xdotool mouse clicks do not trigger GPUI controls; keyboard navigation (Tab/Enter/Space) works.
- The Windows scenario capture run was interrupted; only 2 images exist in target/parity-shots/windows-amd64/scenarios/. scripts/snap-native supports macOS only.
- Not built: real account avatars (D1), provider icons (D2), quick-link drop (D3), legacy Quick Play (D4), productized legacy ARM64 and the LWJGL 2 picker restriction (D5), OptiFine re-import for an existing target.
- Native E2E is Pending for most STATUS rows. No page is Accepted.

== Overrides to v6 ==
O1. Windows AMD64 is the primary platform for native UI E2E and visual review (user decision, 2026-10-22). Run v6 flows B1, B2, B3, B5, B6, B7 and B8 on Windows instead of macOS. Keep on macOS only: CJK IME (D6), the D3 real drag, the bounded D5 attempt, and re-checks of macOS-specific code you change (titlebar, keychain, PKCE).
O2. Each Linux host still needs one native Welcome → Play → main menu → Stop. Linux ARM64 also runs a D5 legacy version through the wizard.
O3. Do not repeat anything listed as verified above unless your change touches that path.
O4. Subagents are optional. If your harness offers them, v6 rule 11 limits apply and every verdict stays with you.

== Relay rules (in addition to v6 working rules 1–10) ==
R1. Honest status.
   - Native E2E is "Yes" only after you drove the real block-app (no --scenario) on that host against real services, reached the end state and saved target/parity-shots/<platform>/e2e-<flow>.png.
   - A game launch must reach the main menu. View the capture if your harness can show images. If it cannot, confirm from the game log (title screen reached, no crash report), check that the capture is not blank, and write "Yes (log)" so the user can spot-check.
   - A Java PID, a "Running" label, a unit test or an API-driver run never counts as Native E2E.
   - Never set Accepted yourself. Only the user's review sets it.
   - Unfinished work stays "Partial" or "No", with the gap in Note.
R2. Small green steps. One work item at a time: implement → targeted tests → `just check` → commit (English message describing the behavior) → push → watch CI. Never push when `just check` fails. Fix a red CI before starting the next item.
R3. Touch only what the item needs. Do not reformat, rename or reorganize unrelated code. Never weaken, delete or #[ignore] an existing test to get a change through; if an existing test fails, assume the code is wrong.
R4. Read before writing. Before changing a subsystem, read its code and tests and follow their patterns: error types, cancellation tokens, revision/stale-result guards, and the local fixture servers in block-auth/block-download.
R5. Three strikes. If one approach fails three times (a build, a host procedure, an automation step), stop it, record the blocker as one STATUS line or Needs-user row, and move to the next item.
R6. Secrets. .env holds SSH targets, Microsoft client credentials and an API key. Use the values only inside commands. Never print, log, commit, screenshot or report them.
R7. Hosts. At the end of each remote session, close owned app instances, remove owned scheduled tasks and soft-stop the Linux AMD64 VM. On Windows, delete stale target/ folders before building (C: is small).
R8. Talk to the user in Chinese; everything in the repo stays English. Batch requests (v6 rule 10). For Device Code: tell the user first and wait until they say they are ready, then start the request and send the code and URL at once.

== Work queue (in order) ==
Q0. Handoff commit. Review the uncommitted diff yourself and keep it unless it is wrong. Run `just check`, then commit the code, the script, STATUS.md and these prompt/plan doc edits. Push and confirm CI.

Q1. Atomic Welcome commit (v6 D7).
   - The identity prepared in step 2 stays in memory; only the final "Ready to go" confirmation persists it, together with the Instance/Profile from step 1.
   - If the game save fails after the credential save, roll the credential back. Cancel, Back, closing the window or quitting the app at any earlier point leaves no credential in the OS store and no catalog entry.
   - Skip still creates no identity.
   - Tests: success, game-save failure with rollback, cancel at each step, a late result after cancel.
   - Then re-run Welcome natively on Windows with a new Offline account in a fresh data dir, and update the STATUS row. In the same run, confirm that no console window appears during install or Play (CREATE_NO_WINDOW check).

Q2. Windows visual capture and review (v6 C2–C3).
   - Add Windows support to scenario capture: extend scripts/snap-native with a Windows mode or add one helper script beside it (no new docs). Start block-app with each --scenario in light and dark inside the interactive session (schtasks /it). Capture only the app's own windows by PID (PrintWindow or equivalent). Write target/parity-shots/windows-amd64/scenarios/<scenario>-<theme>.png, using the prototype scenario names and the child-window names parity-board expects.
   - Capture every scenario, copy the images back, then run `scripts/parity-board` and `scripts/parity-board --platform macos-arm64`.
   - Review every pair yourself. If you cannot view images, check for missing pairs and size mismatches only, and say so in the message.
   - Send the user one review message, as in v6 C3. Fix your planned differences without waiting. Apply the user's corrections and re-capture. Set Accepted per page only on the user's verdict.

Q3. Linux AMD64 native Device Code (v6 E3).
   - Start the VM and launch block-app on its desktop. Use keyboard navigation to reach Add account → Microsoft → Device Code. Follow R8, and after the secure save close the app and soft-stop the VM.
   - Later Linux flows need the mouse, so try to fix the clicks within R5. For example, send a pointer motion before the click, then mousedown/mouseup with a short delay. If that fails, keep using the keyboard.

Q4. Product gaps. Specs are in v6 section D. Each gap is one or more Q0-style commits:
   a. D4: legacy Quick Play, including the real local-server join verification.
   b. D5: productized legacy ARM64, then the LWJGL 2 picker restriction.
   c. OptiFine re-import for an existing target through native file selection (STATUS gap).
   d. D1: account avatars. D2: provider icons.
   e. D3: the quick-link drop target in one dedicated crate (crates/block-drop). This is the highest-risk item.
      - Before writing any code, study GPUI's raw-window-handle access and the platform drag APIs.
      - Keep the unsafe surface minimal, with a SAFETY comment on every unsafe block. Never touch GPUI's own classes or drop targets.
      - Platform order: Windows → macOS → Linux X11 → Wayland attempt. Real drag checks as in v6 D3. Batch user drags when automation cannot perform them.

Q5. Native E2E flows on Windows (O1) and Linux (O2). Fill the Native E2E column row by row, then run the macOS-only checks. Fix every bug you find, with a regression test where the logic is testable.

Q6. v6 D6 (motion, reduced motion, a short display on the Linux AMD64 VM at 1366×768, keyboard/clipboard/scrolling), D8 (bounded Windows lifecycle-flake investigation) and D9 (sweep for remaining disabled or placeholder items).

Q7. v6 F older representatives. The user already put the official OptiFine 1.16.5 G8 and 1.12.2 G5 JARs in ~/Downloads. Validate and install them through native file selection.

Q8. Completion report as in v6 Completion.

Q2 and Q3 wait on the user. While you wait, keep working on Q4. Finish Q1 before any later item.

== Reporting ==
After each queue item, send a short Chinese message: what changed, the commits, capture/board paths, the STATUS rows that changed, and anything the user needs to do. The final report follows v6 Completion. Say "implementation ready; awaiting user verification" and list what is pending, unless the user has accepted everything.
```

---

## Pilot Completion Goal

```text
Goal: Finish the Block pilot. P0–P3 and most of the P4 backend are done and pushed (HEAD 78f1595 when this prompt was written; CI green on all three platforms). What remains:
- prove every feature through the native app itself, not through API drivers
- close the remaining product gaps, including four new user decisions
- complete live Microsoft sign-in and visual acceptance with the user, who is available today
- finish the P5 matrix
"Complete" is unchanged: a complete native Minecraft launcher matching the React prototype under the product decisions in docs/STATUS.md, every exposed feature working against real services, nothing faked or left as a placeholder. Architecture governance stays deferred to v0.

Repository: /Users/zeithrold/Documents/Develop/Native/Block
Prototype (read-only reference): /Users/zeithrold/Documents/Develop/Frontend/Block-UI, snapshot commit fd2713b; its README.md is the design spec.
Read first: docs/next-tasks.md, docs/STATUS.md, `git status`, `git log --oneline 5c634fe..`, the justfile and the prototype README.

== What changed since v5 ==
- The user is available today (2026-10-22, Asia/Taipei). Live sign-in and visual review are now current work, scheduled as soon as their prerequisites are met (sections C and E).
- Every game smoke so far launched through a manual API driver or the CLI. That proves the backend, not the product. From now on a feature counts as verified only when exercised through the native app (section B). Do not repeat API-driver smokes that already passed.
- New user decisions (2026-10-22), detailed in section D:
  - Quick Play falls back to the legacy `--server`/`--port` arguments for versions without `--quickPlayMultiplayer`.
  - Legacy ARM64 support for LWJGL 3-era versions is built into the product, not left as a manual composition.
  - Authlib quick-link drag and drop is mandatory, as in the prototype. Input/paste alone is not an accepted difference.
  - Bounded tasks may be delegated to subagents running cheaper models or lower reasoning effort.
- docs/STATUS.md drifted back into an evidence ledger. Restructure it first (section A).

== Working rules ==
1. Commit and push directly to main, as often as useful. No PRs, no force-push, no history rewriting. First commit or discard (after inspecting) whatever the v5 run left uncommitted.
2. Project-authored docs, code comments, commit messages and UI strings are in English. Preserve third-party source, attribution and license text.
3. Lean verification: targeted `cargo check` / `cargo test -p <crate> <filter>` while working, full `just check` once before each push, CI as cross-platform evidence. No hash receipts, fingerprints, attempt logs, evidence JSON or per-item ledgers. Do not re-run passing checks to re-confirm them.
4. Test business logic, protocols and state rules. New network protocols get local HTTP fixture tests (success, expiry, throttling, cancellation, error mapping) in the existing block-auth/block-download style. Tests never touch the internet. No tests for pixel layout, focus internals, animation frames, platform drag plumbing or framework mechanics.
5. No architecture work: no structural refactors, new layers/traits/registries, file splitting or tidying working code. When structure gets in the way, add one line to the STATUS.md "v0 notes" (at most 15 lines) and keep going.
6. Lints: keep clippy::all, the existing deny list (unwrap/expect/panic/todo/dbg/unimplemented/wildcard_imports) and `unsafe_code = "forbid"`. The only exception is the dedicated drop-target crate in D3.
7. Docs: maintain only docs/STATUS.md (English, at most 200 lines, see section A) and tick the boxes in docs/next-tasks.md. Do not create other docs.
8. Product honesty: never show fake success or fake progress. Do not port prototype demo behavior (fake sign-in, BroadcastChannel, simulated progress, sample data, the Reopen button, /auth/preview, "Interface preview" copy). Compatibility, validation and busy-state restrictions may disable an action only with a stated reason. Official download sources only; offer only "Official" where the prototype has a source picker.
9. Use temporary or dedicated --data-dir directories only. Never copy player credentials or worlds. Never write .env values into logs, commits, screenshots or subagent prompts.
10. Needs-user items go into the STATUS.md table and one batched message to the user; continue other work while waiting. Do not ask for review after every page.
11. Subagents. You may delegate bounded, fully specified tasks and choose a cheaper/faster model or lower reasoning effort for them.
    - Good candidates: the prototype screenshot script (C1, delegate this one), remote-host smoke runs and their screenshots, scenario/fixture additions, screenshot refreshes.
    - Keep in the main agent: product decisions, auth/installer/protocol code, the D3 platform code, STATUS.md, and every verification verdict.
    - Give each subagent the rules it needs (test data dirs only, no .env values, no commits in the prototype repo, no network in tests).
    - Review the diff, and look at the screenshots and results yourself, before committing or recording anything. A subagent's claim is not evidence.

== Order ==
A (first, small) → start C1 on a subagent → send the user the E1 request → B and D in parallel with C/E → visual review batch when C is ready → F → completion report.

== A. Restructure STATUS.md ==
- Inventory: one table per page with the columns Feature | Built | Native E2E | Accepted | Note.
  - Built: implemented, with tests where rule 4 applies.
  - Native E2E: exercised by clicking through the real app against real services. Write "fixture" when the Goal allows a local fixture (for example an Authlib provider without a live account).
  - Accepted: the user reviewed the page's screenshots/behavior and requested corrections are done.
  - Note: at most one short clause.
- Replace the Verification section with:
  - a smoke matrix: rows are hosts; columns are Vanilla, Fabric, Quilt, Forge, NeoForge, OptiFine, Authlib, Older; each cell is `<version> API|UI <short commit>` or empty
  - a legacy ARM64 table: combination | outcome | concrete limitation
  - at most 10 lines of notes that fit neither table, such as the open Windows lifecycle flake
- Delete CI run IDs, test counts, per-commit histories and repeated "this does not establish…" caveats. Git and CI already record them, and the columns carry the caveat.
- Keep product decisions, intended differences, Needs-user and v0 notes. Aim for about 150 lines.

== B. Native end-to-end flows ==
Drive the real block-app (no --scenario) with a fresh dedicated --data-dir per flow group. On macOS use your computer-use/desktop automation and CGWindowID captures. On remote hosts use the established xdotool/Spectacle (Linux) and interactive scheduled task (Windows) procedures. Confirm the main menu by observing/capturing the real game window; a Java PID or generic Running state is not enough. Save one end-state capture per flow as target/parity-shots/<platform>/e2e-<flow>.png and fill the Native E2E column. Fix the bugs you find, adding a regression test where the logic is testable.

macOS ARM64, each once (again only after relevant code changes):
1. First run: Welcome's three steps with a new Offline account and a Vanilla game → the downloads window shows real progress → Home → Play → main menu → Stop. Also Skip: no identity is created and Play still requires an account.
2. Loaders through the Game Wizard, including the loader/build picker → Play from Home → main menu → Stop: Fabric, Quilt, Forge, NeoForge, OptiFine standalone, and OptiFine with Forge (using the already provided official 1.21.1 JAR through the native file selection).
3. Authlib: add a provider through the native provider window (local Yggdrasil fixture unless the user supplies a live account), including a real quick-link drag once D3 lands → sign in through the native form → choose a character → Play → injected main menu.
4. Live Microsoft account (after E) → Play Vanilla → main menu → Stop → quit and relaunch the app → Play again without signing in (token refresh path).
5. Settings: switch Downloads to SOCKS5 through a local loopback forwarder and observe the next install going through it; relocate storage through the native UI and confirm cleanup in its window; Play still works afterwards. Change the theme and reduced motion, then cancel and save, checking draft/saved/preview separation.
6. Separated mode: convert (confirmed, one-way), add a second Profile to an existing Instance, Play it, delete a Profile and confirm the player files remain.
7. Crash help: launch a configuration that genuinely fails (for example an invalid extra JVM argument) and confirm the crash-help UI and recovery.
8. Quick Play: auto-connect to a local official server through the native Profile settings (D4).

Each remote host (Windows AMD64, Linux AMD64, Linux ARM64): fresh data dir → Welcome → Vanilla or Fabric → Play from Home → main menu → Stop, all through the native UI. Windows also creates legacy Forge 1.12.2 through the wizard. Linux ARM64 also runs a D5 legacy version through the wizard.

== C. Prototype reference screenshots and visual review ==
C1 (delegate to a subagent). In the prototype repo, add scripts/snap-proto (Playwright + Chromium):
- Scenario names match the native scenario names already used by scripts/snap-native, so the board pairs them. Cover at least: home-empty, home-ready, home-downloading, games-bundled, games-separated, games-detail, games-instance-settings, wizard-setup, wizard-review, accounts-list, accounts-detail, accounts-providers, every auth-wizard step, settings-general/game/profiles/downloads/storage, about, welcome-1/2/3, and each dialog/drawer separately: home-profile-drawer, home-downloads-drawer, wizard-version-picker, remove-confirmation, provider add.
- Viewport: Home 432×768, everything else 960×600, deviceScaleFactor 2. Light and dark (through settings or the .dark class). Disable animations. Crop to the app card only.
- Reach states by clicking and typing. Touch only scripts/, devDependencies in package.json, and selector-only data-scenario attributes that do not change appearance or behavior. Code and comments in English. Do not commit in the prototype repo.
- Output: /Users/zeithrold/Documents/Develop/Native/Block/target/parity-shots/proto/<scenario>-<theme>.png.
C2 (main agent). Review the subagent's diff and a sample of its images, rebuild target/parity-shots/index.html with scripts/parity-board, then refresh any stale native captures.
C3. Send the user one review message: the board path, and a list of differences you see (at most 20 lines, grouped by page) separated into intended differences (native windows, chrome, real data instead of samples) and differences you plan to fix. Fix the planned ones without waiting. Apply the user's corrections, re-capture the affected scenarios and mark Accepted per page.

== D. Remaining product gaps ==
D1. Account avatars. Home, selectors and Accounts must show the real player face instead of initials when available.
   - Microsoft: the active skin from the Minecraft profile response.
   - Authlib: the profile textures property, downloaded only from the provider's declared skin domains.
   - Render the face plus hat layer at the needed sizes and cache it in the data dir with validation. Refresh it with the account.
   - Offline or on failure: initials, with no error noise. Never show the demo Steve/Alex images.
   - Tests: texture parsing, domain allowlist, cache corruption and fetch failure, using local fixtures.
D2. Authlib provider icons: retrieve the cached icon URL with bounded size/type, validate it, cache it and display it, falling back to the current monogram. Fixture-tested.
D3. Authlib quick-link drag and drop (mandatory).
   - Behavior: the Add provider window has the prototype's directly visible drop zone. It accepts the official `text/plain` payload `authlib-injector:yggdrasil-server:<URI-encoded API root>` dragged from a browser or other app, fills the editable name/API fields and continues through the existing discovery/consent/duplicate flow. Pasting a quick link into the API field may stay.
   - Finding: gpui-pre 0.3.9 is the latest published release. Its platforms register only file drops (macOS NSFilenamesPboardType, Windows CF_HDROP; check Linux). Base components offer nothing either.
   - Authorized approach: one new dedicated crate (for example crates/block-drop) that installs an owned native drop target over the drop zone, reaching the window through GPUI's raw-window-handle. Examples: an owned NSView subclass added as a subview on macOS; an owned child HWND with its own IDropTarget on Windows; XDND/data-device handling on Linux.
   - Constraints: do not modify, swizzle or re-register GPUI's own classes, views or drop targets, and do not vendor or patch GPUI. That crate alone may lower `unsafe_code` from forbid to deny and allow it only in its platform modules, with a SAFETY comment on every unsafe block. The rest of the workspace keeps forbid. Record the exception in v0 notes.
   - Platforms: macOS, Windows and Linux X11 are required. Try Linux Wayland. If a platform proves infeasible without patching GPUI, record the problem and options in Needs-user and continue.
   - Tests: payload parsing and import rules only.
   - Verify with a real drag from a browser page with a draggable link on macOS and at least one other platform. If automation cannot perform a drag on a host, put that host's drag check in the user batch.
D4. Quick Play.
   - Versions whose manifest has `--quickPlayMultiplayer` keep using it.
   - Older versions use the legacy `--server <host> --port <port>` game arguments where that version supports them. Establish and document the supported range in code from official version data and real tests, not guesswork. Default to port 25565; handle IPv6 literals and invalid addresses.
   - Where neither form is supported, disable auto-connect with the reason. Replace the current "until support is known" wording.
   - Unit-test argument compilation per version family.
   - Real verification: run an official Minecraft server JAR from Mojang's piston metadata in a temp dir with online-mode=false. Auto-connect a recent version (quickPlay) and an older version (legacy arguments) through the native UI, and confirm each join in the server log. Use AMD64 hosts for versions that cannot run on ARM64.
D5. Legacy ARM64, built into the product.
   - For LWJGL 3-era versions whose manifest selects LWJGL natives without ARM64 builds, the launcher substitutes verified official LWJGL ARM64 artifacts from Maven Central (org.lwjgl) and required companions such as JNA, based on the composition proven for Linux ARM64 1.16.5.
   - Key the substitution on the manifest's LWJGL version, not on individual Minecraft versions.
   - Where Mojang's runtime has no ARM64 Java 8, provision one from an official vendor API (the tested Azul Zulu) with checksum verification.
   - Substitution is explicit in the plan and logs, never silent; checksums come from the official source.
   - Linux ARM64: verify 1.16.5 plus at least one other LWJGL 3-era version through the native wizard and Play.
   - macOS ARM64: make one bounded new attempt with the same mechanism. The established failures (window-icon/GLFW binding) stay a stated boundary if it fails.
   - LWJGL 2-era versions (1.12.2 and earlier) are disabled on ARM64 in the version picker with the reason (official natives are x86/x64 only), not hidden.
   - Record every outcome in the legacy ARM64 table, including the narration limitation.
D6. Cross-page.
   - Page/detail/wizard transitions with correct directions, 460 ms cubic-bezier(0.16,1,0.3,1), upstream opacity/offset only.
   - System and in-app reduced motion.
   - Fit-to-display at window creation on short screens, checked on a real short display, for example the Linux AMD64 VM at 1366×768.
   - Text input including CJK IME on macOS, clipboard, search, Enter/Escape/Tab in windows, scrolling, and draft/error preservation. Check these in the native app and fix them.
D7. First-run account/game commit: confirm the new account and the game are saved together only at the final Welcome step, and that cancelling or failing leaves neither half behind. Implement it if missing.
D8. Windows lifecycle flake: a bounded investigation, for example the relevant lifecycle test in a loop of about 50 runs on the Windows host. Fix it if reproduced; otherwise leave one STATUS note.
D9. Re-walk the inventory and the native code for any remaining disabled, unavailable or placeholder item, and build it.

== E. Live sign-in ==
E1. Today, in your first message to the user, ask them to:
   (a) enable "Allow public client flows" on the Azure app registration
   (b) tell you when they are ready to sign in
   Do not start any Device Code request before (b).
E2. macOS: PKCE through the native app into a dedicated data dir (for example ~/block-test-data), then Device Code into a second dedicated data dir. Then run flow B4. Keep both data dirs for reuse; do not delete the accounts.
E3. Remote hosts: Device Code, batched. When the user is ready, start the Linux AMD64 VM first, start all three hosts' requests together, and send the codes/URLs in one message. A live launch on remote hosts is optional.
E4. Authlib live only if the user supplies a provider account. Otherwise flow B3 with the fixture is the recorded result, labeled "fixture".

== F. P5 completion ==
- The required recent matrix already passed through API drivers on all four hosts. Do not repeat it; section B adds the native-UI evidence.
- Older representatives where they exist, through the native UI where practical, on hosts that can run them:
  - Vanilla 1.12.2 and 1.8.9 on AMD64
  - Fabric 1.16.5
  - Quilt 1.18.2
  - NeoForge 1.20.1 (done)
  - legacy Forge 1.12.2 on Windows (done via API; repeat via UI in B)
  - OptiFine for an older version, needing the user's official JAR: add it to the batched user request
- Remote host procedure: fetch the pushed commit into ~/block-src. Never copy the local tree or target/. Screenshots go to target/parity-shots/<platform>/.
  - Linux ARM64: `export PATH=$HOME/.cargo/bin:$PATH`, Xwayland `:1` with the session Xauthority, Spectacle for capture (scrot returns black), xdotool to drive.
  - Windows: GUI apps through `schtasks /create /it`; PowerShell output is GBK; clean old target/ folders first because C: is tight; remove owned tasks afterwards.
  - Linux AMD64: start the VM from the Windows host with vmrun (see next-tasks.md), wait for SSH, and stop it with `vmrun -T ws stop <vmx> soft` afterwards. If guest SSH drops, restart only the guest services from the console, as before.

== Stop and ask the user when ==
- the prototype conflicts with real backend behavior in a way you cannot resolve
- D3 cannot be done on a required platform without patching GPUI (write down the problem and options, continue elsewhere)
- something would delete user data
- a licensing or distribution question remains after the approved OptiFine flow
- the same CI failure survives three fix attempts

== Completion ==
The Pilot Goal is complete when:
- every STATUS.md inventory row is Built and Native E2E (or "fixture" where allowed), and Accepted
- the D1–D9 items and the four 2026-10-22 decisions are implemented and verified
- live Microsoft PKCE and Device Code sign-in have been verified with the user, and B4 passed
- the P5 matrix, older representatives and legacy ARM64 table are complete and honest
- CI is green and STATUS.md is accurate

If user acceptance, live sign-in, a user-supplied OptiFine JAR or a user-performed drag check is still pending, report "implementation ready; awaiting user verification" with the explicit list. Do not report "Pilot Goal complete". An established legacy ARM64 limitation is a stated support boundary, not a pending item.

When reporting progress to the user, include what changed, the commits, the native screenshots/board path, remaining differences and what the user needs to do.

Authorized:
- commit and push to main
- the deletions and restructuring described above
- delegating bounded tasks to subagents with cheaper models or lower reasoning effort
- uncommitted script/devDependency/data-scenario changes in the prototype repo (C1)
- one dedicated crate with scoped unsafe code for D3, under the constraints above
- using the test hosts in .env over SSH and vmrun
- downloading games, loaders, authlib-injector, official Minecraft server JARs, LWJGL/JNA from Maven Central and an ARM64 Java 8 runtime from its official vendor API into temporary or test directories
- automatically installing user-selected official OptiFine JARs

Not authorized: force-push or history rewriting, releases, GPUI framework patches or vendoring, swizzling or replacing GPUI's own platform classes, editing prototype product content, commits in the prototype repo, deleting user data.
```
