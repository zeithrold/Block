# Custom window chrome: first production increment

Date: 2026-10-23. User-authorized Milestone III increment, implemented by GPT-6 Luna at high reasoning effort and reviewed by the parent agent. Production native verification is partial with the explicit boundaries below; Accepted remains Pending.

## Implemented behavior

- Product windows request client decorations with `titlebar: None` and `app_owns_titlebar_drag: true`. macOS has no native traffic lights or reserved 80 px left inset.
- The existing restrained header contains a dedicated drag span, an icon-only Minimize button and an icon-only Close button. Interactive controls occupy separate hit regions. The Home body retains its composition.
- Drag starts from a left press in that span followed by movement while Left remains pressed. Release clears the transient state; transition snapshots never retain an armed drag.
- Onboarding and management pages share these controls. Standalone and inline crash help also have their own drag/minimize/close header and retain existing report actions.
- Window titles remain explicitly available to the operating system. Closing the last product window exits the app on Linux as well as Windows/macOS. Returning from crash help opens the requested launcher window before removing the help window.
- `is_resizable: false` is retained and the product header exposes no maximize or double-click zoom action. Strict Linux window-manager size enforcement remains a separate boundary.

The change uses the published GPUI 0.3.9 and component 0.8.0 dependencies. Source fingerprints for this reviewed increment: [reviewed-sources.json](../target/chrome-probe/luna-production/reviewed-sources.json). The implementation was validated over baseline `140b490`; the fingerprints identify the six tested Rust source files. Validation and source publication do not set user acceptance.

The remaining production-platform, content-fit, fixed-size, modal, game-lifecycle, provenance and visual-capture tasks are listed in the M04 follow-up checklist in [next-tasks.md](next-tasks.md). Fresh macOS Home captures use disposable fixture data: [light](../target/chrome-probe/luna-production/macos-home-light.png), [dark](../target/chrome-probe/luna-production/macos-home-dark.png), [capture provenance](../target/chrome-probe/luna-production/macos-home-capture.json). These captures do not close the earlier same-run interaction-provenance gap.

## Automated verification

- `cargo fmt --all -- --check`: PASS.
- `cargo check --locked --workspace --all-targets --all-features`: PASS.
- `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`: PASS.
- `cargo test --locked --workspace --all-features`: PASS, 39 suites, 517 tests, 0 failed, 0 ignored. [Full output](../target/chrome-probe/luna-production/workspace-tests.log).

The first sandboxed test run stopped at seven detached-supervisor tests because local IPC returned `Operation not permitted`. The permitted run outside the sandbox passed all eight supervisor tests and the rest of the suite. This is an execution-environment distinction, not a hidden failed product gate.

The geometry regressions verify that the menu, positive-width drag span, Minimize and Close remain within 960×600 and 360×540 viewports. The crash-help regression verifies the new header and existing report actions remain visible. Existing overlay, onboarding, queued launch, detached-session and recovery tests also passed. Automated geometry assertions do not prove native window movement.

## Native verification

The current production checks are listed below. After the user explicitly authorized the six-file source transfer, Windows AMD64 production build and core window interactions passed. Modal-specific native coverage and broader M04 boundaries remain as described below.

| Environment | Production result | Evidence |
| --- | --- | --- |
| macOS Apple Silicon | No OS titlebar/traffic lights; native CGEvent drag moved Home +80/+40 at unchanged 432×768; custom Minimize/restore and Close observed through AX/app state. Earlier interactions are attributed to PID 43236; its exact argv/binary hash were not captured. A separate fresh launch proves current binary/argv/window provenance without repeating those actions. | [Measurements and input-path limits](../target/chrome-probe/luna-production/macos-native-results.json), [management presentation](../target/chrome-probe/luna-production/macos-after-drag.png), [native input helper](../target/chrome-probe/luna-production/mac_native_evidence.swift), [fresh launch provenance](../target/chrome-probe/luna-production/macos-direct-launch.json) |
| Linux AMD64, XFCE X11 | No system decorations; measured drag +54/+36; custom Minimize set `_NET_WM_STATE_HIDDEN`, restore retained geometry, and custom Close removed the task window and exited its PID. | [Initial properties](../target/chrome-probe/luna-production/linux-amd64-native-preflight.json), [drag](../target/chrome-probe/luna-production/linux-amd64-overlay-drag.json), [minimize/restore](../target/chrome-probe/luna-production/linux-amd64-minimize-restore.json), [close](../target/chrome-probe/luna-production/linux-amd64-close.json) |
| Windows AMD64 | Native build PASS with the six reviewed files. Exact PID/argv and executable hash recorded. Client size 432×768; native insets [8, 0, 8, 8] confirm zero OS titlebar height. Header drag moved +80/+40 with stable size; custom Minimize, native restore and custom Close/process exit passed. Two consecutive header clicks preserved geometry; native double-click recognition was not proven. | [Build](../target/chrome-probe/luna-production/windows-amd64/build-confirmation.json), [parent source verification](../target/chrome-probe/luna-production/windows-amd64/parent-source-verification.json), [native actions](../target/chrome-probe/luna-production/windows-amd64/actions.json), [reviewed screenshot](../target/chrome-probe/luna-production/windows-amd64/after-drag.png) |

Linux's component border reserves 20 px on each side of the measured drawable. The measured 432×704 X11 drawable therefore has a smaller product surface. This increment does not claim a cross-platform content-size match. An anchored menu remained open during one header move; that popover check does not prove modal backdrop blocking. Failed direct-window GPU captures and initial synthetic-input misses are retained alongside the corrected measurements.

The Windows screenshots retain an NVIDIA popup over the lower-right body. The unobscured custom top header and Win32 measurements establish titlebar removal; these are not clean full-Home visual-acceptance captures. No production modal test was run on Windows.

A separate Linux `games-help` Dialog launch had directly observed argv, task PID and window geometry. The drag/Escape attempt did not establish event consumption or dismissal, and its screenshot was all black. That modal result remains unverified: [receipt and cleanup](../target/chrome-probe/luna-production/linux-amd64-modal.json).

Task-owned macOS/Linux app and stuck capture processes, temporary source worktree, bundle and scratch paths were removed. The original Linux checkout is clean and its build cache is preserved. Windows task cleanup is also complete: no task app process or scheduled task remains, the task worktree is removed and the original checkout is clean. [Parent cleanup verification](../target/chrome-probe/luna-production/windows-amd64/parent-cleanup-verification.json), [all task-name prefixes](../target/chrome-probe/luna-production/windows-amd64/parent-task-cleanup-verification.json). [Evidence index](../target/chrome-probe/luna-production/README.md).

Linux ARM64 Wayland/Xwayland and Windows ARM64 were not re-tested with this production increment. The earlier Linux ARM64 capability probes remain separate evidence: [Windows/Linux feasibility](window-chrome-feasibility.md) and [KDE Xwayland investigation](xwayland-compatibility-investigation.md).

Evidence under `target/` is local and ignored by Git; a source push does not upload these images or logs.
