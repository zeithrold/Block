# Codex Goal Prompts (v5)

Context and decisions: [next-tasks.md](next-tasks.md).

| Prompt | Runner | When |
| --- | --- | --- |
| Pilot Goal | Codex | Now |
| Prototype screenshots | DeepSeek Harness | After the Pilot Goal finishes P0 (the prototype snapshot commit) |

---

## Pilot Goal

```text
Goal: Ship the Block pilot. It is a complete native Minecraft launcher: every page matches the React prototype subject to the explicit product decisions below, every feature the prototype exposes works against real services, and nothing is faked or left as an unimplemented placeholder. Real compatibility and state restrictions may disable an action with an explanation. Architecture governance is deferred to v0. Spend your effort on product completeness and correctness, not on code structure.

Repository: /Users/zeithrold/Documents/Develop/Native/Block
Prototype (read-only reference): /Users/zeithrold/Documents/Develop/Frontend/Block-UI (its README.md is the design spec)
Read first: docs/next-tasks.md, then git status, the justfile, docs/minecraft-launcher-design.md, docs/recipes.md, docs/session-supervisor.md and the prototype README.

== Execution priority and user availability ==
- Finish the foundations first: the cleaned baseline and CI, unpatched dependencies, working native-window lifecycle on all platforms, and usable screenshot scenarios and comparison tooling. Follow P0 → P1 → P2 before expanding page/backend work. A source-only implementation is not proof that these foundations work.
- Prepare actual native screenshots in light and dark, the comparison board, and a short list of differences for the user's review on the next day (2026-10-22, Asia/Taipei). The user will judge visual acceptance from the screenshots; do not block other implementation on an immediate taste decision or substitute automated pixel tests for that review.
- Defer user-assisted live sign-in to the morning of 2026-10-22, Asia/Taipei, when the user is available. Do not start expiring Device Code requests in advance. Use Offline accounts and local auth fixtures meanwhile and continue all independent work.
- These are sequencing instructions, not permission to claim acceptance or live authentication has passed. User review and required live sign-in remain pending until actually performed.

== Working rules ==
1. Commit and push directly to main, as often as is useful. No PRs, no branches required, no force-push, no history rewriting.
2. Project-authored docs, code comments, commit messages and UI strings committed from this work are in English. Preserve original third-party source, attribution and license text.
3. Keep verification lean.
   - While working, run targeted `cargo check` and `cargo test -p <crate> <filter>`.
   - Run the full `just check` once before each push.
   - CI (Windows AMD64, Linux AMD64, macOS ARM64) is the cross-platform evidence.
   - Do not write hash receipts, source fingerprints, attempt logs, evidence JSON or per-item ledgers.
   - Do not re-run passing checks just to re-confirm them.
   - If a check fails, fix it and re-run it. Do not create a separate failure ledger; retain normal diagnostic output and report unresolved blockers honestly.
4. Testing.
   - Test business logic, protocols and state rules: drafts and saving, persistence, catalog/profile rules, auth flows, installers, launch arguments and late async results.
   - New network protocols get local HTTP fixture tests covering success, expiry, throttling, cancellation and error mapping. Follow the existing block-auth/block-download style. Tests never touch the internet.
   - Do not write tests for pixel layout, focus-tree internals, accessibility, animation frames or framework mechanics.
   - Delete existing tests that only cover code you remove.
5. Architecture governance is out of scope.
   - Do not refactor for structure, introduce layers, traits or registries, split files, or tidy code that already works.
   - Build features inside the existing structure, with straightforward code.
   - Extract a shared component only when the product itself needs it, for example the same control appearing on several pages.
   - When structure gets in your way, add one line to the "v0 notes" section of docs/STATUS.md (at most 15 lines) and keep going.
6. If the clippy `pedantic` or `nursery` groups cost you time, you may drop them from the workspace lints. Do not spend time cleaning up existing `#[expect(...)]` attributes. Keep `clippy::all`, the existing deny list (unwrap/expect/panic/todo/dbg/unimplemented/wildcard_imports) and `unsafe_code = "forbid"`.
7. Docs.
   - Maintain only docs/STATUS.md, in English and at most 200 lines. It holds the feature inventory checklist, page status, intended differences from the prototype, product decisions, the Needs-user table and the v0 notes.
   - Tick the boxes in docs/next-tasks.md as you go.
   - Fix dangling references in existing docs and update existing packaging notices/licenses when required by P0/P1. These are exceptions to the STATUS-only maintenance rule.
   - Do not create other docs.
8. Product honesty.
   - Every feature in the prototype must work for real by the end.
   - Do not port the prototype's demo-only behavior: fake sign-in, BroadcastChannel, simulated downloads or progress, sample data, the Reopen button, /auth/preview and "Interface preview" copy.
   - Never show fake success or fake progress.
   - Compatibility, validation and busy-state restrictions are allowed and must explain the reason. Unimplemented-feature placeholders are not allowed. Legacy ARM64 game support is best effort under P5, not a promise that every old version can run.
   - The only planned omission is mirror download sources (BMCLAPI and similar). Use official sources only, and offer only "Official" wherever the prototype has a source picker.
9. Use temporary or dedicated --data-dir directories only. Never copy player credentials or worlds. Never write .env values into logs, commits or screenshots.
10. When something needs the user (permissions, real sign-in, taste calls), add it to the Needs-user table in STATUS.md, tell the user, and continue with other work while you wait. Batch visual review and live sign-in for the availability window above instead of requesting immediate review after every page.

== P0 Baseline and cleanup ==
- In the prototype repo, snapshot-commit the current working tree once with `git add -A && git commit`. Change no content and do not push. Use that commit as the reference from now on.
- Move the product decisions that still hold into a new English docs/STATUS.md. Take them from docs/ui-parity/ and docs/gpui-product-progress.md: Profile mode rules, window policy, the first-run flow, sign-in boundaries and download policies.
- Before staging or making any baseline commit in Block, delete the following and fix any dangling references:
  - docs/ui-parity/, including its 45 MB evidence folder
  - docs/block-ui-parity-audit.md
  - docs/gpui-ui-parity-plan.md
  - docs/gpui-ui-parity-goal-prompt.md
  - docs/gpui-product-progress.md
- Keep docs/mvp-progress.md, docs/session-supervisor.md, docs/recipes.md and docs/minecraft-launcher-design.md.
- Add the screenshot output folders under target/ to .gitignore if they are not already ignored.
- Old evidence and ledgers must not enter Git history. Do not commit them first and delete them later, archive them, or copy them into another committed path. Inspect the staged paths before each baseline commit. If any are already in existing history, report that fact; history rewriting remains unauthorized.
- Only after the cleanup above, commit the retained working tree to main in a few logical commits, push, and check CI. Group by vendor, backend crates, block-ui, CI/scripts/packaging and retained English docs as appropriate; do not create a commit for deleted untracked materials. Red CI comes first.

== P1 Platform simplification ==
- GPUI crates:
  - Remove vendor/gpui-pre, vendor/gpui-pre-macos and vendor/gpui-base, together with the [patch.crates-io] and exclude entries.
  - Depend on the published gpui-pre, gpui-pre-platform and gpui-base crates unmodified. Keep =0.3.9 / =0.8.0 unless they fail to build unpatched.
  - Update the packaging notices and licenses that referenced vendor/.
- Window size:
  - All windows are fixed-size and cannot be resized by the user. Home is 432×768; full management pages are 960×600. Dialog and picker windows use fixed, content-appropriate sizes.
  - Keep the existing fit-to-display logic for short screens, applied when a window is created.
  - Remove layout code that existed only for user resizing.
- Window chrome:
  - macOS uses the upstream transparent titlebar with native traffic lights. Remove the custom window-control buttons and drag handling.
  - Windows and Linux keep system decorations.
- Dialogs. Every dialog, confirmation, drawer and modal picker that is currently an in-window overlay becomes its own GPUI window: not an OS alert component, and not a layer inside the parent.
  - Examples: Home's profile/account selector and downloads drawers, remove/delete confirmations, the wizard's version/loader/location pickers, and the sign-in wizard.
  - Choosing the window type: check WindowKind and WindowOptions in the gpui-pre 0.3.9 source in the cargo registry and pick the most suitable kind. Position the window relative to its parent when you create it.
  - The parent window stays interactive. Prevent conflicts with state, not with input blocking:
    - only one window per purpose
    - triggering an open window again focuses it
    - child windows close with their parent
    - results flow back through entities or events
- Anchored lightweight elements (select lists, menus, tooltips) use the unpatched gpui-base components as they are: click outside to dismiss, no backdrop.
- Delete:
  - OverlayHost and the stacked overlay system
  - inert/backdrop handling and the paint_scale animations
  - drag-to-dismiss drawers
  - accessibility semantics and AX helpers: the .accessibility(...) calls and related infrastructure; keep system reduced-motion preference queries
  - crates/block-ui/tests/{accessibility,inert,paint_scale}.rs and examples/native_*.rs
  - test assertions that only cover AX or overlays
- Animations use only what upstream GPUI provides: opacity and offset.
- Done when vendor/ is gone, CI is green on all three platforms, and every former overlay works as a native window.

== P2 Comparison tool ==
- block-app gets a debug-only flag: `--scenario <name> --theme <light|dark>`. It opens a window in a fixed state built from fixtures, with a temp data dir and no network. Cover states that are hard to reach by clicking, such as:
  - home-ready, home-downloading
  - games-bundled, games-separated, wizard-review
  - accounts-list
  - each settings section
  - about, the welcome steps
  - each dialog window
- scripts/snap-native runs on macOS. It launches each scenario and captures its primary window with CGWindowID + `screencapture -l` into target/parity-shots/native/<scenario>-<theme>.png. Save additional windows as <scenario>-<window-purpose>-<theme>.png so they do not overwrite each other. If Screen Recording permission is missing, add it to Needs-user and continue.
- scripts/parity-board writes target/parity-shots/index.html, showing the prototype and native shots side by side.
  - Prototype shots come from target/parity-shots/proto/ and use the same names. A separate agent writes them.
  - Until they exist, open the prototype dev server yourself.
- Match scenario names and logical content sizes across both tools; account for native display scaling and show content crops at equal logical scale. Label fixture screenshots as UI scenarios, not real-service verification. Native chrome and separate dialog windows are intended differences.
- Capture the prototype product content from its P0 snapshot. The separate screenshot task may add scripts, development dependencies and selector-only data-scenario attributes; those changes must not alter the reference appearance or behavior.

== P3 Inventory ==
Walk every route, component and option in the prototype, and every disabled, unavailable or placeholder item in the native code. Write the results into STATUS.md as one checklist, grouped by page. Each line names a feature or state and its real backend need. This checklist defines "complete".

== P4 Pages, each with its full real backend ==
Order: Home → Accounts and sign-in → Games and Game Wizard → Settings → About and Welcome → cross-page items.

1. Home
   - Player area, profile/account selector windows, Play/Stop, downloads window, empty/busy/error states.
2. Accounts and sign-in
   - Microsoft PKCE is already real; keep it working.
   - Microsoft Device Code:
     - request and poll, handling interval, slow_down, expiry and cancel
     - then reuse the existing XBL → XSTS → Minecraft chain and secure storage
     - a public client never uses a client secret
   - Authlib-injector:
     - provider management page: add, edit and remove; API discovery via the ALI header; cached metadata (name, icon, skin domains)
     - Yggdrasil authenticate, refresh, validate, invalidate and signout, with multi-profile selection; tokens go into secure storage
     - at launch, download authlib-injector from its official API (authlib-injector.yushi.moe), verify it, and add `-javaagent:<jar>=<api>` and `-Dauthlibinjector.yggdrasil.prefetched=<base64 metadata>`
   - Offline: username validation, and a UUID v3 derived from "OfflinePlayer:<name>", following the prototype's options.
3. Games and Game Wizard
   - Bundled and Separated modes; details; instance, launch and game settings sub-pages; creation; Home sync.
   - Loaders (version lists filtered by Minecraft version; installs resumable and verified; failures retryable; progress from real tasks):
     - Fabric (meta.fabricmc.net v2) and Quilt (meta.quiltmc.org v3)
     - Forge: 1.13+ via installer and processors, reusing the NeoForge path; 1.12.2 and earlier via the legacy universal/launchwrapper layout
     - NeoForge: the full version list instead of one pinned build
     - OptiFine uses a user-assisted download from optifine.net followed by automatic installation. Open the official download page, guide the user to download the JAR, let them select it, validate its identity/version and compatibility, and automate the remaining standalone or Forge-mod installation. Do not require manual extraction or file placement, scrape around the download flow, bundle the JAR, or redistribute it. When it is incompatible with the chosen loader, disable it and give the reason. A missing user-supplied JAR is an explicit waiting state, not a fake success or an unimplemented placeholder.
4. Settings: all five sections (general, game, profiles, downloads, storage), with saved, draft and preview kept separate.
   - SOCKS5 proxy (reqwest socks feature), alongside the existing System/Direct/HTTP(S) policies, concurrency, rate limits and retries.
   - Storage relocation, done transactionally:
     - order: copy, verify, atomically switch the config, then delete the old directory only after the user confirms
     - recovers or rolls back after an interruption
     - blocks installs and launches while it runs
     - fails early when disk space is short
5. About and Welcome: the three-step first-run flow, with real saving.
6. Cross-page: page/detail/wizard motion with correct directions, reduced motion (system and in-app), short displays, and every remaining inventory item (for example Quick Play).

A page is done when all of these hold:
a) Its scenarios match the prototype in light and dark: layout, hierarchy, spacing, type, color, radius, icons, and empty/busy/error states. Any intended difference gets one line in STATUS.md, for example a native window instead of a web dialog.
b) Every feature on the page works for real, with no disabled placeholders, and the existing flows show no regressions: PKCE, install, launch/stop, persistence, Profile mode rules and crash help.
c) New logic and protocols have tests (rule 4).
d) It is committed and pushed, and CI is green.
e) You have posted a short report to the user: what changed, the commits, actual native screenshots and the board path, remaining differences, and anything the user needs to do. Prepare pages for batched user visual acceptance; user review can remain pending while other work proceeds.

Shared sizes: Home is 432×768 and management windows are 960×600. The browser column is about 32% wide and the footer 72 px. Motion runs 460 ms with cubic-bezier(0.16,1,0.3,1).

== P5 Real verification ==
- Live sign-in.
  - The user owns a genuine Microsoft account.
  - Start user-assisted verification in the morning of 2026-10-22, Asia/Taipei, when the user is available. Until then, mark it pending and continue independent work; do not repeatedly ask the user to sign in.
  - On macOS, ask the user once to sign in through the app into a dedicated test data dir (for example ~/block-test-data), and reuse it for later launches.
  - On remote hosts, use Device Code: start the sign-in, send the user the code and URL, and wait. Ask at most once per host, batching requests.
  - Use Offline accounts for quick iteration.
  - Test Authlib live only if the user provides a provider account; otherwise use a local Yggdrasil fixture for the protocol flow and the actual Authlib-injected launch below. Distinguish fixture verification from a live-provider pass.
- Install and launch smoke. Run each combination once, and again only after relevant code changes. Install into a test data dir, launch, confirm the main menu by observing/capturing the real game window or by a specifically validated menu-ready signal, then stop. A Java PID, a generic Running supervisor state or render-thread startup alone does not prove the main menu was reached.
  - macOS: Vanilla, Fabric, Quilt, Forge, NeoForge and OptiFine, each on one recent version it actually supports. Also one Authlib-injected launch; use a local Yggdrasil fixture if no live provider account is supplied. OptiFine smoke waits for the user's official JAR when needed.
  - Old versions: exercise a representative older supported version for each loader where one exists; version choices need not be identical across loaders. Verify legacy Forge on Windows. Do not require nonexistent loader/Minecraft combinations.
  - Legacy ARM64 support is best effort on macOS and Linux. Investigate and try viable compatible Java/native-library options from official upstream sources within the existing product and no-framework-patch rules. Do not reject all older versions without investigating, silently use incompatible binaries, or turn this into an open-ended emulation/framework project. Record the tested combination, outcome and concrete limitation concisely in STATUS.md. Proven unsupported combinations may remain unavailable with a reason and do not block completion; uninvestigated combinations are not passes or established incompatibilities.
  - Linux ARM64, Windows and Linux AMD64: Vanilla, Fabric and one modern Forge, plus a UI click-through of Home → Games → a dialog window with a screenshot.
- Remote host procedure: fetch the pushed commit into ~/block-src on each host. Never copy the local working tree or target/. Put screenshots into target/parity-shots/<platform>/.
  - Linux ARM64: `export PATH=$HOME/.cargo/bin:$PATH`, `DISPLAY=:0`, scrot and xdotool.
  - Windows: start GUI apps through `schtasks /create /it`. PowerShell output is in GBK. Clean old target/ folders first, because drive C: is tight.
  - Linux AMD64: start the VM from the Windows host with vmrun (see next-tasks.md). Wait for SSH, run the tests, then stop it with `vmrun -T ws stop <vmx> soft`.
- Also run a UI-only smoke on the three remote hosts right after P1, so platform-window problems surface early.

== Stop and ask the user when ==
- the prototype conflicts with real backend behavior in a way you cannot resolve
- a licensing or distribution question remains unresolved after applying the approved official-download-then-automatic-install flow for OptiFine
- something would delete user data
- a removed patch turns out to be essential and has no upstream alternative (write down the problem and the options, then continue elsewhere)
- the same CI failure survives three fix attempts

The Pilot Goal is complete when:
- every item in the STATUS.md inventory works for real
- every page meets a–e
- the user has reviewed the native screenshots and accepted the visual result, with any requested corrections completed
- live Microsoft sign-in (PKCE and Device Code) has been verified with the user
- the required supported P5 smoke combinations have passed on all four hosts, and legacy ARM64 best-effort outcomes and unsupported combinations are stated explicitly
- CI is green
- STATUS.md is accurate

Needs-user items that the user has not yet done must be listed explicitly. They are the only acceptable pending external actions while implementation continues. If required visual acceptance, live sign-in or a required smoke run is still pending, report "implementation ready; awaiting user verification" rather than "Pilot Goal complete". A legacy ARM64 limitation established under the best-effort policy is a stated support boundary, not a pending user action.

Authorized:
- commit and push to main
- one snapshot commit in the prototype repo, without pushing
- the deletions listed above
- using the test hosts in .env over SSH and vmrun
- downloading games, loaders and authlib-injector from official sources into temporary or test directories; guiding the user to download OptiFine from optifine.net and automatically installing their selected JAR

Not authorized: force-push or history rewriting, releases, framework patches, editing prototype product content (the separate screenshot task has the selector-only exception below), or deleting user data.
```

---

## DeepSeek Harness: prototype screenshots (start after the Pilot Goal's P0)

Start this only after the Pilot Goal has made its snapshot commit in the prototype repo. Otherwise these uncommitted changes would be swept into that commit.

```text
In /Users/zeithrold/Documents/Develop/Frontend/Block-UI, add scripts/snap-proto (Playwright + Chromium).

Scenarios. Start the dev server and capture:
- Pages: home-empty, home-ready, home-downloading, games-bundled, games-separated, games-detail, games-instance-settings, wizard-setup, wizard-review, accounts-list, accounts-detail, accounts-providers, every auth-wizard step, settings-general, settings-game, settings-profiles, settings-downloads, settings-storage, about, welcome-1, welcome-2, welcome-3.
- Dialogs and drawers, each on its own: home-profile-drawer, home-downloads-drawer, wizard-version-picker, remove-confirmation.

Capture settings:
- Viewport: Home 432×768, everything else 960×600, deviceScaleFactor 2.
- Capture both light and dark themes, switched through the settings or the .dark class.
- Disable animations before each capture.
- Crop to the app card only, without the page background.
- Use the P0 snapshot as the product reference. Selector-only attributes must not change its appearance or behavior; do not incorporate later product edits.

Rules:
- Reach states by clicking and typing from the script. Do not change product components; the only exception is adding data-scenario attributes where you need a stable selector.
- Write each image to /Users/zeithrold/Documents/Develop/Native/Block/target/parity-shots/proto/<scenario>-<theme>.png.
- Only touch scripts/, the devDependencies in package.json, and data-scenario attributes. Write all code and comments in English.
- Do not commit. When done, list the changed files.
```
