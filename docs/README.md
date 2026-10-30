# Block documentation

## Start here

- [Project README](../README.md): setup, running Block, workspace layout and commands.
- [Current status](STATUS.md): implemented behavior, native evidence and compatibility limits.
- [Next tasks](next-tasks.md): milestone priorities, product decisions and test-host inventory.
- [Test-platform guidelines](test-platforms.md): check selection, flexible host use and evidence reuse.

Status and next tasks own the current milestone picture. Feature reports below
describe their named scope and recorded runs; they do not independently set
product acceptance or require repeating historical verification.

## Usage and development guides

- [Launch recipes](recipes.md): CLI preparation, execution and compatibility options.
- [Configuration storage](config-storage.md): data roots and TOML documents.
- [Logging](logging.md): local launcher diagnostics, filters, retention and export.
- [Session supervision](session-supervisor.md): process ownership, crash help and recovery.
- [Authentication](../crates/block-auth/README.md): public-client setup and native credential storage.
- [Pilot builds](pilot-builds.md): package workflow, build profile and platform limits.
- [Packaging](../packaging/README.md): desktop artifacts, installers and product identity.
- [Minecraft manifest experiment](../experiments/minecraft-manifests/README.md): corpus, schema observations and reproduction.

## Design and implementation

These documents explain feature decisions and implementation scope. Consult
[next tasks](next-tasks.md) for which remaining work is currently scheduled.

- [UI refinement plan](ui-refinement-plan.md): the first Milestone III interaction package.
- [UI and crate boundaries](ui-architecture-proposal.md): manifest ownership, typed navigation and shared page layouts.
- [Home and management frame](home-management-frame.md): dimensions, navigation and shell transitions.
- [Toast and Drawer implementation](toast-drawer-implementation.md): behavior and timing.
- [First-run onboarding](onboarding-implementation-plan.md): agreed flow, implementation and review.
- [Custom window chrome](window-chrome-implementation.md): the production increment and its verification.

## Verification and investigation reports

### UI and native interaction

- [Whole-product GUI coverage audit](gui-coverage-audit.md): surface inventory, nine-issue repair disposition, native evidence and remaining checks.
- [Architecture migration verification](architecture-migration-verification.md): crate migration, Welcome retirement, OptiFine entry and page-host checks.
- [Evidence calibration](evidence-calibration.md): corrected account/scenario captures and comparison semantics.
- [Visual review cases](visual-review-cases.json): machine-readable scenario/reference mapping used by the review tools.
- [Drawer motion comparison](drawer-motion-comparison.md): frame measurements and reproduction.
- [Motion platform verification](motion-platform-verification.md): Windows/Linux observations and analyzer limits.
- [Motion platform fixes](motion-platform-fixes.md): changes and native follow-up checks.
- [Onboarding platform follow-up](onboarding-platform-followup.md): exit-point changes and platform results.
- [Window chrome feasibility](window-chrome-feasibility.md): isolated native capability probes.
- [KDE Xwayland investigation](xwayland-compatibility-investigation.md): geometry, input behavior and integration boundaries.

### Builds and performance

- [Release binary size](binary-size.md): measured options, attribution and platform tradeoffs.
- [Linux glibc baseline](linux-glibc-baseline.md): compatibility investigation, build options and reproduction.

## Archive

Completed task briefs, superseded design-stage assumptions and earlier acceptance
records live in `archive/`. Their results retain their original scope; their
instructions and unfinished queues are not current execution guidance.

- [MVP acceptance record](archive/mvp-progress.md): early gameplay and authentication baseline.
- [Milestone II goal prompt](archive/codex-goal-prompt.md): the closed functional-closeout brief.
- [Initial launch-engine design](archive/minecraft-launcher-design.md): original resource preparation and execution design.
- [KDE Xwayland task brief](archive/xwayland-investigation-task.md): the completed investigation assignment; results are in the report above.
