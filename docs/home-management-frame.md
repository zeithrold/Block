# Home and management frame

Date: 2026-10-24. This is the user-authorized Milestone III geometry/navigation increment. Accepted remains Pending.

## Dimensions and rationale

All dimensions below are logical **visible content** pixels. Linux client-frame shadow/border insets are added to the outer window separately.

| Part | Preferred size |
| --- | --- |
| Home | 360 × 720 |
| Management | 1152 × 720 (16:10) |
| Management sidebar | 360 wide, fixed rather than a fraction of the growing window |
| Management content | 792 wide before its own padding/borders |
| Shared title area | 56 high |

The sidebar is 31.25% of the default management frame. The choice starts with Home's content budget: 24 px side padding leaves 312 px for the existing primary action and its profile selector. The management content budget accommodates the existing 608 px forms plus surrounding space. The 8 px grid and a common height/header keep both frames related without adding a Home dashboard or changing its composition. The preferred management ratio is a product choice, not a universal design rule. For comparison, Microsoft's [NavigationView default expanded pane](https://learn.microsoft.com/en-us/uwp/api/windows.ui.xaml.controls.navigationview.openpanelength?view=winrt-28000) is 320 DIP; Block uses 360 to preserve its Home action budget and the requested width equality.

Accounts/Games, Settings, providers, About, game setup, account setup and Welcome share the fixed desktop sidebar token. Existing narrow-screen alternatives remain available when the fitted management width cannot accommodate two panes. This fallback takes precedence over displaying an unusably narrow detail pane.

Startup reserves an on-screen expanded frame position before showing compact Home. Both initial sizes use the same available-height rule. Navigation changes width while retaining the current height and top-left position. The expanded target is bounded by the space to the current display's right edge. Display margins remain estimates (16 px horizontal, 32 px top, 64 px bottom); the 360 × 540 outer minimum and platform compositor/work-area behavior remain explicit limits. Physical screens smaller than that minimum cannot be made to fit by this rule.

## Coordinated transition

One monotonic clock drives four tracks with cubic Bézier `(0.22, 1, 0.36, 1)`:

| Track | Home → management | Management → Home |
| --- | --- | --- |
| Home | 0–140 ms: fade out and move up 12 px | 420–620 ms: fade in and return from 12 px above |
| Sidebar | 60–220 ms: fade in and rise 14 px | 380–540 ms: fade out and move down 14 px |
| Native width | 220–540 ms: expand to the right | 160–480 ms: retract |
| Detail | 340–620 ms: fade in and move left 24 px | 0–180 ms: fade out and move right 24 px |

Sidebar and detail starts differ by 280 ms. The sidebar finishes its entrance before expansion starts; content begins 120 ms into expansion. The management page is laid out at its final content width throughout, clipped by the real window, so the narrow-layout breakpoint and text wrapping do not change during expansion.

Reversal samples the current opacity, position and actual native width. A return before any expansion uses a shorter 220 ms sequence. Application or system reduced motion immediately settles width and panes, releases the outgoing visual data, and does not replay the sequence when motion is enabled again. Normal management-to-management and detail/wizard navigation retain their existing motion scope.

Only the outgoing page body is retained. Its snapshot owns no engine or credential URL. A shared cell contains only copyable presentation values; it does not retain the live view. Body input and tab traversal are suspended during the bounded morph, while the live title area/menu can reverse or retarget navigation. Frozen layers are discarded on completion or reduced motion.

## Verification

- The final local macOS verification entry passed formatting, workspace all-target/all-feature checks, strict Clippy, **565 tests across 39 suites**, the native build and build-identity check. This is an automated result, not visual acceptance.
- Regressions cover phase ordering, reversal continuity, actual pane outer width, unchanged height/origin, runtime reduced motion, outgoing-layer release, hidden incoming controls, compact Home actions, CSD insets, negative display origins and small-display startup bounds. TestWindow receives simulated native resize callbacks because its resize implementation only records OS bounds.
- A foreground macOS disposable scenario replay submitted **602 measured frame paints** over ten navigation actions, including two interrupted returns. Every recorded native top-left was `(704, 344)` and every height was `720`; width ranged from `360` to `1152`. No production account or game operation was performed.
- In the three completed forward runs (Accounts, Settings, About), the first visible sidebar paint occurred 67–74 ms after the action; the first native width change occurred 230–238 ms after it; the first visible detail paint occurred 349–353 ms after it. Complete sequences settled in 625–633 ms. These include event/render latency and differ from the timeline's logical start times.
- Paint probes also record actual pane bounds, distinguishing the 360 px outer sidebar from its 359 px inside-border area. These are paint-submission measurements, not a video/compositor presentation timestamp.
- Local evidence: `target/frame-motion/native.jsonl`, `measurement.json`, `report.html`, `home-light.jpg`, `accounts-light.jpg`, `settings-light.jpg`, and `macos-verify.log`. Ignored artifacts are not uploaded by a source push.

## Remaining To-Do

- [ ] Review the preferred proportions and the staggered sequence with the user before setting any Accepted marker.
- [ ] Repeat physical window-origin/height measurements and rapid reversal on Windows and Linux supported compositor paths; CI compilation/tests do not replace these observations.
- [ ] Review actual Accounts/Settings and long/localized content at 1280 × 720 and multiple scale factors, including Linux CSD insets and near-right-edge narrow fallback.
- [ ] Complete the existing macOS system Reduce Motion review and keyboard/IME interaction acceptance across navigation/overlays.
- [ ] Decide later whether management-to-management transitions need the same outgoing-content treatment; this increment only changes Home/frame navigation.
