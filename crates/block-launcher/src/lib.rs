//! Recipe parsers compile immutable preparation plans and complete Java commands.

pub mod azul;
pub mod catalog;
pub mod command;
pub mod crash;
pub mod executor;
pub mod install;
pub mod launch;
pub mod library;
pub mod plan;
pub mod platform;
pub mod recipe;
pub mod repair;
pub mod resolver;
pub mod rules;
pub mod runtime;
pub mod session;
pub mod session_host;
mod session_ipc;
pub mod storage;
pub mod support;

pub use block_download::{DownloadEvent, DownloadOptions, EventSink, ProxyMode, Resource, Store};
pub use tokio_util::sync::CancellationToken;
