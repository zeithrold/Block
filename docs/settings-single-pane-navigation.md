# Single-pane Settings navigation

## Design contract

The user selected **A: category index and drill-in pages**, with **Home and
Settings at the same size**. The research budget is Home's current
[360 × 720 logical visible-content pixels](home-management-frame.md#dimensions-and-rationale).
The earlier [620 px A board](settings-layout-comparison.md#paired-boards) is not
a narrow-window prototype. There is no authorization here to enlarge Home to
accommodate that board.

The goal is to find a setting, edit it and return without losing orientation
or mistaking navigation for saving. Prefer a shallow, predictable hierarchy
over exposing all destinations at once. The design study below records the
selected direction; [implementation and native evidence](#implementation-and-native-evidence)
distinguishes the delivered behavior from the static storyboard.

## Official patterns inspected

| Source | Explicit guidance | Application to Block |
| --- | --- | --- |
| [GNOME Browsing](https://developer.gnome.org/hig/patterns/nav/browsing.html) | Content opens another view; Back returns. In left-to-right locales Back occupies the top-left, and the header reflects the current view. Support standard navigation shortcuts. | Keep one recognizable Back location and a category-specific heading. |
| [GNOME Header Bars](https://developer.gnome.org/hig/patterns/containers/header-bars.html) | Keep few controls, adapt them to context, and retain blank dragging space. | Do not pack Back, five tabs, Save and a menu into the narrow title bar. |
| [GNOME Boxed Lists](https://developer.gnome.org/hig/patterns/containers/boxed-lists.html) | Small static lists can combine titles and supporting information. Navigation rows have a trailing next arrow; organize rows semantically. | Five category rows are sufficient for the index, with short descriptions and clear navigation affordances. |
| [GOV.UK Error Summary](https://design-system.service.gov.uk/components/error-summary/) | Explain validation failures, link to the affected inputs and keep summary and inline wording consistent. Its own pattern moves focus to a summary. | Make hidden errors reachable and understandable, but do not adopt the web service's mandatory summary/focus model wholesale. Block currently focuses the invalid field. |

These sources were read as official text. They support navigation and feedback
relationships, not proof that a particular 360 px layout is attractive or easy
to use. GNOME's widgets and GOV.UK's web markup are not implementation
dependencies for this GPUI application.

## Recommended navigation: index, category, return

Use this small hierarchy:

```text
Home
  Settings index
    General
    Game defaults
    Profile mode
    Downloads & network
    Storage
```

Only one view occupies the content area at a time. Categories are destinations,
not wizard steps: no numbered rail, Next button or implied completion order.
Do not add a second set of permanent tabs or a hidden category drawer. With five
destinations, an index is a legible starting point; a search facility is not
needed to compensate for unclear category names.

### Index: help users choose before entering

Give each category a full-width navigation row with a name, a short scope
description and a trailing chevron. Example content:

| Category | Scope description |
| --- | --- |
| General | Language, appearance and motion |
| Game defaults | Memory, Java and window defaults |
| Profile mode | How instances and profiles work together |
| Downloads & network | Downloads, retries and proxy |
| Storage | View data and game locations |

The whole navigation row is one action; do not put a miniature settings form
inside it. Descriptions should wrap rather than disappear behind ellipses.
Mark a category with pending edits using text such as “Modified”, not only a
colored dot. This is draft state, not a task-completion badge.

The index title is Settings. Its Back action returns to Home, subject to the
dirty-draft exit rule below. On returning from a category, restore index scroll
and focus to the row that opened it. An index should be a useful map, not an
interstitial screen users must decipher again on every visit.

### Category: show location without recreating a sidebar

Use a category-specific title, such as Downloads & network, with Back in the
same top-left location. Its accessible label and tooltip should name the
destination, for example “Back to settings”. Do not stack a large Settings
title, breadcrumb trail and another large category title.

Back from any category returns to the Settings index and keeps the draft.
This remains true after validation routes to a category: it is a stable parent
relationship, not an unpredictable return to whichever page happened to
initiate Save. At the index, a second Back returns to Home.

Use Alt+Left for navigation; handle a menu or dialog's dismissal before page
navigation. Do not overload Escape with unconditional draft discard. Keyboard
focus follows the new context; returning restores the originating row.

## Make saving understandable without a sidebar

Preserve the current shared draft and
[save-all validation boundary](../crates/block-ui/src/settings/settings.rs#L1043-L1125).
Narrow layout is not a reason to invent independent per-category persistence.

- Keep a consistent bottom action area with **Save all changes** as the primary
  action and a compact global dirty-state message. It is available at the
  index and category views, including read-only Storage when another category
  has pending edits.
- Remove the three-button reset/cancel/save row from the narrow prototype.
  Put **Reset this category** in its category content, separate from one-way
  mode conversion. Back supplies navigation; it is not a substitute label for
  Cancel.
- Leaving Settings for Home with pending changes offers **Save and leave**,
  **Discard changes**, and **Keep editing**. Apply the same rule when leaving
  from Storage. This is a proposed explicit exit contract, not existing
  behavior demonstrated by the old Close button.
- Successful ordinary Save stays in Settings and reports success. If validation
  or persistence fails, retain the draft and stay in the editing session.
  Save-and-leave only leaves after successful persistence.
- Keep the existing live-preview semantics for language, appearance and motion.
  Discard restores saved preferences. Completed mode conversion is separate
  and cannot be undone by discarding the preference draft.

For example: edit Game defaults, return to Settings, edit Downloads, then Save.
Both categories' edits are included. Back never silently commits or discards
either one.

### Validation is guided correction, not unexplained navigation

For a field error, route to its category, reveal any conditional field, retain
the typed value, scroll it into view and focus it, matching the
[existing field-targeted behavior](../crates/block-ui/src/settings/settings.rs#L899-L928).
Add concise context such as “Changes were not saved. Check the proxy address.”
near the correction; do not rely on the category changing to explain failure.

For the current first-error contract, do not fabricate an all-errors dashboard
or counts for categories that have not been validated. If several categories
contain invalid input, correcting the first error and saving again may reveal
another. The next prototype must test whether this is understandable.

After a failed Save, Back still returns to the index with the draft intact.
A persistence failure without an owning field stays in the current context
with retry guidance rather than sending users to an unrelated category.

## Recompose for 312 px, do not shrink the 560 px form

With Home's existing 24 px side padding, the working body width is 312 px.
This is a starting budget, not a rendered-fit claim.

- Place field labels above wide selectors and text inputs. Stack memory and
  Java sections rather than preserving the wide layout's side-by-side fields.
- Show custom Java and proxy inputs only when the controlling choice needs
  them. Give paths full-width space and an inspectable/selectable value;
  horizontal field scrolling is not a substitute for understanding a path.
- Prefer vertically stacked width and height controls initially. Keep units
  explicit. Test any paired-field alternative against English labels and
  native text scaling before choosing it.
- Group longer Game defaults content into sections with headings. Keep the
  category navigation shallow; do not make every setting a new page.
- Allow the body to scroll while keeping navigation and the primary action
  reachable. A footer may grow for translated labels or text scaling; do not
  assume the old 72 px footer fits every state.

Category return should preserve a useful reading position. The
[implemented category switch](../crates/block-ui/src/settings/settings.rs#L956-L975)
retains its scroll handle and focuses the visible viewport on return.
Field-error navigation deliberately overrides a saved scroll position to
reveal the field needing correction.

## Motion: support the parent relationship

Home → index and index → category can use local, restrained directional
transitions with a corresponding return. Keep native bounds unchanged between
Home and Settings. Avoid full-width page flights, field-by-field entrance
staggering and window expansion.

Selection should respond immediately. Interrupted navigation must settle on the
latest destination without blocking editing. Error correction should emphasize
the destination and explanation, not replay multiple navigation transitions.
Reduced motion retains the same hierarchy, focus and return behavior without
depending on travel for orientation.

No duration or easing curve is chosen by this text-only study.

## 360 × 720 storyboard

The [single-pane storyboard](design/settings-layout/03-single-pane-360.svg) has
twelve fixed-size frames. Read left to right, then down. Frame numbers are
review annotations, not steps shown to the player.

| Frames | What the design examines |
| --- | --- |
| 01–03 | Choose a category, edit General, and return to its focused index row with a Modified label. |
| 04–05 | Stack memory, custom Java and window controls; scroll to dimensions, JVM arguments and remaining actions. |
| 06–08 | Edit Downloads, return to the index and Save all; arrive at an invalid proxy field, retain the draft and correct the address. |
| 09 | Return to the index and request Home; distinguish Save and leave, Discard changes and Keep editing. |
| 10 | Alternative ordinary-Save outcome: remain at the index with a saved state and disabled Save. |
| 11–12 | Inspect English category labels and a full long Java path at the same window size. |

Home itself is unchanged and not redrawn. Every illustrated window uses its
360 × 720 size. Transitions through the index are described in captions where
the intervening view would repeat; frame 10 is an alternative branch, not a
screen reached after Save and leave. Example values and paths are not captured
runtime state.

### Composition decisions visible in the artifact

- One integrated 18 px category title occupies the 56 px header. There is no
  second oversized page title in the body. This is a narrow-layout study role,
  not a change to production tokens.
- The study retains the current Inter/system-fallback family, 14 px body,
  12 px supporting text, 32 px ordinary controls and existing rounded-control
  vocabulary. Local font fallback is not evidence of Linux glyph metrics.
- The index uses shared edges and separators rather than five competing cards.
  Text descriptions explain scope before entry. The framed row in 03 represents
  restored keyboard focus, not a persistent selection panel.
- The footer grows from the existing 72 px to 88 px in the study to separate
  the global draft message from one full-width primary action. Labels are not
  reduced to force three actions onto a row.
- The error state uses local emphasis, an explanation and a focused field.
  A corrected value is not shown as saved or network-tested.
- Frame 12 exposes a full path below its compact field. The value is manually
  divided at directory boundaries for this study. Editing must retain the real,
  unabridged string; the displayed ellipsis is not a persisted input value.

### Interaction decisions not established by the drawings

The visible title-bar Close is a window action, not another Back button. A
native implementation must distinguish leaving Settings for Home from closing
the launcher, and name the actual destination in any dirty-draft prompt.
Closing the launcher must preserve accepted game supervision. The storyboard
only illustrates the Home-return confirmation.

The corrected-field frame represents editing after failed validation, not a
demonstrated live-validation policy. When the inline error clears and how that
is announced need native review. Storage and one-way mode conversion retain
the contracts above but do not yet have 360 px detail/confirmation frames.

## Evidence and next validation

All twelve static frames were rendered with macOS ARM64 Quick Look and inspected
in four row captures. XML parsing, unique IDs and internal references were
checked. The rendered Chinese and English samples fit their shown body and
action regions; this does not establish automatic wrapping for different text.
Captures are scratch evidence; the SVG is the maintained design artifact.

The static study's validation scope is separate from the implementation evidence
below. Manually placed labels are not responsive-layout evidence.

No production code, build or behavioral test suite ran for this static study.
It does not establish scrolling, accessibility, motion comfort, dark-theme
appearance or unchanged runtime window bounds. Native Linux ARM64 interaction
remains required for implementation acceptance.

## Implementation and native evidence

Candidate `cd7ff3125b4fcf35419dffed07e70a3104650cc4` implements the selected
navigation without changing Home's composition:

- [PageSpec](../crates/block-ui/src/navigation/navigation.rs#L245-L278) assigns
  Settings the Home frame and single-pane host. The category heading changes
  in the custom chrome; the [native title](../crates/block-ui/src/navigation/shell.rs#L326-L335)
  remains localized `Block · Settings`.
- [Category navigation](../crates/block-ui/src/settings/settings.rs#L956-L1023)
  retains reading position, restores the originating index row, and reveals
  controls reached through keyboard traversal. Category changes settle
  immediately rather than retaining an inert outgoing form.
- [Save all](../crates/block-ui/src/settings/settings.rs#L1043-L1125) validates
  the shared draft and remains in Settings. Dirty exits distinguish Home,
  native Close and Quit; failures keep the draft. One-way Profile conversion
  remains separate from preference rollback.
- Narrow confirmation actions stack. Alt+Left cancels an active Settings
  dialog before navigating the underlying page.

### Executed checks

| Host | Evidence |
| --- | --- |
| macOS ARM64 | Full `just check` passed on the initial implementation. The focused repair passed workspace strict `just clippy`; the integrated candidate passed `just fmt-check`, `cargo test --locked --package block-ui` (582 unit + 3 framework tests), `cargo build --locked --package block-app --bin block`, seven scenario-contract Python tests and `git diff --check`. |
| Windows ARM64 | The integrated candidate passed `cargo test --locked --package block-ui` (581 unit + 3 framework tests) and `cargo build --locked --package block-app --bin block`. |
| Linux ARM64 | Integrated candidate passed `cargo test --locked --package block-ui` (582 unit + 3 framework tests) and `cargo build --locked --package block-app --bin block`. Native light-theme navigation, scroll/focus restoration, conversion confirmation and dirty Close were verified on Debian/KDE Wayland. |
| Windows AMD64 | Initial full package run had one discard-navigation failure; the unchanged isolated rerun passed. The assertion remains intact with stronger diagnostics. Final full-suite rerun and native checks are in progress. |
| Linux AMD64 | Integrated candidate passed `cargo build --locked --package block-app --bin block`; native light-theme index, General and dirty Close dialog were inspected on Debian/XFCE X11. |

On macOS ARM64, all nine Settings scenarios passed strict title, logical
dimensions, PNG scale and required-text validation in both light and dark
themes: **18 captures at 360 × 720**. Representative Game, Profile mode,
SOCKS5, Storage and dirty-exit images were visually inspected. Native
interaction also verified the stacked conversion dialog, Alt+Left dismissal
without leaving Profile mode, retained Game scrolling, and Tab revealing the
Memory field and label after return.

On Windows ARM64 at 200% display scaling, native dark-theme Profile mode and
the stacked conversion confirmation were inspected. Alt+Left dismissed the
confirmation without leaving the category. The 724 × 1442 outer window
includes native borders; it is not a 724 px logical content requirement.

On Linux AMD64, the 402 × 762 outer bounds include client-side frame insets.
Native interaction verified index-to-General navigation, motion-preview edits,
the native Close guard, Alt+Left dismissing that guard without leaving Settings,
and Discard closing the window. The window-capture path produced horizontal
black bands while a simultaneous desktop capture showed intact content;
desktop captures, not the corrupted window images, supplied the visual evidence.
AT-SPI reads exposed controls, but subsequent token actions reported expired
objects; desktop pointer input supplied the tested actions.

On Linux ARM64, desktop captures showed the 360 × 720 content intact. Native
interaction verified General's motion edit remaining pending across categories,
Game's memory edit and retained reading position after returning through the
index, and Tab revealing the Memory input and its label. The conversion dialog's
two actions fit vertically; Alt+Left dismissed it without leaving Profile mode.
Native Close offered all three dirty-exit actions, Keep editing retained the
draft, and Discard closed the application. The Wayland window was absent from
window enumeration, so authorized desktop input and captures supplied this
evidence; these are not strict native-title/OCR capture-contract results.

An earlier native macOS normal-engine run verified cross-category Save from
Storage, saved memory/motion preferences after restart, Chinese live preview,
and discard through the native Close prompt. Scenario fixtures have no engine:
they establish validation and draft retention, not successful persistence.
Local native artifacts are under `target/settings-single-pane/` and
`target/parity-shots/native/`.

### Acceptance limits

Acceptance remains pending the final Windows AMD64 checks.
Static captures and GPUI test windows do not establish
native IME composition, completed application Quit, game survival after launcher
closure, or subjective motion comfort. No macOS AMD64 host was exercised.
Independent source reviews accepted the draft lifecycle and scoped layout
repairs; they do not substitute for these native checks.
