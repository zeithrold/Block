# Minecraft version manifest parsing experiment

## Result

As checked on 2026-10-20, explicit Rust structs and enums cover both official Java Edition version index formats and all 919 version documents referenced by the captured official index. Every document passes strict deserialization and a lossless structural JSON round trip. No version was skipped, and no arbitrary JSON payload is stored in the schema models.

This is an empirical result for the current official corpus, not a published universal schema or a promise about future fields. It does not cover removed/unlisted versions, earlier revisions of metadata that Mojang has replaced, Bedrock metadata, or Forge/Fabric/custom launcher manifests.

## Sources and reproducibility

- [Official v2 index](https://piston-meta.mojang.com/mc/game/version_manifest_v2.json)
- [Official v1 index](https://piston-meta.mojang.com/mc/game/version_manifest.json)
- `snapshot/`: verbatim indexes captured for this experiment, including immutable per-version URLs and SHA-1 values.
- `report.json`: machine-readable coverage results and SHA-1 fingerprints of both indexes.
- `cache/`: ignored local copies of the indexes and downloaded version JSON documents. No game JARs, libraries, assets, or Java runtimes are downloaded.
- `../../crates/minecraft-manifest/tests/fixtures/official/`: the complete pinned corpus used by ordinary Cargo integration tests, independent of this cache.

From the repository root, run the recorded corpus through Rust's integration test harness:

```sh
cargo test --locked -p minecraft-manifest --test official_manifests
```

Or use the equivalent shortcut:

```sh
just manifest-verify
```

All 919 original version files and both full indexes are stored under the integration test fixtures (about 24 MB). Tests require neither network access nor the ignored download cache, and run by default in `cargo test --workspace`, `just test`, and `just check`. Missing, corrupt, or unindexed fixture files fail the suite; no full-corpus test is ignored or conditionally skipped. The test fixtures are frozen independently of later downloads.

The standalone CLI remains available to audit freshly downloaded metadata or regenerate a report. To audit whatever the official service currently publishes:

```sh
just manifest-fetch
cargo run --locked -p minecraft-manifest --bin manifest-audit -- verify
```

To redownload the original snapshot into the separate ignored cache and regenerate its report:

```sh
just manifest-fetch-snapshot
cargo run --locked -p minecraft-manifest --bin manifest-audit -- verify
```

Fetching uses a Rust HTTP client, eight bounded download workers, timeouts, and up to three attempts for HTTP/network failures. Existing files are reused only if their SHA-1 matches the index. Integrity and schema failures are reported, not treated as successful downloads or parsing results. Verification itself is offline. These commands overwrite the local cache indexes and coverage report; use the recorded snapshot to return to the original corpus.

The CLI also accepts a separate experiment directory:

```sh
cargo run --locked -p minecraft-manifest --bin manifest-audit -- fetch-snapshot /path/to/experiment
cargo run --locked -p minecraft-manifest --bin manifest-audit -- verify /path/to/experiment
```

For snapshot fetching, that directory must already contain both index files under `snapshot/`.

## Recorded coverage

| Version type | Documents |
| --- | ---: |
| Release | 103 |
| Snapshot | 755 |
| Old beta | 26 |
| Old alpha | 35 |
| Total | 919 |

The two full index documents also pass, for 921 JSON documents in total. The version documents contain 23,772,351 bytes and 53,896 library declarations.

| Schema variation | Versions |
| --- | ---: |
| Legacy `minecraftArguments` string | 349 |
| Modern `arguments` object | 570 |
| `default-user-jvm` argument list | 60 |
| No `javaVersion` | 18 |
| No `logging` | 114 |

The latest pointers in the recorded snapshot are release `26.3` and snapshot `26.4-snapshot-4`. The v2 index SHA-1 is `f9e02be69ab99dcd2626c25fc4263cbe3276c285`.

## Model design

`crates/minecraft-manifest/src/model.rs` contains explicit models for index entries, version documents, asset-index descriptors, game downloads, Java requirements, libraries, native classifiers, extraction settings, logging, launch arguments, rule conditions, and OS version ranges.

- Every fixed-shape object uses `deny_unknown_fields`, including objects nested inside untagged argument enums.
- Both argument generations have explicit fields. Argument entries are either strings or conditional objects; conditional values are either one string or a string array.
- Optional fields retain absence when serialized. Missing required fields fail parsing.
- Maps appear only where keys are data: feature names to booleans, OS names to native-classifier names, and classifier names to typed artifact descriptors. There is no `flatten` or `serde_json::Value` escape hatch in the model.
- Timestamps, URLs, regular expressions, Maven coordinates, and placeholders remain strings. This preserves their original text; the experiment verifies structural coverage, not their full semantic validity or launcher execution behavior.
- `serde_json::Value` is used only as an independent comparison representation in the verifier and tests. The JSON round trip compares all object keys, array order, and values, while ignoring whitespace and object-key order.

The full-corpus verifier also checks file SHA-1 against v2, duplicate IDs, agreement between index generations, latest-pointer membership, per-version ID/type agreement, and exactly one argument generation. Missing or corrupt documents produce failures and a nonzero exit status.

## Regression checks

```sh
just check
```

Eleven offline integration tests cover the full corpus and focused regressions. The three tests in `tests/official_manifests.rs` verify agreement and lossless parsing of both complete indexes, SHA-1 and lossless typed parsing for every version, exact fixture-directory membership, and the recorded corpus coverage totals. Failures identify the affected versions and are aggregated across the corpus.

The eight focused tests additionally cover seven verbatim version fixtures, sampled entries from both indexes, both argument generations, nested unknown-field rejection, wrong argument types, missing required fields, trailing JSON, successful CLI auditing, and aggregate reporting of missing/corrupt documents. Sample fixture provenance and hashes are recorded in `tests/fixtures/sources.json`; the two small index fixtures contain selected entries. Full-corpus provenance is recorded directly in the complete indexes under `tests/fixtures/official/`.
