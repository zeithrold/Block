# MVP implementation and acceptance

Independent per-session supervision and native crash assistance were added after the baseline gameplay/authentication acceptance below. Their implementation, failure boundaries and separate cross-platform verification results are recorded in [Session supervision and crash help](session-supervisor.md). Earlier baseline test counts do not replace that feature's complete verification runs.

The target workflow is Microsoft sign-in → Instance → Profile → install → Play → world → normal exit → relaunch. Fresh Microsoft-authorized Vanilla and NeoForge sessions completed world entry, normal exit and same-world relaunch on macOS. Computer Use drove the native interface and games; a temporary application bundle around the identical Java executable made game windows targetable. Platform and runtime limits remain explicit below.

## Implemented boundaries

- `Instance` persists the Minecraft version, installation directory and launch targets. Immutable assets, libraries and managed runtimes use a shared verified cache.
- `Profile` belongs to an Instance and selects its target, Java policy, memory, JVM/game arguments, optional default Account and mutable-data policy.
- `Shared` uses the Instance game directory; `Isolated` uses a Profile directory. Copying preserves settings and policy: Isolated copies get a new directory, while Shared copies keep sharing. Deletion removes metadata and preserves game data.
- Catalog updates use an exclusive transaction lock and atomic publication. A canonical game-directory lock remains held through Java exit to prevent concurrent writers.
- `Account` metadata is separate from protected credentials. Profiles and catalog data never contain refresh tokens.
- The existing serializable `ExecutionPlan` describes secret-free preparation. `LaunchPlan` is a transient resolved Java invocation with a verified session and held lifecycle locks; it cannot be serialized.
- `LaunchExecutor` supervises Java and redacted output. Authentication, installation and catalog persistence happen before execution.

The native interface provides Instance creation, Vanilla and representative NeoForge installation, Profile management, Account selection, progress, Repair and process lifecycle controls. Recipe-based offline development remains available separately.

## Acceptance gates

| Requirement | Status | Evidence and limits |
|---|---|---|
| Persistent Instance and multiple Profiles | PASS | Backend tests, Windows/Linux native UI CRUD and fresh-process persistence; macOS retained edited Profiles across launcher restarts |
| Shared/Isolated data and deletion safety | PASS | Lock/copy/deletion/transaction tests; a second Windows launch was refused before Java while the original game stayed alive; Debian gameplay preserved the original Profile data |
| Public Microsoft PKCE and protected credentials | PASS on macOS | Fresh public-client authorization, ownership/profile verification, protected credential save, fresh-process Account restoration, production Play refresh and native sign-out passed; other hosts still need fresh authorization |
| Authenticated Account/Profile selection and native Play | PASS on macOS | Computer Use selected dedicated Isolated Profiles and a newly authorized Account; NeoForge also persisted an explicit default Account. Each Play refreshed authorization before Java |
| Clean Vanilla installation | PASS | Native Debian AMD64 UI install from a fresh application data directory and separate clean macOS backend install |
| Representative NeoForge installation | PASS | Native macOS, Windows and Debian AMD64 installation of Minecraft 1.20.6 / NeoForge 20.6.144, verified manifests and Ready targets |
| Developer-offline world/exit/relaunch | PASS | Both targets completed this workflow on Windows and Debian AMD64: eight successful uncancelled sessions, each exiting 0 |
| Authenticated world/exit/relaunch | PASS on macOS | Vanilla and NeoForge each completed two uncancelled sessions, with world/HUD screenshots, normal exit, fresh launcher process and same-world relaunch; temporary Java bundle disclosed below |
| Repair and interrupted-install recovery | PARTIAL | Product corruption/refusal/repair, cancellation/retry and actual interrupted HTTP-body recovery passed; full native UI interruption/retry remains pending |
| Token redaction | PASS for tested outputs | Synthetic tests and four actual authenticated sessions: all command previews redact access tokens; twelve stable launcher command/stdout/stderr artifacts passed focused heuristic checks. Raw game crash files and arbitrary encodings were outside this audit |
| Strict local checks | PASS | Full macOS `just check`: formatting, all-target/all-feature checks, strict Clippy and 105 tests; Windows strict checks and full 100-test platform suite |
| Four-platform graphical acceptance | PARTIAL | Platform-specific limits below remain distinct from build and gameplay results |

## Platform results

| Platform | Build and checks | Native UI and installation | Gameplay and authentication |
|---|---|---|---|
| macOS ARM64 | PASS: full strict checks and 105 tests | PASS for focused workflows: native NeoForge install, Profile editing/default Account binding and normal close/reopen; clean backend Vanilla and NeoForge install/repair | PASS: fresh public PKCE, protected credential restoration/refresh/sign-out, and Vanilla/NeoForge authenticated world/exit/relaunch; gameplay used the identical-Java bundle wrapper |
| Windows AMD64 | PASS: strict checks, full 100-test suite, UI/CLI builds and synthetic native credential probe | PASS: Vanilla/NeoForge install, missing-account Play gate, Profile workflows and normal close/reopen | PASS for both developer-offline world/exit/relaunch workflows; authenticated Play BLOCKED |
| Linux AMD64 | PASS: full 105-test/check/Clippy baseline, native builds and subsequent focused fixture regression | PASS: native Vanilla/NeoForge install, Profile workflows, persistence and normal close | PASS for both developer-offline workflows; authentication BLOCKED by unavailable Secret Service and fresh authorization |
| Linux ARM64 | PASS: strict checks, test baseline, UI/CLI builds and focused source-overlay regressions | PASS: 11 focused native UI cases covering cache-assisted Vanilla Ready, missing-account Play gate, Profile workflows, data preservation and normal close/reopen | Fresh authorization, gameplay and NeoForge remain unverified |

Windows and Linux AMD64 workloads ran sequentially on their shared host. The Linux AMD64 guest was reachable through an SSH jump; direct bridge reachability remained limited. No network reconfiguration was counted as a product fix.

GPUI on Linux AMD64 enumerated a Vulkan CPU `llvmpipe` device. Minecraft separately reported VMware SVGA3D, Mesa and OpenGL 4.3 Core. Neither establishes physical GPU acceleration. Accepted screenshots used exact owned client bounds after direct OpenGL window capture proved clipped; Java was not restarted to repair capture.

Legacy ARM64 support and CPU emulation remain outside this MVP. Linux ARM64 managed Profiles locate and probe an installed matching native Java because the current official runtime catalog lacks that platform; system Java alternatives are not changed.

Linux ARM64 native acceptance used the already unlocked desktop without changing login or lock policies. UI instance creation reached Vanilla 1.20.6 Ready using 3,921 verified public cache objects and two Mojang metadata files; this was cache-assisted installation, not a clean download. No catalog, Profiles, Accounts or game data were copied. Play without an account required sign-in before Java execution.

The 11 focused cases also covered Profile creation/editing, Shared settings, copying settings into an Isolated Profile and deletion through the native interface. Deletion preserved all three synthetic data markers with exact bytes, hashes and modification times while retaining the other Profiles and Ready installation. Both repaired UI runs closed through the normal window protocol with exit 0; reopening the same data root preserved the exact catalog and rendered the saved settings. No ARM64 authentication, game launch or NeoForge installation was attempted in this acceptance.

## Gameplay and preservation

On macOS, Computer Use drove two authenticated Vanilla sessions and two authenticated NeoForge sessions. Both families showed terrain and HUD, used Save and Quit to Title followed by Quit Game, and returned to Block's normal-exit state. The launcher was normally closed and reopened between each family's two game sessions; the saved Account and Profile settings were restored and Play refreshed authorization again. NeoForge's persisted default Account was also verified. No developer-offline session was used for these four runs.

Fixed saved-world snapshots retained each family's exact signed 64-bit seed, name, version, player UUID, position and dimension. World time and LastPlayed advanced across each pair. Seven original Profile files retained their bytes, size and exact modification times. NeoForge testing also preserved the completed Vanilla save.

The initial authenticated macOS managed-Java attempt started correctly, but bare Java lacked an application identifier for Computer Use. It was stopped with native Stop and remains a cancelled targeting-adjustment attempt. The four passing game sessions instead selected a temporary registered application bundle as Custom Java; its executable was a hard link to the same managed JRE binary, with identical SHA-256, and the original runtime bytes and modification times remained unchanged. Direct managed-path gameplay is therefore not independently claimed from those four sessions. Block itself also used a temporary bundle of the unchanged built binary. These bundles were local test accommodations, not product source changes.

The explicit developer-offline probe used the actual `Library::launch` → `LaunchPlan::resolve` → `LaunchExecutor::run` path. It did not create Accounts, weaken the native Play gate or transfer personal credentials, worlds or caches between machines.

Each successful Vanilla/NeoForge pair created and entered a world with visible terrain/HUD, used Save and Quit followed by Quit Game, then selected the saved world in a distinct second Java process and repeated normal exit. Original lifecycle receipts record one Java start, exit 0 and no cancellation per session.

Read-only saved-world snapshots preserved exact signed 64-bit seeds. Both runs retained the same Profile, identity, directory, world name, seed and Minecraft version, while world time and LastPlayed advanced. Player positions were retained; input delivery alone was not counted as movement.

After all four Debian sessions, the original Default/Shared Profile records, paths, marker bytes/hashes/exact modification times and game trees were unchanged. Windows preserved all deliberate marker bytes/hashes and deleted-copy data; unchanged timestamps were not claimed without a baseline.

An earlier Windows attempt reached menus but timed out before world acceptance. Its cancelled receipt remains separate from the eight successful sessions. Test/controller failures were not relabeled as successful game exits. All owned test/game/UI processes were retired; detailed receipts and screenshots remain in private local acceptance artifacts rather than this repository.

## Authentication and recovery limits

Authentication uses a public desktop client ID, Authorization Code with PKCE, a one-use loopback callback, Xbox/XSTS and Minecraft identity/entitlement checks. Refresh rotation is stored before downstream calls; changed player UUIDs are rejected. Provider errors omit raw response bodies. Launch binding checks session validity after preparation, and output is redacted before publication.

Earlier confidential-client diagnostics do not prove public desktop registration. A synthetic invalid-code rejection or browser redirect is also insufficient. See [authentication setup](../crates/block-auth/README.md). No Client Secret or plaintext credential fallback is supported. Authentication on additional hosts still requires fresh user authorization; missing secure storage remains a blocker rather than a plaintext fallback.

A fresh macOS attempt reproduced a registration rejection with the original callback: the same localhost path belonged to Native, Web and SPA registrations differing only by port. Block now uses the distinct Native-only `/block/callback` path. The existing application gained only that Native URI; previous callbacks, account audience and permissions were preserved. A new browser authorization then completed the actual public PKCE, Xbox/Minecraft ownership/profile and protected credential-save chain successfully. The callback regression rejects the old path and accepts the matching state on the new one. Full `just check` passed 105 tests after the change.

The same binary was reopened in a temporary macOS application bundle so Computer Use could target it. Fresh processes restored the saved Account. Computer Use edited dedicated Isolated Profiles and clicked native Play; the production authorization-refresh path succeeded before each launch. After both game pairs, native Sign out reported saved credentials removed and no saved Accounts remained. Play with the former default Account was then refused with an explicit unavailable-account message and returned to Accounts. Instances, Profiles and worlds remained intact. The preceding unbundled UI was retired with an owned-process signal; that retirement is not normal-close evidence.

The launch identity currently leaves XUID empty; external multiplayer compatibility remains unverified even though authenticated single-player gameplay passed. The product Library immediately executes a resolved LaunchPlan; an external caller that delays execution does not get another expiry check after binding.

Repair was tested with equal-length NeoForge corruption: Play refused before Java, then Repair retained corrupt bytes for diagnosis and restored expected immutable content without changing a save marker. A separate installation cancellation retried the same IDs to Ready while preserving mutable-data sentinels.

An actual loopback HTTP server disconnected partway through declared response bodies. Failed attempts published no cache object and retained no partial file. Restoring the endpoint passed the digest and complete-byte check; later offline reuse made no new request. These boundary checks do not establish complete native UI network-failure acceptance.

Linux now uses GPUI's existing final-window shutdown behavior to avoid a reentrant close callback. A separate timestamp test fixture requests the non-truncating writable handle Windows requires; production installer code and assertions were unchanged. Its focused regression passed on all four environments. Targeted reruns were not presented as new full suites.

## Reproduction

```sh
just check
just build
```

These commands use locked workspace dependencies. The [CI workflow](../.github/workflows/check.yml) runs independent core checks on Linux, Windows and macOS, plus desktop checks and GPUI framework tests. The captured 919-version corpus exercises parsing/planning, not graphical gameplay for every version.

Real authentication requires a personal-account-capable desktop registration, an available native credential backend and new browser authorization on each host. Do not copy credentials between machines. Graphical acceptance requires an unlocked usable desktop. A commit or CI result does not resolve the pending authentication gates.

## Deferred extensions

1. **authlib-injector:** an explicitly trusted HTTPS Yggdrasil provider, distinct Account type/credential namespace, verified Java agent and concrete compatibility checks. Never pass Microsoft tokens to that provider or present it as Microsoft ownership verification.
2. **Local CurseForge ZIP import:** validate manifest/Loader compatibility and extraction paths/types/resource limits in isolated staging; fetch permitted dependencies, explain restricted/manual downloads, apply overrides to the new Profile and reuse ordinary installation/launch. Never execute archive scripts or embed an unapproved API key.

Both follow stable Microsoft/Vanilla/NeoForge acceptance. Historical Loader expansion, an online marketplace, WASM plugins and generic installer frameworks remain outside this MVP.
