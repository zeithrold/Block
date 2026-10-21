# Block: Pilot Task List

Date: 2026-10-21. This file replaces `gpui-ui-parity-plan.md` and `gpui-ui-parity-goal-prompt.md` as the entry point. Execution prompts: [codex-goal-prompt.md](codex-goal-prompt.md).

## What "Pilot" means

The pilot is the **complete product**:
- every page of the prototype is matched natively
- every feature it exposes works against real services
- nothing is faked or left as an unimplemented placeholder; genuine compatibility, validation and busy-state restrictions explain why an action is unavailable

Only **architecture governance** is deferred to v0. That covers structural refactoring, layering, file splitting, lint tidy-up and cleaning up code that already works.

## Where we are

- `just check` passes on macOS ARM64. Every product page has a native implementation: Home, Games, Game Wizard, Profile/Instance editors, Accounts with real Microsoft PKCE, Settings, About and first-run setup.
- About 30k lines of `block-ui` work have never been committed or run through CI. The same goes for the vendored `gpui-pre` / `gpui-pre-macos` / `gpui-base` patches (about 2.5k diff lines).
- The previous Goal stalled on process, not code:
  - a 188-scenario / 464-item evidence ledger
  - hash receipts and a 45 MB evidence folder
  - custom framework patches for accessibility, inert layers and paint scaling
  - a framework upgrade partway through that reset all acceptance
- The backend lacks the following, which the UI currently marks as unavailable:
  - Device Code, Authlib and Offline accounts
  - Fabric, Forge and Quilt loaders, and NeoForge versions beyond one pinned build
  - OptiFine
  - SOCKS proxies
  - storage relocation
  - Quick Play
- The prototype (`Frontend/Block-UI`) has one commit. Most of its current product components are uncommitted.

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

## Test hosts (addresses in `.env`)

| Host | Notes |
| --- | --- |
| Linux ARM64 `LINUX_VM_SSH` | Debian 13, KDE Wayland + Xwayland. Prepend `~/.cargo/bin` to `PATH`. Run GUI apps with `DISPLAY=:0`; capture with `scrot`, drive with `xdotool` |
| Windows AMD64 `WINDOWS_SSH` | Windows 10, PowerShell, output in GBK. The SSH session has no desktop, so start GUI apps with `schtasks /create /it`. Drive C: is low on space |
| Linux AMD64 `LINUX_AMD64_VM_SSH` | Off by default; it is a VMware guest on the Windows host. Start it with `& "C:\Program Files\VMware\VMware Workstation\vmrun.exe" -T ws start "D:\Virtual Machines\Block-Debian-AMD64\block-debian-amd64.vmx" nogui` and stop it with `... stop <vmx> soft` |

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
- [ ] **P3 Inventory:** list every prototype page, state and feature in `STATUS.md` as the completion checklist.
- [ ] **P4 Pages.** Each page ships with its full real backend:
  - [ ] Home
  - [ ] Accounts and sign-in: Microsoft PKCE and Device Code; Authlib-injector with provider management and launch injection; Offline
  - [ ] Games and Game Wizard:
    - Bundled and Separated modes
    - Fabric, Quilt, Forge (modern and legacy), NeoForge (full version list) and OptiFine
  - [ ] Settings: all five sections, including SOCKS5 and transactional storage relocation
  - [ ] About and Welcome
  - [ ] Cross-page work: motion, reduced motion, short displays and the remaining inventory items (for example Quick Play)
- [ ] **P5 Real verification:**
  - Supported install and launch smoke combinations on all four hosts, with actual main-menu verification; legacy Forge on Windows and legacy ARM64 as best effort.
  - State legacy ARM64 outcomes and unsupported combinations explicitly.
  - User visual acceptance from native screenshots and the comparison board on 2026-10-22.
  - Live sign-in with the user in the morning of 2026-10-22, Asia/Taipei, when available.
  - CI green.

Pending required user acceptance, live sign-in or smoke runs must remain pending. Report implementation readiness separately from Pilot completion.

## Needs the user

- [x] Grant macOS Screen Recording permission to the terminal running Codex.
- [ ] Enable "Allow public client flows" on the Azure app registration. Device Code sign-in requires it.
- [ ] Sign in with the genuine Microsoft account in the morning of 2026-10-22, Asia/Taipei, when available; batch host requests and do not start expiring codes ahead of time.
- [ ] Provide an Authlib provider account if one should be tested live.
- [ ] Download the required OptiFine JAR from optifine.net when needed; Block then handles installation automatically.
- [ ] Review the native screenshots and side-by-side board on 2026-10-22; independent implementation continues while review is pending.

## Deferred to v0 (architecture governance)

- Per-page entities, replacing engine polling with async events, splitting `lib.rs` and `game_wizard.rs`.
- Lint tidy-up and dead-abstraction cleanup.
- Accessibility, packaging, signing and release.
- Mirror download sources (BMCLAPI).
