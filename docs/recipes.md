# Launch recipes and execution plans

The offline MVP has three independent boundaries:

1. A `RecipeParser` resolves a version source into typed metadata. Mojang version IDs and local official-format manifest files have separate parsers.
2. The pure plan compiler combines metadata, platform rules, runtime selection, and recipe options into a serializable `ExecutionPlan`. It contains resource downloads, preparation actions, and a complete Java command template. It contains no account token.
3. The executor downloads verified resources, prepares an isolated instance, binds a `SessionProvider`, and executes the resulting `JavaCommand` directly through the process API. It does not invoke a shell or reinterpret the recipe.

OAuth will supply a different `SessionProvider` at the final binding boundary. Offline profiles currently use the standard Java offline UUID derived from `OfflinePlayer:<username>` and do not grant authenticated multiplayer access. Account mode is independent of whether resource downloads are allowed.

## Quick start

The native UI starts with `just run`: choose a version, press Enter or click **Launch offline**, and use **Stop** to stop the running instance. The initial profile is `Player`. **Open recipe…** loads an explicit JSON configuration, including its username and instance. The interface shows preparation progress, game output, and the Java program, working directory, and argument array with secret fields redacted. You can also open and start a recipe directly:

```sh
cargo run --locked -p block-app -- --recipe examples/vanilla-offline.json --launch
```

`BLOCK_DATA_DIR` selects the native UI's external data directory when a custom location is needed. Recipe-relative manifest/Java paths resolve against the recipe document's directory, consistently in the UI and CLI.

The current native interface uses online metadata resolution and downloads. Use the CLI for an explicit cache-only run or to inspect/save an execution plan.

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

`plan` can fetch source, asset-index, and runtime metadata to discover the resource graph; it does not download the full game payload. `prepare` executes preparation without starting the game. `launch` resolves, prepares, prints the command preview, then supervises the game. `execute` consumes a saved plan without resolving its recipe again. A saved plan is portable across data directories for its target, but an explicit local Java path must exist on the execution machine.

Ctrl+C stops preparation or the running Java process. **Stop**, Ctrl+C, and the optional `--run-for-seconds` limit terminate the process; use Minecraft's own quit flow when you need a clean world save.

## Recipe fields

See the JSON files in `examples/` for complete input. Schema version 1 supports:

| Field | Meaning |
| --- | --- |
| `schema_version` | Required recipe format version, currently `1` |
| `instance` | Stable ASCII instance ID; each instance gets a separate game directory and exclusive run/preparation lock |
| `source` | `mojang` with `version`, or `local_manifest` with `path` |
| `session` | `offline` with a validated Java player `username` |
| `runtime` | `managed` (default), or `local` with the path to a matching Java executable |
| `target` | Optional explicit `os`, `arch`, and `os_version` for deterministic cross-target planning |
| `memory_mib` | Heap limit; defaults to 2048 MiB |
| `jvm_args` | Additional argument-array entries; each entry remains a single process argument |
| `features` | Explicit boolean launch capabilities used by manifest rules |
| `compatibility` | `official` (default), `linux_arm64_lwjgl3`, or `macos_jna_5_13`; compatibility profiles explicitly transform selected library resources |

The compiler preserves library/classpath and conditional argument order, selects only the target's native classifiers, resolves both legacy and modern argument generations, and constructs legacy virtual/resources asset views. Managed runtime manifests provide file trees, executable flags, and scoped links. Runtime probes require the Java major selected by the version metadata and the target CPU architecture. A plan targeting a different execution OS/architecture can be inspected but is rejected by the local executor. CPU emulation is not configured automatically.

`local_manifest` accepts a complete official-format version document. Mod-loader inheritance, custom installers, and version patches do not have parsers in this MVP. Treat recipes and saved plans as executable configuration: they select downloaded Java code, local executables, and JVM arguments.

## macOS ARM64 native support

Native architecture eligibility is a shared policy in the resolver, plan compiler, and saved-plan executor. Each selected LWJGL module/version must have a selected `natives-macos-arm64` counterpart, including LWJGL core and its GLFW or SDL window backend. Legacy LWJGL 2 and x64-only native metadata are rejected before game resources, runtimes, or instance files are prepared. A saved plan is checked again at execution.

This is an artifact-based support wall, not a version-number guess. Native artifact availability is necessary but does not establish compatibility of arbitrary mods. There is no automatic Rosetta, CPU emulation, or replacement-native fallback. Windows is the primary target for legacy Minecraft and legacy loaders. A separately reviewed native loader profile must still pass the same policy.

## Linux ARM64

The official Java catalog does not supply Linux ARM64 runtimes, and the supported version manifests select x64 LWJGL 3 Linux classifiers. `examples/linux-arm64-offline.json` uses a separately installed Java 21 and matching LWJGL 3 ARM64 classifier artifacts from Maven Central. The transformation preserves artifact versions.

```sh
cargo run --locked -p block-cli --bin block-launch -- launch \
  --recipe examples/linux-arm64-offline.json
```

Adjust the local Java path for the machine. This compatibility profile deliberately rejects versions without replaceable LWJGL 3 classifiers. Legacy LWJGL 2 ARM64 and further native-library compatibility require additional tested recipes. Missing managed Java combinations produce an explicit error instead of silently selecting an incompatible JDK.

## macOS and older JNA libraries

Some older versions, including Minecraft 1.20.1, ship JNA releases that can abort during native-library loading on modern macOS. An explicit compatibility recipe replaces the matching JNA core/platform pair with 5.13.0:

```sh
just launch examples/macos-arm64-1.20.1-offline.json
```

The profile accepts affected JNA versions from 5.8.0 through 5.12.1, requires macOS, and refuses mixed or newer pairs. It preserves the original version manifest and records the replacement resources and filenames in the execution plan. The default `official` profile uses the original libraries. See the [upstream JNA fix](https://github.com/java-native-access/jna/blob/5.13.0/CHANGES.md) and [Minecraft launcher compatibility issue](https://github.com/PrismLauncher/PrismLauncher/issues/3446).

## Tests and CI

`just check` retains the strict workspace lint policy. Ordinary offline integration tests cover the full 919-version source corpus, 58 real asset indexes, 29 Java runtime manifests, and deterministic plan/command compilation for Windows x64, Linux x64, and macOS x64. Plan compilation proves metadata handling, not a successful graphical game launch.

Local fake HTTP tests cover cache reuse, same-size corruption, concurrent requests, conflicting resource sizes, cancellation, retry policy, permanent HTTP errors, and path validation. The GitHub Actions workflow tests the core on native Windows, Linux, and macOS runners and separately checks each desktop build. Actual graphical launch evidence is kept outside the repository.
