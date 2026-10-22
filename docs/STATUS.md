# Block Pilot Status

Updated: 2026-10-22. Built, native verification and user acceptance are separate gates.

## Product decisions and intended differences

- The P0 React snapshot and README define product content; explicit user decisions take precedence.
- Windows is the primary native UI/E2E and visual-review platform (user decision, 2026-10-22); retain platform-specific macOS/Linux checks.
- New catalogs use Bundled mode: one Instance and exactly one associated Profile; names and appearance match Home.
- Existing catalogs without a mode keep Separated behavior. Confirmed Bundled-to-Separated conversion is one-way; Restore defaults preserves mode.
- Separated mode permits multiple Profiles per Instance. Deletion removes catalog metadata and preserves player files.
- Version, release type and loader/build are fixed after creation. Other editable fields use local drafts until Save.
- Section Done updates only the local draft; Cancel restores its checkpoint. Final creation saves the Instance/Profile pair atomically.
- Late results update actual catalog/session state, but cannot complete or clear a newer draft, wizard or authentication attempt.
- Home is 432×768; full management pages are 960×600; dialogs use content-appropriate fixed sizes. Fit to short displays at creation.
- macOS uses the upstream transparent titlebar and native traffic lights; Windows/Linux use system decorations.
- Dialogs, drawers, wizard sections and the account flow are in-window overlay components built on the gpui-kit/gpui-component Dialog and Sheet primitives: prototype-aligned scrim, background interaction blocked, Escape closes, focus stays inside the overlay and is restored afterwards. Anchored menus/selects/tooltips use upstream Base components.
- Accessibility semantics remain deferred; stacked overlays, system and in-app reduced motion are retained.
- First run keeps Your game → Your account → Ready to go. Skipping creates no identity and never bypasses launch authorization.
- Authentication approval prepares an identity; final confirmation saves it. Cancellation, expiry and method changes invalidate pending results.
- First-run creation installs a real game and returns Home while downloads continue. Download/Retry requires no signed-in account or duplicate game.
- Settings distinguish saved values, drafts and appearance previews; defaults apply only to newly created games. Library defaults affect new Instances; moving existing managed games is separate and preserves custom folders. Cache changes copy verified managed resources while account/control metadata stays at the stable root.
- Only official upstream download sources are offered; no BMCLAPI or other third-party mirrors.
- Download bytes, retries, verification and cache hits come from real task events. Unknown totals remain indeterminate.
- Connection testing uses the Downloads draft without saving, has bounded requests and rejects late results after draft changes.
- Player avatars must come from actual account data; initials are valid fallback, demo Steve/Alex images are not real skins.
- Server addresses save separately from auto-connect. Use manifest Quick Play or supported legacy --server/--port arguments; explain unsupported versions.
- OptiFine: user downloads from optifine.net, then selects the JAR for automatic validation/installation; no bundling or redistribution.
- Productize explicit official LWJGL 3 ARM64/runtime substitutions; legacy support remains best effort. Disable LWJGL 2 on ARM64 with the reason.
- Use only dedicated test data directories; never copy player credentials/worlds or expose .env values.

## Page inventory

Yes = built; Partial = a stated gap remains; Pending = not verified/accepted. Native E2E requires the actual app and real services; allowed local providers are labeled fixture.

### Home

| Feature | Built | Native E2E | Accepted | Note |
| --- | --- | --- | --- | --- |
| Profile/account selection; empty/loading/error states | Yes | Pending | Pending | Persistent catalog and selector sheets. |
| Real account avatars in Home and selectors | Yes | Pending | Pending | Microsoft active skin and Authlib declared textures only; bounded validated fetch, data-dir cache with digest/URL/structure revalidation, silent initials fallback (no demo skins). Native display check pending. |
| Play/Stop, recovery and crash help | Yes | Partial | Pending | Windows Play/Stop and macOS relaunch passed; crash help pending. |
| Downloads, retries and file details | Yes | Pending | Pending | Real byte events. |

### Accounts and sign-in

| Feature | Built | Native E2E | Accepted | Note |
| --- | --- | --- | --- | --- |
| List/detail, selection, deletion, last-account empty state | Yes | Pending | Pending | OS credential storage. |
| Microsoft browser PKCE and final confirmation | Yes | Yes (macOS) | Pending | Native authorization and secure save passed. |
| Microsoft Device Code and final confirmation | Yes | Yes | Pending | macOS, Windows, Linux ARM64 and Linux AMD64 native saves passed; Linux AMD64 was 0f7e26b. |
| Offline account and final confirmation | Yes | Pending | Pending | Validated Java UUID identity. |
| Authlib provider add/edit/remove and discovery | Yes | Pending | Pending | Linked-account protection. |
| Authlib provider icon retrieval and display | Yes | Pending | Pending | Bounded validated fetch against the provider API host, data-dir cache, monogram fallback. ICO-only favicons fall back to the monogram (accepted limitation). Native display check pending. |
| Provider URL/paste import, duplicate selection, HTTP consent | Yes | Pending | Pending | Address-bound consent. |
| External quick-link text drop | Yes | Yes (Windows) | Pending | Owned native drop target (`crates/block-drop`); real browser→drop-zone drag on Windows AMD64 with a real Authlib provider and the duplicate path. macOS/Linux native targets pending. |
| Authlib password form, characters and final save | Yes | Pending | Pending | Local fixture allowed. |
| Yggdrasil session lifecycle and launch injection | Yes | Pending | Pending | Local fixture allowed. |
| Wizard back/cancel/finish, provider selection, stale results | Yes | Pending | Pending | Prepared identities stay transient. |
| Real player faces and hat layers | Yes | Pending | Pending | Microsoft active skin / Authlib declared textures with domain validation, bounded fetch, validated cache, silent initials fallback; native display check pending. |

### Games and Game Wizard

| Feature | Built | Native E2E | Accepted | Note |
| --- | --- | --- | --- | --- |
| Bundled/Separated browsing, search, details, Home sync | Yes | Pending | Pending | Mode rules persisted. |
| Profile/Instance appearance and settings drafts | Yes | Pending | Pending | Version/loader immutable. |
| Icons/colors, memory/JVM, Java/window defaults, folders/account | Yes | Pending | Pending | Compiled into launch recipes. |
| Bundled two-step / Separated four-step creation | Yes | Pending | Pending | Atomic Instance/Profile pair. |
| Version, loader/build and location pickers | Yes | Pending | Pending | Real catalogs. |
| Vanilla installation, progress, retry and resume | Yes | Partial | Pending | macOS/Windows Welcome/install/Play passed; retry pending. |
| Fabric installation and build selection | Yes | Pending | Pending | Official catalog. |
| Quilt installation and build selection | Yes | Pending | Pending | Older representative pending. |
| Forge modern/legacy installation and build selection | Yes | Pending | Pending | Windows native legacy flow pending. |
| NeoForge full catalog and installation | Yes | Pending | Pending | Includes 1.20.1 and 26.x. |
| OptiFine standalone/Forge import and installation | Partial | Pending | Pending | Existing-target re-import UI missing. |
| Quick Play and saved server settings | Yes | Pending | Pending | Manifest Quick Play for 1.20+, legacy --server/--port for older releases. |
| Productized LWJGL 3 ARM64 compatibility and Java 8 | Partial | Pending | Pending | Linux ARM64 substitutes official ARM64 LWJGL natives for 3.3.1+ manifests; pre-3.3.1 composition and ARM64 Java 8 pending. |
| ARM64 LWJGL 2 picker restriction | Yes | Yes (Linux ARM64) | Pending | Rows stay visible but disabled with the inline reason; typed exact IDs are refused in-dialog. Native proof on Linux ARM64 (851e23b); macOS/Windows ARM64 picker checks pending. |
| Deletion, confirmation and linked Profiles | Yes | Pending | Pending | Player files retained. |

### Settings

| Feature | Built | Native E2E | Accepted | Note |
| --- | --- | --- | --- | --- |
| General: theme preview/save/cancel, reduced motion | Yes | Pending | Pending | Saved and preview values separate. |
| Game: Java, memory/JVM, window/fullscreen, snapshots | Yes | Pending | Pending | New-game defaults only. |
| Memory 1–32 GB, Java choice, window presets/custom, JVM | Yes | Pending | Pending | Validated defaults. |
| Profiles: confirmed one-way conversion, Restore defaults | Yes | Pending | Pending | Mode preserved by reset. |
| Downloads: System/Direct/HTTP(S)/SOCKS5 and connection test | Yes | Pending | Pending | Draft-bound probe. |
| Parallel 1–16, retries 0–10, nonnegative MB/s limit | Yes | Pending | Pending | Real transport policies. |
| Storage: paths, verified relocation, interruption recovery | Yes | Pending | Pending | Original copies retained. |
| Relocation leases and confirmed retained-copy cleanup | Yes | Pending | Pending | Cleanup confirmation dialog. |

### About and Welcome

| Feature | Built | Native E2E | Accepted | Note |
| --- | --- | --- | --- | --- |
| About: build/product, brand, links and copy actions | Yes | Pending | Pending | Real information. |
| Welcome: three steps, drafts, skip, download/retry | Yes | Pending | Pending | No implicit identity on Skip. |
| Welcome: atomic new-account/game final commit | Yes | Yes (Windows) | Pending | Identity is memory-only until the final commit; failed save rolls back. Native run 1ac4564. |

### Cross-page

| Feature | Built | Native E2E | Accepted | Note |
| --- | --- | --- | --- | --- |
| Light/dark layout, typography, icons and states | Partial | Pending | Pending | Windows captures complete; visual comparison in review. |
| Directional 460 ms upstream opacity/offset transitions | Partial | Pending | Pending | Audit timing and directions. |
| System/in-app reduced motion | Yes | Pending | Pending | Native interaction pending. |
| Fit-to-display at creation and recovery | Yes | Partial | Pending | Linux AMD64 1280×800 captured. |
| CJK IME, clipboard, search and scrolling | Partial | Pending | Pending | Real input checks pending. |
| Enter/Escape/Tab, draft/error preservation | Partial | Pending | Pending | Full interaction checks pending. |
| In-window overlays: Sheets for Home drawers, Dialogs elsewhere | Yes | Partial | Pending | Windows AMD64 native check passed on c18e416: one window in every state, stacked Dialog over Sheet, Escape closes only the topmost layer, focus returns to the avatar, Add account is the Accounts page in the same window and saves/restarts correctly. macOS native overlay check deferred while the Mac is in use; scenario captures exist. |

## Smoke matrix

Entries are Minecraft version, API or UI, and the tested commit. API includes CLI/manual-library drivers. Empty cells remain unverified; details and images live in target/parity-shots/<platform>/.

| Host | Vanilla | Fabric | Quilt | Forge | NeoForge | OptiFine | Authlib | Older |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| macOS ARM64 | 1.20.6 UI 78f1595 | 1.20.6 API 41982ff | 1.20.6 API 41982ff | 1.20.1/1.21.1 API 80ed125 | 1.20.1/1.21.1/26.3 API 80ed125 | 1.21.1 standalone/Forge API 78f1595 | 1.20.6 API fixture 553834f | NeoForge 1.20.1 API 80ed125 |
| Windows AMD64 | 1.20.6 UI 1ac4564 | 1.20.6 API bb901b7 | | 1.21.1 API 168a6e1 | | | | Forge 1.12.2 API 168a6e1 |
| Linux AMD64 | 1.20.6 native UI 016c695 | 1.20.6 API bd5f6ff | | 1.21.1 API bd5f6ff | | | | |
| Linux ARM64 | 1.16.5/1.18.2 native menus (Azul runtime) 851e23b | 1.20.6 API bb901b7 | | 1.21.1 API bb901b7 | | | | 1.12.2 picker disabled (native proof) 851e23b |

## Legacy ARM64

| Combination | Outcome | Concrete limitation |
| --- | --- | --- |
| Linux ARM64 1.16.5, Zulu 8u504, LWJGL 3.3.1, JNA 5.13, TTS 1.12.4 + slf4j companion | Native wizard → Play → main menu → Stop verified (851e23b) | Narration lacks a native ARM64 flite wrapper; latest.log gets no "Stopping!" line on the managed stop path. |
| Linux ARM64 1.18.2, plain Zulu 17, LWJGL 3.3.1 | Native wizard → Play → main menu → Stop verified (851e23b) | Narration limitation as above. |
| Linux ARM64 1.19+ / 1.20.6, LWJGL 3.3.1+ | Official ARM64 natives substituted explicitly in plan and log | Native wizard and Play run pending. |
| macOS ARM64 1.16.5, Zulu 8u504, LWJGL 3.3.1/JNA 5.13 | Failed at window creation | GLFW window-icon error. |
| macOS ARM64 1.16.5, alternate official GLFW/binding combinations | Failed | Missing 3.4 binding function or incompatible CallbackI interface. |
| Linux/macOS/Windows ARM64 1.12.2 and 1.8.9, official LWJGL 2 natives | Unsupported selected binaries | Verified archives contain x86/x64 only. The picker keeps them visible but disabled with the inline reason and rejects typed IDs (3e5a801; native Linux ARM64 proof). |

## Verification notes

- Windows intermittent lifecycle failure: bounded repeated host test still required. CREATE_NO_WINDOW validated natively on Windows through install and Play on 1ac4564.
- Known low-frequency test flake (pre-existing): block-launcher `storage::tests::cache_can_return_to_the_managed_root_without_touching_games_or_control_files` failed 2/25 under load, then passed 55/55 stress runs; no code change, tracked until it reappears.
- Native screenshots cover 71 debug scenarios in light/dark; every overlay now renders inside its parent window (one capture per scenario). Scenario captures are visual fixtures.
- Comparison board: target/parity-shots/index.html (Windows primary) and macos-arm64.html; 142 single-window Windows scenario shots (71 stems) captured at d1be580 after the overlay refactor and Q2 fixes.
- Remote picker captures prove navigation and overlays, not native installation/Play.
- Installer processors use verified Store inputs and run offline; macOS SOCKS5 backend smoke passed.
- macOS live PKCE/Device Code, native Welcome/download/Play/menu/Stop and restart/Play/menu/Stop passed; dedicated test accounts retained.
- macOS storage relocation/cleanup followed by game menu passed through the API.
- Linux ARM64 substitutions are explicit in the saved plan and the launch log; the AI/CLI pass the same note through. The 1.16.5 composition also adds the official checksum-verified slf4j-api 1.8.0-beta4 companion required by the reviewed text2speech (native crash found and fixed during Part B).
- Windows ARM64 LWJGL substitution is merged (016c695): explicit `windows_arm64_lwjgl3` compatibility, official Maven Central `natives-windows-arm64` artifacts (3.3.x same version; 3.2.x replaced with the reviewed 3.3.1 composition), plan guards and a `Compatibility:` log note; fixture-tested, x64 unchanged. The native ARM64 VM run waits for a Mac-idle window.
- ARM64 hosts (Linux, Windows, macOS) keep LWJGL 2-era versions visible but disabled in the version picker with "official LWJGL 2 natives are x86/x64 only"; typed exact IDs are refused with the same reason (3e5a801, native Linux ARM64 proof). Linux ARM64 native 1.16.5/1.18.2 wizard→Play→menu→Stop passed at 851e23b with the Azul runtime-substitution lines and the title-screen game-log lines; captures in target/parity-shots/linux-arm64/.
- Linux AMD64 native Device Code saved on 0f7e26b after the client ID was supplied at launch; the built binary refuses sign-in without it.
- Windows native first-run flow passed on 1ac4564: pending identity discarded on quit, account and game saved together, real 1.20.6 download, Play, main menu and Stop.
- CI covers Windows AMD64, Linux AMD64, Linux ARM64 and macOS ARM64; remote hosts are separate evidence. sccache is installed on the macOS host and the Linux ARM64 VM for host rebuilds.
- All four test hosts share a WireGuard overlay (Mac hub 10.77.0.1; Windows AMD64 10.77.0.11, Windows ARM64 10.77.0.12, Linux ARM64 10.77.0.21, Linux AMD64 10.77.0.22); the Mac forwards between peers and the fixed Mac LAN address is the endpoint. The Linux AMD64 VM must still be started detached from its Windows host, and its login keyring relocks after a reboot.
- Linux ARM64 uses Xwayland :1 and Spectacle; Linux AMD64 uses X11 window capture.
- Q9 D6 on Linux AMD64 (191bf24): 1366×768 fit-to-display clamps Home to 432×620 and fits management 960×600; in-app reduced motion removes transitions; upstream 460 ms sheet/page transitions observed; Tab/Enter/Escape, clipboard, search, scrolling and draft restore pass. Linux system reduced motion is now read from GTK `gsettings`; Windows system reduced motion is still unwired (in-app toggle works). Minor: opening a section dialog clears the inline field error and cancelling does not restore it (toast/draft stay; next Continue revalidates).
- Linux AMD64 Q6 native flow passed on 016c695: fresh data dir, Welcome Vanilla 1.20.6, Offline account, real install + managed Mojang runtime, Play → main menu → Stop, restart persistence; evidence in target/parity-shots/linux-amd64/e2e-q6-*.png. Narration only is unavailable on that host because official 1.20.6 metadata ships no Linux `libflite.so`; the game reaches the menu and audio/rendering work. Known upstream limitation, no launcher fix.
- Close owned test apps/tasks and softly stop the AMD64 VM after each remote session.

## Needs-user

| Item | Status |
| --- | --- |
| Azure public client flows and readiness for live sign-in | Confirmed enabled and ready; native macOS PKCE saved successfully. |
| Microsoft macOS PKCE/Device Code and batched remote Device Code | macOS passed; Windows/Linux ARM64 saved; AMD64 Linux keyring configured; native preflight diagnosis pending. |
| Linux AMD64 secure credential store | Resolved: host store is healthy (probe and in-app preflight pass). The earlier failure was a launch without MICROSOFT_CLIENT_ID, not the store. |
| Prototype/native board and behavior acceptance | Windows single-window matrix and boards recaptured at d1be580 (142 images); one grouped review sent, user verdict pending. macOS recapture/native overlay check deferred while the Mac is in use. |
| Windows ARM64 Q4 run | Cross-build from Windows AMD64 is blocked on a missing VS VC++ ARM64 CRT/toolset component (check passes, final link lacks msvcrt.lib); the VM has the native MSVC ARM64 toolchain. Needs a Mac-idle window to build natively on the VM (or the VS component installed on the AMD64 host). |
| Windows ARM64 game launch | No official ARM64 Java 8 exists (Azul/Adoptium/Liberica empty; Mojang windows-arm64 has 17/21/25 only). 1.20.6/1.18.2 best effort remains; 1.16.5 and older are a stated boundary. Running x64 Java under emulation is a product decision to confirm, not implemented. |
| Diagnostic token exposure (2026-10-22) | A refresh token was printed once in a host diagnostic transcript (no file/commit/log). Revoke Block's app access in the Microsoft account and sign in again when convenient. |
| Older official OptiFine JAR | Downloads contains 1.16.5 G8 and 1.12.2 G5; import validation pending. |
| Authlib live account | Optional; use a labeled local fixture otherwise. |
| Real quick-link drag on hosts automation cannot drive | Windows AMD64 done with a real SendInput browser drag. macOS drag still needs the user batch (Mac in use); Linux X11/Wayland phases pending. |

## v0 notes

- Scoped unsafe exception: `crates/block-drop` is the only crate that lowers `unsafe_code` (workspace still forbids it); every unsafe block carries a SAFETY comment, and the crate owns its child HWND/IDropTarget without patching, subclassing or swizzling GPUI.
- Architecture governance, file splitting, layering and lint tidy-up remain deferred.
- Preserve clippy::all, existing deny list and unsafe_code=forbid outside the authorized dedicated drop-target crate.
- Accessibility, release packaging/signing and mirror sources remain deferred.
