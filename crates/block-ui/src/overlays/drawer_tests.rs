//! Pointer, layout and focus regressions against the production drawer host.
use std::time::Duration;

use crate::{
    Block, Form, Tab, UiAction,
    first_run::Stage as FirstRunStage,
    game_wizard::{CreationKind, WizardStep},
    import_games::Page,
    shell::{Drawer, Surface},
};
use block_launcher::game_import::{DiscoveryReport, GameCandidate, ImportSourceReport, SourceKind};
use gpui::{Entity, Modifiers, MouseButton, TestAppContext, VisualTestContext, point, px, size};

fn fixture(
    cx: &mut TestAppContext,
    reduced: bool,
) -> (
    Entity<Block>,
    &mut VisualTestContext,
    crate::platform::SystemMotionOverride,
) {
    let system_motion = crate::platform::test_system_motion(false);
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.snapshot = crate::test_support::ready_snapshot();
        view.selected_profile = Some("profile-a".into());
        view.selected_instance = Some("instance-a".into());
        view.preferences.reduce_motion = reduced;
        view.saved_preferences.reduce_motion = reduced;
        view
    });
    cx.simulate_resize(size(px(432.), px(768.)));
    cx.run_until_parked();
    (view, cx, system_motion)
}

fn act(view: &Entity<Block>, action: UiAction, cx: &mut VisualTestContext) {
    cx.update(|window, cx| view.update(cx, |view, cx| view.handle(action, window, cx)));
    cx.run_until_parked();
}

fn frame(cx: &mut VisualTestContext, millis: u64) {
    cx.executor().advance_clock(Duration::from_millis(millis));
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

#[expect(
    clippy::expect_used,
    reason = "Missing painted geometry is a test failure."
)]
fn bounds(cx: &mut VisualTestContext, selector: &'static str) -> gpui::Bounds<gpui::Pixels> {
    let message = format!("drawer control painted: {selector}");
    crate::test_support::control_bounds(cx, selector).expect(&message)
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let position = bounds(cx, selector).center();
    cx.simulate_click(position, Modifiers::none());
    cx.run_until_parked();
}

#[expect(
    clippy::expect_used,
    reason = "The named header control must be painted."
)]
fn control_node(
    cx: &mut VisualTestContext,
    id: &'static str,
) -> gpui_base::test_support::ElementSnapshot {
    cx.update(|window, _| gpui_base::test_support::find(window, &[], &id.into()))
        .expect("header control has a native accessibility node")
}

fn assert_import_selection_summary(cx: &mut VisualTestContext, label: &str) {
    let title = bounds(cx, "drawer-title-text");
    let count = bounds(cx, "import-selection-count");
    assert!(
        title.bottom() < count.top() && count.right() <= bounds(cx, "import-select-all").left(),
        "selection summary stays in the body beside its bulk action"
    );
    assert_eq!(
        control_node(cx, "import-selection-count").label(),
        Some(label)
    );
}

fn focus_by_tab(cx: &mut VisualTestContext, id: &'static str) {
    let mut reached = false;
    for _ in 0..20 {
        if control_node(cx, id).focused() == Some(true) {
            reached = true;
            break;
        }
        crate::test_support::press(cx, "tab");
        cx.run_until_parked();
    }
    assert!(reached, "{id} was not reachable by Tab");
}

fn control_quads(cx: &mut VisualTestContext, selector: &'static str) -> Vec<gpui::Quad> {
    let bounds = bounds(cx, selector);
    cx.update(|window, _| {
        let scale = window.scale_factor();
        window
            .painted_quads()
            .into_iter()
            .filter(|quad| {
                (quad.bounds.origin.x.0 - f32::from(bounds.origin.x) * scale).abs() < 0.5
                    && (quad.bounds.origin.y.0 - f32::from(bounds.origin.y) * scale).abs() < 0.5
                    && (quad.bounds.size.width.0 - f32::from(bounds.size.width) * scale).abs() < 0.5
                    && (quad.bounds.size.height.0 - f32::from(bounds.size.height) * scale).abs()
                        < 0.5
            })
            .collect()
    })
}

fn matches_rgb(actual: gpui::Rgba, expected: u32) -> bool {
    let expected = gpui::rgb(expected);
    (actual.r - expected.r).abs() < 0.001
        && (actual.g - expected.g).abs() < 0.001
        && (actual.b - expected.b).abs() < 0.001
}

fn assert_control_color(cx: &mut VisualTestContext, selector: &'static str, expected: u32) {
    let painted = control_quads(cx, selector).into_iter().any(|quad| {
        quad.background
            .as_solid()
            .is_some_and(|actual| matches_rgb(actual.to_rgb(), expected))
    });
    assert!(painted, "{selector} did not paint the expected color");
}

fn assert_selection_surface(
    view: &Entity<Block>,
    cx: &mut VisualTestContext,
    selector: &'static str,
    selected: bool,
) {
    let palette = view.read_with(cx, |view, _| view.palette);
    assert_control_color(
        cx,
        selector,
        if selected {
            palette.accent
        } else {
            palette.popover
        },
    );
    let scale = cx.update(|window, _| window.scale_factor());
    assert!(
        !control_quads(cx, selector)
            .iter()
            .any(|quad| { quad.border_color.a > 0.99 && quad.border_widths.top.0 > scale }),
        "selection must use a surface and indicator without a thick perimeter"
    );
}

fn control_has_ghost_border(
    cx: &mut VisualTestContext,
    selector: &'static str,
    surface: u32,
) -> bool {
    control_quads(cx, selector).into_iter().any(|quad| {
        quad.border_color.a > 0.999
            && matches_rgb(quad.border_color.to_rgb(), surface)
            && [
                quad.border_widths.top,
                quad.border_widths.right,
                quad.border_widths.bottom,
                quad.border_widths.left,
            ]
            .into_iter()
            .all(|width| width.0 > 0.)
    })
}

fn import_candidate(id: &str) -> GameCandidate {
    GameCandidate {
        id: id.into(),
        source: SourceKind::OfficialLauncher,
        source_root: "/synthetic/launcher".into(),
        name: id.into(),
        game_directory: format!("/synthetic/{id}").into(),
        configured_version: Some("1.20.6".into()),
        minecraft_version: Some("1.20.6".into()),
        loader: None,
        latest_release: None,
        latest_snapshot: None,
        importable: true,
        unsupported_reason: None,
        metadata_warning: None,
    }
}

#[gpui::test]
fn headers_keep_their_text_aligned_without_a_back_action(cx: &mut TestAppContext) {
    let (view, cx, _system_motion) = fixture(cx, true);
    for width in [360., 432.] {
        cx.simulate_resize(size(px(width), px(768.)));
        for drawer in [
            Drawer::Accounts,
            Drawer::Profiles,
            Drawer::Downloads,
            Drawer::ImportGames,
        ] {
            act(&view, UiAction::Drawer(Some(drawer)), cx);
            let panel = bounds(cx, "block-drawer");
            let title = bounds(cx, "drawer-title");
            let description = bounds(cx, "drawer-description");
            let close = bounds(cx, "drawer-close");
            assert_eq!(
                title.left(),
                panel.left() + px(20.),
                "{drawer:?} title should start at the panel inset"
            );
            assert_eq!(
                description.left(),
                panel.left() + px(20.),
                "{drawer:?} description should start at the panel inset"
            );
            assert_eq!(close.right(), panel.right() - px(20.));
            assert!(title.right() <= close.left());
            assert!(description.top() >= close.bottom());
            assert!(crate::test_support::control_bounds(cx, "import-header-back").is_none());
            let node = control_node(cx, "drawer-close");
            assert_eq!(node.role(), Some(gpui::Role::Button));
            assert_eq!(node.label(), Some("Close"));
            act(&view, UiAction::Drawer(None), cx);
        }
    }
}

fn assert_source_card_selection(view: &Entity<Block>, cx: &mut VisualTestContext) {
    assert!(crate::test_support::control_bounds(cx, "import-source-AdjacentMinecraft").is_none());
    let source = bounds(cx, "import-source-OfficialLauncher");
    if let Some(inactive) = cx.debug_bounds("import-list-content") {
        assert!(
            inactive.left() >= bounds(cx, "import-sources-content").right(),
            "inactive page decorations must not overlap the visible viewport"
        );
    }
    let footer = bounds(cx, "import-new");
    assert_eq!(source.left(), footer.left());
    assert_eq!(source.right(), footer.right());
    assert_eq!(
        bounds(cx, "import-sources-content").right(),
        bounds(cx, "block-drawer").right() - px(4.),
        "the track belongs in the existing outer right margin"
    );
    assert_eq!(
        control_node(cx, "import-source-OfficialLauncher").role(),
        Some(gpui::Role::RadioButton)
    );
    assert_selection_surface(view, cx, "import-source-OfficialLauncher", false);
    let view_button = bounds(cx, "import-view-OfficialLauncher");
    let title = bounds(cx, "import-source-title-OfficialLauncher");
    assert_eq!(
        title.left() - source.left(),
        px(16.),
        "source title starts at the content inset without a folder icon slot"
    );
    assert!(
        title.right() <= view_button.left()
            && f32::from(title.center().y - view_button.center().y).abs() <= 0.5,
        "the independent View button belongs beside the source card title"
    );
    cx.simulate_click(
        point(source.left() + px(24.), source.bottom() - px(24.)),
        Modifiers::none(),
    );
    cx.run_until_parked();
    assert_eq!(bounds(cx, "import-source-OfficialLauncher"), source);
    assert_eq!(
        control_node(cx, "import-source-OfficialLauncher").checked(),
        Some(true)
    );
    assert_selection_surface(view, cx, "import-source-OfficialLauncher", true);
    crate::test_support::press(cx, "tab");
    focus_by_tab(cx, "import-source-OfficialLauncher");
    assert_selection_surface(view, cx, "import-source-OfficialLauncher", true);
    let max_base_border = cx.update(|window, _| 1.5 * window.scale_factor());
    assert!(
        !control_quads(cx, "import-source-OfficialLauncher")
            .iter()
            .any(|quad| {
                quad.border_color.a > 0.99 && quad.border_widths.top.0 > max_base_border
            }),
        "keyboard focus must not add a thick inner border underneath the outer selection"
    );
    assert_eq!(
        view.read_with(cx, |view, _| view.shell.import_games.page),
        Page::Sources,
        "the path area selects the source without navigating"
    );
    assert_eq!(
        control_node(cx, "import-view-OfficialLauncher").label(),
        Some("1 game")
    );
    assert_eq!(view_button.size.height, px(24.));
    assert!(view_button.size.width < source.size.width - px(80.));
    assert_eq!(
        view.read_with(cx, |view, _| view
            .shell
            .import_games
            .source_scroll
            .max_offset()
            .x),
        px(0.),
        "using the outer gutter must not introduce horizontal scrolling"
    );
}

#[gpui::test]
fn import_header_uses_ghost_controls_and_back_retains_selection(cx: &mut TestAppContext) {
    let (view, cx, _system_motion) = fixture(cx, true);
    view.update(cx, |view, cx| {
        view.shell.import_games.report = Some(DiscoveryReport {
            sources: vec![ImportSourceReport {
                kind: SourceKind::OfficialLauncher,
                root: "/synthetic/launcher".into(),
                candidates_found: 2,
                warning: None,
            }],
            candidates: vec![import_candidate("one"), import_candidate("two")],
        });
        view.shell.import_games.selected.insert("one".into());
        cx.notify();
    });
    act(&view, UiAction::Drawer(Some(Drawer::ImportGames)), cx);
    let mut wide_pair = None;
    for width in [360., 432., 504.] {
        cx.simulate_resize(size(px(width), px(768.)));
        view.update(cx, |view, cx| {
            view.shell.import_games.source = SourceKind::AdjacentMinecraft;
            cx.notify();
        });
        cx.run_until_parked();
        assert_source_card_selection(&view, cx);
        if width >= 432. {
            let pair = (
                bounds(cx, "import-source-title-OfficialLauncher").size,
                bounds(cx, "import-view-OfficialLauncher").origin,
            );
            if let Some(previous) = wide_pair {
                assert_eq!(
                    pair, previous,
                    "extra width stays after the title/button pair, not between them"
                );
            }
            wide_pair = Some(pair);
        }
    }
    click(cx, "import-view-OfficialLauncher");
    assert_eq!(
        view.read_with(cx, |view, _| view.shell.import_games.page),
        Page::List
    );

    let panel = bounds(cx, "block-drawer");
    let back = bounds(cx, "import-header-back");
    let title = bounds(cx, "drawer-title");
    let description = bounds(cx, "drawer-description");
    let close = bounds(cx, "drawer-close");
    assert_eq!(back.left(), panel.left() + px(20.));
    assert_eq!(back.size, size(px(32.), px(32.)));
    assert!(title.left() >= back.right() + px(8.));
    assert_eq!(description.left(), panel.left() + px(20.));
    assert_eq!(back.center().y, close.center().y);
    assert!(description.top() >= back.bottom());
    let node = control_node(cx, "import-header-back");
    assert_eq!(node.role(), Some(gpui::Role::Button));
    assert_eq!(node.label(), Some("Back to sources"));

    let mut border_failures = Vec::new();
    for (appearance, palette) in [
        (crate::Appearance::Light, crate::Palette::LIGHT),
        (crate::Appearance::Dark, crate::Palette::DARK),
    ] {
        view.update(cx, |view, cx| {
            view.preferences.appearance = appearance;
            cx.notify();
        });
        cx.run_until_parked();
        cx.simulate_mouse_move(point(px(0.), px(0.)), None, Modifiers::none());
        cx.run_until_parked();
        assert_control_color(cx, "import-copy", palette.primary);
        for selector in ["drawer-close", "import-header-back"] {
            cx.simulate_mouse_move(point(px(0.), px(0.)), None, Modifiers::none());
            cx.run_until_parked();
            assert_control_color(cx, selector, palette.background);
            if !control_has_ghost_border(cx, selector, palette.background) {
                border_failures.push(format!("{appearance:?} {selector}"));
            }
            let position = bounds(cx, selector).center();
            cx.simulate_mouse_move(position, None, Modifiers::none());
            cx.run_until_parked();
            assert_control_color(cx, selector, palette.accent);
        }
    }
    assert!(
        border_failures.is_empty(),
        "idle Ghost borders should match the surface: {border_failures:?}"
    );

    click(cx, "import-header-back");
    view.read_with(cx, |view, _| {
        assert_eq!(view.shell.import_games.page, Page::Sources);
        assert!(view.shell.import_games.selected.contains("one"));
        assert!(matches!(
            view.shell.surfaces.top().map(|entry| &entry.kind),
            Some(Surface::Drawer(Drawer::ImportGames))
        ));
    });
    assert!(crate::test_support::control_bounds(cx, "import-header-back").is_none());
    assert_eq!(
        bounds(cx, "drawer-title").left(),
        bounds(cx, "block-drawer").left() + px(20.)
    );

    view.update(cx, |view, cx| {
        let state = &mut view.shell.import_games;
        state.page = Page::Results;
        state.submitted.push("one".into());
        state.outcomes.insert(
            "one".into(),
            crate::import_games::Outcome::Failed("Fixture failure".into()),
        );
        cx.notify();
    });
    cx.run_until_parked();
    assert!(crate::test_support::control_bounds(cx, "import-header-back").is_none());
    assert_eq!(
        bounds(cx, "drawer-description").left(),
        bounds(cx, "block-drawer").left() + px(20.)
    );
    cx.simulate_mouse_move(point(px(0.), px(0.)), None, Modifiers::none());
    cx.run_until_parked();
    assert_control_color(cx, "import-retry", crate::Palette::DARK.background);
    assert_control_color(cx, "import-done", crate::Palette::DARK.primary);
}

#[gpui::test]
fn import_list_keeps_bulk_selection_and_uses_only_header_back(cx: &mut TestAppContext) {
    let (view, cx, _system_motion) = fixture(cx, true);
    view.update(cx, |view, cx| {
        view.shell.import_games.report = Some(DiscoveryReport {
            sources: Vec::new(),
            candidates: vec![import_candidate("one"), import_candidate("two")],
        });
        view.shell.import_games.selected.insert("one".into());
        cx.notify();
    });
    act(&view, UiAction::Drawer(Some(Drawer::ImportGames)), cx);
    click(cx, "import-view-OfficialLauncher");

    assert_import_selection_summary(cx, "1 / 2 selected");
    assert!(
        crate::test_support::control_bounds(cx, "import-sources").is_none(),
        "the header owns the only return action"
    );
    let panel = bounds(cx, "block-drawer");
    // The compact viewport may show only part of the last row initially.
    // Reach the real scroll limit before requiring its full outer perimeter.
    view.update(cx, |view, cx| {
        let scroll = &view.shell.import_games.list_scroll;
        scroll.set_offset(point(px(0.), -scroll.max_offset().y));
        cx.notify();
    });
    cx.run_until_parked();
    let card = bounds(cx, "import-game-two");
    assert_eq!(
        bounds(cx, "import-game-one").top(),
        bounds(cx, "import-select-all").bottom() + px(12.),
        "one selection row must directly precede the cards without a separate count row"
    );
    assert_eq!(
        card.top() - bounds(cx, "import-game-one").bottom(),
        px(4.),
        "candidate choices retain a small vertical gap"
    );
    assert_eq!(
        control_node(cx, "import-game-two").role(),
        Some(gpui::Role::CheckBox)
    );
    for position in [
        point(card.left() + px(12.), card.top() + px(52.)),
        point(card.right() - px(20.), card.center().y),
    ] {
        cx.simulate_click(position, Modifiers::none());
        cx.run_until_parked();
        assert_eq!(
            bounds(cx, "import-game-two"),
            card,
            "selection cannot move its hitbox"
        );
        assert_eq!(
            view.read_with(cx, |view, _| view.shell.import_games.selected.clone()),
            ["one".to_owned(), "two".to_owned()].into_iter().collect(),
            "clicking the metadata or trailing indicator must toggle the complete card once"
        );
        assert_eq!(control_node(cx, "import-game-two").checked(), Some(true));
        assert_import_selection_summary(cx, "2 / 2 selected");
        assert_selection_surface(&view, cx, "import-game-two", true);
        crate::test_support::press(cx, "space");
        cx.run_until_parked();
        assert_eq!(control_node(cx, "import-game-two").checked(), Some(false));
        assert_import_selection_summary(cx, "1 / 2 selected");
        assert_eq!(
            bounds(cx, "import-game-two"),
            card,
            "deselection cannot move its hitbox"
        );
        assert_selection_surface(&view, cx, "import-game-two", false);
        assert_eq!(
            view.read_with(cx, |view, _| view.shell.import_games.selected.clone()),
            ["one".to_owned()].into_iter().collect(),
            "keyboard activation toggles this card without clearing its neighbor"
        );
    }
    view.update(cx, |view, cx| {
        view.shell
            .import_games
            .list_scroll
            .set_offset(point(px(0.), px(0.)));
        cx.notify();
    });
    cx.run_until_parked();
    assert_eq!(
        bounds(cx, "import-select-all").right(),
        panel.right() - px(20.),
        "bulk selection stays aligned with the fixed footer"
    );
    assert_eq!(card.left(), panel.left() + px(20.));
    assert_eq!(card.right(), panel.right() - px(20.));
    assert_eq!(
        control_node(cx, "import-select-all").label(),
        Some("Select all")
    );
    click(cx, "import-select-all");
    view.read_with(cx, |view, _| {
        assert_eq!(view.shell.import_games.selected.len(), 2);
        assert!(view.shell.import_games.selected.contains("one"));
        assert!(view.shell.import_games.selected.contains("two"));
    });
    assert_eq!(control_node(cx, "import-select-all").label(), Some("Clear"));
    click(cx, "import-header-back");
    assert_eq!(
        view.read_with(cx, |view, _| view.shell.import_games.page),
        Page::Sources
    );
    assert_eq!(
        view.read_with(cx, |view, _| view.shell.import_games.selected.len()),
        2
    );

    click(cx, "import-view-OfficialLauncher");
    assert_eq!(control_node(cx, "import-select-all").label(), Some("Clear"));
    click(cx, "import-select-all");
    assert!(view.read_with(cx, |view, _| view.shell.import_games.selected.is_empty()));
    assert_eq!(
        control_node(cx, "import-select-all").label(),
        Some("Select all")
    );
    click(cx, "import-header-back");
    assert_eq!(
        view.read_with(cx, |view, _| view.shell.import_games.page),
        Page::Sources
    );
    assert!(view.read_with(cx, |view, _| view.shell.import_games.selected.is_empty()));
}

#[gpui::test]
fn import_source_button_counts_its_selected_games_and_keeps_list_access(cx: &mut TestAppContext) {
    for (selected, label) in [(0, "0 games"), (1, "1 game"), (2, "2 games")] {
        let (view, cx, _system_motion) = fixture(cx, true);
        view.update(cx, |view, cx| {
            let mut adjacent = import_candidate("adjacent");
            adjacent.source = SourceKind::AdjacentMinecraft;
            view.shell.import_games.report = Some(DiscoveryReport {
                sources: Vec::new(),
                candidates: vec![
                    import_candidate("one"),
                    import_candidate("two"),
                    import_candidate("three"),
                    adjacent,
                ],
            });
            view.shell.import_games.selected = ["one", "two", "three"]
                .into_iter()
                .take(selected)
                .chain(std::iter::once("adjacent"))
                .map(str::to_owned)
                .collect();
            cx.notify();
        });
        act(&view, UiAction::Drawer(Some(Drawer::ImportGames)), cx);
        assert_eq!(
            control_node(cx, "import-view-OfficialLauncher").label(),
            Some(label)
        );
        click(cx, "import-view-OfficialLauncher");
        assert_eq!(
            view.read_with(cx, |view, _| view.shell.import_games.page),
            Page::List,
            "partial and empty selections must retain access to all candidates"
        );
        assert_import_selection_summary(cx, &format!("{selected} / 3 selected"));
        click(cx, "import-header-back");
        assert_eq!(
            control_node(cx, "import-view-OfficialLauncher").label(),
            Some(label)
        );
    }
}

#[gpui::test]
fn import_source_count_button_stays_distinct_from_the_selected_card(cx: &mut TestAppContext) {
    let (view, cx, _system_motion) = fixture(cx, true);
    view.update(cx, |view, cx| {
        view.shell.import_games.report = Some(DiscoveryReport {
            sources: Vec::new(),
            candidates: vec![import_candidate("one"), import_candidate("two")],
        });
        view.shell.import_games.selected = ["one".into(), "two".into()].into_iter().collect();
        view.shell.import_games.source = SourceKind::OfficialLauncher;
        cx.notify();
    });
    act(&view, UiAction::Drawer(Some(Drawer::ImportGames)), cx);
    for (appearance, palette) in [
        (crate::Appearance::Light, crate::Palette::LIGHT),
        (crate::Appearance::Dark, crate::Palette::DARK),
    ] {
        view.update(cx, |view, cx| {
            view.preferences.appearance = appearance;
            cx.notify();
        });
        cx.simulate_mouse_move(point(px(0.), px(0.)), None, Modifiers::none());
        cx.run_until_parked();
        assert_selection_surface(&view, cx, "import-source-OfficialLauncher", true);
        assert_control_color(cx, "import-view-OfficialLauncher", palette.info_background);
        let position = bounds(cx, "import-view-OfficialLauncher").center();
        cx.simulate_mouse_move(position, None, Modifiers::none());
        cx.run_until_parked();
        assert!(
            control_quads(cx, "import-view-OfficialLauncher")
                .iter()
                .any(|quad| {
                    quad.background.as_solid().is_some_and(|color| {
                        !matches_rgb(color.to_rgb(), palette.accent)
                            && !matches_rgb(color.to_rgb(), palette.info_background)
                    })
                }),
            "hover feedback must stay distinct from the selected card and idle button"
        );
    }
}

#[gpui::test]
fn import_cards_keep_unsupported_and_saved_games_disabled(cx: &mut TestAppContext) {
    let (view, cx, _system_motion) = fixture(cx, true);
    view.update(cx, |view, cx| {
        let mut unsupported = import_candidate("unsupported");
        unsupported.importable = false;
        unsupported.unsupported_reason = Some("Unsupported fixture".into());
        view.shell.import_games.report = Some(DiscoveryReport {
            sources: Vec::new(),
            candidates: vec![unsupported, import_candidate("saved")],
        });
        view.shell
            .import_games
            .outcomes
            .insert("saved".into(), crate::import_games::Outcome::Saved);
        cx.notify();
    });
    act(&view, UiAction::Drawer(Some(Drawer::ImportGames)), cx);
    click(cx, "import-view-OfficialLauncher");
    for (id, checked) in [
        ("import-game-unsupported", false),
        ("import-game-saved", true),
    ] {
        let node = control_node(cx, id);
        assert_eq!(node.role(), Some(gpui::Role::CheckBox));
        assert_eq!(node.disabled(), Some(true));
        assert_eq!(node.checked(), Some(checked));
        click(cx, id);
        assert!(view.read_with(cx, |view, _| view.shell.import_games.selected.is_empty()));
    }
}

#[gpui::test]
fn import_header_keyboard_back_and_close_preserve_dismissal_policy(cx: &mut TestAppContext) {
    #[derive(Debug)]
    enum Dismissal {
        Pointer,
        Keyboard,
        Escape,
        Backdrop,
    }

    let (view, cx, _system_motion) = fixture(cx, true);
    let invoking_focus = cx.update(|window, cx| window.focused(cx));
    act(&view, UiAction::Drawer(Some(Drawer::ImportGames)), cx);
    act(
        &view,
        UiAction::ImportGames(crate::import_games::Action::View(
            SourceKind::OfficialLauncher,
        )),
        cx,
    );

    focus_by_tab(cx, "import-header-back");
    crate::test_support::press(cx, "enter");
    cx.run_until_parked();
    assert_eq!(
        view.read_with(cx, |view, _| view.shell.import_games.page),
        Page::Sources
    );

    for dismissal in [
        Dismissal::Pointer,
        Dismissal::Keyboard,
        Dismissal::Escape,
        Dismissal::Backdrop,
    ] {
        if !view.read_with(cx, |view, _| view.shell.surfaces.is_open()) {
            act(&view, UiAction::Drawer(Some(Drawer::ImportGames)), cx);
        }
        let cancel = view.update(cx, |view, cx| {
            view.shell.import_games.page = Page::Progress;
            view.shell.import_games.running = true;
            view.shell.import_games.cancel = block_launcher::CancellationToken::new();
            cx.notify();
            view.shell.import_games.cancel.clone()
        });
        cx.run_until_parked();
        assert!(crate::test_support::control_bounds(cx, "import-header-back").is_none());
        assert!(crate::test_support::control_bounds(cx, "import-stop").is_some());
        match dismissal {
            Dismissal::Pointer => click(cx, "drawer-close"),
            Dismissal::Keyboard => {
                focus_by_tab(cx, "drawer-close");
                crate::test_support::press(cx, "space");
            }
            Dismissal::Escape => crate::test_support::press(cx, "escape"),
            Dismissal::Backdrop => {
                let panel = bounds(cx, "block-drawer");
                cx.simulate_click(
                    point(panel.center().x, panel.top() - px(10.)),
                    Modifiers::none(),
                );
            }
        }
        cx.run_until_parked();
        assert!(
            !view.read_with(cx, |view, _| view.shell.surfaces.is_open()),
            "{dismissal:?} should dismiss the drawer"
        );
        assert!(
            cancel.is_cancelled(),
            "{dismissal:?} should cancel the running import"
        );
        assert_eq!(cx.update(|window, cx| window.focused(cx)), invoking_focus);
    }
}

#[gpui::test]
fn handle_drag_tracks_pointer_rebounds_then_dismisses_and_restores_focus(cx: &mut TestAppContext) {
    let (view, cx, _system_motion) = fixture(cx, true);
    let invoking_focus = cx.update(|window, cx| window.focused(cx));
    act(&view, UiAction::Drawer(Some(Drawer::Profiles)), cx);
    let start_panel = bounds(cx, "block-drawer");
    let handle = bounds(cx, "drawer-handle").center();
    cx.simulate_mouse_down(handle, MouseButton::Left, Modifiers::none());
    cx.update(|window, app| {
        let layer = gpui_base::Root::read(window, app).plugin::<super::DrawerLayer>();
        assert!(
            layer.is_some_and(|layer| layer
                .read(app)
                .active
                .as_ref()
                .is_some_and(|active| active.drag.is_some())),
            "handle press starts the drag"
        );
    });
    frame(cx, 100);
    let moved = handle + point(px(0.), px(40.));
    cx.simulate_mouse_move(moved, MouseButton::Left, Modifiers::none());
    cx.run_until_parked();
    assert_eq!(
        bounds(cx, "block-drawer").origin.y,
        start_panel.origin.y + px(40.)
    );
    frame(cx, 150); // A pause removes fling velocity, so a short pull rebounds.
    cx.simulate_mouse_up(moved, MouseButton::Left, Modifiers::none());
    cx.run_until_parked();
    assert_eq!(bounds(cx, "block-drawer").origin.y, start_panel.origin.y);
    assert!(view.read_with(cx, |view, _| view.shell.surfaces.is_open()));

    let handle = bounds(cx, "drawer-handle").center();
    let end = handle + point(px(0.), start_panel.size.height * 0.35);
    cx.simulate_mouse_down(handle, MouseButton::Left, Modifiers::none());
    frame(cx, 100);
    cx.simulate_mouse_move(end, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_up(end, MouseButton::Left, Modifiers::none());
    cx.run_until_parked();
    assert!(!view.read_with(cx, |view, _| view.shell.surfaces.is_open()));
    assert!(crate::test_support::control_bounds(cx, "block-drawer").is_none());
    assert_eq!(cx.update(|window, cx| window.focused(cx)), invoking_focus);
}

#[gpui::test]
fn body_pointer_motion_does_not_start_a_drawer_gesture(cx: &mut TestAppContext) {
    let (view, cx, _system_motion) = fixture(cx, true);
    act(&view, UiAction::Drawer(Some(Drawer::Profiles)), cx);
    let panel = bounds(cx, "block-drawer");
    let list = bounds(cx, "selector-results");
    let start = list.origin + point(px(8.), px(8.));
    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(
        start + point(px(0.), px(100.)),
        MouseButton::Left,
        Modifiers::none(),
    );
    cx.simulate_mouse_up(
        start + point(px(0.), px(100.)),
        MouseButton::Left,
        Modifiers::none(),
    );
    cx.run_until_parked();
    assert_eq!(bounds(cx, "block-drawer").origin.y, panel.origin.y);
    assert!(view.read_with(cx, |view, _| view.shell.surfaces.is_open()));
}

#[gpui::test]
fn fixed_panel_keeps_management_action_visible(cx: &mut TestAppContext) {
    let (view, cx, _system_motion) = fixture(cx, true);
    cx.simulate_resize(size(px(360.), px(720.)));
    act(&view, UiAction::Drawer(Some(Drawer::Profiles)), cx);
    let panel = bounds(cx, "block-drawer");
    assert_eq!(panel.size.height, px(540.));
    assert!(panel.origin.y >= bounds(cx, "window-title").bottom() + px(super::TITLE_BAR_GAP));
    for selector in ["drawer-handle", "selector-results", "manage-games"] {
        let control = bounds(cx, selector);
        assert!(
            control.size.height > px(0.),
            "{selector} collapsed: {control:?}"
        );
        assert!(
            control.origin.y >= panel.origin.y && control.bottom() <= panel.bottom(),
            "{selector} overflows the panel: {control:?}, {panel:?}"
        );
    }
}

/// A panel wanting more room than the window holds stops a gap below the title
/// bar instead of covering it, so its rounded top corners stay on screen and
/// the surface still reads as a drawer.
#[gpui::test]
fn full_height_panel_stops_a_gap_below_the_title_bar(cx: &mut TestAppContext) {
    let (view, cx, _system_motion) = fixture(cx, true);
    cx.simulate_resize(size(px(360.), px(540.)));
    act(&view, UiAction::Drawer(Some(Drawer::Profiles)), cx);
    let title = bounds(cx, "window-title");
    let backdrop = bounds(cx, "block-drawer-backdrop");
    let panel = bounds(cx, "block-drawer");
    assert!(
        panel.size.height < px(540.),
        "the window cannot hold the request"
    );
    assert_eq!(panel.origin.y, title.bottom() + px(super::TITLE_BAR_GAP));
    assert_eq!(
        panel.size.height,
        backdrop.size.height - title.size.height - px(super::TITLE_BAR_GAP),
        "the panel fills the sheet down to its bottom edge"
    );
    assert!(
        bounds(cx, "drawer-handle").origin.y >= title.bottom() + px(super::TITLE_BAR_GAP),
        "the drag handle is reachable below the title bar"
    );
    for selector in ["selector-results", "manage-games"] {
        let control = bounds(cx, selector);
        assert!(
            control.origin.y >= panel.origin.y && control.bottom() <= panel.bottom(),
            "{selector} overflows the shortened panel: {control:?}, {panel:?}"
        );
    }
    let quad = panel_quad(cx);
    let scale = cx.update(|window, _| window.scale_factor());
    let corners = quad.corner_radii;
    assert!((corners.top_left.0 - super::PANEL_RADIUS * scale).abs() < 0.01);
    assert!((corners.top_right.0 - super::PANEL_RADIUS * scale).abs() < 0.01);
    assert_eq!(corners.bottom_left.0, 0.);
    assert_eq!(corners.bottom_right.0, 0.);
    assert!(
        quad.content_mask.bounds.origin.y.0 <= quad.bounds.origin.y.0,
        "the panel's top corners fall outside the sheet's clip"
    );
}

/// The dismissal threshold follows the shortened panel: a 152 px pull is 32% of
/// the 472 px a 540 px window holds below its title bar and gap, but only 28% of
/// the requested 540 px, so a gesture still measured against the request would
/// rebound here.
#[gpui::test]
fn full_height_gesture_measures_the_shortened_panel(cx: &mut TestAppContext) {
    let (view, cx, _system_motion) = fixture(cx, true);
    cx.simulate_resize(size(px(360.), px(540.)));
    act(&view, UiAction::Drawer(Some(Drawer::Profiles)), cx);
    let handle = bounds(cx, "drawer-handle").center();
    let pulled = handle + point(px(0.), px(152.));
    cx.simulate_mouse_down(handle, MouseButton::Left, Modifiers::none());
    frame(cx, 100);
    cx.simulate_mouse_move(pulled, MouseButton::Left, Modifiers::none());
    frame(cx, 150); // A pause removes fling velocity, so distance decides.
    cx.simulate_mouse_up(pulled, MouseButton::Left, Modifiers::none());
    cx.run_until_parked();
    assert!(!view.read_with(cx, |view, _| view.shell.surfaces.is_open()));
}

/// The painted background of the settled panel, found by its painted size.
#[expect(
    clippy::expect_used,
    reason = "A missing panel background is a test failure."
)]
fn panel_quad(cx: &mut VisualTestContext) -> gpui::Quad {
    let panel = bounds(cx, "block-drawer");
    cx.update(|window, _| {
        let scale = window.scale_factor();
        window
            .painted_quads()
            .into_iter()
            .find(|quad| {
                quad.background.as_solid().is_some()
                    && (quad.bounds.size.width.0 - f32::from(panel.size.width) * scale).abs() < 1.
                    && (quad.bounds.size.height.0 - f32::from(panel.size.height) * scale).abs() < 1.
            })
            .expect("drawer panel background painted")
    })
}

#[gpui::test]
fn exit_keeps_body_and_modal_input_until_animation_finishes(cx: &mut TestAppContext) {
    let (view, cx, _system_motion) = fixture(cx, false);
    let invoking_focus = cx.update(|window, cx| window.focused(cx));
    act(&view, UiAction::Drawer(Some(Drawer::Profiles)), cx);
    frame(cx, 500);
    let panel = bounds(cx, "block-drawer");
    act(&view, UiAction::Drawer(None), cx);
    assert!(!view.read_with(cx, |view, _| view.shell.surfaces.is_open()));
    assert!(
        crate::test_support::control_bounds(cx, "manage-games").is_some(),
        "exit retains the real body"
    );
    assert_ne!(cx.update(|window, cx| window.focused(cx)), invoking_focus);
    frame(cx, 100);
    assert!(bounds(cx, "block-drawer").origin.y > panel.origin.y);
    frame(cx, 400);
    assert!(crate::test_support::control_bounds(cx, "block-drawer").is_none());
    assert_eq!(cx.update(|window, cx| window.focused(cx)), invoking_focus);
}

#[gpui::test]
fn rapid_reopen_cannot_be_removed_by_the_previous_exit(cx: &mut TestAppContext) {
    let (view, cx, _system_motion) = fixture(cx, false);
    let invoking_focus = cx.update(|window, cx| window.focused(cx));
    act(&view, UiAction::Drawer(Some(Drawer::Profiles)), cx);
    frame(cx, 500);
    let stale = view.read_with(cx, |view, _| {
        view.shell.surfaces.top().map(|layer| layer.revision)
    });
    act(&view, UiAction::Drawer(None), cx);
    frame(cx, 100);
    act(&view, UiAction::Drawer(Some(Drawer::Accounts)), cx);
    frame(cx, 500);
    assert!(crate::test_support::control_bounds(cx, "manage-accounts").is_some());
    if let Some(stale) = stale {
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.dismiss_surface_overlay(stale, window, cx)
            })
        });
    }
    cx.run_until_parked();
    assert!(crate::test_support::control_bounds(cx, "manage-accounts").is_some());
    assert!(view.read_with(cx, |view, _| view.shell.surfaces.is_open()));
    act(&view, UiAction::Drawer(None), cx);
    frame(cx, 500);
    assert_eq!(cx.update(|window, cx| window.focused(cx)), invoking_focus);
}

#[gpui::test]
fn dialog_opened_during_drawer_exit_keeps_focus_and_returns_to_the_trigger(
    cx: &mut TestAppContext,
) {
    use gpui_component::WindowExt as _;

    let (view, cx, _system_motion) = fixture(cx, false);
    let invoking_focus = cx.update(|window, cx| window.focused(cx));
    act(&view, UiAction::Drawer(Some(Drawer::Profiles)), cx);
    frame(cx, 500);
    act(&view, UiAction::Drawer(None), cx);
    act(&view, UiAction::DeleteProfileId("profile-a".into()), cx);
    let dialog_focus = cx.update(|window, cx| window.focused(cx));
    frame(cx, 500);
    assert!(cx.update(|window, cx| window.has_active_dialog(cx)));
    assert_eq!(cx.update(|window, cx| window.focused(cx)), dialog_focus);
    crate::test_support::press(cx, "escape");
    frame(cx, 250);
    assert_eq!(cx.update(|window, cx| window.focused(cx)), invoking_focus);
}

#[gpui::test]
fn system_reduced_motion_closes_immediately_with_the_app_preference_off(cx: &mut TestAppContext) {
    let (view, cx, _normal_policy) = fixture(cx, false);
    let invoking_focus = cx.update(|window, cx| window.focused(cx));
    act(&view, UiAction::Drawer(Some(Drawer::Profiles)), cx);
    frame(cx, 500);
    let _reduced_policy = crate::platform::test_system_motion(true);
    act(&view, UiAction::Drawer(None), cx);
    assert!(!view.read_with(cx, |view, _| view.preferences.reduce_motion));
    assert!(crate::test_support::control_bounds(cx, "block-drawer").is_none());
    assert_eq!(cx.update(|window, cx| window.focused(cx)), invoking_focus);
}

/// Deliver the animation request itself, so this path cannot hide a missing
/// invalidation behind a forced window.refresh().
fn animation_frame(cx: &mut VisualTestContext, millis: u64) {
    cx.executor().advance_clock(Duration::from_millis(millis));
    cx.update(|window, app| {
        assert!(
            window.simulate_next_frame(app) > 0,
            "motion requested another frame"
        );
    });
    cx.run_until_parked();
}

fn scrim_alpha(cx: &mut VisualTestContext) -> f32 {
    let bounds = bounds(cx, "block-drawer-backdrop");
    cx.update(|window, _| {
        let scale = window.scale_factor();
        window
            .painted_quads()
            .into_iter()
            .filter(|quad| {
                (quad.bounds.size.width.0 - f32::from(bounds.size.width) * scale).abs() < 1.
                    && (quad.bounds.size.height.0 - f32::from(bounds.size.height) * scale).abs()
                        < 1.
            })
            .filter_map(|quad| quad.background.as_solid())
            .filter(|color| color.l == 0. && color.a <= 0.1001)
            .map(|color| color.a)
            .fold(0., f32::max)
    })
}

#[gpui::test]
fn entrance_matches_browser_panel_and_backdrop_tracks_without_forced_refresh(
    cx: &mut TestAppContext,
) {
    let (view, cx, _system_motion) = fixture(cx, false);
    act(&view, UiAction::Drawer(Some(Drawer::Accounts)), cx);
    let start = bounds(cx, "block-drawer");
    let mut samples = Vec::new();
    let mut previous = 0;
    // Independent browser cubic-bezier reference samples for each CSS track.
    for (time, panel, scrim) in [
        (45, 0.4010969, 0.2698207),
        (90, 0.6739781, 0.6556943),
        (135, 0.8321921, 0.8471536),
        (225, 0.9613825, 0.9548094),
        (315, 0.9941938, 0.9882384),
        (450, 1., 1.),
    ] {
        animation_frame(cx, time - previous);
        previous = time;
        samples.push((bounds(cx, "block-drawer"), scrim_alpha(cx), panel, scrim));
    }
    let settled = bounds(cx, "block-drawer");
    let travel = settled.size.height + px(2.);
    assert!((start.origin.y - settled.origin.y - travel).abs() < px(0.6));
    for (painted, alpha, panel, scrim) in samples {
        let expected = settled.origin.y + travel * (1. - panel);
        assert!(
            (painted.origin.y - expected).abs() < px(0.6),
            "{painted:?} vs {expected:?}"
        );
        assert!(
            (alpha - 0.1 * scrim).abs() < 0.0001,
            "{alpha} vs {}",
            0.1 * scrim
        );
    }
}

#[gpui::test]
fn reduced_motion_enters_with_settled_panel_and_backdrop(cx: &mut TestAppContext) {
    let (view, cx, _system_motion) = fixture(cx, true);
    act(&view, UiAction::Drawer(Some(Drawer::Accounts)), cx);
    let panel = bounds(cx, "block-drawer");
    assert_eq!(
        panel.origin.y,
        bounds(cx, "block-drawer-backdrop").bottom() - panel.size.height
    );
    assert!((scrim_alpha(cx) - 0.1).abs() < 0.0001);
}

/// Scroll the wizard body to its end so a control below the fold stays clickable
/// in the compact layout.
fn scroll_wizard_to_end(view: &Entity<Block>, cx: &mut VisualTestContext) {
    view.update(cx, |view, cx| {
        let scroll = &view.shell.wizard.body_scroll;
        scroll.set_offset(point(px(0.), -scroll.max_offset().y));
        cx.notify();
    });
    cx.run_until_parked();
}

/// Choosing a hand-made game from the source sheet continues into the Setup the
/// ordinary creation entry already owns. The sheet is only the first step; the
/// destination is the open draft, so nothing reaches the catalog here.
#[gpui::test]
fn import_new_opens_the_ordinary_new_instance_setup(cx: &mut TestAppContext) {
    let (view, cx, _system_motion) = fixture(cx, true);
    let instances = view.read_with(cx, |view, _| view.snapshot.instances.len());
    act(&view, UiAction::CreateInstance, cx);
    assert_eq!(
        view.read_with(cx, |view, _| view.shell.wizard.step),
        WizardStep::GetStarted
    );

    scroll_wizard_to_end(&view, cx);
    click(cx, "wizard-copy-games");
    assert!(crate::test_support::control_bounds(cx, "import-new").is_some());
    click(cx, "import-new");

    view.read_with(cx, |view, _| {
        assert!(
            !view.shell.surfaces.is_open(),
            "the sheet leaves once a new game owns the page"
        );
        assert_eq!(view.router.tab(), Tab::Instances);
        assert_eq!(view.shell.wizard.kind, CreationKind::NewInstance);
        assert_eq!(view.shell.wizard.step, WizardStep::Setup);
        assert!(
            matches!(view.form, Some(Form::Instance(_))),
            "a draft owns the page instead of a saved game"
        );
        assert_eq!(view.snapshot.instances.len(), instances);
        assert!(!view.busy);
        assert!(!view.shell.import_games.running);
        assert!(view.shell.import_games.batch.is_none());
        assert!(view.shell.import_games.outcomes.is_empty());
    });
    assert!(crate::test_support::control_bounds(cx, "wizard-field-GameName").is_some());
    assert!(crate::test_support::control_bounds(cx, "wizard-next").is_some());

    // The draft that is already open is resumed, not replaced with a fresh one.
    click(cx, "wizard-back");
    assert_eq!(
        view.read_with(cx, |view, _| view.shell.wizard.step),
        WizardStep::GetStarted
    );
    view.update(cx, |view, cx| {
        if let Some(Form::Instance(form)) = &view.form {
            form.name
                .update(cx, |field, cx| field.set_value("Hand made", cx));
        }
    });
    cx.run_until_parked();
    scroll_wizard_to_end(&view, cx);
    click(cx, "wizard-copy-games");
    click(cx, "import-new");
    view.read_with(cx, |view, cx| {
        assert_eq!(view.shell.wizard.step, WizardStep::Setup);
        let name = match &view.form {
            Some(Form::Instance(form)) => form.name.read(cx).value(),
            _ => String::new(),
        };
        assert_eq!(name, "Hand made", "the open draft survives the sheet");
    });
}

/// Setup keeps its own contract: a new game chosen from the sheet returns to its
/// game step and never starts the ordinary Setup draft beside it.
#[gpui::test]
fn import_new_returns_first_run_to_its_game_step(cx: &mut TestAppContext) {
    let (view, cx, _system_motion) = fixture(cx, true);
    cx.simulate_resize(size(px(1152.), px(1000.)));
    view.update(cx, |view, cx| {
        view.preferences.onboarding_complete = false;
        view.saved_preferences.onboarding_complete = false;
        view.shell.first_run.enabled = true;
        view.shell.first_run.stage = FirstRunStage::Overview;
        view.router.navigate(Tab::Onboarding);
        cx.notify();
    });
    cx.run_until_parked();
    let instances = view.read_with(cx, |view, _| view.snapshot.instances.len());

    click(cx, "setup-game");
    assert_eq!(
        view.read_with(cx, |view, _| view.shell.first_run.stage),
        FirstRunStage::GameChoice
    );
    click(cx, "setup-import-game");
    assert!(crate::test_support::control_bounds(cx, "import-new").is_some());
    click(cx, "import-new");

    view.read_with(cx, |view, _| {
        assert!(!view.shell.surfaces.is_open());
        assert_eq!(view.shell.first_run.stage, FirstRunStage::GameChoice);
        assert!(
            view.form.is_none(),
            "setup keeps its own game step rather than a draft"
        );
        assert_eq!(view.router.tab(), Tab::Onboarding);
        assert_eq!(view.snapshot.instances.len(), instances);
        assert!(!view.busy);
    });
    assert!(crate::test_support::control_bounds(cx, "setup-custom").is_some());
}
