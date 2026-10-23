# Block: Pilot Task List

Date: 2026-10-21, updated 2026-10-23 for the milestone split. This file replaces `gpui-ui-parity-plan.md` and `gpui-ui-parity-goal-prompt.md` as the entry point. Milestone II execution prompt: [codex-goal-prompt.md](codex-goal-prompt.md).

## Development milestones (user decision, 2026-10-23)

| Stage | Focus | Owner / current state |
| --- | --- | --- |
| Pilot Milestone II | Basic usability: real features and end-to-end flows work. Interaction polish is deferred. | DeepSeek; nearly complete, remaining functional verification stays explicit. |
| Pilot Milestone III | Interaction quality: inspect and fix interactions one by one, including visual presentation, navigation, input, focus, overlays, motion and feedback. Architecture design is deferred. | Codex; next stage, not started by this planning update. |
| Pilot Milestone IV | Begin architecture design, using the working product and interaction findings as inputs. | Future stage; implementation of a redesign needs its own scope. |
| v0 | First public unstable release. | Future stage; release preparation and publication are separate from Pilot completion. |

These stages supersede the earlier combined Pilot acceptance gate and the rule that architecture waits for v0. Milestone II still requires real behavior rather than placeholders, safe persistence, and explicit compatibility limits. An interaction defect that blocks a core flow remains a Milestone II usability issue; visual or interaction refinement belongs to Milestone III. Only the user's review sets Accepted in Milestone III. Missing visual acceptance does not by itself block Milestone II completion.

## Where we are (2026-10-23, reviewed HEAD `2ce25e7`)

- DeepSeek is closing Milestone II under the scoped v8 prompt in [codex-goal-prompt.md](codex-goal-prompt.md). Codex owns Milestone III. v6/v7 are in Git history.
- CI: four platforms green at `2ce25e7` (run 39831335710); Windows ARM64 green with an uploaded artifact at `82b3847` (run 39823711474). Changes between those commits are documentation only.
- Native app evidence:
  - macOS: live PKCE and Device Code; Welcome → Vanilla → main menu → Stop; relaunch and play without signing in again; overlay, IME and real quick-link drag checks passed.
  - Windows AMD64: Device Code; atomic Welcome; Vanilla → main menu → Stop; Q6 loader flows (Fabric/Quilt/Forge/NeoForge/legacy Forge), separated mode, crash help, interrupted-download retry, real quick-link drag.
  - Linux ARM64: native 1.16.5/1.18.2 wizard → Play → menu → Stop with Azul runtimes; LWJGL2 picker restriction.
  - Linux AMD64: native Welcome → Vanilla → menu → Stop; Q9 D6 short-display/reduced-motion/input checks; Authlib fixture sign-in + real avatar + injected 1.20.6 menu; provider icons; X11 quick-link drag.
  - Windows ARM64: native launcher build, Welcome, catalog, install, offline account and restart persistence passed; game rendering is blocked by the VM's missing OpenGL ICD, a recorded best-effort boundary.
- Windows Settings/SOCKS5/storage, loader and older-version flows, OptiFine re-import, queued Play, and lifecycle race fixes have evidence in STATUS.md and target/parity-shots/.
- Milestone II remaining: **prepare the complete screenshot evidence set** (native scene matrices for Windows/macOS, a scenario→page/state mapping manifest, the missing React prototype references, an E2E flow screenshot index, and real-data light/dark screenshots for Home/Accounts/Games/Wizard/Settings/About/Welcome), then the security revoke and final live-account sign-in/relaunch check plus Microsoft live-avatar verification. Screenshot preparation is a Milestone II deliverable; it has **not been started** (paused at the user's request for an interview). Completion has not been declared.
- Milestone III backlog: page-by-page visual and interaction review using the completed Milestone II screenshot set; macOS system reduced motion, transition-frame audit and actual Settings/Accounts fit at 1280×720; interaction fixes and acceptance. Each platform currently has 142 native scenario images; only 39 scenario names pair with React references and each board has 72 missing-side placeholders despite no broken image links, which the Milestone II screenshot preparation must resolve or explain.

## Decisions (user, 2026-10-21)

1. The main agent (Codex until v7, DeepSeek from v8) may commit and push directly to `main` ("bulldozer" mode). No PRs. No force-push and no history rewriting.
2. Delete the old ledgers and evidence before the first baseline commit. Do not archive them or let them enter Git history. Do not rewrite existing history.
3. No accessibility work for the foreseeable future.
4. Windows are fixed-size. Users cannot resize them.
5. Delete all GPUI patches and the `vendor/` copies. Use the published crates unmodified.
6. ~~Dialogs become real native windows rendered by GPUI.~~ Superseded by 19.
7. The combined Pilot completion definition and architecture-at-v0 rule are superseded by the 2026-10-23 milestone split above.
8. The full backend is in scope, using official sources only. Mirrors such as BMCLAPI are not supported for now.
9. The user owns a genuine Minecraft account. User-assisted live sign-in waits until the morning of 2026-10-22, Asia/Taipei, when the user is available; use Offline accounts and local fixtures meanwhile.
10. Project-authored docs, code comments, commit messages and UI strings committed from this work are in English. Preserve original third-party source and license text.
11. Legacy ARM64 game support is best effort: investigate and try viable upstream runtime/native-library combinations, record actual outcomes and explain proven incompatibilities. No blanket promise of old-version support and no silent compatibility bypasses.
12. OptiFine uses an external download from optifine.net followed by automatic validation and installation of the user-selected JAR. No bundling, redistribution or manual file-placement requirement.
13. Finish P0/P1/P2 foundations before expanding page/backend work. Prepare native light/dark screenshots and the comparison board for user visual acceptance on 2026-10-22; immediate per-page taste approval is not a prerequisite for continuing independent work.

## Decisions (user, 2026-10-22)

14. Quick Play falls back to the legacy `--server`/`--port` arguments for versions without `--quickPlayMultiplayer`. Disable it with a reason only where neither form is supported.
15. Legacy ARM64 support for LWJGL 3-era versions is built into the product: verified official LWJGL ARM64 artifacts and an official ARM64 Java 8 runtime, substituted explicitly. LWJGL 2-era versions stay disabled on ARM64 with the reason.
16. Authlib quick-link drag and drop is mandatory, as in the prototype. Published GPUI only supports file drops, so one dedicated crate may implement an owned native drop target with scoped unsafe code. GPUI itself is never patched, vendored or swizzled.
17. The main agent may delegate bounded tasks, including the prototype screenshots, to subagents running cheaper models or lower reasoning effort. Verification verdicts stay with the main agent.
18. A feature counts as verified only when exercised through the native app. API-driver smokes prove the backend only.
19. Retract the separate-native-window scheme for dialogs and drawers. Rebuild them as in-window overlay components (prototype-aligned dimmed scrim, blocked background interaction, Escape close, focus contained and restored on close) on the gpui-kit/gpui-component Dialog and Sheet primitives; keep the project's own content components and palette. Windows remain fixed-size. Use the component library's animation/overlay machinery instead of hand-rolled equivalents.
20. Use gpui-kit/gpui-component components wherever they meet the motion and presentation requirements, instead of building our own.
21. The CI matrix includes Linux ARM64 (`ubuntu-24.04-arm`).
22. Block supports running on Windows ARM64. The test host is a Windows-on-ARM VM on the Mac; the user installs the Rust platform on it. Launching Minecraft natively on Windows ARM64 is best effort, like decision 11: record real outcomes, explain limitations, no silent bypass.
23. Review order: finish the overlay refactor and recapture first, then send one grouped visual review. Accepted is set only from the user's review.
24. Windows AMD64 and Linux AMD64 share one physical machine. Never run heavy tests or builds on both at once; agents coordinate host use.
25. Add account from Home navigates automatically to the Accounts management page's Add account page, as the prototype's `/accounts/new` does. It is not an overlay, and no separate page or component is built for it.

## Operations (user, 2026-10-22)

- The Windows AMD64 workspace lives on D: (`D:\block-src`, `D:\block-target`). Cleaning temporary Codex/pilot folders and destructively cleaning the Linux VMs to free space were approved and done.
- Agents may install software on the test hosts (capture tools, Java, sccache, Python, Git and similar) without further approval.
- Multiple agents may collaborate through Git on dedicated branches. No force-push; main history is never rewritten.
- Agents may use the hosts' overlay addresses for machine-to-machine services; temporary services stop after use.
- `.env` values may be passed to workers inside commands so they can use the real Microsoft client ID. They are never printed or written to disk.
- The number of agents may grow as needed.
- WireGuard overlay: Mac hub with `10.77.0.0/24` routes only, no full tunnel. It carries SSH, Git, VNC, the Authlib fixture and Minecraft servers.
- Service placement: Git and the Authlib fixture on the Mac or Linux ARM64; Minecraft servers on Linux AMD64; VNC on each host.

## Test hosts (addresses in `.env`)

| Host | Notes |
| --- | --- |
| WireGuard overlay | Mac hub `10.77.0.1/24` on UDP 51820 at the fixed Mac LAN address; peers `10.77.0.11` (Windows AMD64), `10.77.0.12` (Windows ARM64), `10.77.0.21` (Linux ARM64), `10.77.0.22` (Linux AMD64). Configs/keys outside the repo under `~/block-wireguard`. The Mac must forward between peers (`net.inet.ip.forwarding=1`) |
| Linux ARM64 `LINUX_ARM64_VM_SSH` | Debian 13, KDE Wayland + Xwayland, `10.77.0.21`. Prepend `~/.cargo/bin` to `PATH`. Run GUI apps on Xwayland `:1` with the session Xauthority; capture with Spectacle (scrot returns black), drive with `xdotool` |
| Windows AMD64 `WINDOWS_AMD64_SSH` | Windows 10, `10.77.0.11`, PowerShell, output in GBK (use UTF-8 scripts or `-EncodedCommand`). The SSH session has no desktop, so start GUI apps with `schtasks /create /it`. Work in `D:\block-src` / `D:\block-target`; C: is small. Python, Git Bash and sccache are installed. Subagent `scp` uploads may be refused; pipe base64 over SSH stdin instead. Capture with `scripts/snap-native-windows` |
| Windows ARM64 `WINDOWS_ARM64_SSH` | Windows-on-ARM VM running on the Mac, `10.77.0.12`; same PowerShell/scheduled-task rules. C: only. Python and Git Bash are installed; the Rust platform comes from the user (decision 22) |
| Linux AMD64 `LINUX_AMD64_VM_SSH` | VMware guest on the Windows AMD64 host, `10.77.0.22`. Start it DETACHED (WMI `Win32_Process.Create`, never as the last command of an SSH session) with `vmrun -T ws start "D:\Virtual Machines\Block-Debian-AMD64\block-debian-amd64.vmx" nogui`. Also reachable with `ssh -o ProxyJump="$WINDOWS_AMD64_SSH"`. Its login keyring relocks after a reboot. Shares the physical machine with Windows AMD64 (decision 24) |

Keep one checkout per host (`~/block-src`; `D:\block-src` on Windows AMD64) and update it with `git fetch`. Never copy `target/` or player data.

## Existing Pilot work checklist

P0–P5 below are historical work-package names, not the new milestone numbers. Built and native functional evidence feed Milestone II; visual and interaction acceptance feeds Milestone III.

- [x] **P0 Baseline:**
  - Snapshot-commit the prototype.
  - Move still-valid product decisions into an English `docs/STATUS.md`.
  - Delete the old ledgers and evidence before staging any Block baseline commit; keep them out of Git history.
  - Commit the retained working tree to `main`, push, and check CI.
- [x] **P1 Platform simplification:**
  - Use unpatched GPUI and drop `vendor/`.
  - Make windows fixed-size; on macOS use the native traffic lights.
  - Turn dialogs and drawers into separate GPUI windows (superseded by decision 19).
  - Delete the overlay, inert, paint-scale and accessibility code.
- [x] **P2 Comparison tool:** a `--scenario` debug flag, a native screenshot script and a side-by-side board.
- [x] **P3 Inventory:** list every prototype page, state and feature in `STATUS.md` as the completion checklist.
- [x] **P4 Pages.** Each page ships with its full real backend (native evidence per STATUS; user acceptance remains user-only):
  - [x] Home, including real account avatars (Authlib fixture native; Microsoft live skin pending a live account)
  - [x] Accounts and sign-in, including provider icons and quick-link drag and drop (Windows/macOS/Linux native)
  - [x] Games and Game Wizard, including legacy Quick Play and productized legacy ARM64
  - [x] Settings (including memory 1–32 GB, SOCKS5/proxy, storage relocation)
  - [x] About and Welcome: atomic first-run account and game commit (native on Windows, 1ac4564); acceptance pending
  - [x] Cross-page work: motion, reduced motion, short displays, IME, keyboard and the inventory items
- [x] **v8 In-window overlays (decisions 19, 20):** Sheets, Dialogs and full-window Auth/About views on gpui-component. Separate surface windows deleted; Windows + macOS native checks and both scenario boards recaptured; visual review with the user pending.
- [x] **v8 Windows ARM64 (decision 22):** native arm64 build on the VM, hosted/manual CI job with artifact, manifest ARM64 natives and runtimes, native Welcome → install/offline/restart; game launch best effort recorded (reaches Java 21 + LWJGL 3.3.3 natives, blocked by the VM guest lacking an OpenGL ICD).
- [x] **v6 STATUS.md restructure:** Built / Native E2E / Accepted columns, a smoke matrix and a legacy ARM64 table instead of the ledger.
- [x] **v6 Native end-to-end flows:**
  - Windows AMD64: Q6/Q10/Q5 flows (Welcome/loaders/Settings/separated/crash/retry/older reps/Quick Play joins); macOS limited to the v8-required Mac-only checks (overlay, CJK IME, D3 drag, Q3 bounded attempt).
  - Each remote host: Linux ARM64 and Linux AMD64 Welcome → Play → menu → Stop; Windows ARM64 native launcher UI/install/offline/restart.
- [ ] **P5 Verification across Milestones II and III:**
  - [x] Required recent install/launch smoke on all four hosts (API driver), plus legacy Forge on Windows.
  - [x] Older representatives per loader and the legacy ARM64 table.
  - [ ] Milestone II: prepare the complete screenshot evidence set (native scene matrices, scenario→page/state mapping, missing React references, E2E flow index, per-page real-data light/dark screenshots).
  - [ ] Milestone III: complete the comparison board and user visual/interaction acceptance using that set.
  - [ ] Milestone II: revoke app access and complete the final live-account re-sign-in/relaunch check; earlier PKCE, Device Code and native launch runs passed.
  - [x] CI green at completion: four platforms + manual windows-arm64 workflow green (one Linux ARM64 help-claim flake bounded at 0/70).

Report Milestone II usability, Milestone III interaction acceptance, Milestone IV architecture design and v0 release readiness separately. Pending live checks and smoke runs remain explicit; Milestone III acceptance is not a Milestone II gate.

## Needs the user

- [x] Grant macOS Screen Recording permission to the terminal running Codex.
- [x] Provide the official OptiFine 1.21.1 JAR.
- [x] Enable "Allow public client flows" on the Azure app registration. Device Code sign-in requires it.
- [x] Sign in with the genuine Microsoft account (macOS PKCE/Device Code, Windows, both Linux hosts done).
- [ ] Revoke Block's app access in the Microsoft account (a refresh token was printed once in a diagnostic transcript), then re-sign in for the final live-account check.
- [x] Install the Rust platform on the Windows ARM64 VM (native build + tests run there).
- [ ] Unlock the Linux AMD64 keyring after VM reboots when a saved Microsoft account is needed there.
- [ ] Milestone III: review native screenshots and behavior page by page after reference/mapping gaps are resolved or explained.
- [x] Download an official OptiFine JAR for one older Minecraft version for the older-representative smoke.
- [ ] Optional: provide an Authlib provider account for live-provider verification.
- [x] Quick-link drags were verified with real browser drags on Windows, macOS and Linux X11.

## Pilot Milestone IV: architecture design inputs

- Evaluate per-page entities, engine polling versus async events, and boundaries in `lib.rs` and `game_wizard.rs`.
- Evaluate lint tidy-up and dead abstractions as part of the design; the milestone split does not itself authorize a broad rewrite.

## v0: first public unstable release

- Prepare release packaging, signing and distribution, and define the public compatibility/known-issues statement.
- Publication requires a separate release action; the stage definition does not publish anything.
- Accessibility and mirror download sources (BMCLAPI) remain deferred without a scheduled milestone.
