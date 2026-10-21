use std::{path::PathBuf, process::ExitCode};

use anyhow::Context as _;
use block_launcher::{crash::CrashReport, session_host};
use block_ui::{Block, CrashView, LaunchOptions};
use clap::Parser as _;
use gpui::{
    App, AppContext as _, Application, Bounds, KeyBinding, WindowBounds, WindowDecorations,
    WindowOptions, px, size,
};

#[derive(Clone, Debug, Default, PartialEq, Eq, gpui::Action)]
#[action(namespace = block)]
struct Quit;

#[derive(Debug, clap::Parser)]
#[command(version, about = "Block Minecraft launcher")]
struct Args {
    #[arg(long)]
    recipe: Option<PathBuf>,
    #[arg(long)]
    launch: bool,
    #[arg(long, conflicts_with_all = ["supervise", "crash"])]
    profile: Option<String>,
    #[arg(long)]
    data_dir: Option<PathBuf>,
    #[arg(long, conflicts_with_all = ["recipe", "launch", "crash", "profile"], requires = "data_dir")]
    supervise: Option<String>,
    #[arg(long, conflicts_with_all = ["recipe", "launch", "supervise", "profile"], requires = "data_dir")]
    crash: Option<String>,
}

fn main() -> ExitCode {
    match run(Args::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Block could not complete this operation: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Args) -> anyhow::Result<()> {
    if let Some(id) = &args.supervise {
        let root = args
            .data_dir
            .as_ref()
            .context("session data directory required")?;
        // The detached session owner never initializes GPUI or requests desktop access.
        return tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()?
            .block_on(session_host::run_session_host(root, id));
    }
    if let Some(id) = &args.crash {
        let root = args
            .data_dir
            .context("crash report data directory required")?;
        let Some(_presentation) = session_host::claim_help(&root, id)? else {
            return Ok(());
        };
        let report = session_host::read_crash(&root, id)?;
        return crash_application(root, report);
    }
    Application::new().run(move |cx: &mut App| {
        configure_application(cx);
        if let Err(error) = open_launcher(
            cx,
            LaunchOptions {
                recipe: args.recipe,
                launch: args.launch,
                profile: args.profile,
                data_dir: args.data_dir,
            },
        ) {
            eprintln!("Failed to open the Block window: {error:#}");
            cx.quit();
        }
        cx.activate(true);
    });
    Ok(())
}

fn configure_application(cx: &mut App) {
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
    #[cfg(not(target_os = "linux"))]
    cx.on_window_closed(|cx| {
        if cx.windows().is_empty() {
            cx.quit();
        }
    })
    .detach();
}

fn window_options(cx: &App, _title: &str) -> WindowOptions {
    let bounds = Bounds::centered(None, size(px(960.0), px(600.0)), cx);
    WindowOptions {
        titlebar: None,
        window_decorations: Some(WindowDecorations::Client),
        app_id: Some("one.ztd.block".into()),
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_min_size: Some(size(px(360.0), px(540.0))),
        ..Default::default()
    }
}

fn open_launcher(cx: &mut App, options: LaunchOptions) -> anyhow::Result<()> {
    let window_options = window_options(cx, "Block");
    cx.open_window(window_options, |window, cx| {
        cx.new(|cx| Block::new(window, cx, options))
    })?;
    Ok(())
}

fn crash_application(root: PathBuf, report: CrashReport) -> anyhow::Result<()> {
    let directory = session_host::report_directory(&root, &report.context.run_id)?;
    let run_id = report.context.run_id.clone();
    let profile = report.context.profile_id.clone();
    Application::new().run(move |cx: &mut App| {
        configure_application(cx);
        let return_root = root.clone();
        let return_to_profile = move |window: &mut gpui::Window, cx: &mut App| {
            let options = LaunchOptions {
                profile: Some(profile.clone()),
                data_dir: Some(return_root.clone()),
                ..Default::default()
            };
            match open_launcher(cx, options) {
                Ok(()) => window.remove_window(),
                Err(error) => eprintln!("Could not return to the instance: {error:#}"),
            }
        };
        let options = window_options(cx, "Block · Crash help");
        match cx.open_window(options, |window, cx| {
            cx.new(|cx| CrashView::new(report, directory, return_to_profile, window, cx))
        }) {
            Ok(_) => {
                let _ = session_host::mark_seen(&root, &run_id);
            }
            Err(error) => {
                eprintln!("Could not open crash help: {error:#}");
                cx.quit();
            }
        }
        cx.activate(true);
    });
    Ok(())
}
