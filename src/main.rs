use gpui::{
    App, Application, Bounds, Context, Window, WindowBounds, WindowOptions, actions, div,
    prelude::*, px, rgb, size,
};

actions!(block, [Quit]);

struct Block;

impl Render for Block {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .size_full()
            .items_center()
            .justify_center()
            .gap_3()
            .bg(rgb(0x18181b))
            .text_color(rgb(0xf4f4f5))
            .child(div().text_3xl().child("Hello, GPUI!"))
            .child(
                div()
                    .text_base()
                    .text_color(rgb(0xa1a1aa))
                    .child("Block · Built with Rust and GPUI"),
            )
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([gpui::KeyBinding::new("cmd-q", Quit, None)]);
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        let bounds = Bounds::centered(None, size(px(720.0), px(480.0)), cx);
        cx.open_window(
            WindowOptions {
                titlebar: Some(gpui::TitlebarOptions {
                    title: Some("Block".into()),
                    ..Default::default()
                }),
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(|_| Block),
        )
        .expect("failed to open the Block window");

        cx.activate(true);
    });
}
