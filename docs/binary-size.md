# Release binary size: analysis and measured options

Updated: 2026-10-24. **Analysis only.** The size experiments did not change `Cargo.toml` profiles or
packaging/build configuration; every option below was measured with `CARGO_PROFILE_RELEASE_*`
environment overrides and disposable builds under `target/size-analysis/`. The release
artifact at `target/aarch64-apple-darwin/release/block` was restored byte-identically
after the measurements (SHA-256 `d5ddef2c…f89d9`, 37,551,520 bytes).

Windows PE icon embedding was added separately after these measurements; see [packaging](../packaging/README.md#windows). None of the measured release-profile options were applied.

## Question

Why does the release build still weigh ~38 MB with the release profile?

Short answer: the binary is not mostly assets or the experiment corpus. Half of it is
code, and a quarter is the Mach-O symbol table that the default profile keeps. The
default `[profile]` spends nothing on link-time optimisation, keeps
`panic=unwind` (≈4.4 MB of unwind/exception tables) and keeps every code symbol
(≈9.3 MB of `nlist` + symbol names). The dependency graph is also large and fully
static: GPUI itself is ≈5.1 MiB of `.text`, the block UI/launcher code ≈4.6 MiB, and
an image/SVG/font stack, TLS/HTTP/tokio and regex add several MiB more.

Measured summary (Apple M5 Pro, macOS 27.0.1, rustc/cargo 1.99.0, target
`aarch64-apple-darwin`):

| Option (env override, no file edits) | Size | vs. baseline | Cold build |
| --- | ---: | ---: | ---: |
| Baseline: default release profile | 37,551,520 B (35.81 MiB) | — | ≈71 s |
| `strip="symbols"` | 28,168,128 B (26.86 MiB) | −9.38 MB (−25.0 %) | 66 s |
| `strip="symbols"` + `lto="thin"` | 28,551,008 B (27.23 MiB) | −9.00 MB (−24.0 %) | 67 s |
| `strip="symbols"` + `panic="abort"` | 20,402,640 B (19.46 MiB) | −17.15 MB (−45.7 %) | 62 s |
| `strip="symbols"` + `lto="fat"` + `codegen-units=1` | 21,677,488 B (20.67 MiB) | −15.87 MB (−42.3 %) | 182 s |
| D + `panic="abort"` | 17,427,824 B (16.62 MiB) | −20.12 MB (−53.6 %) | 167 s |
| E + `opt-level="s"` | 13,481,776 B (12.86 MiB) | −24.07 MB (−64.1 %) | 128 s |

## 1. Baseline: build command, artifact, profile defaults

The distribution build is the single release binary target:

```sh
cargo build --locked --release -p block-app --bin block --target aarch64-apple-darwin
```

(`packaging/README.md` shows the same command without `--target` and reads
`target/release/block`; with an explicit target the path is
`target/aarch64-apple-darwin/release/block`. `scripts/verify-platform.py` uses the same
`-p block-app --bin block` form, but in debug mode — see §7.)

Measured baseline artifact:

| Property | Value |
| --- | --- |
| Path | `target/aarch64-apple-darwin/release/block` |
| Size | 37,551,520 bytes = 37.55 MB (decimal) = 35.81 MiB |
| SHA-256 | `d5ddef2c49a9f31385e97f3aa7ebe50815300603f2e9b97d3e871180201f89d9` |
| Built | 2026-10-24 01:27, the `Cargo.lock` at measurement time (`--locked`) |
| Smoke test | `block --build-info` → `{"product_id":"one.ztd.block","version":"0.1.0","os":"macos","arch":"aarch64",…}` |

The "~38 MB" figure is consistent: 37.55 MB decimal, 35.8 MiB binary.

There is no `[profile.release]` section in the workspace `Cargo.toml`. Under cargo 1.99
that means: `opt-level=3`, `codegen-units=16`, `lto=false`, `panic="unwind"`,
`debug=0`, and — verified against a throwaway project with `cargo build --release -v` —
`strip="debuginfo"` (cargo passes `-C strip=debuginfo`). So the baseline is *not*
completely unstripped: debug info is already gone, but all code symbol names are kept.
That distinction matters below.

Reproducibility note: `cargo bloat` forces `CARGO_PROFILE_RELEASE_STRIP=false` to read
symbols, which builds a slightly larger 39,036,728-byte variant. The 1.48 MB difference
between that and the 37.55 MB profile default is entirely symbol payload (86,423 →
91,209 `nlist` entries, 7.93 MB → 9.34 MB of symbol strings); the `.text` sections differ
by 4.7 KB. Crate attribution in §2c was produced on that variant and applies to the
baseline; its `.text` size (19,003,516 B) is within 0.03 % of the baseline `.text`
(19,008,252 B).

## 2. Where the 37.55 MB goes

### a. Machine segments and sections (`size -m`, `otool -l`)

| Region | Bytes | % of file |
| --- | ---: | ---: |
| `__TEXT,__text` (+ `__text_startup`/`__stubs`/`__stub_helper`) | 19,022,020 | 50.7 % |
| `__TEXT,__const` | 2,253,968 | 6.0 % |
| `__TEXT,__eh_frame` | 2,692,804 | 7.2 % |
| `__TEXT,__gcc_except_tab` | 1,187,644 | 3.2 % |
| `__TEXT,__unwind_info` | 484,712 | 1.3 % |
| `__TEXT,__cstring` | 255,501 | 0.7 % |
| `__DATA_CONST` (`__const` 1,214,136 + `__got`/`__mod_init_func`) | 1,228,800 | 3.3 % |
| `__DATA` (`__data` 140,912 + lazy pointers/thread vars/bss) | 163,840 | 0.4 % |
| `__LINKEDIT` | 10,256,384 | 27.3 % |
| **Total file** | **37,551,520** | 100 % |

Rebucketed by purpose:

| Purpose | Bytes | % |
| --- | ---: | ---: |
| Code (`.text` + stubs) | 19,022,020 | 50.7 % |
| Symbol table (`nlist` 1,382,768 + strings 7,929,136) | 9,311,904 | 24.8 % |
| Unwind/exception metadata (`__eh_frame` + `__gcc_except_tab` + `__unwind_info`) | 4,365,160 | 11.6 % |
| Constant/data payload (`__TEXT,__const` + `__DATA_CONST,__const` + `__DATA,__data`) | 3,609,016 | 9.6 % |
| Export trie | 543,320 | 1.4 % |
| Ad-hoc code signature | 291,248 | 0.8 % |
| C strings | 255,501 | 0.7 % |
| dyld rebase/bind/lazy + function starts | 105,768 | 0.3 % |
| Segment padding, `__got`/`__mod_init_func`, thread vars, zerofill bookkeeping | ≈ 48,000 | 0.1 % |

### b. `__LINKEDIT` inside (10.26 MB = 27.3 %)

| Component | Bytes | Notes |
| --- | ---: | --- |
| Symbol string table | 7,929,136 | 86,423 symbols, avg. 91.7 B/name |
| Symbol `nlist` records | 1,382,768 | 86,423 × 16 B (78,313 local, 8,110 external) |
| Export trie | 543,320 | only needed by dyld for exported names |
| Ad-hoc code signature | 291,248 | rustc/cargo linker-signed |
| Function starts (LC_FUNCTION_STARTS) | 73,216 | |
| dyld rebase/bind/lazy bind info | 32,552 | |
| Alignment | ≈ 4,100 | |

This is the single biggest lever: **≈9.3 MB of symbol payload is dead weight in a
distributed executable** (it only improves crash backtraces), and `strip="symbols"`
removes it completely — `nlist` drops 86,423 → 497, strings 7.9 MB → 10 KB. The export
trie (543 KB) survives `strip="symbols"` because those names are dyld-reachable; only LTO
internalises them.

### c. Per-crate contribution (`cargo bloat --crates`, address-range attribution)

`.text` is 48.7 % of the 39.0 MB cargo-bloat analysis file (50.6 % of the baseline);
the largest single function is 100.6 KiB
(`gpui_component::theme::theme_color::ThemeTokens::reconcile`), and there are 44,699
methods — **there is no hotspot to optimise locally**; size is spread across generic
monomorphisations.

| Group | `.text` | % of `.text` |
| --- | ---: | ---: |
| First-party (`block_ui`, `block_launcher`, `block_app`, `block_auth`, `block_download`, `block_drop`, `minecraft_manifest`) | 4.62 MiB | 25.5 % |
| GPUI framework (`gpui`, `gpui_base`, `gpui_component`, `gpui_macos`, `gpui_apple`, `accesskit*`, taffy, lyon) | 5.12 MiB | 28.3 % |
| Image / SVG / font stack (`image` + codecs, `usvg`, `resvg`, `tiny_skia`, `rustybuzz`, `ttf_parser`, `html5ever`, …) | 2.42 MiB | 13.3 % |
| std/core/backtrace | 1.76 MiB | 9.7 % |
| Networking / HTTP / TLS / tokio (`rustls`, `reqwest`, `hyper`, `tokio`, `ring`, …) | 1.56 MiB | 8.6 % |
| regex stack (`regex_automata`, `regex_syntax`, `aho_corasick`) | 0.63 MiB | 3.5 % |
| clap CLI | 0.24 MiB | 1.3 % |
| serde/JSON | 0.19 MiB | 1.0 % |
| all remaining crates | 1.59 MiB | 8.7 % |
| **Total `.text` (18.13 MiB)** | | 100 % |

Largest individual crates: `block_ui` 2.87 MiB, `gpui` 2.02, `std` 1.70, `gpui_base`
1.60, `block_launcher` 1.59, `gpui_component` 0.97, `image` 0.44, `rustls` 0.41,
`reqwest` 0.40, `usvg` 0.37, `regex_automata` 0.29, `rustybuzz` 0.27.

`nm -S` cannot size symbols on Mach-O (the `nlist` format has no size field, so `nm`
prints 0); attribution therefore uses cargo-bloat's next-symbol-address heuristic, which
the tool itself labels approximate. Ratios are reliable to a few percent; absolute
per-crate bytes are not exact. A complete crate/module/function/symbol-table/unwind
attribution is in [§8](#8-crate--module--function-attribution-and-hotspots).

### d. Embedded assets: 0.34 MiB (0.95 %)

`include_bytes!` reachability in the shipped binary:

| Embedded asset | Bytes |
| --- | ---: |
| Inter fonts: Regular/Medium/SemiBold/Bold TTF (`crates/block-ui/assets/fonts/`) | 266,168 |
| Lucide-style SVG icons, 49 files (`crates/block-ui/assets/icons/`) | 16,020 |
| UI logo/icon `block.png` (256 px) | 70,540 |
| `installer-cache.jar` (generated NeoForge installer cache helper) | 4,327 |
| **Total** | **357,055 B = 0.34 MiB** |

Not embedded:

- `crates/block-ui/assets/block-1024.png` (810,799 B) is the packaging/ICNS source only;
  no `include_bytes!` references it.
- `experiments/minecraft-manifests/` — 925 files, 24,754,957 B (23.6 MiB) on disk. The
  only reference is `crates/minecraft-manifest/src/bin/manifest-audit.rs:53`, which is a
  separate `manifest-audit` binary resolved through `CARGO_MANIFEST_DIR`; it is not a
  dependency of `block-app`. The shipped `block` binary contains 0 occurrences of the
  string `minecraft-manifests`. The experiment corpus is **not** in the app binary.

Assets are therefore not a size cause; removing every asset would save under 1 %.

### e. Linux-only GPUI features on macOS: no measured effect

The workspace `Cargo.toml` enables `gpui_platform` features `wayland` and `x11`
unconditionally. In `gpui-pre-platform 0.3.9` those features only forward to
`gpui_linux` (`wayland = ["gpui_linux/wayland"]`, `x11 = ["gpui_linux/x11"]`), and
`gpui_linux` is declared under
`[target.'cfg(any(target_os = "linux", target_os = "freebsd"))'.dependencies]`.
`cargo tree --target aarch64-apple-darwin` for `block-app` contains no
`gpui_linux`/`wayland`/`x11`/`smithay`/`calloop` crates, so on macOS the features enable
nothing and cost nothing. This is a configuration-hygiene issue, not a macOS size cause.
Gating them properly (moving them to a `[target.'cfg(target_os = "linux")']` dependency
entry) requires editing `crates/block-app/Cargo.toml` and `crates/block-ui/Cargo.toml`;
it would **not** reduce the macOS binary.

## 3. Measured options (environment overrides, no file edits)

Method: parallel-free sequential full release rebuilds with
`CARGO_PROFILE_RELEASE_*` overrides, each recorded for size, cold build time and
`block --build-info` (all seven binaries executed successfully). Files:
`target/size-analysis/variants.sh`, `variants.tsv`, `build-*.log`, `block-*` binaries.

| # | Configuration (env override) | Size | Δ | Build | Verdict |
| --- | --- | ---: | ---: | ---: | --- |
| — | baseline, default profile | 37,551,520 B | — | ≈71 s | current state |
| A | `STRIP=symbols` | 28,168,128 B | −25.0 % | 66 s | safe, adopt first |
| B | A + `LTO=thin` | 28,551,008 B | −24.0 % | 67 s | **worse than A**; reject |
| C | A + `PANIC=abort` | 20,402,640 B | −45.7 % | 62 s | best size/time ratio |
| D | A + `LTO=fat` + `CODEGEN_UNITS=1` | 21,677,488 B | −42.3 % | 182 s | LTO spend; see E |
| E | D + `PANIC=abort` | 17,427,824 B | −53.6 % | 167 s | smallest defensible |
| F | E + `OPT_LEVEL=s` | 13,481,776 B | −64.1 % | 128 s | not recommended (quality) |

Build times are wall-clock on a 15-logical-core M5 Pro with warm registry cache; they
vary ±10–15 % run-to-run (D measured slower than E), so treat the ~2.4–2.9× LTO factor as
approximate. Each variant is a full dependency rebuild because profile settings are part
of cargo's fingerprint.

Where the bytes actually move (`size -m`):

| Section / segment | Baseline | A (strip) | C (A+abort) | D (A+fat LTO) | E (D+abort) |
| --- | ---: | ---: | ---: | ---: | ---: |
| `__TEXT` | 25,919,488 | 25,903,104 | 18,235,392 | 20,332,544 | 16,121,856 |
| `__DATA_CONST` | 1,228,800 | 1,228,800 | 1,212,416 | 933,888 | 933,888 |
| `__DATA` | 163,840 | 163,840 | 163,840 | 163,840 | 163,840 |
| `__LINKEDIT` | 10,256,384 | 901,120 | 819,200 | 278,528 | 229,376 |
| `.text` | 19,008,252 | 19,000,892 | 15,505,916 | 15,571,432 | 13,749,012 |
| `__eh_frame` | 2,692,804 | 2,690,596 | 62,208 | 1,461,448 | 44,072 |
| `__gcc_except_tab` | 1,187,644 | 1,186,860 | 11,472 | 777,008 | 8,568 |
| `__unwind_info` | 484,712 | 484,272 | 126,432 | 254,800 | 60,120 |
| `__cstring` | 255,501 | 255,401 | 253,105 | 170,083 | 169,909 |
| Symbol strings | 7,929,136 | 10,392 | 10,392 | 10,360 | 10,352 |
| Export trie | 543,320 | 542,672 | 540,504 | 10,464 | 10,472 |

Findings that shape the recommendation:

- **Symbol stripping is the cheapest 25 %**: it removes the whole 9.3 MB symbol payload
  plus most of the export trie, at *no* build-time cost, and cannot change program
  behaviour (only crash backtrace quality).
- **`lto="thin"` is a net loss here.** At 16 codegen units it inlines more across units,
  growing `.text` by ~0.70 MB while `__LINKEDIT` shrinks by only ~0.15 MB → 0.38 MB
  *larger* than strip alone. Thin LTO is not a size option for this crate graph.
- **`panic="abort"` is extremely effective**: ≈4.16 MB of unwind/exception tables and a
  further ≈3.50 MB of landing-pad code, for a 62 s build (i.e. no extra build cost over
  the baseline). `strip+abort` (C) beats `strip+fat LTO` (D) in both size and build time.
- **Fat LTO still buys 2.98 MB on top of abort** (C → E) and internalises the export
  trie, but costs ~100 s more build time and serialises codegen (`codegen-units=1`).
- **`opt-level="s"` is a further 3.95 MB** (E → F), the largest single remaining lever,
  but GPUI render/animation hot paths are exactly what `opt-level=3` is for and the user
  requirement is smooth animation. No frame-time data was collected here; do not adopt
  without motion/perf validation.

## 4. Recommendation (measured, **not applied**)

The user's instruction for this pass was analysis only, so nothing below has been added
to `Cargo.toml`, packaging or scripts. If a fix is approved, the staged configuration is:

1. **Adopt now (no behaviour change, no build-cost):** `strip = "symbols"` → 28.17 MB
   (−25 %). Only cost: release crash backtraces lose symbol names.
2. **Best value (needs a behaviour sign-off):** add `panic = "abort"` → **20.40 MB
   (−45.7 %)** with no measurable build-time increase.
3. **Shipping build (needs build-time budget):** add `lto = "fat"` and
   `codegen-units = 1` → **17.43 MB (−53.6 %)**; cold build ≈2.4× baseline (167–182 s vs
   ≈71 s).

The corresponding future `[profile.release]` (root `Cargo.toml`), shown for review only —
**not applied**:

```toml
[profile.release]
strip = "symbols"
panic = "abort"
lto = "fat"
codegen-units = 1
```

Leave `opt-level = "s"` out unless frame-time validation on the motion scenarios shows no
regression.

Behaviour tradeoffs:

- `strip = "symbols"`: diagnostics only. The binary still launches (`--build-info`
  verified) and keeps its rustc-applied ad-hoc signature. Panics in release no longer
  produce named frames.
- `panic = "abort"`: any panic anywhere (including third-party code) terminates the
  process instead of unwinding the panicking thread. Block's own crates contain no
  `catch_unwind`/`resume_unwind`, and the workspace clippy lints deny `panic!`, `unwrap`
  and `expect`; the only `catch_unwind` uses found in gpui-pre are inside
  `#[test]`/`#[gpui::test]` functions (`text_system/line.rs`, `profiler/hang.rs`), so
  tests are unaffected. The real risk is a user-visible "app exits instead of one feature
  failing" on a latent dependency panic; a launcher restarts cheaply, but this must be a
  deliberate product decision and covered by runtime smoke/soak tests before shipping.
- `lto = "fat"` + `codegen-units = 1`: slower, more memory-hungry builds and no
  per-crate parallelism; runtime code is normally equal or faster. Only shipping builds
  should pay for it.

## 5. Residual reduction ideas and honest limits

Ordered by expected value; none of these were implemented (source/packaging changes are
out of scope for this analysis).

| Idea | Candidate saving | Limits |
| --- | ---: | --- |
| Drop unused `image` decoders (all 15 format features are on via `gpui-pre` defaults: bmp, dds, exr, ff, gif, hdr, ico, jpeg, png, pnm, qoi, rayon, tga, tiff, webp) | ≈0.6–1.0 MB code (`image_webp` 213 KiB, `exr` 129 KiB, `zune_jpeg` 90 KiB, `png` 66 KiB, `tiff` 61 KiB, plus part of the 455 KiB `image` core) | Not reachable from this repo: `gpui-pre` enables them by default. Needs an upstream feature or a patched/forked GPUI dependency, which also breaks the "no source changes" constraint and adds a supply-chain patch. |
| Pre-rasterise SVG icons and drop / trim the SVG pipeline (`usvg` 379 KiB, `resvg` 180 KiB, `tiny_skia` 160 KiB, `rustybuzz` 277 KiB, `ttf_parser` 122 KiB, `html5ever` 124 KiB, `quick_xml` 57 KiB) | ≈1.3 MB of `.text` at most | `gpui`/`gpui_component` also rasterise SVG assets (gpui-kit-assets), so the stack probably cannot be removed entirely; partial only, and it changes icon fidelity/scaling. |
| Replace the regex stack with a small matcher | ≈0.63 MB (`regex_automata` 302 KiB, `regex_syntax` 209 KiB, `aho_corasick` 120 KiB) | One call site: `crates/block-launcher/src/rules.rs:60` `regex::Regex::new(version)` over user-configured version rules. Re-implementation must preserve exact matching semantics; needs its own tests. |
| Font subsetting (Inter, 4 × TTF = 266 KB) | ≈0.1–0.2 MB | Subsetting the fonts to used glyphs loses future localisation/glyph coverage and changes asset provenance receipts. |
| Remove/trim clap | ≈0.24 MB | Manual argument parsing trades a well-tested CLI layer for a small saving. |
| Full post-link `strip` beyond `strip="symbols"` | ≈0.5 MB (export trie + residual external symbols) | Would invalidate the ad-hoc code signature that packaging copies byte-for-byte (`packaging/README.md`); requires adding a `codesign --force -s -` re-sign step to packaging, outside this analysis and a signing/release concern. |
| Debug-symbol splitting / dSYM | 0 | Moot: release already has `debug=0` and cargo strips debuginfo. The removable payload is code-symbol *names*, not DWARF. A dSYM for this build would be empty. |
| Dynamic linking of dependencies (dylib) | ~0 download saving | Moves bytes into the bundle instead of removing them, adds loader/rpath and signing complexity; not worthwhile for a launcher. |

What cannot be removed: the fully static dependency graph is the reason a GPUI launcher
is this big at all — GPUI + block UI/launcher code alone are ≈9.7 MiB of `.text`, and
that is the application. std/core/backtrace (1.76 MiB) and the TLS/HTTP/tokio stack
(1.56 MiB, required for Microsoft auth, manifest/download and updates) are also
inherent. The realistic floor with an acceptable product is the 17.4 MB (E) or, if
`panic="abort"` is rejected, 21.7 MB (D) configuration; the 13.5 MB (F) number is a
size-only result that should not be shipped on animation-quality grounds.

Measurement limits: one host/toolchain; per-crate attribution is heuristic; build times
are load-dependent; only size and `--build-info` launching were verified per variant — no
UI, motion, signing, notarisation or package-install verification was performed. Runtime
performance of `opt-level="s"` and of `panic="abort"` crash semantics remain unvalidated.

## 6. Re-measure

```sh
# baseline size (+ sections, symbols, linkedit)
cargo build --locked --release -p block-app --bin block --target aarch64-apple-darwin
B=target/aarch64-apple-darwin/release/block
stat -f '%z bytes' "$B"; size -m "$B"; otool -l "$B" | grep -A4 LC_SYMTAB
"$B" --build-info

# per-crate / per-function attribution (forces CARGO_PROFILE_RELEASE_STRIP=false)
cargo bloat --release --locked -p block-app --bin block --target aarch64-apple-darwin --crates -n 0 --message-format json
cargo bloat --release --locked -p block-app --bin block --target aarch64-apple-darwin -n 25

# a candidate profile variant (no file edits; replace/skip overrides as needed)
CARGO_PROFILE_RELEASE_STRIP=symbols \
CARGO_PROFILE_RELEASE_LTO=fat \
CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1 \
CARGO_PROFILE_RELEASE_PANIC=abort \
cargo build --locked --release -p block-app --bin block --target aarch64-apple-darwin
stat -f '%z bytes' "$B" && "$B" --build-info
```

Notes: measure variants sequentially — profile settings change cargo's fingerprint, so
each is a full rebuild. `CARGO_PROFILE_RELEASE_*` overrides are per-invocation and leave
`Cargo.toml` untouched. `cargo build` without an explicit `--target` writes
`target/release/block` (as in `packaging/README.md`). Script and raw data used here:
`target/size-analysis/variants.sh`, `variants.tsv`, `build-*.log`, `bloat-crates.json`,
`bloat-functions.txt`, and the preserved binaries (`block-baseline`, `block-strip`,
`block-strip-abort`, `block-thin`, `block-fat`, `block-fat-abort`, `block-fat-abort-s`).

## 7. Impact on CI

CI's uploaded launcher artifact is the **debug** binary: `scripts/verify-platform.py`
runs `cargo build --locked -p block-app --bin block --target <host>` and uploads
`target/<host>/debug/block` (`executable=` output in the workflow). Any release-profile
change therefore affects distribution builds only; it does not change CI artifacts,
CI evidence or test coverage.

## 8. Crate / module / function attribution and hotspots

Follow-up analysis requested after the profile-option study: what exactly makes
up the 37.55 MB, including third-party dependencies. Everything here is
analysis only; no source, `Cargo.toml`, script or packaging file was changed.
Raw outputs are under `target/size-analysis/` (`attribution-*.csv`,
`symbol-summary.json`, `unwind-summary.json`, `code-by-name-owner.json`) and
are reproducible with the scripts listed in §8.8.

### 8.1 Basis, method and caveats

- Code attribution: `cargo bloat 0.12.1` against the same sources —
  `cargo bloat --release --locked -p block-app --bin block --target aarch64-apple-darwin --crates -n 0 --message-format json`,
  plus the same command without `--crates` for all 44,724 functions. cargo-bloat
  forces `CARGO_PROFILE_RELEASE_STRIP=false` to read symbols, so its analysis
  binary is 39,036,728 B (37.2 MiB), not the 37,551,520 B release artifact; its
  `.text` is 19,003,516 B vs 19,008,252 B in the artifact (+0.03 %), so per-crate
  shares transfer. `% of .text` uses 19,003,516 B; `% of file` uses the real
  release artifact (37,551,520 B) as denominator.
- Two attribution axes are used, and they answer different questions:
  **(a) link-unit attribution** (`cargo bloat --crates`): which compiled object
  the bytes sit in; monomorphised generic copies are counted with the crate that
  instantiates them, so generic-heavy dependencies look smaller than their
  symbol names suggest. **(b) defining-path attribution** (the crate/module path
  in the demangled function name): which crate's source the code/monomorphisation
  belongs to; this is used for the module and generic-machinery views.
- Symbol table: parsed directly from the release artifact's `LC_SYMTAB`
  (86,423 `nlist` records, 7,929,136 B string table, exact sizes), names
  demangled in bulk with Xcode's `llvm-cxxfilt`, attributed by the first path
  component. ld64 merges shared string suffixes; per-owner string bytes are
  therefore scaled to the real 7,929,136 B table (global factor
  0.8492); `nlist` bytes are exact.
- Unwind attribution: `dwarfdump --eh-frame` on the release artifact for every
  FDE `(pc, length)`, mapped to its function via `LC_FUNCTION_STARTS` (44,740
  starts) and `LC_SYMTAB` (100 % of FDEs resolved). FDE byte sums match the
  section exactly (1,427 CIEs + 34,838 FDEs = 2,692,804 B). `__gcc_except_tab`
  and `__unwind_info` have no per-function records in a release build; their
  per-owner split is an estimate proportional to FDE bytes.
- One host/toolchain (Apple M5 Pro, macOS 27.0.1, rustc/cargo 1.99.0); cargo-bloat
  sizes symbols by next-symbol-address ranges (its own output calls the result
  "guesswork"). Ratios are solid to a few percent; individual function sizes are
  approximate. No runtime measurements were taken.

### 8.2 Per-crate `.text` (link-unit attribution)

First-party crates (7, all of them):

| # | first-party crate | .text bytes | % of .text | % of file |
| :---: | :--- | ---: | ---: | ---: |
| 1 | `block_ui` | 3,014,412 | 15.86 % | 8.03 % |
| 5 | `block_launcher` | 1,670,364 | 8.79 % | 4.45 % |
| 33 | `block_auth` | 84,272 | 0.44 % | 0.22 % |
| 35 | `block_download` | 68,632 | 0.36 % | 0.18 % |
| 69 | `block` | 19,284 | 0.10 % | 0.05 % |
| 110 | `minecraft_manifest` | 6,064 | 0.03 % | 0.02 % |
| 114 | `block_drop` | 5,848 | 0.03 % | 0.02 % |

Third-party crates, top 50 of 234 (remaining 184 crates total 1,010,360 B,
1.0 MiB):

| # | third-party crate | group | .text bytes | % of .text | % of file |
| :---: | :--- | :--- | ---: | ---: | ---: |
| 1 | `gpui` | gpui-framework | 2,113,488 | 11.12 % | 5.63 % |
| 2 | `std` | std-core | 1,781,692 | 9.38 % | 4.75 % |
| 3 | `gpui_base` | gpui-framework | 1,680,400 | 8.84 % | 4.47 % |
| 4 | `gpui_component` | gpui-framework | 1,017,716 | 5.36 % | 2.71 % |
| 5 | `image` | image-svg-font | 466,324 | 2.45 % | 1.24 % |
| 6 | `rustls` | net-tls-async | 431,368 | 2.27 % | 1.15 % |
| 7 | `reqwest` | net-tls-async | 422,024 | 2.22 % | 1.12 % |
| 8 | `usvg` | image-svg-font | 388,172 | 2.04 % | 1.03 % |
| 9 | `regex_automata` | regex | 309,064 | 1.63 % | 0.82 % |
| 10 | `rustybuzz` | image-svg-font | 283,408 | 1.49 % | 0.76 % |
| 11 | `gpui_macos` | gpui-framework | 268,432 | 1.41 % | 0.71 % |
| 12 | `clap_builder` | clap-cli | 244,568 | 1.29 % | 0.65 % |
| 13 | `tokio` | net-tls-async | 242,072 | 1.27 % | 0.65 % |
| 14 | `image_webp` | image-svg-font | 218,536 | 1.15 % | 0.58 % |
| 15 | `regex_syntax` | regex | 213,856 | 1.12 % | 0.57 % |
| 16 | `markdown` | other-utility | 189,136 | 0.99 % | 0.50 % |
| 17 | `resvg` | image-svg-font | 184,676 | 0.97 % | 0.49 % |
| 18 | `serde_json` | serde-json | 172,952 | 0.91 % | 0.46 % |
| 19 | `tiny_skia` | image-svg-font | 163,920 | 0.86 % | 0.44 % |
| 20 | `ring` | net-tls-async | 150,908 | 0.79 % | 0.40 % |
| 21 | `exr` | image-svg-font | 131,096 | 0.69 % | 0.35 % |
| 22 | `html5ever` | image-svg-font | 126,904 | 0.67 % | 0.34 % |
| 23 | `ttf_parser` | image-svg-font | 125,372 | 0.66 % | 0.33 % |
| 24 | `aho_corasick` | regex | 123,092 | 0.65 % | 0.33 % |
| 25 | `taffy` | gpui-framework | 122,804 | 0.65 % | 0.33 % |
| 26 | `autocfg` | other-utility | 120,424 | 0.63 % | 0.32 % |
| 27 | `accesskit_consumer` | gpui-framework | 113,476 | 0.60 % | 0.30 % |
| 28 | `hyper` | net-tls-async | 105,416 | 0.56 % | 0.28 % |
| 29 | `zune_jpeg` | image-svg-font | 92,228 | 0.48 % | 0.25 % |
| 30 | `gpui_apple` | gpui-framework | 87,356 | 0.46 % | 0.23 % |
| 31 | `accesskit_macos` | gpui-framework | 76,012 | 0.40 % | 0.20 % |
| 32 | `webpki` | net-tls-async | 67,892 | 0.36 % | 0.18 % |
| 33 | `png` | image-svg-font | 67,492 | 0.35 % | 0.18 % |
| 34 | `tiff` | image-svg-font | 62,548 | 0.33 % | 0.17 % |
| 35 | `lyon_tessellation` | gpui-framework | 60,556 | 0.32 % | 0.16 % |
| 36 | `quick_xml` | image-svg-font | 58,576 | 0.31 % | 0.16 % |
| 37 | `hyper_util` | other-utility | 55,232 | 0.29 % | 0.15 % |
| 38 | `url` | net-tls-async | 53,952 | 0.28 % | 0.14 % |
| 39 | `sum_tree` | other-utility | 50,500 | 0.27 % | 0.13 % |
| 40 | `roxmltree` | image-svg-font | 48,980 | 0.26 % | 0.13 % |
| 41 | `ropey` | other-utility | 48,748 | 0.26 % | 0.13 % |
| 42 | `proc_macro2` | other-utility | 48,508 | 0.26 % | 0.13 % |
| 43 | `tiny_skia_path` | other-utility | 47,704 | 0.25 % | 0.13 % |
| 44 | `svgtypes` | image-svg-font | 47,684 | 0.25 % | 0.13 % |
| 45 | `http` | net-tls-async | 44,260 | 0.23 % | 0.12 % |
| 46 | `bytes` | net-tls-async | 42,600 | 0.22 % | 0.11 % |
| 47 | `schemars` | other-utility | 41,184 | 0.22 % | 0.11 % |
| 48 | `idna` | net-tls-async | 40,212 | 0.21 % | 0.11 % |
| 49 | `cc` | other-utility | 37,944 | 0.20 % | 0.10 % |
| 50 | `scheduler` | other-utility | 37,232 | 0.20 % | 0.10 % |

Group rollup (all 241 crates; groups are a documentation convenience, not cargo
features):

| group | .text bytes | crates | % of .text | % of file |
| :--- | ---: | ---: | ---: | ---: |
| `gpui-framework` | 5,565,892 | 17 | 29.3 % | 14.82 % |
| `first-party` | 4,868,876 | 7 | 25.6 % | 12.97 % |
| `image-svg-font` | 2,535,700 | 22 | 13.3 % | 6.75 % |
| `std-core` | 1,863,108 | 6 | 9.8 % | 4.96 % |
| `net-tls-async` | 1,661,528 | 21 | 8.7 % | 4.42 % |
| `other-utility` | 1,254,912 | 126 | 6.6 % | 3.34 % |
| `regex` | 656,132 | 5 | 3.5 % | 1.75 % |
| `clap-cli` | 252,152 | 6 | 1.3 % | 0.67 % |
| `serde-json` | 185,472 | 5 | 1.0 % | 0.49 % |
| `compression-archive` | 61,788 | 5 | 0.3 % | 0.17 % |
| `objc-apple` | 54,312 | 12 | 0.3 % | 0.14 % |
| `async-util` | 32,940 | 4 | 0.2 % | 0.09 % |
| `crypto-hash` | 15,120 | 5 | 0.1 % | 0.04 % |

Readings:

- **First-party code = 4,868,876 B, 25.6 % of `.text` /
  12.97 % of the release file.** It is concentrated in
  `block_ui` (3,014,412 B; 62 % of first-party code — pages, shell, components,
  settings) and `block_launcher` (1,670,364 B — install/repair/process
  supervision). The `block` bin itself is 19,284 B: application logic lives in
  the libraries, not `main`.
- **Third-party code = 14,139,056 B, 74.4 % of `.text` /
  37.65 % of the file** across 234 crates. The single biggest
  subsystem is the **GPUI framework stack: 5,565,892 B,
  29.3 % of `.text`, 14.82 %
  of the file** (17 crates: `gpui` 2,113,488; `gpui_base` 1,680,400;
  `gpui_component` 1,017,716; `gpui_macos` 268,432; `gpui_apple`, taffy, lyon,
  accesskit…). It is also *the biggest third-party driver of the 38 MB*.
- Top 5 crates by `.text` account for 10,260,356 B = 54.0 % of `.text` (27.3 % of
  the file): `block_ui`, `gpui`, `std`, `gpui_base`, `block_launcher`.
- Size is very concentrated: the 30 crates at ≥100 KiB hold 16,596,072 B =
  87.3 % of `.text`; 63 crates contribute less than
  1 KiB each for a combined 25,544 B.
- Image/SVG/font stack: 2,535,700 B
  (6.75 % of file) over 22 crates — image
  codecs, `usvg`/`resvg`/`tiny_skia`, `rustybuzz`/`ttf_parser`, HTML/XML parsing
  for SVG. TLS/HTTP/tokio: 1,661,528 B
  (4.42 % of file). Regex:
  656,132 B (1.75 %).

### 8.3 Module-level rollup (defining path)

Top 40 modules after excluding the generic-machinery roots (`core`, `alloc`,
`std`, `hashbrown` are shown separately below). "Link unit" is cargo-bloat's
crate attribution for the same functions and can differ from the path because of
monomorphisation/inlining.

| # | crate::module (defining path) | .text bytes | % of .text | functions | link unit |
| :---: | :--- | ---: | ---: | ---: | :--- |
| 1 | `serde_json::de` | 664,432 | 3.50 % | 774 | `block_launcher` |
| 2 | `block_ui::Block` | 602,272 | 3.17 % | 267 | `block_ui` |
| 3 | `gpui::app` | 482,032 | 2.54 % | 1386 | `gpui` |
| 4 | `tokio::runtime` | 469,000 | 2.47 % | 1931 | `block_ui` |
| 5 | `gpui_base::text` | 367,788 | 1.94 % | 261 | `gpui_base` |
| 6 | `gpui::window` | 344,804 | 1.81 % | 808 | `gpui` |
| 7 | `gpui::element` | 339,064 | 1.78 % | 1166 | `gpui_base` |
| 8 | `gpui_component::theme` | 332,468 | 1.75 % | 52 | `gpui_component` |
| 9 | `gpui::elements` | 228,800 | 1.20 % | 644 | `gpui` |
| 10 | `rustybuzz::hb` | 219,844 | 1.16 % | 193 | `rustybuzz` |
| 11 | `block_ui::components` | 204,532 | 1.08 % | 95 | `block_ui` |
| 12 | `gpui_base::input` | 198,172 | 1.04 % | 598 | `gpui_base` |
| 13 | `usvg::parser` | 198,148 | 1.04 % | 174 | `usvg` |
| 14 | `taffy::compute` | 186,128 | 0.98 % | 72 | `taffy` |
| 15 | `block_ui::engine` | 180,532 | 0.95 % | 102 | `block_ui` |
| 16 | `serde_path_to_error::wrap` | 158,988 | 0.84 % | 224 | `block_launcher` |
| 17 | `image_webp::lossless` | 143,220 | 0.75 % | 17 | `image_webp` |
| 18 | `smallvec::SmallVec` | 140,108 | 0.74 % | 262 | `gpui` |
| 19 | `serde_path_to_error::de` | 135,800 | 0.71 % | 365 | `block_launcher` |
| 20 | `zune_jpeg::decoder` | 126,804 | 0.67 % | 44 | `image` |
| 21 | `serde_json::ser` | 120,876 | 0.64 % | 234 | `block_launcher` |
| 22 | `ttf_parser::tables` | 115,180 | 0.61 % | 158 | `ttf_parser` |
| 23 | `image::codecs` | 112,304 | 0.59 % | 228 | `image` |
| 24 | `block_launcher::install` | 110,248 | 0.58 % | 107 | `block_launcher` |
| 25 | `serde::private` | 103,576 | 0.55 % | 114 | `block_launcher` |
| 26 | `markdown::construct` | 103,472 | 0.54 % | 384 | `markdown` |
| 27 | `rustls::client` | 101,420 | 0.53 % | 138 | `rustls` |
| 28 | `rustls::msgs` | 98,072 | 0.52 % | 164 | `rustls` |
| 29 | `gpui_macos::window` | 86,568 | 0.46 % | 177 | `gpui_macos` |
| 30 | `serde_json::value` | 86,236 | 0.45 % | 146 | `block_ui` |
| 31 | `hyper_util::client` | 81,072 | 0.43 % | 144 | `hyper_util` |
| 32 | `anyhow::error` | 80,608 | 0.42 % | 1094 | `block_launcher` |
| 33 | `html5ever::tree_builder` | 80,364 | 0.42 % | 70 | `html5ever` |
| 34 | `http::header` | 79,240 | 0.42 % | 106 | `http` |
| 35 | `tiny_skia::pipeline` | 78,848 | 0.41 % | 127 | `tiny_skia` |
| 36 | `regex_automata::meta` | 78,640 | 0.41 % | 175 | `regex_automata` |
| 37 | `image_webp::vp8` | 77,076 | 0.41 % | 34 | `image_webp` |
| 38 | `zune_jpeg::bitstream` | 76,796 | 0.40 % | 15 | `image` |
| 39 | `sum_tree::SumTree` | 75,776 | 0.40 % | 24 | `sum_tree` |
| 40 | `block_launcher::catalog` | 73,752 | 0.39 % | 98 | `block_launcher` |

Generic machinery — functions whose defining path lies in `core`/`alloc`/`std`/
`hashbrown` (drop glue, iterators, formatting, collections) — totals
**5,539,588 B = 29.1 % of `.text` / 14.75 % of
the file**. Top 10 modules including that machinery:

| # | module (all) | .text bytes | % of .text | functions | kind |
| :---: | :--- | ---: | ---: | ---: | :--- |
| 1 | `core::ptr` | 2,852,416 | 15.01 % | 12012 | generic machinery |
| 2 | `serde_json::de` | 664,432 | 3.50 % | 774 | subsystem |
| 3 | `block_ui::Block` | 602,272 | 3.17 % | 267 | subsystem |
| 4 | `core::slice` | 482,144 | 2.54 % | 585 | generic machinery |
| 5 | `gpui::app` | 482,032 | 2.54 % | 1386 | subsystem |
| 6 | `tokio::runtime` | 469,000 | 2.47 % | 1931 | subsystem |
| 7 | `alloc::vec` | 405,532 | 2.13 % | 1132 | generic machinery |
| 8 | `gpui_base::text` | 367,788 | 1.94 % | 261 | subsystem |
| 9 | `gpui::window` | 344,804 | 1.81 % | 808 | subsystem |
| 10 | `gpui::element` | 339,064 | 1.78 % | 1166 | subsystem |

Readings:

- `core::ptr` alone is 2,852,416 B over 12,012 functions (15.0 % of `.text`):
  overwhelmingly `drop_in_place`/drop-glue and pointer shims for every concrete
  type in the graph. That is Rust monomorphisation overhead, not application
  code, and it is what fat LTO + `panic="abort"` reclaim (measured `.text`
  19.0 → 13.7 MB). By link unit, the copies are instantiated mainly by `std`
  (1,749,064 B,
  31.6 % of all generic
  machinery), `gpui` (563,572),
  `block_ui` (562,768),
  `gpui_base` (485,232) and
  `block_launcher` (306,028).
- Largest non-generic module: `serde_json::de` — 664,432 B over 774 functions,
  instantiated mostly in `block_launcher`'s unit: manifest/theme/document
  deserialisation (serde's generated visitors are the second-largest single
  function in the binary, see §8.4).
- `block_ui::Block` — 602,272 B / 267 functions: the app's page/render state
  machine, the biggest first-party module.
- `gpui::app` 482,032, `tokio::runtime` 469,000 (1,931 functions — broad and
  thin, not a hotspot), `gpui_base::text` 367,788, `gpui::window` 344,804,
  `gpui::element` 339,064, `gpui_component::theme` 332,468 (only 52 functions —
  theme token machinery is dense), `gpui::elements` 228,800,
  `rustybuzz::hb` 219,844.
- `tokio::runtime`'s 1,931 functions average ~243 B each: the async runtime is
  numerous but individually small, the opposite of theme/taffy code.

### 8.4 Code-level attribution: top 60 functions

| # | size | % of .text | crate | function (demangled) |
| :---: | ---: | ---: | :--- | :--- |
| 1 | 103,064 | 0.54 % | `gpui_component` | `<gpui_component::theme::theme_color::ThemeTokens>::reconcile` |
| 2 | 82,500 | 0.43 % | `gpui_component` | `<<gpui_component::theme::schema::ThemeConfigColors as serde_core::de::Deserialize>::deserial…` |
| 3 | 69,860 | 0.37 % | `taffy` | `taffy::compute::grid::track_sizing::resolve_intrinsic_track_sizes::<taffy::tree::taffy_tree:…` |
| 4 | 34,852 | 0.18 % | `gpui_component` | `<gpui_component::theme::theme_color::ThemeColor>::apply_config` |
| 5 | 34,516 | 0.18 % | `gpui_base` | `<gpui_base::input::element::TextElement<gpui_base::input::kind::InputMode> as gpui::element:…` |
| 6 | 32,760 | 0.17 % | `html5ever` | `<html5ever::tree_builder::TreeBuilder<alloc::rc::Rc<markup5ever_rcdom::Node>, markup5ever_rc…` |
| 7 | 29,696 | 0.16 % | `gpui` | `taffy::compute::flexbox::compute_flexbox_layout::<taffy::tree::taffy_tree::TaffyView<gpui::t…` |
| 8 | 27,548 | 0.14 % | `block_ui` | `<block_ui::Block>::render_page_body` |
| 9 | 25,640 | 0.14 % | `gpui_component` | `<<gpui_component::theme::schema::ThemeConfigColors as serde_core::de::Deserialize>::deserial…` |
| 10 | 25,344 | 0.13 % | `block_ui` | `<block_ui::Block>::game_wizard_body` |
| 11 | 25,088 | 0.13 % | `block_ui` | `<block_ui::Block>::render_surface_dialog_body` |
| 12 | 24,152 | 0.13 % | `block_ui` | `<block_ui::Block>::browser_detail_content` |
| 13 | 22,740 | 0.12 % | `usvg` | `usvg::text::layout::layout_text` |
| 14 | 22,280 | 0.12 % | `gpui_base` | `<gpui_base::text::node::BlockNode>::render_table` |
| 15 | 22,144 | 0.12 % | `resvg` | `resvg::filter::apply_inner` |
| 16 | 21,920 | 0.12 % | `block_ui` | `<block_ui::Block>::poll` |
| 17 | 21,688 | 0.11 % | `image_webp` | `<image_webp::lossless::LosslessDecoder<core::io::util::Take<&mut core::io::cursor::Cursor<&a…` |
| 18 | 21,368 | 0.11 % | `block_ui` | `<block_ui::Block>::render_onboarding_transition_body` |
| 19 | 20,780 | 0.11 % | `block_ui` | `<block_ui::Block>::handle` |
| 20 | 20,700 | 0.11 % | `html5ever` | `<html5ever::tokenizer::Tokenizer<html5ever::tree_builder::TreeBuilder<alloc::rc::Rc<markup5e…` |
| 21 | 20,544 | 0.11 % | `gpui_base` | `<gpui_base::text::node::BlockNode>::render_block` |
| 22 | 20,368 | 0.11 % | `exr` | `<exr::meta::header::Header>::read::<exr::io::Tracking<core::io::cursor::Cursor<&[u8]>>>` |
| 23 | 20,352 | 0.11 % | `image_webp` | `<image_webp::lossless::LosslessDecoder<core::io::util::Take<&mut core::io::cursor::Cursor<&[…` |
| 24 | 19,960 | 0.10 % | `block_ui` | `<block_ui::Block>::home` |
| 25 | 19,336 | 0.10 % | `image_webp` | `<image_webp::lossless::LosslessDecoder<&mut core::io::util::Take<&mut core::io::cursor::Curs…` |
| 26 | 19,332 | 0.10 % | `image_webp` | `<image_webp::lossless::LosslessDecoder<core::io::util::Take<&mut core::io::cursor::Cursor<&[…` |
| 27 | 18,780 | 0.10 % | `block_ui` | `<block_ui::engine::Engine>::sign_in::{closure#0}::{closure#0}` |
| 28 | 18,368 | 0.10 % | `image_webp` | `<image_webp::lossless::LosslessDecoder<&mut core::io::util::Take<&mut core::io::cursor::Curs…` |
| 29 | 18,252 | 0.10 % | `block_launcher` | `<block_launcher::library::Library>::install::{closure#0}` |
| 30 | 18,204 | 0.10 % | `gpui_base` | `<gpui_base::text::node::Paragraph>::render` |
| 31 | 17,892 | 0.09 % | `block_ui` | `<block_ui::engine::Engine>::play::{closure#0}::{closure#0}` |
| 32 | 17,764 | 0.09 % | `gpui_component` | `<gpui_component::button::button::Button as gpui::element::RenderOnce>::render` |
| 33 | 17,604 | 0.09 % | `gpui` | `<gpui::window::Window>::new` |
| 34 | 17,568 | 0.09 % | `gpui_component` | `<gpui_component::theme::schema::ThemeConfigColors as core::clone::Clone>::clone` |
| 35 | 17,440 | 0.09 % | `gpui_component` | `<gpui_component::dialog::dialog::Dialog as gpui::element::RenderOnce>::render` |
| 36 | 17,432 | 0.09 % | `gpui` | `<gpui::window::Window>::draw` |
| 37 | 17,264 | 0.09 % | `block_launcher` | `block_launcher::install::neoforge::install_staged::{closure#0}` |
| 38 | 16,380 | 0.09 % | `gpui_base` | `gpui_base::text::inline_flow::line_ranges` |
| 39 | 16,232 | 0.09 % | `block_ui` | `<block_ui::crash_view::CrashView as gpui::element::Render>::render` |
| 40 | 16,052 | 0.08 % | `block_ui` | `<block_ui::Block>::settings_section_body` |
| 41 | 15,824 | 0.08 % | `clap_builder` | `<clap_builder::parser::parser::Parser>::get_matches_with` |
| 42 | 15,624 | 0.08 % | `block_ui` | `<&mut serde_json::de::Deserializer<serde_json::read::SliceRead> as serde_core::de::Deseriali…` |
| 43 | 15,592 | 0.08 % | `gpui` | `<gpui::app::App>::flush_effects` |
| 44 | 15,508 | 0.08 % | `block_launcher` | `block_launcher::install::forge::read_installer::{closure#0}` |
| 45 | 15,176 | 0.08 % | `usvg` | `usvg::parser::filter::collect_children` |
| 46 | 14,912 | 0.08 % | `gpui_component` | `<&mut serde_json::de::Deserializer<serde_json::read::StrRead> as serde_core::de::Deserialize…` |
| 47 | 14,884 | 0.08 % | `block_ui` | `<block_ui::Block>::render_profile_editor` |
| 48 | 14,828 | 0.08 % | `block_launcher` | `block_launcher::resolver::resolve_inner::{closure#0}` |
| 49 | 14,788 | 0.08 % | `gpui` | `<image::codecs::openexr::OpenExrDecoder<core::io::cursor::Cursor<&[u8]>> as image::io::decod…` |
| 50 | 14,748 | 0.08 % | `block_ui` | `<block_ui::feedback::ToastLayer as gpui::element::Render>::render` |
| 51 | 14,516 | 0.08 % | `block_ui` | `<block_ui::Block as gpui::element::Render>::render` |
| 52 | 14,500 | 0.08 % | `block_ui` | `<block_ui::Block>::render_browser_detail` |
| 53 | 14,372 | 0.08 % | `gpui_component` | `<<gpui_component::theme::color::ShadcnColors as serde_core::de::Deserialize>::deserialize::_…` |
| 54 | 14,104 | 0.07 % | `block_ui` | `<block_ui::Block>::render_shell` |
| 55 | 14,016 | 0.07 % | `taffy` | `taffy::compute::block::compute_inner::<taffy::tree::taffy_tree::TaffyView<gpui::taffy::NodeC…` |
| 56 | 13,424 | 0.07 % | `block_ui` | `<block_ui::drawer::DrawerLayer as gpui::element::Render>::render` |
| 57 | 13,304 | 0.07 % | `http` | `<http::header::name::StandardHeader>::from_bytes` |
| 58 | 13,272 | 0.07 % | `exr` | `<exr::image::read::layers::FirstValidLayerReader<exr::image::read::specific_channels::Specif…` |
| 59 | 13,236 | 0.07 % | `block_launcher` | `block_launcher::resolver::resolve_managed_runtime::{closure#0}` |
| 60 | 12,964 | 0.07 % | `hyper_util` | `<hyper_util::client::legacy::client::Client<reqwest::connect::Connector, reqwest::async_impl…` |

Crates for the top 5: `gpui_component::theme::theme_color::ThemeTokens::reconcile`
(103,064 B), the generated serde visitor for `ThemeConfigColors` (82,500 B),
taffy's grid `resolve_intrinsic_track_sizes` monomorphised for
`TaffyView<gpui::taffy::NodeContext, …>` (69,860 B), `ThemeColor::apply_config`
(34,852 B) and `gpui_base`'s `TextElement::prepaint` (34,516 B).

Readings:

- **There is no single-function hotspot**: the largest function is 0.54 % of
  `.text`; top 10 functions = 465,780 B
  (2.45 %), top 60 = 1,343,356 B (7.07 %), top 500 = 4,123,968 B (21.7 %). Size is
  a long tail of 44,724 methods, which is why only profile-level changes (strip,
  LTO, abort) move it.
- The visible patterns are exactly the generic/monomorphisation families:
  **GPUI theme machinery** (serde-generated visitors + token reconciliation +
  colour application), **taffy layout** monomorphised twice (grid in `taffy`,
  flexbox inside `gpui`), **GPUI element/text framework**
  (`TextElement::prepaint`, `BlockNode::render_table`/`render_block`, text node
  render), **Block UI render functions** (`render_page_body`, `game_wizard_body`,
  `render_surface_dialog_body`, `browser_detail_content`, `poll`, `handle`,
  `home` — a first-party cluster of roughly 0.2 MB), **SVG/HTML parsing**
  (`html5ever` tree builder 32,760 B, tokenizer 20,160 B, `usvg::text::layout`
  22,740 B, `resvg::filter::apply_inner` 22,144 B) and **image codecs**
  (`image_webp` lossless decode ~21 KB × three monomorphisations, `exr` header
  read 19,900 B).

### 8.5 Symbol-table attribution (the 24.8 % payload)

Exact: 86,423 symbols; `nlist` 1,382,768 B; name strings 7,929,136 B; total
9,311,904 B = 24.8 % of the release file (avg 91.7 B per name after the linker's
suffix sharing; 108 B unshared). By symbol kind:

| symbol kind | symbols | payload bytes | % of payload |
| :--- | ---: | ---: | ---: |
| Rust code, non-generic | 28,028 | 4,330,664 | 46.5 % |
| Generic machinery (`core`/`alloc`/`std`/`hashbrown`) | 21,545 | 3,485,548 | 37.4 % |
| Compiler/linker local labels | 14,616 | 765,995 | 8.2 % |
| Exception-table labels (`GCC_except_table*`) | 21,445 | 698,855 | 7.5 % |
| Undefined imports | 515 | 17,452 | 0.2 % |
| C/asm/system | 273 | 13,357 | 0.1 % |

Owner split by subsystem group, and the top 20 individual owners:

| symbol owner group | symbols | payload bytes | % of symbol payload | % of file |
| :--- | ---: | ---: | ---: | ---: |
| `std-core` | 21545 | 3,485,550 | 37.4 % | 9.28 % |
| `gpui-framework` | 8678 | 1,415,597 | 15.2 % | 3.77 % |
| `(compiler local labels)` | 14616 | 765,995 | 8.2 % | 2.04 % |
| `net-tls-async` | 4697 | 730,732 | 7.8 % | 1.95 % |
| `(exception tables)` | 21445 | 698,855 | 7.5 % | 1.86 % |
| `other-utility` | 4736 | 651,832 | 7.0 % | 1.74 % |
| `serde-json` | 2138 | 525,550 | 5.6 % | 1.40 % |
| `first-party` | 2453 | 371,648 | 4.0 % | 0.99 % |
| `image-svg-font` | 2784 | 321,840 | 3.5 % | 0.86 % |
| `regex` | 934 | 99,912 | 1.1 % | 0.27 % |
| `objc-apple` | 829 | 96,881 | 1.0 % | 0.26 % |
| `async-util` | 335 | 67,073 | 0.7 % | 0.18 % |
| `(C/ObjC/imports)` | 788 | 30,809 | 0.3 % | 0.08 % |
| `clap-cli` | 250 | 26,477 | 0.3 % | 0.07 % |
| `compression-archive` | 182 | 21,320 | 0.2 % | 0.06 % |
| `crypto-hash` | 12 | 1,796 | 0.0 % | 0.00 % |
| `(Mach-O bookkeeping)` | 1 | 33 | 0.0 % | 0.00 % |

| # | symbol owner | symbols | payload bytes | % of payload |
| :---: | :--- | ---: | ---: | ---: |
| 1 | `core` | 15616 | 2,452,225 | 26.3 % |
| 2 | `gpui` | 5467 | 1,005,842 | 10.8 % |
| 3 | `(compiler local labels)` | 14616 | 765,995 | 8.2 % |
| 4 | `alloc` | 4172 | 744,968 | 8.0 % |
| 5 | `(exception tables)` | 21445 | 698,855 | 7.5 % |
| 6 | `tokio` | 2449 | 472,797 | 5.1 % |
| 7 | `serde_json` | 1297 | 297,284 | 3.2 % |
| 8 | `anyhow` | 2026 | 283,755 | 3.0 % |
| 9 | `gpui_base` | 1723 | 214,920 | 2.3 % |
| 10 | `block_ui` | 1355 | 206,326 | 2.2 % |
| 11 | `serde_path_to_error` | 652 | 190,350 | 2.0 % |
| 12 | `hashbrown` | 655 | 139,602 | 1.5 % |
| 13 | `std` | 1015 | 138,760 | 1.5 % |
| 14 | `block_launcher` | 639 | 89,941 | 1.0 % |
| 15 | `image` | 521 | 74,011 | 0.8 % |
| 16 | `async_task` | 247 | 73,641 | 0.8 % |
| 17 | `rustls` | 642 | 70,602 | 0.8 % |
| 18 | `smallvec` | 307 | 64,828 | 0.7 % |
| 19 | `gpui_component` | 459 | 60,885 | 0.7 % |
| 20 | `regex_automata` | 424 | 48,133 | 0.5 % |

Readings:

- The payload is **dominated by third-party monomorphised names**: `core` alone
  2,452,225 B (26.3 %), plus `alloc` 744,968 B, `hashbrown` 139,602 B and `std`
  — the generic-machinery group is 3,485,550 B (37.4 %). GPUI stack:
  1,415,597 B (15.2 %, `gpui` alone 1,005,842 B). serde/JSON: 525,550 B (5.6 %).
  Networking/TLS/tokio: 730,732 B (7.8 %).
- **First-party crates own only 371,648 B (4.0 %) of the payload** although they
  own 25.6 % of the code: their names are short (`block_ui::Block::render_page_body`),
  while generic third-party items produce very long v0-mangled names. Symbol cost
  is not proportional to code cost (e.g. `anyhow` carries 283,755 B of names for
  ~116,612 B of code, ~2.4×; `block_ui` 206,326 B of names for 1,305,196 B of
  code, 0.16× — both measured on the same basis).
- 765,995 B (8.2 %) are **compiler/linker local labels** and 698,855 B (7.5 %)
  are **exception-table labels** (21,445 of them). The latter exist *because* of
  `panic="unwind"`: one `GCC_except_tableN` label per landing-pad table. Both
  categories are pure linker bookkeeping and disappear with `strip = "symbols"`
  (measured: `nlist` 86,423 → 497, strings 7.9 MB → 10 KB in §3 option A).

### 8.6 Unwind / exception metadata attribution

Total unwind metadata is 4,365,160 B = 11.6 % of the release file:
`__eh_frame` 2,692,804 B, `__gcc_except_tab` 1,187,644 B, `__unwind_info`
484,712 B. `__eh_frame` parses exactly: 1,427 CIEs (33,588 B) + 34,838 FDEs
(2,659,216 B) = 2,692,804 B; the FDE bytes alone are 14.0 % of
`.text`. Every FDE was resolved to its function's crate:

| # | owner | FDEs | __eh_frame bytes (exact) | % of __eh_frame | est. total unwind bytes |
| :---: | :--- | ---: | ---: | ---: | ---: |
| 1 | `core` | 12120 | 956,940 | 36.0 % | 1,558,751 |
| 2 | `gpui` | 3924 | 287,396 | 10.8 % | 468,137 |
| 3 | `alloc` | 2936 | 218,496 | 8.2 % | 355,906 |
| 4 | `tokio` | 1657 | 122,116 | 4.6 % | 198,914 |
| 5 | `serde_json` | 1209 | 96,016 | 3.6 % | 156,400 |
| 6 | `block_ui` | 982 | 79,924 | 3.0 % | 130,187 |
| 7 | `gpui_base` | 949 | 73,612 | 2.8 % | 119,906 |
| 8 | `hashbrown` | 506 | 45,496 | 1.7 % | 74,108 |
| 9 | `std` | 679 | 45,152 | 1.7 % | 73,548 |
| 10 | `serde_path_to_error` | 531 | 41,680 | 1.6 % | 67,892 |
| 11 | `anyhow` | 635 | 40,368 | 1.5 % | 65,755 |
| 12 | `block_launcher` | 426 | 33,940 | 1.3 % | 55,285 |
| 13 | `rustls` | 434 | 33,408 | 1.3 % | 54,418 |
| 14 | `markdown` | 411 | 26,796 | 1.0 % | 43,648 |
| 15 | `regex_automata` | 355 | 24,808 | 0.9 % | 40,410 |
| 16 | `gpui_macos` | 284 | 22,528 | 0.8 % | 36,696 |
| 17 | `smallvec` | 274 | 21,820 | 0.8 % | 35,542 |
| 18 | `gpui_component` | 249 | 19,188 | 0.7 % | 31,255 |
| 19 | `image` | 246 | 18,460 | 0.7 % | 30,069 |
| 20 | `usvg` | 220 | 18,172 | 0.7 % | 29,600 |

Structural vs crate-specific: `panic="abort"` removed 4,165,048 B (95.4 %) of the
4,365,160 B in the `strip` configuration (residual `__eh_frame` 62,208 +
`__gcc_except_tab` 11,472 + `__unwind_info` 126,432 = 200,112 B), and
4,252,400 B (97.4 %) with fat LTO (residual 112,760 B). So the unwind budget is
essentially **structural Rust panic machinery**, distributed roughly in
proportion to code (core 36.0 %, gpui 10.8 %, alloc 8.2 %, tokio 4.6 %,
serde_json 3.6 %, block_ui 3.0 %, gpui_base 2.8 %); it is not a crate-specific
hotspot. The residual after `panic="abort"` is C/ObjC-adjacent unwind from the
Apple platform glue (`objc2`/AppKit), which no Rust profile setting removes.
Per-owner `__gcc_except_tab`/`__unwind_info` values in the table are estimates
(FDE-proportional) — only `__eh_frame` is exact.

### 8.7 Hotspot ranking (the actual "重灾区")

Bytes are against the 37,551,520 B release artifact unless marked `.text`.
"Reclaimable" refers to the measured profile options in §3, not to untested
ideas.

| # | hotspot | bytes | share of file | evidence | inherent? | reclaimable |
| :---: | :--- | ---: | ---: | :--- | :--- | :--- |
| 1 | **Symbol table** (`nlist` + names) | 9,311,904 | 24.8 % | `LC_SYMTAB`; §8.5 | no — pure linker overhead | **yes, −9.38 MB** with `strip="symbols"` (measured, option A) |
| 2 | **Unwind/exception metadata** | 4,365,160 | 11.6 % | sections + 34,838 FDEs; §8.6 | structural to `panic=unwind` | **95–97 %** with `panic="abort"` (measured, A→C = −7.77 MB including landing-pad code) |
| 3 | **GPUI framework stack** | 5,565,892 `.text` | 14.82 % | 17 crates; §8.2 | yes — it *is* the UI framework | only indirectly (LTO/abort shrink all code ~28 %) |
| 4 | **First-party application code** | 4,868,876 `.text` | 12.97 % | 7 crates; §8.2 | yes — the app itself | feature/refactor work only; UI/launcher logic |
| 5 | **Monomorphised generic machinery** | 5,538,092 `.text` | 14.75 % | `core`/`alloc`/`std`/`hashbrown` paths; §8.3 | inherent to Rust monomorphisation | fat LTO + abort: `.text` 19.0 → 13.7 MB; E−C = −2.97 MB file |
| 6 | **Image / SVG / font stack** | 2,535,700 `.text` | 6.75 % | 22 crates; §8.2 | yes for icons/images/fonts | ~0.6–1.0 MB dead codecs, only by patching `gpui-pre` features |
| 7 | **TLS / HTTP / tokio** | 1,661,528 `.text` | 4.42 % | 21 crates; §8.2 | yes for auth, downloads, updates | no |
| 8 | **std/core/backtrace** | 1,863,108 `.text` | 4.96 % | 6 crates; §8.2 | yes | `backtrace` symbolication only (needs strip anyway) |
| 9 | **regex stack** | 656,132 `.text` | 1.75 % | 5 crates; one call site | no — one rule-matching call | ~0.6 MB by replacing the regex use in `block-launcher` |
| 10 | **clap CLI** | 252,152 `.text` | 0.67 % | 6 crates | mostly | ~0.25 MB by hand-rolled arg parsing |
| 11 | **Embedded assets** | 357,055 | 0.95 % | §2d | yes (fonts/icons/jar) | not worth it |

Rows 3–8 are **overlapping views of the same 19.0 MB of code** (a crate, a
group, a generic slice and dependency stacks), not additive; rows 1, 2, 11 and
the fixed costs below are file-level blocks. Non-attributable fixed costs (also
in §2a): constant/data sections 3,609,016 B, export trie 543,320 B, ad-hoc code
signature 291,248 B, C strings 255,501 B, dyld info 105,768 B.

Summary answer to "confirm the hotspots":

- No crate, module or function dominates: the largest crate is 15.9 % of `.text`
  (`block_ui`), the largest module 15.0 % (`core::ptr`), the largest function
  0.54 %. The 38 MB is an *additive* result of a large generic framework
  (`gpui*` 5.57 MB of code), the application (4.87 MB), Rust monomorphisation
  machinery (5.54 MB), an image/SVG/font stack (2.54 MB), TLS/HTTP/tokio
  (1.66 MB) and std/core (1.86 MB) — plus two large non-code blocks that the
  default profile keeps: **symbol names (9.31 MB, 24.8 %)** and **unwind tables
  (4.37 MB, 11.6 %)**.
- **Biggest third-party driver: the GPUI framework stack** — 5,565,892 B
  (29.3 % of `.text`, 14.82 % of the release
  file, 17 crates); the single largest third-party crate is `gpui` with
  2,113,488 B (5.63 % of the file), followed by `std` (1,781,692 B),
  `gpui_base` (1,680,400 B) and `gpui_component` (1,017,716 B).
- Biggest *reclaimable* items are the two non-code blocks (symbols and unwind
  tables) — together 13.68 MB, 36.4 % of the file — which is exactly what the
  measured profile options remove.

### 8.8 Reproduce this attribution

```sh
# code attribution: crate table + every function (JSON)
cargo bloat --release --locked -p block-app --bin block --target aarch64-apple-darwin \
  --crates -n 0 --message-format json > target/size-analysis/bloat-crates.json
cargo bloat --release --locked -p block-app --bin block --target aarch64-apple-darwin \
  -n 0 --message-format json > target/size-analysis/bloat-functions.json

# crate / module / function tables + generic-machinery rollup
python3 target/size-analysis/attrib_bloat.py

# symbol-table attribution (reads target/size-analysis/block-baseline directly)
python3 target/size-analysis/attrib_symbols.py

# unwind attribution (dwarfdump --eh-frame + LC_FUNCTION_STARTS)
python3 target/size-analysis/attrib_unwind.py

# markdown tables for this section
python3 target/size-analysis/make_attrib_tables.py
```

`cargo bloat` rebuilds `target/aarch64-apple-darwin/release/block` with
`strip=false` as a side effect; copy the release artifact before running it and
restore it afterwards if the evidence artifact must stay byte-identical (as done
here). The scripts are self-contained Python 3 (standard library only) plus
Xcode's `llvm-cxxfilt`/`dwarfdump`.

### 8.9 Limits of this attribution

- cargo-bloat sizes code by address ranges between symbols; its own output calls
  the numbers guesswork. Function-level bytes are approximate, ratios are good.
- Link-unit vs defining-path attribution differ under monomorphisation/inlining;
  both are reported rather than mixing them.
- The module rollup takes the first two path segments of the demangled name. For
  deeply nested or closure-heavy paths this is a heuristic, and generic
  instantiations are grouped by defining path (which is why `core::ptr` is large)
  while the per-crate table groups them by link unit.
- Symbol name bytes are suffix-sharing-scaled; per-owner string values are
  therefore estimates whose sum is exact. Per-owner `__gcc_except_tab` and
  `__unwind_info` are FDE-proportional estimates.
- One host/toolchain, one commit (`--locked`); absolute byte counts will shift
  with dependency or toolchain updates. No runtime/performance claims.

## 9. Per-platform release-option and performance validation (2026-10-24)

Follow-up requested by the user after the four-platform motion check: on every
platform, measure how much each release optimization parameter compresses the
binary and what performance impact it has. **Measurement only** — no
`Cargo.toml`, packaging or source was changed; every variant is a
`CARGO_PROFILE_RELEASE_*` environment override built from the same snapshot
(`988c533` + the `select.rs` label tweak) in a disposable target dir. Windows
AMD64 and Linux AMD64 share one physical machine and were measured strictly one
after the other (decision 24); the ARM64 hosts and macOS ran separately.

Variants (cumulative, matching `target/motion-verify/SIZE-PERF-ADDENDUM.md`):
V0 baseline release; V1 `strip=symbols`; V2 +`panic=abort`; V3 +`lto=fat` +
`codegen-units=1`; V4 +`opt-level=s`. Performance variants P0/P3/P4 add
`CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=true` because the scenario/trace code is
`debug_assertions`-gated; debug assertions are identical across P0/P3/P4, so
only relative comparisons are valid.

### 9.1 Size matrix

Bytes (shipping profile, no debug assertions). Δ is against that platform's own
V0. Build wall times are single-run and load-dependent; order of magnitude only.

| Platform | V0 baseline | V1 strip | V2 +abort | V3 +fat LTO/CU1 | V4 +opt=s |
| --- | ---: | ---: | ---: | ---: | ---: |
| macOS ARM64 | 37,551,520 B<br>35.81 MiB | 28,168,128 B<br>−25.0 % | 20,402,640 B<br>−45.7 % | 17,427,824 B<br>−53.6 % | 13,481,776 B<br>−64.1 % |
| Windows ARM64 | 31,306,240 B<br>29.86 MiB | 31,303,168 B<br>−0.01 % | 20,396,032 B<br>−34.85 % | 18,212,864 B<br>−41.82 % | 13,490,176 B<br>−56.91 % |
| Windows AMD64 | 36,742,656 B<br>35.04 MiB | 36,743,680 B<br>+0.003 % | 23,117,824 B<br>−37.1 % | 20,970,496 B<br>−42.9 % | 15,117,824 B<br>−58.8 % |
| Linux ARM64 | 70,766,480 B<br>67.49 MiB | 47,791,320 B<br>−32.5 % | 38,812,632 B<br>−45.2 % | 31,731,936 B<br>−55.2 % | 23,802,032 B<br>−66.4 % |
| Linux AMD64 | 74,543,952 B<br>71.09 MiB | 53,720,872 B<br>−27.9 % | 43,339,176 B<br>−41.9 % | 36,714,064 B<br>−50.7 % | 26,651,600 B<br>−64.2 % |

Cumulative build-time cost (single cold build, same host): macOS 71/66/62/167/128 s
for V0–V4; Windows ARM64 (V0 cold 429.7 s, V1 254.5, V2 195.7, V3 352.3,
V4 201.6 s); Windows AMD64 502.0/476.4/371.5/729.2/496.0 s; Linux ARM64
251.2/247.3/182.9/304.0/198.3 s; Linux AMD64 870/885/729/941/659 s.

**Baselines differ by platform ABI, not by application code.** Linux ARM64's
70.8 MiB baseline is ~1.9× macOS and ~2.3× Windows ARM64; the extra weight is
ELF symbol/exception metadata and the Linux dependency set that the default
profile keeps. The same source tree therefore has very different absolute
numbers per platform, and percentages, not absolute MiB, are comparable across
rows.

Per-platform increments:

- **macOS ARM64:** strip is the single free win (−9.38 MiB, −25.0 % of the file)
  because the Mach-O symbol table (86,423 symbols) ships in the binary;
  `panic=abort` removes most unwind metadata (−20.6 pp); fat LTO adds a further
  −8.0 pp; `opt-level=s` a final −10.5 pp.
- **Windows ARM64:** `strip=symbols` is effectively a no-op (−0.01 %, −3,072 B)
  because MSVC/PE keeps debug information in a separate PDB and the EXE has
  almost no COFF symbol payload. The big levers are `panic=abort` (−34.8 pp of
  V0), fat LTO (−10.7 pp) and `opt-level=s` (−25.9 pp); total −56.9 %.
- **Windows AMD64:** `strip=symbols` is also a no-op (+0.003 %, +1,024 B) for the
  same PDB reason as Windows ARM64; `panic=abort` takes the binary to 22.05 MiB
  (−37.1 %), fat LTO/CU=1 to 20.00 MiB (−42.9 %) and `opt-level=s` to
  14.42 MiB (−58.8 %). The baseline (35.04 MiB) is close to macOS, unlike the
  ARM64 PE baseline (29.86 MiB), so the ABI explanation alone does not cover the
  platform spread — the dependency/codegen mix differs too.
- **Linux AMD64:** the ELF build behaves like Linux ARM64, not like PE:
  `strip=symbols` is the second-largest single win (−27.9 %), `panic=abort` adds
  −14.0 pp, fat LTO/CU=1 −7.2 pp and `opt-level=s` −27.4 % relative to V3; total
  −64.2 %. Its 71.1 MiB baseline is the largest measured.

### 9.2 Performance matrix

Optimized instrumented variants, F replay (`--scenario home-ready`, foreground,
10 actions), committed or supplemental analysis:

| Platform | Metric | P0 (baseline optimized) | P3 (V3 + DA) | P4 (V4 + DA) |
| --- | --- | ---: | ---: | ---: |
| macOS ARM64 | instrumented bytes / build | 51,460,864 B / 66 s | 22,447,232 B / 260 s | 16,947,856 B / 176 s |
| | F paints / foreground replay | 408 / 387 | 403 / 382 | 405 / 383 |
| | forward settle | 383.6–389.1 ms | 381.4–388.8 ms | 378.6–386.3 ms |
| | analyzer / stage order | pass, ordered | pass, ordered | pass, ordered |
| Windows ARM64 | instrumented bytes / build | 44,265,984 B | 23,849,472 B | 17,316,352 B |
| | F paints / foreground replay | 45 / 40 | 44 / 39 | 45 / 39 |
| | median paints per forward transition | 3 | 3 | 3 |
| | forward settle | 385.9 / 411.6 / 487.7 ms | 395.9 / 584.6 / 458.7 ms | 405.1 / 401.9 / 481.3 ms |
| | analyzer / stage order | fails on sparse paints | fails on sparse paints | fails on sparse paints |
| Linux ARM64 | instrumented bytes / build | 90,181,240 B / 208.8 s | 38,023,776 B / 376.2 s | 27,931,112 B / 202.5 s |
| | F foreground replay paints | 222 | 237 (+6.8 %) | 249 (+12.2 %) |
| | median paints per forward transition | 27 | 28 | 29 |
| | forward settle (median) | 400.7 ms | 398.2 ms | 395.5 ms |
| | analyzer / stage order | pass on replay slice | pass | pass |
| Windows AMD64 | instrumented bytes / build | 50,716,160 B | 27,326,976 B | 19,397,120 B |
| | F paints / foreground replay | 401 / 381 | 410 / 383 | 413 / 386 |
| | median paints per forward transition | 46 | 45 | 47 |
| | forward settle | 382.8–389.3 ms | 384.6–391.8 ms | 382.7–389.5 ms |
| | analyzer / stage order | pass, ordered | pass, ordered | pass, ordered |
| Linux AMD64 | instrumented bytes / build | not built (time) | not built (time) | not built (time) |
| | performance | covered by Windows AMD64 (same physical CPU) and Linux ARM64 (same ELF toolchain family); Linux AMD64 motion replay paints at ~11.7/s on this 2-vCPU software-GL guest | | |

Time-to-first-window for the shipping binaries (5 launches per variant, warm
rounds discarded):

| Platform | V0 median | V3 median | V4 median | Reading |
| --- | ---: | ---: | ---: | --- |
| Windows ARM64 | 210.7 ms (warm 203.5) | 211.2 ms (warm 209.5) | 208.9 ms (warm 208.4) | no measurable difference |
| Linux ARM64 | 159.8 ms | 159.6 ms | 161.5 ms | no measurable difference |
| Windows AMD64 | 271 ms (259–314) | 278 ms (258–322) | 271 ms (262–298) | no measurable difference |
| Linux AMD64 | not measured | not measured | not measured | not measured (same reason; no numbers claimed) |
| macOS ARM64 | not measured | not measured | not measured | paint-perf only |

### 9.3 Reading the performance data

- **No size-option-specific animation regression was observed.** On macOS,
  Windows AMD64 and Linux ARM64 the three P variants are statistically
  indistinguishable in paint volume, settle time and stage order (macOS
  403–408 paints, Windows AMD64 401–413, Linux ARM64 222–249); Linux ARM64's
  +6.8 %/+12.2 % paint volume with smaller binaries is a single-sample
  direction, not a regression. Windows ARM64 is uniform across P0/P3/P4 (3
  median paints per forward transition) and
  its sparse sampling is a property of the optimized build path on that VM, not
  of a size option: the plain P0 baseline is just as sparse, and a display
  stimulus control produced 48 vs 45 paints.
- **Timing resolution limits the Windows ARM64 comparison.** With 0–5 paints per
  380 ms transition, stage starts cannot be ordered; only paint volume,
  geometry (identical: origin/height fixed, width 360↔1114 clamped) and the
  380–490 ms settle band were compared. Choppiness of the optimized build on
  that VM deserves a non-VM follow-up; it is orthogonal to the profile options.
- **Startup is unaffected** by strip/abort/LTO/`opt-level=s` within noise
  (Windows ARM64 209–211 ms; Linux ARM64 160–162 ms; Windows AMD64
  271–278 ms; macOS not measured).
- **Linux AMD64 performance variants were not built** (Lead scope decision):
  the 2-vCPU software-GL guest needed 870 s for the baseline alone and the
  full eight-build recipe would have added ~2.5 h; its animation paint cadence
  is ~11.7 paints/s, the least informative host for optimized-build
  comparisons. Its sizes are fully measured; performance is covered by
  Windows AMD64 (same physical i7-6700, same option set, committed analyzer
  passes) and Linux ARM64 (same ELF/LLVM toolchain family). No Linux AMD64
  performance numbers are claimed.
- **`opt-level=s` remains performance-unvalidated for shipping.** The
  instrumented replays show no regression, but single samples on VMs with
  debug-assertion builds and sparse sampling are not a frame-time study. The
  §4 recommendation (do not adopt without motion/perf validation on real
  hardware) stands.
- Caveats common to every row: one build and one replay per variant; paint
  submissions, not compositor presentation; debug-assertion overhead is
  relative-only; shared/VM hosts make wall-clock build times and startup noisy;
  different platforms are not directly comparable in absolute numbers.

### 9.4 Per-platform recommendation summary (measured, **not applied**)

| Platform | Cheapest useful step | Best size/time step | Notes |
| --- | --- | --- | --- |
| macOS ARM64 | `strip=symbols` (−25.0 %, no build cost) | +`panic=abort` +fat LTO (−53.6 %) | stripping is the free half |
| Windows ARM64 | `panic=abort` (−34.9 %, no build cost) | +fat LTO +`opt-level=s` (−56.9 %) | `strip` is a no-op on PE |
| Windows AMD64 | `panic=abort` (−37.1 %, no build cost) | +fat LTO +`opt-level=s` (−58.8 %) | `strip` is a no-op on PE |
| Linux ARM64 | `strip=symbols` (−32.5 %) then `panic=abort` (−45.2 % cumulative) | +fat LTO +`opt-level=s` (−66.4 %) | largest baseline, largest absolute savings |
| Linux AMD64 | `strip=symbols` (−27.9 %) then `panic=abort` (−41.9 % cumulative) | +fat LTO +`opt-level=s` (−64.2 %) | largest baseline; ELF strip works |

Nothing in this section has been applied to the project. The behaviour tradeoffs
(stripped crash backtraces, `panic=abort` semantics, ~2.4× build time for fat
LTO) are described in §3 and §4. CI uploads the debug binary, so these numbers
affect distribution builds only (§7).

### 9.5 Evidence

- macOS ARM64: `target/motion-verify/macos-arm64/size-perf/` (`P0/P3/P4` traces,
  `measurement.json`, summaries) plus `docs/binary-size.md` §3 and
  `target/size-analysis/`.
- Windows ARM64: `target/motion-verify/windows-arm64/size-perf/summary.md`,
  `variants.tsv`, build logs, `perf/P0|P3|P4`, `startup.tsv`, `source.txt`.
- Linux ARM64: `target/motion-verify/linux-arm64/size-perf/summary.md`,
  `variants.tsv`, build logs, perf traces, `startup.tsv`, `tools/`.
- Windows AMD64: `target/motion-verify/windows-amd64/size-perf/summary.md`,
  `variants.tsv`, build logs, `perf/P0|P3|P4`, `startup.tsv`, `binaries.txt`.
- Linux AMD64: `target/motion-verify/linux-amd64/size-perf/summary.md`,
  `variants.tsv`, `build-V0..V4.log`, `build-info-V0..V4.txt`, `binaries.txt`,
  `host/` scripts.
