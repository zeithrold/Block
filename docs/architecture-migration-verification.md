# Architecture migration verification

## Implemented boundaries

- Official metadata is public at `block_launcher::manifest`; `manifest-audit`
  is a `block-cli` binary. One fixture corpus lives under
  `block-launcher/tests/fixtures/manifests`.
- Router owns typed page/task destinations and authentication return context.
  Page rendering, region transitions and frame choice consume one `PageSpec`.
  The shared page host consumes fresh management/wizard slots; component hosts
  retain their own motion.
- Legacy Welcome replay, menu entry, controller, private actions/surfaces and
  six debug scenarios are removed. Modern first run retains its account and
  completion state and navigation restrictions.
- The OptiFine management card opens the existing Instance draft with its
  section revealed and focused. Generic settings still focuses Name; Cancel
  restores the triggering card without saving.

## Architecture migration checks

Host: macOS ARM64. Candidate: the integrated architecture changes above
`b36681247e7c18686e10b3ef2d2eda2c1c7b5b2b`, including the preceding UI polish,
before physical source-directory regrouping. These checks are not Linux ARM64
UI acceptance.

- `just check`: passed formatting, workspace/all-target/all-feature type checks,
  strict Clippy and all 662 tests, including 335 UI unit tests and three
  framework tests.
- `just manifest-verify`: passed the retargeted corpus recipe.
- `cargo test --locked -p block-cli --test manifest_audit`: passed both real
  audit subprocess cases.
- `cargo build --locked -p block-app --bin block`: passed.
- Direct comparison with the baseline confirmed all 1,020 relocated fixture
  files are byte-identical with exact membership. Decoder/model/resources
  differ only in the nested resource import.
- The final binary's `--list-scenarios` excludes the six legacy replay names
  and retains modern setup.
- All 15 scenario/capture/review-case Python tests passed with the
  corrected modern setup title/frame contracts.

Focused regressions cover the real OptiFine card click, reveal, Enter-to-prompt,
Cancel focus return, unsupported Loader reason and generic Name focus. Router
tests cover task-return context, shared page/region identity, setup guards and
late catalog results. Welcome regressions cover retained drafts on return,
staged resizing and immediate settlement when reduced motion changes.

Independent source reviews found no outstanding functional issues in manifest
migration, navigation/draft ownership or page-host motion orchestration.

## Source-directory organization checks

The UI source root contains only `lib.rs`; its nine responsibility folders
contain 4–12 direct files each. Core's source root contains nine direct Rust
files. The 48 SVG icons are in six role-based folders containing 3–15 icons.
Logical Rust module names, public APIs and runtime asset keys are unchanged.

Direct source comparison confirms that only root module maps, nested test/helper
paths and embedded resource paths changed. All 48 SVG contents are byte-identical.
Current `cargo fmt --all -- --check`, the 15 Python scenario/capture/review tests,
and core strict Clippy passed. The core worker also ran 147 unit and 90 integration
tests successfully on its identical moved core source.

Native UI verification is assigned to Linux ARM64. The configured host
`10.77.0.21:22` accepts TCP but closes the SSH connection before authentication.
The reorganized UI's Linux build, tests and native interaction remain blocked
until host access is restored; no additional macOS UI run replaces them.

## Native evidence and limitations

The macOS architecture-migration executable passed exact native title, dimensions and OCR assertions
for Home, Accounts, authentication, game Setup/Review and Instance settings in
both themes: 12 captures. Modern setup Overview, ready Overview, Game choice,
Account and Game also passed in both themes: ten captures. Artifacts remain in
ignored `target/parity-shots/native/`. Import Sources and List passed in both
themes as well, for 26 automatically validated captures in total.

Modern Welcome was inspected in a retained native capture at 360×720. Its sparse
text confuses the host OCR orientation detector; automated content validation
does not pass. The required phrases remain mandatory, and the validator's
thresholds are unchanged.

Before migration, automatic native frame/pane replays passed with 322 and 236
foreground paint samples respectively. After migration, native activation
returns false and the replay remains before its first action. This comparison
is blocked; no post-migration motion equivalence or display-presentation claim
is made. Failed traces and activation observations are retained under ignored
`target/router-migration/`.

OS pointer injection is also unavailable because Accessibility/PostEvent
permissions are absent. GPUI interaction tests do not substitute for this native
pointer/focus acceptance. No system permissions were changed.

No Windows/Linux native runs, live Microsoft authentication, Minecraft gameplay
or live manifest downloads were performed. The dependency `block` emits Cargo's
future-incompatibility notice; strict project Clippy passes.
