//! Schema/action regressions for the same data submitted to `AppKit`. These are
//! not a substitute for the separate native AX/VoiceOver acceptance gate.
use gpui::accesskit::{Action, ActionRequest, Node, NodeId, Role, TreeId, TreeUpdate};
use gpui::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, Styled as _, Window, div, px,
};
use std::{cell::Cell, rc::Rc};

fn semantic(role: Role, label: &'static str) -> Node {
    let mut node = Node::new(role);
    node.set_label(label);
    node
}
fn named(tree: &TreeUpdate, label: &str) -> NodeId {
    let id = tree
        .nodes
        .iter()
        .find(|(_, node)| node.label() == Some(label))
        .map(|(id, _)| *id);
    assert!(id.is_some(), "missing semantic node {label}");
    id.unwrap_or(NodeId(0))
}
const fn request(action: Action, target_node: NodeId) -> ActionRequest {
    ActionRequest {
        action,
        target_tree: TreeId::ROOT,
        target_node,
        data: None,
    }
}
struct Harness {
    modal: bool,
    activations: usize,
}
impl Harness {
    fn control(label: &'static str, cx: &Context<'_, Self>) -> impl IntoElement + use<> {
        let mut node = semantic(Role::Button, label);
        node.add_action(Action::Click);
        div()
            .id("same-control-id")
            .tab_index(0)
            .h(px(32.))
            .child(label)
            .accessibility(
                node,
                false,
                cx.listener(|view, event: &ActionRequest, _, cx| {
                    if event.action == Action::Click {
                        view.activations += 1;
                        cx.notify();
                    }
                }),
            )
    }
}
impl Render for Harness {
    fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        div()
            .id("scope-root")
            .flex()
            .flex_col()
            .w_full()
            .child(
                div()
                    .id("background")
                    .accessibility(semantic(Role::Group, "Background"), false, |_, _, _| {})
                    .child(Self::control("Background control", cx)),
            )
            .children(self.modal.then(|| {
                div()
                    .id("modal")
                    .accessibility(semantic(Role::Dialog, "Confirmation"), false, |_, _, _| {})
                    .accessibility_modal(true)
                    .child(Self::control("Modal control", cx))
            }))
    }
}
#[gpui::test]
fn modal_tree_excludes_background_and_stale_actions(cx: &mut gpui::TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, _| Harness {
        modal: false,
        activations: 0,
    });
    cx.run_until_parked();
    let background =
        cx.update(|window, _| named(&window.accessibility_tree(), "Background control"));
    view.update(cx, |view, cx| {
        view.modal = true;
        cx.notify();
    });
    cx.run_until_parked();
    let modal = cx.update(|window, cx| {
        let tree = window.accessibility_tree();
        let modal = named(&tree, "Modal control");
        assert_ne!(background, modal, "same local id must remain scoped");
        assert!(!tree.nodes.iter().any(|(id, _)| *id == background));
        window.simulate_accessibility_action(request(Action::Click, background), cx);
        window.simulate_accessibility_action(request(Action::Focus, modal), cx);
        modal
    });
    cx.run_until_parked();
    assert_eq!(
        cx.update(|window, _| window.accessibility_tree().focus),
        modal
    );
    cx.update(|window, cx| window.simulate_accessibility_action(request(Action::Click, modal), cx));
    assert_eq!(view.read_with(cx, |view, _| view.activations), 1);
    view.update(cx, |view, cx| {
        view.modal = false;
        cx.notify();
    });
    cx.run_until_parked();
    cx.update(|window, cx| window.simulate_accessibility_action(request(Action::Click, modal), cx));
    assert_eq!(view.read_with(cx, |view, _| view.activations), 1);
}
struct Cached {
    renders: Rc<Cell<usize>>,
}
impl Render for Cached {
    fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
        self.renders.set(self.renders.get() + 1);
        div()
            .id("cached-control")
            .h(px(32.))
            .child("Cached control")
            .accessibility(
                semantic(Role::Button, "Cached control"),
                false,
                |_, _, _| {},
            )
    }
}
struct CacheHarness {
    child: Entity<Cached>,
    hidden: bool,
}
impl Render for CacheHarness {
    fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
        let parent = div()
            .id("cache-parent")
            .accessibility(semantic(Role::Group, "Cached group"), false, |_, _, _| {})
            .child(
                gpui::AnyView::from(self.child.clone()).cached(gpui::StyleRefinement::default()),
            );
        if self.hidden {
            parent.invisible()
        } else {
            parent
        }
    }
}
#[gpui::test]
fn accessibility_recomputes_cached_views_with_stable_identity_and_hides_descendants(
    cx: &mut gpui::TestAppContext,
) {
    let renders = Rc::new(Cell::new(0));
    let (view, cx) = cx.add_window_view(|_, cx| CacheHarness {
        hidden: false,
        child: cx.new(|_| Cached {
            renders: renders.clone(),
        }),
    });
    cx.run_until_parked();
    let before = cx.update(|window, _| named(&window.accessibility_tree(), "Cached control"));
    let count = renders.get();
    view.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    assert_eq!(
        renders.get(),
        count + 1,
        "the official tree must recompute an AX-active view instead of replaying stale semantics"
    );
    let after = cx.update(|window, _| window.accessibility_tree());
    assert_eq!(named(&after, "Cached control"), before);
    let group = named(&after, "Cached group");
    assert!(
        after
            .nodes
            .iter()
            .any(|(id, node)| *id == group && node.children().contains(&before))
    );
    view.update(cx, |view, cx| {
        view.hidden = true;
        cx.notify();
    });
    cx.run_until_parked();
    let hidden = cx.update(|window, _| window.accessibility_tree());
    assert!(
        hidden
            .nodes
            .iter()
            .all(|(id, _)| *id != before && *id != group),
        "hidden parents must suppress cached descendant semantics"
    );
}
