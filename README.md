# Block

A minimal desktop application built with Rust and [GPUI](https://gpui.rs/).

## Run on macOS

Install stable Rust and full Xcode, including the macOS SDK and Metal Toolchain.

Open Xcode once to complete its initial setup. If the Metal Toolchain is missing, run:

```sh
xcodebuild -downloadComponent MetalToolchain
```

Verify `xcrun metal --version`, then start the application:

```sh
cargo run
```

The application opens a window titled `Block` displaying `Hello, GPUI!`. Close the window or press `Cmd+Q` to quit.

The first build downloads and compiles GPUI dependencies. Subsequent builds reuse Cargo's cache.

## Checks

```sh
cargo fmt --check
cargo check --locked
```

The entry point is `src/main.rs`. Dependencies are declared in `Cargo.toml` and resolved versions are pinned in `Cargo.lock`.
