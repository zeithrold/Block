# Block: Pilot Task List

Date: 2026-10-21, updated 2026-10-23 with the Milestone III assessment backlog. This file replaces `gpui-ui-parity-plan.md` and `gpui-ui-parity-goal-prompt.md` as the entry point. Milestone II execution prompt: [codex-goal-prompt.md](codex-goal-prompt.md).

## Development milestones (user decision, 2026-10-23)

| Stage | Focus | Owner / current state |
| --- | --- | --- |
| Pilot Milestone II | Basic usability: real features and end-to-end flows work. Interaction polish is deferred. | DeepSeek; complete at `f2a5fd7`, with documented compatibility and environment boundaries. |
| Pilot Milestone III | Interaction quality: inspect and fix interactions one by one, including visual presentation, navigation, input, focus, overlays, motion and feedback. Architecture design is deferred. | Codex; initial screenshot/source assessment complete at `5538bf3`; implementation and native interaction acceptance pending. |
| Pilot Milestone IV | Begin architecture design, using the working product and interaction findings as inputs. | Future stage; implementation of a redesign needs its own scope. |
| v0 | First public unstable release. | Future stage; release preparation and publication are separate from Pilot completion. |

These stages supersede the earlier combined Pilot acceptance gate and the rule that architecture waits for v0. Milestone II still requires real behavior rather than placeholders, safe persistence, and explicit compatibility limits. An interaction defect that blocks a core flow remains a Milestone II usability issue; visual or interaction refinement belongs to Milestone III. Only the user's review sets Accepted in Milestone III. Missing visual acceptance does not by itself block Milestone II completion.

## Where we are (2026-10-23, reviewed HEAD `f2a5fd7`)

- DeepSeek completed Milestone II under the scoped v8 prompt in [codex-goal-prompt.md](codex-goal-prompt.md). Codex's handoff review confirmed artifact counts and links; the subsequent Milestone III screenshot/source assessment found content and comparison issues listed below. Neither review repeats native E2E or sets Accepted. v6/v7 are in Git history.
- CI: latest completed four-platform run is green at `df2db40` (run 39885268348); `c8406ad` and `f2a5fd7` documentation runs are still in progress at this review. Windows ARM64 is green with an uploaded artifact at `82b3847` (run 39823711474). Pending runs retain their own result.
- Native app evidence:
  - macOS: live PKCE and Device Code; Welcome → Vanilla → main menu → Stop; relaunch and play without signing in again; overlay, IME and real quick-link drag checks passed.
  - Windows AMD64: Device Code; atomic Welcome; Vanilla → main menu → Stop; Q6 loader flows (Fabric/Quilt/Forge/NeoForge/legacy Forge), separated mode, crash help, interrupted-download retry, real quick-link drag.
  - Linux ARM64: native 1.16.5/1.18.2 wizard → Play → menu → Stop with Azul runtimes; LWJGL2 picker restriction.
  - Linux AMD64: native Welcome → Vanilla → menu → Stop; Q9 D6 short-display/reduced-motion/input checks; Authlib fixture sign-in + real avatar + injected 1.20.6 menu; provider icons; X11 quick-link drag.
  - Windows ARM64: native launcher build, Welcome, catalog, install, offline account and restart persistence passed; game rendering is blocked by the VM's missing OpenGL ICD, a recorded best-effort boundary.
- Windows Settings/SOCKS5/storage, loader and older-version flows, OptiFine re-import, queued Play, and lifecycle race fixes have evidence in STATUS.md and target/parity-shots/.
- Milestone II evidence complete: screenshot set prepared (native matrices, mapping manifest, React references/explained gaps, E2E index, per-page real-data captures, master index), plus post-revoke live re-sign-in and avatars on all five platforms. Game-menu/relaunch checks passed on macOS, Windows AMD64 and Linux ARM64; Linux AMD64 live-account game launch was skipped for time, and Windows ARM64 rendering is environment-blocked. Milestone III (Codex) owns interaction review and acceptance.
- Milestone III backlog: follow the ordered checklist below, including macOS system reduced motion, transition-frame audit and actual Settings/Accounts fit at 1280×720. Each platform has 142 native scenario images; the manifest pairs 60 stems with React references, explains 11 native-only stems and maps/explains 4 proto-only names. The remaining 30 board placeholders are explained and local links resolve, but this is structural coverage: three real-data captures show the wrong page, and some same-name pairs represent different tasks or states. Correct these before visual acceptance.
- Git audit: all 14 Block worktrees are clean and all local branch commits are reachable from fetched remote refs. The screenshot directory is ignored and remains local; uploading source does not upload it. The user keeps Block-UI local without committing/uploading its capture-tool changes.

## Pilot Milestone III: ordered To-Do

Assessment baseline: Native `5538bf3`; prototype `fd2713b` with the existing local capture-tool changes. The assessment inspected all 71 Windows light scenario comparisons, representative dark/macOS scenarios, the 72 real-data page captures, current source and archived live/motion evidence. It did not produce a new native interaction PASS. All items below are pending; their IDs match the assessment findings. P1/P2/P3 indicate experience priorities, not security severity.

Judge the product by task clarity, truthful state and recoverability. Use the prototype as a reference, not an unconditional pixel target. Preserve fixed-size windows, platform-native decorations, in-window overlays, published unmodified GPUI crates, the three-step Welcome flow, real authentication, official-source installation, safe persistence and queued launch. Keep architecture design in Milestone IV and accessibility work deferred. Only the user's review sets Accepted.

### 0. Correct the review evidence first

- [ ] **E01 — Replace three incorrect real-data captures.** Windows `windows-amd64/pages/page-light-accounts.png` shows Games; macOS `macos-arm64/pages/accounts-light.png` and `account-offline-detail-light.png` show Home and are byte-identical to `home-light.png`. The corresponding dark account captures are correct. Preserve the original evidence and record the replacement capture's commit, data state and method. Make capture scripts assert the expected page title, viewport and unique page content before saving. File counts and existing links alone do not prove coverage.
- [ ] **E02 — Audit semantic equivalence in the comparison manifest.** Separate account removal from game removal (`remove-confirmation`), provider creation from existing-provider editing (`provider-add`), Fabric from Vanilla (`wizard-loader-picker`), search-no-results from no-compatible-builds (`wizard-loader-empty`), and closed authorization from expired authorization (`auth-error`). Compare the different Welcome flows as task strategies. Record object, trigger, data, viewport, focus, scroll position, commit and comparable scope; identify intentional reuse of list/detail screenshots rather than counting it as independent coverage.
- [ ] **E03 — Make fixtures represent the states under review.** Add a Home failure scenario with an actual Failed installation target; the current `home-error` only changes status text, while the real Failed branch already offers Retry download. Keep search-empty, incompatible, loading and request-failure states distinct. Record that scenario captures force reduced motion and use synthetic identities/initials; use archived live skins for avatar review and recordings for motion review. Do not infer a functional defect from an incomplete fixture.

Exit criterion: expected pages and task states are visibly correct, comparisons state their limits, and static fixtures are not presented as live behavior or motion acceptance.

### 1. Correct action and state semantics

- [ ] **S01 — Make Back, Cancel, Close and Finish mean different, consistent things.** The account wizard's top Back cancels the whole flow, while its footer Back changes steps; completion still exposes exits that discard the prepared identity. Reserve Back for a previous step, label task exit by its consequence, and emphasize Finish on completion. Apply the same rule to game editors where Back currently discards drafts. Protect costly prepared state where needed without adding confirmation to every navigation action. Verify the destination, discarded state and restored focus through the native app.
- [ ] **S02 — Distinguish drafts, previews, durable saves and immediate actions.** Replace `Saved for this session` in persistent Settings with accurate saved-state wording. Give irreversible, immediately applied Profile mode conversion a footer that cannot imply Cancel undoes it or Save is still required. Make appearance preview and leaving Settings with unsaved edits understandable; explicitly evaluate draft retention versus a targeted discard prompt instead of silently introducing autosave. Verify save/restart, cancel and conversion behavior separately.
- [ ] **S03 — Put player tasks ahead of implementation details.** Replace first-level UTF-8 byte limits, raw byte counters, refresh-credential terminology and catalog/transaction descriptions with clear outcomes and next actions. Keep necessary validation and truthful unknown states. Use task summary, recovery details and optional diagnostics as separate levels; never invent file names, overall percentages or remaining time.
- [ ] **G01 — Keep Bundled mode's visible objects coherent.** Avoid presenting the same Game and Initial Profile as separate beginner decisions. Make removal titles and buttons name the same object; reserve explicit Instance/Profile relationships for Separated mode. Distinguish the inspected item, the Home selection and a running session; reconsider `In use` where it only means selected. Preserve the existing catalog model and one-way conversion policy.

Exit criterion: a user can predict what each action changes, what survives leaving/restarting, and whether a label describes selection, installation or a running game.

### 2. Complete the daily launch flow

Review Home → switch game/account → Launch → download or queue → Running → Stop → launch again as one continuous task before doing broad cosmetic changes.

- [ ] **H01 — Give running and stopping their own task presentation.** Remove installation-resource counts such as `0 resources verified · overall total unknown` from Running; eliminate repeated Running/PID text and keep PID in diagnostics. Present preparation, download/verification, queued launch, starting, running, stopping and failure distinctly. Verify that Stop provides continuing feedback until the actual session state changes.
- [ ] **H02 — Make account switchers focus on selection.** Move per-row Remove actions into account management or a clearly secondary menu. Reduce repeated environment text and redundant search controls, while preserving enough provider information to distinguish same-name identities. Show Clear when relevant. Verify selection, keyboard navigation, Escape and return focus in the actual sheet.
- [ ] **H03 — Make the launch target and queue consequences explicit.** Keep portrait Home, but make the current game and necessary version/loader information readable near the primary action. Review which target queued launch binds to, how selection changes affect it, and how users cancel the queue without confusing that action with stopping installation. Distinguish no-account, no-game and fully empty Home states; communicate that preparing/downloading a game does not require sign-in.
- [ ] **A01 — Correct provider-specific identity markers.** Accounts currently share a generic Account icon path that adds a blue Microsoft-style Grid marker and badge treatment even for Offline identities. Represent Microsoft, Offline and Authlib accurately; use a real provider icon when available and an explicit fallback otherwise. Verify both themes with mixed account types. This is a presentation defect, not evidence that authentication used the wrong provider.
- [ ] **H04 — Improve real skin rendering without replacing identity data.** Preserve crisp Minecraft face/hat pixels when enlarged; inspect the current scaled full-skin crop and renderer sampling on real displays. Keep transparency, real cached skin data and initials fallback. Use suitable smooth scaling for ordinary provider artwork rather than forcing all images into pixel rendering. Verify Home, list and selector sizes on Windows and macOS.
- [ ] **D01 — Organize downloads by installation/repair task.** Lead with the game, current phase, known progress and recovery actions. Move observed-resource verification groups, cache hits, hashes and transport counters into details. Keep progress indeterminate when the full total is unknown. Evaluate file detail as a subview of the same sheet instead of an additional dialog; if stacking remains, verify Back/Escape order and scroll restoration. Confirm retry preserves the existing game and queue behavior remains clear.

Exit criterion: the selected target is clear, queue/stop/retry consequences are predictable, running UI contains no installation-only residue, and interrupted installs recover without duplicate games.

### 3. Refine game management and creation

- [ ] **G02 — Rebalance game details and contain long paths.** Surface the useful game/installation/launch summary before repeated immutable-component explanations push preferences below the fold. Explain fixed version/loader choices once. Show a managed-location label or short path by default, with a way to inspect/copy/open the complete location; constrain wrapping or ellipsis inside cards. Verify long Windows/macOS paths and names without shrinking all text.
- [ ] **G03 — Make the OptiFine entry reach the relevant editor.** OptiFine and Instance settings currently both open the general Instance editor, whose initial fields precede the OptiFine section. Either provide a focused OptiFine editing context or reveal/focus that section reliably. Retain the game identity, official-JAR import, compatibility checks, cancel/save behavior and reinstall semantics. Verify the native landing position and draft handling.
- [ ] **W01 — Choose containers by task complexity.** Evaluate right-pane subpages for version, loader, runtime/location and OptiFine tasks that need long lists or multiple fields; keep short confirmations as Dialogs and quick selections in appropriate sheets/popovers. Preserve the single-window/component-library requirements. Avoid duplicate background titles/Done footers behind a section dialog. Reduce first-level catalog/keyboard explanation so useful results remain visible, while retaining explicit ARM64 disabled-version reasons.
- [ ] **W02 — Fit component tiles to normal content.** Eliminate default-state truncation such as `Automatic Ja…` and `Download, then sel…` using concise values and complete essential status. Compare four-across tiles with two-by-two or compact-row variants within fixed windows. Keep the four component concepts without treating square geometry as a requirement; do not hide required information solely in a tooltip.
- [ ] **W03 — Make review emphasize actual choices.** Lead with game, version/loader, account, intentional overrides and the consequence that creation begins downloads. Put exhaustive defaults behind an expandable summary; replace undifferentiated Change links with clear targets. Remove redundant Bundled Game/Profile summaries. Verify that change/return preserves drafts and failed downloads remain retryable after successful creation.

Exit criterion: entry points land in the intended context, common fields and decisions are easy to find, normal content is readable, and review explains the impending action without exposing unnecessary internal concepts.

### 4. Refine account and settings tasks

- [ ] **A02 — Make the default Microsoft action easy to find.** Put the primary sign-in action near its explanation and keep Device Code as a clear alternative. Keep copy/open/approve/wait actions in a readable order on the device screen; avoid duplicated waiting/security explanations that push browser recovery below the fold. Preserve real browser authorization, source URLs, expiry and cancellation semantics; do not reproduce the prototype's local approval simulator.
- [ ] **A03 — Adapt account flow presentation to the identity type.** Evaluate a shorter local-name/confirmation presentation for Offline accounts and an explicitly named character-selection stage for multi-character Authlib accounts, instead of forcing identical four-step wording. Show real character names/avatars where available and put UUIDs in details. Preserve transient preparation, final confirmation and secure persistence; verify each provider's back/cancel/finish paths.
- [ ] **A04 — Keep the Authlib trust destination visible.** Present provider identity, actual endpoint and HTTP(S) context near credential entry. Unify Identify/Recognize wording and distinguish recognized, existing and unsaved providers. Keep drag, paste and manual entry, including an honest Wayland fallback. Give confirmable HTTP risk a consistent treatment distinct from operational failure, while preserving explicit acknowledgement and password/token boundaries.
- [ ] **T01 — Explain effective settings and their scope.** Make new-game defaults, this game's settings and Separated Profile overrides understandable through effective values and their sources, not only inheritance prose. Keep existing games unchanged by new defaults. Consolidate the memory input, step controls and repeated value into a compact control without losing the 1–32 GB policy, validation or IME protections. Verify save/cancel and new-versus-existing behavior through native flows.
- [ ] **T02 — Separate storage defaults from migration decisions.** Distinguish the future game library, shared cache location and moving existing managed games. Before migration, identify source, destination, affected data and retained-copy policy; during migration show readable phases/units; afterward identify the active location and any old copy. Make interrupted rollback and cleanup targets clear. Preserve copy verification, recovery and explicit cleanup; do not invent disk savings/ETA or silently remove old data.
- [ ] **T03 — Describe connection tests within their actual scope.** Keep results beside the current proxy draft and state that testing does not save it. Describe the endpoint tested without implying every CDN/loader service is healthy. Keep retry/concurrency/speed limits readable and move low-level transport qualifications into secondary detail. Verify draft edits invalidate stale results and a later Save persists the intended configuration.

Exit criterion: users can identify the service receiving credentials, understand what Finish saves, distinguish settings scopes, and explain which storage copy is active after success, cancellation or failure.

### 5. Converge visual language, motion and platform behavior

- [ ] **S04 — Standardize visual hierarchy across complete tasks.** Define consistent page/section headings, field labels, values and secondary explanations, building on the shared font, palette, sidebar and footer tokens. Improve differentiation between selected, hovered and focused controls; compare a less dominant input focus treatment without losing feedback. Verify light/dark and both primary desktop platforms; do not misclassify OS font rasterization as a broken font implementation.
- [ ] **M01 — Review actual transition behavior, not matching timing constants.** The prototype animates outgoing and incoming content; Native currently renders an incoming translated/faded layer. Review continuity, scope, interruption and Home/management geometry together. Compare shorter or smaller transitions for frequent actions rather than enforcing 460ms everywhere. Preserve reduced-motion behavior and avoid introducing a broad rendering/state rewrite to imitate the prototype.
- [ ] **M02 — Verify content budgets in fixed windows.** Test actual Settings/Accounts at 1280×720 on macOS, plus Home, low-position windows, scaling differences and long content on the other platforms. Keep primary actions stable and content scrolling bounded; adapt padding, secondary text and column/tile layouts to available height. Check the whole system frame as well as client bounds: the current fit helper estimates space from display bounds and retains a 540px minimum, not an exact OS work-area guarantee.
- [ ] **M03 — Complete native interaction verification.** Exercise Tab/Shift-Tab order, top-layer-only Escape, return focus, search/list scroll restoration, visible/focused validation errors, Chinese IME, rapid forward/back navigation and every motion scope under system/app reduced-motion settings. Retain the pending macOS system Reduce Motion and transition-frame checks. Separate genuine keyboard/mouse results from synthetic-input limitations; static scenario captures do not satisfy this item.
- [ ] **T04 — Refine About and define release traceability separately.** Lead with product identity, Pilot/unstable channel, version and help; make output/build diagnostics secondary and copyable. Preserve truthful missing commit/fingerprint information in local builds. Define required source metadata for v0 release artifacts without treating this assessment or a wording change as release readiness.

Exit criterion: full tasks remain readable in both themes and fitted windows, real skins are clear, motion is coherent and interruptible, and actual input/platform checks have explicit evidence. Submit grouped review to the user; leave Accepted pending until reviewed.

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
  - [x] Home, including real account avatars (Authlib fixture native; Microsoft live skin on all five platforms)
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
  - [x] Milestone II: complete screenshot evidence set prepared (`target/parity-shots/m2-artifacts.html`: matrices, mapping, references/explained gaps, E2E index, per-page real-data captures).
  - [ ] Milestone III: complete the comparison board and user visual/interaction acceptance using that set.
  - [x] Milestone II: app access revoked, then live-account re-sign-in and real avatars verified on all five platforms; macOS, Windows AMD64 and Linux ARM64 passed game-menu/relaunch checks. Linux AMD64 live-account game launch was skipped for time; Windows ARM64 rendering is environment-blocked by the missing OpenGL ICD.
  - [x] CI green at completion: four platforms + manual windows-arm64 workflow green (one Linux ARM64 help-claim flake bounded at 0/70).

Report Milestone II usability, Milestone III interaction acceptance, Milestone IV architecture design and v0 release readiness separately. Recorded skips and platform limits remain explicit; Milestone III acceptance is not a Milestone II gate.

## Needs the user

- [x] Grant macOS Screen Recording permission to the terminal running Codex.
- [x] Provide the official OptiFine 1.21.1 JAR.
- [x] Enable "Allow public client flows" on the Azure app registration. Device Code sign-in requires it.
- [x] Sign in with the genuine Microsoft account (macOS PKCE/Device Code, Windows, both Linux hosts done).
- [x] Revoke Block's app access in the Microsoft account (a refresh token was printed once in a diagnostic transcript), then re-sign in for the final live-account check (done on all five platforms; re-revoking the new test grant is optional).
- [x] Install the Rust platform on the Windows ARM64 VM (native build + tests run there).
- [x] Linux AMD64 keyring was unlocked for the live check; it relocks after a reboot and only needs attention again for future saved-account runs.
- [ ] Milestone III (Codex): review native screenshots and behavior page by page after reference/mapping gaps are resolved or explained.
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
