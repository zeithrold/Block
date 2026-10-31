//! Local structured diagnostics shared by the desktop and CLI entry points.
//!
//! Only entry points install the global subscriber. The returned guard must
//! live until shutdown so the background writer can drain its accepted events.

mod export;
mod files;
mod worker;

use std::{path::PathBuf, sync::OnceLock};

use anyhow::{Context as _, anyhow};
use tracing::{Level, Metadata};
use tracing_subscriber::{EnvFilter, Layer, Registry, layer::SubscriberExt as _};

pub use export::export_launcher_logs;
pub use worker::LogGuard;

static FAILURE: OnceLock<worker::Failure> = OnceLock::new();

fn failure_state() -> &'static worker::Failure {
    FAILURE.get_or_init(worker::Failure::default)
}

/// Start the process-wide subscriber and its background file writer.
///
/// `role` must be one of `launcher`, `cli`, `supervisor`, or `crash-help`.
/// Events use the `RUST_LOG` filter when present; otherwise Block targets are
/// enabled at INFO and other targets at WARN.
///
/// # Errors
/// Returns a safe diagnostic if the directory, writer, or subscriber cannot be
/// initialized. The same failure is visible through [`failure`].
pub fn init(role: &str) -> anyhow::Result<LogGuard> {
    init_inner(role).map_err(|_| {
        failure_state().report(worker::Problem::Initialization);
        anyhow!("Could not initialize local logging.")
    })
}

fn init_inner(role: &str) -> anyhow::Result<LogGuard> {
    anyhow::ensure!(files::valid_role(role), "invalid logging role");
    let directory = log_directory()?.join("launcher");
    let (writer, guard) = worker::start(
        directory,
        role,
        files::Limits::default(),
        failure_state().clone(),
    )?;
    let subscriber = subscriber(writer, filter(std::env::var("RUST_LOG").ok().as_deref()));
    tracing::subscriber::set_global_default(subscriber)
        .map_err(|_| anyhow!("a global subscriber is already installed"))?;
    Ok(guard)
}

fn subscriber(
    writer: worker::EventWriterFactory,
    filter: Box<dyn Layer<Registry> + Send + Sync>,
) -> impl tracing::Subscriber + Send + Sync {
    tracing_subscriber::registry().with(filter).with(
        tracing_subscriber::fmt::layer()
            .json()
            .with_writer(writer)
            .log_internal_errors(false),
    )
}

fn filter(specification: Option<&str>) -> Box<dyn Layer<Registry> + Send + Sync> {
    let specification = specification.unwrap_or_default().trim();
    if specification.is_empty() {
        Box::new(tracing_subscriber::filter::filter_fn(default_enabled))
    } else {
        Box::new(
            EnvFilter::builder()
                .with_regex(false)
                .parse_lossy(format!("warn,{specification}")),
        )
    }
}

fn default_enabled(metadata: &Metadata<'_>) -> bool {
    const BLOCK_TARGETS: &[&str] = &[
        "block",
        "block_app",
        "block_auth",
        "block_cli",
        "block_download",
        "block_drop",
        "block_launch",
        "block_launcher",
        "block_logging",
        "block_ui",
        "manifest_audit",
    ];
    let target = metadata.target();
    let is_block = BLOCK_TARGETS.iter().any(|name| {
        target == *name
            || target
                .strip_prefix(name)
                .is_some_and(|suffix| suffix.starts_with("::"))
    });
    *metadata.level() <= if is_block { Level::INFO } else { Level::WARN }
}

/// Return Block's system log directory, independent of launcher data directories.
///
/// Launcher diagnostic segments are stored in its `launcher` subdirectory.
///
/// # Errors
/// Returns an error if the operating system cannot provide a base directory.
pub fn log_directory() -> anyhow::Result<PathBuf> {
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    if let Some(state) = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
    {
        return Ok(state.join("block/logs"));
    }
    #[cfg(target_os = "windows")]
    if let Some(local) = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
    {
        return Ok(local.join("Block/logs"));
    }
    let directories =
        directories::BaseDirs::new().context("Cannot determine the system log directory.")?;
    #[cfg(target_os = "macos")]
    let base = directories.home_dir().join("Library/Logs/Block");
    #[cfg(target_os = "windows")]
    let base = directories.data_local_dir().join("Block/logs");
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let base = directories.home_dir().join(".local/state/block/logs");
    Ok(base)
}

/// Return a safe logging failure observed in this process, if any.
///
/// This includes initialization, writer failures, and lost events. The message
/// contains no paths, event fields, or operating-system error details.
pub fn failure() -> Option<String> {
    failure_state().message().map(str::to_owned)
}

#[cfg(test)]
mod tests;
