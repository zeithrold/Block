# Block: Pilot Task List

Date: 2026-10-21, updated 2026-10-22 for v6. This file replaces `gpui-ui-parity-plan.md` and `gpui-ui-parity-goal-prompt.md` as the entry point. Execution prompts: [codex-goal-prompt.md](codex-goal-prompt.md).

## What "Pilot" means

The pilot is the **complete product**:
- every page of the prototype is matched natively
- every feature it exposes works against real services
- nothing is faked or left as an unimplemented placeholder; genuine compatibility, validation and busy-state restrictions explain why an action is unavailable

Only **architecture governance** is deferred to v0. That covers structural refactoring, layering, file splitting, lint tidy-up and cleaning up code that already works.

## Where we are (2026-10-22, relay to DeepSeek)

- Codex's v6 run stopped mid-way. HEAD `442b94e` is green in CI. A small set of verified fixes is uncommitted: Windows CREATE_NO_WINDOW and secure-store error copy.
- Native app evidence so far:
  - macOS: live PKCE and Device Code; Welcome → Vanilla → main menu → Stop; relaunch and play without signing in again.
  - Windows: Device Code; Welcome → Vanilla → main menu → Stop.
  - Linux ARM64: Device Code.
- Open:
  - Welcome is not atomic (a real defect)
  - Linux AMD64 native sign-in
  - Windows scenario captures and the visual review
  - D1–D5 product gaps and OptiFine re-import
  - most native E2E flows, D6/D8/D9 and older representatives
- Windows is now the primary native UI/E2E and visual-review platform (user decision).
- The relay prompt is the "Pilot Relay Goal" in [codex-goal-prompt.md](codex-goal-prompt.md).

## Where we are (2026-10-22, v6)

- P0–P3 are done. HEAD `78f1595` is green in CI on Windows AMD64, Linux AMD64 and macOS ARM64.
- The P4 backend is nearly complete:
  - accounts: Microsoft PKCE and Device Code, Offline, Authlib (providers, Yggdrasil, launch injection)
  - loaders: Fabric, Quilt, Forge (modern and legacy), the full NeoForge list and OptiFine
  - settings: SOCKS5 and transactional storage relocation
  - Quick Play for versions that declare `--quickPlayMultiplayer`
- Real main menus were reached on all four hosts for the required recent loaders, plus legacy Forge on Windows. Every one of these launches went through a manual API driver or the CLI. None went through the native UI.
- Gaps:
  - prototype reference screenshots (the separate screenshot task never ran)
  - native end-to-end flows
  - real account avatars and provider icons
  - Authlib quick-link drag and drop
  - Quick Play for older versions
  - productized legacy ARM64
  - cross-page motion/IME/short-display checks
  - the unconfirmed Windows lifecycle flake
- Not yet started: live sign-in and visual acceptance.
- `docs/STATUS.md` drifted into an evidence ledger, and its checklist mixes "built" with "accepted". v6 restructures it.

## Decisions (user, 2026-10-21)

1. Codex may commit and push directly to `main` ("bulldozer" mode). No PRs. No force-push and no history rewriting.
2. Delete the old ledgers and evidence before the first baseline commit. Do not archive them or let them enter Git history. Do not rewrite existing history.
3. No accessibility work for the foreseeable future.
4. Windows are fixed-size. Users cannot resize them.
5. Delete all GPUI patches and the `vendor/` copies. Use the published crates unmodified.
6. Dialogs become real native windows rendered by GPUI. They are neither OS alert components nor in-window overlays, so no backdrop or input-blocking machinery is needed.
7. Pilot means the complete product. Only architecture governance waits for v0.
8. The full backend is in scope, using official sources only. Mirrors such as BMCLAPI are not supported for now.
9. The user owns a genuine Minecraft account. User-assisted live sign-in waits until the morning of 2026-10-22, Asia/Taipei, when the user is available; use Offline accounts and local fixtures meanwhile.
10. Project-authored docs, code comments, commit messages and UI strings committed from this work are in English. Preserve original third-party source and license text.
11. Legacy ARM64 game support is best effort: investigate and try viable upstream runtime/native-library combinations, record actual outcomes and explain proven incompatibilities. No blanket promise of old-version support and no silent compatibility bypasses.
12. OptiFine uses an external download from optifine.net followed by automatic validation and installation of the user-selected JAR. No bundling, redistribution or manual file-placement requirement.
13. Finish P0/P1/P2 foundations before expanding page/backend work. Prepare native light/dark screenshots and the comparison board for user visual acceptance on 2026-10-22; immediate per-page taste approval is not a prerequisite for continuing independent work.

19. Retract the separate-native-window scheme for dialogs and drawers. Rebuild them as in-window overlay components (prototype-aligned dimmed scrim, blocked background interaction, Escape close, focus contained and restored on close) on the gpui-kit/gpui-component Dialog and Sheet primitives; keep the project's own content components and palette. Windows remain fixed-size. Use the component library's animation/overlay machinery instead of hand-rolled equivalents.

## Decisions (user, 2026-10-22)

14. Quick Play falls back to the legacy `--server`/`--port` arguments for versions without `--quickPlayMultiplayer`. Disable it with a reason only where neither form is supported.
15. Legacy ARM64 support for LWJGL 3-era versions is built into the product: verified official LWJGL ARM64 artifacts and an official ARM64 Java 8 runtime, substituted explicitly. LWJGL 2-era versions stay disabled on ARM64 with the reason.
16. Authlib quick-link drag and drop is mandatory, as in the prototype. Published GPUI only supports file drops, so one dedicated crate may implement an owned native drop target with scoped unsafe code. GPUI itself is never patched, vendored or swizzled.
17. Codex may delegate bounded tasks, including the prototype screenshots, to subagents running cheaper models or lower reasoning effort. Verification verdicts stay with the main agent.
18. A feature counts as verified only when exercised through the native app. API-driver smokes prove the backend only.

## Test hosts (addresses in `.env`)

| Host | Notes |
| --- | --- |
| Linux ARM64 `LINUX_ARM64_VM_SSH` | Debian 13, KDE Wayland + Xwayland. Prepend `~/.cargo/bin` to `PATH`. Run GUI apps with `DISPLAY=:0`; capture with `scrot`, drive with `xdotool` |
| Windows AMD64 `WINDOWS_AMD64_SSH` | Windows 10, PowerShell, output in GBK. The SSH session has no desktop, so start GUI apps with `schtasks /create /it`. Drive C: is low on space |
| Windows ARM64 `WINDOWS_ARM64_SSH` | Windows on ARM; same PowerShell/scheduled-task rules. Used for ARM64 native checks |
| Linux AMD64 `LINUX_AMD64_VM_SSH` | VMware guest on the Windows AMD64 host. Start it DETACHED (WMI `Win32_Process.Create`, never as the last command of an SSH session) with `vmrun -T ws start "D:\Virtual Machines\Block-Debian-AMD64\block-debian-amd64.vmx" nogui`; while the Windows wired Ethernet is disconnected, reach the guest with `ssh -o ProxyJump="$WINDOWS_AMD64_SSH"`. Its login keyring relocks after a reboot |

Keep one checkout per host at `~/block-src` and update it with `git fetch`. Never copy `target/` or player data.

## Pilot Goal

- [x] **P0 Baseline:**
  - Snapshot-commit the prototype.
  - Move still-valid product decisions into an English `docs/STATUS.md`.
  - Delete the old ledgers and evidence before staging any Block baseline commit; keep them out of Git history.
  - Commit the retained working tree to `main`, push, and check CI.
- [x] **P1 Platform simplification:**
  - Use unpatched GPUI and drop `vendor/`.
  - Make windows fixed-size; on macOS use the native traffic lights.
  - Turn dialogs and drawers into separate GPUI windows.
  - Delete the overlay, inert, paint-scale and accessibility code.
- [x] **P2 Comparison tool:** a `--scenario` debug flag, a native screenshot script and a side-by-side board.
- [x] **P3 Inventory:** list every prototype page, state and feature in `STATUS.md` as the completion checklist.
- [ ] **P4 Pages.** Each page ships with its full real backend. The backend is nearly complete; see v6 for the remaining work:
  - [ ] Home, including real account avatars
  - [ ] Accounts and sign-in, including provider icons and quick-link drag and drop
  - [ ] Games and Game Wizard, including legacy Quick Play and productized legacy ARM64
  - [ ] Settings
  - [ ] About and Welcome, including the atomic first-run account and game commit
  - [ ] Cross-page work: motion, reduced motion, short displays, IME, keyboard and the remaining inventory items
- [x] **v6 STATUS.md restructure:** Built / Native E2E / Accepted columns, a smoke matrix and a legacy ARM64 table instead of the ledger.
- [ ] **v6 Native end-to-end flows:**
  - macOS: Welcome, every loader, Authlib, live Microsoft, Settings, Separated mode, crash help and Quick Play.
  - Each remote host: one Welcome → Play flow.
- [ ] **P5 Real verification:**
  - [x] Required recent install/launch smoke on all four hosts (API driver), plus legacy Forge on Windows.
  - [ ] Older representatives per loader and the legacy ARM64 table.
  - [ ] Prototype reference screenshots, the comparison board and user visual acceptance.
  - [ ] Live PKCE and Device Code sign-in with the user, plus a live-account native launch on macOS.
  - [ ] CI green at completion.

Pending required user acceptance, live sign-in or smoke runs must remain pending. Report implementation readiness separately from Pilot completion.

## Needs the user

- [x] Grant macOS Screen Recording permission to the terminal running Codex.
- [x] Provide the official OptiFine 1.21.1 JAR.
- [x] Enable "Allow public client flows" on the Azure app registration. Device Code sign-in requires it.
- [ ] Sign in with the genuine Microsoft account on 2026-10-22 when ready; Codex batches host requests and does not start expiring codes ahead of time.
- [ ] Review the native screenshots and side-by-side board once the prototype reference shots exist.
- [x] Download an official OptiFine JAR for one older Minecraft version for the older-representative smoke.
- [ ] Optional: provide an Authlib provider account for live-provider verification.
- [ ] Possibly: perform a quick-link drag on hosts where automation cannot.

## Deferred to v0 (architecture governance)

- Per-page entities, replacing engine polling with async events, splitting `lib.rs` and `game_wizard.rs`.
- Lint tidy-up and dead-abstraction cleanup.
- Accessibility, packaging, signing and release.
- Mirror download sources (BMCLAPI).
