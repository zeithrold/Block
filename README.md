# Block

A Minecraft Java Edition launcher MVP built with Rust and [GPUI](https://gpui.rs/), with a shared core for the native interface and command-line tool.

Block separates persistent Instances, launch Profiles and Microsoft Accounts from transient LaunchPlans. The native interface creates installations, manages multiple Profiles, installs Vanilla or the supported NeoForge representative, and supervises a verified Java invocation. Developer-offline Vanilla and NeoForge world/exit/relaunch workflows passed on Windows and Debian AMD64. Fresh Microsoft-authorized native Play remains pending; see [the acceptance ledger](docs/mvp-progress.md) for the complete platform results.

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

Sign in with Microsoft through the system browser, create an Instance, choose Minecraft and its supported Loader, then select a Profile and Account and press **Play**. Installation shows resource progress and the current task. **Repair** checks and restores declared immutable files while preserving corrupt copies for diagnosis; it does not remove worlds or configuration. **Stop** cancels preparation or terminates the owned game process.

Each Instance can have several Profiles. A Profile selects its launch target, Java, memory, additional arguments, default account and **Shared** or **Isolated** mutable game data. Copying a Profile copies settings and preserves its data policy: Isolated copies get a new directory; Shared copies continue sharing the Instance directory. Deleting a Profile keeps game files. Shared assets, libraries and managed Java remain in the content-addressed cache. A canonical game-directory lock prevents simultaneous writers.

Development builds need the application's public Microsoft client ID, either at build time or runtime:

```sh
MICROSOFT_CLIENT_ID="your-public-application-id" just run
```

Use a personal-account-capable Mobile/Desktop application registration with `http://localhost/callback`. No Client Secret is required or supported. Do not distribute a secret or copy credentials between hosts. Refresh tokens use macOS Keychain, Windows Credential Manager, or Linux Secret Service; an unavailable/locked secure backend fails explicitly. See [authentication setup](crates/block-auth/README.md).

Offline recipes remain an explicit development interface, separate from Microsoft authentication acceptance:

```sh
just run --recipe examples/vanilla-offline.json --launch
just plan examples/vanilla-offline.json
just launch examples/vanilla-offline.json
```

The CLI's `--cache-only` forbids network access once required metadata and resources are cached. See [launch recipes](docs/recipes.md) for saved secret-free preparation plans and compatibility settings. Normal Profile launches verify the installed closure locally; missing or damaged files have an explicit Repair entry. Resolved token-bearing LaunchPlans are never saved.

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
    ├── block-auth/         # Public Microsoft PKCE, ownership, native secure storage
    ├── block-cli/          # block-launch: plan, prepare, launch, execute
    ├── block-launcher/     # Recipes, resolution, planning, sessions, execution
    ├── block-download/     # Verified resource cache, retries, cancellation
    └── minecraft-manifest/ # Typed official metadata and offline corpus tests
```

Both interfaces reuse `block-launcher`. Source parsers and the pure compiler produce secret-free preparation instructions; the native library coordinates persistent installations and Profiles. After resource validation, a transient LaunchPlan binds a currently valid session and mutable-data directory. LaunchExecutor handles the actual Java process and redacted logs. Account credentials stay outside saved plans. Workspace crates inherit shared package metadata, dependencies, and lint settings.

## Compatibility

The core targets macOS, Windows, and Linux. Planning can target another OS and CPU architecture; execution requires a matching host and Java architecture. Available Java runtimes and native libraries vary by Minecraft version. Linux ARM64 uses the explicit LWJGL compatibility policy and automatically locates an installed matching Java for managed Profiles because the current official runtime catalog has no Linux ARM64 entry; legacy ARM64 native support and execution through CPU emulation are outside this MVP.

The captured 919-version corpus exercises parsing and planning across all three x64 OS families. It does not establish graphical launch support for every version and platform. The product NeoForge representative is Minecraft1.20.6 with NeoForge20.6.144. Other Loader generations remain external research results and are not advertised as product installers. Actual Microsoft login, world gameplay, normal exit and relaunch remain distinct acceptance gates; a menu or synthetic protocol test does not establish them.

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
