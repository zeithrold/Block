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
The UI, Linux PNG and Windows ICO continue to use the original 256 px source.
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
shortcut, provenance and license notices. The ICO wraps the existing PNG
without changing it. It is supplied separately; this script does not inject
Windows PE icon/version resources into an already compiled executable.
The package is not an installer and does not provide an Authenticode signature.

## License status

The repository currently has no project-wide `LICENSE`. The script preserves
the supplied UI dependency and asset notices without labeling the entire
product as Apache-2.0, MIT, ISC or OFL. The package contains:

| License file under `licenses/` | Source |
| --- | --- |
| `GPUI-LICENSE-APACHE` | Existing TextInput adaptation and the gpui-pre 0.3.9 Zed family; the original adaptation notice remains intact |
| `GPUI-BASE-LICENSE-APACHE` | gpui-base 0.8.0, including its separate Longbridge copyright |
| `Inter-OFL.txt` | Bundled Inter fonts |
| `Lucide-LICENSE` | Bundled Lucide icons, including inherited Feather MIT notices |
| `SOURCES.md` | Explicit supplied UI dependency/asset versions and license source paths |

[License sources](licenses/SOURCES.md) are repository-local, so packaging needs
no Cargo registry, prototype checkout or Node tools. The obsolete xattr patch
is not in the resolved dependency graph and is no longer a packaging input.
`THIRD_PARTY_NOTICES.txt` describes the notices' scope. A project license
decision and complete transitive dependency notice inventory remain release
prerequisites. This wiring change does not claim a new package/build smoke
has passed; the upgraded binary and package verification remain pending.
