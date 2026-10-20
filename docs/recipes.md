# Launch recipes and execution plans

The offline MVP has three independent boundaries:

1. A `RecipeParser` resolves a version source into typed metadata. Mojang version IDs and local official-format manifest files have separate parsers.
2. The pure plan compiler combines metadata, platform rules, runtime selection, and recipe options into a serializable `ExecutionPlan`. It contains resource downloads, preparation actions, and a complete Java command template. It contains no account token.
3. The executor downloads verified resources, prepares an isolated instance, binds a `SessionProvider`, and executes the resulting `JavaCommand` directly through the process API. It does not invoke a shell or reinterpret the recipe.

OAuth will supply a different `SessionProvider` at the final binding boundary. Offline profiles currently use the standard Java offline UUID derived from `OfflinePlayer:<username>` and do not grant authenticated multiplayer access. Account mode is independent of whether resource downloads are allowed.

## Quick start

```sh
cargo run --locked -p block-cli --bin block-launch -- launch --recipe examples/vanilla-offline.json
```

The default recipe prepares Minecraft 1.20.6 with a matching managed Java runtime and an offline `Player` identity. The platform target defaults to the current OS and CPU architecture. Downloaded objects, mutable instance data, and run logs live in the OS application-data directory, outside this checkout. Use `--data-dir /absolute/path` before or after the subcommand to choose another external directory.

```sh
# Inspect/save a plan before downloading game resources.
cargo run --locked -p block-cli --bin block-launch -- plan \
  --recipe examples/vanilla-offline.json --output /tmp/block-plan.json

# Execute that exact plan with identity supplied at the end.
cargo run --locked -p block-cli --bin block-launch -- execute \
  --plan /tmp/block-plan.json --username Player

# Reuse cached metadata and resources without network access.
cargo run --locked -p block-cli --bin block-launch -- --cache-only launch \
  --recipe examples/vanilla-offline.json
```

`plan` can fetch small source/asset/runtime metadata to discover the resource graph. `prepare` executes preparation without starting the game. `launch` resolves, prepares, prints the command preview, then supervises the game. `execute` consumes a saved plan without resolving its recipe again. Ctrl+C stops preparation or the running Java process. The optional `--run-for-seconds` limit force-stops a launch for explicit smoke checks; it is not a clean-save mechanism.

## Recipe fields

See the JSON files in `examples/` for complete input. Schema version 1 supports:

| Field | Meaning |
| --- | --- |
| `instance` | Stable ASCII instance ID; each instance gets a separate game directory and exclusive run/preparation lock |
| `source` | `mojang` with `version`, or `local_manifest` with `path` |
| `session` | `offline` with a validated Java player `username` |
| `runtime` | `managed` (default), or `local` with the path to a matching Java executable |
| `target` | Optional explicit `os`, `arch`, and `os_version` for deterministic cross-target planning |
| `memory_mib` | Heap limit; defaults to 2048 MiB |
| `jvm_args` | Additional argument-array entries; each entry remains a single process argument |
| `features` | Explicit boolean launch capabilities used by manifest rules |
| `compatibility` | `official` (default), or the explicit `linux_arm64_lwjgl3` resource transformation |

The compiler preserves library/classpath and conditional argument order, selects only the target's native classifiers, resolves both legacy and modern argument generations, and constructs legacy virtual/resources asset views. Managed runtime manifests provide file trees, executable flags, and scoped links. Runtime probes reject a mismatched Java major or CPU architecture. A plan targeting a different execution OS/architecture can be inspected but is rejected by the local executor.

## Linux ARM64

The official Java catalog does not supply Linux ARM64 runtimes, and Mojang's selected LWJGL 3 Linux classifiers are x64. `examples/linux-arm64-offline.json` demonstrates a reviewed compatibility recipe using a separately installed Java 21 and the matching LWJGL 3 ARM64 classifier artifacts from Maven Central. Versions/artifacts are preserved; this is not a blanket LWJGL upgrade.

```sh
cargo run --locked -p block-cli --bin block-launch -- launch \
  --recipe examples/linux-arm64-offline.json
```

Adjust the local Java path for the machine. This compatibility profile deliberately rejects versions without replaceable LWJGL 3 classifiers. Legacy LWJGL 2 ARM64 and further native-library compatibility require additional tested recipes. Missing managed Java combinations produce an explicit error instead of silently selecting an incompatible JDK.

## Tests and CI

`just check` retains the strict workspace lint policy. Ordinary offline integration tests cover the full 919-version source corpus, 58 real asset indexes, 29 Java runtime manifests, and deterministic plan/command compilation for Windows x64, Linux x64, and macOS x64. Plan compilation proves metadata handling, not a successful graphical game launch.

Local fake HTTP tests cover cache reuse, same-size corruption, concurrent requests, conflicting resource sizes, cancellation, retry policy, permanent HTTP errors, and path validation. The GitHub Actions workflow tests the core on native Windows, Linux, and macOS runners and separately checks each desktop build. Actual graphical launch evidence is kept outside the repository.
