# UI polish and reviewability

## Scope

Refine the current desktop UI without changing authentication, installation,
saved game data or independent session supervision. Crate reorganization and a
Layout + Slot navigation framework are proposals only; neither is implemented
by this package.

## Design decisions

- Microsoft is the default account choice. Authlib and Offline live under
  “Other sign-in options” at every entry point. An already selected alternative
  stays visible, and disclosure never clears the authentication draft.
- Account identity uses the cached skin face and hat when available, with
  initials as the fallback. The account type remains a text badge; a decorative
  provider mark must not obscure the face or imply a different provider.
- Selector badges use semantic theme colors while retaining text labels.
  Account authority and game version/Loader remain distinguishable without color.
- Drawers retain their modality, gestures, reversal and reduced-motion behavior.
  Rounded upper corners and a small inset below the title bar distinguish the
  full-height panel from a replacement page.
- Home's launch progress and primary action have explicit separation, including
  the queued-launch state.

## Work and acceptance

1. Account UI and shared avatar rendering: independent implementation in
   `authentication.rs`, `providers.rs` and `components.rs`; coordinator integrates
   browser call sites. Cover disclosure, retained selection and skin cropping.
2. Drawer framing: independent implementation in `drawer.rs` and existing drawer
   tests. Verify top inset, corners and unchanged focus/dismissal behavior.
3. Home and selectors: coordinator owns `shell.rs`, `search_selector.rs`,
   `appearance.rs` and their tests. Verify semantic colors in both themes and
   progress/action separation through the rendered layout.
4. Reachability: trace production navigation, contextual actions and setup before
   deletion. Scenario-only entry is not production reachability, but a secondary
   or conditional entry is. Remove confirmed dead implementations and their
   obsolete state/actions, not legitimate error or recovery states.
5. Complexity: enforce selected native Clippy rules and reduce cohesive
   responsibilities in oversized UI code. A file split alone is not a reduction
   in conceptual complexity. Keep the dispatch boundary exhaustive and keep
   lifecycle guards in one place.
6. Architecture: research actual crate consumers and navigation/motion ownership;
   present a concrete recommendation and migration sequence for approval.

## Verification

Use focused package tests while iterating, followed by the `just check` sequence
on the integrated candidate. Keep `--locked`. Inspect the affected native pages
in both themes using disposable scenario data, including expanded account
options, management avatars, selectors, queued launch and full-height drawers.
Static geometry tests complement native interaction; neither old screenshots nor
successful compilation establish visual acceptance.

Independent review covers the integrated UI/dispatch behavior and evidence gaps.
Report exact executed checks and any unavailable native or platform verification.
No publication or landing is authorized.
