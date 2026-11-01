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

/// A synthetic Modrinth archive with one optional file and no real mod content.
pub(super) fn write_modpack(path: &std::path::Path) -> anyhow::Result<()> {
    const ZIP_HEX: &str = "504b0304140000000800000021588bc5782127010000b6010000130000006d6f6472696e74682e696e6465782e6a736f6e5550cb6e84300cfc952a67c4c6793afcc19eabf652edc1244ea1650101da4a5df1ef0d5a556ae593673c637bee224fcb95b6575ed67e1a45039578a72b8b465cfb91e342791395181fd0cbf969a6f85980db63fe9c0a0aa54f3cf398788c3dafa2b9ff1117be56b206b15722f7c341bfddc54c5b77ec98d27a9ae6ad58d1507fd052ac3a5abb87c9da11942183e45b440c0cd62867a5722e8221c0885ea241cc04c11e671485055534d6a3c2846c2c688f6c9d61cd29c564a56c7522a01c0339297534ca1b155a2d432297b2e46cb2cc11988ccd21648b21b3493ab52d2a8596b556927c2c8dca5e3b2d351bef9d54d0b6111c4549faf7dbe7febb048725a0e96b1c264ac7fba2dbb6b9399d40f95a968226fc0fe152091e6f470471e8793c42fce5c5be5ff61f504b01021403140000000800000021588bc5782127010000b60100001300000000000000000000008001000000006d6f6472696e74682e696e6465782e6a736f6e504b0506000000000100010041000000580100000000";
    write_zip_hex(path, ZIP_HEX)
}

/// A plain Minecraft archive with no target metadata.
pub(super) fn write_plain_pack(path: &std::path::Path) -> anyhow::Result<()> {
    const ZIP_HEX: &str = "504b0304140000000800000021581d2541e814000000120000000b0000006f7074696f6e732e7478742baecc2bc9482dc94c56c82f28c9cccf2be60200504b01021403140000000800000021581d2541e814000000120000000b00000000000000000000008001000000006f7074696f6e732e747874504b05060000000001000100390000003d0000000000";
    write_zip_hex(path, ZIP_HEX)
}

fn write_zip_hex(path: &std::path::Path, hex: &str) -> anyhow::Result<()> {
    let bytes = hex
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|digits| Ok(u8::from_str_radix(std::str::from_utf8(digits)?, 16)?))
        .collect::<anyhow::Result<Vec<_>>>()?;
    std::fs::write(path, bytes)?;
    Ok(())
}

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
                optifine: None,
                optifine_mod: None,
            }],
            settings: None,
            pack: None,
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
        authlib: None,
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
