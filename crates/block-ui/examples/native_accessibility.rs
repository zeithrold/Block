//! Opt-in macOS provider regression. It reads real `NSAccessibility` objects and
//! invokes their native press method. No player data, TCC changes or artifacts.
//! Run explicitly: `BLOCK_NATIVE_AX_REGRESSION=1 cargo run -p block-ui --example native_accessibility`.
#[cfg(all(target_os = "macos", not(test)))]
fn main() {
    if std::env::var_os("CI").is_some() {
        eprintln!(
            "native AppKit provider regression requires a local macOS desktop; CI compiles only"
        );
        std::process::exit(1);
    }
    if std::env::var_os("BLOCK_NATIVE_AX_REGRESSION").as_deref() != Some(std::ffi::OsStr::new("1"))
    {
        println!(
            "native AppKit provider regression not run; set BLOCK_NATIVE_AX_REGRESSION=1 on local macOS"
        );
        return;
    }
    native::run();
}
#[cfg(any(not(target_os = "macos"), test))]
fn main() {
    println!("native AppKit provider regression is an explicit local macOS example");
}

#[cfg(all(target_os = "macos", not(test)))]
mod native {
    #![expect(
        clippy::redundant_pub_crate,
        reason = "The fixture entry point is shared only with its main."
    )]
    use gpui::accesskit::{Action, ActionRequest, Node, Role};
    use gpui::{
        App, AppContext as _, Context, EntityId, InteractiveElement as _, IntoElement,
        ParentElement as _, Render, Styled as _, Window, WindowOptions, div, px,
    };
    use std::{cell::Cell, fmt::Display, rc::Rc, time::Duration};

    struct Harness {
        actions: Rc<Cell<usize>>,
    }
    fn node(role: Role, label: &'static str) -> Node {
        let mut node = Node::new(role);
        node.set_label(label);
        node
    }
    impl Render for Harness {
        fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
            let mut button = node(Role::Button, "Run native action");
            button.add_action(Action::Click);
            let mut input = node(Role::TextInput, "Native text value");
            input.set_value("中文 · Block");
            div()
                .id("native-dialog")
                .size_full()
                .p_4()
                .flex()
                .flex_col()
                .gap_4()
                .accessibility(node(Role::Dialog, "Native regression"), false, |_, _, _| {})
                .accessibility_modal(true)
                .child(
                    div()
                        .id("native-button")
                        .tab_index(0)
                        .h(px(32.))
                        .child("Run native action")
                        .accessibility(
                            button,
                            false,
                            cx.listener(|view, event: &ActionRequest, _, cx| {
                                if event.action == Action::Click {
                                    view.actions.set(view.actions.get() + 1);
                                    cx.notify();
                                }
                            }),
                        ),
                )
                .child(
                    div()
                        .id("native-input")
                        .child("中文 · Block")
                        .accessibility(input, false, |_, _, _| {}),
                )
                .child(div().id("native-status").child("Ready").accessibility(
                    node(Role::Status, "Native status"),
                    false,
                    |_, _, _| {},
                ))
        }
    }
    fn fail(message: impl Display) -> ! {
        eprintln!("native provider regression failed: {message}");
        // AppKit termination may exit before Application::run returns, so a
        // failed callback must explicitly retain a nonzero process exit code.
        std::process::exit(1);
    }

    fn after_draw(
        window: &Window,
        root: EntityId,
        callback: impl FnOnce(&mut Window, &mut App) + 'static,
    ) {
        // The platform executes next-frame callbacks before drawing that frame.
        // The first callback requests the real draw; the second sees its result.
        window.on_next_frame(move |window, cx| {
            println!("actual AppKit frame barrier entered");
            window.on_next_frame(callback);
            cx.notify(root);
        });
    }

    pub(super) fn run() {
        let complete = Rc::new(Cell::new(false));
        gpui_platform::application().run(move |cx| {
            let watchdog = complete.clone();
            cx.spawn(async move |cx| {
                cx.background_executor().timer(Duration::from_secs(5)).await;
                if !watchdog.get() {
                    fail("window did not produce a verified provider frame within five seconds");
                }
            }).detach();
            let actions = Rc::new(Cell::new(0));
            let view_actions = actions.clone();
            let window = cx.open_window(WindowOptions::default(), |_, cx| {
                cx.new(|_| Harness { actions: view_actions })
            }).unwrap_or_else(|error| fail(error));
            window.update(cx, |_, window, cx| {
                // The official adapter initially returns only its bootstrap
                // Window node and activates GPUI for a subsequent tree update.
                let primed = window.native_accessibility_snapshot()
                    .unwrap_or_else(|| fail("native provider unavailable"));
                println!("official provider activation primed: {} objects", primed.len());
                let root = cx.entity_id();
                after_draw(window, root, move |window, _| {
                    let snapshot = window.native_accessibility_snapshot()
                        .unwrap_or_else(|| fail("native provider unavailable"));
                    println!("actual AppKit provider objects: {snapshot:#?}");
                    let roles = snapshot.iter().any(|node| {
                        node.role == "AXButton" && node.label == "Run native action"
                    }) && snapshot.iter().any(|node| {
                        node.role == "AXTextField"
                            && node.label == "Native text value"
                            && node.value.as_deref() == Some("中文 · Block")
                    });
                    let pressed = window.native_accessibility_press("Run native action");
                    after_draw(window, root, move |_, cx| {
                        let count = actions.get();
                        println!("native accessibilityPerformPress={pressed}; GPUI action count={count}; roles_and_value={roles}");
                        if !(roles && pressed && count == 1) {
                            fail("native roles/value/press did not reach the GPUI handler exactly once");
                        }
                        complete.set(true);
                        cx.quit();
                    });
                });
                cx.notify();
            }).unwrap_or_else(|error| fail(error));
            cx.activate(true);
        });
    }
}
