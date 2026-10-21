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
- Settings distinguish saved values, drafts and appearance previews; defaults apply only to newly created games.
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
- [ ] Offline account — explicit local identity, validated username/Java UUID v3, final-save confirmation, persistent selection/removal and launch dispatch implemented; actual game smoke remains pending.
- [ ] Authlib provider add/edit/remove, discovery and quick-link import — real metadata, HTTPS policy, linked-account rules and caching.
- [ ] Provider URL/pasted or dropped quick links, duplicate handling and explicit HTTP consent — validated normalization, ALI discovery, persisted consent and password-safe requests.
- [ ] Authlib account/password form, selected provider and multiple characters — real credential exchange, profile selection, cancellation and final secure-save confirmation.
- [ ] Yggdrasil authenticate/refresh/validate/invalidate/signout and profile selection — local protocol fixtures and secure storage.
- [ ] Authlib launch injection — official verified agent JAR, prefetched metadata and actual injected game launch.
- [ ] Wizard back/cancel/finish and provider picker — independent windows, local drafts and stale-result rejection.

### Games and Game Wizard
- [ ] Bundled/Separated browsing, search, details and Home synchronization — real Instance/Profile joins and mode rules.
- [ ] Profile/Instance appearance and launch/settings editing — validation, immutable version/loader and transactional persistence.
- [ ] Profile icons/colors, memory, extra JVM arguments, inherited Java/window defaults, isolated/shared files and default account — persisted drafts compiled into actual launch recipes.
- [ ] Bundled two-step and Separated four-step creation — section drafts, existing-Instance selection and atomic final save.
- [ ] Version/loader/location pickers — real catalogs, compatibility filtering, retryable loading and independent windows.
- [ ] Vanilla installation — verified official metadata/resources, real progress, retries and resumable downloads.
- [ ] Fabric and Quilt — official version catalogs and verified installation filtered by Minecraft version.
- [ ] Forge modern/legacy — installer/processors or universal/launchwrapper as appropriate; verified artifacts and retryable tasks.
- [ ] NeoForge — full official version catalog and verified installation, replacing the single pinned build.
- [ ] OptiFine standalone/Forge — official external download guidance, JAR import, compatibility validation and automatic installation.
- [ ] Quick Play — manifest-capability checks, saved server preferences and real compiled launch arguments.
- [ ] Deletion/confirmation and linked Profiles — preserve player files and revalidate targets while parent remains interactive.

### Settings
- [ ] General — System/Light/Dark preview/save/cancel, system plus in-app reduced motion and persisted preferences.
- [ ] Game — Java, memory/JVM, window/fullscreen and snapshot defaults applied to new setups only.
- [ ] Default memory 1–32 GB, automatic/selected Java, three window presets/custom sizes and extra JVM arguments — validated persisted defaults without rewriting existing Profiles.
- [ ] Profiles — confirmed one-way mode conversion; Restore defaults preserves mode and existing setups.
- [ ] Downloads — concurrency/rate/retry policies, System/Direct/HTTP(S)/SOCKS5 proxy and real connection testing.
- [ ] Parallel downloads 1–16, retries 0–10 and nonnegative MB/s limit (0 means unlimited) — validated draft/save behavior and real transport policies.
- [ ] Storage — validated library/cache paths and transactional relocation: copy, verify, atomic switch, interruption recovery, space check.
- [ ] Relocation concurrency and cleanup — block launches/installs; delete old directory only after explicit user confirmation.

### About and Welcome
- [ ] About — real build/product information, brand, source/feedback links and copy actions.
- [ ] Welcome three-step flow — real drafts, account preparation, atomic creation, preference saving, skip and retry paths.

### Cross-page
- [ ] Light/dark layouts, spacing, typography, icons, states and short displays — actual native screenshots for user review.
- [ ] Page/detail/wizard transitions and correct directions — upstream opacity/offset, 460 ms easing and reduced motion.
- [ ] Text input/IME, clipboard, search, scrolling, keyboard actions and draft/error preservation — real interactive workflows.
- [x] Prototype route/component/option inventory: Home selectors/launch/downloads; Accounts/wizard/providers; Bundled/Separated games and component/settings subpages; five Settings sections; About/Welcome and shared navigation. Demo-only routes, sample identities/catalogs/progress and browser channels are excluded from product behavior.

## Verification

- Microsoft authorization `9ecc1fd` passed three-platform CI run `39640688731`. Offline `8b6a136` passed full local `just check` (332 tests), release compilation and macOS/Linux CI; Windows session-supervisor verification remains under investigation.
- Old ui-parity evidence was removed before staging. `gpui-product-progress.md` already existed in earlier history; its current-tree deletion does not rewrite those prior commits.
- Native macOS Welcome/version picker captured under `target/parity-shots/native/p1-*-system.png`; closing the parent removed both windows. Visual acceptance remains pending.
- Native screenshots cover 48 debug UI scenarios in both themes (148 primary/child images), including Microsoft method choice/device request/waiting/saving and Offline information/error/review/detail. `scripts/parity-board` builds `target/parity-shots/index.html`. Fixture codes are not live authorization codes; release compilation excludes scenario flags/fixtures.
- The React reference server is available at `http://127.0.0.1:5173/`; the separate prototype screenshot task has not supplied its images. The board labels missing reference shots explicitly. Current native fixture records cover implemented Microsoft/Vanilla/NeoForge models; additional provider/loader scenarios await their real implementations.
- All three remote hosts completed actual Home → Games → native version-picker smoke, with visible screenshots under `target/parity-shots/{linux-arm64,windows-amd64,linux-amd64}/`. ARM64/Windows used pushed `d97206d`; Linux AMD64 used `def4a1b`. These establish the P1 window baseline, not current-page parity or P5 game verification.
- Linux ARM64 KDE uses Xwayland `:1` plus session Xauthority and Spectacle; scrot returns black pixels there. Linux AMD64 X11 needs ImageMagick screen-based window capture. Windows captures run in an interactive scheduled task and enumerate visible windows by process ID. Test tasks were removed and the AMD64 VM was stopped softly after capture.
- Required supported launches: macOS Vanilla/Fabric/Quilt/Forge/NeoForge/OptiFine plus Authlib injection; three remote hosts Vanilla/Fabric/modern Forge.
- macOS ARM64 Vanilla 1.20.6: official managed Java/game installation, explicit Offline recipe, actual main menu captured at `target/parity-shots/macos-arm64/vanilla-1.20.6-menu.png`, then launcher cancellation stopped Java. This verifies the CLI-backed install/launch path; Desktop selection/Play and live-account smoke remain required.
- Older supported representatives vary by loader; legacy Forge runs on Windows. Legacy ARM64 investigation/results remain pending.
- A process PID or generic supervisor Running state does not establish the main menu; observe the game or validate a specific readiness signal.
- Detached supervisors retain ordinary stderr in their private run directory; process tests include persisted status, child-exit markers and bounded/redacted stderr when a lifecycle wait fails. Diagnostic retention is not a claim that the intermittent Windows failure is fixed.
- Local auth fixtures and UI scenarios are separate from live-provider sign-in and actual game acceptance.
- Eight Device Code loopback tests cover the complete ownership chain, poll intervals, slow_down/429, timeout backoff, cancellation during I/O, expiry, malformed links and controlled error mapping. UI regressions cover prepared-versus-saved state, method changes, orphan URLs and parent drafts after delayed save completion.
- Microsoft preparation does not write account metadata or refresh credentials; final confirmation writes both. First-run's combined new-account/game commit remains required Welcome work. No live device request has been started. The public client ID is present in local .env and must be passed privately to the test app's environment.
- Offline preparation is local and memory-only until Finish; saved metadata has no credential/client references. Launch checks the saved name/UUID and never falls back from failed Microsoft authorization. Local tests cover name boundaries, case-sensitive Java UUIDs, duplicate/colliding identities, corrupt metadata, durable save/removal and editable/cancelled drafts.
- The prototype's outer page padding is not launcher content. Compare cropped Home at 432×768 and management content at 960×600 logical pixels; browser viewport dimensions must allow for that padding.
- Home visual work: square initial fallback, provider badge, centered launch icon/text and integrated Profile selector; official version details remain in the launch tooltip. Reserve macOS traffic-light space and give account/Profile windows usable list height. Real account avatars and full Home acceptance remain open.
- Screenshot capture includes the launched process's allocated windows, including inactive floating panels; their actual CGWindowID capture was inspected. It does not require System Events activation.
- Required user review/sign-in/smoke pending means implementation readiness can be reported, but Pilot completion cannot.

## Needs-user

| Item | When / status |
| --- | --- |
| Native light/dark screenshots and comparison review | Prepare for 2026-10-22; do not block independent work on immediate taste approval. |
| Microsoft PKCE/Device Code live sign-in | Morning of 2026-10-22 Asia/Taipei, when user is available; batch hosts, do not start expiring codes early. |
| Azure public client flow setting | Required before real Device Code verification. |
| macOS Screen Recording permission | Available; actual Welcome/version-picker capture succeeded. No user action needed. |
| OptiFine official JAR | User downloads when needed, launcher automates installation. |
| Authlib provider account | Optional live-provider verification; local fixture otherwise. |

## v0 notes

- Architecture governance, file splitting, layering and lint tidy-up remain deferred.
- Pedantic/nursery lint groups are disabled; preserve existing expectations, clippy::all, the deny list and unsafe_code=forbid.
- Accessibility, release packaging/signing and mirror sources remain deferred; required existing notice/reference fixes are in scope.
