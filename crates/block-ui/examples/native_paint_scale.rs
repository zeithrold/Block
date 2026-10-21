//! Explicit local macOS/CoreText regression; never opens a CI window or reads
//! player data. No default artifact writes. Standard tests only compile it.
#[cfg(all(target_os = "macos", not(test)))]
#[path = "../tests/support/paint_scale_fixture.rs"]
mod fixture;

#[cfg(all(target_os = "macos", not(test)))]
fn main() {
    if std::env::var_os("CI").is_some() {
        eprintln!("native paint scale regression requires a local macOS desktop; CI compiles only");
        std::process::exit(1);
    }
    if std::env::var_os("BLOCK_NATIVE_PAINT_REGRESSION").as_deref()
        != Some(std::ffi::OsStr::new("1"))
    {
        println!("not run; set BLOCK_NATIVE_PAINT_REGRESSION=1 on local macOS");
        return;
    }
    native::run();
}

#[cfg(any(not(target_os = "macos"), test))]
fn main() {
    println!("native paint scale regression is an explicit local macOS example");
}

#[cfg(all(target_os = "macos", not(test)))]
mod native {
    #![expect(
        clippy::redundant_pub_crate,
        reason = "The explicit local entry point is visible only to its parent main."
    )]
    use super::fixture::{Harness, PaintedControl, assert_scaled};
    use gpui::{
        App, AppContext as _, AsyncApp, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent,
        PlatformInput, Point, Window, WindowHandle, WindowOptions,
        accesskit::{Action, ActionRequest, NodeId, TreeId},
        point, px,
    };
    use std::{cell::Cell, rc::Rc, time::Duration};

    fn fail(message: impl std::fmt::Display) -> ! {
        eprintln!("native paint scale regression failed: {message}");
        std::process::exit(1);
    }

    pub(super) fn run() {
        // AppKit has one application lifecycle per process. Fresh child
        // processes exercise the official AX-disabled and AX-active modes.
        match std::env::var("BLOCK_NATIVE_PAINT_PHASE").as_deref() {
            Ok("cache") => run_phase(false),
            Ok("accessibility") => run_phase(true),
            Err(std::env::VarError::NotPresent) => {
                let executable = std::env::current_exe().unwrap_or_else(|error| fail(error));
                for phase in ["cache", "accessibility"] {
                    let status = std::process::Command::new(&executable)
                        .env("BLOCK_NATIVE_PAINT_PHASE", phase)
                        .status()
                        .unwrap_or_else(|error| fail(error));
                    if !status.success() {
                        fail(format!("{phase} phase exited {status}"));
                    }
                }
                println!("both native paint phases passed");
            }
            _ => fail("BLOCK_NATIVE_PAINT_PHASE must be cache or accessibility"),
        }
    }

    /// Await a real completed frame without globally invalidating child caches.
    #[expect(
        clippy::future_not_send,
        reason = "The frame barrier belongs to GPUI's foreground executor and must retain its main-thread AsyncApp."
    )]
    async fn frame(window: WindowHandle<Harness>, cx: &mut AsyncApp) {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        window
            .update(cx, |_, window, cx| {
                let root = cx.entity_id();
                // Next-frame callbacks precede the actual draw on macOS. The second
                // callback observes the draw requested by the first callback.
                window.on_next_frame(move |window, cx| {
                    println!("actual AppKit paint frame barrier entered");
                    window.on_next_frame(move |_, _| {
                        let _ = sender.send(());
                    });
                    cx.notify(root);
                });
                cx.notify();
            })
            .unwrap_or_else(|error| fail(error));
        receiver.await.unwrap_or_else(|error| fail(error));
    }

    fn action_node(window: &Window) -> Option<NodeId> {
        window
            .accessibility_tree()
            .nodes
            .iter()
            .find(|(_, node)| node.label() == Some("Scaled popup action"))
            .map(|(id, _)| *id)
    }

    fn click(window: &mut Window, cx: &mut App, position: Point<gpui::Pixels>) {
        // These events stay in this independent fixture window. No operating
        // system input is sent to another application or product window.
        window.simulate_mouse_move(position, cx);
        window.dispatch_event(
            PlatformInput::MouseDown(MouseDownEvent {
                button: MouseButton::Left,
                position,
                modifiers: Modifiers::none(),
                click_count: 1,
                first_mouse: false,
            }),
            cx,
        );
        window.dispatch_event(
            PlatformInput::MouseUp(MouseUpEvent {
                button: MouseButton::Left,
                position,
                modifiers: Modifiers::none(),
                click_count: 1,
            }),
            cx,
        );
    }

    fn run_phase(accessibility: bool) {
        std::panic::set_hook(Box::new(|panic| fail(panic)));
        let complete = Rc::new(Cell::new(false));
        let app = if accessibility {
            gpui_platform::application()
        } else {
            gpui::Application::new_inaccessible(gpui_platform::current_platform(false))
        };
        app.run(move |cx| {
            let watchdog = complete.clone();
            cx.spawn(async move |cx| {
                cx.background_executor().timer(Duration::from_secs(8)).await;
                if !watchdog.get() { fail("no verified CoreText frame within eight seconds"); }
            }).detach();
            let renders = Rc::new(Cell::new(0));
            let presses = Rc::new(Cell::new(0));
            let backdrop = Rc::new(Cell::new(0));
            let window = cx.open_window(WindowOptions::default(), |window, cx| {
                cx.new(|cx| {
                    let focus = cx.focus_handle().tab_stop(true);
                    focus.focus(window, cx);
                    Harness {
                        factor: 1., nested: false, explicit_inert: false,
                        backdrop: backdrop.clone(),
                        child: cx.new(|_| PaintedControl {
                            renders: renders.clone(), presses: presses.clone(), focus, deferred: true,
                        }),
                    }
                })
            }).unwrap_or_else(|error| fail(error));
            if accessibility {
                window.update(cx, |_, window, _| {
                    let prime = window.native_accessibility_snapshot()
                        .unwrap_or_else(|| fail("native provider unavailable"));
                    println!("official provider activation primed: {} objects", prime.len());
                }).unwrap_or_else(|error| fail(error));
            }
            cx.spawn(async move |cx| {
                frame(window, cx).await;
                let (original, origin, focus, original_node) = window.update(cx, |view, window, cx| {
                    let primitives = window.painted_primitives();
                    let glyphs = primitives.iter().filter(|primitive| matches!(primitive.kind, "monochrome" | "subpixel")).count();
                    let images = primitives.iter().filter(|primitive| primitive.kind == "polychrome").count();
                    if glyphs == 0 || images == 0 { fail("actual CoreText glyph/image primitives missing"); }
                    if window.is_a11y_enabled() != accessibility { fail("official AX phase configuration differs"); }
                    let original_node = accessibility.then(|| action_node(window)).flatten();
                    if accessibility && original_node.is_none() { fail("primed official frame has no control node"); }
                    println!("actual macOS renderer: AX={accessibility}, glyph/svg/subpixel={glyphs}, image/emoji={images}, total={}", primitives.len());
                    (primitives, point(px(150.), px(130.)).scale(window.scale_factor()), view.child.read(cx).focus.clone(), original_node)
                }).unwrap_or_else(|error| fail(error));

                let count = renders.get();
                frame(window, cx).await;
                window.update(cx, |_, window, _| {
                    assert_scaled(&original, &window.painted_primitives(), 1., Point::default());
                    if accessibility {
                        if renders.get() <= count { fail("AX-active unit-scale frame did not recompute semantics"); }
                        if action_node(window) != original_node { fail("semantic recomputation changed the stable node identity"); }
                    } else if renders.get() != count { fail("AX-disabled unit-scale frame did not actually reuse its child cache"); }
                }).unwrap_or_else(|error| fail(error));

                window.update(cx, |view, _, cx| { view.factor = 0.8; cx.notify(); }).unwrap_or_else(|error| fail(error));
                frame(window, cx).await;
                let scaled = window.update(cx, |_, window, cx| {
                    let actual = window.painted_primitives();
                    assert_scaled(&original, &actual, 0.8, origin);
                    if accessibility {
                        if action_node(window).is_some() { fail("scaled subtree leaked accessibility"); }
                        if let Some(id) = original_node {
                            window.simulate_accessibility_action(ActionRequest { action: Action::Click, target_tree: TreeId::ROOT, target_node: id, data: None }, cx);
                        }
                    }
                    actual
                }).unwrap_or_else(|error| fail(error));
                let count = renders.get();
                for _ in 0..3 {
                    frame(window, cx).await;
                    window.update(cx, |_, window, _| assert_scaled(&scaled, &window.painted_primitives(), 1., Point::default())).unwrap_or_else(|error| fail(error));
                }
                if !accessibility && renders.get() != count { fail("AX-disabled native repeated frames did not exercise cached replay"); }
                window.update(cx, |_, window, cx| click(window, cx, point(px(150.), px(130.)))).unwrap_or_else(|error| fail(error));
                if presses.get() != 0 || backdrop.get() != 0 { fail("scaled content received input or lost its passive occlusion"); }

                window.update(cx, |view, _, cx| { view.factor = 1.; cx.notify(); }).unwrap_or_else(|error| fail(error));
                frame(window, cx).await;
                window.update(cx, |_, window, cx| {
                    assert_scaled(&original, &window.painted_primitives(), 1., Point::default());
                    if !focus.is_focused(window) { fail("settling changed the original focus handle"); }
                    if accessibility {
                        if action_node(window) != original_node { fail("settled accessibility identity did not return"); }
                        let provider = window.native_accessibility_snapshot().unwrap_or_else(|| fail("native provider unavailable"));
                        if !provider.iter().any(|node| node.role == "AXButton" && node.label == "Scaled popup action") { fail("settled actual AppKit control did not return"); }
                    }
                    click(window, cx, point(px(150.), px(130.)));
                }).unwrap_or_else(|error| fail(error));
                if presses.get() != 1 { fail("settled hitbox did not activate exactly once"); }
                println!("native AX={accessibility}: actual glyph corners/metrics, repeated geometry, hitbox isolation and focus restoration verified");
                complete.set(true);
                cx.update(|cx| cx.quit());
            }).detach();
            cx.activate(true);
        });
    }
}
