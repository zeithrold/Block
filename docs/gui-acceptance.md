# GUI acceptance walkthrough

## Scope and recording

Base commit: `985238e` (`Add local diagnostics and repair GUI interaction flows`).
Record any subsequent task patches and the actual executable before execution;
the [Windows verification report](windows-amd64-gui-verification.md#native-checks-and-builds)
distinguishes the logging, shared-control and wizard fixes from that base.
All human acceptance items below start **Pending**. Earlier automated checks,
source reviews and native observations are preparation evidence, not human
approval. The [coverage audit](gui-coverage-audit.md#repair-disposition-and-verification)
records their scope and limitations.

Start with Linux ARM64. Record OS, CPU, display backend (Xwayland or native
Wayland), build, theme and scale for each session. A Wayland desktop running
Block through Xwayland is Xwayland evidence. Record other platforms separately;
do not inherit a pass across backends or platforms.

For each numbered item, record:

`ID | Passed / Failed / Not run / Blocked / Deferred | environment | observation | evidence`

An unprepared test fixture or script is Not run, with a preparation owner and
next step; it is not a request for the user to do that preparation. Blocked
requires a specific unavailable permission, service, platform capability or
safe supported entry, and should say whether user action is actually needed.
An intentional postponement is Deferred. A failure should include the control,
action, expected result and actual result; screenshots help with layout, but do
not prove disk effects or late-response ownership.

Use a fresh isolated data root and synthetic accounts/catalogs/reports for
recovery and destructive paths. Do not import, delete, convert or repair real
player data for acceptance. Service-backed authentication requires the user's
explicit participation; never record passwords, tokens or device codes in
evidence. Stop at a failed step and retain its draft and evidence.

### Session prerequisites and execution order

`--data-dir` alone does not isolate the entire workflow. Before starting, record
the actual source-installation discovery locations, game library/cache,
launcher-log directory and native credential environment. Import discovery can
read installations beside the executable and under the OS user directory;
automatic setup storage follows installation shape. Logging and secure
credentials use their own OS locations. Do not export existing private logs or
touch existing identities merely because the catalog is isolated.

Record the executable and entry method for each session:

- `--scenario` is a presentation fixture without the normal Engine/message
  workflow; it cannot establish persistence, copying, authentication or sessions.
- Normal-engine journeys need a task-owned store and explicit safe game/source
  paths. Check resolved paths before starting installation, import or repair.
- Mixed import outcomes, held save requests, storage failures, session
  interruption and crash/modal delivery need a named controllable fixture or
  driver. Record its start, fault, release and observation steps. If no such
  entry has been prepared, record Not run and assign the preparation. If a safe
  entry cannot be provided through supported boundaries, document that specific
  blocker; do not patch production source or substitute a screenshot.
- Production windows request fixed sizing. Record whether compact/growth checks
  used a supported fixture, test API or OS window-manager operation. An OS that
  forbids the resize leaves that manipulation Blocked, not a failed text field.

Execute stateful cases by dependency, not mechanically by number. Use separate
roots for fresh setup and restart. Test Bundled deletion and conversion Cancel
before confirming the one-way conversion; only then test Separated Copy. Keep
online identity and game-service checks separate from presentation fixtures.

## Common checks on every surface

Apply these to the actual controls reached in each journey, not just screenshots:

- **Layout:** title, complete identity, status, validation and primary/secondary
  actions are readable; long content scrolls without hiding the footer.
- **Interaction:** pointer and keyboard produce the same permitted action;
  disabled controls do nothing; enabled controls are not inert.
- **Focus:** Tab/Shift+Tab order is usable and visibly focused; Escape dismisses
  only the top layer; dismissal restores focus to a meaningful live control.
- **Recovery:** loading, empty, no search results, validation, service failure,
  retry and success remain distinguishable; recoverable failure retains input.
- **Ownership:** cancel, navigation, view closure and late results do not mutate
  replacement drafts, launch an unauthorized operation or write an abandoned
  export. Already authorized persistence can finish after navigation; record
  its commit boundary. Passwords intentionally cleared after submission are
  not expected to survive a failure.
- **Appearance:** inspect light/dark, reduced motion, long names/paths and the
  supported compact window. Check OS scaling separately; resizing is not a
  substitute for text/font scaling or moving between DPI environments.

## A. Shell and common controls

| ID | Entry and actions | Expected result |
| --- | --- | --- |
| UI-01 | Open the launcher; navigate Home → Games → Accounts → Settings → About and back using the menu and keyboard. Repeat a rapid reversal. | Correct page/title/current marker, stable frame, no stale outgoing controls accepting input, no unexpected draft loss. |
| UI-02 | Drag, minimize, restore and close a disposable window. Open a dialog before trying background chrome. On available hosts, change work area and move between differently scaled monitors. | Controls remain visible; drag regions do not steal button presses; modal ownership is coherent. Missing monitor/backend combinations remain Blocked. |
| UI-03 | In a labeled text field, enter a long ASCII/CJK/emoji value; use selection, Home/End, arrows, copy/cut/paste, undo/redo and IME composition. Resize compact → wide → compact. | Correct value/caret/selection, no retained tail-only scrolling, no broken graphemes or accidental Escape/Enter during composition. Password fields stay masked. |
| UI-04 | Use checkbox, radio, select and searchable list controls with pointer and keyboard. Try empty search, disabled entries, first/last row and a long list. | Focus, active inspection and committed selection remain distinct; scrolling reveals the active row; cancel preserves the old value. |
| UI-05 | Stack a sheet and dialog; press Escape twice. Trigger repeated notifications, hover/focus them, dismiss one and change reduced motion. | Only the top surface closes each time; scrim blocks background activation; focus returns correctly; notifications remain readable and their interaction/timing is coherent. |

## B. First run and import

| ID | Entry and actions | Expected result |
| --- | --- | --- |
| UI-06 | Fresh setup: press a Welcome-frame corner outside the circle, drag outside → inside and inside → outside, then activate the center and keyboard Enter/Space. | Only a complete permitted circle press or keyboard activation starts setup; hover does not enlarge the hit region. Check touch separately if available. |
| UI-07 | Follow Welcome → Overview → account/game tasks, Back and re-entry. Exercise neither task complete, account-only, game-only and both complete; restart unfinished setup; try Skip on another isolated root. | Progress and hidden drafts survive their documented transitions; Continue routes to the missing task; Finish persists completion; Skip does not fabricate an account/game. |
| UI-08 | Import fixture: inspect Sources/List, supported/unsupported/already-imported rows, warnings, select-all/clear and source switching. Cancel before copying. | Eligibility, destination and selection are clear; unsupported rows cannot import; source installations remain untouched. |
| UI-09 | Synthetic multi-game import: observe Progress, Stop mid-copy, mixed Results, Retry remaining and Continue setup. Also close the drawer during a separate batch. | Successful games remain saved exactly once; retry excludes successes; failed/not-imported counts and controls are correct; mutable source files are unchanged. Requires a controlled copy fixture, not a static Sources/List screenshot. |

## C. Accounts and providers

| ID | Entry and actions | Expected result |
| --- | --- | --- |
| UI-10 | Accounts: loading/failure/retry, empty/search-empty, inspect another account, Use account, removal Cancel and confirmed removal using a disposable offline identity, including the last account. | Inspection does not change current identity; confirmation captures the intended identity; removal reconciles current selection and empty state. |
| UI-11 | Add Offline account: invalid name, valid name, Back/edit, review, save; use a controlled save failure and retry. | Validation identifies the field; draft survives failure; only confirmed save persists, once, across restart. |
| UI-12 | Microsoft: Browser and Device Code paths, preparing/waiting, reopen/copy, expiry/failure/retry, method switch and cancel; optionally complete a real sign-in with the user. | Clear progress/recovery, expired data not reused, cancelled/old attempts cannot finish the new one. Fixture layout approval does not count as live authentication success. |
| UI-13 | Authlib: provider choice, long HTTP consent, inline Add-provider dialog, account/password, character list, verification failure, Back/cancel and save failure. | Warning and actions fit compact windows; Escape/focus respects nested ownership; provider changes invalidate sensitive drafts; failed verification preserves the intended character selection. |
| UI-14 | Providers: loading/error/retry, empty/search-empty, recognize/paste URL, duplicate reuse, HTTP consent after address edits, save/discovery error, editing and removal. Native drop is tested in UI-13's inline Add-provider dialog. | Correct provider/address binding; no duplicate creation or forbidden removal; draft and recovery remain available. Check rename/restart and both linked/unlinked records as detailed below. |

## D. Games, creation and editing

| ID | Entry and actions | Expected result |
| --- | --- | --- |
| UI-15 | Bundled browser: search, inspect B while A is current, use B, inspect components and return. Separated browser: switch Instance/Profile tabs, linked rows, independent searches and unavailable records. | Correct identities and current markers; inspection alone never changes Home selection; linked navigation targets the named record. |
| UI-16 | Create game through the real controls: Bundled setup/review; Separated new Instance and existing Instance paths. Back/cancel/re-enter; exercise save failure/retry and first-run save-only. | Draft and step history are preserved; Review names the correct target and consequence; final confirmation persists once; first-run save-only does not silently install. |
| UI-17 | Wizard Version/Loader cards: catalog loading/error/retry, search-empty, snapshots/early builds, exact ID, stable/preview, no compatible builds, retained selection and changing Minecraft version. | Disabled ARM64 versions explain their restriction; stale catalog results do not overwrite current choices; Done validates and Cancel rolls back only that section. |
| UI-18 | Wizard OptiFine and Configuration: choose/cancel/remove JAR; incompatible/unsupported/validation states; folder, Java and custom window fields; short-window section scrolling. Inject chooser failure and retry. | Draft survives cancellation/error; actionable feedback is visible inside the modal; superseded chooser responses are ignored; Done/Cancel remain reachable. |
| UI-19 | Instance editor: generic Name entry and OptiFine-card entry, runtime/window settings, invalid field reveal, cancel, real metadata-save failure/retry; separately exercise committed OptiFine change followed by reinstall failure/cancel. | Entry focuses its owning field; no silent draft loss; metadata-save outcome and subsequent install outcome are not confused. |
| UI-20 | Profile editor: main, Appearance, Launch and Data/account subpages; inheritance/missing defaults, long arguments/server, memory limits, target/account selection, subpage Done/Cancel and final Save/Cancel. | Subpage Done changes only the draft; Cancel restores its checkpoint; final save persists the intended effective settings; errors reveal the right field. |
| UI-21 | Disposable catalogs/world markers: Bundled deletion and conversion Cancel first; confirm one-way Bundled → Separated conversion; then copy/delete Profiles and change Isolated/Shared policy. Reopen from disk. | Distinguish the concrete metadata and directory outcomes below; no moving/merging worlds on policy change. UI confirmation alone is not proof of disk effects. |

## E. Home, downloads and sessions

| ID | Entry and actions | Expected result |
| --- | --- | --- |
| UI-22 | Home: no account/game, missing default account, Ready/Pending/Failed target, Download/Retry/Launch, account/profile search sheets. During install click Queue launch, then Stop in another run. Use compact Home. | Dynamic actions really activate; queue/status agree; Stop clears queued intent; identity, task and action remain reachable. |
| UI-23 | Ready-but-damaged synthetic installation: find explicit Repair on an inspected noncurrent Profile and activate it through a controlled installer. | Repair targets the inspected game without changing Home selection; immutable corrupt copies and mutable world/config markers follow the existing repair contract. Record failed-attempt and completed-repair evidence separately. |
| UI-24 | Downloads sheet and file-detail dialog: unknown totals, retries, cache/verified groups, changing membership, empty detail group, long resources and nested Back/Escape. | Progress is truthful, detail changes do not strand focus, long lists scroll and only the top layer dismisses. |
| UI-25 | Authorized disposable game lifecycle: pre-handoff cancel, accepted preparation/running, explicit Stop/failure, normal exit, abnormal exit, launcher close/reopen and monitoring interruption. | Accepted game survives launcher closure; Stop is explicit; terminal/reconnect states are truthful. Do not clear reservations or act on saved PIDs to force recovery. Menu display alone is not gameplay/normal-exit evidence. |
| UI-26 | With task game A running, inspect/select another Profile and attempt the normal Play action. Separately publish a controlled crash report while Games Help or Downloads/file details is open. | The active-run policy prevents a second Play; Stop still targets the running game rather than the inspected Profile. Crash/modal pointer, keyboard, dismissal and Return ownership remain usable. Record synthetic report delivery separately from a real supervisor crash. |

## F. Crash help and diagnostics

| ID | Entry and actions | Expected result |
| --- | --- | --- |
| UI-27 | Inline and standalone Crash help: failed-start/abnormal/no diagnosis, long sanitized details/attachments, copy, folder, Return and Close. Inspect B before injecting A's crash in both Profile modes. | Readable report/actions, correct sanitized copy, Return shows A and never relaunches it; focus and scrolling work with long content. |
| UI-28 | Crash JSON export with a controlled portal: normal Save, OS cancel, error, indefinitely waiting request → Cancel → retry; deliver late old responses and Return before another response. | Clear waiting/writing/result states; recovery is reachable by keyboard; abandoned choices produce no file and cannot clear a newer export. Accepted writing is not presented as cancellable. |
| UI-29 | About: copy build identity, external links, output/recipe actions, Open logs and ZIP export. Exercise picker cancel/error/stall/retry, existing destination, write failure and unavailable logging storage. | Feedback matches the outcome; existing ZIP is not overwritten; launcher logs and sanitized Crash JSON remain distinct; copied identity is complete; external apps actually open where claimed. |

## G. Settings and cross-cutting acceptance

| ID | Entry and actions | Expected result |
| --- | --- | --- |
| UI-30 | General/Game defaults: theme and reduced-motion preview, section Restore, change multiple sections, hidden invalid field, Save failure/retry, Cancel and navigation away. | Section reset is local; Save handles the whole draft; cancellation restores saved preferences; values and preview survive recoverable failure. |
| UI-31 | Network: limits and invalid boundaries, System/Direct/HTTP(S)/SOCKS, connection test pending/success/error; edit or replace the draft while a response is pending. Storage: effective paths and corrupt-config error. | Limits validate; stale connection responses are ignored; credentials are not accepted in proxy URLs; Storage is read-only and reports real effective paths without exposing retired relocation controls. |
| UI-32 | Keyboard operation through a field, validation, radio/select, dialog, Crash actions and Home controls; test both themes, reduced motion, scale and long content. | Visible focus follows a usable order; inputs and actions remain reachable; Escape dismisses the top layer and restores focus; retained outgoing content cannot accept input; no clipped content or hidden primary actions. |

## Points that must not disappear from final acceptance

### Required substeps from the independent completeness audit

Keep the 32 IDs. Record the following branches individually under their owning
ID, with separate presentation, persistence and service outcomes.

1. **UI-05 — Pointer ownership and recovery.** Drag a Drawer handle a short
   distance and release to rebound; pull down to dismiss; scroll its body
   without dismissing; click the scrim without activating background controls;
   reopen during exit and check focus. For notifications, observe duplicate
   coalescing, hover/focus pause and resumed expiry, short-swipe rebound and
   accepted swipe dismissal. Ordinary and error lifetimes are 4 and 8 seconds
   before pauses; do not judge expiry by an unspecified “coherent” feeling.

2. **UI-07/16 — Creation commit boundaries.** Recommended setup submits
   immediately; do not expect an additional Review confirmation. Custom
   first-run creation saves only, while Finish can start preparation of a
   pending target. Ordinary Create commits catalog metadata before installing.
   Test install failure/Stop after that commit: the saved record remains
   available for recovery and retry must not create another record. This is
   different from a catalog-save failure.

3. **UI-10–13/25 — Credential and save lifecycle.** Distinguish unavailable
   secure storage from network failure, and verify retry after restoring the
   controlled boundary. Before Finish, cancellation must not save an identity;
   after Finish authorizes Saving, ordinary navigation need not roll it back.
   First-run Saving prevents cancellation. A completed save must not dismiss an
   unrelated replacement draft. Authlib deliberately clears the submitted
   password. With a separately authorized online fixture, restart after saving,
   launch using saved-identity refresh, exit and launch again; test missing
   credentials and online deletion success/failure independently of Offline
   deletion. Check the resulting account selection and credential outcome
   through supported interfaces, never by extracting secrets. Without that
   environment, those branches remain Blocked.

4. **UI-13/14 — Provider lifecycle and real drop entry.** In the authentication
   wizard's inline Add-provider dialog, drop a valid quick link, reject
   unsupported content and verify that closing the dialog removes its target.
   The Providers management page is not the drop entry. In management, rename
   an existing Provider and verify after restart; a linked Provider's address
   stays read-only; cancel then confirm deletion of an unlinked Provider.
   Native Wayland's unsupported drop remains a documented limit.

5. **UI-15/21 — Games Help.** Open the `?` dialog, read the current mode and
   scroll, then close and check restored focus. Reopen and follow its Profile
   mode link: reach that Settings section with no remaining Help scrim. Check
   the changed explanation after conversion.

6. **UI-16/20 — Server address versus automatic connection.** Save an address
   with automatic connection off, then with it enabled on a supported version.
   Exercise unknown/unsupported manifest guidance and change Minecraft version
   after choosing the option. Invalid automatic connection must require
   correction without losing the address. Actual automatic joining needs an
   authorized controlled server; address persistence alone is not a successful
   join.

7. **UI-21/25 — Concrete data-policy and lock outcomes.** Bundled deletion
   removes the Profile and associated Instance metadata; Separated Profile
   deletion retains the Instance. Both preserve game files. A copied Isolated
   Profile has a new ID/effective directory and does not copy worlds; a Shared
   copy uses the same Instance directory. Changing policy selects a directory,
   not a move or merge of existing files. Use distinct marker files to identify
   those directories. Separately hold a game-directory reservation and verify
   refusal of a second conflicting writer; marker preservation alone does not
   prove lock ownership.

8. **UI-25/27 — Automatic Crash delivery and acknowledgement.** With launcher
   open, a controlled abnormal/start-failed session should reach inline help;
   with launcher closed, standalone help should appear. Prevent presentation
   in a controlled way, then reopen to recover the unread report; once
   acknowledged it must not repeatedly present. Normal exit/confirmed Stop
   should not produce a false Crash alert. Record Return separately from
   Close/Escape: Return navigates without relaunching; inline Close/Escape can
   remove the launcher window, rather than returning to Games.

9. **UI-30/31 — Saved-setting consumers.** Save game defaults, restart and
   create a new draft: it uses the new defaults while existing games remain
   unchanged. Test connection uses the unsaved network draft; Cancel afterward
   must not persist it. Then save a controlled network setting and observe an
   actual catalog refresh/download using it. Endpoint success proves only that
   endpoint, not all authentication or download traffic.

10. **UI-30/32 — System preferences.** Check Follow system separately from
    manual Light/Dark. With application reduced motion off and OS reduced
    motion on, effective motion remains reduced. Use an authorized reversible
    system-preference change, or leave the branch Blocked. On Linux, record its
    existing refresh/next-interaction behavior rather than demanding an
    unimplemented immediate notification. Restore any approved temporary
    system change afterward.

The static matrix has 72 current scenario stems plus six historical registry
rows. Covering those stems does not independently cover UI-09, UI-12's live
service, UI-19's post-save reinstall failure, UI-21's filesystem effects,
UI-25/26's lifecycle, UI-28's late responses or UI-32's keyboard/focus journeys.
Use controlled workflows for these instead of inventing screenshot coverage.

The user can accept presentation while leaving a service/backend check Blocked,
provided the record names that boundary. A whole-product acceptance statement
must still identify all Failed, Not run, Blocked and Deferred items.
