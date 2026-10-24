# Drawer entrance frame comparison

Date: 2026-10-23. Milestone III evidence; Accepted remains Pending.

The user identified Drawer entrance as the remaining motion mismatch. This
follow-up changes the shared Drawer presentation and leaves Home's composition
and Toast behavior intact.

## Cause and correction

The installed prototype's `components/ui/drawer.tsx` uses two separate CSS
transitions. Its panel moves for 450 ms with `cubic-bezier(.22, 1, .36, 1)`;
its backdrop progresses for 450 ms with `cubic-bezier(.32, .72, 0, 1)`.
The closed panel transform includes its height plus a 2 px border bleed.

Native already used 450 ms, but `Transition::new` selected the default
`ease_out_cubic` curve. Its backdrop also reused panel progress, and its closed
translation omitted the extra 2 px. Matching duration alone therefore left a
substantially slower initial movement and the wrong backdrop trajectory.

The shared layer now gives panel and backdrop separate Base Presence channels
with the prototype's curves, and includes the 2 px closed offset. Base Sheet
continues to own modal/focus semantics. Drag rebound and dismissal policy are
unchanged; no dependency fork or prototype repository change is needed.

## Measurement and findings

All three controlled captures used a 432 × 768 client viewport, DPR 1 and a
540 px account drawer, with the window active and reduced motion disabled.
The reference harness imports the actual prototype wrapper and application CSS,
including Base UI Drawer 1.9.0. It supplies synthetic account rows and sets the
public popup height to match Native; it does not replace transition rules.

Native records actual panel bounds during paint, together with its render
clock and backdrop progress. The browser records RAF timestamps, computed
bounds/opacity and the CSS transition's reported `startTime`/`currentTime`.
The analysis uses the first entrance: 56 Native paint samples before, 56 after,
and 97 browser samples. Median sample intervals were approximately 8.3 ms.
The Native logs also retain a second entrance and both exits for inspection.

Two clocks are retained. Trigger-relative time includes application dispatch,
browser mounting and paint-submission delay. Animation phase starts at Native's
first entrance render and uses the browser's observed CSS timeline. No offset
is fitted to make the trajectories agree. Before/after an active CSS transition,
RAF minus its reported start time extends that same timeline.

Panel progress comes from measured top coordinates, normalized by each source's
closed-to-rest travel: 540 px before and 542 px after/in the prototype. The raw
coordinates retain that geometry difference. Linear interpolation uses an 8 ms
common grid over the shared entrance interval, capped at 450 ms of phase time
or 500 ms of trigger time; settled frames do not dilute the mean.

| Animation-phase metric | Native before vs prototype | Native after vs prototype |
| --- | ---: | ---: |
| Mean absolute panel travel error | 7.711% | 0.052% |
| Maximum absolute panel travel error | 18.674% | 0.247% |
| Mean absolute backdrop progress error | 6.260% | 0.035% |
| Maximum absolute backdrop progress error | 19.951% | 0.205% |
| Time to 99% measured panel travel | 353.9 ms | 295.9 ms |

The prototype reached 99% at 292.6 ms of its observed animation timeline. The
remaining maximum phase error corresponds to roughly 1.34 px over the corrected
542 px travel. Native rounding, interpolation and recording the render clock
after layer construction limit the precision of this comparison.

Trigger-relative mean panel error decreased from 5.916% to 2.419%; its maximum
after correction was 10.576%. The browser's initial mount/starting-style frames
and the Native dispatch path still have different startup timing. This evidence
supports matching the motion curves; it does not establish identical click-to-
display timing or cross-platform frame pacing. Native timestamps describe paint
submission and browser timestamps describe RAF observations, neither GPU display
presentation. The report's geometry reconstructions are labeled as such.

## Reproduction and artifacts

The debug-only Native probe is off unless `BLOCK_DRAWER_MOTION_TRACE` is set.
With `--scenario home-ready --theme light`, it uses disposable scenario data,
waits for the window to become active, warms up, then opens/closes the account
drawer twice. It records geometry and timing, not account credentials or logs.
The probe and replay are excluded from release builds.

Example on macOS ARM64, after building the current executable:

```sh
BLOCK_DRAWER_MOTION_TRACE="$PWD/target/drawer-motion/native-after.jsonl" \
  target/aarch64-apple-darwin/debug/block --scenario home-ready --theme light
```

The browser capture needs the existing local prototype and its installed
dependencies. Start the reference harness, open its displayed localhost URL
with `?height=540` in a 432 × 768/DPR 1 viewport, choose Account content, and open
the drawer while the tab is focused:

```sh
node scripts/trace-drawer-reference.mjs
python3 scripts/compare-drawer-motion.py --input-dir target/drawer-motion
```

The analyzer expects `native-before.jsonl`, `native-after.jsonl` and
`drawer-prototype.json`; it writes `report.html`, `summary.json`,
`raw-frames.csv` and `common-time.csv`. The before capture must come from the
pre-correction presentation with the same opt-in probes. Capture files, app
bundles, the browser harness and the screenshot `overview.png` remain local
under ignored `target/drawer-motion/`.

The new regressions compare actual Native panel bounds and painted backdrop
alpha at 45/90/135/225/315/450 ms against independent browser reference values.
They advance animation callbacks rather than forcing a window refresh, so a
missing frame request cannot silently pass. A separate reduced-motion case
checks that both tracks enter settled. The full macOS gate passed formatting,
all-target/all-feature checking, strict Clippy, 556 tests across 39 suites with
zero failures/ignored tests, and the desktop executable build.

## Cross-platform native check (2026-10-24)

The same drawer entrance/exit replay was measured on the four non-macOS native
platforms (paint clock, foreground, animated path): Windows AMD64 first open
464.6 ms panel / 463.7 ms backdrop with 23 intermediate panel samples; Windows
ARM64 462.6 ms / 27 samples; Linux ARM64 479.0 ms / 11–24 samples; Linux AMD64
459.4 ms / 458.3 ms with 10 samples. macOS reference: 446.6 ms / 53 samples.
Every run kept the 540 px panel and translation 0↔542 px (540 px + the 2 px
closed offset), a separate backdrop curve and `reduced:false`. Physical video on
Windows and Linux shows intermediate frames rather than hard cuts. Details and
platform caveats: [motion-platform-verification.md](motion-platform-verification.md).
Interrupted drag, focus return, short-window content and system Reduce Motion
remain open; this does not set Accepted.

User review follow-up (2026-10-24): the user manually looked at the Windows ARM64 motion behavior because the automated replay could not detect it; this is a scoped manual observation, not product acceptance. **No platform has passed acceptance**, and other platform/scenario reviews remain open. The later Toast lifecycle fixes and scoped manual observation are recorded in [motion-platform-fixes.md](motion-platform-fixes.md).

## Remaining To-Do

- [ ] Review the corrected entrance and interrupted drag on native Windows,
  macOS, Linux X11 and Wayland/Xwayland desktops. Entrance is now physically
  measured on Windows AMD64/ARM64 and Linux AMD64/ARM64 (see the section above);
  interrupted drag and user visual review stay open.
- [ ] Inspect OS Reduce Motion and actual displayed transition frames during
  the existing Milestone III review.
- [ ] Evaluate native backdrop blur only if the product flow requires it;
  the current tint and curve comparison does not implement browser blur.
