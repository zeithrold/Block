set positional-arguments

# List available commands.
default:
    @just --list

# Start the desktop application.
run *args:
    cargo run --locked --package block-app --bin block -- "$@"

# Compile a launch recipe into an inspectable execution plan.
plan recipe="examples/vanilla-offline.json":
    cargo run --locked --package block-cli --bin block-launch -- plan --recipe "$1"

# Prepare and run an offline game instance from a recipe.
launch recipe="examples/vanilla-offline.json":
    cargo run --locked --package block-cli --bin block-launch -- launch --recipe "$1"

# Build every workspace target.
build:
    cargo build --locked --workspace --all-targets

# Format the workspace.
fmt:
    cargo fmt --all

# Verify formatting without changing files.
fmt-check:
    cargo fmt --all -- --check

# Type-check every workspace target and feature.
check-code:
    cargo check --locked --workspace --all-targets --all-features

# Run strict Clippy checks; any warning fails the command.
clippy:
    cargo clippy --locked --workspace --all-targets --all-features -- -D warnings

# Run workspace tests and documentation tests.
test:
    cargo test --locked --workspace --all-features

# Download the current official Minecraft indexes and version metadata.
manifest-fetch:
    cargo run --locked --package block-cli --bin manifest-audit -- fetch

# Recreate the recorded experiment using its pinned official index snapshot.
manifest-fetch-snapshot:
    cargo run --locked --package block-cli --bin manifest-audit -- fetch-snapshot

# Run the complete official corpus through Cargo integration tests, offline.
manifest-verify:
    cargo test --locked --package block-launcher --test official_manifests

# Run the complete verification suite in order.
check:
    just fmt-check
    just check-code
    just clippy
    just test
