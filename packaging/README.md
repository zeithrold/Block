# Desktop packages

`scripts/package-desktop.py` packages an existing native binary using Python
3.11+ and its standard library. It never builds, installs, signs, notarizes,
launches, or changes application source. Product identity is maintained in
`product.json`: **one.ztd.block**, [block.ztd.one](https://block.ztd.one).

Build the executable first, then choose a new output directory:

```sh
cargo build --locked --package block-app --bin block --release
python3 scripts/package-desktop.py --binary target/release/block --output-dir /private/tmp/block-package-example
```

The default binary is `target/debug/block` (`block.exe` on Windows). The
script reads the target and architecture from the executable header rather
than relabeling the host binary. `--platform` and `--arch` can assert the
expected target. Version defaults to `workspace.package.version` in Cargo.toml.
Existing output artifacts are never overwritten. Use a fresh output directory
for each run.

The generated manifest records the binary hash and each packaged file's hash,
mode and size. Archives use sorted members, fixed modes and fixed timestamps;
the default timestamp is zero, overridable with `SOURCE_DATE_EPOCH`. Identical
binary, metadata, icon-conversion tools and epoch produce identical archives.
This does not make separately compiled binaries reproducible.

## macOS

Produces `Block.app` and `Block-<version>-macos-<arch>.zip`. `Info.plist` includes
the bundle identifier, executable, version, games category and website metadata.
The original 1024 px PNG from Block-Icon (`icon-v4-1024.png`, copied unchanged
to `crates/block-ui/assets/block-1024.png`) is converted to an ICNS with Apple's
`sips` and `iconutil`. All representations are downscaled from that source;
none are enlarged from the UI's 256 px image. If `iconutil` rejects the iconset,
the script wraps the existing PNG representations in standard ICNS chunks,
preserving the original 1024 px PNG bytes. This changes only the container;
native `sips` still performs all downscaling.
The UI and Linux PNG continue to use the original 256 px source.
Both native PNG sources and the SVG have individual SHA-256 provenance receipts.
No image-generation tool or package installation is needed.

The executable is copied byte for byte, including any toolchain-applied ad hoc
signature. The package does not provide a Developer ID signature, notarization,
or a signed installer. Packaging metadata makes no signing claim. It does not
change the launcher's existing application-data location.

Check a generated app without modifying a user installation:

```sh
plutil -lint /private/tmp/block-package-example/Block.app/Contents/Info.plist
/private/tmp/block-package-example/Block.app/Contents/MacOS/block --help
```

## Linux

Produces a portable prefix tree and `.tar.gz`, containing `bin/block`,
`share/applications/one.ztd.block.desktop`, scalable and PNG icons, and notices
under `share/doc/one.ztd.block`. The desktop entry expects `block` on `PATH`.
Install the prefix into a user-selected location using the normal package or
desktop integration mechanism; this script does not modify desktop settings.
Shared-library dependencies must already be present on the target host.
`StartupWMClass` declares the intended identity; runtime Wayland/X11 identity
still depends on the application and compositor.

## Windows

Produces a portable directory and ZIP with `block.exe`, `Block.ico`, website
shortcut, provenance and license notices. `crates/block-app/build.rs` uses
[`winresource`](https://github.com/BenjaminRi/winresource) to compile the icon
into the PE at build time for Windows targets, including AMD64 and ARM64.
Both debug and release builds use `packaging/windows/Block.ico`; resource
compilation errors fail the build. Non-Windows targets skip resource compilation.
Windows builds require the Windows SDK resource compiler; native MSVC builds
use the SDK installed with the C++ build tools.

The ICO contains unchanged 16, 32, 64, 128 and 256 px PNG representations from
Block-Icon's v4 assets. Their source hashes and the ICO hash are recorded in
`windows/icon-provenance.json`, included in the package. The separate ICO and
embedded PE icon use the same asset. Packaging copies the existing executable
byte for byte and does not rewrite its resources.
The package is not an installer and does not provide an Authenticode signature.

Native verification (2026-10-24): Windows AMD64 debug and Windows ARM64 release
builds passed strict Clippy and the `--build-info` startup check. Reading the
resulting PE resources found group icon 1 with all five PNG representations;
each embedded payload matched its source hash. The AMD64 portable package smoke
also passed, preserving the executable's hash. Receipts are under
`target/windows-pe-icon/`. The existing ARM64 interactive review window remained
running while its release build was verified.

## License status

Block is licensed under the [BSD 3-Clause License](../LICENSE). Every package
includes the project's `LICENSE`, alongside the supplied third-party dependency
and asset notices. Third-party components retain their respective licenses.
The package contains:

| License file | Source |
| --- | --- |
| `LICENSE` | Block project, BSD-3-Clause |
| `licenses/GPUI-LICENSE-APACHE` | Existing TextInput adaptation and the gpui-pre 0.3.9 Zed family; the original adaptation notice remains intact |
| `licenses/GPUI-BASE-LICENSE-APACHE` | gpui-base 0.8.0, including its separate Longbridge copyright |
| `licenses/Inter-OFL.txt` | Bundled Inter fonts |
| `licenses/Lucide-LICENSE` | Bundled Lucide icons, including inherited Feather MIT notices |
| `licenses/SOURCES.md` | Explicit supplied UI dependency/asset versions and license source paths |

[License sources](licenses/SOURCES.md) are repository-local, so packaging needs
no Cargo registry, prototype checkout or Node tools. The obsolete xattr patch
is not in the resolved dependency graph and is no longer a packaging input.
`THIRD_PARTY_NOTICES.txt` describes the notices' scope. The complete transitive
dependency notice inventory remains a release prerequisite. The scoped Windows
checks above do not establish complete
packaging validation or distribution readiness across every platform.
