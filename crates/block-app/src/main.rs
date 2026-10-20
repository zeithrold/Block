use block_ui::{Block, LaunchOptions};
use clap::Parser as _;
use gpui::{
    App, AppContext as _, Application, Bounds, KeyBinding, TitlebarOptions, WindowBounds,
    WindowOptions, px, size,
};

#[derive(Clone, Debug, Default, PartialEq, Eq, gpui::Action)]
#[action(namespace = block)]
struct Quit;

#[derive(Debug, clap::Parser)]
#[command(version, about = "Block Minecraft launcher")]
struct Args {
    #[arg(long)]
    recipe: Option<std::path::PathBuf>,
    #[arg(long)]
    launch: bool,
}

fn main() {
    let args = Args::parse();
    Application::new().run(move |cx: &mut App| {
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
        // GPUI's Linux backends stop their loop when the final window is dropped.
        // Calling quit inside their close callback reborrows the active X11 client.
        #[cfg(not(target_os = "linux"))]
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        let bounds = Bounds::centered(None, size(px(1040.0), px(720.0)), cx);
        let window = cx.open_window(
            WindowOptions {
                titlebar: Some(TitlebarOptions {
                    title: Some("Block".into()),
                    ..Default::default()
                }),
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(760.0), px(560.0))),
                ..Default::default()
            },
            |window, cx| {
                cx.new(|cx| {
                    Block::new(
                        window,
                        cx,
                        LaunchOptions {
                            recipe: args.recipe,
                            launch: args.launch,
                        },
                    )
                })
            },
        );

        if let Err(error) = window {
            eprintln!("Failed to open the Block window: {error:#}");
            cx.quit();
            return;
        }

        cx.activate(true);
    });
}
