# Cross-platform motion verification (Windows/Linux)

Date: 2026-10-24. Observation-only verification of the recent motion increments
on the four non-macOS native platforms, requested by the user, with macOS ARM64
as the reference. **Nothing here is user/visual acceptance; Accepted remains
Pending.** Paint traces and physical compositor recordings are separate kinds of
evidence: the trace measures paint submissions, the recordings show the frames
that actually reached the screen.

Tested source: `988c533` (`Tighten brand version spacing and label Pilot M2`)
plus the working-tree `crates/block-ui/src/select.rs` label tweak. Debug build
for every platform (the scenario CLI and motion traces are
`debug_assertions`-gated). Hosts: macOS ARM64 local, Windows AMD64 (1920×1080,
100 % DPI, DWM), Windows ARM64 VM (200 % scale, 1146×852 logical), Linux AMD64
VM (1920×1080, X11/Xfwm4, software GL), Linux ARM64 VM (1280×800, KDE Wayland
with Xwayland `:1`). The Windows AMD64 and Linux AMD64 phases ran strictly one
at a time because they share one physical machine (decision 24); the ARM64 VMs
ran in parallel.

Scope: the Home ↔ management coordinated frame (380 ms, width 360 ↔ 1152,
staggered sidebar/width/detail, interrupt and reverse paths), bounded
management/content transitions (sidebar 200 ms/12 px, detail 200 ms + 40 ms
start/20 px, child content horizontal-only), drawer entrance/exit (450 ms panel
curve, separate backdrop curve, 2 px closed offset), reduced motion, and Toast
timing where physically reachable.

## Verdict summary

| Platform | F frame | P pane | D drawer | Reduced motion | Toast | Physical frame evidence |
| --- | --- | --- | --- | --- | --- | --- |
| macOS ARM64 (reference) | PASS | PASS | PASS | not physically checked | not physically covered | none (paint trace only) |
| Windows AMD64 | PASS | PASS (2nd run) | PASS | PASS | not covered | 36.5–38.8 fps gdigrab; 6 distinct widths per transition |
| Windows ARM64 | PASS | PASS | PASS | PASS | not covered | 27–30 fps DXGI ddagrab; 4–6 intermediate frames per transition |
| Linux ARM64 | PASS (replay slice) | PASS (inset-normalized) | PASS | PASS | presence only | 30 fps X11/Xwayland window raster; 3–4 intermediate frames per transition |
| Linux AMD64 | PARTIAL (raw analyzer) | PARTIAL (raw analyzer) | PASS | PASS | not covered | 30 fps x11grab in every replay window; 4–6 changed frames per transition |

"PASS" means the recorded evidence satisfies the invariants of the committed
motion analyzers on that platform or, where marked, an equivalent documented
platform-shaped analysis (raw traces kept unchanged). "PARTIAL" means the app
motion is visible and numerically consistent, but the committed analyzer rejects
the raw trace for environment-shaped reasons listed below — these are analyzer
portability findings, not product motion defects. No platform crashed, froze or
showed an unexpected hard cut during an animated run.

## Cross-platform results

Forward settle times include event/render/paint cadence and are not directly
comparable across hosts; the staggered stage order and final geometry are.

| Platform | F foreground replay paints | Native origin / height | Width (logical) | Forward settle | Drawer first open (paint clock) |
| --- | ---: | --- | --- | --- | ---: |
| macOS ARM64 | 351 | (704, 344) / 720 | 360 ↔ 1152 | 383.6–394.0 ms | 446.6 ms (53 intermediate samples) |
| Windows AMD64 | 146 | (384, 160) / 720 | 360 ↔ 1152 | 394.6–421.5 ms | 464.6 ms (23 samples) |
| Windows ARM64 | 154 | (16, 47) / 720 | 360 ↔ 1114 (display-clamped) | 389.4–455.6 ms | 462.6 ms (27 samples) |
| Linux ARM64 | 164 | (44, 6) / 704 outer | 402 ↔ 1194 outer = 360 ↔ 1152 content | 396.6–405.1 ms | 479.0 ms (11–24 samples) |
| Linux AMD64 | 82 | (0, 27) / 762 outer | 402 ↔ 1194 outer = 360 ↔ 1152 content | 396–433 ms | 459.4 ms (10 samples) |

All platforms kept a fixed native origin and height throughout every replay and
animated the window width in steps rather than one jump; independent
window-geometry logs (20–50 Hz outside the app) agree with the trace. Paint
cadence differs widely: macOS submits every ~8 ms, Windows AMD64 every
~25–30 ms, Windows ARM64 similar, Linux ARM64 roughly half the macOS rate, and
the Linux AMD64 software-GL guest about 11.7 paints/s (≈85 ms). That cadence,
not a different motion curve, explains the larger settle-time spread and the
low-pass-filtered stage starts.

### Windows AMD64

- F: committed frame analyzer exit 0. Origin (384,160), height 720 fixed,
  width 360–1152; forward stages sidebar 51.6–66.9 ms → width 127.9–192.4 ms →
  detail 225.1–252.6 ms; settle 394.6–421.5 ms. Both rapid reversal pairs
  painted; the interrupted 4600 ms expansion was cut at 1141 px.
- P: first run rejected by a 1-ULP f32 epsilon on the wizard track
  (`opacity 1.0000001192`, 2 of 92 samples) — an analyzer tolerance issue; the
  unmodified retry passed (104 paints, sidebar ≤12 px, content ≤20 px, child
  pages horizontal-only with a stationary sidebar, settle 303.9–367.6 ms pages
  / 247.9–256.1 ms child).
- D: PASS. First open 464.6 ms panel / 463.7 ms backdrop, translation 0→542 px,
  540 px panel fixed, separate backdrop scrim, `reduced:false`.
- Physical: gdigrab 1920×1080 at 36.5–38.8 fps; forward expansion showed six
  distinct physical widths (942→1083→1144→1160→1165→1166 px) over 233 ms;
  independent geometry 584 samples at ~32 Hz. Reduced motion: one geometry
  sample and one video frame change (361→1165 px), no intermediate widths.
- Toast timing is not physically covered (no trace/scenario, no reliable
  trigger).

### Windows ARM64

- F/P/D all passed the committed analyzers on the first try, window foreground
  for the whole replay (0 inactive replay paints).
- The logical display is 1146×852 at 200 % scale, so the product's fit-to-display
  rule clamps management to 1114 logical px; the animation is real
  (360 ↔ 1114, 37 distinct viewport widths) but the literal 1152 geometry and
  the 21 px CSD inset could not be observed. VM display-mode changes were
  rejected by VMware (`VMwareResolutionSet` exit 1), so this is a VM/display
  limitation.
- Stages: sidebar 37–42 ms → width 156–183 ms → detail 217–288 ms; settle
  389–456 ms. Both interrupt paths exercised (interrupted Accounts after 6
  paints; the 4600 ms expansion reversed at 1068 px).
- Physical: DXGI desktop duplication (gdigrab only reached 16.8 fps) at
  27–30 fps; 4–6 compositor intermediate frames per expansion, 36 distinct
  GetWindowRect widths; geometry 271 samples at 29.6 Hz, origin/height fixed.
- Reduced motion: single-step geometry jump 746→2254 outer px and a hard cut
  722→2232 px between consecutive 33.3 ms frames. Toast not physically covered.

### Linux ARM64

- F: raw committed analyzer rejects the trace because the first ~1.5 s before
  the replay contains an inactive pre-configure paint at the requested client
  geometry (64,32 / 1152×704) and one transient outer size (1234×744); the
  replay slice from the first action to `complete` is byte-identical to the raw
  trace and passes unchanged (164 paints, all foreground; origin (44,6), height
  704 outer; X11 width 402–1194 = logical 360–1152; stages 49.8–65.4 →
  158.6–187.6 → 223.5–233.8 ms; settle 396.6–405.1 ms). The slice and the raw
  trace are both kept.
- P: raw analyzer fails only because it hard-codes content origin x = 360 while
  Linux X11 content origin is +21 px; a copy with exactly that one
  transformation (48 `fragment` records, `x -= 21`) passes (104 paints,
  settle 285.0–326.4 ms pages / 222.1–229.9 ms child, 12/20 px bounds held,
  child selections keep the sidebar stationary).
- D: PASS directly (open 479.0 ms, close 433.9/434.9 ms, second open 467.9 ms,
  11–24 intermediate samples per transition, panel 531.6→0 px, separate
  backdrop scrim).
- Physical: `ffmpeg -f x11grab :1` is black under KWin compositing;
  `org.kde.KWin.ScreenShot2` rejects unprivileged callers. Motion was therefore
  captured as the real X11/Xwayland window raster (XGetImage, the pixels KWin
  composites) at 30 fps, plus KWin-composited Spectacle stills at ~4.1 fps as
  a compositor-authorized cross-check. Both show intermediate widths (e.g.
  402→1191 px in 139.7 ms with 738/961/1132/1179 px frames; stills 436/397 px).
- Reduced motion: two window widths only (402/1194), hard cut between adjacent
  frames. Toast presence was visible but appeared and dismissed between 60 fps
  frames, so 400/200 ms timing is not physically verified.

### Linux AMD64

- F: raw committed analyzer rejects the single pre-replay X11 configure paint
  (requested client 1152×720 at (384,164) before the WM settled the outer window
  at (0,27) 1194×762). Every replay paint is stable and active. Supplemental
  analysis of the same raw trace (committed logic, replay-slice geometry and
  same-paint stage ties allowed): 82 replay paints at 11.7 paints/s, width
  402→1194 outer, strict stage order for Accounts/Settings/About, settle
  396–433 ms, short-return rapid reversal and the interrupted 4600 ms expansion
  as designed.
- P: raw analyzer fails on the 21 px X11 body offset (child fragments at
  x = 381) and on two stage-order ties at 14.4 paints/s. Supplemental analysis:
  59 replay paints, no distance/vertical/sidebar/fragment violations, settle
  262–419 ms.
- D: PASS directly (104 paints, all active; first open 459.4 ms panel /
  458.3 ms backdrop, 10 intermediate samples, translation 0→542 px, 540 px
  panel, scrim 0→1, `reduced:false`).
- Physical: x11grab at 30 fps in every replay window (the low overall average is
  software-GL startup), 4–6 changed frames per transition, content widths
  362/1154 px against 360/1152; geometry 300 samples at 30 Hz, outer origin
  (0,27), height 762, 26 distinct widths. Reduced motion: one geometry sample
  and one video frame. Toast not covered.
- The first 1280×800 attempt was discarded: Xfwm4 clamped the window height to
  704 content instead of the 720 design height, so the layout context was not
  comparable; both attempts are retained. The definitive run is the
  1920×1080 one.

## What the evidence establishes

- The recent motion work renders and animates on all four platforms: real
  intermediate compositor frames exist for the 380 ms window expansion and
  contraction, the drawer 450 ms entrance/exit, and pane transitions; there is
  no crash, no orphaned stale frame, no stuck width, and rapid reversal/
  interruption completes.
- The staggered frame order (sidebar, then native width, then detail) and the
  bounded pane travel are preserved on every platform, at each host's own paint
  resolution.
- Reduced motion settles immediately on all four platforms (single-sample
  geometry and single-frame video jumps) for the Home/frame path.
- Platform differences are latency/cadence and display/CSD geometry, not a
  different animation policy or a missing animation.

## Analyzer and measurement limits

- The committed `analyze-frame-motion.py` and `analyze-pane-motion.py` are
  shaped for macOS: they reject pre-replay startup paints with a different
  geometry, hard-code the macOS content origin (360) and cannot resolve stage
  order when a host paints below the 40 ms stage gap. Linux raw traces are
  therefore rejected even though the replay invariants hold; the normalized/
  supplemental analyses are documented in each platform evidence dir and the
  raw traces are preserved. A platform-aware analyzer (ignore pre-action paints,
  derive the content inset, allow same-paint stage starts at low cadence) is a
  follow-up tooling task, not a product change.
- Windows AMD64 P needed one retry for a 1-ULP opacity epsilon; this is an
  analyzer tolerance issue.
- Physical recordings measure the compositor path but not presentation
  timestamps; video fps and window-geometry rates vary by host. No user visual
  acceptance, multi-monitor/scale sweep, system-level Reduce Motion check or
  Toast-timing check was performed.
- Windows ARM64 could not observe the unclamped 1152 management width because
  its VM display is clamped to 1146 logical px; the animation there is real but
  the final geometry differs by design of the fit-to-display rule.
- Linux ARM64 motion video is the X11/Xwayland window raster because KWin
  blocks x11grab and ScreenShot2 for this caller; compositor stills are a
  separate authorized cross-check. Linux AMD64 has software GL and only ~11.7
  paints/s, which coarsens all timings.

## Reproduction

Per platform (debug build; the full protocol is `target/motion-verify/PROTOCOL.md`):

```
cargo build --locked -p block-app --bin block
# F: BLOCK_FRAME_MOTION_TRACE=<dir>/frame/frame-trace.jsonl
# P: F env + BLOCK_PANE_MOTION_REPLAY=1
# D: BLOCK_DRAWER_MOTION_TRACE=<dir>/drawer/drawer-trace.jsonl
target/debug/block --scenario home-ready --theme light
python3 scripts/analyze-frame-motion.py <frame-trace.jsonl>
python3 scripts/analyze-pane-motion.py <pane-trace.jsonl>
python3 target/motion-verify/analyze-drawer-trace.py <drawer-trace.jsonl> <out.json>
```

Keep the disposable window foreground for the whole replay. Record the screen
at ≥25 fps during the replay and log independent window geometry.

## Evidence

All raw evidence is local (git-ignored, not uploaded by a source push) under
`target/motion-verify/<platform>/`:

| Platform | Evidence |
| --- | --- |
| macOS ARM64 | `macos-arm64/summary.md`, `frame/`, `pane/`, `drawer/` |
| Windows AMD64 | `windows-amd64/summary.md`, `frame/`, `pane/`, `drawer/`, `reduced/`, `video/`, `geometry.log` |
| Windows ARM64 | `windows-arm64/summary.md`, `frame/`, `pane/`, `drawer/`, `reduced/`, `video/`, `geometry.log`, `screens/` |
| Linux ARM64 | `linux-arm64/summary.md`, `frame/`, `pane/`, `drawer/`, `compositor-frame/`, `video/`, `geometry` |
| Linux AMD64 | `linux-amd64/summary.md`, `attempt-1920/`, `analyzer/`, `video/`, `geometry.log` |

Nothing in this document or the evidence dirs changes a product `Accepted`
marker. macOS remains the only reference run; the four platforms are separate
physical evidence with the limits above.

The subsequent [motion fixes and native revalidation](motion-platform-fixes.md)
record production and analyzer changes separately from this observation-only
baseline, including the Toast lifecycle defect found by deterministic replay.
