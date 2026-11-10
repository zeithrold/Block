# Working on Block

## Architecture and data boundaries

- Keep desktop and CLI launch behavior in the shared `block-launcher` core rather than implementing parallel rules in each interface. See [workspace architecture](README.md#workspace-layout).
- Official metadata belongs to `block-launcher::manifest`; `manifest-audit` belongs to the CLI. Prefer modules for coupled core features; create crates for real consumer, dependency or platform boundaries. See [architecture boundaries](docs/ui-architecture-proposal.md).
- UI page/frame/region identity comes from `navigation::PageSpec`; the shared page host composes slots and orchestrates page motion, while component hosts retain their own motion. See [UI ownership](docs/ui-architecture-proposal.md#ownership-and-interfaces).
- Keep handwritten source directories small and grouped by responsibility, with tests beside their owner. Preserve logical module/API boundaries when reorganizing physical paths; do not add wrapper modules or crates just to reduce directory counts.
- Keep persistent Instances and Accounts separate from transient LaunchPlans. Launch settings belong directly to each Instance; do not restore a separate product Profile or settings identity. Preparation instructions are secret-free; bind current authentication only when producing a transient execution plan.
- Preserve the independent per-game supervisor's ownership of its accepted plan, game-directory lock and Java child. Closing the launcher must not terminate an accepted game; explicit Stop is a different operation. Microsoft authentication remains in the launcher. See [session supervision](docs/session-supervisor.md).

## Safety

- Never persist resolved token-bearing LaunchPlans, copy credentials between hosts, or put credentials in fixtures, logs or committed files. Use the native secure credential backend; an unavailable backend must fail explicitly rather than fall back to plaintext. See [authentication](crates/block-auth/README.md).
- Preserve user worlds, configuration and source installations. Ordinary registration saves references without copying or rewriting Minecraft data or importing credentials. Registered resources remain read-only; explicit Repair writes replacements in Block's cache. Instance deletion preserves game files and accepted sessions.
- Shared and Isolated policies select the actual game directory without moving data. Preserve explicit discovery evidence, the Unknown isolated warning and the same-Drawer policy choice, and canonical actual-directory locking. See [Instance behavior](README.md#run).
- Keep cache preparation state separate from managed game roots; relocate only explicitly owned Managed roots, preserving Registered sources and external game directories. Catalog schema 2 rejects unsupported documents without migration or reset. See [storage boundaries](docs/config-storage.md).

## Verification and evidence

- Use the [justfile](justfile) as the command source of truth. `just check` is the full formatting, type-check, Clippy and test sequence; use focused package tests while iterating. Preserve `--locked` for builds and checks unless intentionally updating dependencies.
- Select test hosts and verification depth by the changed contract, following [test-platform guidelines](docs/test-platforms.md). Reuse unaffected evidence; a documentation change or a focused native check does not require a fresh full-platform matrix.
- Follow the existing [lint policy](README.md#lint-policy); do not broadly suppress diagnostics to get a check passing.
- Report exactly which checks ran and which did not. Parsing or planning success does not prove graphical launch support; reaching a menu does not prove world gameplay, normal exit or relaunch. Offline recipes do not establish Microsoft authentication acceptance.
- Keep platform, CPU architecture, Minecraft version and Loader evidence explicit. Do not generalize a host result to other targets. Use [current status](docs/STATUS.md) for recorded compatibility limits, not assumptions.
- Tests should protect observable contracts against credible regressions. Extend existing coverage before duplicating it, exercise the real implementation rather than a copy, and confirm regression tests fail before the fix when practical. Prefer established dependency boundaries over test-only production interfaces.
- For UI changes, verify the actual native behavior affected. Use semantic theme roles rather than borrowing visually similar tokens; fix layout or font-metric causes rather than hiding clipping with cosmetic offsets. Record any native verification that could not be performed.
- Run native UI verification on Linux ARM64. Another host's build, tests or screenshots do not replace that UI evidence.
- Shared text break rules live in the maintained [GPUI patch](vendor/gpui-pre/PATCHES.md). Verify its owner tests and Linux native shaping; preserve localized bytes rather than hiding punctuation defects in copy or font changes.
- Reused QA scenes can retain old executable hardlinks. Match the running executable's hash to the intended build; source transfers must not leave timestamps that let Cargo reuse an obsolete binary.

## Maintaining these instructions

- Keep this file for project-wide, non-obvious constraints. Put crate- or feature-specific discoveries in an `AGENTS.md` near the owning code when needed.
- Record stable causes, boundaries, coupled changes and verification pitfalls in 1–3 lines per insight. Do not add session logs, speculative rules, standard Rust advice or copies of existing documentation.
- Link to the owning documentation or command instead of maintaining a second source of truth. Update instructions when the underlying behavior changes.
