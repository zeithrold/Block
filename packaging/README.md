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
the supplied GPUI and xattr notices without labeling the entire product as
Apache-2.0 or MIT. `THIRD_PARTY_NOTICES.txt` describes their scope. A project
license decision and complete transitive dependency notice inventory remain
release prerequisites.
