# Working on Block

## Architecture and data boundaries

- Keep desktop and CLI launch behavior in the shared `block-launcher` core rather than implementing parallel rules in each interface. See [workspace architecture](README.md#workspace-layout).
- Keep persistent Instances, Profiles and Accounts separate from transient LaunchPlans. Preparation instructions are secret-free; bind current authentication only when producing a transient execution plan.
- Preserve the independent per-game supervisor's ownership of its accepted plan, game-directory lock and Java child. Closing the launcher must not terminate an accepted game; explicit Stop is a different operation. Microsoft authentication remains in the launcher. See [session supervision](docs/session-supervisor.md).

## Safety

- Never persist resolved token-bearing LaunchPlans, copy credentials between hosts, or put credentials in fixtures, logs or committed files. Use the native secure credential backend; an unavailable backend must fail explicitly rather than fall back to plaintext. See [authentication](crates/block-auth/README.md).
- Preserve user worlds, configuration and source installations. Repair restores declared immutable resources while preserving corrupt copies for diagnosis; imports copy games without modifying originals or importing credentials. Profile deletion preserves game files.
- Shared and Isolated Profile data policies are intentional. Do not change their copying or directory-lock behavior as an incidental cleanup. See [Profile behavior](README.md#run).

## Verification and evidence

- Use the [justfile](justfile) as the command source of truth. `just check` is the full formatting, type-check, Clippy and test sequence; use focused package tests while iterating. Preserve `--locked` for builds and checks unless intentionally updating dependencies.
- Follow the existing [lint policy](README.md#lint-policy); do not broadly suppress diagnostics to get a check passing.
- Report exactly which checks ran and which did not. Parsing or planning success does not prove graphical launch support; reaching a menu does not prove world gameplay, normal exit or relaunch. Offline recipes do not establish Microsoft authentication acceptance.
- Keep platform, CPU architecture, Minecraft version and Loader evidence explicit. Do not generalize a host result to other targets. Use [current status](docs/STATUS.md) for recorded compatibility limits, not assumptions.
- Tests should protect observable contracts against credible regressions. Extend existing coverage before duplicating it, exercise the real implementation rather than a copy, and confirm regression tests fail before the fix when practical. Prefer established dependency boundaries over test-only production interfaces.
- For UI changes, verify the actual native behavior affected. Use semantic theme roles rather than borrowing visually similar tokens; fix layout or font-metric causes rather than hiding clipping with cosmetic offsets. Record any native verification that could not be performed.

## Maintaining these instructions

- Keep this file for project-wide, non-obvious constraints. Put crate- or feature-specific discoveries in an `AGENTS.md` near the owning code when needed.
- Record stable causes, boundaries, coupled changes and verification pitfalls in 1–3 lines per insight. Do not add session logs, speculative rules, standard Rust advice or copies of existing documentation.
- Link to the owning documentation or command instead of maintaining a second source of truth. Update instructions when the underlying behavior changes.
