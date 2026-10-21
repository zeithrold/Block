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
    StatefulInteractiveElement, StyleRefinement, Styled, Window,
    accesskit::{Action, Node, Role},
    px, rgb,
};
use gpui_base::{Scrollbar, ScrollbarMode};
use std::{cell::RefCell, rc::Rc};

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
        .aria_label(label.to_owned())
        .tab_index(0)
        .focus(move |style| style.shadow(super::design::focus_ring(palette.ring)));
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
        for action in [
            Action::ScrollUp,
            Action::ScrollDown,
            Action::ScrollLeft,
            Action::ScrollRight,
        ] {
            if !horizontal && matches!(action, Action::ScrollLeft | Action::ScrollRight) {
                continue;
            }
            let scroll = self.scroll.clone();
            self.content = self.content.on_a11y_action(action, move |_, window, _| {
                if accessible_scroll(&scroll, action, horizontal) {
                    window.refresh();
                }
            });
        }
        let bar: Rc<RefCell<Option<AnyElement>>> = Rc::default();
        let prepared_bar = bar.clone();
        let painted_bar = bar;
        let scroll = self.scroll.clone();
        let palette = self.palette;
        let bar_id = SharedString::from(format!("scroll-area-base-scrollbar:{}", self.label));
        self.content = self
            .content
            .on_prepaint_bounds(move |_, bounds, window, cx| {
                let mut bar = base_scrollbar(&scroll, palette, horizontal)
                    .id(bar_id.clone())
                    .into_any_element();
                bar.prepaint_as_root(
                    bounds.origin,
                    bounds.size.map(AvailableSpace::Definite),
                    window,
                    cx,
                );
                *prepared_bar.borrow_mut() = Some(bar);
            })
            .on_paint_bounds(move |_, _, window, cx| {
                if let Some(bar) = &mut *painted_bar.borrow_mut() {
                    bar.paint(window, cx);
                }
            });
        ScrollViewport {
            content: self.content.into_element(),
            scroll: self.scroll,
            horizontal,
        }
    }
}

/// Delegate the real viewport's layout and native semantics directly. Painting
/// Base separately avoids adding a synthetic row or including chrome in overflow.
struct ScrollViewport<E: Element> {
    content: E,
    scroll: ScrollHandle,
    horizontal: bool,
}
impl<E: Element> IntoElement for ScrollViewport<E> {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}
impl<E: Element> Element for ScrollViewport<E> {
    type RequestLayoutState = E::RequestLayoutState;
    type PrepaintState = E::PrepaintState;

    fn id(&self) -> Option<ElementId> {
        self.content.id()
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        self.content.source_location()
    }
    fn a11y_role(&self) -> Option<Role> {
        self.content.a11y_role()
    }
    fn write_a11y_info(&self, node: &mut Node) {
        self.content.write_a11y_info(node);
        write_scroll_info(node, &self.scroll, self.horizontal);
    }
    fn a11y_synthetic_children(
        &mut self,
        state: &mut Self::PrepaintState,
        builder: &mut gpui::A11ySubtreeBuilder<'_>,
    ) {
        self.content.a11y_synthetic_children(state, builder);
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
        self.content
            .prepaint(id, inspector, bounds, layout, window, cx)
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
            .paint(id, inspector, bounds, layout, state, window, cx);
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

fn write_scroll_info(node: &mut Node, scroll: &ScrollHandle, horizontal: bool) {
    node.set_scroll_y(f64::from(-scroll.offset().y));
    node.set_scroll_y_min(0.);
    node.set_scroll_y_max(f64::from(maximum(scroll, Axis::Vertical)));
    if horizontal {
        node.set_scroll_x(f64::from(-scroll.offset().x));
        node.set_scroll_x_min(0.);
        node.set_scroll_x_max(f64::from(maximum(scroll, Axis::Horizontal)));
    }
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

fn accessible_scroll(scroll: &ScrollHandle, action: Action, horizontal: bool) -> bool {
    let (axis, direction) = match action {
        Action::ScrollUp => (Axis::Vertical, -1.),
        Action::ScrollDown => (Axis::Vertical, 1.),
        Action::ScrollLeft if horizontal => (Axis::Horizontal, -1.),
        Action::ScrollRight if horizontal => (Axis::Horizontal, 1.),
        _ => return false,
    };
    page(scroll, axis, direction);
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
mod tests {
    use super::*;
    use gpui::{Context, Modifiers, MouseButton, Render, VisualTestContext, point, size};

    struct Pane {
        scroll: ScrollHandle,
        focus: FocusHandle,
    }
    impl Render for Pane {
        fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
            gpui::div().size_full().child(
                gpui::div()
                    .id("scroll-test")
                    .w_full()
                    .h(px(120.))
                    .flex()
                    .flex_col()
                    .scroll_area(
                        &self.scroll,
                        "Scrollable test content",
                        Palette::LIGHT,
                        true,
                    )
                    .track_focus(&self.focus)
                    .children((0..40).map(|index| {
                        gpui::div()
                            .flex_shrink_0()
                            .h(px(32.))
                            .w(px(900.))
                            .child(format!("{index}: 中文 and long unwrapped content"))
                    })),
            )
        }
    }

    struct FittingPane {
        scroll: ScrollHandle,
    }
    impl Render for FittingPane {
        fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
            gpui::div()
                .id("fitting-scroll")
                .w(px(180.))
                .h(px(120.))
                .p_4()
                .flex()
                .flex_col()
                .child(
                    gpui::div()
                        .h(px(32.))
                        .flex_shrink_0()
                        .child("Fits in the viewport"),
                )
                .scroll_area(&self.scroll, "Fitting content", Palette::LIGHT, false)
        }
    }

    struct GroupPane {
        scroll: ScrollHandle,
        focus: FocusHandle,
    }
    impl Render for GroupPane {
        fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
            gpui_base::RadioGroup::new("version-scroll-group")
                .aria_label("Versions")
                .w(px(280.))
                .h(px(120.))
                .p_2()
                .flex()
                .flex_col()
                .gap_2()
                .children((0_usize..40).map(|index| {
                    gpui_base::Radio::new(("version-radio", index))
                        .checked(index == 39)
                        .h(px(32.))
                        .flex_shrink_0()
                        .child(format!("Version {index}"))
                }))
                .scroll_area(&self.scroll, "Versions", Palette::LIGHT, false)
                .track_focus(&self.focus)
        }
    }

    fn painted_thumb(cx: &mut VisualTestContext, vertical: bool) -> Option<Bounds<Pixels>> {
        cx.update(|window, _| {
            let scale = window.scale_factor();
            window.painted_quads().into_iter().find_map(|quad| {
                let background = quad.background;
                let thumb_color = [
                    Palette::LIGHT.border,
                    Palette::LIGHT.muted,
                    Palette::LIGHT.primary,
                ]
                .into_iter()
                .any(|color| background == rgb(color).into());
                let bounds = Bounds::new(
                    point(
                        px(quad.bounds.origin.x.0 / scale),
                        px(quad.bounds.origin.y.0 / scale),
                    ),
                    size(
                        px(quad.bounds.size.width.0 / scale),
                        px(quad.bounds.size.height.0 / scale),
                    ),
                );
                (thumb_color
                    && if vertical {
                        bounds.size.width <= px(6.) && bounds.size.height >= px(24.)
                    } else {
                        bounds.size.height <= px(6.) && bounds.size.width >= px(24.)
                    })
                .then_some(bounds)
            })
        })
    }

    #[gpui::test]
    fn padding_and_scrollbar_decoration_do_not_create_overflow_or_item_indices(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(gpui_base::init);
        let (view, cx) = cx.add_window_view(|_, _| FittingPane {
            scroll: ScrollHandle::new(),
        });
        cx.run_until_parked();
        let scroll = view.read_with(cx, |view, _| view.scroll.clone());
        assert_eq!(scroll.max_offset(), point(px(0.), px(0.)));
        assert!(scroll.bounds_for_item(0).is_some());
        assert!(scroll.bounds_for_item(1).is_none());
        assert!(painted_thumb(cx, true).is_none());
    }

    #[gpui::test]
    fn keyboard_accessibility_dragging_and_resize_share_the_real_scroll_handle(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(gpui_base::init);
        let (view, cx) = cx.add_window_view(|_, cx| Pane {
            scroll: ScrollHandle::new(),
            focus: cx.focus_handle().tab_stop(true),
        });
        cx.simulate_resize(size(px(360.), px(240.)));
        cx.run_until_parked();
        let scroll = view.read_with(cx, |view, _| view.scroll.clone());
        assert!(scroll.max_offset().y >= px(1_000.));
        assert!(scroll.max_offset().x >= px(500.));
        cx.simulate_click(point(px(20.), px(20.)), Modifiers::none());
        cx.simulate_keystrokes("pagedown");
        assert!(scroll.offset().y < px(-90.));
        cx.simulate_keystrokes("end");
        assert_eq!(scroll.offset().y, -scroll.max_offset().y);
        cx.simulate_keystrokes("home shift-pagedown");
        assert_eq!(scroll.offset().y, px(0.));
        assert!(scroll.offset().x < px(-200.));
        cx.update(|window, app| {
            let tree = window.accessibility_tree();
            let node = tree
                .nodes
                .iter()
                .find(|(_, node)| node.label() == Some("Scrollable test content"));
            assert!(node.is_some());
            if let Some((id, node)) = node {
                assert_eq!(node.role(), Role::ScrollView);
                assert!(node.supports_action(Action::ScrollDown));
                assert!(node.supports_action(Action::ScrollRight));
                assert_eq!(node.scroll_y_max(), Some(f64::from(scroll.max_offset().y)));
                assert_eq!(node.scroll_x_max(), Some(f64::from(scroll.max_offset().x)));
                window.simulate_accessibility_action(
                    gpui::accesskit::ActionRequest {
                        action: Action::ScrollDown,
                        target_tree: gpui::accesskit::TreeId::ROOT,
                        target_node: *id,
                        data: None,
                    },
                    app,
                );
            }
        });
        cx.run_until_parked();
        assert!(scroll.offset().y < px(0.));
        let thumb = painted_thumb(cx, true);
        assert!(
            thumb.is_some(),
            "Base must really paint the scrollbar: {:?}",
            cx.update(|window, _| window
                .painted_quads()
                .into_iter()
                .map(|quad| (quad.bounds, quad.background))
                .collect::<Vec<_>>())
        );
        if let Some(thumb) = thumb {
            cx.simulate_mouse_down(thumb.center(), MouseButton::Left, Modifiers::none());
            cx.simulate_mouse_move(
                thumb.center() + point(px(0.), px(40.)),
                MouseButton::Left,
                Modifiers::none(),
            );
            cx.simulate_mouse_up(
                thumb.center() + point(px(0.), px(40.)),
                MouseButton::Left,
                Modifiers::none(),
            );
        }
        assert!(scroll.offset().y < px(-200.));
        cx.simulate_resize(size(px(960.), px(240.)));
        cx.run_until_parked();
        assert_eq!(scroll.max_offset().x, px(0.));
        assert_eq!(scroll.offset().x, px(0.));
        assert!(painted_thumb(cx, false).is_none());
    }

    #[gpui::test]
    fn base_radio_group_keeps_forty_direct_rows_and_reveals_the_last_actual_item(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(gpui_base::init);
        let (view, cx) = cx.add_window_view(|_, cx| GroupPane {
            scroll: ScrollHandle::new(),
            focus: cx.focus_handle().tab_stop(true),
        });
        cx.run_until_parked();
        let scroll = view.read_with(cx, |view, _| view.scroll.clone());
        assert!(scroll.bounds_for_item(39).is_some());
        assert!(
            scroll.bounds_for_item(40).is_none(),
            "chrome must not become a data row"
        );
        cx.update(|window, app| {
            let tree = window.accessibility_tree();
            let group = tree
                .nodes
                .iter()
                .find(|(_, node)| node.label() == Some("Versions"));
            assert!(group.is_some());
            if let Some((id, node)) = group {
                assert_eq!(node.role(), Role::RadioGroup);
                assert!(node.supports_action(Action::ScrollDown));
                window.simulate_accessibility_action(
                    gpui::accesskit::ActionRequest {
                        action: Action::ScrollDown,
                        target_tree: gpui::accesskit::TreeId::ROOT,
                        target_node: *id,
                        data: None,
                    },
                    app,
                );
            }
        });
        cx.run_until_parked();
        assert!(scroll.offset().y < px(0.));
        let focus = view.read_with(cx, |view, _| view.focus.clone());
        cx.update(|window, cx| focus.focus(window, cx));
        cx.simulate_keystrokes("end");
        cx.run_until_parked();
        let last = scroll.bounds_for_item(39);
        assert!(last.is_some());
        if let Some(last) = last {
            let visible_bottom = last.bottom() + scroll.offset().y;
            assert!(visible_bottom <= scroll.bounds().bottom() + px(1.));
            assert!(visible_bottom > scroll.bounds().top());
        }
        assert_eq!(scroll.offset().y, -scroll.max_offset().y);
        cx.simulate_keystrokes("home");
        scroll.scroll_to_item(39);
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        assert!(scroll.offset().y < px(-1_000.));
        assert!(scroll.bounds_for_item(40).is_none());
    }

    #[gpui::test]
    fn scroll_actions_clamp_both_axes_and_do_not_claim_unsupported_directions(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(gpui_base::init);
        let (view, cx) = cx.add_window_view(|_, cx| Pane {
            scroll: ScrollHandle::new(),
            focus: cx.focus_handle().tab_stop(true),
        });
        cx.run_until_parked();
        cx.simulate_resize(size(px(360.), px(240.)));
        cx.run_until_parked();
        let scroll = view.read_with(cx, |view, _| view.scroll.clone());
        set_axis(&scroll, Axis::Vertical, px(-100_000.));
        assert_eq!(scroll.offset().y, -scroll.max_offset().y);
        set_axis(&scroll, Axis::Vertical, px(100_000.));
        assert_eq!(scroll.offset().y, px(0.));
        assert!(!accessible_scroll(&scroll, Action::ScrollRight, false));
        assert_eq!(scroll.offset().x, px(0.));
        assert!(accessible_scroll(&scroll, Action::ScrollRight, true));
        assert!(scroll.offset().x < px(0.));
        assert!(!accessible_scroll(&scroll, Action::Click, true));
    }
}
