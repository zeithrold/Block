# Test-platform guidelines

## Scope and entry points

Use test platforms to answer a concrete development question: reproduce a fault,
check a changed behavior, inspect native presentation, measure a workload, or
verify a release artifact. Platform access does not make a full acceptance run
necessary. This document owns test selection and host-use policy; [host access
and existing authorizations](next-tasks.md#operations-user-2026-10-22),
[commands](../justfile), and [compatibility results](STATUS.md) remain separate.
Historical evidence reports describe their own runs, not prerequisites for every
new task. Explicit task acceptance requirements and CI gates still apply.

## Choose the evidence that changes the decision

Before running a check, name the failure it could detect and what would change
if it failed. Use the narrowest existing entry point that exercises that path.
Stop when the task's claim has sufficient evidence; do not repeat a successful
check without a relevant change or unresolved uncertainty.

- Documentation-only changes: inspect the diff, references and rule consistency.
  Do not build Rust, contact test hosts or recapture UI for prose alone.
- Contained code changes: start with the affected package/test. Use the
  [justfile](../justfile) for workspace commands, retaining `--locked`; use
  `just check` for workspace-wide verification, not as a prerequisite to every
  iteration or native observation.
- Shared core or cross-cutting changes: cover affected consumers and failure
  paths, then run the full workspace checks. Use CI's native matrix rather than
  repeating the same full suite manually on every test host.
- Native UI changes: exercise the affected page/state and actual interaction on
  a suitable native desktop. Start on one available affected host; extend to
  other OSes, display backends or architectures where the changed dependency,
  platform branch or observed symptom makes their behavior relevant. A
  representative result is not a pass for untested platforms.
- OS integration, packaging and architecture-specific changes: test the actual
  affected target and artifact. Cross-compilation establishes build evidence,
  not native execution. An OS VM can establish launcher interaction, but cannot
  establish missing hardware capabilities such as a working graphics driver.
- Authentication and game lifecycle changes: exercise the affected real flow.
  Offline fixtures do not prove Microsoft sign-in; reaching a menu does not prove
  world gameplay, normal exit or relaunch. Do not repeat unrelated sign-in or
  game/version/Loader combinations for a visual-only change.

Static scenarios, API drivers, screenshots, motion traces and manual interaction
are complementary tools. Choose among them by the claim: screenshots can show
layout, not focus restoration or animation; synthetic input that cannot operate
the desktop leaves that interaction blocked, not failed product behavior. Use
manual native observation when automation cannot answer the question, recording
what was actually observed. Full visual/product acceptance remains the user's
decision.

## Use hosts flexibly

The [host inventory](next-tasks.md#test-hosts-addresses-in-env) records known
working methods, not an exclusive tool or service-placement requirement. Choose
SSH, interactive desktop access, scheduled tasks, native capture tools or a
task-owned helper as appropriate. Existing authorization permits test-host
software installation and temporary services; stop those services after use.
Do not add a permanent orchestration layer for a one-off check.

Use the existing checkout and Cargo/download caches by default. A separate
task-owned checkout, worktree or source snapshot is appropriate for concurrent
work, regression comparison or isolating uncommitted changes. Follow workspace
attachment/edit permissions before editing another checkout. Keep Windows AMD64
work on D: because C: is small. Never run heavy Windows AMD64 and Linux AMD64
builds/tests concurrently: they share one physical machine. Coordinate other
heavy work when it competes for the same host resources.

All test hosts may retrieve current source, including uncommitted changes.
Record the commit and relevant patch/snapshot plus the executable used; do not
substitute the host checkout's current HEAD for an older running binary. Do not
introduce per-file hashes, source fingerprints or receipt systems as routine
proof. Existing capture tools keep their own input contracts; replacing reviewed
evidence still requires their validation, not weaker assertions.

Exclude `.env`, credentials, Git metadata and player data from transferred source
snapshots. Do not copy `target/` between hosts; building an artifact for a
designated target and transferring that artifact for a packaging/runtime check
is different from copying build caches. Use task-owned data stores for fixtures;
never overwrite real worlds, configuration, source installations or credentials.
Do not put secrets in command text, logs or evidence. Test access does not
authorize destructive cleanup, publication or landing on a shared branch.

## Reuse and report evidence

Reuse a prior result when the code path, dependencies, environment and recipe
supporting its claim remain applicable. A documentation-only commit does not
invalidate native evidence. After a relevant implementation, dependency, fixture
or environment change, rerun the affected checks, not unrelated pages or hosts.
Name the earlier run and the reason it still applies; do not report reuse as a
fresh execution. Preserve historical results and their limits.

Report the command or manual action, source/artifact, OS/CPU and relevant desktop
conditions, observed result and any local evidence path. Include Minecraft
version/Loader when testing games. Keep this in the task summary or existing
report; a new ledger or machine receipt is not required.

Use `passed`, `failed`, `blocked`, `not_run` and `unknown` literally. List omitted
checks by relevant scope and reason, not an exhaustive catalog of unrelated tests.
An unavailable host or graphics driver is blocked; a deliberately unnecessary
check is not run. Do not retry without a cause, relax expectations after failure,
or convert missing evidence into acceptance.
