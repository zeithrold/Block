use anyhow::Context as _;
use gpui::{
    Bounds, Context, Entity, FocusHandle, InteractiveElement as _, IntoElement, ParentElement as _,
    Pixels, Render, StatefulInteractiveElement as _, Styled as _, TestAppContext,
    VisualTestContext, Window, div, px, size,
};

use crate::{
    appearance::Palette,
    components::{self, ButtonVariant, ControlState},
};

struct Fixture {
    expanded: bool,
    reduced: bool,
    mounted: bool,
    width: f32,
    text: String,
    activations: usize,
    adjustments: usize,
    choice_focus: FocusHandle,
    action_focus: FocusHandle,
}

impl Render for Fixture {
    fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        if !self.mounted {
            return div().p_4();
        }
        let choice = components::checkbox_row(
            "reveal-choice",
            "",
            self.expanded,
            Palette::LIGHT,
            ControlState::default(),
            cx.listener(|view, (): &(), _, cx| {
                view.expanded = !view.expanded;
                view.activations += 1;
                cx.notify();
            }),
        )
        .aria_label("Choice")
        .track_focus(&self.choice_focus);
        let action = components::button(
            "reveal-adjust",
            "Adjust",
            Palette::LIGHT,
            ButtonVariant::Ghost,
            ControlState::default(),
            cx.listener(|view, (): &(), _, cx| {
                view.adjustments += 1;
                cx.notify();
            }),
        )
        .track_focus(&self.action_focus)
        .block_mouse_except_scroll()
        .w(px(56.))
        .h(px(24.))
        .text_xs();
        let content = div()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .debug_selector(|| "reveal-title".into())
                            .text_size(px(14.))
                            .line_height(px(20.))
                            .child("Choice"),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .min_h(px(24.))
                            .child(div().flex_1().text_xs().child("Version · Isolated"))
                            .child(action),
                    ),
            )
            .child(super::reveal(
                "reveal-risk",
                self.expanded.then(|| self.text.clone()),
                Palette::LIGHT.warning,
                self.reduced,
            ));
        div().p_4().child(
            div()
                .w(px(self.width))
                .flex()
                .flex_col()
                .child(
                    components::selection_card(choice, content)
                        .debug_selector(|| "reveal-card".into()),
                )
                .child(
                    div()
                        .debug_selector(|| "reveal-reference".into())
                        .w(px(self.width - 60.))
                        .pt(px(8.))
                        .text_size(px(12.))
                        .line_height(px(18.))
                        .font_weight(gpui::FontWeight::NORMAL)
                        .whitespace_normal()
                        .child(self.text.clone()),
                ),
        )
    }
}

fn fixture(
    cx: &mut TestAppContext,
    expanded: bool,
    test: impl FnOnce(&Entity<Fixture>, &mut VisualTestContext) -> anyhow::Result<()>,
) {
    let (view, cx) = crate::test_support::window(cx, |_, cx| Fixture {
        expanded,
        reduced: false,
        mounted: true,
        width: 320.,
        text: "暂用隔离，原有数据可能不可见。".into(),
        activations: 0,
        adjustments: 0,
        choice_focus: cx.focus_handle(),
        action_focus: cx.focus_handle(),
    });
    cx.update(|_, cx| cx.set_reduce_motion(false));
    cx.run_until_parked();
    let result = test(&view, cx);
    assert!(result.is_ok(), "{result:#?}");
}

fn bounds(cx: &mut VisualTestContext, id: &'static str) -> anyhow::Result<Bounds<Pixels>> {
    cx.debug_bounds(id)
        .with_context(|| format!("missing painted bounds for {id}"))
}

fn height(cx: &mut VisualTestContext) -> anyhow::Result<f32> {
    Ok(f32::from(bounds(cx, "reveal-risk")?.size.height))
}

fn assert_center(cx: &mut VisualTestContext) -> anyhow::Result<()> {
    let card = bounds(cx, "reveal-card")?;
    let selection = crate::test_support::control_bounds(cx, "reveal-choice")
        .context("missing native choice")?;
    let indicator = bounds(cx, "reveal-choice-indicator")?;
    let scale = cx.update(|window, _| window.scale_factor());
    assert_eq!(selection, card);
    assert_eq!(indicator.size, size(px(16.), px(16.)));
    assert_eq!(card.right() - indicator.right(), px(16.));
    assert!(
        (f32::from(card.center().y) - f32::from(indicator.center().y)).abs() <= 0.5 / scale,
        "card {card:?}, indicator {indicator:?}"
    );
    Ok(())
}

fn refresh(
    view: &Entity<Fixture>,
    cx: &mut VisualTestContext,
    change: impl FnOnce(&mut Fixture),
) -> anyhow::Result<f32> {
    view.update(cx, |view, cx| {
        change(view);
        cx.notify();
    });
    cx.run_until_parked();
    assert_center(cx)?;
    height(cx)
}

#[gpui::test]
fn first_frame_and_reduced_motion_match_native_wrapped_text(cx: &mut TestAppContext) {
    fixture(cx, true, |view, cx| {
        assert_center(cx)?;
        assert_eq!(
            height(cx)?,
            f32::from(bounds(cx, "reveal-reference")?.size.height)
        );
        assert!(height(cx)? > 0.);
        for width in [248., 320.] {
            for text in [
                "暂用隔离，原有数据可能不可见。",
                "Using isolated data; existing data may not appear.",
                "隔離を仮選択しています。既存データが見えない場合があります。",
            ] {
                let actual = refresh(view, cx, |view| {
                    view.width = width;
                    view.text = text.into();
                    view.reduced = true;
                })?;
                assert_eq!(
                    actual,
                    f32::from(bounds(cx, "reveal-reference")?.size.height)
                );
            }
        }
        Ok(())
    });
}

#[gpui::test]
fn reveal_retargets_pixels_and_remounts_at_the_current_geometry(cx: &mut TestAppContext) {
    fixture(cx, false, |view, cx| {
        assert_eq!(height(cx)?, 0.);
        assert_eq!(refresh(view, cx, |view| view.expanded = true)?, 0.);
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(70));
        let entering = refresh(view, cx, |_| {})?;
        assert!(entering > 0. && entering < 26., "{entering}");
        let reversed = refresh(view, cx, |view| view.expanded = false)?;
        assert!((entering - reversed).abs() < 0.02);
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(20));
        let sampled = refresh(view, cx, |_| {})?;
        let resumed = refresh(view, cx, |view| {
            view.expanded = true;
            view.width = 248.;
            view.text = "隔離を仮選択しています。既存データが見えない場合があります。".into();
        })?;
        assert!((sampled - resumed).abs() < 0.02);
        let final_height = refresh(view, cx, |view| view.reduced = true)?;
        assert_eq!(
            final_height,
            f32::from(bounds(cx, "reveal-reference")?.size.height)
        );
        view.update(cx, |view, cx| {
            view.mounted = false;
            cx.notify();
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("reveal-risk").is_none());
        let remounted = refresh(view, cx, |view| {
            view.mounted = true;
            view.reduced = false;
        })?;
        assert_eq!(remounted, final_height);
        Ok(())
    });
}

#[gpui::test]
fn selection_and_adjust_have_independent_pointer_and_keyboard_activation(cx: &mut TestAppContext) {
    fixture(cx, true, |view, cx| {
        let click = |cx: &mut VisualTestContext, position| {
            cx.simulate_click(position, gpui::Modifiers::none());
            cx.run_until_parked();
        };
        let title = bounds(cx, "reveal-title")?.center();
        click(cx, title);
        assert_eq!(view.read_with(cx, |view, _| view.activations), 1);
        refresh(view, cx, |view| view.reduced = true)?;
        let indicator = bounds(cx, "reveal-choice-indicator")?.center();
        click(cx, indicator);
        let risk = bounds(cx, "reveal-risk")?.center();
        click(cx, risk);
        assert_eq!(view.read_with(cx, |view, _| view.activations), 3);
        let action = crate::test_support::control_bounds(cx, "reveal-adjust")
            .context("missing Adjust action")?;
        click(cx, action.center());
        assert_eq!(
            view.read_with(cx, |view, _| (view.activations, view.adjustments)),
            (3, 1)
        );
        cx.update(|window, cx| {
            let focus = view.read(cx).choice_focus.clone();
            focus.focus(window, cx);
        });
        crate::test_support::press(cx, "tab");
        assert!(cx.update(|window, cx| view.read(cx).action_focus.is_focused(window)));
        crate::test_support::press(cx, "space");
        assert_eq!(
            view.read_with(cx, |view, _| (view.activations, view.adjustments)),
            (3, 2)
        );
        crate::test_support::press(cx, "shift-tab");
        crate::test_support::press(cx, "space");
        assert_eq!(view.read_with(cx, |view, _| view.activations), 4);
        Ok(())
    });
}
