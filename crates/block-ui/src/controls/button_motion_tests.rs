//! Observe the shared button's painted fill through real pointer interaction.

use super::{ButtonVariant, ControlState, button, mix};
use crate::appearance::Palette;
use gpui::{
    Context, Entity, Hsla, IntoElement, Modifiers, MouseButton, ParentElement as _, Render, Rgba,
    Styled as _, TestAppContext, VisualTestContext, Window, div, point, px, rgb,
};
use std::time::Duration;

struct Fixture {
    palette: Palette,
    variant: ButtonVariant,
    state: ControlState,
    activations: usize,
    mounted: bool,
    background: Option<Hsla>,
}

impl Render for Fixture {
    fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        div().p_4().children(self.mounted.then(|| {
            let mut control = button(
                "hover-button",
                "Action",
                self.palette,
                self.variant,
                self.state,
                cx.listener(|view, &(), _, _| view.activations += 1),
            );
            if let Some(background) = self.background {
                control = control.bg(background);
            }
            control
        }))
    }
}

fn fixture(
    cx: &mut TestAppContext,
    palette: Palette,
    variant: ButtonVariant,
) -> (Entity<Fixture>, &mut VisualTestContext) {
    let (view, cx) = crate::test_support::window(cx, |_, _| Fixture {
        palette,
        variant,
        state: ControlState::default(),
        activations: 0,
        mounted: true,
        background: None,
    });
    cx.update(|_, cx| cx.set_reduce_motion(false));
    cx.simulate_mouse_move(point(px(0.), px(0.)), None, Modifiers::none());
    frame(cx, 0);
    (view, cx)
}

fn frame(cx: &mut VisualTestContext, millis: u64) {
    cx.executor().advance_clock(Duration::from_millis(millis));
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

#[expect(
    clippy::expect_used,
    reason = "A pointer target missing from the painted fixture is a test failure."
)]
fn hover(cx: &mut VisualTestContext, inside: bool) {
    let position = if inside {
        crate::test_support::control_bounds(cx, "hover-button")
            .expect("rendered button")
            .center()
    } else {
        point(px(0.), px(0.))
    };
    cx.simulate_mouse_move(position, None, Modifiers::none());
    frame(cx, 0);
}

#[expect(
    clippy::expect_used,
    reason = "Missing button bounds or its painted fill is a test failure."
)]
fn fill(cx: &mut VisualTestContext) -> Rgba {
    let bounds = crate::test_support::control_bounds(cx, "hover-button").expect("rendered button");
    cx.update(|window, _| {
        let scale = window.scale_factor();
        window
            .painted_quads()
            .into_iter()
            .find_map(|quad| {
                ((quad.bounds.origin.x.0 - f32::from(bounds.origin.x) * scale).abs() < 0.5
                    && (quad.bounds.origin.y.0 - f32::from(bounds.origin.y) * scale).abs() < 0.5
                    && (quad.bounds.size.width.0 - f32::from(bounds.size.width) * scale).abs()
                        < 0.5
                    && (quad.bounds.size.height.0 - f32::from(bounds.size.height) * scale).abs()
                        < 0.5)
                    .then(|| quad.background.as_solid().map(|color| color.to_rgb()))
                    .flatten()
            })
            .expect("button painted its background")
    })
}

fn near(actual: Rgba, expected: Rgba) {
    for (channel, wanted) in [
        (actual.r * actual.a, expected.r * expected.a),
        (actual.g * actual.a, expected.g * expected.a),
        (actual.b * actual.a, expected.b * expected.a),
        (actual.a, expected.a),
    ] {
        assert!(
            (channel - wanted).abs() < 0.002,
            "{actual:?} != {expected:?}: {channel} != {wanted}"
        );
    }
}

fn progress(color: Rgba, idle: Rgba, hovered: Rgba) -> f32 {
    let delta = [hovered.r - idle.r, hovered.g - idle.g, hovered.b - idle.b];
    let actual = [color.r - idle.r, color.g - idle.g, color.b - idle.b];
    actual.iter().zip(delta).map(|(a, d)| a * d).sum::<f32>()
        / delta.iter().map(|d| d * d).sum::<f32>()
}

#[gpui::test]
fn button_hover_eases_early_and_reverses_from_its_painted_fill(cx: &mut TestAppContext) {
    for palette in [Palette::LIGHT, Palette::DARK] {
        for (variant, idle, hovered) in [
            (
                ButtonVariant::Primary,
                palette.primary,
                mix(palette.primary, palette.background, 80),
            ),
            (
                ButtonVariant::Secondary,
                palette.secondary,
                mix(palette.secondary, palette.foreground, 95),
            ),
            (ButtonVariant::Outline, palette.background, palette.accent),
            (ButtonVariant::Ghost, palette.background, palette.accent),
            (
                ButtonVariant::Destructive,
                palette.destructive_background,
                mix(palette.destructive_background, palette.destructive, 90),
            ),
            (ButtonVariant::Link, palette.background, palette.accent),
        ] {
            let (_, cx) = fixture(cx, palette, variant);
            let (idle, hovered) = (rgb(idle), rgb(hovered));
            near(fill(cx), idle);
            hover(cx, true);
            near(fill(cx), idle);
            frame(cx, 20);
            let entering = fill(cx);
            let p = progress(entering, idle, hovered);
            assert!(p > 0.4 && p < 0.5, "{variant:?}: {p}");
            hover(cx, false);
            near(fill(cx), entering);
            frame(cx, 45);
            near(fill(cx), idle);

            hover(cx, true);
            frame(cx, 20);
            hover(cx, false);
            frame(cx, 10);
            let exiting = fill(cx);
            let p = progress(exiting, idle, hovered);
            assert!(p > 0. && p < 0.4, "{variant:?}: {p}");
            hover(cx, true);
            near(fill(cx), exiting);
            frame(cx, 100);
            near(fill(cx), hovered);
            hover(cx, false);
            near(fill(cx), hovered);
            frame(cx, 50);
            let p = progress(fill(cx), idle, hovered);
            assert!(p > 0.10 && p < 0.15, "{variant:?}: {p}");
            frame(cx, 50);
            near(fill(cx), idle);
        }
    }
}

#[gpui::test]
#[expect(
    clippy::expect_used,
    reason = "The fixture must paint the button before a native press."
)]
fn button_press_disable_and_reduced_motion_settle_without_delaying_activation(
    cx: &mut TestAppContext,
) {
    let palette = Palette::LIGHT;
    let (view, cx) = fixture(cx, palette, ButtonVariant::Ghost);
    hover(cx, true);
    frame(cx, 20);
    let bounds = crate::test_support::control_bounds(cx, "hover-button").expect("rendered button");
    cx.simulate_mouse_down(bounds.center(), MouseButton::Left, Modifiers::none());
    near(
        fill(cx),
        Rgba {
            a: 0.85,
            ..rgb(palette.accent)
        },
    );
    cx.simulate_mouse_up(bounds.center(), MouseButton::Left, Modifiers::none());
    assert_eq!(view.read_with(cx, |view, _| view.activations), 1);

    for state in [
        ControlState {
            disabled: true,
            ..ControlState::default()
        },
        ControlState {
            busy: true,
            ..ControlState::default()
        },
    ] {
        view.update(cx, |view, cx| {
            view.state = state;
            cx.notify();
        });
        frame(cx, 0);
        near(
            fill(cx),
            Rgba {
                a: 0.5,
                ..rgb(palette.background)
            },
        );
        cx.simulate_click(bounds.center(), Modifiers::none());
        assert_eq!(view.read_with(cx, |view, _| view.activations), 1);
    }
    view.update(cx, |view, cx| {
        view.state = ControlState::default();
        cx.notify();
    });
    frame(cx, 0);
    cx.update(|_, cx| cx.set_reduce_motion(true));
    frame(cx, 0);
    near(fill(cx), rgb(palette.accent));
    hover(cx, false);
    near(fill(cx), rgb(palette.background));
    cx.update(|_, cx| cx.set_reduce_motion(false));
    hover(cx, true);
    near(fill(cx), rgb(palette.background));
    frame(cx, 100);
    near(fill(cx), rgb(palette.accent));
}

#[gpui::test]
fn button_hover_preserves_custom_fills_and_discards_unmounted_motion(cx: &mut TestAppContext) {
    let palette = Palette::LIGHT;
    let (view, cx) = fixture(cx, palette, ButtonVariant::Ghost);
    let transparent = gpui::hsla(0., 0., 0., 0.);
    view.update(cx, |view, cx| {
        view.background = Some(transparent);
        cx.notify();
    });
    frame(cx, 0);
    near(fill(cx), transparent.to_rgb());
    hover(cx, true);
    frame(cx, 20);
    let partial = fill(cx);
    assert!(partial.a > 0.4 && partial.a < 0.5, "{partial:?}");
    near(partial, rgb(palette.accent).alpha(partial.a));
    view.update(cx, |view, cx| {
        view.mounted = false;
        cx.notify();
    });
    frame(cx, 0);
    assert!(crate::test_support::control_bounds(cx, "hover-button").is_none());
    hover(cx, false);
    view.update(cx, |view, cx| {
        view.mounted = true;
        cx.notify();
    });
    frame(cx, 0);
    near(fill(cx), transparent.to_rgb());
    hover(cx, true);
    near(fill(cx), transparent.to_rgb());
    frame(cx, 100);
    near(fill(cx), rgb(palette.accent));
}
