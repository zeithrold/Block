use super::{Language, Surface, Tab, UiAction, click, press, px, settle, settle_tab};
use gpui::Modifiers;
use gpui_base::test_support::{ElementSnapshot, snapshots};

fn menu_row(cx: &mut gpui::VisualTestContext, label: &str) -> ElementSnapshot {
    cx.update(|window, _| {
        snapshots(window).into_iter().find(|row| {
            row.visible() && row.role() == Some(gpui::Role::MenuItem) && row.label() == Some(label)
        })
    })
    .unwrap_or_else(|| unreachable!("missing menu item {label}"))
}

#[gpui::test]
fn first_arrow_opens_inspection_without_waiting_for_animation(cx: &mut gpui::TestAppContext) {
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.router.navigate(Tab::Home);
        view
    });
    press(cx, "tab enter down down enter");
    assert_eq!(
        view.read_with(cx, |view, _| view.router.tab()),
        Tab::Accounts
    );
    assert!(view.read_with(cx, |view, _| !view.shell.surfaces.is_open()));
}

#[gpui::test]
fn menu_checks_current_page_and_routes_keyboard_and_pointer_choices(cx: &mut gpui::TestAppContext) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    view.update_in(cx, |view, window, cx| {
        view.preferences.reduce_motion = true;
        view.handle(UiAction::Tab(Tab::Home), window, cx);
    });
    click(cx, "app-menu");
    assert_eq!(menu_row(cx, "Home").checked(), Some(true));
    press(cx, "up up");
    assert_eq!(menu_row(cx, "Settings").selected(), Some(true));
    assert_eq!(view.read_with(cx, |view, _| view.router.tab()), Tab::Home);
    press(cx, "enter");
    assert!(
        view.read_with(cx, |view, _| view.router.tab() == Tab::Settings
            && view.settings_form.is_some()
            && !view.shell.surfaces.is_open())
    );
    click(cx, "settings-back");
    assert_eq!(view.read_with(cx, |view, _| view.router.tab()), Tab::Home);
    click(cx, "app-menu");
    let games = menu_row(cx, "Games").bounds();
    cx.simulate_click(games.center(), Modifiers::none());
    settle(cx);
    assert!(
        view.read_with(cx, |view, _| view.router.tab() == Tab::Instances
            && !view.shell.surfaces.is_open())
    );
}

#[gpui::test]
fn open_menu_follows_language_and_keeps_focus_on_the_menu(cx: &mut gpui::TestAppContext) {
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.router.navigate(Tab::Home);
        view
    });
    press(cx, "tab");
    let trigger = cx
        .update(|window, cx| window.focused(cx))
        .unwrap_or_else(|| unreachable!("menu trigger must be focused after Tab"));
    press(cx, "enter");
    let focus = cx
        .update(|window, cx| window.focused(cx))
        .unwrap_or_else(|| unreachable!("menu must be focused after Enter"));
    assert_ne!(focus, trigger);
    for (language, title, first_choice) in [
        (Language::English, "Block menu", "Home"),
        (Language::SimplifiedChinese, "Block 菜单", "主页"),
        (Language::Japanese, "Block メニュー", "ホーム"),
    ] {
        view.update_in(cx, |view, window, cx| {
            view.handle(UiAction::SetLanguage(language), window, cx)
        });
        settle(cx);
        assert!(view.read_with(cx, |view, _| {
            view.shell
                .surfaces
                .top()
                .is_some_and(|layer| layer.kind.title(language) == title)
        }));
        assert_eq!(menu_row(cx, first_choice).checked(), Some(true));
        assert!(cx.update(|window, _| focus.is_focused(window)));
    }
    press(cx, "escape");
    settle(cx);
    assert!(cx.update(|window, _| trigger.is_focused(window)));
    click(cx, "app-menu");
    cx.simulate_click(gpui::point(px(300.), px(50.)), Modifiers::none());
    settle(cx);
    assert!(view.read_with(cx, |view, _| view.router.tab() == Tab::Home
        && !view.shell.surfaces.is_open()));
}

#[gpui::test]
fn tab_in_a_dialog_never_closes_the_shell_menu_beneath_it(cx: &mut gpui::TestAppContext) {
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.preferences.reduce_motion = true;
        view.router.navigate(Tab::Home);
        view
    });
    click(cx, "app-menu");
    view.update_in(cx, |view, window, cx| {
        view.handle(UiAction::GamesHelp, window, cx)
    });
    settle(cx);
    for keys in ["tab", "shift-tab"] {
        press(cx, keys);
        settle_tab(cx);
        assert!(view.read_with(cx, |view, _| {
            matches!(
                view.shell.surfaces.top().map(|layer| &layer.kind),
                Some(Surface::GamesHelp)
            ) && view
                .shell
                .surfaces
                .layers()
                .any(|layer| matches!(&layer.kind, Surface::AppMenu { .. }))
        }));
    }
    press(cx, "escape");
    settle(cx);
    assert!(view.read_with(cx, |view, _| matches!(
        view.shell.surfaces.top().map(|layer| &layer.kind),
        Some(Surface::AppMenu { .. })
    )));
    press(cx, "escape");
    settle(cx);
    assert!(view.read_with(cx, |view, _| !view.shell.surfaces.is_open()));
}

#[gpui::test]
fn saved_overlay_snapshot_keeps_the_selector_offset_it_captured(cx: &mut gpui::TestAppContext) {
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.router.navigate(Tab::Home);
        view.preferences.reduce_motion = true;
        view
    });
    view.update_in(cx, |view, window, cx| {
        view.open_appearance_select(true, true, window, cx)
    });
    let captured = gpui::point(px(0.), px(-40.));
    let (saved, live) = view.read_with(cx, |view, _| {
        let Some(Surface::Select(state)) = view.shell.surfaces.top().map(|layer| &layer.kind)
        else {
            unreachable!("appearance selector opened");
        };
        state.scroll.set_offset(captured);
        (view.shell.surfaces.snapshot(), state.scroll.clone())
    });
    assert_eq!(live.offset(), captured);
    live.set_offset(gpui::point(px(0.), px(0.)));
    assert!(
        matches!(saved.top().map(|layer| &layer.kind), Some(Surface::Select(state)) if state.scroll.offset() == captured)
    );
}
