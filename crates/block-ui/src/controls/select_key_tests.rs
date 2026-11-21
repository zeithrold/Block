use super::{
    Appearance, Block, Duration, Tab, UiAction, VisualTestContext, click, press, px, select,
    settle, settle_tab, size,
};

const APPEARANCE: &str = "settings-appearance";

fn settings(cx: &mut gpui::TestAppContext) -> (gpui::Entity<Block>, &mut VisualTestContext) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    cx.simulate_resize(size(px(1152.), px(720.)));
    view.update_in(cx, |view, window, cx| {
        view.preferences.reduce_motion = true;
        view.handle(UiAction::Tab(Tab::Settings), window, cx);
        view.handle(UiAction::SettingsPage(0), window, cx);
    });
    settle(cx);
    (view, cx)
}

#[gpui::test]
fn settings_select_anchors_to_trigger_and_keeps_inspection_separate_from_value(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = settings(cx);
    click(cx, APPEARANCE);
    let trigger = select(cx, APPEARANCE);
    assert_eq!(trigger.expanded(), Some(true));
    assert_eq!(trigger.value(), Some("Follow system"));
    let first = cx
        .debug_bounds("select-choice-0")
        .unwrap_or_else(|| unreachable!("first option is painted"));
    let last = cx
        .debug_bounds("select-choice-2")
        .unwrap_or_else(|| unreachable!("last option is painted"));
    let bounds = trigger.bounds();
    assert!(first.left() >= bounds.left() && first.left() - bounds.left() <= px(24.));
    assert!((first.top() - bounds.bottom()).abs() <= px(24.));
    assert!(first.top() >= px(0.) && last.bottom() <= px(720.));

    press(cx, "end");
    settle(cx);
    assert_eq!(select(cx, APPEARANCE).value(), Some("Follow system"));
    assert_eq!(
        view.read_with(cx, |view, _| view.preferences.appearance),
        Appearance::System
    );
    press(cx, "escape");
    settle(cx);
    assert_eq!(select(cx, APPEARANCE).expanded(), Some(false));
    assert_eq!(select(cx, APPEARANCE).focused(), Some(true));
    assert_eq!(
        view.read_with(cx, |view, _| view.preferences.appearance),
        Appearance::System
    );

    click(cx, APPEARANCE);
    press(cx, "end enter");
    settle(cx);
    assert_eq!(select(cx, APPEARANCE).value(), Some("Dark"));
    assert_eq!(select(cx, APPEARANCE).expanded(), Some(false));
    assert!(view.read_with(cx, |view, _| view.preferences.appearance
        == Appearance::Dark
        && view.saved_preferences.appearance == Appearance::System));

    click(cx, APPEARANCE);
    // Test the popup's right-side hit target, not the narrower label bounds.
    cx.simulate_click(
        gpui::point(bounds.right() - px(20.), first.center().y),
        gpui::Modifiers::none(),
    );
    settle(cx);
    assert_eq!(select(cx, APPEARANCE).expanded(), Some(false));
    assert_eq!(select(cx, APPEARANCE).value(), Some("Follow system"));
}

#[gpui::test]
fn settings_select_tab_closes_and_traverses_once_in_both_directions(cx: &mut gpui::TestAppContext) {
    let (view, cx) = settings(cx);
    click(cx, APPEARANCE);
    press(cx, "escape");
    settle(cx);
    assert_eq!(select(cx, APPEARANCE).focused(), Some(true));
    let trigger = cx
        .update(|window, cx| window.focused(cx))
        .unwrap_or_else(|| unreachable!("Escape restored the trigger"));
    for keys in ["tab", "shift-tab"] {
        cx.update(|window, cx| trigger.focus(window, cx));
        press(cx, keys);
        settle_tab(cx);
        let expected = cx.update(|window, cx| window.focused(cx));
        assert_ne!(expected, Some(trigger.clone()), "{keys} moves focus");
        click(cx, APPEARANCE);
        press(cx, "end");
        press(cx, keys);
        settle_tab(cx);
        assert_eq!(select(cx, APPEARANCE).expanded(), Some(false), "{keys}");
        assert_eq!(
            cx.update(|window, cx| window.focused(cx)),
            expected,
            "{keys} traverses once"
        );
        assert_eq!(
            view.read_with(cx, |view, _| view.preferences.appearance),
            Appearance::System
        );
    }
}

#[gpui::test]
fn styled_select_blur_closes_without_committing(cx: &mut gpui::TestAppContext) {
    let (view, cx) = settings(cx);
    cx.update(|window, _| window.activate_window());
    settle_tab(cx);
    assert!(cx.update(|window, _| window.is_window_active()));
    click(cx, "settings-language");
    press(cx, "escape");
    settle(cx);
    assert_eq!(select(cx, "settings-language").focused(), Some(true));
    let other = cx
        .update(|window, cx| window.focused(cx))
        .unwrap_or_else(|| unreachable!("language trigger"));
    click(cx, APPEARANCE);
    press(cx, "end");
    settle_tab(cx);
    assert_eq!(select(cx, APPEARANCE).expanded(), Some(true));
    assert_eq!(select(cx, APPEARANCE).focused(), Some(true));
    assert_ne!(
        cx.update(|window, cx| window.focused(cx)),
        Some(other.clone())
    );
    assert!(cx.update(|window, _| window.is_window_active()));
    cx.update(|window, cx| other.focus(window, cx));
    settle_tab(cx);
    assert_eq!(cx.update(|window, cx| window.focused(cx)), Some(other));
    assert_eq!(select(cx, "settings-language").focused(), Some(true));
    assert_eq!(select(cx, APPEARANCE).expanded(), Some(false));
    assert_eq!(select(cx, APPEARANCE).value(), Some("Follow system"));
    assert_eq!(
        view.read_with(cx, |view, _| view.preferences.appearance),
        Appearance::System
    );
}

#[gpui::test]
fn selector_typeahead_moves_real_focus_without_committing_until_activation(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = settings(cx);
    click(cx, APPEARANCE);
    press(cx, "d");
    settle(cx);
    assert_eq!(select(cx, APPEARANCE).value(), Some("Follow system"));
    assert_eq!(select(cx, APPEARANCE).focused(), Some(true));
    assert_eq!(
        view.read_with(cx, |view, _| view.preferences.appearance),
        Appearance::System
    );
    press(cx, "enter");
    settle(cx);
    assert_eq!(
        view.read_with(cx, |view, _| view.preferences.appearance),
        Appearance::Dark
    );

    click(cx, APPEARANCE);
    press(cx, "d");
    cx.executor().advance_clock(Duration::from_millis(701));
    press(cx, "l enter");
    settle(cx);
    assert_eq!(select(cx, APPEARANCE).value(), Some("Light"));
    assert!(view.read_with(cx, |view, _| view.preferences.appearance
        == Appearance::Light
        && view.saved_preferences.appearance == Appearance::System));
}
