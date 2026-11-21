use super::{Block, click, row_text, select, settle, settle_tab};
use crate::{
    browser::CollectionLoad,
    game_wizard::{CreationKind, LoaderCatalog, WizardStep},
    modpack_import::{Phase, State, TargetDraft},
    test_support::{control_bounds, press},
};
use block_launcher::catalog::LoaderKind;
use block_launcher::install::fabric::LoaderVersion;
use gpui::{Entity, Modifiers, ScrollDelta, ScrollEvent, VisualTestContext, point, px, size};

const COUNT: usize = 70;
const VERSION: &str = "modpack-target-version";
const BUILD: &str = "modpack-target-build";

#[path = "select_catalog_refresh_tests.rs"]
mod refresh;

fn version(index: usize) -> String {
    format!("1.21.{index}")
}

fn build(index: usize) -> String {
    format!("0.16.{index}")
}

fn wizard(
    cx: &mut gpui::TestAppContext,
    draft: TargetDraft,
) -> (Entity<Block>, &mut VisualTestContext) {
    let inspection = (|| -> anyhow::Result<_> {
        let root = tempfile::tempdir()?;
        let source = root.path().join("plain.zip");
        crate::test_support::write_plain_pack(&source)?;
        block_launcher::modpack::inspect(&source)
    })()
    .unwrap_or_else(|error| unreachable!("create Plain archive fixture: {error:#}"));
    let (view, cx) = crate::test_support::window(cx, move |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.preferences.reduce_motion = true;
        view.versions = (0..COUNT).map(version).collect();
        view.begin_game_wizard(cx);
        view.shell.wizard.kind = CreationKind::ImportPack;
        view.shell.wizard.step = WizardStep::Setup;
        view.shell.pack_import = State::default();
        view.shell.pack_import.phase = Phase::Review;
        view.shell.pack_import.draft = Some(draft);
        view.shell.pack_import.inspection = Some(inspection);
        view
    });
    cx.simulate_resize(size(px(1152.), px(720.)));
    settle(cx);
    (view, cx)
}

fn open(view: &Entity<Block>, cx: &mut VisualTestContext, field: &'static str) {
    let bounds = select(cx, field).bounds();
    view.update(cx, |view, cx| {
        let scroll = &view.shell.pack_import.scroll;
        let viewport = scroll.bounds();
        let delta = if bounds.bottom() > viewport.bottom() {
            viewport.bottom() - bounds.bottom() - px(8.)
        } else if bounds.top() < viewport.top() {
            viewport.top() - bounds.top() + px(8.)
        } else {
            px(0.)
        };
        scroll.set_offset(point(scroll.offset().x, scroll.offset().y + delta));
        cx.notify();
    });
    click(cx, field);
    settle_tab(cx);
    let picker = select(cx, field);
    assert_eq!(picker.expanded(), Some(true));
    assert_eq!(picker.focused(), Some(true));
}

fn wheel(cx: &mut VisualTestContext, delta: f32) {
    let bounds = cx
        .debug_bounds("select-choice-0")
        .or_else(|| cx.debug_bounds("select-choice-69"))
        .unwrap_or_else(|| unreachable!("an endpoint is rendered before a wheel gesture"));
    let pointer = bounds.center();
    cx.simulate_mouse_move(pointer, None, Modifiers::none());
    cx.simulate_event(ScrollEvent {
        position: pointer,
        delta: ScrollDelta::Pixels(point(px(0.), px(delta))),
        ..ScrollEvent::default()
    });
    settle(cx);
}

fn verify_catalog(cx: &mut VisualTestContext, field: &str, label: fn(usize) -> String) {
    let expected: Vec<_> = (0..COUNT).map(label).collect();
    let mut seen = Vec::new();
    // Exercise the native virtual list's reveal path across the full catalog.
    // Every row must paint in order, including those outside the first viewport.
    for _ in 0..COUNT {
        let rows = row_text(cx, field);
        assert!(
            !rows.is_empty() && rows.len() < COUNT,
            "bounded catalog: {rows:?}"
        );
        for row in rows {
            if !seen.contains(&row) {
                seen.push(row);
            }
        }
        press(cx, "down");
        settle_tab(cx);
    }
    assert_eq!(seen, expected);
    press(cx, "home");
    settle_tab(cx);
}

fn draft_version(view: &Entity<Block>, cx: &VisualTestContext) -> Option<String> {
    view.read_with(cx, |view, _| {
        view.shell
            .pack_import
            .draft
            .as_ref()
            .and_then(|draft| draft.version.clone())
    })
}

#[gpui::test]
fn long_target_version_list_scrolls_inside_a_bounded_popup_and_keeps_every_choice(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = wizard(cx, TargetDraft::default());
    open(&view, cx, VERSION);
    let unset = select(cx, VERSION).value().map(str::to_owned);
    assert_eq!(draft_version(&view, cx), None);
    verify_catalog(cx, VERSION, version);
    press(cx, "end");
    settle_tab(cx);
    assert_eq!(row_text(cx, VERSION).last(), Some(&version(COUNT - 1)));
    assert_eq!(select(cx, VERSION).value(), unset.as_deref());
    press(cx, "home");
    settle_tab(cx);
    assert_eq!(row_text(cx, VERSION).first(), Some(&version(0)));
    press(cx, "down enter");
    settle_tab(cx);
    assert_eq!(draft_version(&view, cx), Some(version(1)));
    open(&view, cx, VERSION);
    press(cx, "up enter");
    settle_tab(cx);
    assert_eq!(draft_version(&view, cx), Some(version(0)));
    open(&view, cx, VERSION);
    press(cx, "end enter");
    settle_tab(cx);
    let last = version(COUNT - 1);
    assert_eq!(draft_version(&view, cx), Some(last.clone()));
    assert_eq!(select(cx, VERSION).expanded(), Some(false));
    assert!(control_bounds(cx, "modpack-import").is_some());
    open(&view, cx, VERSION);
    assert_eq!(select(cx, VERSION).value(), Some(last.as_str()));
    assert_eq!(row_text(cx, VERSION).last(), Some(&last));
}

#[gpui::test]
fn wheel_scrolls_the_long_selector_without_moving_the_wizard_beneath(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = wizard(cx, TargetDraft::default());
    open(&view, cx, VERSION);
    let behind = view.read_with(cx, |view, _| {
        (view.shell.pack_import.scroll.offset(), view.router.tab())
    });
    wheel(cx, -4000.);
    assert_eq!(row_text(cx, VERSION).last(), Some(&version(COUNT - 1)));
    assert_eq!(draft_version(&view, cx), None);
    wheel(cx, 4000.);
    assert_eq!(row_text(cx, VERSION).first(), Some(&version(0)));
    assert_eq!(
        view.read_with(cx, |view, _| (
            view.shell.pack_import.scroll.offset(),
            view.router.tab()
        )),
        behind
    );
    assert!(view.read_with(cx, |view, _| view.pack_wizard_active()));
    assert_eq!(select(cx, VERSION).expanded(), Some(true));
    press(cx, "enter");
    settle_tab(cx);
    assert_eq!(
        draft_version(&view, cx),
        Some(version(0)),
        "wheel did not move the cursor"
    );
}

#[gpui::test]
fn wizard_selector_keyboard_and_escape_preserve_the_page(cx: &mut gpui::TestAppContext) {
    let (view, cx) = wizard(cx, TargetDraft::default());
    open(&view, cx, VERSION);
    let popup_focus = cx.update(|window, cx| window.focused(cx));
    press(cx, "enter");
    settle_tab(cx);
    assert_eq!(draft_version(&view, cx), Some(version(0)));
    assert_eq!(select(cx, VERSION).expanded(), Some(false));
    assert_eq!(select(cx, VERSION).focused(), Some(true));
    assert!(view.read_with(cx, |view, _| view.pack_wizard_active()));
    let trigger = cx
        .update(|window, cx| window.focused(cx))
        .unwrap_or_else(|| unreachable!("the trigger regained focus"));
    assert_ne!(popup_focus, Some(trigger.clone()));

    for keys in ["tab", "shift-tab"] {
        cx.update(|window, cx| trigger.focus(window, cx));
        press(cx, keys);
        settle_tab(cx);
        let expected = cx.update(|window, cx| window.focused(cx));
        assert_ne!(expected, Some(trigger.clone()), "{keys} moves focus");
        open(&view, cx, VERSION);
        press(cx, "end");
        press(cx, keys);
        settle_tab(cx);
        assert_eq!(
            select(cx, VERSION).expanded(),
            Some(false),
            "{keys} closes the popup"
        );
        assert_eq!(
            cx.update(|window, cx| window.focused(cx)),
            expected,
            "{keys} traverses once"
        );
        assert!(view.read_with(cx, |view, _| view.pack_wizard_active()));
        assert_eq!(
            draft_version(&view, cx),
            Some(version(0)),
            "{keys} chooses nothing"
        );
    }

    open(&view, cx, VERSION);
    press(cx, "escape");
    settle_tab(cx);
    assert_eq!(select(cx, VERSION).expanded(), Some(false));
    assert!(view.read_with(cx, |view, _| view.pack_wizard_active()));
    assert_eq!(cx.update(|window, cx| window.focused(cx)), Some(trigger));
    press(cx, "escape");
    settle(cx);
    assert!(!view.read_with(cx, |view, _| view.pack_wizard_active()));
}

#[gpui::test]
fn long_loader_build_list_scrolls_and_chooses_its_last_build(cx: &mut gpui::TestAppContext) {
    let minecraft = version(0);
    let (view, cx) = wizard(
        cx,
        TargetDraft {
            version: Some(minecraft.clone()),
            loader: Some(LoaderKind::Fabric),
            catalog: Some(LoaderCatalog {
                request: uuid::Uuid::nil(),
                minecraft,
                kind: LoaderKind::Fabric,
                builds: (0..COUNT)
                    .map(|index| LoaderVersion {
                        version: build(index),
                        stable: true,
                    })
                    .collect(),
                load: CollectionLoad::Ready,
            }),
            ..TargetDraft::default()
        },
    );
    open(&view, cx, BUILD);
    verify_catalog(cx, BUILD, build);
    press(cx, "end enter");
    settle_tab(cx);
    assert_eq!(
        view.read_with(cx, |view, _| view
            .shell
            .pack_import
            .draft
            .as_ref()
            .and_then(|draft| draft.build.clone())),
        Some(build(COUNT - 1))
    );
    assert_eq!(select(cx, BUILD).expanded(), Some(false));
    assert!(view.read_with(cx, |view, _| view.pack_wizard_active()));
    assert!(control_bounds(cx, "modpack-import").is_some());
}
