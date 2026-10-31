#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod build_info;

use std::{cell::RefCell, path::PathBuf, process::ExitCode, rc::Rc};

use anyhow::Context as _;
use block_launcher::{crash::CrashReport, session_host};
use block_ui::{Block, CrashView, LaunchOptions};
use clap::Parser as _;
use gpui::{App, AppContext as _, KeyBinding, WindowBounds, WindowDecorations, WindowOptions};

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
    let args = Args::parse();
    let informational = args.build_info;
    #[cfg(debug_assertions)]
    let informational = informational || args.list_scenarios;
    let role = if args.supervise.is_some() {
        "supervisor"
    } else if args.crash.is_some() {
        "crash-help"
    } else {
        "launcher"
    };
    let logging = Rc::new(RefCell::new(if informational {
        None
    } else {
        block_logging::init(role)
            .inspect_err(|_| eprintln!("Block: could not initialize local logging."))
            .ok()
    }));
    if !informational {
        tracing::info!(
            event = "application_started",
            role,
            version = env!("CARGO_PKG_VERSION"),
            os = std::env::consts::OS,
            arch = std::env::consts::ARCH,
            debug = cfg!(debug_assertions),
        );
    }
    let result = run(args, logging.clone());
    report_exit(result, &logging)
}

type Logging = Rc<RefCell<Option<block_logging::LogGuard>>>;

fn report_exit(result: anyhow::Result<()>, logging: &Logging) -> ExitCode {
    match result {
        Ok(()) => {
            if logging.borrow().is_some() {
                tracing::info!(event = "application_stopped", failed = false);
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            // Arbitrary error chains can contain paths, URLs or credentials.
            // Detailed game failures stay in the supervisor's redacted reports.
            tracing::error!(
                event = "application_failed",
                error_code = "operation_failed",
                io_kind = ?error.downcast_ref::<std::io::Error>().map(std::io::Error::kind),
                os_error = error.downcast_ref::<std::io::Error>().and_then(std::io::Error::raw_os_error),
            );
            eprintln!("Block could not complete this operation: {error:#}");
            tracing::info!(event = "application_stopped", failed = true);
            ExitCode::FAILURE
        }
    }
}

fn run(args: Args, logging: Logging) -> anyhow::Result<()> {
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
        return crash_application(root, report, logging);
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
            install_logging_shutdown(cx, logging);
            if let Err(error) = block_ui::load_fonts(cx) {
                tracing::warn!(event = "bundled_fonts_unavailable");
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
                    build_info: block_ui::BuildInfo::default(),
                    #[cfg(debug_assertions)]
                    scenario: args.scenario.map(|name| block_ui::ScenarioOptions {
                        name,
                        dark: args.theme.as_deref() == Some("dark"),
                    }),
                },
            ) {
                tracing::error!(event = "window_open_failed", window = "launcher");
                eprintln!("Failed to open the Block window: {error:#}");
                cx.quit();
            }
            cx.activate(true);
        });
    Ok(())
}

fn install_logging_shutdown(cx: &App, logging: Logging) {
    // Native Cocoa termination does not unwind main. GPUI's quit callback is
    // the lifecycle boundary shared by window close, Cmd-Q and OS quit.
    cx.on_app_quit(move |_| {
        let logging = logging.clone();
        async move {
            // GPUI releases windows before polling quit futures, so their
            // final operation/view events reach the writer before it drains.
            tracing::info!(event = "application_stopped");
            drop(logging.borrow_mut().take());
        }
    })
    .detach();
}

fn configure_application(cx: &mut App) {
    block_ui::initialize(cx);
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
    cx.on_window_closed(|cx, _| {
        if cx.windows().is_empty() {
            cx.quit();
        }
    })
    .detach();
}

fn window_options(cx: &App) -> WindowOptions {
    let bounds = block_ui::initial_window_bounds(cx);
    WindowOptions {
        // Product chrome is rendered by Block on every desktop platform.
        // Linux compositors may ignore client-side decoration requests; the
        // in-app controls remain available in that case as well.
        titlebar: None,
        window_decorations: Some(WindowDecorations::Client),
        app_owns_titlebar_drag: true,
        is_resizable: false,
        is_minimizable: true,
        app_id: Some("one.ztd.block".into()),
        window_bounds: Some(WindowBounds::Windowed(bounds)),
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
    let window_options = window_options(cx);
    cx.open_window(window_options, |window, cx| {
        window.set_window_title("Block");
        let view = cx.new(|cx| Block::new(window, cx, options));
        cx.new(|cx| block_ui::ProductRoot::new(view, window, cx))
    })?;
    tracing::info!(event = "window_opened", window = "launcher");
    Ok(())
}

fn crash_application(root: PathBuf, report: CrashReport, logging: Logging) -> anyhow::Result<()> {
    let directory = session_host::report_directory(&root, &report.context.run_id)?;
    let run_id = report.context.run_id.clone();
    let profile = report.context.profile_id.clone();
    gpui_platform::application()
        .with_assets(block_ui::UiAssets)
        .run(move |cx: &mut App| {
            install_logging_shutdown(cx, logging);
            if let Err(error) = block_ui::load_fonts(cx) {
                tracing::warn!(event = "bundled_fonts_unavailable");
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
                    Err(error) => {
                        tracing::error!(event = "window_open_failed", window = "return_to_profile");
                        eprintln!("Could not return to the instance: {error:#}");
                    }
                }
            };
            let options = window_options(cx);
            match cx.open_window(options, |window, cx| {
                window.set_window_title("Block · Crash help");
                let view =
                    cx.new(|cx| CrashView::new(report, directory, return_to_profile, window, cx));
                cx.new(|cx| block_ui::ProductRoot::new(view, window, cx))
            }) {
                Ok(_) => {
                    tracing::info!(event = "window_opened", window = "crash-help");
                    let _ = session_host::mark_seen(&root, &run_id);
                }
                Err(error) => {
                    tracing::error!(event = "window_open_failed", window = "crash-help");
                    eprintln!("Could not open crash help: {error:#}");
                    cx.quit();
                }
            }
            cx.activate(true);
        });
    Ok(())
}
