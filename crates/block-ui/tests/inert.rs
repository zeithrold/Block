//! Retained exit layers keep painting without retaining live interaction. These
//! fake-platform checks exercise the same frame registrations used by `AppKit`;
//! the separate native AX gate still inspects real operating-system objects.

use std::{
    cell::{Cell, RefCell},
    ops::Range,
    rc::Rc,
};

use gpui::{
    App, AppContext as _, Bounds, Context, Entity, FocusHandle, InputHandler,
    InteractiveElement as _, IntoElement, KeyDownEvent, Modifiers, ParentElement as _, Pixels,
    Point, Render, StatefulInteractiveElement as _, Styled as _, UTF16Selection, Window,
    accesskit::{Action, ActionRequest, Node, NodeId, Role, TreeId},
    canvas, deferred, div, px,
};

#[derive(Clone, Debug, Default, PartialEq, Eq, gpui::Action)]
#[action(namespace = inert_regression)]
struct Activate;

#[derive(Default)]
struct Events {
    mouse: Cell<usize>,
    keys: Cell<usize>,
    actions: Cell<usize>,
    accessibility: Cell<usize>,
    text: RefCell<String>,
}

struct TextSink(Rc<Events>);

impl InputHandler for TextSink {
    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut App,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: 0..0,
            reversed: false,
        })
    }
    fn marked_text_range(&mut self, _: &mut Window, _: &mut App) -> Option<Range<usize>> {
        None
    }
    fn text_for_range(
        &mut self,
        _: Range<usize>,
        _: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut App,
    ) -> Option<String> {
        Some(self.0.text.borrow().clone())
    }
    fn replace_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        _: &mut App,
    ) {
        self.0.text.borrow_mut().push_str(text);
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: Option<Range<usize>>,
        _: &mut Window,
        _: &mut App,
    ) {
        self.0.text.borrow_mut().push_str(text);
    }
    fn unmark_text(&mut self, _: &mut Window, _: &mut App) {}
    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        _: &mut Window,
        _: &mut App,
    ) -> Option<Bounds<Pixels>> {
        None
    }
    fn character_index_for_point(
        &mut self,
        _: Point<Pixels>,
        _: &mut Window,
        _: &mut App,
    ) -> Option<usize> {
        None
    }
}

struct CachedControl {
    focus: FocusHandle,
    events: Rc<Events>,
    renders: Rc<Cell<usize>>,
    deferred: bool,
}

impl Render for CachedControl {
    fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
        self.renders.set(self.renders.get() + 1);
        let events = self.events.clone();
        let mut semantic = Node::new(Role::Button);
        semantic.set_label("Retained control");
        semantic.add_action(Action::Click);
        let control = div()
            .id("retained-control")
            .debug_selector(|| "retained-control".into())
            .w(px(180.))
            .h(px(40.))
            .tab_index(0)
            .track_focus(&self.focus)
            .on_click({
                let events = events.clone();
                move |_, _, _| events.mouse.set(events.mouse.get() + 1)
            })
            .on_key_down({
                let events = events.clone();
                move |event: &KeyDownEvent, _, cx| {
                    if event.keystroke.key == "enter" {
                        events.keys.set(events.keys.get() + 1);
                        cx.stop_propagation();
                    }
                }
            })
            .on_action({
                let events = events.clone();
                move |_: &Activate, _, _| events.actions.set(events.actions.get() + 1)
            })
            .accessibility(semantic, false, {
                let events = events.clone();
                move |_, _, _| events.accessibility.set(events.accessibility.get() + 1)
            })
            .child(
                canvas(
                    {
                        let focus = self.focus.clone();
                        move |_, window, cx| {
                            focus.focus(window, cx);
                        }
                    },
                    {
                        let focus = self.focus.clone();
                        move |_, (), window, cx| {
                            window.handle_input(&focus, TextSink(events.clone()), cx);
                        }
                    },
                )
                .size_full(),
            )
            .child("Retained control");
        if self.deferred {
            deferred(control).into_any_element()
        } else {
            control.into_any_element()
        }
    }
}

struct Harness {
    inert: bool,
    child: Entity<CachedControl>,
    live_focus: FocusHandle,
}

impl Render for Harness {
    fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
        div()
            .id("fixture")
            .flex()
            .flex_col()
            .child(div().id("retained-parent").inert(self.inert).child(
                gpui::AnyView::from(self.child.clone()).cached(gpui::StyleRefinement::default()),
            ))
            .child(
                div()
                    .id("live-control")
                    .tab_index(0)
                    .track_focus(&self.live_focus)
                    .w(px(180.))
                    .h(px(40.))
                    .child("Current control"),
            )
    }
}

fn node(cx: &mut gpui::VisualTestContext) -> Option<NodeId> {
    cx.update(|window, _| {
        window
            .accessibility_tree()
            .nodes
            .iter()
            .find(|(_, node)| node.label() == Some("Retained control"))
            .map(|(id, _)| *id)
    })
}

fn press(cx: &mut gpui::VisualTestContext, id: NodeId) {
    cx.update(|window, cx| {
        window.simulate_accessibility_action(
            ActionRequest {
                action: Action::Click,
                target_tree: TreeId::ROOT,
                target_node: id,
                data: None,
            },
            cx,
        );
    });
}

#[gpui::test]
fn cached_inert_subtree_drops_mouse_keys_actions_input_and_tab(cx: &mut gpui::TestAppContext) {
    exercise_inert(cx, false, false);
}

#[gpui::test]
fn deferred_draw_keeps_inherited_inert_policy_during_cached_replay(cx: &mut gpui::TestAppContext) {
    exercise_inert(cx, true, false);
}

#[gpui::test]
fn active_accessibility_inert_subtree_rejects_stale_actions_and_restores_identity(
    cx: &mut gpui::TestAppContext,
) {
    exercise_inert(cx, false, true);
}

#[gpui::test]
fn active_accessibility_deferred_inert_subtree_rejects_stale_actions(
    cx: &mut gpui::TestAppContext,
) {
    exercise_inert(cx, true, true);
}

fn exercise_inert(cx: &mut gpui::TestAppContext, deferred: bool, accessibility: bool) {
    if !accessibility {
        cx.disable_accessibility();
    }
    let events = Rc::new(Events::default());
    let renders = Rc::new(Cell::new(0));
    let (view, cx) = cx.add_window_view(|_, cx| Harness {
        inert: false,
        live_focus: cx.focus_handle().tab_stop(true),
        child: cx.new(|cx| CachedControl {
            focus: cx.focus_handle().tab_stop(true),
            events: events.clone(),
            renders: renders.clone(),
            deferred,
        }),
    });
    cx.run_until_parked();
    let stale = accessibility.then(|| node(cx)).flatten();
    assert_eq!(stale.is_some(), accessibility);
    let bounds = cx.debug_bounds("retained-control");
    assert!(bounds.is_some(), "retained control must actually paint");
    let bounds = bounds.unwrap_or_default();
    let before = renders.get();
    view.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    assert_eq!(
        renders.get(),
        before + usize::from(accessibility),
        "AX-active views recompute; AX-disabled views must actually replay their cache"
    );
    cx.simulate_click(bounds.center(), Modifiers::none());
    cx.simulate_keystrokes("enter");
    cx.dispatch_action(Activate);
    cx.simulate_input("中");
    if let Some(stale) = stale {
        press(cx, stale);
    }
    assert_eq!(events.mouse.get(), 1);
    assert_eq!(events.keys.get(), 1);
    assert_eq!(events.actions.get(), 1);
    assert_eq!(events.accessibility.get(), usize::from(accessibility));
    assert_eq!(&*events.text.borrow(), "中");

    view.update(cx, |view, cx| {
        view.inert = true;
        cx.notify();
    });
    cx.run_until_parked();
    assert!(
        !accessibility || node(cx).is_none(),
        "inert descendants must leave the native semantic submission"
    );
    for _ in 0..2 {
        cx.simulate_click(bounds.center(), Modifiers::none());
        cx.simulate_keystrokes("enter");
        cx.dispatch_action(Activate);
        cx.simulate_input("文");
        if let Some(stale) = stale {
            press(cx, stale);
        }
        view.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
    }
    assert_eq!(events.mouse.get(), 1);
    assert_eq!(events.keys.get(), 1);
    assert_eq!(events.actions.get(), 1);
    assert_eq!(events.accessibility.get(), usize::from(accessibility));
    assert_eq!(
        &*events.text.borrow(),
        "中",
        "inert editor must lose the platform input handler"
    );
    cx.update(|window, cx| {
        window.blur(cx);
        window.focus_next(cx);
    });
    let live_focus = view.read_with(cx, |view, _| view.live_focus.clone());
    cx.update(|window, _| assert!(live_focus.is_focused(window)));

    view.update(cx, |view, cx| {
        view.inert = false;
        cx.notify();
    });
    cx.run_until_parked();
    if accessibility {
        assert_eq!(node(cx), stale, "recovery preserves real semantic identity");
    }
    let before = renders.get();
    view.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    assert_eq!(renders.get(), before + usize::from(accessibility));
    cx.simulate_click(bounds.center(), Modifiers::none());
    cx.simulate_keystrokes("enter");
    cx.dispatch_action(Activate);
    if let Some(id) = accessibility.then(|| node(cx)).flatten() {
        press(cx, id);
    }
    assert_eq!(events.mouse.get(), 2);
    assert_eq!(events.keys.get(), 2);
    assert_eq!(events.actions.get(), 2);
    assert_eq!(events.accessibility.get(), 2 * usize::from(accessibility));
}
