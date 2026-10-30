# Whole-product GUI coverage audit

Date: 2026-10-30.

## Result and scope

The audit identified nine confirmed issue groups. The repair candidate below
implements all nine within their stated boundaries; full native accessibility
acceptance remains outstanding. The inventory also has material coverage gaps
and superseded evidence. This is not full-product visual acceptance or a claim
that no other defects exist.

The inspected candidate is Git base
`ebd5b375a54f1794f31584b5cc68490b6f9590d4` plus the thread's uncommitted logging
and cancellable-picker changes. HEAD alone does not identify this candidate.
Three independent read-only reviews covered Accounts/setup/import, Games/session/
Crash help, and Settings/native boundaries. The coordinating pass covered
navigation, common controls, windows, overlays, evidence inventories and native
spot checks. No product source was changed during this audit.

Inventory comes from the current [PageSpec and route selection](../crates/block-ui/src/navigation/navigation.rs#L103-L166),
[tabs](../crates/block-ui/src/navigation/navigation.rs#L12-L22),
and [drawer/surface definitions](../crates/block-ui/src/navigation/shell.rs#L25-L47):
eight page classes, seven tabs, five drawer kinds and ten surface variants.
Standalone Crash help, native dialogs, external applications and shared
controls are additional boundaries, not extra PageSpec pages.

## Repair disposition and verification

The repaired candidate is the integrated uncommitted working tree over the same
Git base, including the shared Input, Welcome and game/recovery packages.
Official Delta integration reported no new conflicts. Two independent read-only
reviews of the frozen integrated content found no remaining concrete defects:
one covered dispatch, repair targeting, layout and chooser ownership; the other
covered Crash export lifetime, native action semantics and Welcome activation.

| Issue | Implemented rule | Evidence and boundary |
| --- | --- | --- |
| G01 | Enabled state and rendered activation share the dynamic queue rule. | Rendered-control regression; Linux ARM64 `home-downloading` click changes Queue launch to Launch queued, and Stop changes the task to Stopping. No authenticated launch was performed. |
| G02 | Games/Profile detail exposes explicit Repair for the inspected target, including Ready targets, without changing Home selection. | Temporary-catalog test verifies the damaged-metadata refusal, correct repair target and world/config markers during an intentionally failed download. Native Bundled and Separated Profile details expose Repair. A completed successful download/repair was not replayed. |
| G03 | Editor scrolling is clamped to the current shaped line and viewport while keeping the caret visible. | Narrow/zero-to-wide rendered regressions; native Authlib field initially shows the complete fixture value and retains it through settled compact → wide → compact resizing. |
| G04 | Hover and activation share the resting circle; mouse-down and mouse-up must both be inside, touch uses its position, keyboard remains unconditional. | Red-before-fix rendered regression; native 360×540 content rejects the corner and both crossing gestures, while Enter still starts setup with the pointer outside. Native touch delivery was not tested. |
| G05 | Overflowing Home identity content starts at a reachable scroll origin. | Compact rendered regression; native 360×540 downloading Home shows the complete player name while retaining Stop and Queue launch. |
| G06 | Explicit return reconciles selected Profile/Instance and both browser inspection identities. | Two-profile regression in both modes; normal-engine native fixtures inspect B, receive A's synthetic crash, and return to A. Separated mode also shows A when reopening the Profile tab. |
| G07 | Crash export separates cancellable selection from accepted writing, cancels waiting on release, and checks the live view before writing. | Owner cancellation/release tests; native private-bus portal replay verifies keyboard cancellation, retry, ignored late cancelled response, current sanitized JSON output, and no file after Return releases Crash help. |
| G08 | All four chooser owners distinguish errors from quiet cancellation and retain draft/request/revision guards. | Production-handler and rendered-cancellation regressions; native folder picker receives an actual D-Bus error, displays an inline explanation inside Configuration and accepts a second request. Other chooser error branches were not individually replayed natively. |
| G09 | Shared inputs publish roles, names, values, validation/selection and supported actions; Crash actions use labeled native Base buttons; Home icon actions have explicit names. | GPUI semantic-tree/action tests and source review passed. Native Crash keyboard cancellation and focus recovery were observed. Native accessibility-tree probing was inconclusive; screen-reader acceptance remains unverified. |

Executed checks:

- **macOS ARM64:** `just check` passed, including formatting, locked workspace
  all-target/all-feature type checking, strict Clippy and workspace tests.
  The integrated UI suite ran **367 unit tests and 3 framework tests**.
- **Linux ARM64:** `cargo test --locked -p block-ui --all-features` passed
  (**367 unit tests and 3 framework tests**);
  `cargo build --locked -p block-app --bin block` passed.
- `python3 -m unittest discover -s scripts -p 'test_*.py'`: **16 passed**.
  `git diff --check`: passed.

Native observations used Debian 13 ARM64, kernel `6.12.113+deb13-arm64`,
KDE's Wayland session and Block's **Xwayland** backend. Synthetic scenarios,
catalogs and crash/session records were isolated under the task-owned
`block-logging-native-20261030T085846Z` root, with final captures under
`gui-fixes-20261030T121340Z/final`. No live authentication, Java launch,
gameplay or real player-data mutation was performed. Test applications and
private portal/bus processes were stopped; the repaired system portal was not
replaced or restarted for these replays.

### Limitations

The installed Linux AccessKit adapter does not implement AT-SPI `EditableText`;
advertising supported GPUI edit actions does not add that platform interface.
The synthetic text run also lacks spatial character geometry. Generic search
consumers retain a named TextInput role unless they already opt into SearchInput.
No custom accessibility bridge or dependency patch is included.

Native Windows/macOS, native Wayland, touch and screen-reader acceptance were
not repeated for this repair. The remaining whole-product review points below,
including multiple active sessions, import Stop/retry and data-policy journeys,
remain separate from these nine fixes. No product acceptance marker is promoted.

## Baseline confirmed issues

The following descriptions record the audited behavior before repair, not
remaining defects in the repaired candidate. Their source locations describe
the baseline and may have moved; current disposition is recorded above.

Priority reflects user consequence, not whether a test is missing.
“Source-confirmed” means a reachable implementation path was traced; it does
not mean a native reproduction or screen-reader session was run.

### G01 — P1: Queue launch is enabled but cannot activate

**Native-reproduced and source-confirmed.** During an install, Home renders an
enabled Queue launch action. Its enabled-state calculation uses the dynamic
`can_activate` predicate, but its callback uses the narrower static
`available_while_busy` allowlist, which rejects Play.

- [Enabled state and callback](../crates/block-ui/src/lib.rs#L1283-L1306).
- [Queue handler that should run](../crates/block-ui/src/lib.rs#L1112-L1121).
- Native `home-downloading`: clicking the primary action leaves Queue launch
  and its status unchanged.
- Existing queue tests call `handle(Play)` directly or check geometry, not this
  rendered callback. The next regression should activate the actual control.

**Fix boundary:** use one activation rule for presentation and dispatch.

### G02 — P1: Integrity failure directs users to an unreachable Repair action

**Source-confirmed.** A target can remain Ready while installed immutable files
or metadata are damaged. Launch refuses it with “choose Repair”, but Ready
targets expose Launch; Repair is offered for Pending/Installing/Failed targets,
not this case. Crash help does not provide Repair either.

- [Integrity refusal](../crates/block-launcher/src/library.rs#L270-L311).
- [Home action selection](../crates/block-ui/src/navigation/shell.rs#L463-L478).
- [Crash help actions](../crates/block-ui/src/feedback/crash_view.rs#L125-L140).
- No damaged real installation was created for this audit.

**Fix boundary:** expose the existing explicit repair operation for an installed
target, preserving immutable-resource quarantine and worlds/configuration.
An interrupted-download retry test is not evidence for this Ready-but-damaged
case.

### G03 — P2: Text fields can retain an invalid horizontal scroll after resizing

**Native-reproduced and source-confirmed.** At a 960×600 content viewport,
Authlib's account field initially displayed only `st` from the fixture value
`player@example.test`. Moving the caret to Home, without editing the value,
restored the complete string. The Instance name and Offline error captures
also showed only the tail of their values.

[TextElement's scroll calculation](../crates/block-ui/src/controls/text_input.rs#L750-L757)
retains `scroll_offset` across width changes without clamping it to the
remaining text overflow. A transient narrow field can therefore remain
scrolled after there is enough room for its entire value.

**Fix boundary:** clamp the common editor's scroll against the current shaped
line and viewport. Add narrow-to-wide and initial frame-growth tests rather
than special-casing individual forms.

### G04 — P2: Welcome activates outside its visible circle

**Native-reproduced and source-confirmed.** The visible circle is 160 px across,
but activation is attached to its rectangular Base button. The radial
calculation controls hover only.

- [Button construction](../crates/block-ui/src/setup/welcome_button.rs#L306-L344).
- Native 360×540 content viewport: clicking drawable `(125,253)`, four pixels
  inside the button's top-left bounding-box corner but outside the circle,
  invoked Start. The engine-free fixture displayed its setup-save error,
  demonstrating activation; this did not import games or persist real setup.
- The [existing hit-area test](../crates/block-ui/src/setup/first_run_tests.rs#L612-L659)
  checks hover/bounds, not corner-click rejection.

**Fix boundary:** share the intended pointer geometry with activation while
preserving keyboard activation.

### G05 — P2: Compact downloading Home clips the player identity

**Native-reproduced.** At the existing tested 360×540 content size, the
downloading Home clips the top of the player name beneath the title area.
Scrolling upward did not reveal it. Stop and Queue launch remained visible;
their visibility alone does not establish correct identity layout.

The identity scroll region uses
[center alignment inside its shrinking area](../crates/block-ui/src/navigation/shell.rs#L594-L615);
the status/task/action regions consume additional height.

**Fix boundary:** preserve a reachable start of scrollable content when it
overflows. Keep the compact Stop/action checks and add identity visibility.

### G06 — P2: Inline crash return can show a different game

**Source-confirmed.** If A fails while the user has inspected B, Return to
instance changes Home selection to A but can leave Games inspection at B.
The detail panel then prefers B, so a following Edit targets B.

- [Requested-profile selection](../crates/block-ui/src/lib.rs#L568-L587).
- [Inspected-profile precedence](../crates/block-ui/src/games/browser.rs#L190-L205).
- [Inline return callback](../crates/block-ui/src/lib.rs#L673-L679).

**Fix boundary:** explicit return navigation must reconcile the destination's
inspection identity as well as selection. Test two Profiles with conflicting
inspection in Bundled and Separated modes.

### G07 — P2: Crash help still has the stalled-picker and released-view export gap

**Source-confirmed.** Crash help awaits its save picker without an abandonment
path. If the view disappears while the process stays alive, a late selection
starts writing before the subsequent view update discovers that it is gone.

- [Crash help export](../crates/block-ui/src/feedback/crash_view.rs#L144-L181).
- [Standalone Return opens the launcher and removes Crash help](../crates/block-app/src/main.rs#L259-L270).

**Fix boundary:** apply waiting cancellation and a live-owner check before
writing to this separate exporter. About's ZIP-export tests do not cover
Crash help's JSON exporter.

### G08 — P2: Four native chooser paths silently discard errors

**Source-confirmed.** Picker errors and dropped receivers are treated like
quiet cancellation, making Browse/Open appear inert:

- [Installation folder](../crates/block-ui/src/lib.rs#L1169-L1197).
- [Development recipe](../crates/block-ui/src/lib.rs#L1210-L1227).
- [Wizard OptiFine](../crates/block-ui/src/games/game_wizard.rs#L536-L556).
- [Instance OptiFine](../crates/block-ui/src/games/instance_editor.rs#L421-L462).

**Fix boundary:** distinguish failure from user cancellation, preserve the
existing draft-identity guards, and expose actionable feedback. A successful
native choice does not exercise these error branches.

### G09 — P2: Handwritten inputs and Crash help actions lack native AX semantics

**Source-confirmed markup omission; native screen-reader impact not replayed.**
The common TextInput paints a custom TextElement and registers an input handler,
but supplies no text-field accessibility role/name/value. Its visible field
label focuses the editor on click without establishing a semantic label
relationship. Crash help's manually keyboard-roved actions are clickable Divs
without button roles.

- [Input paint/input-handler registration](../crates/block-ui/src/controls/text_input.rs#L796-L838).
- [Input root](../crates/block-ui/src/controls/text_input.rs#L841-L939).
- [Visible field label](../crates/block-ui/src/controls/components.rs#L430-L454).
- [Crash action construction](../crates/block-ui/src/feedback/crash_view.rs#L184-L209).

GPUI 0.3.9's installed implementation does not synthesize an accessibility role
from `Window::handle_input`; Div reports only an explicit non-generic role.
Native IME support and keyboard focus therefore do not establish accessible
field/action semantics.

**Fix boundary:** expose the appropriate roles, label/value/state/action
relationships at these owners, then inspect the actual native accessibility
tree and screen-reader interaction.

## Complete review inventory and remaining coverage

Owner tests and native records exist for many cells below. The last column
identifies the specific missing check, not an assertion that the whole area is
untested. G01–G09 references identify the audit's original findings; their
implemented fixes and remaining verification limits are recorded in the
disposition table above. Historical real-service checks retain their original scope.

| Product group | Surfaces and states included in this audit | Remaining discriminating review |
| --- | --- | --- |
| Window and global navigation | Custom title/drag/minimize/close, app menu, Home/management frame, rapid reversal, Settings/About previews | Work-area changes, mixed-DPI monitor movement, strict fixed-size user WM paths; modal versus chrome interaction |
| Home and selectors | No account/game; Ready/Pending/Installing/Failed; Download/Retry/Launch/Queue/Stop; account/profile sheets; search/empty/unavailable/current choice | G01/G02/G05; real control activation during busy state, not direct handler dispatch |
| First run | Welcome, resumed Overview, neither/account-only/game-only/both ready, Continue/Finish/Back/Skip and setup-save failure | G04; current setup states missing from the semantic review registry |
| Game entry and import | Recommended/custom/existing setup; Sources/List/Progress/Results; unsupported/imported/warning rows, Stop, close, retry remaining | Mixed-success Results and mid-copy Stop→retry→Overview controls/layout; current static import scenarios cover Sources/List only |
| Account browser | Loading/error/retry/empty/search-empty/list/detail, inspected versus selected identity, provider fallback, removal confirmation | Native confirmed removal/last-account reconciliation on current code; no real identity deletion in this audit |
| Microsoft authentication | Browser and Device Code, preparing/waiting/expiry/reopen/copy, verify/review/save/failure/back/cancel/method change | Current failure/preflight/expiry layouts and native focus; archived successful sign-in is not visual acceptance |
| Offline authentication | Name validation, review, Back, save/error/cancel | G03/G09; invalid/recovery and narrow-to-wide input presentation |
| Authlib authentication | Provider and HTTP consent, account/password, inline provider dialog, character list/verification failure, review/save | Empty/error provider choices, long HTTP/recovery content, nested-dialog Escape/focus on supported compact windows |
| Provider administration | Loading/error/empty/search-empty/list/add/edit/discover/duplicate/remove, linked-account locks, drop/paste and HTTP consent reset | Native pending discovery and failure presentation; native Wayland drop remains intentionally unsupported |
| Games browsers | Bundled and Separated Instance/Profile collections, queries/scroll/roving, inspection/selection, component cards, linked records and unavailable targets | G02/G06; actual delete/copy transitions with disposable world/config markers rather than confirmation-cancel alone |
| Instance editor | Name, Java/window/fullscreen/directory, OptiFine unsupported/select/replace/remove, validation/dirty/save/error/retry/cancel | G03/G08/G09; metadata-save success followed by reinstall failure/cancellation is distinct from save failure |
| Profile editor | Appearance; Launch; Data/account; inherited defaults, arguments, memory, target/server/Quick Play, Shared/Isolated policy, subpage Done/Cancel and final Save | GUI-driven data-policy/copy/delete effects using disposable files; long values and missing defaults/accounts |
| Game wizard | Bundled two-step and Separated new/existing Instance flow; Setup/Profile/Review; pending/failure; ordinary install versus first-run save-only | Real rendered entry/action chains, especially compact section dialogs rather than direct page assignment |
| Wizard components | Version catalog/search/snapshots/exact ID/ARM64 restriction; all Loaders and loading/error/no-build/search-empty/delisted states; OptiFine; Location/Java/window | G08; nested section Dialog/select/native chooser cancellation/focus and long-tail scrolling |
| Download presentation | Overall/current known/unknown totals, verified/cache/retry groups, empty state, drawer and stacked file-detail dialog | Live group changes while details are open; long resource/error strings; keyboard/focus during active updates |
| Session lifecycle | Pre-handoff, accepted preparation/running, Stop/stopping/failure, exit/crash, monitoring interruption/reconnect, reopen and queued launch | Two accepted active sessions and remaining-session representation after the newest ends; crash arrival while another modal is mounted |
| Crash help | Inline and standalone, diagnoses/evidence/details/attachments/truncation, folder/export/copy/return/close and logging notice | G06/G07/G09; long sanitized report, action focus/scroll and native picker error/lifecycle |
| General and Game settings | Theme/reduced motion preview; Java/memory/window/JVM/snapshots; section Restore, whole-draft Save/Cancel, hidden validation and save failure | Current native keyboard/focus and long-value/small-window presentation; preserve entered draft on every error |
| Profile mode and network settings | Confirmed one-way conversion; System/Direct/HTTP(S)/SOCKS proxy, limits, connection-test running/result/error and stale-result rejection | Conversion during other operations; unsaved/edited network draft versus in-flight response; real endpoint success has endpoint-only scope |
| Storage settings | Automatic library/cache paths, read-only view and configuration error; automatic first-run handoff | Path/error visibility and actual current location policy; relocation/cleanup controls are retired, not missing features |
| About and diagnostics | Build identity, native clipboard/links, output/recipe actions, log directory, export waiting/writing/success/cancel/error/retry and log-storage failure | G08; newest cancellation control's Windows/macOS native focus/layout; disk-write failure distinct from existing-file refusal |
| Shared controls and feedback | Text/password fields, checkboxes/radio/select/listbox, IME/clipboard/undo, tooltips, validation, toast stack/pause/swipe/exit, sheet/dialog stack/focus | G03/G09; native AX audit, keyboard-only journeys, CJK composition on each relevant backend, high scaling/long names, physical Toast timing |

Cross-page checks must follow the actual state as well as the page name:
loading/empty/search-empty/error/retry, dirty/saving/save-failed, disabled versus
enabled-but-inert, cancellation and late results, stacked modal ownership, and
focus/scroll after a control or page disappears.

## Evidence inventory omissions

### Current scenarios and semantic review cases differ

Current [SCENARIOS](../crates/block-ui/src/runtime/scenarios.rs#L28-L101) has
**72** entries. The [semantic review registry](visual-review-cases.json#L42)
now has **72 active cases and six explicitly retired historical cases**.
The registry repair added the eight previously omitted current native stems:

`setup-welcome`, `setup-overview`, `setup-overview-ready`, `setup-game-choice`,
`setup-import-sources`, `setup-import-list`, `setup-account`, `setup-game`.

The six retained historical native stems are `welcome-1`, `welcome-2`,
`welcome-3`, `welcome-about`, `welcome-settings`, `welcome-version-picker`.
Their provenance starts with `retired:` and they do not count as current
scenario coverage. A registry test requires the active stems to match the
current scenario set exactly once each.
The current [capture contracts](../scripts/scenario_contracts.py#L119-L127)
already covered the new setup scenarios; scenario registration was not missing.

The registry retains unknown focus and scroll fields.
These intentionally preserve archive uncertainty: they must not be interpreted
as proof that those interactions failed, or that all corresponding tests are
absent. Separate current capture observations may exist.

### Seventy-two scenarios are not a complete product-state inventory

The current static scenario list does not independently seed:

- Profile editor Appearance, Launch, and Data/account pages.
- Inline and standalone Crash help.
- Import Progress/Results, mixed outcomes, stopped batches and retry remaining.
- Games/Accounts/Providers collection loading, failed, empty and search-empty
  variants as a complete management-state set.
- Settings connection-test running/result/error, corrupt Storage presentation,
  or About log-export waiting/writing/failure states.
- Accepted game running/stopping, monitoring interruption, multiple active
  sessions, or crash arrival above an existing modal.

Some are covered by owner tests, historical native interactions or the recent
dedicated log-export replay. They can also be reached interactively from other
scenarios. The omission is that a static screenshot matrix cannot count these
as independently captured states merely because it covers every registered
scenario.

### Historical rows do not all describe current GUI behavior

- [STATUS Storage relocation/cleanup rows](STATUS.md#L168-L169) describe retired
  GUI controls. Its [later automatic-storage note](STATUS.md#L227) and current
  Settings source supersede them.
- [The retries note](STATUS.md#L167) says 11 was clamped to 10; current settings
  parsing rejects values above 10.
- Old Welcome mappings and inline-provider descriptions in the semantic
  registry still describe the previous flow or a separate OS window.
- A Storage scenario's semantic description attributes paths to its isolated
  root, while the [current fixture supplies fixed synthetic paths](../crates/block-ui/src/runtime/scenarios.rs#L437-L445).
- Historical 142-shot matrices and older test totals are not evidence that
  this candidate's 72 scenarios, new controls and changed layouts are all
  accepted. [Accepted remains Pending](STATUS.md#L224).

## Checks actually performed

### New in this audit

- Inspected current route/surface definitions, owner implementations and tests,
  capture contracts, semantic registry and relevant native reports.
- `python3 -m unittest discover -s scripts -p test_scenario_contracts.py`:
  **4 passed**.
- `python3 -m unittest discover -s scripts -p test_review_cases.py`:
  **5 passed**. Their success does not test equality of the two inventories
  described above.
- Captured and visually inspected **16 native scenario/theme combinations** on
  Debian 13 ARM64, Linux `6.12.113+deb13-arm64`, KDE Wayland with the application
  on Xwayland `:1`. Compact captures used 360×540 Home or 960×600 management
  content; X11 drawable sizes include 21 px insets on every side.
- Native interaction probes reproduced G01, G03, G04 and G05 without live
  accounts, installation, real game import or player-data mutation.
- Captures and fixture-only stderr are retained in
  `target/gui-coverage-audit/linux-arm64/`.

The 16 combinations were Welcome, Overview, Import List, Authlib information,
Offline error, Wizard Review, Version picker, Instance settings, Separated
Games, Downloads settings, Storage settings, Home profile drawer, account
removal confirmation, Home error, Home downloading and About. Their themes
were selected spot checks, not a complete two-theme matrix. Static capture did
not confirm destructive actions or service-backed success.

The Import drawer blocked the first capture runner's background chrome-close
click; that test app was terminated for cleanup. Subsequent capture cleanup
dismissed overlays first and recorded forced cleanup where necessary. This
was not counted as a product close/focus pass or a new defect.

### Reused within scope

The prior macOS ARM64 `just check`, 348 UI tests, Linux ARM64 focused About tests
and bounded native log-export/cancellation/late-result checks remain valid for
unchanged code. Their detailed scope is in [logging](logging.md#L99). They were
not rerun to manufacture fresh whole-product evidence.

### Not established by this audit

There was no new full workspace build/test run, full-platform/native-Wayland
matrix, mixed-DPI monitor test, screen-reader session, live sign-in, real game
lifecycle, real player import or exhaustive theme/state capture. Source-traced
defects and missing evidence remain distinct. No Accepted markers changed.

## Follow-up order

1. Restore truthful operation reachability: G01 Queue launch and G02 explicit
   Repair, with rendered-control tests.
2. Fix the common input boundary: G03 scroll clamping and G09 semantic fields;
   their consumers span Accounts, Games, wizards and Settings.
3. Close the crash-recovery chain: G06 destination identity and G07 export
   lifetime, then G08 native chooser failure feedback.
4. Correct G04 pointer geometry and G05 compact Home overflow.
5. Reconcile review mappings, then fill the specific state/native gaps in the
   inventory, starting with import Progress/Results and modal/crash interaction.

Implementation and landing are separate follow-ups; this audit changed
documentation only.
