#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod build_info;

use std::{path::PathBuf, process::ExitCode};

use anyhow::Context as _;
use block_launcher::{crash::CrashReport, session_host};
use block_ui::{Block, CrashView, LaunchOptions};
use clap::Parser as _;
use gpui::{
    App, AppContext as _, Bounds, KeyBinding, TitlebarOptions, WindowBounds, WindowDecorations,
    WindowOptions, px, size,
};

#[derive(Clone, Debug, Default, PartialEq, Eq, gpui::Action)]
#[action(namespace = block)]
struct Quit;

#[derive(Debug, clap::Parser)]
#[command(version, about = "Block Minecraft launcher")]
struct Args {
    #[arg(long)]
    build_info: bool,
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
    #[cfg(debug_assertions)]
    #[arg(long, conflicts_with_all = ["recipe", "launch", "profile", "data_dir", "supervise", "crash", "build_info"], value_parser = clap::builder::PossibleValuesParser::new(block_ui::SCENARIOS.iter().copied()))]
    scenario: Option<String>,
    #[cfg(debug_assertions)]
    #[arg(long, requires = "scenario", value_parser = ["light", "dark"])]
    theme: Option<String>,
    #[cfg(debug_assertions)]
    #[arg(long)]
    list_scenarios: bool,
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
    #[cfg(debug_assertions)]
    if args.list_scenarios {
        println!("{}", block_ui::SCENARIOS.join("\n"));
        return Ok(());
    }
    if args.build_info {
        println!("{}", build_info::current());
        return Ok(());
    }
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
    #[cfg(debug_assertions)]
    let _scenario_directory = args
        .scenario
        .as_ref()
        .map(|_| {
            tempfile::Builder::new()
                .prefix("block-ui-scenario-")
                .tempdir()
        })
        .transpose()?;
    #[cfg(debug_assertions)]
    let args = {
        let mut args = args;
        if let Some(directory) = &_scenario_directory {
            args.data_dir = Some(directory.path().to_owned());
        }
        args
    };
    gpui_platform::application()
        .with_assets(block_ui::UiAssets)
        .run(move |cx: &mut App| {
            if let Err(error) = block_ui::load_fonts(cx) {
                eprintln!("Could not register bundled fonts: {error:#}");
            }
            configure_application(cx);
            if let Err(error) = open_launcher(
                cx,
                LaunchOptions {
                    recipe: args.recipe,
                    launch: args.launch,
                    profile: args.profile,
                    data_dir: args.data_dir,
                    system_window_chrome: false,
                    build_info: block_ui::BuildInfo::default(),
                    #[cfg(debug_assertions)]
                    scenario: args.scenario.map(|name| block_ui::ScenarioOptions {
                        name,
                        dark: args.theme.as_deref() == Some("dark"),
                    }),
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
    block_ui::initialize(cx);
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
    #[cfg(not(target_os = "linux"))]
    cx.on_window_closed(|cx, _| {
        if cx.windows().is_empty() {
            cx.quit();
        }
    })
    .detach();
}

fn window_options(cx: &App, title: &str) -> WindowOptions {
    let bounds = Bounds::centered(None, size(px(960.0), px(600.0)), cx);
    WindowOptions {
        titlebar: Some(TitlebarOptions {
            title: Some(title.to_owned().into()),
            appears_transparent: cfg!(target_os = "macos"),
            ..Default::default()
        }),
        window_decorations: Some(if cfg!(target_os = "macos") {
            WindowDecorations::Client
        } else {
            WindowDecorations::Server
        }),
        is_resizable: false,
        app_id: Some("one.ztd.block".into()),
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_min_size: Some(size(px(360.0), px(540.0))),
        ..Default::default()
    }
}

fn open_launcher(cx: &mut App, mut options: LaunchOptions) -> anyhow::Result<()> {
    options.build_info = block_ui::BuildInfo {
        version: env!("CARGO_PKG_VERSION"),
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        source_sha256: option_env!("BLOCK_SOURCE_SHA256"),
        source_commit: option_env!("BLOCK_SOURCE_COMMIT"),
    };
    let window_options = window_options(cx, "Block");
    options.system_window_chrome = window_options.titlebar.is_some();
    cx.open_window(window_options, |window, cx| {
        let view = cx.new(|cx| Block::new(window, cx, options));
        cx.new(|cx| block_ui::ProductRoot::new(view, window, cx))
    })?;
    Ok(())
}

fn crash_application(root: PathBuf, report: CrashReport) -> anyhow::Result<()> {
    let directory = session_host::report_directory(&root, &report.context.run_id)?;
    let run_id = report.context.run_id.clone();
    let profile = report.context.profile_id.clone();
    gpui_platform::application()
        .with_assets(block_ui::UiAssets)
        .run(move |cx: &mut App| {
            if let Err(error) = block_ui::load_fonts(cx) {
                eprintln!("Could not register bundled fonts: {error:#}");
            }
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
                let view =
                    cx.new(|cx| CrashView::new(report, directory, return_to_profile, window, cx));
                cx.new(|cx| block_ui::ProductRoot::new(view, window, cx))
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
