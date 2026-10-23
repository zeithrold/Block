# Subagent task: KDE Xwayland chrome compatibility

Requested by the user on 2026-10-23. Scope: exploration and native compatibility testing; not a Home redesign or production chrome rollout.

This investigation is complete; see [xwayland-compatibility-investigation.md](xwayland-compatibility-investigation.md). The user subsequently authorized a narrow production follow-up for titlebar removal and custom dragging, again assigned to GPT-6 Luna/high. The assessment-only editing restrictions below describe the completed investigation; the follow-up's current scope is tracked in M04 of [next-tasks.md](next-tasks.md).

Runner decision: the user replaced the proposed DeepSeek Harness dispatch with a GPT-6 Luna subagent at high reasoning effort. The earlier external DeepSeek launch was rejected by automatic approval review and never started; it must not be retried as part of this task. The user has explicitly authorized this Codex subagent investigation instead. Credentials are for normal authentication only, never task content or logs.

## Objective

Determine whether Block's published GPUI stack supports removing the system titlebar under KDE Xwayland while preserving working drag, minimize, restore, close, in-window dialogs and the fixed-window policy. Resolve the incomplete interaction results from the Codex capability probe. Separate actual product/backend defects from input-automation, focus, coordinate and timing mistakes. A source-only conclusion or a visible titlebar-free screenshot is insufficient for an interaction PASS.

## Read first

1. `docs/window-chrome-feasibility.md` and its linked receipts.
2. `docs/next-tasks.md`: current product decisions, host layout and M04.
3. `crates/block-ui/examples/window_chrome_probe.rs`.
4. `target/chrome-probe/linux-arm64/x11-fixed-output.json`, `x11-fixed-actions.json`, `post-close-check.json` and `cleanup.json`.
5. `target/chrome-probe/fixed_actions_remote.py`, `wayland_actions_remote.py`, `uinput_daemon.py`, `remote.py` and the existing collection/cleanup helpers. These are local investigation materials, not production APIs.

Repository: `/Users/zeithrold/Documents/Develop/Native/Block`. The canonical plan file is `docs/next-tasks.md`; there is no separate next-step file. The prototype repository is a read-only reference and must not be committed/uploaded.

## Starting evidence and environment

- Main is `140b490` before this task. Codex has uncommitted changes in `docs/STATUS.md`, `docs/next-tasks.md`, the new chrome feasibility report and the new probe example. Preserve them. Inspect current status first in case state changed.
- The remote test checkouts were at `df2db406d082c9785aaffb6fa8946fdd841db46f`. A local diff of `crates/`, `Cargo.toml` and `Cargo.lock` against `140b490` was empty before adding the probe. Verify provenance if you change the native source.
- Published stack: GPUI/platform backends 0.3.9; gpui-component and gpui-base 0.8.0. No GPUI patch/vendor copies are allowed.
- Windows AMD64, XFCE native X11 on Linux AMD64, and KDE native Wayland on Linux ARM64 passed the fixed-control probe's core interactions. Do not repeat unrelated account/game tests.
- KDE Xwayland on Linux ARM64 removed system decorations: `_MOTIF_WM_HINTS` had decorations off, `_GTK_FRAME_EXTENTS` showed 20 px per side, and the custom header was visible. X11 input subsequently incremented the content counter and activated Close. Prior drag attempts did not move the window, and the native-input automation run did not activate its intended controls. This is incomplete evidence, not a proven drag defect.
- KDE reported frame/client geometry differing by the CSD shadow insets; do not assume `xdotool`, `xwininfo`, KWin and GPUI coordinates describe identical rectangles. For example, a 680 × 460 X11 drawable surrounded a 640 × 420 frame in one run.
- The generic component TitleBar's maximize control and double-click zoom enlarged Linux windows despite `is_resizable: false`. `--fixed-controls` removes those two actions. Its ordinary edge/double-click tests passed in XFCE and native Wayland, but KWin still advertised resize capability. Window-manager shortcuts and other resize paths require explicit evidence before claiming strict fixed sizing.
- Remote Linux ARM64 credentials/target are in `.env` under `LINUX_ARM64_VM_SSH`. Read values only as needed and never print secrets. The prior KDE session used DISPLAY `:1`, WAYLAND_DISPLAY `wayland-0`, `/run/user/1000` and a dynamically named Xauthority file. Rediscover the live session environment; do not hardcode an old authority filename.
- Probe binary remains at `~/block-src/target/debug/examples/window_chrome_probe` on test hosts. Its temporary source was removed after matching SHA-256 `16f2b4168da25890fb42839fe668824f78d35c980fe702fbf2f1119e8991c986`; source remains in this local workspace. Copy/rebuild explicitly if you need to modify it.
- All test probe processes, the Windows temporary task and the Wayland virtual input device were cleaned up. An existing unrelated Block live-data session was present on Linux ARM64; do not terminate it or operate on its data.

## Investigation work

1. Verify the actual backend using process/session environment and X11 window-manager properties. Distinguish native Wayland, Xwayland and native X11 rather than relying only on a launcher flag.
2. Establish active/focused window, visual content coordinates, CSD shadow/border offsets, scaling and pointer position. Check whether titlebar text/children intercept the drag region or initial pointer events. Record actual input delivery before treating an inert action as a backend failure.
3. Trace the published GPUI X11 `start_window_move` / `_NET_WM_MOVERESIZE` path and the KDE/Xwayland requirements, supported by native observations. Compare native desktop input with XTest if useful; do not assume that one automation path represents physical input. Prefer local source; use primary upstream sources if external research is needed.
4. Validate the minimal header's drag, minimize/restore, close, content buttons, dialog background blocking and Escape dismissal. Measure geometry/state before and after; label unsupported or unverified actions honestly.
5. Check fixed-window behavior beyond the two removed controls where practical. Identify which restrictions the public APIs enforce, which are just absent UI actions, and which the window manager can override. Explain any consequence for the product's fixed-size requirement.
6. If a probe/coordinate/input correction resolves the gap, demonstrate the corrected native sequence. If a backend issue remains, produce a minimal reproduction and a bounded proposed remedy compatible with unmodified GPUI, or explicitly explain the remaining boundary. Do not implement a broad production workaround.

## Scope and operating rules

- Use one GPT-6 Luna investigator at high reasoning effort. Do not delegate further or perform unrelated milestone work.
- You may create dedicated scripts, adjust a task-owned copy of the probe, install small necessary test utilities and run native builds/tests on the existing authorized hosts. Preserve prior receipts; write new artifacts under `target/chrome-probe/deepseek-xwayland/`.
- Do not overwrite Codex's uncommitted files or old evidence. Prefer a task-owned probe copy when experimenting; record exact source and dependency fingerprints. Do not alter `crates/block-app/src/` or the production UI as part of this exploration.
- Keep the prototype's restrained Home unchanged. macOS traffic-light deletion is mandatory but outside this task; do not start macOS implementation. Architecture remains Milestone IV.
- Do not change authentication, download sources, game/loader settings, user Java installations, player files, locks or game sessions. Do not inspect or use live account credentials. Do not run heavy work concurrently on Windows AMD64 and Linux AMD64, which share a physical host.
- No commits, pushes, PRs or messages to third parties. Do not restart the historical Milestone II goal or set Accepted.
- Clean up only task-owned windows/processes, virtual devices, temporary desktop settings and helper services. Restore any desktop settings you change and verify restoration. Do not leave caffeinate or unrelated background work running.

## Deliverables

- An English report at `docs/xwayland-compatibility-investigation.md` with provenance, exact native environment, reproduction, root cause or remaining uncertainty, per-action verdicts, evidence links, cleanup and a recommended next step.
- Reproducible task-owned scripts/source and raw receipts under `target/chrome-probe/deepseek-xwayland/`.
- An English proposed To-Do delta in the report for Codex to reconcile into `docs/next-tasks.md`; do not edit that dirty shared file yourself.
- Final handoff in Chinese summarizing what is proven, what remains unverified, and whether production integration can proceed without GPUI patches. Do not describe automated-input limitations as verified product defects.
