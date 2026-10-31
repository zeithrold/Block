# Minecraft crash help design

## Outcome and status

Use one window with a recovery overview, locally expandable diagnosis evidence,
and a directly accessible log reader. New players should find a useful next step
without reading logs; experienced players should reach diagnostic text without
expanding a sequence of explanations.

The recovery overview and local reader described below are implemented in the
desktop crash-help view. The design research used revision
`253adb2a4e605636137cdfdaf8ee7eb3326432a2` with a clean working tree.
Reference observations are not native acceptance evidence for Block.

## Reference research

The references inform composition and interaction, not a replacement component
stack or global theme. Block retains its native UI foundation.

| Reference | Observed evidence | Applicable design |
| --- | --- | --- |
| [IBM Carbon notifications](https://carbondesignsystem.com/components/notification/usage/) | Official anatomy, actionable notification and high/low-contrast images were viewed. | Separate title, explanation and action through alignment, weight and spacing. Use status color locally rather than across the entire page. |
| [PatternFly log viewer](https://www.patternfly.org/extensions/log-viewer/react-demos/with-complex-toolbar/) | The demo was rendered and viewed in Chrome at 1280×960; an official search-state image was also viewed. | Put source selection, find and text operations above one continuous reading area. Preserve context around search matches. |
| [JetBrains error handler](https://plugins.jetbrains.com/docs/marketplace/error-handler.html) | The official IDE Internal Errors screenshot was viewed. | Separate attachment selection from the current attachment body. Borrow this for details, not the beginner-facing overview. |
| [KDE DrKonqi](https://kde.haraldsitter.eu/posts/plasma-crash-course-drkonqi/) | Report-window and notification screenshots in the developer's 2024 article were viewed. | Explain the event before technical content, while keeping the technical entry visible. Block prioritizes player recovery rather than report submission. |
| [VS Code 1.97 Output panel](https://code.visualstudio.com/updates/v1_97#output-panel-filtering) | The official filtering screenshot was viewed. | Give source, filtering and text distinct places. Structured severity filters require structured data and cannot be inferred from Block's current text alone. |

Supporting documentation was also read:

- [Radix Themes Callout](https://www.radix-ui.com/themes/docs/components/callout):
  soft, surface and outline treatments provide emphasis without full-page alarm color.
- [shadcn Alert](https://ui.shadcn.com/docs/components/base/alert) and
  [Collapsible](https://ui.shadcn.com/docs/components/base/collapsible):
  compose explanation and related action, revealing supporting content on demand.
- [Fluent 2 Message bar](https://fluent2.microsoft.design/components/web/react/core/messagebar/usage):
  use specific titles, avoid repeating them in the body, and provide an actionable
  next step; let long messages reflow.
- [Adobe Spectrum Alert Dialog](https://opensource.adobe.com/spectrum-web-components/components/alert-dialog/):
  distinguish a primary action from secondary and dismissal actions.
- [Raycast's Action Bar design](https://www.raycast.com/blog/launch-week-summary)
  and [Action Panel](https://developers.raycast.com/api-reference/user-interface/action-panel):
  organize contextual actions and provide direct expert paths.
- [Nova Issues and Warnings](https://help.nova.app/editor/issues-and-warnings/):
  connect a problem summary to supporting detail. This source was read as text,
  not visually verified.

Carbon is the strongest reference for the overview, PatternFly for log reading,
and JetBrains for attachment organization. These are relevant examples, not a
ranking of the best desktop applications.

## Research baseline and constraints

At the research revision, the crash view presented evidence before advice,
mixed all actions in one row, appended every attachment, and repeated the exit
event in its header. These observations motivated the implementation below.
- [Current diagnosis rules](../crates/block-launcher/src/crash.rs#L322-L365)
  produce `Possible` or `Unknown`, not `High`. Their order is rule-definition
  order, not a confidence ranking. Do not promote the first result to
  “most likely cause.”
- [Report data](../crates/block-launcher/src/crash.rs#L137-L170) already separates
  advice, evidence, exit facts and attachments. Evidence strings do not carry
  reliable source-file and original-line locations.
- Collected text is sanitized and bounded. Call the reader “Logs and details,”
  not “Complete original logs”; attachment truncation is explicit, while recent
  output does not guarantee completeness.

Preserve the [session supervision contract](session-supervisor.md): returning to
the instance does not restart Minecraft, and closing help does not terminate an
accepted game. Keep facts about termination distinct from inferred causes.

## Implemented composition

### Recovery overview

```text
[Status icon] Minecraft closed unexpectedly
Instance name · Minecraft version · Loader

[Overview]  [Logs and details]
----------------------------------------------------

Possible cause                         Inferred from logs
Diagnosis title

Suggested action
Specific advice supplied by the diagnosis.

[View diagnosis evidence · N items]

----------------------------------------------------
Diagnostics stay on this device. Review before sharing.

[Copy summary] [Export diagnostics…]   [Close] [Return to instance]
```

- Lead with the exit fact, then the cause and complete suggestion.
- Expand evidence beneath its diagnosis, preserving its reading context.
- Present multiple possible causes at equal hierarchy, separated by spacing or
  light dividers. Do not invent probabilities or claim a confirmed root cause.
- Keep the log entry visible from the initial view. Progressive information
  disclosure must not force a three-step route to expert tools.
- Group support-material actions separately from dismissal and navigation.
  “Return to instance” is the sole primary button and must remain clear about
  not restarting the game.
- Keep the footer reachable while the body scrolls. Move “Open report folder”
  into the log view, near the content it concerns.

### Logs and details

```text
[Overview]  [Logs and details]
----------------------------------------------------
Exit code / signal / Windows status, when available
Session ID

Source: [Recent game output v]     [Open report folder]
[Find in current text………] [Previous] [Next] [Wrap]

+--------------------------------------------------+
| Current source's sanitized text                   |
| One continuous, selectable reading area           |
+--------------------------------------------------+

Attachment truncation notice, when applicable
----------------------------------------------------
The same reachable footer
```

Use a source selector rather than a permanent file tree: recent output and the
bounded attachment set do not justify a navigation pane at narrow widths.
Initially select recent output and list attachments by their existing names.

Prioritize finding text with match navigation over “errors only” filtering.
Minecraft and JVM output often explains an exception in surrounding lines.
Preserve indentation, allow selection and copying, and offer wrapping without
changing the collected text. Per-source reading position and search state may
remain local to the open report.

## Visual language and window behavior

Inherit Block's [typography and control roles](../crates/block-ui/src/settings/design.rs#L13-L39),
including its current Inter/CJK font strategy. Use the existing 14/20 body role
for explanation and advice, and the 12 auxiliary role for metadata. Establish
hierarchy through order, weight and spacing rather than enlarging every heading.

Use one main content region with restrained separators. Avoid nested cause and
log cards, oversized warning art, or a centered marketing-style error page.
Use existing Lucide icons with labels. Reserve error color for the abnormal
state and actual operation failures, not every possible diagnosis.

Both the standalone help window and embedded entry resolve their appearance
through the existing [preferences and semantic palette](../crates/block-ui/src/settings/appearance.rs).
The standalone entry uses the preferences already loaded for language startup.

Do not assume the standalone window's initial dimensions apply to the embedded
entry. At narrow widths, let toolbar controls and footer action groups reflow
rather than shrinking text or automatically enlarging the window. Narrow log
toolbars use the existing Lucide icons with localized names and tooltips; the
reader's viewport becomes shorter in a short window while text remains at the
normal body size. Expansion preserves reading position and keeps keyboard
focus meaningful.

## Failure and operation states

| State | Presentation and recovery |
| --- | --- |
| Possible cause found | Show the title and suggestion first, with expandable evidence. Preserve uncertainty. |
| Unknown cause | State that available information is insufficient. Offer logs, copy and export rather than an empty evidence box or generic checklist of unrelated causes. |
| No game output | Say “No game output was captured.” Retain available exit facts and attachments; do not imply all diagnostics are missing. |
| Start failed | Say Minecraft could not start, not that it crashed during play. Show only exit fields that exist. |
| Monitoring interrupted | Explain that Minecraft may still be running. Do not imply exit or safe restart. Current notification rules do not create crash help for this outcome; this design does not propose changing them. |
| Waiting for export destination | Show the waiting state and existing cancellation path near export. Do not invent progress. |
| Writing export | Prevent duplicate submission and show the writing state. Preserve the existing boundary that cancellation is offered only while waiting for the destination. |
| Export failed | Keep a visible local message near export, suggesting another folder or copying the summary. Preserve the report, selected source and reading position. |
| Export canceled | Use neutral feedback, not error styling. |
| Export saved | Confirm locally without replacing the diagnostic view; retain the reminder to review before sharing. |

Do not promise that worlds or saves are intact without evidence.

## Implementation boundaries

| Scope | Required work |
| --- | --- |
| Existing-data composition | Reorder advice before evidence, remove duplicate exit wording, split body and footer, and group existing actions. No report-model change is needed. |
| Local reading interactions | Add overview/log selection, per-diagnosis evidence disclosure, source selection, text selection/copy, find and wrapping. These are behavior changes, not just styling. |
| Keyboard and focus | Native controls and focus traversal replace the former fixed action-order model. The selector and inputs retain their own key handling; search typing does not activate a window action. |
| Entry integration | Cover both the [standalone window](../crates/block-app/src/main.rs#L249-L283) and embedded help, reusing existing theme and lifecycle ownership. |
| Separate capability decisions | Original-file/line navigation, structured severity filtering, diagnosis ranking, additional runtime metadata, direct settings navigation and automatic repair require further contracts or data. |

The implementation uses the collected report only. Each source retains its own
read-only text state, query, match position and line-wrap choice while the view
is open. Automatic repair and ranking are not part of this package.

One adjacent follow-up: standalone “Return to instance” failures currently
[reach logging and stderr](../crates/block-app/src/main.rs#L269-L274), not visible
help feedback. Surfacing the result requires callback integration, not a label
change.

## Acceptance and evidence limits

No Block build, tests or native runtime verification was performed during the
research. External screenshots and the PatternFly rendering establish reference
observations, not Block interaction correctness. KDE images represent the linked
2024 article; the VS Code image represents version 1.97.

The implementation's GPUI test harness was exercised on macOS ARM64 with
synthetic, already-sanitized reports. It covers keyboard source and search
operations, Unicode selection and copy, per-source state, export cancellation
and failure, and footer bounds at 360×540, 400×540 and 1152×720. This is
automated layout and interaction evidence, not a native desktop visual review
or evidence of Minecraft gameplay or Microsoft authentication.

The final implementation passed `just check` on macOS ARM64 and
`scripts/verify-platform.py --platform linux-arm64` on Debian 13 ARM64, including
470 UI tests and three framework tests. Strict workspace Clippy also passed
with `tracing/log` enabled.

Native verification on Debian 13 ARM64, KDE Wayland with an Xwayland window,
used independent synthetic reports, not a real Minecraft failure. It covered
English, Simplified Chinese and Japanese, light/dark preferences, long instance
names, multiple/unknown diagnoses, start failure, monitoring interruption,
missing output and truncated attachments. At 360×540, forward and reverse
keyboard traversal revealed the second diagnosis's evidence control; Space
expanded it while the footer remained reachable. One physical Enter advanced
one of three search matches, and Shift-Enter moved back once.

Native reader checks covered exact copying of sanitized Unicode text,
indentation and tabs, read-only input, source switching, wrapping and retained
search/reading position. Canceling the native Save File dialog preserved the
selected source, query and reading position. Screenshots and logs are retained
under the task-owned `crash-redesign-native` evidence directory.

The following remain outside that native evidence: redesigned inline entry,
text enlargement, Windows/macOS desktop rendering, export failure and accepted
write completion timing. Inline entry, export failures and lifecycle ownership
have automated coverage and source review; an accepted write was not paused
deterministically while closing its view.

Further native coverage follows the [test-platform guidelines](test-platforms.md)
and [justfile](../justfile):

- Standalone and embedded entries, light/dark themes, narrow/short windows,
  long instance names and text enlargement.
- Multiple possible causes, unknown cause, missing output and truncated
  attachments.
- Keyboard access to overview, evidence, sources, find and footer actions;
  visible focus and sensible focus retention after switching or expanding.
- Export cancellation, writing, failure and completion without losing the
  diagnostic reading context.

The usability target is concrete: a new player can identify a next step without
reading logs, and an expert can begin inspecting a selected source without
scrolling through the entire report.
