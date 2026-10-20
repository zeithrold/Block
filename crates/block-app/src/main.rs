use block_ui::Block;
use gpui::{
    App, AppContext as _, Application, Bounds, KeyBinding, TitlebarOptions, WindowBounds,
    WindowOptions, px, size,
};

#[derive(Clone, Debug, Default, PartialEq, Eq, gpui::Action)]
#[action(namespace = block)]
struct Quit;

fn main() {
    Application::new().run(|cx: &mut App| {
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        let bounds = Bounds::centered(None, size(px(720.0), px(480.0)), cx);
        let window = cx.open_window(
            WindowOptions {
                titlebar: Some(TitlebarOptions {
                    title: Some("Block".into()),
                    ..Default::default()
                }),
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(|_| Block),
        );

        if let Err(error) = window {
            eprintln!("Failed to open the Block window: {error:#}");
            cx.quit();
            return;
        }

        cx.activate(true);
    });
}
