# Linux glibc baseline: investigation and options

Date: 2026-10-25. Status: research report. No packaging change has been
applied yet. Related: [pilot builds](pilot-builds.md),
[packaging README](../packaging/README.md).

## Summary

The Linux pilot packages built on `ubuntu-24.04` / `ubuntu-24.04-arm` require
**glibc 2.39** on both x86_64 and aarch64. That limits the packages to very
recent distributions (Ubuntu 24.04+, Debian 13+, Fedora 40+, RHEL 10+).

Three ways to lower the baseline were investigated:

1. **Build on an older GitHub-hosted runner** (`ubuntu-22.04`,
   `ubuntu-22.04-arm`): no container, two-line workflow change, expected
   baseline **glibc 2.35**.
2. **Build in an older base container** (`ubuntu:20.04`): verified locally to
   produce a working release binary with **glibc 2.30**.
3. **`cargo-zigbuild` with a glibc target suffix** (`…-gnu.2.17`): documented
   to support older glibc without a container, but the full GUI build fails at
   link time against the runner's newer system libraries. It cannot lower the
   baseline without also providing an older sysroot.

Recommendation: for a quick win choose option 1 (2.35). For wider coverage with
a small CI change choose option 2 (2.30). `cargo-zigbuild` alone is not
sufficient for this GUI stack.

## Current baseline (measured)

Artifacts from Pilot packages run `40112948723` (commit `1878a3e`, built on
`ubuntu-24.04` / `ubuntu-24.04-arm`, CI release profile):

| Symbol version | Examples | Binding |
| --- | --- | --- |
| `GLIBC_2.30` | `gettid` | weak |
| `GLIBC_2.32` | `pthread_getattr_np` | hard |
| `GLIBC_2.33` | `stat`, `stat64`, `fstat*`, `lstat*` | hard |
| `GLIBC_2.34` | `dlopen`/`dlsym`, `pthread_create`/`join`, `__libc_start_main` | hard |
| `GLIBC_2.35` | `hypot`, `hypotf` | hard |
| `GLIBC_2.38` | `__isoc23_strtol`, `__isoc23_strtoul` | hard |
| `GLIBC_2.39` | `pidfd_spawnp`, `pidfd_getpid` | weak on amd64, **hard on arm64** |

The 2.38/2.39 requirements do not come from the application code choice alone:
`__isoc23_*` and the 2.33 `stat*` versions come from the Ubuntu 24.04
headers/libraries used during the build, and the 2.39 pidfd helpers are Rust
std probes (`weak!`) that the linker resolves against the build host's libc.

The practical minimum is therefore the build environment's glibc, not a value
the project selects in advance.

## Option 1 — older GitHub-hosted runner (no container)

Runner images currently available for public repositories (checked against the
`actions/runner-images` table):

| Label | Architecture |
| --- | --- |
| `ubuntu-22.04` | x86_64 |
| `ubuntu-22.04-arm` | arm64 |

Building on 22.04 replaces the 24.04 system libraries, so the resulting
requirement is expected to be **glibc 2.35** (the 2.35 `hypot` and 2.36+
symbols disappear; the exact value must be re-measured on the first CI run).

- Coverage: Ubuntu 22.04+, Debian 12+, Fedora 36+ (`glibc 2.35`).
  RHEL 9 (`glibc 2.34`) remains unsupported.
- Cost: change `runs-on` for the two Linux jobs; no new tools, no containers.
- Risk: low. Only the build image changes; packaging and verify steps stay the
  same.
- Not yet measured on CI.

## Option 2 — Ubuntu 20.04 container (verified locally)

A local Docker build on arm64 (`ubuntu:20.04`, Rust 1.99.1 installed via
rustup, the same desktop dependencies as the workflow) succeeded in 4 min 16 s
and produced a 36 MB stripped release binary with:

| Symbol version | Examples |
| --- | --- |
| `GLIBC_2.25` | `getrandom` |
| `GLIBC_2.27` | `copy_file_range` |
| `GLIBC_2.28` | `statx` |
| `GLIBC_2.29` | `exp`/`log`/`pow`, `posix_spawn_file_actions_addchdir_np` |
| `GLIBC_2.30` | `gettid` |

Maximum hard requirement: **glibc 2.30**.

- Coverage: Ubuntu 20.04+, Debian 11+, Fedora 31+, RHEL 9+, openSUSE Leap
  15.3+. RHEL 8 / Debian 10 (`glibc 2.28`) would still need a separate build.
- Cost: a container job (or a `docker run` build step) for the Linux jobs;
  apt packages and Rust come from inside the container, so the job no longer
  uses the runner's installed toolchain for the build.
- Risk: medium. Verified locally for arm64 only; amd64 and the CI container
  plumbing still need a first run. Build time and image pulls add CI cost.
- Reproducible command (arm64 example):
  ```sh
  docker run --rm -v "$PWD:/src" -w /src ubuntu:20.04 bash -lc '
    apt-get update && apt-get install -y curl ca-certificates build-essential \
      clang pkg-config git libasound2-dev libxkbcommon-x11-dev libwayland-dev \
      libx11-xcb-dev libxcb-shape0-dev libxcb-xfixes0-dev libfontconfig1-dev \
      libfreetype6-dev libvulkan-dev libegl1-mesa-dev libgl1-mesa-dev
    curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal
    . "$HOME/.cargo/env"
    cargo build --locked --release -p block-app --bin block
  '
  ```

## Option 3 — cargo-zigbuild with a glibc suffix (does not work alone)

`cargo-zigbuild` supports versioned glibc targets, for example
`cargo zigbuild --release --target aarch64-unknown-linux-gnu.2.17`
(see the [cargo-zigbuild README](https://github.com/rust-cross/cargo-zigbuild)).
This is the standard non-container approach for lowering the glibc requirement
of a Rust binary.

However, running it on an Ubuntu 24.04 host against this workspace failed while
linking the final executable:

```text
ld.lld: error: undefined reference: __isoc23_strtoul@GLIBC_2.38
  >>> referenced by /usr/lib/aarch64-linux-gnu/libxkbcommon.so
ld.lld: error: undefined reference: stat64@GLIBC_2.33
  >>> referenced by /usr/lib/aarch64-linux-gnu/libxkbcommon.so
```

The cause is that the GUI stack links against the build host's shared libraries
(`libxkbcommon`, Wayland, EGL, …). Those libraries are themselves built against
newer glibc, and Zig's glibc 2.17 stubs cannot satisfy their versioned
references. `cargo-zigbuild` only controls the glibc used for the Rust/C
objects it links; it does not replace the system `.so` files.

Workarounds and their limits:

- **Old sysroot**: supply Ubuntu 20.04/18.04 headers and shared libraries via a
  cross sysroot (`PKG_CONFIG_SYSROOT_DIR`, `-L`, `--sysroot`). This is
  effectively a hand-built container and much more fragile; not recommended.
- **`--allow-shlib-undefined`**: lets the link succeed but the binary still
  loads the runner's newer libraries at runtime, so it does not actually lower
  the practical requirement.
- **Static glibc**: unsupported by `zig cc`; musl would break the dlopen-based
  X11/Wayland/EGL/Vulkan stack.

Conclusion: `cargo-zigbuild` is not a viable shortcut for this project without
an older sysroot. If a sysroot is acceptable, building inside the matching
distribution container is simpler and already proven.

## Rejected approaches

- **Patching version requirements after the fact** (`patchelf`, `objcopy`,
  symbol-version rewriting): unsafe, can silently bind incompatible symbols, and
  is not supported for release packages.
- **Static musl build**: GPUI's Linux backends load X11/Wayland/EGL/Vulkan at
  runtime; a musl-static build would not remove that dependency and is not
  tested for this application.
- **Shipping a private glibc**: licensing/maintenance burden and ABI hazards;
  not appropriate for the pilot.

## Coverage comparison

| Build baseline | Ubuntu | Debian | Fedora | RHEL | Notes |
| --- | --- | --- | --- | --- | --- |
| 24.04 (current) | 24.04+ | 13+ | 40+ | 10+ | current pilot |
| 22.04 runner | 22.04+ | 12+ | 36+ | — | simplest lowering |
| 20.04 container | 20.04+ | 11+ | 31+ | 9+ | verified locally |
| 18.04 container (untested) | 18.04+ | 10+ | 28+ | 8+ | would need Rust 1.99 + deps check |

These tables describe glibc only. The GUI also needs a working X11 or Wayland
session with EGL/GL/Vulkan, fontconfig/freetype, ALSA and xkbcommon; lowering
glibc does not bundle those libraries, and minimum-distro desktop testing has
not been performed.

## Recommendation

1. If the goal is "a bit older without CI changes": switch the Linux pilot jobs
   to `ubuntu-22.04` / `ubuntu-22.04-arm` and re-measure (expected 2.35).
2. If RHEL 9 / Debian 11 / Ubuntu 20.04 support matters: adopt the verified
   Ubuntu 20.04 container build for the Linux jobs (expected 2.30). Validate
   amd64 first, then arm64.
3. Keep `cargo-zigbuild` out of the release path unless an older sysroot is
   maintained deliberately.
4. Whatever is chosen, record the measured baseline and add it to the package
   documentation and (eventually) a `LSB`/requirements note on the download
   page.

## Reproduction: measuring the baseline

`readelf -W --version-info --dyn-syms <binary>` and the helper script from the
cargo-zigbuild README; on macOS without GNU binutils, a small ELF parser can
read `.gnu.version_r`/`.gnu.version` from both CI artifacts and local builds.
The measurements in this report used the CI artifacts from run `40112948723`
and a local Ubuntu 20.04 arm64 container build.
