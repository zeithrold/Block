//! Delegate keyboard traversal to the upstream Base Root.

use gpui::{App, Window};

/// Base 0.8.0 registers these actions but keeps their Rust types private. Resolve
/// through GPUI's public action registry, so Block's entrance settling and menu
/// arrows delegate to precisely the same Root traversal as physical Tab.
pub(super) fn advance_focus(backwards: bool, window: &mut Window, cx: &mut App) {
    let name = if backwards {
        "root::TabPrev"
    } else {
        "root::Tab"
    };
    if let Ok(action) = cx.build_action(name, None) {
        window.dispatch_action(action, cx);
    }
}
