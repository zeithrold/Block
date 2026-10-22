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
| Profile/account selection; empty/loading/error states | Yes | Yes (Windows, Linux) | Pending | Persistent catalog and selector sheets exercised in Q6/Q9 runs; empty/error states observed. |
| Real account avatars in Home and selectors | Yes | Yes (Authlib fixture) | Pending | Microsoft active skin and Authlib declared textures only; bounded validated fetch, data-dir cache with digest/URL/structure revalidation, silent initials fallback (no demo skins). Native fixture display and fallback verified on Linux AMD64; Microsoft live skin display still pending a live account. |
| Play/Stop, recovery and crash help | Yes | Yes (Windows, Linux) | Pending | Windows Q6 Play/Stop all loaders, crash help with recovery, and Linux ARM64/AMD64 menu→Stop passed. Stale game-directory reservations now surface a dedicated actionable diagnosis (session_reservation) instead of "cause not determined"; auto-clearing stays intentionally forbidden. macOS native check pending. |
| Downloads, retries and file details | Yes | Yes (Windows) | Pending | Q6 interrupted download→Retry→Repair complete; real byte progress and file detail views exercised. |

### Accounts and sign-in

| Feature | Built | Native E2E | Accepted | Note |
| --- | --- | --- | --- | --- |
| List/detail, selection, deletion, last-account empty state | Yes | Yes (Windows, Linux) | Pending | Accounts list/detail/selection exercised natively; last-account/empty state observed in fixture and Q6 runs. |
| Microsoft browser PKCE and final confirmation | Yes | Yes (macOS) | Pending | Native authorization and secure save passed. |
| Microsoft Device Code and final confirmation | Yes | Yes | Pending | macOS, Windows, Linux ARM64 and Linux AMD64 native saves passed; Linux AMD64 was 0f7e26b. |
| Offline account and final confirmation | Yes | Yes (Windows, Linux) | Pending | Linux Q6 and Windows Q6/Q10 created offline identities and persisted them across restart. |
| Authlib provider add/edit/remove and discovery | Yes | Yes (fixture, Linux AMD64) | Pending | Linked-account protection. Native fixture add/consent/discovery/duplicate and delete paths observed; Windows/macOS native checks pending. |
| Authlib provider icon retrieval and display | Yes | Yes (fixture, Linux AMD64) | Pending | Bounded validated fetch against the provider API host, data-dir cache (file+sidecar digests verified), monogram fallback. Native fixture checks: good icon shown/cached, corrupt and oversized rejected silently to monogram with no cache, stale cache offline still shown, missing cache offline monogram. ICO-only favicons fall back to the monogram (accepted limitation). |
| Provider URL/paste import, duplicate selection, HTTP consent | Yes | Yes (Linux fixture) | Pending | Fixture/Q8 runs exercised typed quick-link identify, HTTP consent, discovery, duplicate notice and saved-provider reuse; Windows typed path also observed. |
| External quick-link text drop | Yes | Yes (Windows, Linux, macOS) | Pending | Owned native drop targets in `crates/block-drop`: OLE child HWND on Windows, XDND ARGB overlay on Linux X11, owned AppKit NSView dragging destination on macOS; all reuse the existing discovery/consent/duplicate flow. Real browser→drop-zone drags prove accept + reject paths on all three; macOS real Chrome drag filled/saved the fixture provider. Wayland unsupported by the pinned GPUI and surfaces the documented error on a real Wayland session; Xwayland works. |
| Authlib password form, characters and final save | Yes | Yes (fixture, Linux AMD64) | Pending | Native fixture sign-in (FixturesAlex/Slim), character choice and save; keyring unlock was user-assisted via VNC. |
| Yggdrasil session lifecycle and launch injection | Yes | Yes (fixture, Linux AMD64) | Pending | Real 1.20.6 + authlib-injector 1.2.8 reached the main menu through the fixture; refresh/invalid-token path exercised. |
| Wizard back/cancel/finish, provider selection, stale results | Yes | Yes (Linux AMD64) | Pending | Native forward/review/back/cancel; provider Back/Close saved no account; a cancelled slow discovery left providers.json unchanged. Prepared identities stay transient. |
| Real player faces and hat layers | Yes | Yes (Authlib fixture) | Pending | Authlib fixture skin rendered face+hat on Home/Accounts/selector; cache sidecar digest equals the served PNG; fixture stop falls back to initials silently. Microsoft live skin display still pending a live account. |

### Games and Game Wizard

| Feature | Built | Native E2E | Accepted | Note |
| --- | --- | --- | --- | --- |
| Bundled/Separated browsing, search, details, Home sync | Yes | Yes (Windows) | Pending | Q6 separated mode: one-way conversion, second Profile sharing an Instance and Home sync exercised natively. |
| Profile/Instance appearance and settings drafts | Yes | Yes (Windows) | Pending | Theme/reduced-motion and separated-profile drafts exercised natively; version/loader immutable. |
| Icons/colors, memory/JVM, Java/window defaults, folders/account | Yes | Yes (Linux AMD64) | Pending | Native draft Cancel/Save for instance name/window/fullscreen, profile icon/color, memory 8192 + JVM arg, isolated folders and default account; catalog persisted. |
| Bundled two-step / Separated four-step creation | Yes | Yes (Windows) | Pending | Q6 separated mode created a second Profile for an existing Instance with the installation preserved. |
| Version, loader/build and location pickers | Yes | Yes (Windows, Linux) | Pending | Real catalogs; Q6/Q10 loader picks, Q3 ARM64 restriction, Q9 search/scroll. |
| Vanilla installation, progress, retry and resume | Yes | Yes (Windows, Linux) | Pending | Windows Q6 interrupted download→Retry→Repair complete→launch; Q10 1.8.9/1.12.2; Linux AMD64 Q6 1.20.6. |
| Fabric installation and build selection | Yes | Yes (Windows, Linux) | Pending | Q6 Windows Fabric 0.19.5→title; Q10 Fabric 1.16.5→title; Linux fixture/AMD64 paths. |
| Quilt installation and build selection | Yes | Yes (Windows) | Pending | Q6 Quilt 0.30.1 on 1.21.11; Q10 Quilt 1.18.2→title; no-builds error now clean. |
| Forge modern/legacy installation and build selection | Yes | Yes (Windows) | Pending | Q6 Forge 1.21.11 and legacy Forge 1.12.2→title (FML log); Q10 legacy rep. |
| NeoForge full catalog and installation | Yes | Yes (Windows) | Pending | Q6 NeoForge 21.11.48→title; Play while installing now queues from Home and auto-launches on completion (e4b3be6, native Windows check). |
| OptiFine standalone/Forge import and installation | Yes | Yes (Windows) | Pending | Existing-target re-import edits OptiFine in Instance settings (select/replace/remove, draft Cancel/Done, atomic catalog commit + reinstall); native J1/G8 re-import on existing 1.21.1/1.16.5 instances reached the menu with OptiFineTweaker logs, Cancel kept the installed version, mismatched JARs produced the targeted error. Wizard imports 1.21.1 J1, 1.16.5 G8 and 1.12.2 G5 standalone also launched. G5-as-Forge remains refused (no declared Forge version); the older C5 installer format lacks launchwrapper-of.txt and is unsupported. |
| Quick Play and saved server settings | Yes | Yes (Windows client → Linux AMD64 server) | Pending | 1.20.6 Quick Play and legacy 1.12.2 --server/--port both joined a real official server; server logs record "Q5Tester joined the game" / "Q5Legacy joined the game"; in-game and Home captures under target/parity-shots/windows-amd64/e2e-quickplay-*.png and linux-amd64/{quickplay,legacy}-join.log. |
| Productized LWJGL 3 ARM64 compatibility and Java 8 | Yes | Partial | Pending | Linux ARM64 substitutes official ARM64 LWJGL natives (pre-3.3.1 → reviewed 3.3.1 composition) and provisions Azul Zulu Java 8 via official metadata; native 1.16.5/1.18.2 Play verified. Windows ARM64 substitution merged (016c695), native VM run pending. |
| ARM64 LWJGL 2 picker restriction | Yes | Yes (Linux ARM64) | Pending | Rows stay visible but disabled with the inline reason; typed exact IDs are refused in-dialog. Native proof on Linux ARM64 (851e23b); macOS/Windows ARM64 picker checks pending. |
| Deletion, confirmation and linked Profiles | Yes | Yes (Windows) | Pending | Q6 separated deletion dialog; player marker file survived, linked Profile rules held. |

### Settings

| Feature | Built | Native E2E | Accepted | Note |
| --- | --- | --- | --- | --- |
| General: theme preview/save/cancel, reduced motion | Yes | Yes (Windows, Linux) | Pending | Windows Q6 Dark preview/Cancel/Save; Linux in-app + GTK system reduced motion; Windows SPI query wired. |
| Game: Java, memory/JVM, window/fullscreen, snapshots | Yes | Yes (Linux AMD64) | Pending | Native save defaults (32 GB, 1600×900, fullscreen, snapshots); applied to a fresh wizard; Cancel did not persist; invalid memory/window rejected. |
| Memory 1–32 GB shown in GB, Java choice, window presets/custom, JVM | Yes | Yes (Linux AMD64) | Pending | Aligned to the prototype: 1024–32768 MiB (1–32 GB) centralized bounds and GB input/display; stepper ±1 GB with clamps, finer in-range values preserved; out-of-range preferences/profiles clamp on load/save and migrate. Native checks 0.25/64 GB rejected, 32 GB saved; presets/custom, fullscreen and JVM validation pass. |
| Profiles: confirmed one-way conversion, Restore defaults | Yes | Yes (Windows) | Pending | Q6 one-way conversion confirmed natively. Restore defaults still pending. |
| Downloads: System/Direct/HTTP(S)/SOCKS5 and connection test | Yes | Yes (Windows) | Pending | Q6 local SOCKS5 forwarder: test HTTP 200 + 22 CONNECT lines during a real install; reset to Direct. |
| Parallel 1–16, retries 0–10, nonnegative MB/s limit | Yes | Yes (Linux AMD64) | Pending | Native parallel 16/retries 10/speed 50 saved; 0 rejected, 11 clamped to 10, negative speed rejected. Copy for the parallel error is being polished. |
| Storage: paths, verified relocation, interruption recovery | Yes | Yes (Windows) | Pending | Q6 native relocation, retained-copy cleanup dialog, relocated game launched. |
| Relocation leases and confirmed retained-copy cleanup | Yes | Yes (Windows) | Pending | Q6 cleanup dialog confirmed the retained copy removal; lock retry hardened for parallel tests. |

### About and Welcome

| Feature | Built | Native E2E | Accepted | Note |
| --- | --- | --- | --- | --- |
| About: build/product, brand, links and copy actions | Yes | Yes (Linux AMD64) | Pending | Version/platform/source fingerprint, links and one build-info Copy action verified with a real clipboard read; external link opening skipped. |
| Welcome: three steps, drafts, skip, download/retry | Yes | Yes (Windows, Linux) | Pending | Q6 Skip created no identity; download/retry and Welcome flows on Windows/Linux. |
| Welcome: atomic new-account/game final commit | Yes | Yes (Windows) | Pending | Identity is memory-only until the final commit; failed save rolls back. Native run 1ac4564. |

### Cross-page

| Feature | Built | Native E2E | Accepted | Note |
| --- | --- | --- | --- | --- |
| Light/dark layout, typography, icons and states | Partial | Pending | Pending | Windows captures complete; visual comparison in review. |
| Directional 460 ms upstream opacity/offset transitions | Yes | Yes (Linux AMD64) | Pending | Q9 D6 sampled the sheet slide/fade and page resize/fade at ~460 ms with upstream opacity/offset; Windows/macOS visual timing audit remains. |
| System/in-app reduced motion | Yes | Partial (Linux, Windows) | Pending | In-app toggle native; Linux system reads GTK `gsettings`; Windows system reads `SPI_GETCLIENTAREAANIMATION` — with SPI disabled 10/10 frames were identical (no transition), restoring SPI could not be shown frame-by-frame because the capture completes after the transition (automation gap, unit tests cover the policy); macOS NSWorkspace. |
| Fit-to-display at creation and recovery | Yes | Yes (Linux AMD64) | Pending | 1280×800 and 1366×768 captured (Home clamps to 432×620, management 960×600 fits). Windows/macOS native checks pending. |
| CJK IME, clipboard, search and scrolling | Yes | Yes (macOS, Linux AMD64) | Pending | Real macOS Pinyin composition/commit/cancel with candidate window, Cmd+C/V read back via pbpaste, CJK search/filter, Tab/Escape behavior and draft preservation passed; Linux AMD64 clipboard/search/scroll also passed. Enter during composition commits raw pinyin (system behavior, same in TextEdit). |
| Enter/Escape/Tab, draft/error preservation | Yes | Yes (Linux AMD64, Windows) | Pending | Q9 D6 Tab/Enter/Escape, draft restore and error behavior; Q6 Windows dialogs/overlays. |
| In-window overlays: Sheets for Home drawers, Dialogs elsewhere | Yes | Partial | Pending | Windows AMD64 native check passed on c18e416: one window in every state, stacked Dialog over Sheet, Escape closes only the topmost layer, focus returns to the avatar, Add account is the Accounts page in the same window and saves/restarts correctly. macOS native overlay check passed (real app, one visible window in every state; Settings/About previews in-window with Escape returning to onboarding; Add account/Accounts page in the same window with size restore; Accounts Sheet with a Remove-account Dialog stacked above it, Escape closing only the top layer and focus returning to the invoking control). A hidden off-screen untitled 500x500 AppKit helper window exists at launch but is not user-visible. macOS scenario matrix recaptured (142 shots). |

## Smoke matrix

Entries are Minecraft version, API or UI, and the tested commit. API includes CLI/manual-library drivers. Empty cells remain unverified; details and images live in target/parity-shots/<platform>/.

| Host | Vanilla | Fabric | Quilt | Forge | NeoForge | OptiFine | Authlib | Older |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| macOS ARM64 | 1.20.6 UI 78f1595 | 1.20.6 API 41982ff | 1.20.6 API 41982ff | 1.20.1/1.21.1 API 80ed125 | 1.20.1/1.21.1/26.3 API 80ed125 | 1.21.1 standalone/Forge API 78f1595 | 1.20.6 API fixture 553834f | NeoForge 1.20.1 API 80ed125 |
| Windows AMD64 | 1.20.6/1.8.9/1.12.2 native Q6/Q10 | 1.16.5 native Q10 | 1.18.2 native Q10 | 1.21.11 + 1.12.2 native Q6 | 1.21.11 native Q6 | 1.21.1/1.16.5/1.12.2 native Q10 | | Forge 1.12.2 native Q6 |
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

- Q9 D8 Windows lifecycle ×50 on e4b3be6: 49/50 passed; the failure (and 1/60 isolated repro) was a ~2% Windows race where the atomic session-record replace hit ERROR_ACCESS_DENIED/ERROR_SHARING_VIOLATION from a transient external handle, leaving the monitor record active. Fixed with a bounded retry (5 attempts, 20→200 ms, Windows errors 5/32 only) in session_host storage; 50/50 full-suite and 100/100 isolated runs then passed. Evidence in target/parity-shots/windows-amd64/q9-lifecycle/.
- Storage lock flake (pre-existing) was an inherited `flock` across a concurrent fork/exec; a bounded 500 ms retry on transient would-block (and test retries) now prevents it, with repeated workspace runs green.
- Native screenshots cover 71 debug scenarios in light/dark; every overlay now renders inside its parent window (one capture per scenario). Scenario captures are visual fixtures.
- Comparison board: target/parity-shots/index.html (Windows primary) and macos-arm64.html; 142 single-window Windows scenario shots (71 stems) captured at d1be580 after the overlay refactor and Q2 fixes.
- Remote picker captures prove navigation and overlays, not native installation/Play.
- Installer processors use verified Store inputs and run offline; macOS SOCKS5 backend smoke passed.
- macOS live PKCE/Device Code, native Welcome/download/Play/menu/Stop and restart/Play/menu/Stop passed; dedicated test accounts retained.
- macOS storage relocation/cleanup followed by game menu passed through the API.
- Linux ARM64 substitutions are explicit in the saved plan and the launch log; the AI/CLI pass the same note through. The 1.16.5 composition also adds the official checksum-verified slf4j-api 1.8.0-beta4 companion required by the reviewed text2speech (native crash found and fixed during Part B).
- Q5 real server joins passed natively: Windows AMD64 client to a Linux AMD64 official server via manifest Quick Play (1.20.6) and legacy --server/--port (1.12.2); server logs confirm both joins.
- Q6 Windows completes except automation-only gaps: Welcome Skip, loaders, Settings/SOCKS5/storage, separated mode, crash help, retry, and Play-while-installing queueing from Home (e4b3be6) were verified natively; Q10 older representatives and OptiFine J1/G8/G5-standalone reached title screens with logs. The running-card Stop square and clipboard Ctrl+V could not be driven by synthetic input on the host; Stop lifecycle was verified repeatedly earlier and queue-clearing is unit-tested. Evidence in target/parity-shots/windows-amd64/e2e-q6-*/e2e-q10-*/queued-*.png.
- Memory bounds aligned to the prototype 1–32 GB on latest main (b468b9f): centralized bounds, GB labels/steppers, migration/clamp behavior and tests; native Linux AMD64 checks pass.
- Q9 follow-ups on latest main: a blank Games page opened from the menu during setup is fixed (setup stays visible with a status/toast and the draft is retained), developer-style parallel-download/offline-username validation copy is polished, and the stale-reservation launch failure now yields an actionable session diagnosis with a manual recovery instruction. Memory bounds alignment to the prototype 1–32 GB is in review.
- Q7(a) native re-import verified on Windows: existing 1.21.1/1.16.5 instances removed, re-imported J1/G8 via the editor, reinstalled on Save and launched with OptiFineTweaker logs; mismatched JARs rejected inline; Cancel preserved the installed version.
- Q7(c) provider icons verified natively on Linux AMD64 (b7730bc): real PNG icon rendered on Providers/detail, cached file+sidecar+fixture sha match; corrupt/oversized icons fall back to the monogram silently with no cache; stale cache works offline; missing cache offline is a monogram. Evidence in target/parity-shots/linux-amd64/q7-provider-icon-*.png.
- Q6(3)+Q7 fixture lane on Linux AMD64 (eb11b2f): local Yggdrasil fixture (RSA-2048 SHA1withRSA textures), native sign-in, character choice, saved account with real avatar, 1.20.6 + authlib-injector 1.2.8 → main menu → quit, and silent initials fallback after the fixture stops. The keyring was unlocked by the user through a temporary VNC session. Evidence in target/parity-shots/linux-amd64/e2e-q6-authlib-*.png and q7-avatar-*.png. The launcher Stop button did not take synthetic clicks while the game window was focused (game-side first-run prompt too); stopping through the game menu worked and returned "Exited normally" — recorded as an input-routing observation, not reproduced as a logic defect.
- Q8 Wayland degradation verified natively on Linux ARM64 (c860ffe): native Wayland window opens and the Add-provider dialog shows the documented unsupported message without pretending a target; Xwayland attaches the XDND overlay and paste/discovery/save work. The host image has no Wayland input-injection tooling, so native-Wayland footer clicks were not reliable (uinput helper used for typing). Evidence in target/parity-shots/linux-arm64/e2e-q8-wayland-*.png.
- Linux XDND drop target merged and re-verified natively on a5e34da: real Firefox drag accepts a quick link, plain text is rejected byte-identically, close/reopen re-attaches, the overlay disappears on close, and idle frames stay identical (no repaint loop). Wayland remains unsupported by the pinned GPUI (clear error, options recorded).
- Windows ARM64 LWJGL substitution is merged (016c695): explicit `windows_arm64_lwjgl3` compatibility, official Maven Central `natives-windows-arm64` artifacts (3.3.x same version; 3.2.x replaced with the reviewed 3.3.1 composition), plan guards and a `Compatibility:` log note; fixture-tested, x64 unchanged. The native ARM64 VM run waits for a Mac-idle window.
- ARM64 hosts (Linux, Windows, macOS) keep LWJGL 2-era versions visible but disabled in the version picker with "official LWJGL 2 natives are x86/x64 only"; typed exact IDs are refused with the same reason (3e5a801, native Linux ARM64 proof). Linux ARM64 native 1.16.5/1.18.2 wizard→Play→menu→Stop passed at 851e23b with the Azul runtime-substitution lines and the title-screen game-log lines; captures in target/parity-shots/linux-arm64/.
- Linux AMD64 native Device Code saved on 0f7e26b after the client ID was supplied at launch; the built binary refuses sign-in without it.
- Windows native first-run flow passed on 1ac4564: pending identity discarded on quit, account and game saved together, real 1.20.6 download, Play, main menu and Stop.
- CI covers Windows AMD64, Linux AMD64, Linux ARM64 and macOS ARM64; remote hosts are separate evidence. sccache is installed on the macOS host and the Linux ARM64 VM for host rebuilds.
- All four test hosts share a WireGuard overlay (Mac hub 10.77.0.1; Windows AMD64 10.77.0.11, Windows ARM64 10.77.0.12, Linux ARM64 10.77.0.21, Linux AMD64 10.77.0.22); the Mac forwards between peers and the fixed Mac LAN address is the endpoint. The Linux AMD64 VM must still be started detached from its Windows host, and its login keyring relocks after a reboot.
- Linux ARM64 uses Xwayland :1 and Spectacle; Linux AMD64 uses X11 window capture.
- Q9 D6 on Linux AMD64 (191bf24): 1366×768 fit-to-display clamps Home to 432×620 and fits management 960×600; in-app reduced motion removes transitions; upstream 460 ms sheet/page transitions observed; Tab/Enter/Escape, clipboard, search, scrolling and draft restore pass. Linux system reduced motion is now read from GTK `gsettings`; Linux and Windows system reduced-motion queries are both wired (Windows native recheck pending). Minor: opening a section dialog clears the inline field error and cancelling does not restore it (toast/draft stay; next Continue revalidates).
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
| Real quick-link drag on hosts automation cannot drive | Windows AMD64 done with a real SendInput browser drag. macOS drag still needs the user batch (Mac in use); Linux X11 verified, Wayland documented unsupported. |

## v0 notes

- Scoped unsafe exception: `crates/block-drop` is the only crate that lowers `unsafe_code` (workspace still forbids it); every unsafe block carries a SAFETY comment, and the crate owns its child HWND/IDropTarget without patching, subclassing or swizzling GPUI.
- Architecture governance, file splitting, layering and lint tidy-up remain deferred.
- Preserve clippy::all, existing deny list and unsafe_code=forbid outside the authorized dedicated drop-target crate.
- Accessibility, release packaging/signing and mirror sources remain deferred.
