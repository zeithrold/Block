//! Product boundary regressions, independent of external prototypes and artifacts.
//!
//! A fresh window owns each rendering assertion because GPUI 0.2.2 retains
//! historical debug bounds between frames. Selectors only locate a real control;
//! assertions check geometry or the result of an actual user action.

use block_launcher::crash::{CrashContext, CrashReport, ExitOutcome};
use gpui::{Bounds, Pixels, TestAppContext, VisualTestContext, px, size};

use super::{
    Tab, UiAction, WizardPage, WizardStep, account_id,
    test_support::{ready_snapshot, test_account},
};

fn inside_viewport(cx: &mut VisualTestContext, selector: &'static str) -> Bounds<Pixels> {
    cx.run_until_parked();
    let bounds = cx.debug_bounds(selector);
    assert!(
        bounds.is_some(),
        "required product control is missing: {selector}"
    );
    let bounds = bounds.unwrap_or_default();
    let viewport = cx.update(|window, _| window.viewport_size());
    assert!(
        bounds.size.width > px(0.)
            && bounds.size.height > px(0.)
            && bounds.left() >= px(0.)
            && bounds.top() >= px(0.)
            && bounds.right() <= viewport.width
            && bounds.bottom() <= viewport.height,
        "{selector} must remain usable inside {viewport:?}: {bounds:?}"
    );
    bounds
}

#[gpui::test]
fn custom_window_chrome_keeps_drag_space_and_controls_inside_the_window(cx: &mut TestAppContext) {
    let (_view, cx) = crate::test_support::window(cx, super::tests::test_view);
    for dimensions in [(960., 600.), (360., 540.)] {
        cx.simulate_resize(size(px(dimensions.0), px(dimensions.1)));
        let menu = inside_viewport(cx, "app-menu");
        let title = inside_viewport(cx, "window-title");
        let minimize = inside_viewport(cx, "window-minimize");
        let close = inside_viewport(cx, "window-close");
        assert!(menu.right() <= title.left());
        assert!(title.right() <= minimize.left());
        assert!(minimize.right() <= close.left());
        assert!(title.size.width > px(0.));
    }
}

#[gpui::test]
fn busy_home_keeps_stop_reachable_and_cancels_the_operation(cx: &mut TestAppContext) {
    let result = (|| -> anyhow::Result<()> {
        let account = test_account()?;
        for dimensions in [(432., 768.), (360., 540.)] {
            let account = account.clone();
            let (view, cx) = crate::test_support::window(cx, move |window, cx| {
                let mut view = super::tests::test_view(window, cx);
                view.snapshot = ready_snapshot();
                view.selected_instance = Some("instance-a".into());
                view.selected_profile = Some("profile-a".into());
                view.selected_account = Some(account_id(&account));
                view.accounts = vec![account];
                view.router.navigate(Tab::Home);
                view.preferences.reduce_motion = true;
                view.busy = true;
                view.operation_cancellable = true;
                view
            });
            cx.simulate_resize(size(px(dimensions.0), px(dimensions.1)));
            let stop = inside_viewport(cx, "home-stop");
            assert!(!view.read_with(cx, |view, _| view.cancel.is_cancelled()));
            cx.simulate_click(stop.center(), gpui::Modifiers::none());
            assert!(
                view.read_with(cx, |view, _| {
                    view.cancel.is_cancelled()
                        && view.play_stop.is_cancelled()
                        && view.busy
                        && view
                            .status
                            .text(crate::Language::English)
                            .starts_with("Stopping")
                }),
                "{dimensions:?}: stop={stop:?}, panel={:?}, content={:?}, state={:?}",
                cx.debug_bounds("home-task-motion"),
                cx.debug_bounds("home-task"),
                view.read_with(cx, |view, _| (
                    view.status.clone(),
                    view.busy,
                    view.can_stop()
                ))
            );
            assert!(view.read_with(cx, |view, _| {
                view.snapshot.instances.len() == 1
                    && view.snapshot.profiles.len() == 1
                    && view.selected_profile.as_deref() == Some("profile-a")
            }));
        }
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn game_wizard_subpages_keep_scrolling_content_above_the_action_rail(cx: &mut TestAppContext) {
    for (page, step) in [
        (None, WizardStep::Setup),
        (Some(WizardPage::Version), WizardStep::Setup),
        (Some(WizardPage::Loader), WizardStep::Setup),
        (Some(WizardPage::Location), WizardStep::Setup),
        (None, WizardStep::Review),
    ] {
        let (view, cx) = crate::test_support::window(cx, super::tests::test_view);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.preferences.reduce_motion = true;
                view.handle(UiAction::CreateInstance, window, cx);
                view.shell.wizard.page = page;
                view.shell.wizard.step = step;
                cx.notify();
            });
        });
        cx.simulate_resize(size(px(960.), px(600.)));
        let body = inside_viewport(cx, "wizard-body");
        let footer = inside_viewport(cx, "wizard-footer");
        assert!(body.bottom() <= footer.top());
        for selector in ["cancel-instance", "wizard-next"] {
            let action = inside_viewport(cx, selector);
            assert!(
                action.top() >= footer.top() && action.bottom() <= footer.bottom(),
                "{selector} must remain in the fixed action rail: {action:?}, {footer:?}"
            );
        }
        assert!(!view.read_with(cx, |view, _| view.busy));
    }
}

#[gpui::test]
fn settings_sections_keep_save_and_cancel_inside_the_default_window(cx: &mut TestAppContext) {
    for page in [0, 1, 3] {
        let (_view, cx) = crate::test_support::window(cx, |window, cx| {
            let mut view = super::tests::test_view(window, cx);
            view.router.navigate(Tab::Settings);
            view.preferences.reduce_motion = true;
            view.open_settings(cx);
            if let Some(draft) = &mut view.settings_form {
                draft.page = page;
            }
            view
        });
        cx.simulate_resize(size(px(960.), px(600.)));
        let cancel = inside_viewport(cx, "cancel-preferences");
        let save = inside_viewport(cx, "save-preferences");
        assert!(cancel.right() <= save.left());
        assert_eq!(cancel.top(), save.top());
    }
}

#[gpui::test]
fn crash_help_preserves_report_actions_in_the_default_window(cx: &mut TestAppContext) {
    let (_view, cx) = crate::test_support::window(cx, |window, cx| {
        super::CrashView::new(
            CrashReport::new(
                CrashContext {
                    run_id: "layout-test-session".into(),
                    profile_id: "profile-a".into(),
                    profile_name: "Test profile".into(),
                    minecraft_version: "1.20.6".into(),
                    loader: None,
                },
                ExitOutcome::start_failed(),
                "Synthetic start failure without player data",
                Vec::new(),
            ),
            std::env::temp_dir(),
            crate::Language::English,
            |_, _| {},
            window,
            cx,
        )
    });
    cx.simulate_resize(size(px(960.), px(600.)));
    for selector in [
        "crash-window-chrome",
        "crash-window-title",
        "crash-window-minimize",
        "crash-window-close",
        "crash-return",
        "crash-open-folder",
        "crash-export",
        "crash-copy",
        "crash-details",
        "crash-close",
    ] {
        inside_viewport(cx, selector);
    }
}
