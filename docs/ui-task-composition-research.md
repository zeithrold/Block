# Task composition and window design research

## Direction

Reassess Block from complete player tasks, not from an existing component
catalog or a predetermined window shape. In particular, Settings does not yet
have an established task-based reason to use the same wide frame as game
management.

This is a research proposal, not an approved redesign. The inspected baseline
is clean Git revision `92b70c6212a5b95012f4429453498d58d4442c28`.
The user's request reopens presentation assumptions, including window width,
sidebar geometry and container choice. Existing persistence, authentication,
player-data and supervisor contracts still govern the behavior being presented.

## What the current implementation establishes

| Evidence | Observation | Design implication |
| --- | --- | --- |
| [Frame rationale](home-management-frame.md#L9-L21) | Home is 360×720; management is 1152×720; the sidebar preserves Home's 360 px width. The rationale explicitly calls the ratio a product choice. | The geometry was selected to relate two frames, not established as necessary for every management task. |
| [Page composition](../crates/block-ui/src/navigation/page_host.rs#L46-L77) and [management host](../crates/block-ui/src/navigation/page_host.rs#L107-L154) | Browsers, providers, Settings and About share a sidebar/detail host with a fixed 360 px sidebar. | A record list, a settings category list and application information inherit the same space allocation despite different jobs. |
| [Settings categories](../crates/block-ui/src/settings/settings.rs#L955-L990) and [content](../crates/block-ui/src/settings/settings.rs#L1103-L1137) | Five categories; the scrolling detail uses a centered container capped at 608 px, including its padding. | Wider outer geometry does not imply a correspondingly wider useful form. |
| [General](../crates/block-ui/src/settings/settings.rs#L1361-L1413) | Language, appearance and reduced motion, with preview guidance. | No visible task here requires comparing two independent panes. |
| [Game defaults](../crates/block-ui/src/settings/settings.rs#L1415-L1515) and [Downloads](../crates/block-ui/src/settings/settings.rs#L1525-L1580) | Conditional Java/window/proxy controls, numerical inputs and a connection test; some controls are arranged in pairs. | Dependencies, scope and validation deserve more attention than preserving paired columns. |
| [Storage](../crates/block-ui/src/settings/settings.rs#L1583-L1645), [Profile mode](../crates/block-ui/src/settings/settings.rs#L1648-L1711) and [footer](../crates/block-ui/src/settings/settings.rs#L1154-L1205) | Read-only information, an explicit mode-conversion action and ordinary draft editing coexist under Settings; footers already differ. | A single generic settings-form treatment is not an adequate behavioral model. |
| [Preview and save](../crates/block-ui/src/settings/settings.rs#L807-L897) | Some changes preview immediately, while durable preferences are explicitly saved. | Container changes must explain and preserve preview versus commit; a switch-shaped control does not authorize autosave. |
| [Browser state](../crates/block-ui/src/games/browser.rs#L1-L51) | Inspecting a record is separate from choosing the game/account used by Home. | Any new list/detail design must keep inspection, launch selection and running identity distinct. |
| [Import results](../crates/block-ui/src/games/import_games.rs#L915-L957) and [pack phases](../crates/block-ui/src/games/modpack_import.rs#L63-L82) | Batch outcomes are per item; pack import distinguishes review, blocked, stopped, installing and complete. | A single progress screen or success checkmark cannot represent the entire task. |

These are source and documentation observations, not a fresh rendered review.
They identify assumptions worth testing, not newly reproduced defects.

## Reference patterns

### GNOME: settings as semantically grouped rows

The official [Boxed Lists guidance](https://developer.gnome.org/hig/patterns/containers/boxed-lists.html)
describes short static lists for preferences, with specialized switch, combo,
entry, spin, property and expander rows. It distinguishes these from large,
dynamic list views. A row generally contains one control, at most two; rows can
also navigate to a subview. Lists have minimum and maximum widths.

Transfer to Block:

- Use a coherent settings group rather than making every field its own card.
- Combine the memory value and adjustment into one numerical control.
- Present read-only storage as properties, not as a form waiting to be saved.
- Reveal a custom Java path or manual proxy only when its controlling choice
  requires it.
- Keep searchable version catalogs out of small preference-row containers.

This is an interaction/composition reference, not a proposal to adopt GTK or
copy GNOME's palette and dimensions.

### Libadwaita: two panes are a presentation, not a navigation requirement

[NavigationSplitView](https://gnome.pages.gitlab.gnome.org/libadwaita/doc/main/class.NavigationSplitView.html)
documents a concrete behavior: sidebar and content appear together when
expanded, but become a navigation view when collapsed. The content then has a
Back action. Sidebar width has its own minimum, maximum and preferred fraction.

The [GNOME adaptiveness guidance](https://developer.gnome.org/hig/guidelines/adaptive.html)
also explains why large widths can separate related controls and make text hard
to read, and recommends maximum-width content containers.

Transfer to Block: preserve category/record identity, draft, search and scroll
position across a change between drill-in and side-by-side presentation.
Do not interpret a sidebar's existence as proof that it must remain visible.
Libadwaita's numerical defaults are not proposed Block dimensions.

### GNOME: auxiliary detail is different from primary navigation

[Utility Panes](https://developer.gnome.org/hig/patterns/containers/utility-panes.html)
are subordinate controls or information relevant to the main view. They can be
transient, and overlap the main view when there is insufficient width.

Transfer to Block: a diagnostic/file detail may be an auxiliary view; game
selection and settings-category navigation are different responsibilities.
Making all three look like a right-hand drawer would conceal this distinction.

### elementary: keep configuration with the object it affects

[Accessible Configuration](https://docs.elementary.io/hig/design-philosophy/accessible-configuration)
recommends sane defaults, obtaining information automatically, keeping controls
contextual and avoiding uninformed engineering decisions during setup.

Transfer to Block: distinguish launcher behavior, defaults for future games,
and configuration of a particular game. A player trying to fix one game's Java
must not accidentally edit only the defaults for future games.

elementary also advocates immediate saving in its own desktop conventions.
That is an alternative interaction policy to evaluate, not a policy Block
inherits merely by borrowing its settings composition.

### shadcn: compose task-specific behavior rather than a universal large widget

The official [Sidebar documentation](https://ui.shadcn.com/docs/components/sidebar)
separates persistent header/footer, scrollable navigation groups, item actions
and collapsible presentation. Its width is configurable and its open state can
be controlled. The useful lesson is explicit region ownership, not a fixed
sidebar silhouette.

The official [Data Table guide](https://ui.shadcn.com/docs/components/data-table)
explicitly argues against a universal data-table component: selection, sorting,
filters, row actions and pagination depend on the application. Its example
separates those concerns instead of treating a visually finished table as a
complete product workflow.

Transfer to Block: batch import needs explicit eligible/selected/saved/failed
semantics. Ordinary game switching does not need a spreadsheet just because
one exists in a reference system. A library demo's selection semantics are not
automatically appropriate for Block's batches.

### Microsoft: separate content width from window width

The official [app settings guidance](https://learn.microsoft.com/en-us/windows/apps/design/app-settings/guidelines-for-app-settings)
recommends a single scrollable column, related settings under section headings,
and SettingsCard/SettingsExpander composition. Expanders reveal subordinate
options without deeper-than-one-level nesting. Disabled controls need a reason.
Its approximately 1000–1100 px recommendation is a **maximum content width**,
not a minimum window width or an instruction to enlarge the window.

The [list/details pattern](https://learn.microsoft.com/en-us/windows/apps/develop/ui/controls/list-details)
explicitly uses one pane with drill-in/back navigation at narrow widths and
simultaneous panes at wider widths. Its Windows-specific breakpoints are not
Block requirements.

Transfer to Block: decide outer window width, navigation width and content
reading width independently. Reserve simultaneous panes for tasks that benefit
from repeatedly switching objects. Microsoft's immediate-effect preference
policy conflicts with Block's current preview/save boundary; borrowing its
composition does not resolve that policy choice.

### Ant Design: complexity comes from relationships and submission boundaries

The official [Form Page research](https://ant.design/docs/spec/research-form/)
distinguishes basic, grouped and sequential forms. Large forms are decomposed
by relevance or dependency, while the task may still be submitted together.
It starts with single-column reading; short, related controls can share a row.
For settings it explicitly distinguishes independent immediate changes from
interdependent settings requiring submission, and recommends one effect mode
per page.

The [Data List pattern](https://ant.design/docs/spec/data-list/)
distinguishes tables for comparing attributes, lists for scanning summaries in
limited space, and cards for object presentation. It provides different detail
routes: inline expansion, drawer, dedicated page or double-column browsing.
Returning should restore the prior browsing position.

Transfer to Block: a full-page version selector may be simpler than a long
catalog inside a modal inside a wizard. A concise normal game-creation path
need not inherit a multi-step enterprise form. Separate preference editing
from mode conversion before deciding whether they should share a footer.
Do not adopt Ant's field-count thresholds as product-independent rules.

### Carbon: large collections need explicit action scope

The [Data table guidelines](https://www.carbondesignsystem.com/building-blocks/core/components/data-table/guidelines)
separate whole-table tools, per-row actions and batch actions. Selection opens
a batch action bar; Carbon disables row-level actions during that mode to
avoid ambiguous scope. Expanded rows contain supplementary information; an
overcrowded expansion should become a more suitable detail surface.

Carbon gives tables ample width to avoid truncation. This is a reason for
width grounded in comparing columns, unlike enlarging a preference form
because it is called a management page.

The [Filtering pattern](https://www.carbondesignsystem.com/building-blocks/core/patterns/filtering)
distinguishes immediate updates from choosing several filters and applying
them together. Hidden filters still need a visible applied count and a way to
clear them without reopening the container.

Transfer to Block: show what an import action operates on, which rows are
eligible, and what remains after partial success. Keep active version/Loader
filters visible enough to distinguish an empty result from missing data.
These rules do not justify adding tables, pagination or elaborate filters to
a small collection without a player task that needs them.

### PatternFly: deciding, submitting and executing are different phases

The official [Wizard guidelines](https://www.patternfly.org/components/wizard/design-guidelines/)
cover dependent steps, review, submission, progress and completion. They
distinguish modal focus from in-page navigation that requires application-owned
draft handling. Narrow presentations collapse the step navigator instead of
continually squeezing the body. After submission, the navigation is hidden
when prior steps can no longer be edited; background work can expose a monitor
link and a Close action.

The [Progress stepper guidelines](https://www.patternfly.org/components/progress-stepper/design-guidelines/)
distinguish guiding user decisions across screens from communicating stages
that may execute automatically. They include pending, active, warning, failure
and completed states, and horizontal, vertical and compact compositions.

Transfer to Block: create/import review belongs to user decisions; preparation
belongs to system execution. Closing a progress view, stopping preparation and
stopping a game must remain separate actions. PatternFly's post-submit Cancel
explicitly promises undoing work back to the pre-wizard state. Block must not
borrow that promise where it only supports stopping and retaining completed
work. A documented progress component is not evidence of resumability or
rollback in the underlying implementation.

## Settings: compare different task structures

The leading hypothesis is **a compact, single-content preference surface with
grouped rows and drill-in detail**, rather than the current mandatory wide
management frame. “Compact” does not mean forcing every task into Home's
360 px width.

Three candidates should be compared with the same content:

1. **Single-content settings.** A category index opens a grouped preference
   page; Back returns to the index with position preserved. Conditional
   technical fields remain near their controlling choice. This prioritizes
   one-task focus and screen fit, at the cost of an extra navigation action
   when changing categories.
2. **Moderate-width preferences.** A compact category navigator and one bounded
   content column remain visible together. This supports repeated cross-category
   work without reserving a Home-sized column for five labels.
3. **Wide management.** Keep simultaneous navigation and detail, but justify the
   space with an actual need for comparison or frequent object switching.
   The current frame is the comparison baseline, not the default winner.

Do not choose exact new dimensions before checking long Chinese/English labels,
custom paths, validation messages, short displays and the reachable footer.
Likewise, manual resizing, fixed task-specific sizes and automatic presentation
changes remain alternatives; no one policy has been selected here.

An illustrative structure for the first candidate:

```text
Settings
  General                 Language · appearance · motion
  New-game defaults       Only affects games created later
  Downloads and network   Limits · proxy · connection test
  Storage                 Effective locations
  Advanced                Profile organization

Settings / Downloads and network
  Download limits
    Concurrent downloads       [value]
    Retry limit                [value]
    Speed limit                [value]
  Connection
    Proxy                      [System / Direct / Manual]
    Manual address             [shown when needed]
    Test current draft         [Test]
    Result belongs here, with its tested scope

  Unsaved changes              [Cancel] [Save]
```

The labels and grouping above are proposals, not final copy. Moving Profile
organization does not change its one-way conversion semantics. Storage remains
read-only. The sketch retains explicit save rather than deciding persistence
policy through a visual redesign.

### Consequences beyond Settings

The reference systems do not support keeping or discarding one global design
system wholesale. Their useful patterns solve different tasks:

- Prefer grouped rows for small static preference sets, a bounded form for
  dependent input, and a searchable list for a catalog.
- Treat a management browser's sidebar as an efficiency feature, not the
  universal foundation of every non-Home page.
- Give a short account task a short path; a provider flow with recognition,
  authorization or character selection can require a different composition.
- Let technical detail become a dedicated workspace when it needs one, rather
  than stacking increasingly large dialogs.
- Reconsider window morph and pane motion after choosing the task structure.
  Existing animation work establishes implementation behavior, not a reason to
  retain the geometry it animates.

The recommended first comparison is the single-content and moderate-width
Settings candidates against the current wide baseline. This is a tractable
test of the shared-frame assumption. Game management and import remain
separate composition decisions, rather than being narrowed automatically to
match Settings.

## Reassess the other large compositions

| Player task | Composition to explore | Deciding issue |
| --- | --- | --- |
| Switch the next game or account | Quick selector with sufficient identifying context | Fast selection and return to Play, not record editing. |
| Inspect and manage games | List/detail when repeated comparison is useful; drill-in when space is constrained | Keep browsing separate from Home selection and the running session. |
| Create a game | A concise default path; searchable subviews for version/Loader; conditional advanced fields | A catalog with hundreds of options is one choice, not hundreds of form fields. |
| Import several local games | Candidate list, explicit eligibility/selection, then per-item outcomes and scoped retry | Preserve successful copies and original sources after partial failure. |
| Import a pack | Inspection/review with unresolved decisions, then execution/recovery | Blocked is actionable review, not merely a disabled progress page. |
| Follow preparation/download | Game/task summary, current phase and persistent recovery actions; resource details secondary | Observed transfer counts are not necessarily the total work. |
| Understand a crash | Recovery overview plus directly reachable logs/details | More technical evidence must not displace the next useful player action. |

The existing [crash-help design](crash-help-design.md#L71-L137) already applies
overview/detail separation. Reuse that work rather than treating every existing
surface as wrong. Its source-selector decision is also a useful example of
declining a permanent pane when the actual content does not justify it.

## Evidence needed before choosing a redesign

Use the same tasks and data across the candidate compositions:

1. Change language/appearance, inspect the preview, cancel, reopen, then save and
   restart. The user can distinguish preview, draft and durable value.
2. Set new-game memory, then edit one existing game's memory. The affected scope
   and effective value are understandable without learning the catalog model.
3. Select Manual proxy, enter an invalid value, correct it, test, edit again and
   save. Input survives recovery; an old test result is not mistaken for the
   current draft or a saved configuration.
4. Leave a category with unsaved edits and return. The draft, error destination
   and eventual save scope are predictable, including errors in another category.
5. Inspect game B while Home uses A, then encounter an event from A. Inspection,
   selection and session context do not silently change one another.
6. Import multiple candidates with one failure, leave/reopen the result and
   retry the failed work. Successful items and source installations remain intact.
7. Enter from Home, change a setting, and return using keyboard only, including
   Escape/Back and text input with IME. Navigation does not lose focus or drafts.

First use content-complete sketches to reject poor structures; then exercise
the selected interaction in Block's native Linux ARM64 UI, in both themes and
with long localized content and constrained available space. Record unnecessary
navigation, misunderstood scope, lost context and unreachable controls rather
than inventing a numerical design score. Reject a narrower layout if important
context is repeatedly hidden or technical editing becomes impractical; reject
wide layouts that consume space without helping these tasks.

## Limitations

This round reads official documentation/examples and current Block source. It
does not establish visual quality, measured task efficiency or native behavior
of a proposed layout. No new Block runtime, build, test or platform acceptance
was executed. Exact dimensions, save policy, navigation depth and motion remain
design decisions to resolve with content-complete alternatives and native
evidence; existing acceptance records are not evidence for these alternatives.
