//! Shared product viewport. Base owns its scrollbar; content, keyboard and
//! accessibility scrolling all use the caller's original `ScrollHandle`.
#![expect(
    clippy::redundant_pub_crate,
    reason = "Shared private product primitive."
)]

use super::Palette;
use gpui::{
    AnyElement, App, AvailableSpace, Bounds, Element, ElementId, FocusHandle, GlobalElementId,
    InspectorElementId, InteractiveElement, Interactivity, IntoElement, KeyDownEvent, LayoutId,
    ParentElement, Pixels, Point, RenderOnce, ScrollHandle, SharedString,
    StatefulInteractiveElement, StyleRefinement, Styled, Window, accesskit::Role, px, rgb,
};
use gpui_base::{Scrollbar, ScrollbarMode};

#[derive(Clone, Copy)]
enum Axis {
    Vertical,
    Horizontal,
}

/// Both native Divs and Base `RadioGroup` retain their own immediate data rows.
pub(super) trait ScrollContent:
    IntoElement + StatefulInteractiveElement + Styled + ParentElement + 'static
{
}
impl<E> ScrollContent for E where
    E: IntoElement + StatefulInteractiveElement + Styled + ParentElement + 'static
{
}

/// A presentation wrapper, with no layout child, entity or second scroll handle.
/// Attach an explicit focus handle after `scroll_area` so it remains the viewport
/// keyboard target instead of being replaced by the default retained handle.
#[derive(IntoElement)]
pub(super) struct ScrollArea<E: ScrollContent> {
    content: E,
    scroll: ScrollHandle,
    label: SharedString,
    palette: Palette,
    horizontal: bool,
    focus: Option<FocusHandle>,
    tab_index: Option<isize>,
    tab_stop: Option<bool>,
}

/// Preserve content sizing, padding and immediate item indices. The scrollbar is
/// painted next to the native viewport, never appended to its tracked children.
pub(super) trait ScrollAreaExt: ScrollContent + Sized {
    fn scroll_area(
        self,
        scroll: &ScrollHandle,
        label: &str,
        palette: Palette,
        horizontal: bool,
    ) -> ScrollArea<Self> {
        viewport_with_role(self, scroll, label, palette, horizontal, Role::ScrollView)
    }

    fn scroll_list_area(
        self,
        scroll: &ScrollHandle,
        label: &str,
        palette: Palette,
    ) -> ScrollArea<Self> {
        viewport_with_role(self, scroll, label, palette, false, Role::ListBox)
            .tab_index(-1)
            .tab_stop(false)
    }
}
impl<E: ScrollContent> ScrollAreaExt for E {}

pub(super) fn viewport<E: ScrollContent>(
    view: E,
    scroll: &ScrollHandle,
    label: &str,
    palette: Palette,
    horizontal: bool,
) -> ScrollArea<E> {
    viewport_with_role(view, scroll, label, palette, horizontal, Role::ScrollView)
}

pub(super) fn viewport_with_role<E: ScrollContent>(
    view: E,
    scroll: &ScrollHandle,
    label: &str,
    palette: Palette,
    horizontal: bool,
    role: Role,
) -> ScrollArea<E> {
    let mut content = view
        .relative()
        .overflow_y_scroll()
        .track_scroll(scroll)
        .role(role)
        .tab_index(0);
    if horizontal {
        content = content.overflow_x_scroll();
    }
    ScrollArea {
        content,
        scroll: scroll.clone(),
        label: label.to_owned().into(),
        palette,
        horizontal,
        focus: None,
        tab_index: None,
        tab_stop: None,
    }
}

impl<E: ScrollContent> Styled for ScrollArea<E> {
    fn style(&mut self) -> &mut StyleRefinement {
        self.content.style()
    }
}
impl<E: ScrollContent> ParentElement for ScrollArea<E> {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.content.extend(elements);
    }
}
impl<E: ScrollContent> InteractiveElement for ScrollArea<E> {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.content.interactivity()
    }

    fn track_focus(mut self, focus: &FocusHandle) -> Self {
        self.focus = Some(focus.clone());
        self.content = self.content.track_focus(focus);
        self
    }

    fn tab_index(mut self, index: isize) -> Self {
        self.tab_index = Some(index);
        self.tab_stop = Some(true);
        self.content = self.content.tab_index(index);
        self
    }

    fn tab_stop(mut self, stop: bool) -> Self {
        self.tab_stop = Some(stop);
        self.content = self.content.tab_stop(stop);
        self
    }
}
impl<E: ScrollContent> StatefulInteractiveElement for ScrollArea<E> {}

impl<E: ScrollContent> RenderOnce for ScrollArea<E> {
    fn render(mut self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let mut focus = self.focus.unwrap_or_else(|| {
            window
                .use_keyed_state(
                    SharedString::from(format!("scroll-viewport-focus:{}", self.label)),
                    cx,
                    |_, cx| cx.focus_handle().tab_stop(true),
                )
                .read(cx)
                .clone()
        });
        if let Some(index) = self.tab_index {
            focus = focus.tab_index(index);
        }
        if let Some(stop) = self.tab_stop {
            focus = focus.tab_stop(stop);
        }
        let key_focus = focus.clone();
        let key_scroll = self.scroll.clone();
        let horizontal = self.horizontal;
        self.content = self
            .content
            .track_focus(&focus)
            .on_key_down(move |event, window, cx| {
                if key_focus.is_focused(window) && keyboard_scroll(&key_scroll, event, horizontal) {
                    window.refresh();
                    cx.stop_propagation();
                }
            });

        ScrollViewport {
            content: self.content.into_element(),
            bar: base_scrollbar(&self.scroll, self.palette, horizontal).into_any_element(),
            focus,
            focus_color: self.palette.ring,
        }
    }
}

/// Delegate the real viewport's layout and native semantics directly. Painting
/// Base separately avoids adding a synthetic row or including chrome in overflow.
struct ScrollViewport<E: Element> {
    content: E,
    bar: AnyElement,
    focus: FocusHandle,
    focus_color: u32,
}
impl<E: Element> IntoElement for ScrollViewport<E> {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}
impl<E: Element> Element for ScrollViewport<E> {
    type RequestLayoutState = E::RequestLayoutState;
    type PrepaintState = (E::PrepaintState, ());

    fn id(&self) -> Option<ElementId> {
        self.content.id()
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        self.content.source_location()
    }
    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        self.content.request_layout(id, inspector, window, cx)
    }
    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let content = self
            .content
            .prepaint(id, inspector, bounds, layout, window, cx);
        self.bar.prepaint_as_root(
            bounds.origin,
            bounds.size.map(AvailableSpace::Definite),
            window,
            cx,
        );
        (content, ())
    }
    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        layout: &mut Self::RequestLayoutState,
        state: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.content
            .paint(id, inspector, bounds, layout, &mut state.0, window, cx);
        self.bar.paint(window, cx);
        if self.focus.is_focused(window) && window.last_input_was_keyboard() {
            // A drop shadow fills the center of a transparent viewport. Paint
            // an actual inside outline so focus cannot tint the page or change
            // its layout; pointer focus needs no extra presentation.
            let mut ring = gpui::outline(
                bounds,
                rgb(self.focus_color).opacity(0.5),
                Default::default(),
            );
            ring.border_widths = gpui::Edges::all(px(2.));
            window.paint_quad(ring);
        }
    }
}

fn base_scrollbar(scroll: &ScrollHandle, palette: Palette, horizontal: bool) -> Scrollbar {
    let bar = if horizontal {
        Scrollbar::new(scroll)
    } else {
        Scrollbar::vertical(scroll)
    };
    bar.id("scroll-area-base-scrollbar")
        .mode(ScrollbarMode::Always)
        .styles(|styles| {
            styles
                .track(|style| {
                    style
                        .width(px(10.))
                        .bg(gpui::transparent_black())
                        .border_color(gpui::transparent_black())
                })
                .thumb(|style| {
                    // Base's minimum includes both longitudinal insets. Keep
                    // the product's painted thumb at least 24px long.
                    style
                        .bg(rgb(palette.border))
                        .width(px(4.))
                        .inset(px(3.))
                        .radius(px(3.))
                        .min_length(px(30.))
                })
                .thumb_hover(|style| {
                    style
                        .bg(rgb(palette.muted))
                        .width(px(6.))
                        .inset(px(2.))
                        .radius(px(3.))
                        .min_length(px(28.))
                })
                .thumb_active(|style| {
                    style
                        .bg(rgb(palette.primary))
                        .width(px(6.))
                        .inset(px(2.))
                        .radius(px(3.))
                        .min_length(px(28.))
                })
        })
}

fn keyboard_scroll(scroll: &ScrollHandle, event: &KeyDownEvent, horizontal: bool) -> bool {
    let modifiers = event.keystroke.modifiers;
    if modifiers.control || modifiers.alt || modifiers.platform || modifiers.function {
        return false;
    }
    let axis = if horizontal && modifiers.shift {
        Axis::Horizontal
    } else {
        Axis::Vertical
    };
    match event.keystroke.key.as_str() {
        "pageup" => page(scroll, axis, -1.),
        "pagedown" => page(scroll, axis, 1.),
        "home" => set_axis(scroll, axis, px(0.)),
        "end" => set_axis(scroll, axis, -maximum(scroll, axis)),
        "up" => set_axis(scroll, Axis::Vertical, scroll.offset().y + px(40.)),
        "down" => set_axis(scroll, Axis::Vertical, scroll.offset().y - px(40.)),
        "left" if horizontal => set_axis(scroll, Axis::Horizontal, scroll.offset().x + px(40.)),
        "right" if horizontal => set_axis(scroll, Axis::Horizontal, scroll.offset().x - px(40.)),
        _ => return false,
    }
    true
}

const fn coordinate(point: Point<Pixels>, axis: Axis) -> Pixels {
    match axis {
        Axis::Vertical => point.y,
        Axis::Horizontal => point.x,
    }
}
const fn length(bounds: Bounds<Pixels>, axis: Axis) -> Pixels {
    match axis {
        Axis::Vertical => bounds.size.height,
        Axis::Horizontal => bounds.size.width,
    }
}
fn maximum(scroll: &ScrollHandle, axis: Axis) -> Pixels {
    match axis {
        Axis::Vertical => scroll.max_offset().y,
        Axis::Horizontal => scroll.max_offset().x,
    }
    .max(px(0.))
}
fn set_axis(scroll: &ScrollHandle, axis: Axis, value: Pixels) {
    let mut offset = scroll.offset();
    let value = value.clamp(-maximum(scroll, axis), px(0.));
    match axis {
        Axis::Vertical => offset.y = value,
        Axis::Horizontal => offset.x = value,
    }
    scroll.set_offset(offset);
}
fn page(scroll: &ScrollHandle, axis: Axis, direction: f32) {
    set_axis(
        scroll,
        axis,
        coordinate(scroll.offset(), axis) - length(scroll.bounds(), axis) * (direction * 0.9),
    );
}

#[cfg(test)]
#[path = "scroll_area_tests.rs"]
mod tests;
