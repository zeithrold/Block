# Block

A minimal desktop application built with Rust and [GPUI](https://gpui.rs/).

The launcher MVP compiles declarative launch recipes into verified resource plans and complete Java commands. It supports offline player profiles while keeping authentication behind a replaceable session interface. See [launch recipes](docs/recipes.md) for CLI usage and platform compatibility.

## Prerequisites

- Stable Rust with the `rustfmt` and `clippy` components.
- [just](https://just.systems/) to run project commands.
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

The application opens a window titled `Block` displaying `Hello, GPUI!`. Close the window or press `Cmd+Q` to quit. `cargo run --locked` also works from the workspace root.

The first build downloads and compiles GPUI dependencies. Subsequent builds reuse Cargo's cache.

## Workspace layout

```text
.
├── Cargo.toml              # Workspace members, shared dependencies, and lint policy
├── Cargo.lock              # Shared dependency lockfile
├── justfile                # Development and verification commands
└── crates/
    ├── block-app/          # Desktop entry point and window lifecycle
    │   └── src/main.rs
    ├── block-ui/           # GPUI views and presentation
    │   └── src/lib.rs
    └── minecraft-manifest/ # Official Minecraft metadata parsing experiment
        ├── src/model.rs
        ├── src/bin/manifest-audit.rs
        └── tests/
            ├── official_manifests.rs
            └── fixtures/official/ # Complete captured corpus, available offline
```

`block-app` depends on `block-ui`. Both crates inherit package metadata, GPUI versions, and lint settings from the root manifest. Keep application startup in `block-app` and reusable views in `block-ui`.

## Development commands

| Command | Purpose |
| --- | --- |
| `just` | List available commands |
| `just run` | Start the desktop application |
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
