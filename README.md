# Block

A Minecraft Java Edition launcher MVP built with Rust and [GPUI](https://gpui.rs/), with a shared core for the native interface and command-line tool.

Block separates persistent Instances, launch Profiles and Accounts from transient LaunchPlans. The native interface manages games and accounts, installs Vanilla, Fabric, Quilt, Forge and NeoForge, imports user-provided official OptiFine JARs, and hands a verified Java invocation to an independent session supervisor. See [Pilot status](docs/STATUS.md) for current native evidence and compatibility limits, and [next tasks](docs/next-tasks.md) for the milestone plan. The [MVP acceptance record](docs/mvp-progress.md) preserves earlier baseline results.

Pilot Milestone II basic usability is complete at `f2a5fd7`, including the screenshot handoff and live Microsoft sign-in/avatar checks on five platforms. Milestone III visual and interaction review is next; user acceptance remains pending. Architecture design (Milestone IV) and the first public unstable release (v0) remain separate future stages.

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

On first launch, the 8:5 setup flow presents Your game → Your account → Ready to go, one decision per page. Name the game and choose its version; loader and folder options are revealed only on request. Microsoft sign-in opens the system browser and can be deferred; final confirmation creates the game and starts its real background download. Skip setup leaves the complete setup without creating a game. Block then opens its portrait Home. You can also sign in and create games later, choose Minecraft and its supported Loader, then select a Profile and Account and press **Play**. Installation shows resource progress and the current task. **Repair** checks and restores declared immutable files while preserving corrupt copies for diagnosis; it does not remove worlds or configuration. Closing Block leaves an accepted game session running; reopening Block reconnects to its supervisor. **Stop** explicitly cancels preparation or asks that supervisor to terminate its game. An abnormal exit or failed start opens local crash help in Block, or in a separate window if the launcher is closed. See [session supervision and crash help](docs/session-supervisor.md) for diagnostics, privacy limits, and interrupted-monitor recovery.

Each Instance can have several Profiles. A Profile selects its launch target, Java, memory, additional arguments, default account and **Shared** or **Isolated** mutable game data. Copying a Profile copies settings and preserves its data policy: Isolated copies get a new directory; Shared copies continue sharing the Instance directory. Deleting a Profile keeps game files. Shared assets, libraries and managed Java remain in the content-addressed cache. A canonical game-directory lock prevents simultaneous writers.

Development builds need the application's public Microsoft client ID, either at build time or runtime:

```sh
MICROSOFT_CLIENT_ID="your-public-application-id" just run
```

Use a personal-account-capable Mobile/Desktop application registration with `http://localhost/block/callback`. No Client Secret is required or supported. Do not distribute a secret or copy credentials between hosts. Refresh tokens use macOS Keychain, Windows Credential Manager, or Linux Secret Service; an unavailable/locked secure backend fails explicitly. See [authentication setup](crates/block-auth/README.md).

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
    ├── block-app/          # Launcher, headless session supervisor, crash-help entry points
    ├── block-ui/           # GPUI launcher, crash help, and asynchronous core adapter
    ├── block-auth/         # Public Microsoft PKCE, ownership, native secure storage
    ├── block-cli/          # block-launch: plan, prepare, launch, execute
    ├── block-launcher/     # Recipes, planning, independent sessions, execution, diagnostics
    ├── block-download/     # Verified resource cache, retries, cancellation
    └── minecraft-manifest/ # Typed official metadata and offline corpus tests
```

Both interfaces reuse `block-launcher`. Source parsers and the pure compiler produce secret-free preparation instructions; the native library coordinates persistent installations and Profiles. After resource validation, a transient LaunchPlan binds a currently valid session and mutable-data directory. LaunchExecutor handles the actual Java process and redacted logs. For native Profile launches, an independent per-game supervisor owns that plan, its locks, and its Java child; Microsoft authentication stays in the launcher. Account credentials stay outside saved plans. Workspace crates inherit shared package metadata, dependencies, and lint settings.

## Compatibility

The launcher targets macOS ARM64, Windows AMD64/ARM64 and Linux AMD64/ARM64. Planning can target another OS and CPU architecture; execution requires a matching host and Java architecture. Available Java runtimes and native libraries vary by Minecraft version. Linux ARM64 uses explicit official LWJGL substitutions and provisions official Azul runtimes for supported legacy versions. Windows ARM64 also uses official LWJGL substitutions; launcher installation/account/restart flows passed, while game rendering on the test VM is blocked by its missing OpenGL ICD. macOS ARM64 1.16.5 has a recorded GLFW window-icon failure. LWJGL 2-era versions are disabled on ARM64 because the official native binaries are x86/x64 only. See [the platform and legacy compatibility tables](docs/STATUS.md) for tested combinations and remaining limits.

The captured 919-version corpus exercises parsing and planning across all three x64 OS families. It does not establish graphical launch support for every version and platform. Native Pilot checks cover modern and older loader representatives, including legacy Forge and official OptiFine imports; they do not promise every version/loader combination. Actual Microsoft login, world gameplay, normal exit and relaunch remain distinct evidence; a menu or synthetic protocol test does not establish them.

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

Development stages: Pilot Milestone II (basic usability, DeepSeek), III (interaction fixes, Codex), IV (architecture design), then v0 (the first public unstable release). See [the milestone plan](docs/next-tasks.md) and [current status](docs/STATUS.md). Desktop packaging and the `one.ztd.block` product identity: [packaging](packaging/README.md).
