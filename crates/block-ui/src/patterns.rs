//! Product layout contracts. Panes own scrolling; action rails never scroll away.
#![expect(
    clippy::redundant_pub_crate,
    reason = "Patterns are shared by private product screens."
)]

use super::Palette;
use gpui::{
    AnyElement, Div, InteractiveElement as _, ParentElement as _, Pixels, Size, Styled as _, div,
    px, relative, rgb,
};

/// Reveal a newly invalid field after its actual layout, including a page that
/// was hidden when validation ran. The same path runs in native and test windows.
pub(super) fn reveal_field<T: Copy + Eq + 'static>(
    field_view: Div,
    scroll: gpui::ScrollHandle,
    pending: std::rc::Rc<std::cell::Cell<Option<T>>>,
    field: T,
) -> Div {
    field_view.on_children_prepainted(move |bounds, window, cx| {
        if pending.get() != Some(field) {
            return;
        }
        pending.set(None);
        if let (Some(first), Some(last)) = (bounds.first(), bounds.last()) {
            let viewport = scroll.bounds();
            let delta = if first.top() < viewport.top() + px(8.) {
                viewport.top() + px(8.) - first.top()
            } else if last.bottom() > viewport.bottom() - px(8.) {
                viewport.bottom() - px(8.) - last.bottom()
            } else {
                px(0.)
            };
            if delta != px(0.) {
                let offset = scroll.offset();
                scroll.set_offset(gpui::point(offset.x, offset.y + delta));
                window.defer(cx, |window, _| window.refresh());
            }
        }
    })
}

pub(super) const NARROW_WIDTH: f32 = 640.;

pub(super) fn is_narrow(viewport: Size<Pixels>) -> bool {
    viewport.width < px(NARROW_WIDTH)
}

pub(super) fn management_footer(viewport: Size<Pixels>, palette: Palette) -> Div {
    div()
        .h(px(if is_narrow(viewport) {
            super::design::NARROW_FOOTER
        } else {
            super::design::MANAGEMENT_FOOTER
        }))
        .flex_shrink_0()
        .flex()
        .items_center()
        .gap_3()
        .px(px(if is_narrow(viewport) { 16. } else { 32. }))
        .border_t_1()
        .border_color(rgb(palette.border))
}

pub(super) fn management_layout(
    viewport: Size<Pixels>,
    browser: AnyElement,
    detail: AnyElement,
    show_detail: bool,
    palette: Palette,
    view: &super::Block,
) -> Div {
    let narrow = is_narrow(viewport);
    let mut row = div()
        .size_full()
        .min_h_0()
        .min_w_0()
        .flex()
        .overflow_hidden();
    if !narrow || !show_detail {
        row = row.child(
            view.frame_sidebar(
                div()
                    .id("management-sidebar")
                    .debug_selector(|| "management-sidebar".into())
                    .h_full()
                    .min_h_0()
                    .min_w_0()
                    .w(if narrow {
                        relative(1.)
                    } else {
                        px(super::design::SIDEBAR_WIDTH).into()
                    })
                    .flex_shrink_0()
                    .bg(rgb(palette.sidebar))
                    .border_r_1()
                    .border_color(rgb(palette.border))
                    .child(browser),
            ),
        );
    }
    if !narrow || show_detail {
        row = row.child(
            view.frame_detail(
                div()
                    .id("management-content")
                    .debug_selector(|| "management-content".into())
                    .h_full()
                    .min_h_0()
                    .min_w_0()
                    .flex_1()
                    .child(detail),
            ),
        );
    }
    row
}

pub(super) fn empty_state(
    title: impl Into<gpui::SharedString>,
    description: impl Into<gpui::SharedString>,
    palette: Palette,
) -> Div {
    div()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .p_6()
        .gap_3()
        .child(div().text_lg().child(title.into()))
        .child(
            div()
                .text_sm()
                .text_color(rgb(palette.muted))
                .text_center()
                .child(description.into()),
        )
}

#[cfg(test)]
mod tests {
    use super::{is_narrow, management_footer};
    use gpui::{
        Context, InteractiveElement as _, IntoElement, Render, Styled as _, Window, px, size,
    };

    struct Footer;
    impl Render for Footer {
        fn render(&mut self, window: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
            management_footer(window.viewport_size(), crate::Palette::LIGHT)
                .id("footer")
                .debug_selector(|| "footer".into())
                .w_full()
        }
    }
    #[gpui::test]
    fn footer_follows_window_width_without_consuming_short_content(cx: &mut gpui::TestAppContext) {
        let (_, cx) = cx.add_window_view(|_, _| Footer);
        for (width, expected) in [(960., 72.), (640., 72.), (360., 56.)] {
            cx.simulate_resize(size(px(width), px(540.)));
            cx.run_until_parked();
            let bounds = cx.debug_bounds("footer");
            assert!(bounds.is_some_and(
                |bounds| bounds.size.height == px(expected) && bounds.right() <= px(width)
            ));
            assert_eq!(is_narrow(size(px(width), px(540.))), width < 640.);
        }
    }
}
