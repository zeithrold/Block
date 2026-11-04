# Settings layout comparison

## Selected direction

**The selected design is A: a category index with single-content drill-in
pages. Home and Settings must have the same window dimensions.** The next
study uses Home's current 360 × 720 logical visible-content budget, rather than
the 620 px comparison fixture below. See the
[single-pane navigation research](settings-single-pane-navigation.md) for the
navigation, draft and narrow-content proposal.

The paired boards remain a comparison of the two structures, not a validated
360 px layout. The selected design does not change the running
[management frame](home-management-frame.md#dimensions-and-rationale) until
implemented.

The comparison holds actual settings content and draft behavior constant.
It does not assume a new persistence model to make either skeleton look better.
The current implementation [saves the preferences draft as a whole](../crates/block-ui/src/settings/settings.rs#L856-L896)
and [routes field errors to their owning category and input](../crates/block-ui/src/settings/settings.rs#L899-L928).
For those semantics, B keeps editing destinations visible; A must communicate
scope without a permanent sidebar. These are reasoned tradeoffs, not measured
usability results.

## Paired boards

- [Navigation, General, Profile mode and Storage](design/settings-layout/01-navigation-and-context.svg):
  category entry, editing, one-way conversion, read-only paths, English text,
  and a clean draft.
- [Downloads and Game defaults](design/settings-layout/02-downloads-and-game-defaults.svg):
  proxy validation, an illustrative connection response, conditional fields,
  vertical scrolling, pending work and failure.

Open the SVGs at full size to inspect each paired row. Every left panel is A;
every right panel is B. Values, filesystem paths and connection responses are
examples, not observations of a running Block installation.

| Geometry in the study | A | B |
| --- | --- | --- |
| Window | 620 × 720 | 864 × 720 |
| Content column | 560 px | 560 px |
| Persistent category region | None | 208 px |
| Switching to another category | Back to index, then select | Select directly |

B uses **244 px more window width** in these sketches. In exchange, all five
categories remain visible and directly reachable. It does not provide more
space for fields, better text wrapping, or less vertical scrolling. These
dimensions are comparison fixtures, not final native window specifications.

### Where A wins

A devotes the window to one category and fits a smaller desktop footprint.
Once inside General, it is visually quieter: the user sees only the current
task and its actions. The category index also gives each destination room for
a description, which can help first-time discovery.

Its cost appears when changing categories. Back is navigation, not cancellation:
the draft must survive it. Returning to the index and choosing another category
adds an intermediate screen. A global Save can also take the user directly to
a different category's invalid field. The implementation already routes that
error; the prototype must additionally make the destination understandable
without suggesting the intervening edits were lost.

### Where B wins

B exposes the current category and the remaining destinations throughout the
editing session. Moving between Downloads and Game defaults requires no return
through an index. An error-driven category change has a visible counterpart in
the selected navigation item.

The sidebar is not permission to fill empty space with explanatory panels or
stretch controls. A sparse General page should remain sparse. The study's
neutral surfaces and stable alignment make the hierarchy inspectable, but are
not a finished visual identity.

This is a useful navigation advantage, but the selected direction prioritizes
the smaller shared Home/Settings footprint. A must solve orientation and draft
visibility within that budget rather than restoring B's persistent sidebar.

## Interaction rules shared by both

1. **Category navigation preserves the draft.** In A, Back returns to categories;
   in B, selection changes the content. Neither action means Cancel or Save.
   Focus should land predictably, and returning should preserve useful browsing
   context rather than replay an entrance for every field.
2. **Save and reset have different scopes.** Save commits the preference draft
   across categories; reset concerns the current category. The final wording
   must make that difference clear. A footer's global dirty state must not
   imply that only the visible fields have changed.
3. **A field error keeps the user's input.** Reveal conditional controls when
   needed, select the owning category, bring the field into view and focus it.
   Explain the error beside the field. The paired proxy-error state represents
   saving from another category; it is not a separate error dialog.
4. **Connection testing is not saving.** The draft can be tested without
   committing it. Pending and result feedback belong beside that action.
   A successful response from one endpoint must not imply that all downloads
   or authentication will work.
5. **Mode conversion is a separate consequential action.** It needs its own
   confirmation and outcome. Preference Cancel must not imply rollback of an
   already completed one-way conversion.
6. **Storage paths are read-only.** The boards give Storage a Close action
   instead of implying that displayed paths are editable settings. Before
   implementation, resolve how closing from Storage handles a dirty draft from
   another category; a static Close button does not establish preservation,
   discard confirmation or saved state.

These rules matter more than choosing between a left arrow and a sidebar.
Neither skeleton fixes ambiguous action scope by itself.

## Visual and motion direction

The broader [visual, flow and motion research](ui-visual-flow-motion-research.md)
separates single-page hierarchy, continuous tasks and transitions. Applied here:

- Use spacing, heading weight and alignment to group related controls; avoid
  wrapping every setting in an equally prominent card. Keep destructive or
  one-way actions distinct from ordinary preference fields.
- Reserve space for local feedback where practical. A pending connection test
  should not replace the page or move the Save action. Avoid repeating the same
  save/preview explanation beside every field and again in a large callout;
  the boards deliberately expose this copy-refinement opportunity.
- In A, index-to-category and Back can communicate a reversible drill-in
  relationship. In B, categories are peers: keep the navigation and footer
  stable and compare a short content crossfade against an immediate change.
  Do not animate peer switches as if each were another level down.
- Reveal custom Java, window and proxy fields locally near their controlling
  choice. Preserve the changed control and the next reachable action.
- Do not resize the native window on every category switch. The paired
  alternatives are different shell strategies, not two states to animate
  between during ordinary editing.

No easing curve or duration is selected by these static boards. Comfort must
be judged at normal speed through repeated changes, reversals, typing, errors
and reduced motion, not inferred from tidy endpoints.

## Evidence and limitations

The comparison is grounded in current Settings source and the official patterns
documented in [task composition research](ui-task-composition-research.md).
Both SVGs were parsed and their internal references checked. Representative
General/Profile mode and proxy-error/result pairs were rendered with macOS
Quick Look and visually inspected together.

The boards are static light-theme studies. English path lines are manually
broken; they do not demonstrate native wrapping, text scaling or responsive
behavior. They do not establish keyboard operation, screen-reader semantics,
actual scrolling, runtime focus or motion comfort.

No production code, build or behavioral test suite was changed or run for this
comparison. Native Linux ARM64 verification remains necessary for an implemented
candidate, including Chinese/English, both themes, long content, dirty drafts,
cross-category errors, keyboard/IME and interrupted/reduced motion.
