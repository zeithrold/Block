# Block

A Minecraft Java Edition launcher MVP built with Rust and [GPUI](https://gpui.rs/), with a shared core for the native interface and command-line tool.

The launcher MVP compiles declarative launch recipes into verified resource plans and complete Java commands. It supports offline player profiles while keeping authentication behind a replaceable session interface. See [launch recipes](docs/recipes.md) for CLI usage and platform compatibility.

## Prerequisites

- Stable Rust with the `rustfmt` and `clippy` components.
- [just](https://just.systems/) to run project commands.
- A native build toolchain for the desktop application. The [CI workflow](.github/workflows/check.yml) lists the Linux development packages used by this project. Windows builds use the MSVC toolchain and Windows SDK.
- On macOS, full Xcode with the macOS SDK and Metal Toolchain.

Open Xcode once to complete its initial setup. If the Metal Toolchain is missing:

```sh
xcodebuild -downloadComponent MetalToolchain
```

Verify `xcrun metal --version` before building. Install missing Rust components with:

```sh
rustup component add rustfmt clippy
```

## Run

```sh
just run
```

Choose a version and select **Launch offline**. The launcher downloads the client, libraries, assets, and a matching Java runtime, then starts an isolated instance. The interface shows progress, game logs, and a redacted Java invocation. **Stop** cancels preparation or stops the game process.

The initial profile is `Player`; load a recipe to change the username, instance, memory limit, or Java selection. To start a recipe directly:

```sh
just run --recipe examples/vanilla-offline.json --launch
```

The same pipeline is available without the desktop interface:

```sh
just plan examples/vanilla-offline.json
just launch examples/vanilla-offline.json
```

Offline identity and network access are separate: the first preparation still needs downloads. Microsoft OAuth is deferred. The CLI's `--cache-only` option forbids network access once the required metadata and resources are cached. See [launch recipes](docs/recipes.md) for saved plans, local Java, and Linux ARM64 configuration.

Application data, downloaded resources, and logs default to the OS application-data directory. Set `BLOCK_DATA_DIR` for the desktop app or pass the CLI's `--data-dir` option to select an external location.

The first build downloads and compiles GPUI dependencies. Subsequent builds reuse Cargo's cache.

## Workspace layout

```text
.
├── Cargo.toml              # Workspace members, shared dependencies, and lint policy
├── Cargo.lock              # Shared dependency lockfile
├── justfile                # Development and verification commands
└── crates/
    ├── block-app/          # Desktop entry point and window lifecycle
    ├── block-ui/           # GPUI interface and asynchronous core adapter
    ├── block-cli/          # block-launch: plan, prepare, launch, execute
    ├── block-launcher/     # Recipes, resolution, planning, sessions, execution
    ├── block-download/     # Verified resource cache, retries, cancellation
    └── minecraft-manifest/ # Typed official metadata and offline corpus tests
```

Both interfaces use `block-launcher`: source parsers resolve typed metadata, a pure compiler builds a serializable execution plan, and the executor prepares resources and binds a session to produce the final Java program and argument array. Account credentials stay outside saved plans. Workspace crates inherit shared package metadata, dependencies, and lint settings.

## Compatibility

The core targets macOS, Windows, and Linux. Planning can target another OS and CPU architecture; execution requires a matching host and Java architecture. Available Java runtimes and native libraries vary by Minecraft version. Linux ARM64 needs an explicit compatibility recipe and local Java; legacy ARM64 native support and execution through CPU emulation are outside this MVP.

The captured 919-version corpus exercises parsing and planning across all three x64 OS families. It does not establish graphical launch support for every version and platform. This MVP handles vanilla official-format manifests; mod loaders and Microsoft login need additional adapters.

## Development commands

| Command | Purpose |
| --- | --- |
| `just` | List available commands |
| `just run` | Start the desktop application |
| `just plan [recipe]` | Resolve a recipe and print its execution plan |
| `just launch [recipe]` | Prepare and launch an offline instance |
| `just build` | Build all workspace targets |
| `just fmt` | Format all workspace code |
| `just fmt-check` | Verify formatting |
| `just check-code` | Type-check all targets and features |
| `just clippy` | Run strict Clippy checks |
| `just test` | Run workspace tests, including the full official Minecraft corpus |
| `just check` | Run formatting, type checks, Clippy, and tests in order |
| `just manifest-fetch` | Download the current official Minecraft metadata |
| `just manifest-fetch-snapshot` | Download metadata from the recorded index snapshot |
| `just manifest-verify` | Run the full official-corpus Cargo integration tests offline |

Verification and build commands use `--locked` to prevent implicit dependency updates.

## Lint policy

The workspace enables Clippy's `all`, `pedantic`, and `nursery` groups. Additional restrictions reject `unwrap`, `expect`, explicit panics, `dbg!`, `todo!`, `unimplemented!`, and wildcard imports. Unsafe code is forbidden in workspace crates.

`just clippy` checks all workspace crates, targets, and features with `-D warnings`. Both Rust and Clippy warnings in project code fail the check. Third-party dependencies are not held to the workspace lint policy. Prefer fixing a diagnostic; if an exception is necessary, use a narrowly scoped `#[expect(..., reason = "...")]` with a concrete explanation.

## Minecraft manifest experiment

See [the experiment notes](experiments/minecraft-manifests/README.md) for the schema coverage, recorded full-corpus results, and reproduction steps. The parser is a separate workspace crate; the desktop application remains the default member.
