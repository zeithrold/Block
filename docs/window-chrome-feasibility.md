# Window chrome feasibility: Windows and Linux

Date: 2026-10-23. Scope: native capability assessment, not production chrome implementation or visual acceptance.

## Decision supported by the tests

Block can remove the Windows system titlebar and use Linux client-side decorations through the published, unmodified GPUI APIs. Native probes demonstrated working drag, minimize, restore, close and in-window dialog interaction on Windows AMD64, XFCE X11 on Linux AMD64, and KDE Wayland on Linux ARM64. A subsequent GPT-6 Luna investigation also verified these core actions on KDE Xwayland using the input paths documented below. System titlebars are not a mandatory product constraint.

The default component TitleBar must not be copied unchanged into a fixed-size Linux window: its maximize button and double-click zoom can enlarge the window. A second probe with only minimize/close controls and a drag region preserved size during ordinary edge dragging and header double-clicks on the three environments above. These are capability results, not a completed integration into Home.

macOS traffic-light removal remains a separate mandatory requirement; this assessment did not test macOS or Windows ARM64.

## Method and provenance

- Reproducible harness: [window_chrome_probe.rs](../crates/block-ui/examples/window_chrome_probe.rs). Build with `cargo build --locked -p block-ui --example window_chrome_probe`; run with `--system` for the baseline, no arguments for the component TitleBar, or `--fixed-controls` for the minimal fixed-window variant.
- All three native builds used remote source baseline `df2db406d082c9785aaffb6fa8946fdd841db46f`. The local comparison of that baseline with reviewed main `140b490` found no changes under `crates/`, `Cargo.toml` or `Cargo.lock` before adding the probe. Application code and dependency selection therefore match for this capability assessment; the production launcher was not rebuilt with replacement chrome.
- Published dependencies: `gpui-pre`/platform backends 0.3.9, `gpui-component` and `gpui-base` 0.8.0. No dependency patch, vendoring, swizzling or new native shim was used for window behavior.
- Final probe source SHA-256, verified on all three hosts: `16f2b4168da25890fb42839fe668824f78d35c980fe702fbf2f1119e8991c986`.
- Follow-up KDE Xwayland investigation: GPT-6 Luna at high reasoning effort used a task-owned instrumented copy while preserving the canonical probe. Its source/binary fingerprints, corrected input coordinates and raw receipts are recorded in [xwayland-compatibility-investigation.md](xwayland-compatibility-investigation.md). No external DeepSeek Harness run was started.
- The probe opens a disposable 640 × 420 requested window and the same Base Root/component overlay stack used by Block. Its literal size label denotes requested bounds, not a measurement of the actual content area; native measurements below are authoritative. It does not authenticate, install games, read player data or start Minecraft. Its test header is not a proposed Home design.
- Windows input ran inside the interactive desktop through a temporary scheduled task. Geometry came from Win32 client/window rectangles and minimization state from `IsIconic`.
- XFCE input used desktop X11 mouse/key events. Window-manager properties, geometry and cropped screen images supplied independent evidence.
- Native Wayland input used a temporary kernel input device because X11-only input did not drive the Wayland window. KWin window state and geometry supplied independent evidence; restore used KWin's window-management API. The temporary device was removed afterward.
- Local probe `cargo fmt --all -- --check` and `cargo clippy --locked -p block-ui --example window_chrome_probe -- -D warnings` passed. This was not a full workspace regression run.

## Verified native matrix

| Environment | System titlebar removed | Drag | Minimize / restore | Close | Dialog blocks content / Escape returns | Ordinary edge drag / double-click preserve size |
| --- | --- | --- | --- | --- | --- | --- |
| Windows 10 LTSC AMD64 | Yes; native top inset 31 px → 0 px | Yes; position moved +80/+40 | Yes / yes | Yes | Yes | Yes, fixed-control variant |
| Linux AMD64, XFCE X11 | Yes; decoration hint off, native frame extents 0/0/0/0 | Yes; position moved +54/+36 | Yes / yes | Yes | Yes | Yes, fixed-control variant |
| Linux ARM64, KDE native Wayland | Yes; client-side decoration and equal KWin frame/client bounds | Yes; position moved +54/+36 | Yes / yes | Yes | Yes | Yes, fixed-control variant |
| Linux ARM64, KDE Xwayland | Yes; decoration hint off and custom header visible | Yes through uinput; position moved +48/+32 | Yes through XTest / KWin restore | Yes through XTest | Yes, mixed uinput/XTest | Edge drag unchanged; strict WM constraints remain unproven |

The initial KDE Xwayland result was incomplete. The follow-up sampled fresh drawable/frame geometry and found that old coordinates hit a 20 px CSD inset rather than the visible header. With corrected coordinates, XTest delivered a GPUI move request but did not move the window in that attempt; uinput delivered a move request and moved KWin's frame +48/+32. Close passed through XTest after a uinput click produced no callback; preserve both observations. These results establish working core actions, not universal equivalence of all synthetic-input paths. The exact reason for the XTest/uinput difference was not established. See the [follow-up investigation](xwayland-compatibility-investigation.md).

## Evidence

All screenshot/log artifacts below are local and ignored by Git; committing this document does not upload them.

| Environment | Before / after or final screenshot | Raw interaction evidence |
| --- | --- | --- |
| Windows | [System](../target/chrome-probe/windows-amd64/fixed/system.png), [custom chrome](../target/chrome-probe/windows-amd64/fixed/client.png), [dialog](../target/chrome-probe/windows-amd64/fixed/overlay.png) | [Geometry baseline](../target/chrome-probe/windows-amd64/fixed/initial.json), [interactions](../target/chrome-probe/windows-amd64/fixed/actions.json), [events](../target/chrome-probe/windows-amd64/fixed/client.log) |
| XFCE X11 | [Custom chrome](../target/chrome-probe/linux-amd64/fixed-initial.png), [dialog](../target/chrome-probe/linux-amd64/fixed-overlay.png), [final](../target/chrome-probe/linux-amd64/fixed-final.png) | [Interactions](../target/chrome-probe/linux-amd64/fixed-actions.json), [initial decoration properties](../target/chrome-probe/linux-amd64/x11-initial.json) |
| KDE Wayland | [Custom chrome](../target/chrome-probe/linux-arm64/wayland-fixed-final.png), [dialog](../target/chrome-probe/linux-arm64/wayland-fixed-overlay.png) | [Interactions](../target/chrome-probe/linux-arm64/wayland-fixed-actions.json), [events](../target/chrome-probe/linux-arm64/wayland-fixed.log), [delayed process-exit check](../target/chrome-probe/linux-arm64/post-close-check.json) |
| KDE Xwayland, follow-up | [Custom chrome](../target/chrome-probe/deepseek-xwayland/remote-receipts/initial.png) | [Native-input drag](../target/chrome-probe/deepseek-xwayland/remote-receipts/drag-uinput-corrected.json), [minimize/restore](../target/chrome-probe/deepseek-xwayland/remote-receipts/minimize-xdotool.json), [close](../target/chrome-probe/deepseek-xwayland/remote-receipts/close-xtest.json), [overlay](../target/chrome-probe/deepseek-xwayland/remote-receipts/overlay-uinput-xwayland.json) |

The first direct X11 window-image capture on XFCE was black because it did not capture the composited GPU output. Preserve it as failed capture evidence; use the later cropped screen captures above for visual review. Windows measurements and the independent window-manager properties prevent screenshot appearance alone from deciding titlebar removal.

The Wayland fixed-control receipt sampled process existence too soon after close and recorded `closed: false`; the close event was already logged. A later process check confirmed exit. Preserve both observations rather than silently rewriting the raw receipt.

## Remaining boundaries and integration requirements

1. **Fixed-size semantics:** the default Linux TitleBar enlarged a window to 1280 × 756 through its maximize control; header double-click also enlarged the X11 component probe. The minimal variant removes those actions and passed the tested edge/double-click paths in the initial verified environments. KWin still advertised the window as resizable. The Xwayland follow-up confirmed that the X11 backend's WM size hint maximum was 16384 × 16384 rather than fixed dimensions despite `is_resizable: false`; its tested edge drag left size unchanged, while shortcut delivery was not independently proven. Window-manager shortcuts, tiling and external resize commands are not certified by these results; production integration must verify or enforce the fixed-window policy beyond simply hiding a button.
2. **Actual content bounds:** Windows retained a 640 × 420 client area. XFCE's tested 640 × 420 drawable included 20 px CSD insets per side; KDE Wayland reported a 600 × 380 frame/content area for the requested bounds. KDE Xwayland also showed a 680 × 460 drawable around a 640 × 420 frame in one probe. Do not transfer old outer-size constants blindly. Fit Home and management using measured content bounds, scaling and decorations, and review short displays again.
3. **Windows decoration reporting:** GPUI's generic decoration query reported Server even when the native Windows titlebar was hidden. Its decoration option is primarily a Linux distinction. Use Win32 geometry and rendered controls to judge Windows; the 31 px → 0 px inset change is the relevant evidence.
4. **Linux environment fallback:** upstream X11 requests can fall back to server decorations when detected CSD requirements are unavailable, and Wayland decoration mode is negotiated with the compositor. Temporarily setting XFCE's compositing preference to false still yielded Client mode in this session; that experiment did not reproduce or certify a truly unsupported-compositor fallback. The preference was restored to true. Other desktop/compositor combinations remain unverified.
5. **Production integration:** move only the necessary close/minimize/drag behavior into Block's existing restrained top region. Recheck overlays, focus, minimum content budgets, DPI, fitted window placement and startup minimize/restore. Do not adopt the probe's styling as a redesign.
6. **Game lifecycle:** this probe did not verify game-session survival, launch-success detection or automatic restore after Minecraft exits. Preserve the independent session model and test those separately when integrating the agreed gameplay behavior.

## Cleanup and status

The temporary Windows scheduled task and Linux virtual input device were removed. Test probe processes were stopped; the pre-existing Block/player sessions were not targeted. Temporary source files on test hosts are removed only after verifying their content hash; the reproducible source remains in this workspace. Native binaries and diagnostic artifacts remain available in ignored test-output locations.

This assessment supplies capability evidence for M04 in [next-tasks.md](next-tasks.md). The subsequent authorized production increment, macOS traffic-light removal, native checks and remaining boundaries are tracked in [window-chrome-implementation.md](window-chrome-implementation.md). This assessment does not mark M04 or Accepted complete.
