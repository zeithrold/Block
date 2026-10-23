//! Disposable native chrome capability probe; not the launcher's Home design.
//!
//! Run with `--system` for a system-titlebar baseline, or without it for client
//! decorations. Add `--fixed-controls` to test a header without maximize or
//! double-click zoom. It uses the same published GPUI/component versions as Block.

use gpui::{
    App, AppContext as _, Bounds, Context, InteractiveElement as _, IntoElement, MouseButton,
    ParentElement as _, Render, Styled as _, TitlebarOptions, Window, WindowBounds,
    WindowControlArea, WindowDecorations, WindowOptions, div, px, rgb, size,
};
use gpui_component::{
    TitleBar, WindowExt as _,
    button::{Button, ButtonVariants as _},
};

struct Probe {
    clicks: usize,
    fixed_controls: bool,
    dragging: bool,
}

impl Render for Probe {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let decorations = format!("{:?}", window.window_decorations());
        let chrome = if self.fixed_controls {
            div()
                .flex()
                .h(px(34.))
                .flex_shrink_0()
                .items_center()
                .child(
                    div()
                        .id("probe-drag-region")
                        .flex_1()
                        .h_full()
                        .window_control_area(WindowControlArea::Drag)
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, _, _| this.dragging = true),
                        )
                        .on_mouse_up(
                            MouseButton::Left,
                            cx.listener(|this, _, _, _| this.dragging = false),
                        )
                        .on_mouse_move(cx.listener(|this, _, window, _| {
                            if this.dragging {
                                this.dragging = false;
                                window.start_window_move();
                            }
                        }))
                        .child("Block fixed chrome probe — drag here"),
                )
                .child(
                    Button::new("probe-minimize")
                        .label("−")
                        .w(px(34.))
                        .on_click(|_, window, _| window.minimize_window()),
                )
                .child(
                    Button::new("probe-close")
                        .label("×")
                        .w(px(34.))
                        .on_click(|_, window, _| window.remove_window()),
                )
                .into_any_element()
        } else {
            TitleBar::new()
                .child("Block chrome probe — drag here")
                .into_any_element()
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x171b24))
            .text_color(rgb(0xffffff))
            .child(chrome)
            .child(
                div()
                    .p_6()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child("Fixed client size: 640 × 420. No player data or network.")
                    .child(format!("Decorations reported by GPUI: {decorations}"))
                    .child(format!("Content clicks: {}", self.clicks))
                    .child(
                        Button::new("increment")
                            .label("Increment")
                            .primary()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.clicks += 1;
                                eprintln!("PROBE content_click {}", this.clicks);
                                cx.notify();
                            })),
                    )
                    .child(Button::new("overlay").label("Open overlay").on_click(
                        |_, window, cx| {
                            eprintln!("PROBE overlay_open");
                            window.open_dialog(cx, |dialog, _, _| {
                                dialog.title("Chrome probe overlay").child(
                                    "Press Escape; the underlying content must stay blocked.",
                                )
                            });
                        },
                    )),
            )
    }
}

fn main() {
    let system = std::env::args().any(|arg| arg == "--system");
    let fixed_controls = std::env::args().any(|arg| arg == "--fixed-controls");
    gpui_platform::application()
        .with_assets(block_ui::UiAssets)
        .run(move |cx: &mut App| {
            block_ui::initialize(cx);
            if let Err(error) = block_ui::load_fonts(cx) {
                eprintln!("PROBE font_error {error:#}");
            }
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    eprintln!("PROBE window_closed");
                    cx.quit();
                }
            })
            .detach();
            let options = WindowOptions {
                titlebar: Some(TitlebarOptions {
                    title: Some("Block chrome probe".into()),
                    appears_transparent: !system,
                    ..Default::default()
                }),
                window_decorations: Some(if system {
                    WindowDecorations::Server
                } else {
                    WindowDecorations::Client
                }),
                app_owns_titlebar_drag: !system,
                is_resizable: false,
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(640.), px(420.)),
                    cx,
                ))),
                ..Default::default()
            };
            match cx.open_window(options, |window, cx| {
                let view = cx.new(|_| Probe {
                    clicks: 0,
                    fixed_controls,
                    dragging: false,
                });
                cx.new(|cx| block_ui::ProductRoot::new(view, window, cx))
            }) {
                Ok(_) => eprintln!("PROBE opened system={system} fixed_controls={fixed_controls}"),
                Err(error) => {
                    eprintln!("PROBE open_error {error:#}");
                    cx.quit();
                }
            }
            cx.activate(true);
        });
}
