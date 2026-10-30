//! Recipe parsers compile immutable preparation plans and complete Java commands.

pub mod avatars;
#[path = "resolution/azul.rs"]
pub mod azul;
pub mod catalog;
#[path = "execution/command.rs"]
pub mod command;
#[path = "persistence/config.rs"]
mod config;
pub mod crash;
#[path = "execution/executor.rs"]
pub mod executor;
pub mod game_import;
pub mod install;
#[path = "execution/launch.rs"]
pub mod launch;
pub mod library;
pub mod manifest;
pub mod plan;
#[path = "resolution/platform.rs"]
pub mod platform;
#[path = "resolution/recipe.rs"]
pub mod recipe;
pub mod repair;
#[path = "resolution/resolver.rs"]
pub mod resolver;
#[path = "resolution/rules.rs"]
pub mod rules;
#[path = "resolution/runtime.rs"]
pub mod runtime;
pub mod session;
pub mod session_host;
mod session_ipc;
#[path = "persistence/storage.rs"]
pub mod storage;
#[path = "resolution/support.rs"]
pub mod support;

pub use block_download::{DownloadEvent, DownloadOptions, EventSink, ProxyMode, Resource, Store};
pub use tokio_util::sync::CancellationToken;

/// Lowest supported maximum memory for a launch Profile: 1 GB.
pub const MIN_MEMORY_MIB: u32 = 1024;
/// Highest supported maximum memory for a launch Profile: 32 GB.
pub const MAX_MEMORY_MIB: u32 = 32768;

/// Clamp a stored or user-entered maximum memory to the supported 1–32 GB range.
#[must_use]
pub fn clamp_memory_mib(value: u32) -> u32 {
    value.clamp(MIN_MEMORY_MIB, MAX_MEMORY_MIB)
}
