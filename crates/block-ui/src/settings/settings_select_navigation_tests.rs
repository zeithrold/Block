//! Settings navigation while a Kit selector owns focus.
use super::{settings_click, settings_frame};
use crate::{Tab, UiAction};
use gpui::{Focusable as _, px, size};
use gpui_component::WindowExt as _;

fn appearance_expanded(cx: &mut gpui::VisualTestContext) -> Option<bool> {
    cx.update(|window, _| {
        gpui_base::test_support::find(window, &[], &"settings-appearance-select".into())?.expanded()
    })
}

#[gpui::test]
fn native_close_dismisses_an_open_selector_before_routing_save_errors(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.router.navigate(Tab::Home);
        view.preferences.reduce_motion = true;
        view.saved_preferences.reduce_motion = true;
        view
    });
    cx.simulate_resize(size(px(360.), px(720.)));
    view.update_in(cx, |view, window, cx| {
        view.handle(UiAction::Tab(Tab::Settings), window, cx);
        view.handle(UiAction::SettingsPage(0), window, cx);
        if let Some(draft) = &view.settings_form {
            draft
                .memory
                .update(cx, |input, cx| input.set_value("invalid", cx));
        }
    });
    settings_click(cx, "settings-appearance");
    assert_eq!(appearance_expanded(cx), Some(true));
    view.update_in(cx, |view, window, cx| {
        assert!(!view.allow_native_window_close(window, cx));
        assert_eq!(view.shell.surfaces.layers().count(), 1);
    });
    settings_frame(cx);
    assert_eq!(appearance_expanded(cx), Some(false));
    assert!(cx.update(|window, cx| window.has_active_dialog(cx)));
    settings_click(cx, "settings-exit-save");
    cx.update(|window, cx| {
        assert!(!window.has_active_dialog(cx));
        let view = view.read(cx);
        let Some(draft) = &view.settings_form else {
            unreachable!("invalid draft retained");
        };
        let memory = draft.memory.read(cx);
        assert!(!view.shell.surfaces.is_open());
        assert_eq!(view.shell.settings_page, Some(1));
        assert_eq!(memory.value(cx), "invalid");
        assert!(memory.focus_handle(cx).is_focused(window));
    });
}

#[gpui::test]
fn alt_left_dismisses_select_before_returning_to_the_index(cx: &mut gpui::TestAppContext) {
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.router.navigate(Tab::Home);
        view.preferences.reduce_motion = true;
        view.saved_preferences.reduce_motion = true;
        view
    });
    cx.simulate_resize(size(px(360.), px(720.)));
    view.update_in(cx, |view, window, cx| {
        view.handle(UiAction::Tab(Tab::Settings), window, cx);
    });
    settings_click(cx, "settings-page-general");
    settings_click(cx, "settings-appearance");
    assert_eq!(appearance_expanded(cx), Some(true));
    crate::test_support::press(cx, "alt-left");
    settings_frame(cx);
    assert_eq!(
        view.read_with(cx, |view, _| view.shell.settings_page),
        Some(0)
    );
    assert_eq!(appearance_expanded(cx), Some(false));
    crate::test_support::press(cx, "alt-left");
    settings_frame(cx);
    assert!(
        view.read_with(cx, |view, _| view.shell.settings_page.is_none()
            && view.router.tab() == Tab::Settings)
    );
}
