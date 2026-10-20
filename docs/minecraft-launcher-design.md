# Minecraft resource preparation and instance launch

Status: the recipe/plan/executor core and offline CLI MVP are implemented. This document also describes later hardening and compatibility work; concrete supported commands are documented in `recipes.md`.

## Scope and agreed constraints

The first implementation targets macOS, Windows, and Linux together. It accepts an already authorized Minecraft Java account session; Microsoft sign-in and refresh-token storage are a later integration. The target corpus is the 919 official Java Edition versions already captured in the integration fixtures, including snapshots, beta, and alpha releases.

The deliverable is an instance preparation and launch engine, with a reusable resource downloader underneath it. A successful download or Java process creation does not establish that a game version reaches its main menu. Compatibility results must identify the version, OS, process architecture, Java runtime, and any compatibility policy used.

Baseline validation targets are Windows x64, Linux x64, macOS x64, and macOS arm64. Other architectures remain representable, including Windows arm64, but require their own runtime/native availability and launch verification before being advertised. Version-specific OS/graphics requirements still apply.

## Evidence collected on 2026-10-20

The repository's complete version fixtures contain these Java requirements:

| Declared Java major | Versions |
| --- | ---: |
| 8 | 552 |
| 16 | 25 |
| 17 | 144 |
| 21 | 119 |
| 25 | 61 |
| Not declared | 18 |

The 18 missing declarations include 1.6-era versions. Their fallback policy must be explicit and tested; a newer Java installation is not an automatic substitute. The fixtures also contain four distinct main-class names, 349 legacy argument strings, 570 modern argument objects, and 60 `default-user-jvm` lists.

All 58 distinct asset-index documents referenced by the corpus were downloaded for research, verified against their declared SHA-1 and size, and inspected. Modern indexes contain `objects`; the `legacy` index sets `virtual: true`, and `pre-1.6` sets `map_to_resources: true`. These modes require named-file reconstruction in addition to object downloads. See the [official legacy index](https://launchermeta.mojang.com/v1/packages/770572e819335b6c0a053f8378ad88eda189fc14/legacy.json) and [official pre-1.6 index](https://launchermeta.mojang.com/v1/packages/3d8e55480977e32acd9844e545177e69a52f594b/pre-1.6.json).

The [official Java runtime catalog](https://piston-meta.mojang.com/v1/products/java-runtime/2ec0cc96c44e5a76b9c8b7c39df7210883d12871/all.json) and 29 nonempty runtime manifests across Linux, macOS, macOS arm64, Windows x64, and Windows arm64 were inspected. Runtime manifests describe files, directories, links, executable flags, and `raw`/`lzma` download alternatives. The catalog has no macOS arm64 `jre-legacy`, `java-runtime-alpha`, or `java-runtime-beta` entry; Windows arm64 has the same gaps. Linux arm64 has no platform entry in the inspected catalog.

These are metadata observations. No game binary, asset object, or Java executable was downloaded or launched during this design work. Research downloads remain outside the repository.

## Workspace boundaries

| Crate | Responsibility |
| --- | --- |
| `minecraft-manifest` | Existing version models; add typed asset-index and Java-runtime models and their real-data fixtures. No GPUI dependency. |
| `block-download` | Streaming HTTP transfers, integrity checks, immutable object cache, request deduplication, retries, cancellation, and progress. No Minecraft argument logic. |
| `block-launcher` | Rules, target/runtime selection, resource planning, instance materialization, argument compilation, session binding, and process supervision. Internal modules initially; no crate per subsystem. |
| `block-cli` | Exercise planning, preparation, and launch independently of the desktop UI. |
| `block-app` / `block-ui` | Invoke the same engine and display structured progress, diagnostics, and instance state. |

The existing experimental blocking downloader remains a research utility. The production engine uses an asynchronous HTTP client and one managed Tokio runtime. GPUI submits work and consumes events without blocking its UI executor; background tasks never access GPUI entities directly.

## Preparation and launch pipeline

```text
InstanceSpec + HostEnvironment + account capabilities
    -> resolve verified metadata
    -> choose execution architecture and Java runtime
    -> compile deterministic PreparePlan
    -> download missing objects
    -> materialize libraries, assets, runtime, and run-local natives
    -> publish PreparedInstance + preparation lock
    -> bind a current AuthorizedSession
    -> build LaunchCommand
    -> spawn and supervise GameProcess
```

Metadata resolution fetches the version document, asset index, runtime catalog, and selected runtime manifest before the final resource graph is compiled. Each referenced document is verified before use. Resource planning is then a pure function over those documents and an explicit target context. Tests can compile a Windows or Linux plan on macOS without consulting the test host.

The first pass can expose metadata-discovery progress; it must not pretend to know the final asset count or byte total before resolving the asset index. The final plan contains resource identities, expected sizes, sources, destinations, and preparation actions, with source-document hashes and compatibility-policy revision recorded for reproducibility.

Proposed conceptual API:

```rust
resolve_metadata(spec, target, policy) -> ResolvedMetadata
compile_plan(spec, target, metadata, policy) -> PreparePlan
prepare(plan, store, cancellation, events) -> PreparedInstance
bind_session(prepared, session, options) -> LaunchCommand
launch(command, events) -> GameProcess
```

Resolution and preparation are asynchronous; plan compilation and command construction are deterministic. Account-dependent feature selection uses explicit non-secret capabilities in the plan and is checked again when binding the session. A changed capability set requires a new plan. An expired session can be replaced without redownloading resources.

## Rules and platform context

Keep host OS/architecture, selected Java process architecture, and JVM-reported architecture distinct. An arm64 launcher can select an x64 game process where an installed translation layer and matching x64 native libraries permit it. A platform label alone does not prove those prerequisites exist.

- Evaluate library and argument rules in their original order. An absent or empty rule list allows the item; a nonempty list starts denied and its last matching rule decides. Conditions within one rule are combined, including feature flags and OS constraints.
- Preserve separate handling for OS-name matching, OS-version regular expressions, architecture aliases, and `versionRange`. The current corpus's historical OS regex is simple, but unsupported regex syntax must not silently become a match.
- Missing feature flags default to false. Disabled quick-play and custom-resolution features do not introduce unused placeholder requirements.
- `${arch}` inside native classifier names is the documented bit-width substitution, not a Rust target-triple or arbitrary architecture string.
- Add differential fixtures for the new Windows `versionRange` boundary at `10.0.17134`. Its min/max inclusivity must be established before those GC-selection rules are marked supported; do not guess from a field name.

The rule engine is our own implementation of the observed manifest format. Prism's transformed metadata and [rule implementation](https://github.com/PrismLauncher/PrismLauncher/blob/develop/launcher/minecraft/Rule.cpp) are useful comparison points, not a complete specification for every raw Mojang field.

## Resource graph

For a client instance, select only the required client JAR, applicable library artifacts and native classifiers, asset index and asset objects, optional logging configuration, and selected Java runtime. Server binaries and mapping files are not client launch prerequisites.

Preserve classpath order from the selected library list, with an explicit tested position for the client JAR. Deduplicate downloads by content identity without sorting the classpath by hash. Conflicting selected files with the same logical path and different contents must become an explicit planning error or reviewed compatibility transformation.

Asset objects use `https://resources.download.minecraft.net/<first-two-hash-characters>/<hash>` and are verified against the asset index. For `virtual`, materialize the named layout under the appropriate asset view. For `map_to_resources`, materialize named files in the instance's game resources directory. `${game_assets}`, `${assets_root}`, and `${assets_index_name}` refer to the selected layout, rather than one hardcoded directory. Prism's [asset reconstruction implementation](https://github.com/PrismLauncher/PrismLauncher/blob/develop/launcher/minecraft/AssetsUtils.cpp) provides an independent behavioral reference.

Support both native-library arrangements seen in the corpus: separate classifier archives with `natives`/`extract`, and platform-qualified native JARs already selected as ordinary artifacts. Apply extraction exclusions, preserve required archive contents, and do not extract every classifier for every platform. Native extraction happens into a new run-local directory, never a shared mutable native folder.

## Download executor and storage

Each download request has a verified content identity, expected size, one or more permitted source URLs, and resource provenance. SHA-1 follows the upstream metadata contract; it is an integrity comparison, not a signature. Version-index roots are fetched over HTTPS and their snapshots are pinned for an instance preparation.

Initial executor policy:

- Stream with bounded memory; begin with eight global transfers and four per host as configurable defaults, not claimed performance optima. Bound queued work as well as active connections.
- Coalesce requests for the same digest and coordinate cache publication across concurrent preparation jobs/processes.
- Write to a unique temporary file beside its final cache destination, hash while streaming, validate size and digest, then publish atomically under a per-object lock. A partial file never counts as a cache hit.
- Retry connection/timeouts, HTTP 408/429, and eligible 5xx failures with bounded backoff and `Retry-After` handling. Permanent errors retain version/resource/URL context. Repeated hash mismatches fail with an integrity diagnostic.
- Cancellation stops scheduling new work and removes uncommitted temporary data. The first implementation restarts incomplete objects. HTTP range resume is a later enhancement requiring validated 206/`Content-Range`, source validators, and a final full hash.
- Offline preparation either produces the same valid prepared instance from cached objects or returns an exact missing-resource list. It never silently substitutes a different library/runtime.

Proposed layout:

```text
data-root/
  objects/sha1/<prefix>/<digest>                  # Immutable downloaded bytes
  metadata/<kind>/<digest>.json
  views/libraries/<manifest-digest>/<maven-path>
  views/assets/<asset-index-digest>/
    indexes/<asset-index-id>.json
    objects/<prefix>/<digest>
    virtual/<asset-index-id>/<logical-path>
  runtimes/<component>/<target>/<runtime-manifest-digest>/
  instances/<instance-id>/
    instance.json
    prepared.lock.json
    game/                                       # Mutable saves/config/resources
    runs/<run-id>/natives/
    runs/<run-id>/logs/
```

Versioned views prevent one asset-index ID or Maven path from silently changing underneath an older instance. Immutable views may use supported hard links with copy fallback; writable legacy resources use independent copies so game writes cannot alter shared objects. Do not require privileged Windows symlink creation for ordinary assets/libraries.

Path resolution must stay within the selected root, including archive entries, asset logical names, Windows drive/UNC syntax, and runtime links. Runtime links may legitimately use relative components; validate the resolved destination, not just individual path text. Reject escape paths and link cycles. Handle case-insensitive collisions explicitly on Windows and applicable macOS filesystems.

An instance is marked prepared only after downloads, materialization, and verification complete. Publish the preparation lock atomically. Preparation/launch locks prevent incompatible simultaneous operations on one mutable game directory, while distinct instances share immutable objects.

## Java runtime and legacy compatibility

Runtime selection order is an instance override validated for major/architecture, an installed managed runtime matching the declared component, then the matching official runtime download. An explicit compatibility profile supplies Java 8 for reviewed versions that omit the requirement. Missing platform/component combinations return a typed capability result rather than selecting the newest local Java.

Install the runtime manifest as a staged file tree, including directory entries, executable flags, and confined links. Prefer the `raw` variant initially; `lzma` is an optional bandwidth optimization with separate compressed/raw integrity checks. Locate the platform-specific Java entry point from the installed layout and probe its actual version and architecture before publishing the runtime as ready. Runtime upgrades use a new manifest-digest directory rather than replacing files belonging to running processes.

For old versions on macOS arm64, assess an x64 Java/native combination and detect an installed Rosetta capability. Do not assume an arm64 Java binary can load x64 JNI libraries or install system emulation as a side effect. Equivalent process-architecture checks apply to Windows arm64. A future external-runtime provider can address catalog gaps, but changing providers does not resolve an incompatible JNI or graphics dependency by itself.

Legacy support uses small reviewed compatibility profiles keyed to version/manifest and target, with reasons and regression fixtures. Candidates include old Java defaults, native-library filename adjustments, instance-directory isolation for ancient game code, and launcher/bootstrap behavior required by a specific entry point. Do not replace all old LWJGL versions or add a Java helper globally. Prism's [native extraction code](https://github.com/PrismLauncher/PrismLauncher/blob/develop/launcher/minecraft/launch/ExtractNatives.cpp) demonstrates one historical filename compatibility adjustment; whether it is needed here must be tested against our actual launch route.

## Arguments, session binding, and process lifecycle

Modern argument arrays retain their ordering and conditional expansion. Legacy argument templates are tokenized before placeholder substitution so a path or player name containing spaces remains one argument. Arguments are passed as `OsString` values to the process API; there is no shell command construction.

Prepare a typed placeholder context for all 26 names observed in the corpus. Distinguish account/session values, directories, classpath, launcher identity, asset layout, logging `${path}`, native classifier `${arch}`, and optional launch features. Unknown placeholders or missing required values fail before spawn. The legacy `${auth_session}` representation belongs to an account-session adapter and must have historical launch tests.

Required manifest JVM arguments stay intact. Evaluate `default-user-jvm` separately, then apply explicit user memory/GC settings with a documented precedence rule so mutually exclusive collectors or duplicate memory limits are not accidentally combined. Preserve macOS first-thread requirements where applicable and construct the classpath with the target platform's separator. Legacy manifests get a tested baseline JVM/classpath/native template because they do not provide a modern JVM array.

`AuthorizedSession` supplies player name, UUID, access token, expiration, account type, and optional XUID/client identifier when required. The caller guarantees that the session was legitimately obtained; this interface does not impersonate a login or prove ownership by accepting arbitrary text. Validate required fields and expiry immediately before spawn. A later account provider owns sign-in, entitlement/profile verification, refresh, and secure storage.

Preparation locks and diagnostic reports never serialize tokens. Session debugging and command previews redact secrets, as do captured logs. CLI session injection uses stdin or a protected session-provider interface rather than a token command-line flag. The actual Java launch may require the token in its argument vector; diagnostics must not imply that log redaction removes same-user OS process inspection.

Use an instance-specific working directory and run ID, drain stdout and stderr concurrently, preserve non-UTF-8 output safely, bound UI log buffering, and record exit status/crash paths. Unix process groups and Windows process/job handling implement explicit stop behavior, with bounded graceful-stop attempts where supported and a reliable terminate fallback. An abrupt forced termination is not presented as a clean save.

State progression is `Resolving -> Planning -> Downloading -> Materializing -> Ready -> Starting -> Running -> Exited`, with failure/cancellation exits. A spawned process enters `Starting`; actual launch acceptance requires evidence of the game reaching a usable screen. Metadata capability statuses distinguish a valid plan, missing runtime/emulation, required compatibility work, and an unverified real launch.

## Test and acceptance strategy

Keep all existing ordinary Cargo integration tests. Extend the same fixture approach with the 58 asset indexes and representative/all selected runtime manifests; network access is not part of normal `cargo test`.

1. Pure rule/plan tests run the complete 919-version corpus across explicit Windows/Linux/macOS target contexts, including arm64/x64 transitions. They verify selection, classpath order, asset layouts, required placeholders, and explicit capability diagnostics. A justified blocked plan is recorded distinctly from a supported launch.
2. Asset/runtime fixture tests validate original bytes, hashes, typed coverage, and lossless round trips. Rule boundary tests include absent/empty lists, allow/disallow ordering, unknown features, the Windows range boundary, and conflicting GC settings.
3. A local fake HTTP server exercises streaming limits, retries, truncated data, hash mismatch, cancellation, cache reuse, concurrent publication, and offline completeness. It serves deterministic fixtures and needs no official service or account.
4. Materialization tests exercise archive escapes, runtime link cycles, executable permissions, filename collisions, copy fallback, legacy layouts, and isolation between two instances. Run these on native Windows, Linux, and macOS CI runners.
5. Fake child processes verify argument boundaries, working directory, redaction, log draining, exit reporting, and stop behavior. They do not stand in for Minecraft launch proof.
6. Separate explicit real-launch tests use authorized sessions on actual OS/architecture runners with graphics. The initial representative matrix includes current Java 25 release/snapshot, 1.20.5 (21), 1.20.1/1.18.2 (17), 1.17.1 (16), 1.16.5/1.12.2 (8), 1.6.4 (missing Java declaration/virtual assets), 1.5.2/b1.7.3 (legacy assets/LaunchWrapper), and early RubyDung versions. Acceptance requires reaching the menu or usable game screen, resource reuse on a second run, instance isolation, and captured exit behavior.

The broader all-version campaign records results per version and target. Passing the representative matrix establishes the selected launch families; it does not establish 919 successful launches on every OS. No Windows/Linux runtime success may be reported from a macOS-only static or cross-compilation check.

## Implementation sequence

1. Extend asset/runtime models and real-data fixtures; implement explicit target context, rules, compatibility diagnostics, and pure resource planning. Establish native CI jobs for all three OS families immediately.
2. Implement `block-download` with fake-server integration tests and immutable cache publication. Deliver CLI `plan` and `prepare` operations before UI integration.
3. Implement managed Java, legacy asset reconstruction, per-run natives, and atomic instance preparation. Validate reproducible preparation and offline reuse across the baseline targets.
4. Bind supplied authorized sessions, launch/supervise processes, and run the representative real-game matrix. Resolve the Windows range boundary and identified legacy compatibility cases before declaring the affected targets supported.
5. Connect GPUI to the same engine and progress events, then expand the recorded version/platform launch matrix. Add Microsoft account integration through the established session-provider boundary afterward.

Each step adds Cargo integration coverage and passes `just check`. Scope is complete only when the selected versions reach a usable game screen on the declared targets, not when their download lists can be generated.
