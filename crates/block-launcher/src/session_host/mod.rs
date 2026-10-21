//! A detached owner for one game, its directory reservation and local diagnostics.

mod client;
mod runtime;
mod storage;
mod wire;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::crash::{CrashContext, ExitOutcome};

pub use client::{spawn_session, status, stop};
pub use runtime::run_session_host;
pub use storage::{claim_help, list_runs, mark_seen, read_crash, read_run, report_directory};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    Starting,
    Preparing,
    Running,
    Finished,
    MonitoringInterrupted,
}

/// Public, bounded metadata. Authentication is never persisted in this document.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunRecord {
    pub schema_version: u32,
    pub context: CrashContext,
    pub instance_id: String,
    pub game_directory: PathBuf,
    pub supervisor_pid: Option<u32>,
    pub java_pid: Option<u32>,
    pub state: RunState,
    pub outcome: Option<ExitOutcome>,
    pub status: String,
    pub revision: u64,
    pub recent_output: Vec<String>,
    pub capture_warnings: Vec<String>,
    pub started_at_unix_ms: u64,
    pub finished_at_unix_ms: Option<u64>,
}

impl RunRecord {
    #[must_use]
    pub const fn is_active(&self) -> bool {
        !matches!(self.state, RunState::Finished)
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |value| {
            u64::try_from(value.as_millis()).unwrap_or(u64::MAX)
        })
}

fn validate_id(id: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        uuid::Uuid::parse_str(id)?.to_string() == id,
        "invalid session run ID"
    );
    Ok(())
}
