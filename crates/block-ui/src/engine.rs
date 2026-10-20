use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use anyhow::{Context as _, ensure};
use block_launcher::{
    CancellationToken, DownloadEvent, Store,
    command::JavaCommand,
    executor::{ExecutionEvent, Executor},
    plan::compile,
    recipe::LaunchRecipe,
    resolver::{ResolveContext, resolve},
};
use minecraft_manifest::{
    decode,
    model::{VersionEntry, VersionIndex},
};
use tokio::sync::mpsc::{Receiver, Sender, channel};

#[derive(Debug)]
#[expect(
    clippy::redundant_pub_crate,
    reason = "The parent view needs these types, but engine internals are not public API."
)]
pub(crate) enum Message {
    Versions(Result<Vec<VersionEntry>, String>),
    Recipe(Result<LaunchRecipe, String>),
    Status(String),
    Progress(usize),
    Log(String),
    Command(String),
    Finished(Result<Option<i32>, String>),
}

#[derive(Debug, Clone)]
#[expect(
    clippy::redundant_pub_crate,
    reason = "The parent view owns the engine; exporting it would conflict with the workspace's unreachable_pub policy."
)]
pub(crate) struct Engine {
    runtime: Arc<tokio::runtime::Runtime>,
    sender: Sender<Message>,
    pub root: PathBuf,
}

impl Engine {
    pub(crate) fn new() -> anyhow::Result<(Self, Receiver<Message>)> {
        let root = std::env::var_os("BLOCK_DATA_DIR").map_or_else(
            || {
                directories::ProjectDirs::from("dev", "zeithrold", "Block")
                    .map(|dirs| dirs.data_local_dir().to_owned())
                    .context("cannot determine the data directory")
            },
            |path| Ok(PathBuf::from(path)),
        )?;
        let runtime = Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()?,
        );
        let (sender, receiver) = channel(256);
        Ok((
            Self {
                runtime,
                sender,
                root,
            },
            receiver,
        ))
    }

    pub(crate) fn load_versions(&self) {
        let root = self.root.clone();
        let sender = self.sender.clone();
        self.runtime.spawn(async move {
            let result = async {
                let store = Store::new(root, false, Arc::new(|_| {}))?;
                let bytes = store
                    .metadata(
                        "version_manifest_v2.json",
                        "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json",
                    )
                    .await?;
                let index: VersionIndex = decode(&bytes)?;
                Ok::<_, anyhow::Error>(index.versions)
            }
            .await;
            let _ = sender
                .send(Message::Versions(
                    result.map_err(|error| format!("{error:#}")),
                ))
                .await;
        });
    }

    pub(crate) fn load_recipe(&self, path: PathBuf) {
        let sender = self.sender.clone();
        self.runtime.spawn(async move {
            let result = async {
                let recipe = LaunchRecipe::load(&path).await?;
                Ok::<_, anyhow::Error>(recipe)
            }
            .await;
            let _ = sender
                .send(Message::Recipe(
                    result.map_err(|error| format!("{error:#}")),
                ))
                .await;
        });
    }

    pub(crate) fn launch(&self, recipe: LaunchRecipe, cancel: CancellationToken) {
        let root = self.root.clone();
        let sender = self.sender.clone();
        self.runtime.spawn(async move {
            let result = run(recipe, root, sender.clone(), cancel)
                .await
                .map_err(|error| format!("{error:#}"));
            let _ = sender.send(Message::Finished(result)).await;
        });
    }
}

async fn run(
    recipe: LaunchRecipe,
    root: PathBuf,
    sender: Sender<Message>,
    cancel: CancellationToken,
) -> anyhow::Result<Option<i32>> {
    let progress = sender.clone();
    let completed = AtomicUsize::new(0);
    let store = Store::new(
        root,
        false,
        Arc::new(move |event| {
            if let DownloadEvent::Complete { .. } = event {
                let count = completed.fetch_add(1, Ordering::Relaxed) + 1;
                if count.is_multiple_of(25) {
                    let _ = progress.try_send(Message::Progress(count));
                }
            }
        }),
    )?;
    let _ = sender
        .send(Message::Status("Resolving launch recipe".into()))
        .await;
    let metadata = resolve(
        &recipe,
        &ResolveContext {
            store: store.clone(),
            cancel: cancel.clone(),
        },
    )
    .await?;
    let plan = compile(&recipe, &metadata)?;
    let events = sender.clone();
    let executor = Executor::new(
        store,
        Arc::new(move |event| {
            let message = match event {
                ExecutionEvent::Stage { name } => Message::Status(name),
                ExecutionEvent::Started { pid } => {
                    Message::Status(format!("Minecraft is running · PID {pid}"))
                }
                ExecutionEvent::Exited { .. } => return,
                ExecutionEvent::Log { line, .. } => {
                    let Some(line) = readable_log(&line) else {
                        return;
                    };
                    Message::Log(line)
                }
            };
            let _ = events.try_send(message);
        }),
    );
    let prepared = executor.prepare(plan, &cancel).await?;
    let command = JavaCommand::bind(
        &prepared.plan,
        &prepared.root,
        &prepared.run_id,
        &recipe.session,
    )?;
    let _ = sender
        .send(Message::Command(serde_json::to_string_pretty(
            &command.preview(),
        )?))
        .await;
    let code = executor.launch(prepared, &recipe.session, &cancel).await?;
    ensure!(
        cancel.is_cancelled() || code == Some(0),
        "Minecraft exited with code {code:?}"
    );
    Ok(code)
}

fn readable_log(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if let Some((_, content)) = trimmed.split_once("<![CDATA[") {
        return Some(content.split("]]>").next().unwrap_or(content).to_owned());
    }
    if trimmed.is_empty() || trimmed.starts_with('<') {
        None
    } else {
        Some(trimmed.to_owned())
    }
}
