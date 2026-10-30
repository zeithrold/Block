# Official Minecraft metadata fixtures

`official/` contains both complete Mojang version indexes captured on 2026-10-20 and the original bytes of all 919 referenced version documents. The 919 files in `official/versions/` are named by the SHA-1 supplied in `official/version_manifest_v2.json`, which also records each original URL and version ID. The captured index contains 103 releases, 755 snapshots, 26 old betas, and 35 old alphas.

`tests/official_manifests.rs` reads these files directly and runs in the normal Cargo integration test suite. It never downloads data, consults the experiment cache, or skips missing files. It checks typed parsing, lossless structural JSON round trips, hashes, index consistency, and exact corpus membership.

The top-level version files are smaller targeted samples of the same official data. `sources.json` records their provenance and hashes; the top-level index files contain only the entries needed by focused parser and CLI regression tests.

The download/report CLI operates on a separate ignored cache. Running it does not update this frozen regression corpus. An intentional corpus refresh must replace both complete indexes and their referenced version documents together and update the recorded count/coverage assertions in the integration tests. Preserve the original response bytes so their hashes remain valid.
