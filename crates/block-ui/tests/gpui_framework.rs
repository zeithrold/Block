//! GPUI fake-platform capability checks, separate from real Block UI and GPU smoke tests.

use std::{path::Path, time::Duration};

use gpui::{
    Context, FocusHandle, InteractiveElement as _, IntoElement, KeyBinding, Modifiers,
    ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, TestAppContext,
    Window, div, point, px, size,
};

#[derive(Clone, Debug, Default, PartialEq, Eq, gpui::Action)]
#[action(namespace = framework_smoke)]
struct Launch;

#[derive(Debug)]
struct FrameworkProbe {
    focus: FocusHandle,
    launches: usize,
    state: &'static str,
}

impl FrameworkProbe {
    fn new(window: &mut Window, cx: &mut Context<'_, Self>) -> Self {
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        Self {
            focus,
            launches: 0,
            state: "ready",
        }
    }
}

impl Render for FrameworkProbe {
    fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        div()
            .id("framework-probe")
            .size_full()
            .key_context("FrameworkProbe")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &Launch, _, cx| {
                this.launches += 1;
                this.state = "launching";
                cx.notify();
            }))
            .child(
                div()
                    .id("launch-button")
                    .debug_selector(|| "launch-button".into())
                    .w(px(180.0))
                    .h(px(50.0))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.launches += 1;
                        this.state = "launching";
                        cx.notify();
                    }))
                    .child(self.state),
            )
    }
}

#[gpui::test(iterations = 8)]
fn framework_smoke_routes_mouse_keys_actions_and_resizes(cx: &mut TestAppContext) {
    cx.update(|cx| cx.bind_keys([KeyBinding::new("enter", Launch, Some("FrameworkProbe"))]));
    let (view, cx) = cx.add_window_view(FrameworkProbe::new);
    let bounds = cx.debug_bounds("launch-button");
    assert!(
        bounds.is_some(),
        "the painted button must have layout bounds"
    );
    let Some(bounds) = bounds else {
        return;
    };
    assert_eq!(bounds.size, size(px(180.0), px(50.0)));
    cx.simulate_click(bounds.center(), Modifiers::none());
    assert_eq!(view.read_with(cx, |view, _| view.launches), 1);
    assert_eq!(view.read_with(cx, |view, _| view.state), "launching");
    cx.simulate_keystrokes("enter");
    assert_eq!(view.read_with(cx, |view, _| view.launches), 2);
    cx.dispatch_action(Launch);
    assert_eq!(view.read_with(cx, |view, _| view.launches), 3);
    cx.simulate_resize(size(px(800.0), px(600.0)));
    cx.run_until_parked();
    cx.update(|window, _| assert_eq!(window.viewport_size(), size(px(800.0), px(600.0))));
    cx.simulate_click(point(px(500.0), px(300.0)), Modifiers::none());
    assert_eq!(view.read_with(cx, |view, _| view.launches), 3);
}

#[gpui::test]
fn framework_smoke_advances_async_view_timers_without_sleeping(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(FrameworkProbe::new);
    view.update(cx, |_, cx| {
        cx.spawn(async move |view, cx| {
            cx.background_executor()
                .timer(Duration::from_secs(30))
                .await;
            let updated = view.update(cx, |view, cx| {
                view.state = "finished";
                cx.notify();
            });
            assert!(updated.is_ok());
        })
        .detach();
    });
    cx.run_until_parked();
    assert_eq!(view.read_with(cx, |view, _| view.state), "ready");
    cx.executor().advance_clock(Duration::from_secs(30));
    cx.run_until_parked();
    assert_eq!(view.read_with(cx, |view, _| view.state), "finished");
}

#[gpui::test]
fn framework_smoke_supplies_a_simulated_save_path(cx: &TestAppContext) {
    let selection = cx.update(|cx| cx.prompt_for_new_path(Path::new("probe"), Some("recipe.json")));
    assert!(cx.did_prompt_for_new_path());
    cx.simulate_new_path_selection(|base| Some(base.join("chosen.json")));
    assert!(
        matches!(cx.foreground_executor().block_test(selection), Ok(Ok(Some(path))) if path == Path::new("probe").join("chosen.json"))
    );
}
