//! Small, network-free product fixtures shared by native behavior tests.
//!
//! These values contain no credentials, player data or playable offline identity.
//! Tests that run the installer use their own temporary catalog instead.

#![expect(
    clippy::redundant_pub_crate,
    reason = "Fixtures are shared only with sibling UI test modules."
)]

use block_auth::{Account, AccountProvider, AccountRef};
use block_launcher::{
    catalog::{
        DataPolicy, InstallStatus, Instance, LaunchTarget, LoaderKind, Profile, ProfileMode,
        Snapshot,
    },
    recipe::{Compatibility, RuntimeSelection},
};
use gpui::AppContext as _;

/// Mount the same Base Root used by product windows, retaining the application
/// entity for assertions. Tab navigation and overlay plugins therefore exercise
/// the production focus route rather than a test-only shortcut.
#[expect(
    clippy::expect_used,
    reason = "The root stores exactly the view type built here."
)]
pub(super) fn window<V: gpui::Render + 'static>(
    cx: &mut gpui::TestAppContext,
    build: impl FnOnce(&mut gpui::Window, &mut gpui::Context<'_, V>) -> V,
) -> (gpui::Entity<V>, &mut gpui::VisualTestContext) {
    let (root, cx) = cx.add_window_view(|window, cx| {
        crate::initialize(cx);
        let view = cx.new(|cx| build(window, cx));
        crate::ProductRoot::new(view, window, cx)
    });
    let view = root
        .read_with(cx, |root, _| root.view().clone().downcast::<V>())
        .expect("the Base Root retains the supplied product entity");
    (view, cx)
}

/// Complete physical presses. GPUI's `simulate_keystrokes` intentionally emits
/// only `KeyDown`; native Base controls activate on the matching `KeyUp`.
#[expect(
    clippy::expect_used,
    reason = "Test key sequences are static valid GPUI bindings."
)]
pub(super) fn press(cx: &mut gpui::VisualTestContext, keys: &str) {
    if let Some(window) = cx.windows().last().copied() {
        *cx = gpui::VisualTestContext::from_window(window, &cx.cx);
    }
    for key in keys.split(' ') {
        let keystroke = gpui::Keystroke::parse(key).expect("valid test keystroke");
        let pressed_in = gpui::VisualContext::window_handle(cx);
        cx.simulate_keystrokes(key);
        if cx
            .windows()
            .iter()
            .any(|window| window.window_id() == pressed_in.window_id())
        {
            cx.simulate_event(gpui::KeyUpEvent { keystroke });
        }
        ensure_live_window(cx);
    }
}

pub(super) fn ready_snapshot() -> Snapshot {
    Snapshot {
        profile_mode: ProfileMode::Bundled,
        instances: vec![Instance {
            id: "instance-a".into(),
            name: "Test game".into(),
            minecraft_version: "1.20.6".into(),
            directory: std::env::temp_dir().join("block-ui-fixture-only"),
            targets: vec![LaunchTarget {
                id: "vanilla".into(),
                kind: LoaderKind::Vanilla,
                loader_version: None,
                manifest: None,
                status: InstallStatus::Ready,
            }],
            settings: None,
        }],
        profiles: vec![Profile {
            id: "profile-a".into(),
            name: "Test profile".into(),
            instance_id: "instance-a".into(),
            target_id: "vanilla".into(),
            runtime: RuntimeSelection::Managed,
            memory_mib: 2048,
            jvm_args: Vec::new(),
            game_args: Vec::new(),
            data_policy: DataPolicy::Isolated,
            default_account: None,
            compatibility: Compatibility::Official,
            appearance: block_launcher::catalog::ProfileAppearance::default(),
            inheritance: block_launcher::catalog::ProfileInheritance::default(),
            server: block_launcher::catalog::ServerPreferences::default(),
        }],
    }
}

pub(super) fn test_account() -> anyhow::Result<Account> {
    Ok(Account {
        reference: AccountRef {
            player_id: "00000000-0000-4000-8000-000000000001".parse()?,
        },
        provider: AccountProvider::Microsoft,
        player_name: "TestPlayer".into(),
        client_id: "synthetic-public-client".into(),
        credential_ref: "synthetic-no-credentials".into(),
    })
}

/// Locate the real product control in its owning window after dialog migration.
pub(super) fn control_bounds(
    cx: &mut gpui::VisualTestContext,
    selector: &'static str,
) -> Option<gpui::Bounds<gpui::Pixels>> {
    cx.run_until_parked();
    let alias = match selector {
        "wizard-next" => Some("section-done"),
        "wizard-back" => Some("section-cancel"),
        _ => None,
    };
    if let Some(alias) = alias {
        for window in cx.windows().into_iter().rev() {
            let mut candidate = gpui::VisualTestContext::from_window(window, &cx.cx);
            if let Some(bounds) = candidate.debug_bounds(alias) {
                *cx = candidate;
                return Some(bounds);
            }
        }
    }
    for window in cx.windows().into_iter().rev() {
        let mut candidate = gpui::VisualTestContext::from_window(window, &cx.cx);
        if let Some(bounds) = candidate.debug_bounds(selector) {
            *cx = candidate;
            return Some(bounds);
        }
    }
    None
}

pub(super) fn ensure_live_window(cx: &mut gpui::VisualTestContext) {
    let current = gpui::VisualContext::window_handle(cx);
    let windows = cx.windows();
    if !windows
        .iter()
        .any(|window| window.window_id() == current.window_id())
        && let Some(window) = windows.last()
    {
        *cx = gpui::VisualTestContext::from_window(*window, &cx.cx);
    }
}
