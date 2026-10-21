//! Exact-version adapter for Base's modal and keyboard contracts. Product
//! surfaces keep their business stack; Base owns each rendered modal host.
#![expect(
    clippy::redundant_pub_crate,
    reason = "The adapter is private product plumbing."
)]

use gpui::{
    AnyElement, App, FocusHandle, InteractiveElement as _, IntoElement, ParentElement as _,
    RenderOnce, SharedString, WeakEntity, Window, div,
};

use crate::{Block, UiAction};

#[derive(IntoElement)]
pub(super) struct Modal {
    pub view: WeakEntity<Block>,
    pub focus: FocusHandle,
    pub revision: usize,
    pub label: SharedString,
    pub depth: usize,
    pub inert: bool,
    pub content: AnyElement,
}

impl RenderOnce for Modal {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let revision = self.revision;
        let cancel = self.view.clone();
        let close = self.view;
        div().id(("base-modal", revision)).inert(self.inert).child(
            gpui_base::Dialog::new(cx)
                .accessibility_label(self.label)
                .focus_handle(self.focus)
                .layer(self.depth, !self.inert)
                .on_ok(|_, _, cx| {
                    // Enter belongs to the focused input/button, whose native
                    // KeyUp activation must run exactly once.
                    cx.propagate();
                    false
                })
                .on_cancel(move |_, window, cx| {
                    cancel
                        .update(cx, |view, cx| {
                            if view.dismiss_tooltip(window, cx) {
                                return false;
                            }
                            view.shell
                                .overlays
                                .top()
                                .is_some_and(|top| top.revision == revision)
                        })
                        .unwrap_or(false)
                })
                .on_open_change(move |open, _, window, cx| {
                    if !open {
                        let _ = close.update(cx, |view, cx| {
                            if view
                                .shell
                                .overlays
                                .top()
                                .is_some_and(|top| top.revision == revision)
                            {
                                view.handle(UiAction::Drawer(None), window, cx);
                            }
                        });
                    }
                })
                .backdrop(self.content),
        )
    }
}

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

#[cfg(test)]
mod tests {
    #[gpui::test]
    fn fixed_base_root_actions_are_registered(cx: &gpui::TestAppContext) {
        cx.update(|cx| {
            crate::initialize(cx);
            assert!(cx.build_action("root::Tab", None).is_ok());
            assert!(cx.build_action("root::TabPrev", None).is_ok());
        });
    }
}
