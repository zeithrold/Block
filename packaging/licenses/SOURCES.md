# Supplied UI license sources

These notices cover the explicitly supplied UI dependencies and assets below.
They do not assign a license to Block or claim a complete transitive dependency
audit. Package filenames refer to the surrounding `licenses/` directory.

| Packaged file | Covered source | Repository source and provenance |
| --- | --- | --- |
| `GPUI-LICENSE-APACHE` | Existing Block TextInput adaptation from Zed GPUI examples; GPUI pre 0.3.9 core and supplied split platform/Apple family | `crates/block-ui/GPUI-LICENSE-APACHE`; official [gpui-pre 0.3.9 license](https://docs.rs/crate/gpui-pre/0.3.9/source/LICENSE-APACHE). |
| `GPUI-BASE-LICENSE-APACHE` | gpui-base 0.8.0, Copyright 2024–2026 Longbridge | `packaging/licenses/GPUI-BASE-LICENSE-APACHE`, copied byte for byte from the official [gpui-base 0.8.0 license](https://docs.rs/crate/gpui-base/0.8.0/source/LICENSE-APACHE). |
| `Inter-OFL.txt` | Inter font, SIL Open Font License 1.1 | `crates/block-ui/assets/fonts/OFL.txt`. The adjacent repository README records @fontsource-variable/inter 5.3.0 and the historical WOFF2 → native TTF conversion; no conversion tool or sibling prototype is read by the build. [Inter project](https://rsms.me/inter/). |
| `Lucide-LICENSE` | Lucide ISC and inherited Feather MIT notices | `crates/block-ui/assets/icons/LICENSE`. The adjacent repository README records extraction from lucide-react 1.59.0; the supplied complete license retains the Feather notice. [Lucide license](https://lucide.dev/license). |

The GPUI pre family uses Apache-2.0. The explicitly used UI crates are
`gpui-pre`, `gpui-pre-platform`, macOS text crate `gpui-pre-apple`, and the
platform selected by Cargo (`gpui-pre-macos`, `gpui-pre-linux`,
`gpui-pre-windows`, or its upstream web/wgpu split). Listing a platform does not
claim its build or GUI has been verified locally. The one Zed license file
above is shared because these supplied 0.3.9 license texts have identical bytes;
Base's Longbridge copyright text is distinct and is preserved separately.

The old xattr patch is absent from the current resolved dependency graph.
Packages no longer read files from that retired source.
Brand provenance is supplied separately as `brand-provenance.json`.
