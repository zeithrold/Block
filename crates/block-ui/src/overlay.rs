//! Window-local overlay lifetime and focus ownership shared by all surfaces.

#![expect(
    clippy::redundant_pub_crate,
    reason = "Overlay internals are shared only by the UI module."
)]

use gpui::{App, Context, FocusHandle, Window};
use std::{cell::RefCell, collections::HashMap};

#[derive(Clone, Debug)]
pub(super) struct Layer<K> {
    pub kind: K,
    pub scroll: gpui::ScrollHandle,
    pub focus: FocusHandle,
    pub revision: usize,
    return_focus: Option<FocusHandle>,
}

/// The last layer alone receives dismissal and keyboard navigation. Parent
/// layers stay mounted, so closing a nested confirmation restores its context.
#[derive(Clone, Debug)]
pub(super) struct OverlayHost<K> {
    pub tooltip: super::tooltip::TooltipHost,
    trigger_focus: RefCell<HashMap<&'static str, FocusHandle>>,
    layers: Vec<Layer<K>>,
    revision: usize,
}

impl<K> Default for OverlayHost<K> {
    fn default() -> Self {
        Self {
            tooltip: super::tooltip::TooltipHost::default(),
            trigger_focus: RefCell::default(),
            layers: Vec::new(),
            revision: 0,
        }
    }
}

impl<K: Clone> OverlayHost<K> {
    pub(super) fn snapshot(&self) -> Self {
        let mut snapshot = self.clone();
        for layer in &mut snapshot.layers {
            let scroll = gpui::ScrollHandle::new();
            scroll.set_offset(layer.scroll.offset());
            layer.scroll = scroll;
        }
        snapshot
    }
}

impl<K> OverlayHost<K> {
    pub(super) fn trigger_focus(&self, id: &'static str, cx: &App) -> FocusHandle {
        self.trigger_focus
            .borrow_mut()
            .entry(id)
            .or_insert_with(|| cx.focus_handle())
            .clone()
    }

    /// The controlled Base Select restores its own trigger on this route.
    /// Other modal dismissal routes continue to use `close` exactly once.
    pub(super) fn discard(&mut self) -> Option<K> {
        self.tooltip.invalidate();
        self.layers.pop().map(|layer| layer.kind)
    }
    pub(super) fn open<T>(&mut self, kind: K, window: &mut Window, cx: &mut Context<'_, T>) {
        self.tooltip.invalidate();
        let return_focus = window.focused(cx);
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        self.revision = self.revision.wrapping_add(1);
        self.layers.push(Layer {
            kind,
            scroll: gpui::ScrollHandle::new(),
            focus,
            return_focus,
            revision: self.revision,
        });
    }

    pub(super) fn close(
        &mut self,
        window: &mut Window,
        fallback: &FocusHandle,
        cx: &mut App,
    ) -> Option<K> {
        self.tooltip.invalidate();
        let layer = self.layers.pop()?;
        layer
            .return_focus
            .as_ref()
            .unwrap_or(fallback)
            .focus(window, cx);
        Some(layer.kind)
    }

    /// Navigation destroys the trigger's page: restore a stable page focus,
    /// rather than keeping a handle into the removed tree.
    pub(super) fn clear(&mut self, window: &mut Window, fallback: &FocusHandle, cx: &mut App) {
        self.tooltip.invalidate();
        self.layers.clear();
        fallback.focus(window, cx);
    }

    pub(super) const fn top(&self) -> Option<&Layer<K>> {
        self.layers.as_slice().last()
    }
    pub(super) const fn top_mut(&mut self) -> Option<&mut Layer<K>> {
        self.layers.as_mut_slice().last_mut()
    }
    pub(super) fn layers(&self) -> impl Iterator<Item = &Layer<K>> {
        self.layers.iter()
    }
    pub(super) const fn is_open(&self) -> bool {
        !self.layers.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::OverlayHost;
    use gpui::{Context, IntoElement, Render, Window, div};

    struct Harness {
        overlays: OverlayHost<&'static str>,
        focus: gpui::FocusHandle,
    }
    impl Render for Harness {
        fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
            div()
        }
    }

    #[gpui::test]
    fn closing_nested_layer_restores_parent_then_trigger(cx: &mut gpui::TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, cx| Harness {
            overlays: OverlayHost::default(),
            focus: cx.focus_handle(),
        });
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.focus.focus(window, cx);
                view.overlays.open("selector", window, cx);
                let parent = view.overlays.top().map(|layer| layer.focus.clone());
                view.overlays.open("confirmation", window, cx);
                assert_eq!(
                    view.overlays.close(window, &view.focus, cx),
                    Some("confirmation")
                );
                assert!(parent.is_some_and(|focus| focus.is_focused(window)));
                assert_eq!(
                    view.overlays.close(window, &view.focus, cx),
                    Some("selector")
                );
                assert!(view.focus.is_focused(window));
                assert!(!view.overlays.is_open());
            });
        });
    }
}
