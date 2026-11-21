use super::{
    Duration, Language, MenuKind, Surface, Tab, UiAction, click, press, px, settle, settle_tab,
};

#[gpui::test]
fn menu_entrance_forwards_the_first_typeahead_once_without_waiting_for_animation(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.router.navigate(Tab::Home);
        view
    });
    press(cx, "tab enter a");
    assert!(view.read_with(cx, |view, _| view.shell.surfaces.top().is_some_and(|layer| matches!(&layer.kind, Surface::Select(state) if state.choices[state.active].label == "Accounts"))), "{:?}", view.read_with(cx, |view, _| view.shell.surfaces.top().map(|layer| format!("{:?}", layer.kind))));
    press(cx, "enter");
    assert_eq!(
        view.read_with(cx, |view, _| view.router.tab()),
        Tab::Accounts
    );
    assert!(view.read_with(cx, |view, _| view.shell.surfaces.top().is_none()));
}

#[gpui::test]
fn app_menu_shares_roving_typeahead_current_page_and_real_navigation(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.preferences.reduce_motion = true;
            view.handle(UiAction::Tab(Tab::Home), window, cx);
        });
    });
    click(cx, "app-menu");
    assert!(view.read_with(cx, |view, _| view.shell.surfaces.top().is_some_and(|layer| matches!(&layer.kind, Surface::Select(state) if state.menu == Some(MenuKind::App) && state.active == 0 && state.width == px(192.)))));

    press(cx, "a");
    assert!(
        view.read_with(cx, |view, _| view.shell.surfaces.top().is_some_and(
            |layer| matches!(&layer.kind, Surface::Select(state) if state.active == 1)
        ))
    );
    press(cx, "a");
    assert!(
        view.read_with(cx, |view, _| view.shell.surfaces.top().is_some_and(
            |layer| matches!(&layer.kind, Surface::Select(state) if state.active == 4)
        ))
    );
    assert_eq!(view.read_with(cx, |view, _| view.router.tab()), Tab::Home);
    cx.executor().advance_clock(Duration::from_millis(701));
    press(cx, "s e t");
    assert!(
        view.read_with(cx, |view, _| view.shell.surfaces.top().is_some_and(
            |layer| matches!(&layer.kind, Surface::Select(state) if state.active == 3)
        ))
    );

    press(cx, "enter");
    assert!(
        view.read_with(cx, |view, _| view.router.tab() == Tab::Settings
            && view.settings_form.is_some()
            && !view.shell.surfaces.is_open())
    );
    click(cx, "settings-back");
    assert_eq!(view.read_with(cx, |view, _| view.router.tab()), Tab::Home);
    click(cx, "app-menu");
    click(cx, "menu-games");
    assert!(
        view.read_with(cx, |view, _| view.router.tab() == Tab::Instances
            && !view.shell.surfaces.is_open())
    );
}

#[gpui::test]
fn menu_labels_resolve_in_the_view_language(cx: &mut gpui::TestAppContext) {
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.router.navigate(Tab::Home);
        view
    });
    cx.update(|window, cx| {
        view.update(cx, |view, cx| view.open_app_menu_at(None, window, cx));
    });
    cx.run_until_parked();
    press(cx, "down");
    let focus = view.read_with(cx, |view, _| {
        view.shell
            .surfaces
            .top()
            .and_then(|layer| match &layer.kind {
                Surface::Select(state) => Some(state.focus.clone()),
                _ => None,
            })
    });
    for (language, title, first_choice) in [
        (Language::English, "Block menu", "Home"),
        (Language::SimplifiedChinese, "Block 菜单", "主页"),
        (Language::Japanese, "Block メニュー", "ホーム"),
    ] {
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.handle(UiAction::SetLanguage(language), window, cx);
            });
        });
        let resolved = view.read_with(cx, |view, _| {
            let layer = view.shell.surfaces.top()?;
            let Surface::Select(state) = &layer.kind else {
                return None;
            };
            assert_eq!(state.active, 1);
            assert_eq!(Some(state.focus.clone()), focus);
            assert!(state.choices[0].selected);
            Some((state.title(language), state.choices[0].label.clone()))
        });
        assert_eq!(
            resolved,
            Some((title.to_owned(), first_choice.to_owned())),
            "{}",
            language.code()
        );
    }
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
            matches!(view.shell.surfaces.top().map(|layer| &layer.kind), Some(Surface::GamesHelp))
                && view.shell.surfaces.layers().any(|layer| matches!(&layer.kind, Surface::Select(state) if state.menu == Some(MenuKind::App)))
        }));
    }
    press(cx, "escape");
    settle(cx);
    assert!(view.read_with(cx, |view, _| matches!(view.shell.surfaces.top().map(|layer| &layer.kind), Some(Surface::Select(state)) if state.menu == Some(MenuKind::App))));
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
    click(cx, "app-menu");
    // The compact app menu does not need to scroll. Seed its native handle to
    // exercise the snapshot's isolation independently of popup size.
    let captured = gpui::point(px(0.), px(-40.));
    let (saved, live) = view.read_with(cx, |view, _| {
        let Some(Surface::Select(state)) = view.shell.surfaces.top().map(|layer| &layer.kind)
        else {
            unreachable!("app menu opened");
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
