# Visual composition, continuous tasks and motion

## Recommendation

Develop Block as a calm player-facing tool: the game or account being acted on
stays recognizable, the next useful action is easy to find, and technical
detail is available without competing with that action. Treat visual quality,
task continuity and movement as one experience, not three independent polish
passes.

The earlier [task-composition research](ui-task-composition-research.md) concerns
page structure. This investigation addresses what happens inside and between
those structures. Settings skeleton comparison is a separate design assignment.
These are proposals, not changes to product behavior or acceptance.

The inspected production baseline is
`92b70c6212a5b95012f4429453498d58d4442c28`. Sources below are selected for
specific relevant strengths; no universal “best design” or award ranking is
claimed.

## Reference evidence

| Reference | Evidence inspected in this round | Useful contribution |
| --- | --- | --- |
| [Linear: A calmer interface for a product in motion](https://linear.app/now/behind-the-latest-design-refresh) | Official article and its actual sidebar and border/composition comparison images. | Control attention through contrast, alignment, spacing and consistent action placement rather than adding containers. |
| [Raycast: A fresh look and feel](https://www.raycast.com/blog/a-fresh-look-and-feel) | Official article and its action-panel screenshot. This is the historical 2022 design, not a capture of current Raycast. | A stable action area connects selected content, default action, secondary actions and keyboard hints. |
| [Raycast: Technical deep dive](https://www.raycast.com/blog/a-technical-deep-dive-into-the-new-raycast) | Official text describing native-feel work, expansion rendering and flicker prevention. | Rendering readiness and first presentation affect perceived quality as much as the intended easing curve. |
| [Fluent 2 Motion](https://fluent2.microsoft.design/motion) | Official text and four official videos sampled into frame sequences: top-level transition, container transform, choreography Do and Don't. | Different relationships require different movement; choreography can guide attention or create unnecessary movement. |
| [Emil Kowalski: Good vs Great Animations](https://emilkowal.ski/ui/good-vs-great-animations) | Article text and example descriptions; interactive examples were not operated. | Origin-aware overlays, appropriate easing and synchronizing selection geometry with text treatment. This is practitioner guidance, not a platform specification. |

Third-party images/videos were inspected in per-turn scratch storage, not
vendored into Block. Sampling used `ffmpeg` at four frames per second. This
reveals ordering and gross spatial relationships, not frame pacing, exact
transition duration or subjective comfort during playback.

## 1. Make a whole page visually coherent

### What is visible in the references

Linear's sidebar comparison reduces inactive emphasis while keeping the
selected item identifiable. Its structure comparison replaces some repeated
pill/button boundaries with quieter text-and-icon controls and shared edges.
The change is not simply “fewer borders”: the information hierarchy remains,
but fewer small elements demand attention individually.

Raycast's pictured action panel names the selected object, groups actions, and
aligns shortcuts at a consistent trailing edge. The bottom bar also exposes
the default action. Its translucent background is not what makes this
relationship understandable; the selected row, panel heading and action
placement do.

These images show composition. They do not establish contrast compliance,
keyboard behavior or usability of the pictured applications.

### Direction for Block

Use **one dominant task region**, not one dominant box around every concept.
A page should read in this order:

1. What am I working on?
2. What is its current state or the decision I need to make?
3. What can I do next?
4. What supporting information explains that decision?

For game creation, the chosen game identity and version/Loader matter more
than a repeated explanation of the catalog. For account authorization, the
provider and next browser action matter more than a decorative step rail.
For a failure, the explanation and recovery action matter more than a large
red surface.

Specific visual proposals:

- Align the page identity, section content and action area to a small number
  of shared edges. Avoid independently centered islands with unrelated widths.
- Group related settings and summary properties with spacing first. Use a
  surface or boundary when it distinguishes a real choice, state or workspace.
- Keep explanatory prose close to the control or result it explains. Move
  repeated technical background into reachable supporting detail.
- Make hover, keyboard focus, current selection and disabled state visibly
  different. Lower navigation emphasis without making necessary text illegible.
- Keep status colors local to status and recovery. Version/Loader identity,
  current inspection and active execution must not rely on color alone.
- Preserve meaningful game/account imagery as identity. Do not borrow a
  marketing page's large illustration to fill a sparse utility page.
- Reserve empty space deliberately: between groups to establish rhythm, not
  between a label and the value or action it describes.

### Typography and control budget

Start from Block's existing [visual roles](../crates/block-ui/src/settings/design.rs#L13-L39):
14 px body with 20 px line height, 12 px supporting text, 32 px ordinary
controls and its current heading hierarchy. The proposal does not introduce
a second font system or make every heading larger.

| Role | Use in the candidate |
| --- | --- |
| Page identity | One page-title role, with concise object context nearby. |
| Section heading | Establish semantic grouping; do not repeat page-title emphasis. |
| Body and editable values | Ordinary readable body/control roles, including Chinese text. |
| Supporting metadata | Existing small role only for genuinely secondary information. |
| Primary action | One visually leading action for the current state, with a concrete consequence. |
| Technical text/path | Bounded, selectable or inspectable detail without shrinking the whole page. |

Spacing, font fallback and real text shape need native visual inspection.
Linear's small navigation labels are not permission to shrink Block's form
content, and a quiet dark screenshot is not proof that the light palette works.

## 2. Make a sequence understandable

Continuity has three parts: recognizable object, predictable action consequence,
and a recoverable path back. A pretty screen fails if the next screen changes
what “selected,” “saved” or “complete” means without explaining it.

### Create a game, then prepare it

Proposed experience:

```text
Choose the game to create
  -> Choose version/Loader only where required
  -> Review actual choices and the consequence of Create
  -> Create and show the same named game preparing
  -> Ready to play OR a persistent preparation problem with Retry
```

Keep the game name and chosen target recognizable across review, preparation
and failure. A changed technical choice updates its summary; returning from
the selector restores the draft rather than creating another game.

The last editing action must explain whether it only saves or also starts
preparation. Block already has distinct first-run and ordinary creation
contracts; the composition must expose those consequences rather than merge
them because a template has one Finish button.

An execution failure belongs to the saved game when creation already succeeded.
It must not send the player back through creation and risk duplicate records.

### Sign in, then return to the task that needed an account

Proposed experience:

```text
Original task needs an account
  -> Provider and authorization destination
  -> Open browser / use device code
  -> Explain what Block is waiting for
  -> Verify identity
  -> Save and use
  -> Return to the original task with its draft intact
```

Keep provider identity and the current authorization step visible. Waiting is
system work, not a demand that the user click Next repeatedly. Saving and
verification are distinct; don't celebrate durable completion before save.

The current [authentication footer](../crates/block-ui/src/accounts/authentication.rs#L2654-L2729)
already distinguishes browser opening, waiting, retry, save-and-use, saving
and done. Retain that truthful action model. The [router](../crates/block-ui/src/navigation/navigation.rs#L82-L103)
also retains the task's return destination. These are existing strengths,
not assumptions that every flow needs replacing.

### Recover from preparation failure

Proposed experience:

```text
Named game preparing
  -> Specific failed phase and an understandable explanation
  -> Retry the supported work OR inspect details
  -> Same task resumes, with visible progress
  -> Ready to play
```

Keep the failure in the task view until acted on; a transient Toast alone
cannot carry the recovery path. Retry must name or clearly imply the same
target. Closing details is not cancelling preparation. Stopping a game is
not the same operation as dismissing an error.

The existing [crash-help overview and log-reader separation](crash-help-design.md#L71-L137)
is a useful model: the player can see a supported next action while technical
evidence remains directly accessible.

## 3. Make movement comfortable and meaningful

### Lessons supported by the sources

Fluent recommends a quick fade for large top-level destination changes to
avoid implying an unintended hierarchy or disorienting the user. Its sampled
top-level video keeps the outer frame steady while replacing content. The
container-transform example instead changes a visible region's geometry.
Those are different relationships, not interchangeable effects.

The sampled choreography pair contrasts staggered and simultaneous movement.
Staggering changes where attention goes and how a group resolves, but it also
adds a schedule that can delay availability. The source explicitly asks
designers to consider total duration and parent motion. It is not evidence
that every list should enter one row at a time.

Kowalski explains why a popover should emerge from its trigger and why selection
highlight movement and text-color change must agree. Raycast's engineering
account explains why an intended smooth expansion can still feel poor when
newly exposed content is blank or stale.

### Proposed motion language for Block

| Relationship | Candidate behavior | What should stay stable |
| --- | --- | --- |
| Hover/press/focus | Immediate state feedback; restrained local visual response. | Hit target, label and reading position. |
| Change inspected game/account | Immediate selection feedback; transition only the detail that changes. | List, search, scroll and unrelated actions. |
| Move between peer destinations | Compare a short crossfade with the existing bounded motion. | Chrome and orientation; no false forward/back hierarchy. |
| Enter a genuine subtask and return | A local directional relationship or connected change, reversible on Back. | Object identity and return context. |
| Open a selector/menu | Enter from the trigger's actual location or edge. | The relationship to the initiating control. |
| Reveal conditional fields | Local expansion near the controlling choice. | The control the user just changed and a reachable next action. |
| Change preparing/failed/ready state | Update the task region in place; emphasize explanation and next action. | Game identity; no replay of whole-page entrance. |
| Change native window geometry | Use only if the selected task structure benefits from it. | Readable text, real bounds continuity and usable controls. |

These are candidates, not authorization to replace existing animation behavior.
The current [page/detail/wizard reconciliation](../crates/block-ui/src/navigation/transitions.rs#L149-L246)
already distinguishes motion scopes, and [disclosure](../crates/block-ui/src/navigation/motion.rs#L65-L129)
supports reversal from its current pose. Preserve those useful guarantees
while evaluating the visual language.

The strongest motion hypothesis is **less competition between moving regions**,
not merely shorter durations. Begin with the region the user acted on; move
unrelated regions only when the task relationship requires it. Avoid animating
every field into place before the player can read or edit it.

### Comfort must be judged across repeated actions

A single successful entrance is insufficient. Compare:

- normal-speed first use and repeated quick switching;
- opening then immediately closing, or changing destination mid-transition;
- typing/IME and keyboard traversal while content changes;
- long text, errors and asynchronous results arriving during movement;
- reduced motion with the same state, actions and focus destinations reachable.

First watch the full-speed sequence without instrumentation obscuring it.
Then use slower playback, presented frames and timing to explain visible
problems. Sampled screenshots can reveal a wrong relationship, but cannot
establish smoothness or subjective comfort.

## Next comparison and acceptance

The next useful artifact is a content-complete storyboard for each of the
three sequences above: initial state, action, intermediate state, result,
failure and return. It should name what persists and what moves in each pair
of screens. The [settings comparison](settings-layout-comparison.md) evaluates
two skeletons; its geometry should not predetermine the rest of the product.

For an implemented candidate, review the actual native Linux ARM64 UI in both
themes, Chinese and English, with keyboard/IME, constrained space, interrupted
motion and reduced motion. Use the same task data before/after. Confirm
understanding through the player's prediction of the next action and what
survives leaving, rather than inferring clarity from visual neatness.

Accept a visual treatment when the task has a clear focal point, readable real
content and consistent action placement. Accept a flow when the user can
predict its consequences and recover without losing context. Accept motion
when it clarifies those changes without obstructing repeated use. These
judgments remain separate from builds and automated regression results.

## Limitations

No production UI was modified, and no Block native runtime or automated
behavioral suite ran in this round. Reference images were actually viewed;
four official motion videos were decoded and inspected as frame sequences,
not played and judged at real-time speed. No current Linear/Raycast account
or authenticated application was operated. Source-traced Block behavior and
design hypotheses are not new native reproductions or usability findings.
