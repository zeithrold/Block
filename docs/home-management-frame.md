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
| Home | 0–85 ms: fade out and move up 12 px | 250–380 ms: fade in and return from 12 px above |
| Sidebar | 30–130 ms: fade in and rise 14 px | 225–330 ms: fade out and move down 14 px |
| Native width | 130–320 ms: expand to the right | 95–290 ms: retract |
| Detail | 200–380 ms: fade in and move left 24 px | 0–110 ms: fade out and move right 24 px |

The user requested a faster overall sequence after trying the initial 620 ms version. Both directions now take 380 ms (39% less time), retaining the curve and stage order. Sidebar and detail starts differ by 170 ms. The sidebar finishes its entrance before expansion starts; content begins 70 ms into expansion. The management page is laid out at its final content width throughout, clipped by the real window, so the narrow-layout breakpoint and text wrapping do not change during expansion.

Reversal samples the current opacity, position and actual native width. A return before any expansion uses a shorter 140 ms sequence. Application or system reduced motion immediately settles width and panes, releases the outgoing visual data, and does not replay the sequence when motion is enabled again.

Only the outgoing page body is retained. Its snapshot owns no engine or credential URL. A shared cell contains only copyable presentation values; it does not retain the live view. Body input and tab traversal are suspended during the bounded morph, while the live title area/menu can reverse or retarget navigation. Frozen layers are discarded on completion or reduced motion.

## Management navigation principles (user decision)

Management-to-management navigation now crossfades the outgoing and incoming panes with **12 px maximum sidebar travel and 20 px maximum right-content travel**. The sidebar starts first and takes 200 ms; the detail pane starts 40 ms later and also takes 200 ms, for a 240 ms timeline. The title area and native window stay fixed. This replaces the former whole-page, full-width translation that the user found dizzying.

A sidebar selection within the current management page animates only the right content: a 180 ms horizontal crossfade with at most 20 px travel. The sidebar's position and opacity stay fixed; its selection feedback updates immediately. The same bounded content motion applies to wizard steps. These scopes do not combine with the Home/window morph.

The timeline begins on the first incoming paint. A heavier initial layout therefore cannot consume the 40 ms delay before either pane has appeared. Paint acknowledgements belong to individual runs, so an interrupted run's callback cannot start its replacement. Rapid retargeting retains only one outgoing snapshot, preferring the currently more visible layer when the incoming page has barely appeared.

Outgoing controls are inert. Live editors continue accepting text during these content transitions. Tab/Shift-Tab first settle the transition, repaint the live focus tree, then forward each queued traversal once to Base Root. Application/system reduced motion settles immediately and drops outgoing snapshots.

The earlier 12 px version's foreground macOS pane replay recorded **228 active-window paint samples**. Completed management switches showed a 34–40 ms observed start gap; native bounds stayed fixed, horizontal poses stayed within 12 px, and the actual content-fragment bounds remained horizontal. Both Settings child selections kept the sidebar stationary. Action-to-settled times were 264–298 ms for management pages and 199–204 ms for child content, including initial layout/paint latency. A prior trace where the Instances first layout consumed the gap is retained as `target/pane-motion/before-paint-anchor.jsonl`.

Replay with `BLOCK_FRAME_MOTION_TRACE=<file> BLOCK_PANE_MOTION_REPLAY=1` and `--scenario home-ready`, activate the window, then run `python3 scripts/analyze-pane-motion.py <file>`. Evidence is in `target/pane-motion/native.jsonl`, `measurement.json` and `macos-verify.log`; it remains local and does not set Accepted.

## Shell alignment follow-up (2026-10-24)

The app menu begins with the Brand display; its vertical padding is 4 px per side rather than 12 px. Navigation rows place the icon before the label and align both to the left, retaining menu roles, accessible labels and keyboard inspection. The shared title divider is inset 16 px from each window edge. Home's primary icon and two-line label align left with 20 px padding, reserving space for the 56 px profile selector. Home's composition and both window sizes are unchanged. At the user's request, right-content travel increased from 12 to 20 px; sidebar travel, easing, durations and the start gap are unchanged.

The local native gate passed 573 tests across 39 suites. Actual macOS Home, menu and Accounts views were inspected and captured. A fresh replay recorded 246 foreground paints, kept the native origin/720 px height fixed, bounded sidebar/content travel at 12/20 px and kept the sidebar stationary during child navigation. The observed page start gap was 37–43 ms. Evidence: `target/ui-alignment/{home-light.jpg,menu-light.jpg,accounts-light.jpg,native.jsonl,measurement.json,macos-verify.log}`. These are native paint submissions rather than compositor timestamps; Windows/Linux visual review remains pending.

## Verification

- The final local macOS verification entry passed formatting, workspace all-target/all-feature checks, strict Clippy, **573 tests across 39 suites**, the native build and build-identity check. This is an automated result, not visual acceptance.
- Regressions cover phase ordering, reversal continuity, actual pane outer width, unchanged height/origin, runtime reduced motion, outgoing-layer release, hidden incoming controls, compact Home actions, CSD insets, negative display origins and small-display startup bounds. TestWindow receives simulated native resize callbacks because its resize implementation only records OS bounds.
- A foreground macOS disposable scenario replay submitted **371 continuously foreground replay paints** over ten navigation actions (423 total paints including startup), including two interrupted returns. Every recorded native top-left was `(704, 344)` and every height was `720`; width ranged from `360` to `1152`. No production account or game operation was performed.
- In the three completed forward runs (Accounts, Settings, About), the first visible sidebar paint occurred 41–44 ms after the action; the first native width change occurred 140–147 ms after it; the first visible detail paint occurred 205–213 ms after it. Complete sequences settled in 385–392 ms. These include event/render latency and differ from the timeline's logical start times.
- Paint probes also record actual pane bounds, distinguishing the 360 px outer sidebar from its 359 px inside-border area. These are paint-submission measurements, not a video/compositor presentation timestamp.
- The initial 620 ms trace is retained separately as `620ms-native.jsonl`. A faster replay that lost window activation is retained as `380ms-background-native.jsonl`; it is excluded from foreground phase conclusions. The analyzer rejects a replay that loses activation during its action sequence. CI also exposed that the original hidden-input test inherited the runner's system motion policy; animated tests now scope that input explicitly, and a separate regression verifies that system reduced motion settles immediately with the application preference off.
- Local evidence: `target/frame-motion/native.jsonl`, `measurement.json`, `report.html`, `home-light.jpg`, `accounts-light.jpg`, `settings-light.jpg`, and `macos-verify.log`. Ignored artifacts are not uploaded by a source push.

## Remaining To-Do

- [ ] Review the preferred proportions and the staggered sequence with the user before setting any Accepted marker.
- [ ] Repeat physical window-origin/height measurements and rapid reversal on Windows and Linux supported compositor paths; CI compilation/tests do not replace these observations.
- [ ] Review actual Accounts/Settings and long/localized content at 1280 × 720 and multiple scale factors, including Linux CSD insets and near-right-edge narrow fallback.
- [ ] Complete the existing macOS system Reduce Motion review and keyboard/IME interaction acceptance across navigation/overlays.
- [ ] Review the bounded horizontal management/child transitions with the user and repeat scope, focus/IME and first-paint timing checks on Windows/Linux physical desktops.
