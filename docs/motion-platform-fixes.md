# Cross-platform motion fixes

Date: 2026-10-24. Follow-up to [the native motion verification report](motion-platform-verification.md). The user accepted this motion increment on Windows ARM64 on 2026-10-24. Other platforms and broader Milestone III acceptance remain Pending.

## Changes

### Linux system preference queries

The old render path synchronously executed `gsettings` whenever it asked for the system motion policy. The Linux path now returns a cached value and permits one background refresh at a time. Successful queries are refreshed on demand after one second; failed queries back off for five seconds and preserve the last successful value. Each command has a 750 ms timeout and its child is reaped. macOS and Windows keep their native API queries.

The query prefers the [standard XDG reduced-motion setting](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Settings.html). On KDE, an older portal can fall back to `kreadconfig6`/`kreadconfig5`, using the same `KDE/AnimationDurationFactor == 0` rule as [KDE's portal implementation](https://github.com/KDE/xdg-desktop-portal-kde/blob/master/src/settings.cpp). GNOME settings remain a fallback. Unknown or unavailable settings are not treated as a successful reduced-motion value. A cold cache starts with no system reduction until the first background query finishes; subsequent decisions use the refreshed value. The application preference still independently requests reduced motion.

An ARM64 Linux probe using disposable command substitutes made 161 render-style reads over about 4.2 seconds and invoked the successful portal command five times. Reads averaged 7.7 microseconds with a 148 microsecond maximum in that run. A command deliberately sleeping for six seconds was terminated and reaped by the timeout, without blocking the reads or accumulating workers. A temporary KDE configuration exercised factors zero and one without changing the desktop configuration. These are query-path measurements, not compositor frame rates.

### Toast exit lifecycle

New native Toast replays exposed the same product defect on macOS and Linux ARM64: the published manager's default 200 ms removal deadline cut off the product's 400 ms ordinary exit. The Linux trace ended with exit progress still at approximately 0.1987. This was a lifecycle defect, separate from the original report's unverified Toast timing.

The final implementation records dismiss intent without starting the manager's deletion timer. The component library's Presence completion determines when the card and its interaction state disappear: ordinary exit is 400 ms, swipe exit is 200 ms, and a collapsed back card uses 500 ms. Only after Presence reports Absent does the adapter dismiss the manager entry, which is reclaimed on the next lifecycle tick with a zero exit duration. Reduced motion settles immediately. Expiry and overflow use the same dismiss intent, and duplicate notifications do not revive an ending card. Both initial construction and runtime motion-policy changes use the same manager policy.

An intermediate 500 ms manager retention fixed the ordinary 400 ms cutoff at higher paint cadences, but did not guarantee completion: its timer started at dismiss, while Presence started at the next render. For a collapsed 500 ms exit, the manager could delete the entry before Presence completed. A delayed-render regression and the collapsed-card test now require Presence completion before reclamation. No additional animation clock or duration buffer is used.

Debug-only `toast-motion` and `toast-motion-reduced` scenarios provide deterministic enter, ordinary dismiss and programmatic swipe-release actions. They use disposable scenario data, record real card bounds during paint, and distinguish requested application policy from effective system policy. After a nominal 700 ms hold, a slow host gets a bounded extra wait for the real enter endpoint before the next action. This changes the replay driver, not the product transition duration. The programmatic release exercises the product's existing gesture-release decision; it is not a physical pointer-gesture acceptance check.

The trace distinguishes card paints, Presence Absent observed during render, and manager removal observed during render. The latter two have no paint geometry. A reduced-motion card may disappear before an ending card paint, so its last same-ID paint and the subsequent absent/removal observation establish the state change. Manager removal alone is not accepted as a completed ordinary animation.

### Portable trace analysis

The frame and pane analyzers now validate action-to-completion geometry, excluding startup window-manager configuration paints. Pane coordinates use the recorded body inset rather than assuming a macOS content origin. Opacity permits only a small f32 rounding tolerance. Missing actions, lost activation, geometry changes, non-finite values, excessive travel and reversed sampled stages remain failures.

Frame detail visibility requires its painted bounds to intersect the observed content viewport. Windows can acknowledge a native resize after an animation target changes: opacity alone does not establish that the still-clipped detail is visible. The analyzer retains both opacity-start and visible-start timestamps. A Windows ARM64 repeat includes an independent Win32 outer-rectangle sidecar; its approximate clock correlation is kept separate from GPUI client-viewport paint timestamps.

Same-paint stage starts are explicitly `sampling-limited`, with the observation interval recorded. They do not establish the precise stagger. All ten original platform F/P captures and the original Windows AMD64 first pane attempt can be analyzed without editing or normalizing their raw JSONL. Linux AMD64 retains one unresolved F sequence and two unresolved P sequences in those original captures. Twenty-one deliberately corrupted copies were rejected. Positive regressions cover an opaque but clipped detail and f32 opacity rounding. Receipts and original trace hashes are under `target/motion-fixes/analyzer-replay/`.

## Native verification

The user reviewed the running Windows ARM64 debug build (`4105f6df…`, opened as a normal application) and accepted its motion behavior on 2026-10-24. This user acceptance does not change the automated frame replay's missing-paint result below, or establish acceptance for the other platforms.

The final Rust sources are recorded in `target/motion-fixes/final-sources.json`. macOS passed the native verification script: formatting, checks, strict Clippy, 577 tests and the desktop build. All four Windows/Linux hosts passed native debug builds, strict Clippy and all 24 feedback tests; both Linux hosts also passed the three preference cache/parser tests. Native architecture and binary/source hashes are recorded with the platform receipts. These are debug checks, not release-profile performance results.

| Platform | Frame / pane / drawer | Ordinary Toast | Application-reduced Toast |
| --- | --- | --- | --- |
| macOS ARM64 | Pass; 350 frame and 246 pane paints, no stage-order ties | Pass; 170 paints | Pass; 6 paints |
| Windows AMD64 | Pass; 179 frame and 111 pane paints, no stage-order ties | Pass; 142 paints | Pass; 6 paints |
| Linux ARM64 (Xwayland) | Pass; 233 frame and 154 pane paints, no stage-order ties | Pass; 88 paints | Pass; 6 paints |
| Linux AMD64 (X11) | Pass; 81 frame and 59 pane paints; one frame and two pane stage orders sampling-limited | Pass; 68 paints | Pass; 6 paints |
| Windows ARM64 | Frame incomplete on two attempts; pane passes with 25 paints and three sampling-limited orders; drawer passes | Pass | Pass |

Successful replay paint observations above were taken with the disposable window active. Frame origin and height remain fixed during the observed replays. Windows ARM64 still fits management width to 1114 logical pixels. Its two final-source frame captures contain no paint between the scheduled rapid Accounts entry and Home reversal about 103 ms later. The strict frame analyzer rejects both captures. Other sampled movement is retained in the raw traces, but the complete fast entry/reversal trajectory is not established. End-of-driver foreground snapshots on this host are separately false; they do not establish continuous OS-level focus beyond the app's active paint records.

Before the final Presence-completion fix, two Windows ARM64 ordinary Toast captures painted about once every 256 ms: enter reached approximately 0.92 before the fixed replay dismiss, and normal exit reached approximately 0.09 before the next render observed manager removal. The intermediate 500 ms retention had only about 42–58 ms of nominal slack after the delayed first ending render. Those captures remain invalid and are retained, including `target/motion-fixes/windows-arm64/evidence/captures/toast/analysis/presence-manager-timing.json`. The final source both waits for the real replay enter endpoint and retains exit data until Presence Absent; ordinary and reduced Toast now pass the same unchanged analyzer on this host.

Final replay receipts are under `target/motion-fixes/macos-arm64/presence-completion/`, `windows-amd64/presence-completion/`, `linux-amd64/presence-completion/`, `linux-arm64/presence-completion/evidence-presence-completion/` and `windows-arm64/evidence/presence-completion/`. Linux ARM64 raw captures were analyzed locally with the current analyzer hashes; its older remote analyzer copies were not used. Earlier intermediate builds, focus/driver failures and failing Toast traces remain separate evidence; they do not establish a final-source pass. The application-reduced runs do not establish a live system-preference toggle on every desktop.

The Windows AMD64 and Linux AMD64 hosts were built and tested serially because the Linux VM shares the Windows machine. Only task-owned apps/helpers were stopped. The Linux AMD64 VM started for this task was soft-stopped afterward. Mac display/system idle sleep was temporarily inhibited for the interactive checks and released after verification.

## Reproduction

Build a debug launcher, create an output directory and keep each disposable window active:

```sh
cargo build --locked -p block-app --bin block
BLOCK_TOAST_MOTION_TRACE=/absolute/output/toast.jsonl \
  target/debug/block --scenario toast-motion --theme light
python3 scripts/analyze-toast-motion.py /absolute/output/toast.jsonl \
  --output-dir /absolute/output/analysis
```

Use `toast-motion-reduced` for the application-reduced variant. The system policy is still honored. F/P replay commands remain in the original report; both analyzers accept `--output-dir` so results can be kept separately from the immutable source traces.

Paint traces describe submission and render-state observations. They do not establish display-presentation timestamps, multi-monitor/scale coverage, compositor smoothness or user visual acceptance. Windows ARM64's fit-to-display width and Linux VM graphics limits retain their original boundaries.
