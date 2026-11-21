use super::{Block, Language, Surface, UiAction};
use crate::{Appearance, Tab, test_support::press};
use gpui::{Bounds, Modifiers, Pixels, VisualTestContext, px, size};
use gpui_base::test_support::{ElementSnapshot, find, texts};
use std::time::Duration;

#[path = "select_catalog_tests.rs"]
mod catalogs;
#[path = "select_draft_tests.rs"]
mod drafts;
#[path = "select_editor_tests.rs"]
mod editors;

fn select(cx: &mut VisualTestContext, trigger: &str) -> ElementSnapshot {
    let id = format!("{trigger}-select").into();
    cx.update(|window, _| find(window, &[], &id))
        .unwrap_or_else(|| unreachable!("missing styled Select {trigger}"))
}

fn row_text(cx: &mut VisualTestContext, trigger: &str) -> Vec<String> {
    let path = select(cx, trigger).path().to_owned();
    cx.update(|window, _| {
        texts(window)
            .into_iter()
            .filter(|text| {
                text.visible()
                    && text.path().starts_with(&path)
                    && text.path().iter().any(|id| id == &"list-state".into())
            })
            .map(|text| text.text().to_owned())
            .collect()
    })
}

fn settle(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(500));
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

fn settle_tab(cx: &mut VisualTestContext) {
    for _ in 0..2 {
        cx.update(|window, cx| window.simulate_next_frame(cx));
        cx.run_until_parked();
    }
    settle(cx);
}

fn click(cx: &mut VisualTestContext, selector: &'static str) -> Option<Bounds<Pixels>> {
    settle(cx);
    let bounds = crate::test_support::control_bounds(cx, selector);
    assert!(
        bounds.is_some(),
        "missing actual product control {selector}"
    );
    if let Some(bounds) = bounds {
        let window_count = cx.windows().len();
        cx.simulate_click(bounds.center(), Modifiers::none());
        cx.run_until_parked();
        if cx.windows().len() > window_count
            && let Some(window) = cx.windows().last()
        {
            *cx = VisualTestContext::from_window(*window, &cx.cx);
        }
        crate::test_support::ensure_live_window(cx);
        cx.run_until_parked();
        crate::test_support::ensure_live_window(cx);
    }
    settle(cx);
    bounds
}

#[path = "select_key_tests.rs"]
mod keys;
#[path = "select_menu_tests.rs"]
mod menus;
