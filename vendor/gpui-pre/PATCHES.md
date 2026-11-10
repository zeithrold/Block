# Maintained GPUI 0.3.9 source patch

This directory contains only the official `gpui-pre` 0.3.9 package and the
owned changes below. It does not upgrade GPUI or vendor the workspace's other
dependencies. Root `[patch.crates-io]` selects this package for every consumer;
the workspace excludes this third-party snapshot from application lint scope.

## Acquisition and licenses

- Registry archive: `gpui-pre-0.3.9.crate`, obtained from Cargo's official
  crates.io archive cache.
- Archive SHA-256:
  `34f61c1cfd36fc9c436af09308c963822ae27a45a84feb21a77ba5c9e051f1c0`.
- Packaged upstream revision:
  `93d745668805029476e6d7fbb19652116fed51de`.
- All 160 archive entries were acquired before patching. The normalized
  `Cargo.toml`, original `Cargo.toml.orig`, packaged lockfile, README, and
  `LICENSE-APACHE` are retained. `Cargo.toml.orig` records the original package,
  not this patch. The package has no `.cargo_vcs_info.json`.
- The normalized manifest adds direct edges to `unicode-linebreak` 0.1.5 and
  the already locked `unicode-segmentation` 1.13.3. These are existing resolved
  dependencies; no package version is upgraded.

To reacquire the base, verify the archive checksum above, then extract it into
an empty directory with `tar -xzf gpui-pre-0.3.9.crate --strip-components=1`.
Compare that base with this directory when updating or submitting the patch.
Do not replace this maintained directory by modifying Cargo's registry cache.

## Text policy

`src/text_system/line_breaks.rs` obtains normal UTF-8 opportunities from
`unicode-linebreak` and intersects them with extended grapheme boundaries.
Its emergency long-word policy uses the library's Unicode break classes,
not a localized punctuation whitelist. Glue, opening/closing punctuation,
nonstarters, and joined clusters remain protected. An indivisible unit wider
than the budget overflows rather than being split.

Shaped wrapping additionally requires a boundary between complete shaping
clusters across all font runs. Prefix maxima and suffix minima prevent a
duplicate or reordered glyph index from splitting a cluster. A visually
reordered span without a safe logical split remains intact.

`LineWrapper` wrapping and all truncation directions use the same text policy.
Existing character-width, indentation, affix, and run-adjustment algorithms
remain; the carried suffix is inspected once on entering the last clamped
row. The historical `is_word_char` classification remains test-only to retain
its original assertions; it no longer governs layout.

The pinned line-break implementation declares Unicode 15.0.0 and resolves
complex-context SA characters as AL. The emergency tailoring is not a claim
of current Unicode or dictionary-based all-script conformance.

No text is rewritten. Public GPUI APIs, original UTF-8 offsets, fonts, and
typography remain unchanged.

## Packaged test prerequisites

The published snapshot could not compile its own unit tests unchanged:

1. Two SVG test fonts referenced paths outside the package. Their exact
   upstream bytes and licenses are acquired under `test-fixtures/fonts/`;
   only the two `include_bytes!` paths change.
2. Three style tests used an unavailable `util_macros::perf` dependency.
   They use ordinary `#[test]` with every assertion retained.
3. The published `gpui::property_test` expansion panicked while parsing its
   generated syntax. The existing randomized render-equivalence case uses
   `proptest!` directly, preserving its four cases, arbitrary `u64` input,
   GPUI seed strategy, seed-config application, dispatcher lifecycle, four
   topology cases, empty-tree case, and all downstream assertions.

These are test setup repairs, not production rendering changes or skipped
tests. Original snapshot files remain otherwise intact; the new policy,
shaped wrapping implementation, and regression suites live in small owned
modules.

All additional font files come from the same upstream revision under
`assets/fonts/`, retaining their original license texts:

| Fixture | SHA-256 |
| --- | --- |
| `ibm-plex-sans/IBMPlexSans-Regular.ttf` | `975dcda37d80f038dcd143c22e33ca2d97a0cc5a929aace1c749153b0fe1afa5` |
| `ibm-plex-sans/license.txt` | `91c25c350d3cac39da2736d74f7ba37ef648f5237a4e330a240615bc8d8c4360` |
| `lilex/Lilex-Regular.ttf` | `8a467ad5a1941dc7c9cebaee34e3ed9364a73b48393034e363e4b528bd711d04` |
| `lilex/OFL.txt` | `a356708a399e3f54b937ec7d9b722c26dabba21365b60f95c56eb7ebca0dee40` |

## Focused verification

From the repository root:

```sh
CARGO_TARGET_DIR=target/ui-refinement-build \
  cargo test --locked -p gpui-pre -p block-ui --lib text_system:: -- --nocapture

# Linux ARM64, headless native font shaping, no desktop input:
CARGO_TARGET_DIR=target/ui-refinement-build \
  cargo test --locked -p block-ui --lib cjk_wrapping_tests -- --nocapture
```

Selecting `block-ui` alongside the excluded dependency in the first command
unifies its existing `test-support` dev feature. The `text_system::` filter
executes GPUI's owner tests; it matches no application tests. This avoids a
second lockfile or additional test dependencies. Cargo rejects a
`--features test-support` flag on an excluded package selected from the root.

Native screenshots, scrolling, accessibility, and full workspace checks are
separate evidence. See the task's implementation report for actual commands,
red/green results, platform scope, and unrun checks.
