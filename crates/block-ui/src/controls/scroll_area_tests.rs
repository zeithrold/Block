use super::*;
use gpui::{
    Context, Entity, MouseButton, Render, TestAppContext, VisualTestContext, div, point, size,
};

struct FocusFixture {
    focus: FocusHandle,
    next: FocusHandle,
    scroll: ScrollHandle,
    palette: Palette,
    explicit_focus: bool,
}

impl Render for FocusFixture {
    fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
        let viewport = div()
            .id("transparent-viewport")
            .debug_selector(|| "transparent-viewport".into())
            .size(px(200.))
            .flex()
            .flex_col()
            .scroll_area(&self.scroll, "Transparent viewport", self.palette, false)
            .child(div().w_full().h(px(800.)).flex_shrink_0());
        let viewport = if self.explicit_focus {
            viewport.track_focus(&self.focus)
        } else {
            viewport
        };
        div()
            .size_full()
            .bg(rgb(self.palette.background))
            .p_4()
            .flex()
            .flex_col()
            .gap_4()
            .child(viewport)
            .child(
                gpui_base::Button::new("next")
                    .track_focus(&self.next)
                    .size(px(30.)),
            )
    }
}

fn fixture(cx: &mut TestAppContext) -> (Entity<FocusFixture>, &mut VisualTestContext) {
    let (view, cx) = crate::test_support::window(cx, |_, cx| FocusFixture {
        focus: cx.focus_handle().tab_stop(true),
        next: cx.focus_handle().tab_stop(true),
        scroll: ScrollHandle::new(),
        palette: Palette::LIGHT,
        explicit_focus: true,
    });
    cx.simulate_resize(size(px(400.), px(400.)));
    cx.run_until_parked();
    (view, cx)
}

fn viewport_bounds(cx: &mut VisualTestContext) -> Bounds<Pixels> {
    let bounds = crate::test_support::control_bounds(cx, "transparent-viewport");
    assert!(bounds.is_some());
    bounds.unwrap_or_default()
}

fn viewport_quads(cx: &mut VisualTestContext) -> Vec<gpui::Quad> {
    let bounds = viewport_bounds(cx);
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

#[gpui::test]
fn transparent_viewports_keep_pointer_focus_clear_and_keyboard_focus_at_the_edges(
    cx: &mut TestAppContext,
) {
    let (view, cx) = fixture(cx);
    for palette in [Palette::LIGHT, Palette::DARK] {
        for explicit_focus in [true, false] {
            view.update(cx, |view, cx| {
                view.palette = palette;
                view.explicit_focus = explicit_focus;
                cx.notify();
            });
            cx.run_until_parked();
            let initial = viewport_bounds(cx);
            cx.simulate_click(initial.center(), gpui::Modifiers::none());
            assert!(
                viewport_quads(cx).is_empty(),
                "pointer focus must not add a viewport fill or ring"
            );
            if explicit_focus {
                assert_eq!(
                    cx.update(|window, app| window.focused(app)),
                    Some(view.read_with(cx, |view, _| view.focus.clone()))
                );
            }
            let focused = cx.update(|window, app| window.focused(app));
            assert!(focused.is_some());
            crate::test_support::press(cx, "tab");
            assert!(
                viewport_quads(cx).is_empty(),
                "moving focus away removes the viewport outline"
            );
            crate::test_support::press(cx, "shift-tab");
            assert_eq!(cx.update(|window, app| window.focused(app)), focused);
            let quads = viewport_quads(cx);
            assert!(
                !quads.is_empty(),
                "keyboard focus paints a real outline rather than a shadow behind transparent content"
            );
            let scale = cx.update(|window, _| window.scale_factor());
            for quad in &quads {
                assert!(
                    quad.background.as_solid().is_none_or(|color| color.a == 0.),
                    "outline must leave the viewport interior transparent"
                );
                assert!(
                    !quad.content_mask.bounds.contains(&quad.bounds.center()),
                    "the renderer must not shade the empty center"
                );
                for width in [
                    quad.border_widths.top,
                    quad.border_widths.right,
                    quad.border_widths.bottom,
                    quad.border_widths.left,
                ] {
                    assert!((width.0 - 2. * scale).abs() < 0.01);
                }
                assert!((quad.border_color.a - 0.5).abs() < 0.01);
            }
            assert_eq!(viewport_bounds(cx), initial, "focus must not change layout");

            crate::test_support::press(cx, "pagedown");
            assert!(
                view.read_with(cx, |view, _| view.scroll.offset().y < px(0.)),
                "keyboard scrolling stays attached to the real viewport"
            );
            crate::test_support::press(cx, "home");
            assert_eq!(view.read_with(cx, |view, _| view.scroll.offset().y), px(0.));
            cx.simulate_mouse_move(point(px(20.), px(20.)), None, gpui::Modifiers::none());
            assert!(
                viewport_quads(cx).is_empty(),
                "pointer input removes keyboard-only feedback"
            );
            cx.simulate_mouse_down(initial.center(), MouseButton::Left, gpui::Modifiers::none());
            assert!(viewport_quads(cx).is_empty());
            cx.simulate_mouse_up(initial.center(), MouseButton::Left, gpui::Modifiers::none());
        }
    }
}
