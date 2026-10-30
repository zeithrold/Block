//! A launch plan is a single resolved invocation, never persistent settings.
use crate::{
    catalog::GameDirectoryGuard,
    command::{CommandPreview, JavaCommand},
    executor::{ExecutionSink, PreparedInstance, supervise},
    session::SessionProvider,
};
use anyhow::ensure;
use std::{collections::BTreeMap, path::PathBuf};
use tokio_util::sync::CancellationToken;

/// Owns preparation and game-directory locks through process exit. Debug is redacted.
#[derive(Debug)]
pub struct LaunchPlan {
    command: JavaCommand,
    logs: PathBuf,
    _installation: PreparedInstance,
    game_directory: GameDirectoryGuard,
    pub(crate) storage: Option<std::sync::Arc<std::fs::File>>,
}

impl LaunchPlan {
    /// # Errors
    /// Requires a prepared installation, a validated session and an exclusively owned directory.
    pub fn resolve(
        prepared: PreparedInstance,
        session: &dyn SessionProvider,
        directory: GameDirectoryGuard,
        environment: BTreeMap<String, String>,
    ) -> anyhow::Result<Self> {
        let mut command = JavaCommand::bind_for_directory(
            &prepared.plan,
            &prepared.root,
            &prepared.run_id,
            session,
            Some(directory.directory()),
        )?;
        command.set_environment(environment)?;
        let logs = directory
            .directory()
            .join(".block-runs")
            .join(&prepared.run_id);
        crate::executor::ensure_write_path(directory.directory(), &logs)?;
        ensure!(!logs.exists(), "launch log directory already exists");
        Ok(Self {
            command,
            logs,
            _installation: prepared,
            game_directory: directory,
            storage: None,
        })
    }

    #[must_use]
    pub fn preview(&self) -> CommandPreview {
        self.command.preview()
    }

    #[must_use]
    pub const fn environment(&self) -> &BTreeMap<String, String> {
        self.command.environment()
    }

    #[must_use]
    pub fn logs_directory(&self) -> &std::path::Path {
        &self.logs
    }

    #[must_use]
    pub fn game_directory(&self) -> &std::path::Path {
        self.game_directory.directory()
    }

    #[must_use]
    pub fn redact(&self, text: &str) -> String {
        self.command.redact(text)
    }
}

/// Process lifecycle only: no login, download, installer or settings access.
#[derive(Debug)]
pub struct LaunchExecutor;

impl LaunchExecutor {
    /// # Errors
    /// Returns process or log IO failures; cancellation terminates the owned Java child.
    pub async fn run(
        plan: LaunchPlan,
        events: ExecutionSink,
        cancel: &CancellationToken,
    ) -> anyhow::Result<Option<i32>> {
        Self::run_borrowed(&plan, events, cancel).await
    }

    /// Keep the plan's directory locks held while its caller finalizes diagnostics.
    /// # Errors
    /// Returns process or log setup failures; termination facts are emitted separately.
    pub async fn run_borrowed(
        plan: &LaunchPlan,
        events: ExecutionSink,
        cancel: &CancellationToken,
    ) -> anyhow::Result<Option<i32>> {
        tokio::fs::create_dir_all(&plan.logs).await?;
        supervise(plan.command.clone(), plan.logs.clone(), events, cancel).await
    }
}
