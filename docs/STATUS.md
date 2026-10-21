# Block Pilot Status

Updated: 2026-10-22. This is implementation status, not a completion claim.

## Baseline and foundations

- [x] Freeze the unchanged React product reference: Block-UI commit `fd2713b` (local only).
- [x] Migrate decisions and remove old evidence before staging; push retained baseline `5c634fe` to main.
- [x] Native-window/scenario baseline CI on Windows AMD64, Linux AMD64 and macOS ARM64 (`c82964f`).
- [x] Remove vendored framework patches; published GPUI 0.3.9 / Base 0.8.0 compile unchanged.
- [x] Fixed-size/native-chrome implementation; macOS real Welcome 960×600 and separate version picker 760×520 captured.
- [x] Independent dialog-window implementation: focus existing purposes, preserve drafts, reject stale results and close children with parents.
- [x] Native-window baseline CI and remote interactive smoke on all three test hosts.
- [x] Debug-only network-free scenario runner, native screenshots and comparison board; React shots remain pending.
- [x] UI-only Home → Games → native version-picker smoke on Windows AMD64, Linux AMD64 and Linux ARM64.

## Product decisions and intended differences

- The P0 React snapshot and README define product content; the current Goal's explicit decisions take precedence.
- New catalogs use Bundled mode: one Instance and exactly one associated Profile; names and appearance match Home.
- Existing catalogs without a mode keep Separated behavior. Confirmed Bundled-to-Separated conversion is one-way; Restore defaults preserves mode.
- Separated mode permits multiple Profiles per Instance. Deletion removes catalog metadata and preserves player files.
- Version, release type and loader/build are fixed after creation. Other editable fields use local drafts until Save.
- Section Done updates only the local draft; Cancel restores its checkpoint. Final creation saves the Instance/Profile pair atomically.
- Late results update actual catalog/session state, but cannot complete or clear a newer draft, wizard or authentication attempt.
- Home is 432×768; full management pages are 960×600; dialogs use content-appropriate fixed sizes. Fit to short displays at creation.
- macOS uses the upstream transparent titlebar and native traffic lights; Windows/Linux use system decorations.
- Dialogs/drawers become separate GPUI windows with interactive parents. Anchored menus/selects/tooltips use upstream Base components.
- Accessibility semantics, inert/backdrop machinery and paint scaling are removed; retain system and in-app reduced motion.
- First run keeps Your game → Your account → Ready to go. Skipping creates no identity and never bypasses launch authorization.
- Authentication approval prepares an identity; final confirmation saves it. Cancellation, expiry and method changes invalidate pending results.
- First-run creation installs a real game and returns Home while downloads continue. Download/Retry requires no signed-in account or duplicate game.
- Settings distinguish saved values, drafts and appearance previews; defaults apply only to newly created games. Library defaults affect new Instances; moving existing managed games is separate and preserves custom folders. Cache changes copy verified managed resources while account/control metadata stays at the stable root.
- Only official upstream download sources are offered; no BMCLAPI or other third-party mirrors.
- Download bytes, retries, verification and cache hits come from real task events. Unknown totals remain indeterminate.
- Connection testing uses the Downloads draft without saving, has bounded requests and rejects late results after draft changes.
- Player avatars must come from actual account data; initials are valid fallback, demo Steve/Alex images are not real skins.
- Server addresses save separately from auto-connect. Quick Play must follow actual manifest capabilities and compiled launch arguments.
- OptiFine: user downloads from optifine.net, then selects the JAR for automatic validation/installation; no bundling or redistribution.
- Legacy ARM64 is best effort: investigate viable official Java/native artifacts, test candidates and explain established incompatibilities.
- Use only dedicated test data directories; never copy player credentials/worlds or expose .env values.

## Page inventory (unchecked means implementation or verification remains)

### Home
- [ ] Profile/account selection and empty/loading/error states — persistent catalog, account store, independent selector windows.
- [ ] Play/Stop, recovery and crash help — real preparation, authorization, supervisor reconnect and cancellation.
- [ ] Downloads, retries and file details — verified transfer events and independent download/detail windows.

### Accounts and sign-in
- [ ] Account list/detail, current selection, deletion and last-account empty state — persistent accounts and secure token storage.
- [ ] Microsoft browser PKCE — ownership/profile verification prepares a memory-only identity; Finish saves credentials/metadata. Local cancellation/refresh/ownership checks pass; live verification remains pending.
- [ ] Microsoft Device Code — request/poll, interval/slow_down, HTTP throttling/timeout backoff, expiry/cancel, XBL → XSTS → Minecraft and final confirmation implemented; live verification remains pending.
- [ ] Offline account — explicit local identity, validated username/Java UUID v3, final-save confirmation, persistent selection/removal and launch dispatch implemented; CLI-backed game smoke passed, while native selection/Play remains pending.
- [ ] Authlib provider management — add/edit/remove, bounded ALI discovery, atomic public metadata persistence and linked-account endpoint/deletion protection implemented. Favicon URL is cached; icon image retrieval remains pending.
- [ ] Provider URL/quick-link import — input/paste, duplicate selection, explicit address-bound HTTP consent, redirect limits and HTTPS downgrade rejection implemented. External text drop requires the user decision below.
- [ ] Authlib account/password form and characters — real authentication, API-bound drafts/HTTP consent, masked ephemeral password, actual profile selection and final OS-store/metadata save implemented. Actual fixture-backed native form interaction remains pending; production OS-store verification passed separately below.
- [ ] Yggdrasil authenticate/refresh/validate/invalidate/signout and profile selection — protocol core and final-confirmation OS-store adapter implemented; local synthetic credential/HTTP fixtures pass. Desktop flows are wired; the production macOS keychain adapter passed the manual synthetic-account smoke below. Native form interaction remains pending.
- [ ] Authlib launch injection — official descriptor, origin-bound SHA-256 artifact download/cache repair, fresh provider metadata and private-session JVM injection implemented. Actual macOS ARM64 1.20.6 menu smoke passed with a local provider fixture; live-provider verification is separate.
- [ ] Wizard back/cancel/finish and providers — upstream anchored provider Select plus independent add-provider window, isolated drafts, duplicate selection and stale-result rejection implemented; native acceptance remains pending.

### Games and Game Wizard
- [ ] Bundled/Separated browsing, search, details and Home synchronization — real Instance/Profile joins and mode rules.
- [ ] Profile/Instance appearance and launch/settings editing — validation, immutable version/loader and transactional persistence.
- [ ] Profile icons/colors, memory, extra JVM arguments, inherited Java/window defaults, isolated/shared files and default account — persisted drafts compiled into actual launch recipes.
- [ ] Bundled two-step and Separated four-step creation — section drafts, existing-Instance selection and atomic final save.
- [ ] Version/loader/location pickers — real catalogs, compatibility filtering, retryable loading and independent windows.
- [ ] Vanilla installation — verified official metadata/resources, real progress, retries and resumable downloads.
- [ ] Fabric and Quilt — official catalogs, verified installation and native loader/build selection implemented in game creation and Welcome; complete native service interaction and the full smoke matrix remain pending.
- [ ] Forge modern/legacy — official filtered catalog, native selection, shared verified installer/processors and legacy universal/LaunchWrapper installation implemented; macOS 1.20.1 and 1.21.1 menu smokes passed; Windows legacy and full native creation/Play acceptance remain pending; current Linux ARM64 modern Forge passed below.
- [ ] NeoForge — full official Maven catalog, version-bound installation and native selection implemented, including snapshots, 1.20.1 history and 26.x; full native creation/Play acceptance remains pending.
- [ ] OptiFine standalone/Forge — official external download guidance, JAR import, compatibility validation and automatic installation.
- [ ] Quick Play — manifest-capability checks, saved server preferences and real compiled launch arguments.
- [ ] Deletion/confirmation and linked Profiles — preserve player files and revalidate targets while parent remains interactive.

### Settings
- [ ] General — System/Light/Dark preview/save/cancel, system plus in-app reduced motion and persisted preferences.
- [ ] Game — Java, memory/JVM, window/fullscreen and snapshot defaults applied to new setups only.
- [ ] Default memory 1–32 GB, automatic/selected Java, three window presets/custom sizes and extra JVM arguments — validated persisted defaults without rewriting existing Profiles.
- [ ] Profiles — confirmed one-way mode conversion; Restore defaults preserves mode and existing setups.
- [ ] Downloads — concurrency/rate/retry policies and System/Direct/manual HTTP(S)/SOCKS5 transport implemented; local/proxy DNS and real draft-based connection testing are wired. Forge/NeoForge processor HTTP inputs now use verified Store downloads and offline execution; full native interaction, remote verification and visual acceptance remain pending.
- [ ] Parallel downloads 1–16, retries 0–10 and nonnegative MB/s limit (0 means unlimited) — validated draft/save behavior and real transport policies.
- [ ] Storage — editable library/cache paths, saved/draft boundaries and transactional relocation implemented: copy, verify, atomic switch, interruption recovery, space check; full native interaction and user acceptance remain pending.
- [ ] Relocation concurrency and cleanup — active-operation leases block launches/installs; retained copies survive restart and require identity-bound confirmation in an independent window before deletion. Native acceptance remains pending.

### About and Welcome
- [ ] About — real build/product information, brand, source/feedback links and copy actions.
- [ ] Welcome three-step flow — real drafts, account preparation, atomic creation, preference saving, skip and retry paths.

### Cross-page
- [ ] Light/dark layouts, spacing, typography, icons, states and short displays — actual native screenshots for user review.
- [ ] Page/detail/wizard transitions and correct directions — upstream opacity/offset, 460 ms easing and reduced motion.
- [ ] Text input/IME, clipboard, search, scrolling, keyboard actions and draft/error preservation — real interactive workflows.
- [x] Prototype route/component/option inventory: Home selectors/launch/downloads; Accounts/wizard/providers; Bundled/Separated games and component/settings subpages; five Settings sections; About/Welcome and shared navigation. Demo-only routes, sample identities/catalogs/progress and browser channels are excluded from product behavior.

## Verification

- Supervisor diagnostics `d6400a9` passed three-platform CI run `39644470331`; Offline `8b6a136` also passed after a Windows-only rerun. Full local baseline: 332 tests plus release compilation. The earlier intermittent Windows lifecycle failure has not been reproduced on the test host or established as fixed.
- Old ui-parity evidence was removed before staging. `gpui-product-progress.md` already existed in earlier history; its current-tree deletion does not rewrite those prior commits.
- Native macOS Welcome/version picker captured under `target/parity-shots/native/p1-*-system.png`; closing the parent removed both windows. Visual acceptance remains pending.
- Native screenshots cover 69 debug UI scenarios in both themes (220 primary/child images), including provider management, external sign-in/characters/confirmation and independent provider add/remove windows. `scripts/parity-board` builds `target/parity-shots/index.html`. Fixture codes are not live authorization codes; release compilation excludes scenario flags/fixtures.
- The React reference server is available at `http://127.0.0.1:5173/`; the separate prototype screenshot task has not supplied its images. The board labels missing reference shots explicitly. Current native fixture records cover Microsoft/Offline/Authlib accounts and Vanilla/NeoForge/Fabric/Quilt/Forge models; remaining loader/operating states await their implementations.
- All three remote hosts completed actual Home → Games → native version-picker smoke, with visible screenshots under `target/parity-shots/{linux-arm64,windows-amd64,linux-amd64}/`. ARM64/Windows used pushed `d97206d`; Linux AMD64 used `def4a1b`. These establish the P1 window baseline, not current-page parity or P5 game verification.
- Linux ARM64 KDE uses Xwayland `:1` plus session Xauthority and Spectacle; scrot returns black pixels there. Linux AMD64 X11 needs ImageMagick screen-based window capture. Windows captures run in an interactive scheduled task and enumerate visible windows by process ID. Test tasks were removed and the AMD64 VM was stopped softly after capture.
- Required supported launches: macOS Vanilla/Fabric/Quilt/Forge/NeoForge/OptiFine plus Authlib injection; three remote hosts Vanilla/Fabric/modern Forge.
- macOS ARM64 Vanilla 1.20.6: official managed Java/game installation, explicit Offline recipe, actual main menu captured at `target/parity-shots/macos-arm64/vanilla-1.20.6-menu.png`, then launcher cancellation stopped Java. This verifies the CLI-backed install/launch path; Desktop selection/Play and live-account smoke remain required.
- Fabric/Quilt backend: official game-filtered catalogs, validated profile identity, official Maven repositories, SHA-1/size verification, atomic merged manifests and retryable catalog states. Six local HTTP tests plus two catalog tests cover success/cache corruption, invalid identity/repositories, unavailable builds, throttling, cancellation/timeouts and persistence. Full local `just check` passed 377 tests. Native selection is now implemented; the backend commit `41982ff` passed Linux/macOS CI run `39665085203`. The Windows one-connection Home fixture uses an explicit zero-retry policy; the correction and native loader integration passed all three platforms in CI run `39668018822` (`5197f0f`).
- Native Fabric/Quilt pickers provide official version-filtered catalogs, semantic build ordering, search, stable/preview states, cancellation, empty/error/retry states and validated final creation. Requests bind to the current game/draft; cancelling restores the checkpoint and rejects late results. Full local `just check` passed 381 tests, including new draft-boundary and real catalog-creation checks without network fallback. Five new light/dark scenarios were captured; the actual app opened its independent first-run picker. Full real-service native interaction remains pending.
- macOS ARM64 Minecraft 1.20.6: Fabric 0.16.14 and Quilt 0.29.2 installed from official sources, reached visually confirmed modded main menus and stopped through the supervisor. Screenshots: `target/parity-shots/macos-arm64/{fabric,quilt}-1.20.6-menu.png`. A dedicated manual API driver used explicit Offline identities; native wizard/Play and the remaining version/host matrix are pending.
- NeoForge now reads both official Maven catalogs, maps three/four-part and snapshot versions to the exact game, sorts builds numerically, verifies archive identity and derives required patched-client outputs from installer data. Existing installed caches remain readable; target/manifest/plan versions must agree. Local protocol tests cover filtering, cache reuse, invalid metadata, removed builds, throttling, cancellation and timeout. Full local `just check` passed 387 tests; release compilation checking passed; `11e9cb9` passed all three platforms in CI run `39672223081`.
- Forge: complete Maven coordinates retain legacy branch suffixes; official catalog filtering, numeric sorting, offline replay, throttling/cancellation/deadlines and metadata errors are fixture-tested. Both legacy installer layouts extract verified universal JARs without running Java; corrupt cache/output and wrong game/build metadata cannot become ready. A native light/dark Forge picker scenario is captured. Verified installer-supplied checksums support legacy artifacts whose official repository omits sidecars. Current shim-coordinate and generated-client layouts are supported; Home Profile badges use the actual loader kind. Full local `just check` passed 396 tests. `c75c4ff` passed all three platforms in CI run `39675473426`; `0b7d3dc` passed all three platforms in CI run `39676499858`.
- macOS ARM64 Forge 47.4.13 / Minecraft 1.20.1: official installer/processors, verified launch plan, actual main menu and supervisor stop passed with a dedicated Offline profile. Screenshot: `target/parity-shots/macos-arm64/forge-1.20.1-menu.png`. Forge 1.12.2 and 1.8.9 official universal installations also passed; this does not establish legacy game launch or native wizard/Play interaction.
- macOS ARM64 Forge 52.1.20 / Minecraft 1.21.1: official shim installer and processor-generated client verification reached the visually confirmed real menu and stopped through the supervisor. Screenshot: `target/parity-shots/macos-arm64/forge-1.21.1-menu.png`. Dedicated Offline/API smoke; native creation/Play and remote game verification remain pending.
- macOS ARM64 NeoForge 21.1.219 on 1.21.1 and historical 47.1.106 on 1.20.1: official install/processor execution, visually confirmed real main menus and supervisor stop passed using dedicated Offline profiles. Screenshots: `target/parity-shots/macos-arm64/neoforge-{1.21.1,1.20.1}-menu.png`. These are public-library/supervisor smokes; native wizard/Play interaction remains separate.
- macOS ARM64 NeoForge 26.3.0.95 on 26.3: the current installer/patch layout reached the real main menu and stopped through the supervisor. The dedicated test Profile used the normal completed first-run preference; this does not establish first-run UI interaction. Screenshot: `target/parity-shots/macos-arm64/neoforge-26.3-menu.png`.
- Older supported representatives vary by loader; legacy Forge runs on Windows. NeoForge 1.20.1 passed natively on macOS ARM64; older LWJGL generations, other loader representatives and Linux ARM64 investigation remain pending.
- A process PID or generic supervisor Running state does not establish the main menu; observe the game or validate a specific readiness signal.
- Detached supervisors retain ordinary stderr in their private run directory; process tests include persisted status, child-exit markers and bounded/redacted stderr when a lifecycle wait fails. Diagnostic retention is not a claim that the intermittent Windows failure is fixed.
- Local auth fixtures and UI scenarios are separate from live-provider sign-in and actual game acceptance.
- Eight Device Code loopback tests cover the complete ownership chain, poll intervals, slow_down/429, timeout backoff, cancellation during I/O, expiry, malformed links and controlled error mapping. UI regressions cover prepared-versus-saved state, method changes, orphan URLs and parent drafts after delayed save completion.
- Microsoft preparation does not write account metadata or refresh credentials; final confirmation writes both. First-run's combined new-account/game commit remains required Welcome work. No live device request has been started. The public client ID is present in local .env and must be passed privately to the test app's environment.
- Offline preparation is local and memory-only until Finish; saved metadata has no credential/client references. Launch checks the saved name/UUID and never falls back from failed Microsoft authorization. Local tests cover name boundaries, case-sensitive Java UUIDs, duplicate/colliding identities, corrupt metadata, durable save/removal and editable/cancelled drafts.
- Provider discovery/state tests cover ALI and redirects, HTTPS/consent, bounded/invalid metadata, throttling, cancellation/timeouts, stale/duplicate writes, corruption and linked-account protection. Existing account IDs remain compatible; external identities include provider, login and character UUID. Yggdrasil desktop login and verified launch injection are implemented; macOS OS-store/injected-game fixture verification passed. Native form interaction and live-provider verification remain separate.
- Provider management `f063612` passed full local `just check` (345 tests), release compilation and three-platform CI run `39649424883`. The default HTTP form shows the consent control and disables saving until accepted. Unreadable provider metadata stays an error instead of being presented as an empty saved list.
- Yggdrasil protocol core uses transient authentication/profile selection, a 15-minute confirmation deadline, validate-before-save/launch, refresh only after invalid-token status, and separate token invalidation versus fresh-password all-session signout. POSTs never follow redirects/ALI. Provider/login/character/API bindings prevent credential reuse across identities; refreshed user IDs/properties remain mutable. Eight local HTTP tests use a test-only synthetic credential store; they do not establish real provider or OS-keychain success. This core passed full local `just check` (355 tests), release compilation and three-platform CI run `39653680654` (`f92fd16`).
- External login rechecks the draft's exact API before sending a password. Changing provider/address clears credentials and HTTP acceptance; animation snapshots omit the password editor. Character assignment uses provider refresh, final Finish writes tokens/metadata, and account removal invalidates only that session before deleting its local credential. Unconfirmed remote revocation is reported explicitly.
- Authlib agent preparation uses the official API and verified SHA-256 cache; transfers share proxy/concurrency/rate/retry policies and emit real bytes/cache/retry events. Private bootstrap carries provider/agent data and user properties; Java binding verifies the JAR again and adds agent/prefetch arguments before the main class. Local fixtures cover discovery validation, origin changes, corruption, throttling, cancellation/timeouts and backwards-compatible private sessions. The implementation passed full local `just check` (369 tests), release compilation, native capture and three-platform CI run `39660780649` (`553834f`). Actual macOS injection smoke is recorded below.
- macOS ARM64 Authlib 1.20.6: a dedicated manual API driver verified two-character assignment, no OS credential before Finish, real keychain save/read/rotation and official agent verification. The actual injected game reached its main menu (`target/parity-shots/macos-arm64/authlib-1.20.6-menu.png`) and queried the loopback provider. Supervisor stop and synthetic credential revocation/deletion passed. This is a local fixture/core smoke, not native sign-in-form or live-provider acceptance.
- Legacy user properties travel only in the transient session/private supervisor pipe and runtime Java arguments; plans, metadata and argument previews exclude them. Authlib uses the raw token for legacy auth_session. Local command/wire tests cover Unicode external names, opaque tokens, property validation/redaction and older bootstrap compatibility.
- The prototype's outer page padding is not launcher content. Compare cropped Home at 432×768 and management content at 960×600 logical pixels; browser viewport dimensions must allow for that padding.
- Home visual work: square initial fallback, provider badge, centered launch icon/text and integrated Profile selector; official version details remain in the launch tooltip. Reserve macOS traffic-light space and give account/Profile windows usable list height. Real account avatars and full Home acceptance remain open.
- Screenshot capture includes the launched process's allocated windows, including inactive floating panels; their actual CGWindowID capture was inspected. It does not require System Events activation.
- SOCKS5: the existing Manual proxy choice accepts `socks5://` (local DNS) and `socks5h://` (proxy DNS), saves the selected protocol and rejects credentials/invalid endpoints. Local binary SOCKS5 fixtures verify metadata, SHA-1 object transfer, HEAD probes, DNS routing, throttling, rejected connections, cancellation and handshake deadlines. UI draft/persistence/cancel tests pass; light/dark Downloads and SOCKS5 scenarios are captured. Full local `just check` passed 402 tests. `a271311` passed all three platforms in CI run `39677881325`. Installer subprocess verification follows below.
- Installer network policy: Block prepares exact official game metadata and SHA-1-verified client mappings through its configured Store. Unmodified installers run with `--offline`; an owned Java 8-compatible cache reader gives their URL-based processors only prepared files and rejects other HTTP requests. Local fixtures cover retries, identity/origin/corruption, cancellation and deadlines; the reader passed actual UTF-8/equal-sign-path and unknown-URL smokes.
- Current installer-policy smoke on macOS ARM64: Forge 47.4.13 / 1.20.1, Forge 52.1.20 / 1.21.1, NeoForge 21.1.219 / 1.21.1 and NeoForge 26.3.0.95 / 26.3 reinstalled, reached actual captured main menus and stopped through the supervisor. Forge raw mappings were downloaded through a dedicated loopback SOCKS5 forwarder before offline processing. Existing menu images under `target/parity-shots/macos-arm64/` were refreshed. Full local `just check` passed 406 tests; remote game verification remains pending.
- Storage baseline `a669325` and Windows-scoped cleanup correction `5e34c70` passed all three platforms in CI run `39687371700`. This establishes the backend baseline; native integration is verified separately.
- Directory ownership explicitly unlocks on guard release, including when a concurrent spawn briefly retains an inherited file description. The process fixture waits for completed startup output before intentionally killing its supervisor, avoiding a broken pipe that could end the fixture child.
- Storage backend separates stable control metadata, default library and reusable cache paths. Cache relocation copies/verifies managed trees, rebases manifests/Java references, atomically switches locations and retains originals; managed-game relocation preserves custom folders and derived preparation data. Functional transaction recovery, byte progress/cancellation, space checks, active-operation leases and identity-bound explicit cleanup are implemented. Native storage editors, byte/phase progress, cancellation, retained-copy review and an independent removal confirmation are implemented. Four UI state tests cover real temporary-cache movement, restart/reopen, draft edits, cancellation, stale results and corrupt/unreadable saved state. Full local `just check` passed 421 tests. Managed Java relative links are preserved exactly; links outside copied trees retain their original destination without copying external files. Full native interaction and user acceptance remain pending. Explicit development recipes retain their own data-directory semantics.
- macOS ARM64 storage smoke: official Vanilla 1.20.6 was prepared, its verified cache and managed game folders moved to new empty locations, and test-only originals removed through explicit cleanup. Strict runtime/installation checks passed after the move; the actual main menu was captured at `target/parity-shots/macos-arm64/storage-relocated-vanilla-1.20.6-menu.png`. A running game rejected relocation; supervisor Stop finished and the Java PID disappeared. This verifies the public-library/supervisor path, not a full native Settings/Play interaction or live-account launch.
- Storage/native integration `bb901b7` passed all three platforms in CI run `39689846774`.
- Linux ARM64 on `bb901b7`: Vanilla 1.20.6, Fabric 0.16.14 / 1.20.6 and Forge 52.1.20 / 1.21.1 installed from official sources, reached visually confirmed real main menus and stopped through the supervisor; exact Java PIDs disappeared. Screenshots: `target/parity-shots/linux-arm64/{vanilla-1.20.6,fabric-1.20.6,forge-1.21.1}-menu.png`. Native Home → Games → Create → independent 760×520 version-picker interaction was also captured. This is Offline/API game verification plus a separate launcher UI smoke, not native Play or live-account acceptance.
- Windows AMD64 on `bb901b7`: Vanilla 1.20.6 and Fabric 0.16.14 / 1.20.6 reached visually confirmed real menus through interactive scheduled tasks; supervisor Stop succeeded from SSH and exact Java PIDs disappeared. Screenshots: `target/parity-shots/windows-amd64/{vanilla,fabric}-1.20.6-menu.png`. Completed owned tasks were removed. Modern/legacy Forge and the current native UI pass remain pending.
- Native creation/Welcome component tiles now separate their value and detail, truncate each line and expose full values in a tooltip. Empty button labels no longer insert a blank flex child. Actual light/dark screenshots show the configuration stays inside the square; full local `just check` passed 421 tests.
- Required user review/sign-in/smoke pending means implementation readiness can be reported, but Pilot completion cannot.

## Needs-user

| Item | When / status |
| --- | --- |
| Native light/dark screenshots and comparison review | Ready: 69 UI scenarios in both themes plus the board. User acceptance is pending; continue independent work. |
| DeepSeek Harness prototype captures | P0 snapshot is ready; the separate assigned task has not supplied reference images. The board labels them pending. |
| Microsoft PKCE/Device Code live sign-in | Morning of 2026-10-22 Asia/Taipei, when user is available; batch hosts, do not start expiring codes early. |
| Azure public client flow setting | Required before real Device Code verification. |
| macOS Screen Recording permission | Available; actual Welcome/version-picker capture succeeded. No user action needed. |
| OptiFine official JAR | User downloads when needed, launcher automates installation. |
| Authlib provider account | Optional live-provider verification; local fixture otherwise. |
| Authlib external text drag/drop | Fixed GPUI 0.3.9 macOS exposes file paths only; user choice pending on accepting input/paste import as an explicit difference. Continue the supported import and provider features meanwhile. |

## v0 notes

- Architecture governance, file splitting, layering and lint tidy-up remain deferred.
- Pedantic/nursery lint groups are disabled; preserve existing expectations, clippy::all, the deny list and unsafe_code=forbid.
- Accessibility, release packaging/signing and mirror sources remain deferred; required existing notice/reference fixes are in scope.
