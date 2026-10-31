# Windows AMD64 GUI development verification

## Result and scope

This is development evidence, not user acceptance. The run has exercised native
Windows rendering and selected real-engine workflows, but it has not completed
the whole-product walkthrough. Statuses below describe the numbered walkthrough
as a whole; a passed substep does not make an incomplete item passed.

The current test candidate is Git commit
`985238e878ff291ed8a69392a618f9cba47a20eb`, plus the independently supplied
logging, shared-control naming, wizard-layer, fresh-import parent, Crash
presentation, wizard-cancel and text-selection fixes. The earlier patches are
retained on the Windows host as `candidate.patch`, `controls.patch`,
`wizard.patch` and `import-parent.patch`. The later incremental patches and
old/new source copies are in `incoming-p3-input\`, including
`crash-modal.patch`, `wizard-cancel.patch`, `text-selection.patch` and the
final test-only `overlay-tests-final.patch`. The current executable is
`D:\block-gui-verify-985238e-20261030T143906Z\bin-modal-wizard-input\block.exe`.
Its `--build-info` reports `0.1.0`, `windows`, `x86_64`, and null source stamp
fields. The commit and patches, rather than an embedded stamp, identify this
manual build. `resume-20261031-0915\final-all-build-info.txt` records the
output. The final test-only adjustment followed the executable build and did
not change production sources; previous binaries and evidence were retained.

The task root used throughout this report is
`D:\block-gui-verify-985238e-20261030T143906Z`. Unless otherwise stated, paths
in the tables are relative to that root. Images are native process-window
captures; they show the indicated state, not all interactions required by an
acceptance item.

### Host and isolation

- Windows 10 Enterprise LTSC, build `19044`, x64; Rust `1.99.1`,
  `x86_64-pc-windows-msvc`; eight logical processors and approximately 8 GB RAM.
- Interactive console session 1, input desktop `Default`, one 1920×1080 display,
  1920×1040 work area and 96 DPI. Mixed-DPI monitors, touch and an actual
  screen-reader session were not available in this run.
- The submitted Git commit was fetched from the official origin into a
  task-owned object store and exported to a non-Git `src` snapshot. The
  unattached `D:\block-src` checkout was not changed. The existing
  `D:\block-target` was used by one builder and was not cleaned.
- Normal-engine tests use task-owned `--data-dir` roots, task-owned
  `LOCALAPPDATA` for logs, and a copied task-owned executable. The Storage page
  showed `data-keyboard-skip\instances` as the game library and
  `data-keyboard-skip` as the download cache.
- Windows Known Folder discovery resolves the real roaming `.minecraft`. After
  the user confirmed that no personal Minecraft installation was present and
  authorized normal discovery, Welcome Start scanned the normal sources. The UI
  reported no games in the official-launcher source; only the task-owned
  executable-adjacent synthetic source was selected for import. No unknown
  source was copied, and Windows Credential Manager was not enumerated.
- `--scenario` uses synthetic, engine-free state. Its captures cannot establish
  persistence, imports, authentication, session ownership or game execution.

## Native checks and builds

`just` was absent on the Windows PATH. The four commands below are the
justfile's equivalent steps, not a claim that `just check` ran. All builds and
checks retained `--locked`; `CARGO_TARGET_DIR=D:\block-target`,
`CARGO_PROFILE_DEV_DEBUG=0` and `RUST_MIN_STACK=67108864` were used. Earlier
checks used `CARGO_BUILD_JOBS=2`; the final UI package checks used one job and a
below-normal-priority parent process while the game remained open.

| Candidate | Command or action | Result and retained evidence |
| --- | --- | --- |
| `985238e` | `cargo build --locked -p block-app --bin block` | **passed**; `build.stdout.log`, `build.stderr.log`; running copy was `bin\block.exe`. |
| `985238e` | `cargo fmt --all -- --check`; `cargo check --locked --workspace --all-targets --all-features` | **passed**; `fmt.*.log`, `check-code.*.log`. |
| `985238e` | `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings` | **failed**: Windows `block-logging/src/lib.rs` nested `if` triggered `clippy::collapsible_if`; `clippy.stderr.log`. |
| `985238e` | `cargo test --locked --workspace --all-features` | **failed**: two `block-logging` tests, `export_is_bounded_complete_owned_and_never_overwrites` and `shutdown_prunes_inactive_files_but_preserves_active_files`; `test.stdout.log`, `test.stderr.log`. The run stopped before later packages. |
| `985238e` diagnostic copy | Each failing logging test, once with temporary diagnostic printing | The actual active segment had directory-entry size 0 while path metadata and the read had the real size. The diagnostic export passed after the additional observations; retention still failed. The unmodified focused tests both failed. `diagnostic-tests.diff`, `diagnostic-*.log`, `focused-*.log`. |
| `985238e` + logging patch | The four equivalent justfile commands, in order | **passed**; `candidate-fmt.*.log`, `candidate-check.*.log`, `candidate-clippy.*.log`, `candidate-test.*.log`. Windows UI tests were 367 unit and 3 framework tests. Both formerly failing focused logging tests and `cargo test --locked -p block-logging` passed separately. |
| Above + controls patch | Two new named-control tests; `cargo clippy --locked -p block-ui --all-targets --all-features -- -D warnings`; `cargo build --locked -p block-app --bin block` | **passed**; `controls-test-*.log`, `controls-clippy.*.log`, `controls-build.*.log`; executable `bin-updated\block.exe`. The earlier full workspace gate applies to the logging-only candidate, not to this later source state. |
| Above + wizard patch | `cargo build --locked -p block-app --bin block` | **passed**; `wizard-build.*.log`; executable `bin-fixed\block.exe`. |
| Above + wizard patch | `cargo test --locked -p block-ui --all-features`; `cargo clippy --locked -p block-ui --all-targets --all-features -- -D warnings` | **passed**, both exit 0; 371 unit tests and 3 framework tests passed, with 0 doc tests. Evidence: `final-ui-test-retry.stdout.log`, `final-ui-clippy-retry.stderr.log`, `final-ui-gates-retry.summary`. This is a package gate, not a new full-workspace run. |
| Above + import-parent patch | `cargo build --locked -p block-app --bin block` | **passed**, one job; `resume-20261031-0915\import-parent-build2.*.log`; executable `bin-import-parent\block.exe`. |
| Above + import-parent patch | `cargo fmt --all -- --check`; `cargo test --locked -p block-ui --all-features import_`; `cargo test --locked -p block-ui --all-features`; `cargo clippy --locked -p block-ui --all-targets --all-features -- -D warnings` | **passed**, respectively 18 focused tests and 378 unit plus 3 framework tests; `resume-20261031-0915\import-parent-gates.summary` and `import-parent-{fmt,import-focused,ui-tests,ui-clippy}.*.log`. This is not a fresh full-workspace gate for this candidate. |
| Above + Crash, wizard-cancel and text-selection patches | `cargo build --locked -p block-app --bin block` | **passed**, one job; `resume-20261031-0915\final-fixes-build-retry.summary` and `final-fixes-build-retry.*.log`; executable `bin-modal-wizard-input\block.exe`. The first SSH build wrapper timed out after Cargo logged completion; a single task-owned retry captured exit 0. |
| Above + final test-only delta | `cargo test --locked -p block-ui --all-features`; `cargo clippy --locked -p block-ui --all-targets --all-features -- -D warnings` | **passed**, both exit 0; 397 unit and 3 framework tests passed, with 0 doc tests. Evidence: `resume-20261031-0915\final-all-ui-gates.summary`, `final-all-ui-{test,clippy}.*.log`. This is not a fresh full-workspace gate. |

The controls candidate's 72 built-in scenario stems were each captured in light
and dark, 144 captures total; the capture helper returned success for all and
no scenario process required forced cleanup. `matrix.log` records individual
results and `matrix\` contains the images. This is a static capture matrix, not
a visual sign-off or evidence for the 32 engine journeys.

## Game lifecycle observations

The real-engine run used Minecraft **1.20.6, Vanilla**, the synthetic offline
identity `BlockDev01` and the task-owned game `BlockWinTest`. It did not use a
Microsoft account. The first install was queued and stopped from Home; Retry
then reached `Repair complete · ready to play` after approximately 192 seconds.
Launch started task-owned managed Java and reached the Minecraft first-run
accessibility prompt. This is not evidence of a playable world.

Closing the launcher normally left the same Java PID `2072` and its supervisor
PID `5800` alive. A new launcher for the same data root showed
`Running · Minecraft PID 2072` and an enabled Stop control. Those processes
were not present at the later observation below; their exit reason is unknown.

A separate, newly created task-owned store subsequently installed and launched
the same version, loader and offline identity as `BlockWinFresh`. Its first run
created `New World`; Save and Quit returned to the Minecraft title, and Quit
produced a public RunRecord with `state=finished` and `outcome=normal`. Relaunch
showed that same world in Singleplayer and reopened it; a second Save and Quit
and Quit also produced `finished`/`normal`. Native screenshots and safe run
summaries are under `resume-20261031-0915\shots\game-*` and
`resume-20261031-0915\game-*-terminal.txt`. Attempts to demonstrate world
interaction did not change the displayed coordinates or open inventory; the
world and normal lifecycle were observed, but playable input is unverified.
In a later launch of this same isolated game on the current candidate, Home
showed the active Java PID and Stop rather than another Launch action. Stop
produced a public `finished`/`stopped` RunRecord, and no Crash help appeared
in the following ten seconds. `resume-20261031-0915\ui25-stop-monitor.txt`,
`ui25-stop-sessions-after.txt` and `shots\ui25-stop-*-native.png` record this
separate run. Game command lines, launch plans and tokens were not recorded as
evidence.

## Walkthrough status

`not_run` includes items for which only the listed substeps were exercised.
`blocked` identifies an unavailable safe fixture, host capability or user
interaction. `failed` identifies an observed unmet contract. No row is marked
passed merely because a subset passed.
The rows follow the current walkthrough; additional UIA diagnostic observations
are retained separately below.

In the resumed evidence below, unprefixed `ui02-*`, `ui03-*`, `ui05-*`, `ui13-*`, `ui17-*`,
`ui18-*`, `ui21-*`,
`ui25-*`, `ui26-*`, `ui27-*`, `ui28-*`, `fresh-*` and `game-*` paths are under
`resume-20261031-0915\`; `shots\` in those rows is its screenshots directory.

| ID | Status | Actions, expected result and observation | Evidence |
| --- | --- | --- | --- |
| UI-01 | not_run | Mouse opened the Home menu; native UIA invoked Accounts, Games, Settings and About on separate runs and showed the corresponding pages. Full keyboard traversal, rapid reversal and draft preservation were not exercised. | `game-navigation.log`, `settings-navigation.log`, `about-navigation.log`; `shots\game-menu-native.png` |
| UI-02 | not_run | A native titlebar drag moved the task window by 120×72 pixels; the product Minimize control made it iconic and Windows Restore returned it. Task windows were normally closed and processes exited. Work-area changes and modal chrome remain untested; mixed-DPI movement is blocked by one display. | `fresh-probe.log`, `launcher-reopen.log`; `resume-20261031-0915\ui02-native.log`, `shots\ui02-{after-drag,restored}-native.png`. |
| UI-03 | blocked | Synthetic ASCII selection/copy/cut/paste/undo worked; a long ASCII/CJK/emoji value pasted exactly. On the previous candidate Home→Shift+End→Copy did not select the line. On the current candidate both Home→Shift+End→Copy and End→Shift+Home→Copy returned the complete 207-unit Unicode value. Windows SendKeys text was altered by the active IME, and real IME composition and grapheme editing were not established. | Previous: `resume-20261031-0915\ui03-unicode-clipboard.log`, `ui03-selection-observation.txt`, `shots\ui03-selection-after-native.png`. Current: `ui03-fixed-selection.log`, `shots\ui03-fixed-selection-native.png`. |
| UI-04 | not_run | Native UIA selected the Offline Mode radio; mouse toggled Reduce animations; Appearance selection worked. Empty/search-long lists, disabled entries and full keyboard focus behavior remain untested. | `offline-navigation.log`, `settings-preview.log` |
| UI-05 | not_run | The account removal dialog's Cancel returned focus to Remove account. In the real first-run import drawer, a short handle drag returned to its original position, content scrolling left it open, and a longer downward handle drag closed it to Overview. Toast timing and reopen-during-exit remain untested. The related Escape ownership failure over an inline Crash is recorded under UI-26. | `offline-delete.log`; `shots\delete-after-cancel-native.png`; `resume-20261031-0915\shots\ui05-{drawer-sources,after-short,after-scroll,after-handle-close}-native.png`, `ui05-action-{handle-short,handle-close}.log`, `ui05-scroll.log`. |
| UI-06 | not_run | Fresh Welcome corner press, outside-to-inside and inside-to-outside drags did not start setup; a complete center click opened the import Sources drawer. Earlier keyboard Skip worked. Enter/Space Start and touch remain untested. | `resume-20261031-0915\import-action-*-probe.log`; `resume-20261031-0915\shots\import-{corner-probe,outside-inside-probe,inside-outside-probe,center-start}-native.png` |
| UI-07 | not_run | Fresh Skip reached empty Home and persisted after restart. Continue after an import failure showed neither task complete; a separate successful import showed two games saved and no account. Remaining task combinations, restart of unfinished setup and Recommended/Custom branches were not exercised. | `skip-interaction.log`; `resume-20261031-0915\shots\import-failed-continue-native.png`, `import-mixed2-continue-native.png` |
| UI-08 | not_run | Normal discovery found the three task-owned adjacent candidates and no games in the official-launcher source. The unsupported row could not be selected; Clear selection changed 2 to 0 and Select all restored 2. Already-imported rows, source switching and cancel-before-copy remain untested. | `resume-20261031-0915\shots\import-{center-start,view-fixtures,unsupported,clear-selection,select-all}-native.png` |
| UI-09 | not_run | On the previous candidate, fresh Start failed both supported adjacent candidates because the fallback `instances` parent did not exist. On the current candidate a fresh store with that parent absent imported two small candidates, created the parent and saved two Pending instances and profiles with all nine source markers unchanged. A second fresh store produced one saved result and one controlled file-sharing failure; Retry saved only the failure without duplicating the first. Separate existing-library tests exercised mixed failure and running-batch Stop/retry. The Stop screenshot showed the second game waiting for copy, not cancellation after bytes began copying; drawer-close-during-batch remains untested. | Old failure: `resume-20261031-0915\shots\import-stop-result-native.png`. Current: `shots\fresh-fixed-{sources,list,result}-native.png`, `fresh-fixed-retry-{mixed,final}-native.png`; `fresh-fixed-{parent-before,parent-after,catalog-source,destinations}.txt`, `fresh-retry-{parent-before,before,after,catalog}.txt`. Existing-library: `shots\import-mixed2-{result,retry}-native.png`, `import-stop2-{progress-screen,result-native,retry-native}.png`. Paths in this row are under `resume-20261031-0915\`. |
| UI-10 | not_run | In a separate root, a synthetic offline account survived restart; removal Cancel preserved it and confirmed removal of the last account produced the empty state. Two-account inspection/use, loading and failure paths remain untested. | `offline-delete-setup.log`, `offline-delete.log`; `shots\delete-after-confirm-native.png` |
| UI-11 | not_run | Synthetic 17-character value produced validation; a valid name survived Back and review. A task-owned file lock then caused a native Offline Finish to fail with error 5 while retaining `FixtureSave11`; after release, the original config bytes matched, Try again returned to review, and Finish saved one account visible after restart. First-run Saving and leave-during-save ownership substeps remain untested. | `uia-offline-save-uia.log`; `resume-20261031-0915\ui11-native.log`, `ui11-restart.log`; `resume-20261031-0915\shots\ui11-{save-failure,retry-review,retry-saved,restart-account}-native.png` |
| UI-12 | blocked | A real Microsoft authorization requires user participation. Static fixtures do not establish live authentication, expiry or attempt ownership. | No live sign-in evidence. |
| UI-13 | not_run | Synthetic discovery was exercised through provider management as described in UI-14. In the native inline Add-provider dialog, an invalid quick link showed an error, a valid link normalized to its API address, and Cancel removed the exposed drop target. Actual OS drop, password/character flow and native secure-store paths were not run; Credential Manager was not touched. | `offline-navigation.log`; `resume-20261031-0915\shots\ui13-inline-{inline-add,invalid-link-recognize,valid-link-recognize,cancel-provider}-native.png`; `fixture-prep\ui13\`. |
| UI-14 | not_run | A task-local HTTP fixture required consent, rejected invalid metadata and a 404, and saved valid provider metadata. Editing the address required renewed consent; rename persisted across restart. Unlinked Remove Cancel preserved the provider and Confirm removed it. A separate synthetic linked-account metadata fixture showed the address read-only and Remove disabled, without signing in or accessing Credential Manager. Loading/retry, duplicate reuse and other URL paths remain untested. | `resume-20261031-0915\shots\ui13-{invalid-address,invalid-discover,missing-discover,valid-discover,restart-providers,remove-cancel,remove-confirm,linked-selected}-native.png`; `ui13-server.stderr.log`, `ui13-linked-readonly.txt`. |
| UI-15 | not_run | Bundled Games inspected B while A was current in the Crash fixture. The Games Help explained both modes and its Profile mode link opened the correct Settings section without leaving an overlay behind. After conversion, Instance and Profile tabs and their linked records were visible. Full searches, unavailable records and Help scrolling/focus remain untested. | `resume-20261031-0915\shots\ui27-inline-inspect-b-native.png`, `ui21-{help,help-profile-mode,separated-games,separated-profile,instance-view}-native.png`. |
| UI-16 | not_run | A real Bundled `BlockWinTest` 1.20.6 Vanilla setup was reviewed, created once and installed after a cancelled first attempt. Separated creation, failure/retry and first-run save-only remain untested. | `fixed-review.log`, `install-workflow.log`; `shots\game-review-before-create-native.png`, `shots\install-after-create-native.png` |
| UI-17 | not_run | On an earlier candidate, opening Version caused a GPUI focus panic; the wizard fix opened it and allowed search, 1.20.6 selection and Done. On the previous candidate, Fabric for 26.3 showed no builds; Done then Cancel restored Vanilla but left the abandoned loader error on the outer page. On the current candidate the same no-build/Done/Cancel path restored Vanilla without the stale error. Preview builds, retry and stale catalog responses remain untested. | Earlier: `version-picker.stderr.log`, `panic-repro.stderr.log`, `fixed-version.log`, `fixed-review.log`; `shots\fixed-version-dialog-native.png`; `resume-20261031-0915\shots\ui17-native-{fabric,no-build-done,after-loader-cancel}-native.png`. Current: `shots\ui17-fixed-{loader-error,after-loader-cancel}-native.png`. |
| UI-18 | not_run | On the previous candidate, Configuration rejected a relative installation folder, but Cancel left that abandoned error on the outer wizard. On the current candidate the same invalid Done still showed validation, and both Cancel and Escape returned without the stale error. OptiFine chooser, native chooser failure and late-response ownership remain untested. | Previous: `resume-20261031-0915\shots\ui18-{configuration,relative-error,relative-after-cancel}-native.png`; `ui18-{relative-error,action-configuration-cancel}.log`. Current: `shots\ui17-fixed-{location-error,after-location-cancel,location-error-escape,after-location-escape}-native.png`. |
| UI-19 | not_run | On a disposable Instance, the settings card opened the editor; an empty name produced field validation, Cancel discarded the invalid draft, and a valid rename saved to the catalog without changing markers. OptiFine, save failure and post-save reinstall outcomes remain untested. | `resume-20261031-0915\shots\ui21-{instance-editor,instance-invalid-save,instance-invalid-cancel,instance-save-valid}-native.png`; `ui21-settings-after-wizard.txt`, `ui21-instance-save-files.txt`. |
| UI-20 | not_run | An empty Profile name was rejected; Cancel then a valid rename saved. Data and account selected Shared and Done changed the draft, but final Cancel preserved Isolated; a second Done and Save persisted Shared. Remaining subpages, validation and checkpoint combinations remain untested. | `resume-20261031-0915\shots\ui21-{profile-invalid-save,profile-invalid-cancel,profile-valid-save,policy-shared-draft,profile-cancel-draft,profile-save-shared}-native.png`; `ui21-{policy-cancel-files,profile-saved-files}.txt`. |
| UI-21 | not_run | In a disposable catalog, Bundled B deletion Cancel preserved it and Confirm removed its Instance and Profile while preserving B's world/config/options markers. Conversion Cancel retained Bundled and Confirm switched to Separated. Isolated copy had a new Profile identity and no copied world; changing A to Shared left the old Isolated world in place. A Shared copy resolved to the same Instance game directory; its later deletion preserved the Instance, other Profiles and both world markers. More policy-transition and restart combinations remain untested. | `resume-20261031-0915\shots\ui21-{delete-cancel,delete-confirm,conversion-cancel,conversion-confirm,copy-isolated,copy-shared,reopen-delete-cancel,reopen-delete-confirm}-native.png`; `ui21-{deleted-markers,mode-after-cancel,copy-isolated-files,policy-cancel-files,shared-copy-files,reopen-deleted-files}.txt`. |
| UI-22 | not_run | Real installation showed unknown totals; Queue launch changed to Launch queued; Stop returned Cancelled/Retry download. Retry reached Ready and enabled Launch. Other Home identities and failure states remain untested. | `install-home-retry.log`, `install-queue-stop.log`, `install-retry.log`; `shots\queue-after-click-native.png`, `shots\queue-after-stop-native.png`, `shots\retry-final-native.png` |
| UI-23 | not_run | With A current and B inspected, native Repair completed for B while Home remained on A. B's manifest-declared client was restored and matched its declared SHA-1; the corrupt 44-byte copy was retained and synthetic world, config and options markers were unchanged. A separate failed Repair attempt remains untested. | `resume-20261031-0915\shots\ui23-{inspect-target-b,repair-start,home-during,repair-final-later}-native.png`; `resume-20261031-0915\ui23-repair-terminal-files.txt` |
| UI-24 | not_run | Home showed an unknown overall total during a real installation. Downloads sheet/file details, membership changes and nested dismissal were not exercised. | `shots\install-home-retry-native.png` |
| UI-25 | blocked | Closing the earlier launcher left Java and its supervisor alive and reopening showed the same game Running. A new isolated 1.20.6 Vanilla store created and reopened a world across two normal exits. A later current-candidate run's explicit Stop yielded `finished`/`stopped` without Crash help. In-world movement/inventory remains unverified after bounded input probes. | `launcher-reopen.log`, `reopen-home.log`; `resume-20261031-0915\shots\game-{create-world-screen,relaunch-world-list-screen,relaunch-open-world-screen,relaunch-save-screen}.png`, `ui25-stop-{running-home,stop-request,after-stop}-native.png`; `game-{normal-session,relaunch-terminal}.txt`, `ui25-stop-sessions-after.txt`. |
| UI-26 | not_run | During a real active 1.20.6 Vanilla run, Home showed one Running session and Stop, with no second Launch control; another Profile was unavailable in that store for inspection. On the previous candidate, Escape with Games Help over a synthetic Crash closed the launcher. On the current candidate, a synthetic Crash remained queued and unseen while Help was open; Escape closed Help, then Crash appeared and Return selected A. In a separate run the default Help Enter closed Help, and another Enter after more than 300 ms returned from Crash to A without refocusing. Switching another Profile while a game runs and real supervisor-triggered Crash remain untested. | Previous: `resume-20261031-0915\shots\ui26-modal-second-before-escape-native.png`, `ui26-modal-{action-escape-help,controller}.log`. Current: `shots\ui26-fixed-escape-{queued,after-escape,returned}-native.png`, `ui26-fixed-enter-{queued,after-first-enter,after-second-enter}-native.png`; `ui26-fixed-escape-seen-{before,after}.txt`, `ui26-fixed-enter-action-{first-enter,second-enter}.log`. |
| UI-27 | not_run | The actual Engine presented a staged synthetic abnormal report inline; Return selected A after B had been inspected and did not relaunch. Explicit standalone `--crash` for a separate synthetic record also returned to A with no Java process observed. An already presented inline report did not reappear after a 12-second restart observation. Real supervisor-triggered standalone presentation, failed-presentation recovery and the remaining report variants were not exercised. | `resume-20261031-0915\shots\ui27-inline-{inspect-b,crash-presented,returned,restart}-native.png`, `ui27-standalone-{crash,returned}-native.png`; `ui27-{inline-return,inline-restart,standalone-native}.log`. |
| UI-28 | not_run | Standalone synthetic Crash export opened the real Windows Save As dialog; Escape canceled without a file, and a later Save produced parseable JSON containing the synthetic report. Error, pending-release and late-response ownership remain untested; there is no supported native late-response control prepared. | `resume-20261031-0915\ui28-export*.log`, `ui28-export-validation.txt`, `ui28-export.json`; `shots\ui28-cancel-dialog.png`, `ui28-{after-cancel,targeted-saved}-native.png`. |
| UI-29 | not_run | About copied complete unstamped build JSON to the Windows clipboard; a real Save As cancel created no ZIP. A task-owned ZIP was exported and opened; confirming overwrite of a synthetic existing file produced an error and left its contents unchanged. External links, stalled/error picker and write-failure paths remain untested. | `about-clipboard.log`, `about-export-retry.log`, `about-existing-retry.log`; `exported-logs.zip`, `shots\existing-retry-after-native.png` |
| UI-30 | not_run | Dark and Reduce animations previewed, canceled and saved across restart. In a separate store, saved default memory 3 GB survived restart and appeared in a new Profile wizard; existing Profiles remained at 2 GB and canceling the wizard created no record. Restore, multiple-section drafts and failed Save remain untested. | `settings-preview.log`, `settings-save.log`, `settings-network.log`; `shots\settings-restart-general-native.png`; `resume-20261031-0915\ui21-settings-{before-restart,after-restart,after-wizard}.txt`, `ui21-defaults-wizard-value.txt`. |
| UI-31 | not_run | Parallel downloads 17 and retries 11 showed field validation. A synthetic proxy URL with a username was rejected as unsupported credentials. A Direct/parallel-3 draft Test connection returned Mojang metadata HTTP 200, endpoint-only; Cancel left saved system proxy and parallel-8 unchanged. Storage showed read-only paths. Saved network effects and stale-response identity remain untested. | `network-validation.log`; `shots\network-invalid-parallel-native.png`, `shots\network-invalid-retry-native.png`, `shots\settings-storage-real-native.png`; `resume-20261031-0915\shots\ui21-{proxy-credentials-rejected,network-draft-test}-native.png`, `ui21-settings-after-wizard.txt`. |
| UI-32 | not_run | Welcome Tab traversal to Skip and Enter activation were observed. Complete keyboard/focus journeys through fields, validation, choices, stacked dialogs and Crash actions, plus scale/long-content variants, remain untested. Static light/dark captures do not establish those interactions. | `skip-interaction.log`; corresponding `shots\*.png`, `matrix\` |

## Prepared fixtures for a later desktop session

The following preparation did not start a launcher, Java, supervisor, crash
observer or HTTP server. All paths are under `fixture-prep\`, separate from
`data-keyboard-skip`. The one-shot `seed-fixtures.exe` used the actual
`Catalog` APIs and official manifest decoder; it also read back staged Crash
and session documents using `session_host::read_crash` and `read_run`.
`fixture-prep\seed.stdout.log` and `fixture-prep\verify-fixtures.stdout.log`
record the checks. Parsing and catalog validation do not establish that the
native Engine or UI consumed a fixture.

For a future authorized interactive Windows desktop session, define these paths:

```powershell
$task = 'D:\block-gui-verify-985238e-20261030T143906Z'
$prep = Join-Path $task 'fixture-prep'
```

`launch-fixture.ps1` sets task-local logging and temporary directories and
launches the earlier `bin-fixed\block.exe` with the selected isolated
`--data-dir`. Later tests used their own controller scripts and the
`bin-import-parent` or `bin-modal-wizard-input` executable identified above.
Run a launcher only on the
interactive desktop, or through the existing
interactive-task mechanism, after a desktop slot is explicitly arranged:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$prep\launch-fixture.ps1" -Case ui11 -ConfirmDesktop
```

The fixture stores start with setup already marked complete. Overriding
`--data-dir`, `LOCALAPPDATA` or `APPDATA` does not redirect Windows Known Folder
source discovery; the resumed Start/import run used the explicit authorization
described above. Do not copy a prepared store to another path: its catalog
contains absolute task-owned installation paths.

### UI-11: offline account save failure and retry

- This fixture has now been consumed by the native failure/retry run above and
  contains the saved synthetic account; it is not a pristine repeat fixture.
- **Start:** launch case `ui11`; create a synthetic Offline account through
  Review, stopping before Finish. Its store starts with only
  `block-global.toml` preferences and no account.
- **Fault:** in a separate PowerShell session run
  `powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$prep\ui11\hold-account-save.ps1"`.
  After `hold.ready` appears, activate Finish. The helper holds only this
  fixture's `block-global.toml` open without delete sharing; it has a ten-minute
  watchdog. Account persistence uses `block-global.toml`, not the catalog's
  `block.toml`.
- **Release:** write `hold.release` in `ui11\` and wait for `hold.result`.
  Check `baseline_unchanged=True` before retrying; if it is false, preserve the
  files and stop rather than overwriting them. `block-global.original` is the
  exact synthetic baseline.
- **Expected:** the first Finish reports failure and retains the draft; after
  release, retry saves exactly one account across restart.
- **Cleanup:** release the helper, wait for it to exit and retain the store and
  evidence. The non-GUI `tempfile::persist` probe reproduced Windows error 5
  with the held file, verified unchanged bytes and succeeded after release
  (`ui11\persist-probe.stdout.log`). No launcher Save was run by that probe.

### UI-13: Authlib provider discovery

- The native run consumed `ui13` and removed its unlinked provider. The separate
  `ui13-linked` store contains only synthetic linked-account metadata, not a
  credential. The short-lived task-local HTTP server has been stopped.
- **Start:** `ui13\index.html` contains the public metadata shape used by the
  existing Authlib tests; `ui13\invalid\index.html` lacks the required key.
  During a later desktop slot, an operator may run the standard-library server
  below only after confirming the port is unused, then launch case `ui13`:

  ```powershell
  python.exe -m http.server 43179 --bind 127.0.0.1 --directory "$prep\ui13"
  ```

- **Fault / release:** discover `http://127.0.0.1:43179/` with explicit HTTP
  consent; `/invalid/` supplies invalid metadata and a missing path supplies a
  404. Retry the valid root. Stop only this temporary server with Ctrl+C after
  the test. The prepared key is the test's public shape fixture, not evidence
  of a real provider signature or a functioning login service.
- **Expected:** valid discovery can save only synthetic provider metadata;
  invalid discovery reports an error without binding it to another draft.
- **Credential boundary:** do not start Authlib sign-in or the secure-store
  probe under the current authorization. Both use the current user's Windows
  Credential Manager. Authlib uses service `dev.block.launcher.authlib` and a
  saved reference shaped as
  `authlib:<provider-uuid>:<hash-of-login>:<player-uuid>`; preflight also
  writes and removes an `authlib:storage-check-<uuid>` reference. A unique
  provider UUID generated for this fixture, login `FixtureOnly413` and player
  UUID `00000000-0000-4000-8000-000000000413` would avoid reference
  collisions; record the public provider UUID from this fixture's own saved
  metadata. This would not isolate the shared OS store. Approve or supply a
  truly isolated store before exercising those writes.
- **Cleanup:** stop the task-local HTTP process; do not enumerate, read or
  delete Credential Manager entries.

### UI-23: noncurrent Ready-but-damaged Repair

- This fixture has now completed the native repair described above; retain it
  and its quarantined copy rather than treating it as damaged for a rerun.
- **Start:** case `ui23` contains two Catalog-created 1.20.6 Vanilla records,
  `Repair home A` and `Repair target B`, both marked Ready. IDs and paths are
  in `ui23\identities.json`. Before Repair, verify A is Home's current Profile,
  inspect B and check B's task-owned `FixtureWorld`, config and options markers.
- **Fault / release:** B's store has a deliberately corrupt client view at the
  path declared by the repository's original official 1.20.6 manifest; no
  verified cache object was invented. An actual Repair must obtain the
  declared resources from official sources. Record a network failure
  separately; only retry after the real dependency is available.
- **Expected:** the inspected B target is repaired without changing Home A;
  on successful repair the corrupt client copy is retained as
  `client.jar.corrupt-<uuid>`, the declared immutable client is restored and
  the mutable markers remain unchanged. A failed attempt is not a completed
  repair.
- **Cleanup:** retain the corrupt copy, markers, catalog and logs for review;
  do not read or copy a generated execution plan. The manifest and catalog
  passed format/identity checks before the later native Repair.

### UI-26: single active session and Crash over a modal

- The supported GUI currently permits one active run. The real running-game
  observation and the synthetic Crash-over-Help result are recorded above.
  The `ui26-modal` store and published records remain available as failure
  evidence; they do not represent a real supervisor crash.
- The earlier `ui26` preparation contains two Pending Profiles with distinct
  fake-Java stand-ins, but was not run as a two-session test. Its local Java
  responds to the version/architecture probe and does not log arguments.
  The public CLI launches recipes through the executor, not a supervised
  Profile session; it is not a substitute for a second GUI session.
- A read-only `Locations::validate` probe accepted the independent `ui26` root
  with the task-owned `game-new\store` as its cache
  (`resume-20261031-0915\ui26-locations.stdout.log`). The corresponding
  `ui26-cache-size.txt` records logical sizes: 851,372,242 bytes of objects,
  818,928,471 bytes of views and 99,041,788 bytes of runtimes; two existing
  run-specific native directories each contained 3,841,024 bytes.
  Immutable views are copied, not hard-linked. Reuse of already present views
  can avoid bulk copying, but cache completeness and future write volume were
  not proven; the probe did not change Locations or install another game.
- Preserve the failed modal fixture and its logs. Do not kill saved PIDs, clear
  reservations, or introduce a second-session hook to fill this coverage gap.

### UI-27: inline and standalone Crash help

- Both staged cases were published and consumed during the native run above.
  Their `seen.json` markers and records are retained as evidence; use a fresh
  independent fixture for another first-presentation test.
- **Start:** `ui27-inline` and `ui27-standalone` each have their own store,
  Catalog-created A/B Profiles and sanitized abnormal-exit payload under
  `payload\sessions\<run-id>`. For inline, choose A as current and inspect B.
- **Fault / release:** only in the arranged desktop session, publish the
  staged inline payload with the explicit command below. It copies into a
  non-run staging name and then renames into that fixture store's `sessions`
  directory; an open launcher can observe the resulting record.

  ```powershell
  powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$prep\publish-crash.ps1" -Case ui27-inline -ConfirmPublish
  ```

  For standalone, publish `ui27-standalone` while its launcher is closed, then
  run `launch-fixture.ps1 -Case ui27-standalone -StandaloneCrash -ConfirmDesktop`.
  The wrapper uses the staged run ID. Each case is independent because
  successful presentation records `seen.json`.
- **Expected:** inline and standalone help show the sanitized A report; Return
  selects A rather than the inspected B and does not relaunch it. Presentation
  failure/recovery and repeated-presentation ownership need separate controlled
  attempts; the prepared JSON alone does not prove them.
- **Cleanup:** close only the fixture's own windows after recording results;
  retain its reports and markers. Never publish into the active gameplay store.

### UI-28: Crash export cancellation and late responses

- The native cancellation and subsequent export consumed this synthetic case.
- **Start:** `ui28` has a separate staged Crash payload. With a fresh case,
  publish it in an authorized desktop slot, then use
  `launch-fixture.ps1 -Case ui28 -StandaloneCrash -ConfirmDesktop`.
- **Fault / release:** the observed Windows Save As dialog is owned by the app
  and can be canceled with Escape; retry with a new destination under `ui28\`.
  No supported native mechanism was prepared to hold an old picker response
  past cancellation or view release. Do not add a product hook or treat a
  normal cancel as a late-response test.
- **Expected:** cancellation creates no file and retry remains usable. The
  actual late-response and Return-before-response ownership checks remain
  `not_run` on Windows.
- **Cleanup:** dismiss only the fixture dialog/window and retain any task-owned
  output. The existing headless `stalled_export_can_be_cancelled_and_a_late_choice_cannot_write`,
  `inline_return_releases_the_waiting_export_without_writing` and About
  stalled-picker tests passed in `final-ui-test-retry.stdout.log`; those tests
  use simulated picker responses and are not native Windows picker evidence.

## Findings, diagnostic observations and current limitations

1. **Resolved on the current candidate:** opening the real Version card on the
   previous controls candidate terminated the launcher with
   `gpui-pre-0.3.9/src/window/a11y.rs:533:17: set_focus called more than once in a single frame`.
   The initial run and one backtrace diagnostic both failed. The current wizard
   candidate reopened the same real-engine path with UIA active, showed one
   focused Search versions field and did not panic. The original and new logs
   remain separate.
2. **Additional UIA diagnostics:** the visible Home menu button, Home primary
   action, account ListItem, Settings sidebar buttons and Appearance ComboBox
   were exported with empty names. The corrected shared Skip label was verified
   independently; that correction does not cover these other owners.
   `updated-ax.log`, `game-navigation.log`, `settings-navigation.log` and
   `offline-delete.log` retain those observations.
3. **Resolved on the current candidate:** fresh adjacent-source import
   previously rejected two supported task-owned candidates because the fallback
   `instances` parent did not exist. The original failure store is retained.
   The current candidate created that parent and saved both candidates on a
   fresh store; an independent fresh mixed-failure/retry test also saved exactly
   two records without changing source markers.
4. **Resolved on the current candidate:** the previous candidate presented a
   synthetic Crash beneath an open Games Help dialog, and one Escape closed the
   launcher. The current candidate kept the report unseen until Help closed;
   Escape then presented Crash, and a separate default-Enter/Enter run returned
   to A after the modal had closed. The old failure and new captures are
   retained under `resume-20261031-0915\`.
5. **Resolved on the current candidate:** after invalid Configuration or
   Loader edits, Cancel previously left the discarded section's error on the
   outer wizard. Current-candidate native tests retained invalid-Done
   validation while Cancel and, for Configuration, Escape cleared that error.
   The old and new screenshots are retained under `resume-20261031-0915\shots\`.
6. **Resolved on the current candidate:** the previous native text input did
   not select the full line using Home→Shift+End. The current candidate copied
   the complete Unicode value in both Shift+End and Shift+Home directions.
7. **Windows native integration:** clipboard tests replaced the existing
   clipboard without reading it; the latest selection test left synthetic text
   on the clipboard. Native external-link opening, touch and mixed-DPI behavior
   remain unverified.
8. **Permissions and unfinished preparation:** real Microsoft authentication,
   isolated native Credential Manager access and in-world gameplay input retain
   the specific boundaries noted above. Separate failed-repair and real
   supervisor-triggered standalone Crash substeps have not yet completed
   native UI verification; controlled Windows late-response injection remains
   unprepared.

The synthetic game and reopened launcher were deliberately kept alive for the
user-assisted gameplay step: Java PID `2072`, supervisor PID `5800` and
reopened launcher PID `6316` were alive at the original handoff. The waiting
task driver PowerShell PID `11024` was forcibly stopped and its scheduled task
deleted to prevent a later automatic launcher close; the game and launcher were
not terminated by that cleanup. A later read-only PID check at
`2026-10-31T07:58:24+08:00` found none of the three PIDs. No exit reason or
exit code was observed, so this is not evidence of normal exit, Stop or a
product failure.

The later task-owned game reached its recorded normal and stopped outcomes.
The resumed launcher windows were closed normally except for the observed
UI-26 Escape failure; their interactive scheduled tasks were removed. The
task-local provider server exited after its stop signal. D: had 4,848,529,408
bytes free at the last check. No shared cache was cleaned.

The initial 14,192,258-byte source upload timed out twice; its two task-owned
PowerShell processes were forcibly stopped before the official-origin Git
object was fetched. No source archive was left by those uploads. The task root,
logs, captures, synthetic data and shared build outputs were retained; no other
host data or caches were cleaned.
