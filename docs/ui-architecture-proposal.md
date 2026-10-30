# UI and crate boundaries

This design defines the manifest ownership and native UI navigation boundaries.

## Crate boundaries

Official metadata lives in `block_launcher::manifest`. The strict decoder,
resource collection and official model share one core module; the audit tool
is a second binary in `block-cli`.

Launcher core and the native UI consume the same public metadata model.
Manifest types also appear in launcher's parsing and resolution contracts.
The public module preserves that ownership boundary without a separate package
or a second set of UI models.

```text
block-launcher/
  src/resolution/...
  src/execution/...
  src/persistence/...
  src/manifest/{mod.rs,model.rs,resources.rs}
  tests/{manifest_parsing.rs,manifest_resources.rs,official_manifests.rs}
  tests/fixtures/manifests/...
block-cli/
  src/bin/manifest-audit.rs
  tests/manifest_audit.rs
```

Create a crate when a current consumer needs an independently usable boundary,
when platform or dependency isolation has a real benefit, or when a separately
operated tool has a distinct lifecycle. Use a module for coupled implementation
details, shared schemas within one core, and feature organization. File size or
a desire for another test target does not by itself justify a crate.

Keep these existing divisions:

- `block-launcher`: persistent catalog, preparation, launch planning and session
  supervision shared by CLI and desktop.
- `block-auth`: authentication protocols and the secure credential boundary.
- `block-download`: transfer, caching, verification and cancellation.
- `block-drop`: platform-specific native integration, including its deliberately
  isolated unsafe code.
- `block-ui`: native presentation and UI orchestration.
- `block-app` / `block-cli`: application entry points, not parallel core rules.

### Migration and acceptance

1. Move the decoder/models/resources and parser tests into launcher, preserving
   unknown-field rejection, complete-document parsing, lossless serialization
   and field-path errors. Change core imports to `crate::manifest` and other
   consumers to `block_launcher::manifest`.
2. Keep one offline fixture corpus. Update every sibling-path reference in
   launcher, UI and audit tests. The full official corpus remains ordinary Cargo
   coverage, including exact membership checks.
3. Move `manifest-audit` and its subprocess tests together to CLI, retaining the
   binary name, subcommands and experiment-directory behavior.
4. Retarget `just manifest-fetch`, `manifest-fetch-snapshot` and
   `manifest-verify`; update package dependencies, lockfile and current docs,
   then remove the old member. No compatibility crate or duplicate DTO layer.

Run parser/resource/corpus and audit subprocess tests, then all affected
launcher/UI consumers and `just check`. This is a package/import migration,
not a parser redesign. The tradeoff is loss of inexpensive standalone
parser-only compilation; no runtime-speed or binary-size gain is claimed.

## Layout, Slot and navigation

A small internal typed navigation model and a shared page host own page
composition. Rendering and transition preparation consume the same `PageSpec`,
instead of independently inferring page identity from forms and shell state.
There is no dynamic route registry or web-style framework.

```text
src/
  lib.rs            # crate-level module map and application state
  navigation/       # Router, PageSpec, page host, shell and page/frame motion
  accounts/         # authentication, providers and shared account draft state
  games/            # browsing, game creation, editors, import and downloads
  setup/            # modern first-run stages, storage and Welcome control
  controls/         # shared controls, selectors, scrolling and layout helpers
  overlays/         # component surfaces and Drawer motion
  feedback/         # Toasts, connection feedback and crash help
  runtime/          # engine adapter, events, platform helpers and scenarios
  settings/         # preferences, theme, assets and application information
```

Feature modules supply content; the host applies layouts and shared motion.
Keep handwritten source directories small and grouped by responsibility, with
tests beside their owner. Explicit crate-root path mappings preserve logical
module names without wrapper modules or public API churn. Embedded icons are
physically grouped by role while their logical asset keys remain stable.

### Ownership and interfaces

- **Router** owns typed current destination and task return context. It accepts
  navigate/back requests; it does not own clocks, animation values, resizing,
  authentication, persistence or page drafts.
- **PageSpec** supplies typed page/region keys, frame class, layout kind and
  transition preset. Rendering and motion consume this same identity.
- **Slots** are explicit fresh GPUI elements:
  `ManagementSlots { sidebar, detail }` and
  `WizardSlots { rail, body, footer }`. The detail keeps its scrolling body and
  fixed footer together. No dynamic injection or implicit context lookup.
- **PageHost** reconciles page/region keys once and applies the existing frame,
  scene and entrance machinery. Pages provide content and actions instead of
  applying their own transition wrappers.
- **Component hosts** retain Drawer, Toast, disclosure and import-list motion.
  Base continues owning modal semantics and primitives.

This centralizes motion management, not motion simplification. Retain Home's
coordinated width morph and reversal, staggered panes, detail-only transitions,
wizard steps, first-paint anchoring, inert outgoing controls, focus traversal
and immediate reduced-motion settlement. Welcome keeps its distinct staged
exit, hidden resize and entrance strategy.

Use the shared transition/presence implementation. Visual snapshots
must remain engine-free and exclude authentication secrets. Animation must not
extend the lifetime of live credential-bearing page state.

### Migration and acceptance

1. Extract named layouts and explicit slots while keeping navigation behavior.
2. Define typed route/region identity once; preserve drafts and explicit task
   return destinations.
3. Migrate management/detail orchestration into PageHost, then wizard and frame/
   Welcome strategies. Remove duplicate wrappers as each family migrates.
4. Group feature files after interfaces settle rather than mixing file moves
   with behavioral changes.

At each stage exercise forward/back, interruption/reversal, retained drafts,
outgoing-control inertness, focus restoration, live input, stacked overlays and
reduced motion in the actual native UI. Retain presented-frame evidence for
changed motion orchestration; static screenshots do not establish it.

Changes to page availability, draft discard or animation replay behavior are
separate product decisions, not implied by reorganizing code.

## Reachability and adjacent polish

Keep Home, Games, separated Instances/Profiles, Accounts, providers, Settings,
About, modern first run, contextual editors, download details and crash help.
Loading/error/verification states are conditional, not dead pages.

Retire the old Welcome guide replay, its menu entry and its private
Game/Account/Review controller. Preserve modern first-run completion/account
state and shared wizard content. First-run navigation restrictions remain:
ordinary Settings/About navigation is not a new setup preview feature.

OptiFine's management entry opens the existing Instance draft and reveals its
editing context; the general settings entry still focuses Instance Name.
Profile editor headings use the shared title hierarchy and consistent
“Launch settings” naming.
