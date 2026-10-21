use std::{
    collections::{HashMap, HashSet},
    fs::File,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, SystemTime},
};

use anyhow::{Context as _, ensure};
use block_auth::{Account, AuthenticatedSession, MicrosoftAuth};
use block_launcher::{
    CancellationToken, DownloadEvent, Store,
    catalog::{Catalog, Profile, Snapshot},
    command::JavaCommand,
    crash::{CrashReport, ExitOutcome},
    executor::{ExecutionEvent, Executor},
    library::Library,
    plan::compile,
    recipe::LaunchRecipe,
    resolver::{ResolveContext, resolve},
    session::{GameSession, SessionProvider},
    session_host::{self, RunRecord, RunState},
};
use minecraft_manifest::{
    decode,
    model::{VersionEntry, VersionIndex},
};
use tokio::sync::mpsc::{Receiver, Sender, channel};

// Browser URLs contain transient authorization state and deliberately have no Debug representation.
#[expect(
    clippy::redundant_pub_crate,
    reason = "This private module exposes items only to its parent view."
)]
pub(crate) enum Message {
    Versions(Result<Vec<VersionEntry>, String>),
    Catalog(Result<Snapshot, String>),
    Accounts(Result<Vec<Account>, String>),
    Browser(String),
    Created {
        instance: String,
        profile: String,
    },
    Recipe(Result<LaunchRecipe, String>),
    Status(String),
    Progress(usize),
    Log(String),
    Command(String),
    Session(Box<RunRecord>),
    Crash {
        report: Box<CrashReport>,
        directory: PathBuf,
        guard: File,
    },
    Finished(Result<String, String>),
}

#[derive(Debug, Clone)]
#[expect(
    clippy::redundant_pub_crate,
    reason = "This private module exposes items only to its parent view."
)]
pub(crate) struct Engine {
    runtime: Arc<tokio::runtime::Runtime>,
    sender: Sender<Message>,
    pub root: PathBuf,
}

impl Engine {
    pub(super) fn new() -> anyhow::Result<(Self, Receiver<Message>)> {
        let root = std::env::var_os("BLOCK_DATA_DIR").map_or_else(
            || {
                directories::ProjectDirs::from("dev", "zeithrold", "Block")
                    .map(|dirs| dirs.data_local_dir().to_owned())
                    .context("cannot determine the data directory")
            },
            |path| Ok(PathBuf::from(path)),
        )?;
        Self::with_root(root)
    }

    pub(super) fn with_root(root: PathBuf) -> anyhow::Result<(Self, Receiver<Message>)> {
        let runtime = Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()?,
        );
        let (sender, receiver) = channel(256);
        let engine = Self {
            runtime,
            sender,
            root,
        };
        engine.watch_sessions();
        Ok((engine, receiver))
    }

    fn watch_sessions(&self) {
        let root = self.root.clone();
        let sender = self.sender.clone();
        self.runtime.spawn(observe_sessions(root, sender));
    }

    pub(super) fn load_catalog(&self) {
        let root = self.root.clone();
        let sender = self.sender.clone();
        self.runtime.spawn(async move {
            send_catalog(&root, &sender).await;
            let accounts = tokio::task::spawn_blocking(move || load_accounts(&root)).await;
            let result = match accounts {
                Ok(result) => result,
                Err(error) => Err(error.into()),
            };
            let _ = sender
                .send(Message::Accounts(
                    result.map_err(|error| format!("{error:#}")),
                ))
                .await;
        });
    }

    pub(super) fn load_versions(&self) {
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

    pub(super) fn create_instance(
        &self,
        name: String,
        version: String,
        directory: Option<PathBuf>,
        neoforge: bool,
        cancel: CancellationToken,
    ) {
        let root = self.root.clone();
        let sender = self.sender.clone();
        self.runtime.spawn(async move {
            let result = async {
                let catalog_root = root.clone();
                let (instance, profile) = tokio::task::spawn_blocking(move || {
                    Catalog::open(catalog_root)?.create_instance(
                        name,
                        version,
                        directory,
                        neoforge.then(|| "20.6.144".to_owned()),
                    )
                })
                .await??;
                let _ = sender
                    .send(Message::Created {
                        instance: instance.id.clone(),
                        profile: profile.id.clone(),
                    })
                    .await;
                send_catalog(&root, &sender).await;
                let library = library(root.clone(), &sender)?;
                library
                    .install(&instance.id, &profile.target_id, &cancel)
                    .await?;
                Ok::<_, anyhow::Error>("Installation complete · ready to play".to_owned())
            }
            .await;
            send_catalog(&root, &sender).await;
            finish(&sender, result).await;
        });
    }

    pub(super) fn save_profile(&self, profile: Profile) {
        self.catalog_change(
            move |catalog| catalog.save_profile(profile),
            "Profile saved",
        );
    }

    pub(super) fn create_profile(&self, instance: String, name: String) {
        self.catalog_change(
            move |catalog| catalog.create_profile(&instance, name).map(|_| ()),
            "Profile created",
        );
    }

    pub(super) fn copy_profile(&self, id: String, name: String) {
        self.catalog_change(
            move |catalog| catalog.copy_profile(&id, name).map(|_| ()),
            "Profile copied",
        );
    }

    pub(super) fn delete_profile(&self, id: String) {
        self.catalog_change(
            move |catalog| catalog.delete_profile(&id),
            "Profile deleted · game data preserved",
        );
    }

    fn catalog_change(
        &self,
        operation: impl FnOnce(Catalog) -> anyhow::Result<()> + Send + 'static,
        success: &'static str,
    ) {
        let root = self.root.clone();
        let sender = self.sender.clone();
        self.runtime.spawn(async move {
            let catalog_root = root.clone();
            let result =
                tokio::task::spawn_blocking(move || operation(Catalog::open(catalog_root)?)).await;
            let result = match result {
                Ok(result) => result.map(|()| success.to_owned()),
                Err(error) => Err(error.into()),
            };
            send_catalog(&root, &sender).await;
            finish(&sender, result).await;
        });
    }

    pub(super) fn install(&self, instance: String, target: String, cancel: CancellationToken) {
        let root = self.root.clone();
        let sender = self.sender.clone();
        self.runtime.spawn(async move {
            let result = async {
                library(root.clone(), &sender)?
                    .install(&instance, &target, &cancel)
                    .await?;
                Ok("Repair complete · ready to play".to_owned())
            }
            .await;
            send_catalog(&root, &sender).await;
            finish(&sender, result).await;
        });
    }

    pub(super) fn sign_in(&self, cancel: CancellationToken) {
        let root = self.root.clone();
        let sender = self.sender.clone();
        self.runtime.spawn(async move {
            let result = async {
                // Reject unreadable/corrupt metadata before creating an OS credential.
                let accounts = load_accounts(&root)?;
                let auth = auth()?;
                let pending = auth.begin_sign_in().await?;
                ensure!(!cancel.is_cancelled(), "Sign-in was cancelled.");
                let _ = sender
                    .send(Message::Browser(pending.authorization_url().to_owned()))
                    .await;
                let _ = sender
                    .send(Message::Status(
                        "Finish Microsoft sign-in in your browser".into(),
                    ))
                    .await;
                let session = auth.complete_sign_in(pending, &cancel).await?;
                let account = session.account;
                let (accounts, existed) = match merge_signed_in_account(accounts, &account) {
                    Ok(merged) => merged,
                    Err(error) => {
                        auth.sign_out(&account).await.context("Could not remove this sign-in's new credential after an account-registration conflict")?;
                        return Err(error);
                    }
                };
                if let Err(error) = save_accounts(&root, &accounts) {
                    if !existed {
                        auth.sign_out(&account).await.context("Account metadata could not be saved, and the new credential could not be removed from secure storage")?;
                    }
                    return Err(error);
                }
                let name = account.player_name.clone();
                let _ = sender.send(Message::Accounts(Ok(accounts))).await;
                Ok(format!(
                    "Signed in as {name} · Java Edition ownership verified"
                ))
            }
            .await;
            finish(&sender, result).await;
        });
    }

    pub(super) fn sign_out(&self, account: Account) {
        let root = self.root.clone();
        let sender = self.sender.clone();
        self.runtime.spawn(async move {
            let result = async {
                // Use the account's public registration ID, including after a build configuration change.
                MicrosoftAuth::new(&account.client_id)?
                    .sign_out(&account)
                    .await?;
                let mut accounts = load_accounts(&root)?;
                accounts.retain(|saved| saved.reference != account.reference);
                save_accounts(&root, &accounts)?;
                let _ = sender.send(Message::Accounts(Ok(accounts))).await;
                Ok("Signed out · saved credentials removed".to_owned())
            }
            .await;
            finish(&sender, result).await;
        });
    }

    pub(super) fn play(
        &self,
        profile: String,
        account: Account,
        cancel: CancellationToken,
        explicit_stop: CancellationToken,
    ) {
        let root = self.root.clone();
        let sender = self.sender.clone();
        self.runtime.spawn(async move {
            let result = async {
                let _ = sender
                    .send(Message::Status(
                        "Preparing · checking account authorization".into(),
                    ))
                    .await;
                let session = auth()?.refresh(&account, &cancel).await?;
                let provider = OnlineSession {
                    game: game_session(&session),
                    expires_at: session.expires_at,
                };
                ensure!(!cancel.is_cancelled(), "Launch was cancelled.");
                let game = provider.session()?;
                let executable = std::env::current_exe()?;
                let record = session_host::spawn_session(
                    &root,
                    &executable,
                    &profile,
                    &game,
                    provider.expires_at,
                )
                .await?;
                // Window closure only cancels authentication. Only an explicit Stop
                // may terminate a session that completed its independent handshake.
                if explicit_stop.is_cancelled() {
                    return session_host::stop(&root, &record.context.run_id).await;
                }
                Ok::<_, anyhow::Error>(record)
            }
            .await;
            send_catalog(&root, &sender).await;
            match result {
                Ok(record) => {
                    let _ = sender.send(Message::Session(Box::new(record))).await;
                }
                Err(error) => finish(&sender, Err(error)).await,
            }
        });
    }

    pub(super) fn stop_session(&self, id: String) {
        let root = self.root.clone();
        let sender = self.sender.clone();
        self.runtime.spawn(async move {
            match session_host::stop(&root, &id).await {
                Ok(record) => {
                    let _ = sender.send(Message::Session(Box::new(record))).await;
                }
                Err(error) => {
                    let _ = sender
                        .send(Message::Status(format!(
                            "Could not stop this session · {error:#}"
                        )))
                        .await;
                }
            }
        });
    }

    pub(super) fn load_recipe(&self, path: PathBuf) {
        let sender = self.sender.clone();
        self.runtime.spawn(async move {
            let result = LaunchRecipe::load(&path).await;
            let _ = sender
                .send(Message::Recipe(
                    result.map_err(|error| format!("{error:#}")),
                ))
                .await;
        });
    }

    pub(super) fn launch(&self, recipe: LaunchRecipe, cancel: CancellationToken) {
        let root = self.root.clone();
        let sender = self.sender.clone();
        self.runtime.spawn(async move {
            let result = run_recipe(recipe, root, sender.clone(), cancel)
                .await
                .map(|code| format!("Development recipe exited · {code:?}"));
            finish(&sender, result).await;
        });
    }
}

fn auth() -> anyhow::Result<MicrosoftAuth> {
    let client = std::env::var("MICROSOFT_CLIENT_ID")
        .ok()
        .or_else(|| option_env!("MICROSOFT_CLIENT_ID").map(str::to_owned))
        .unwrap_or_default();
    Ok(MicrosoftAuth::new(&client)?)
}

#[derive(Debug)]
struct OnlineSession {
    game: GameSession,
    expires_at: SystemTime,
}

impl SessionProvider for OnlineSession {
    fn session(&self) -> anyhow::Result<GameSession> {
        ensure!(
            self.expires_at
                .duration_since(SystemTime::now())
                .is_ok_and(|remaining| remaining >= Duration::from_secs(30)),
            "The account session expired while preparing. Click Play again to refresh it."
        );
        self.game.session()
    }
}

fn game_session(session: &AuthenticatedSession) -> GameSession {
    GameSession {
        username: session.account.player_name.clone(),
        uuid: session.account.reference.player_id.simple().to_string(),
        access_token: session.expose_access_token().to_owned(),
        user_type: "msa".into(),
        xuid: String::new(),
        client_id: session.account.client_id.clone(),
    }
}

fn library(root: PathBuf, sender: &Sender<Message>) -> anyhow::Result<Library> {
    let progress = sender.clone();
    let completed = AtomicUsize::new(0);
    let download = Arc::new(move |event| {
        if let DownloadEvent::Complete { .. } = event {
            let count = completed.fetch_add(1, Ordering::Relaxed) + 1;
            if count.is_multiple_of(10) {
                let _ = progress.try_send(Message::Progress(count));
            }
        }
    });
    let events = sender.clone();
    Library::new(
        root,
        download,
        Arc::new(move |event| forward_event(&events, event)),
    )
}

fn forward_event(sender: &Sender<Message>, event: ExecutionEvent) {
    let message = match event {
        ExecutionEvent::Stage { name } => Message::Status(name),
        ExecutionEvent::Started { pid } => {
            Message::Status(format!("Running · Minecraft PID {pid}"))
        }
        ExecutionEvent::Exited { .. } | ExecutionEvent::Terminated { .. } => return,
        ExecutionEvent::CaptureFailed { message } => Message::Log(message),
        ExecutionEvent::Log { line, .. } => {
            let Some(line) = readable_log(&line) else {
                return;
            };
            Message::Log(line)
        }
    };
    let _ = sender.try_send(message);
}

async fn observe_sessions(root: PathBuf, sender: Sender<Message>) {
    let mut seen: HashMap<String, (u64, RunState)> = HashMap::new();
    let mut reported_failure = false;
    while !sender.is_closed() {
        let list_root = root.clone();
        let records =
            tokio::task::spawn_blocking(move || session_host::list_runs(&list_root)).await;
        match records {
            Ok(Ok(mut records)) => {
                reported_failure = false;
                records.sort_by_key(|record| std::cmp::Reverse(record.started_at_unix_ms));
                let (active, mut finished): (Vec<_>, Vec<_>) =
                    records.into_iter().partition(RunRecord::is_active);
                finished.truncate(128);
                let retained: HashSet<_> = active
                    .iter()
                    .chain(&finished)
                    .map(|record| record.context.run_id.as_str())
                    .collect();
                seen.retain(|id, _| retained.contains(id.as_str()));
                for record in finished {
                    if !publish_session(&root, &sender, &mut seen, record).await {
                        return;
                    }
                }
                if !observe_active_sessions(&root, &sender, &mut seen, active).await {
                    return;
                }
            }
            _ if !reported_failure => {
                reported_failure = true;
                let _ = sender
                    .send(Message::Log(
                        "Session monitoring is temporarily unavailable; retrying.".into(),
                    ))
                    .await;
            }
            _ => {}
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

async fn observe_active_sessions(
    root: &Path,
    sender: &Sender<Message>,
    seen: &mut HashMap<String, (u64, RunState)>,
    active: Vec<RunRecord>,
) -> bool {
    // Restore every reservation immediately, even if older monitors are unreachable.
    for record in &active {
        if !seen.contains_key(&record.context.run_id)
            && !publish_session(root, sender, seen, record.clone()).await
        {
            return false;
        }
    }
    let mut pending = active.into_iter();
    let mut checks = tokio::task::JoinSet::new();
    loop {
        // Cap concurrent local IPC independently of the number of retained sessions.
        while checks.len() < 8 {
            let Some(record) = pending.next() else {
                break;
            };
            let root = root.to_owned();
            checks.spawn(async move { check_live_session(&root, record).await });
        }
        let Some(result) = checks.join_next().await else {
            return true;
        };
        if let Ok(record) = result
            && !publish_session(root, sender, seen, record).await
        {
            return false;
        }
    }
}

async fn check_live_session(root: &Path, mut record: RunRecord) -> RunRecord {
    if let Ok(current) = session_host::status(root, &record.context.run_id).await {
        current
    } else {
        record.state = RunState::MonitoringInterrupted;
        record.status = "Monitoring interrupted · Minecraft may still be running".into();
        record
    }
}

async fn publish_session(
    root: &Path,
    sender: &Sender<Message>,
    seen: &mut HashMap<String, (u64, RunState)>,
    record: RunRecord,
) -> bool {
    let id = record.context.run_id.clone();
    let fingerprint = (record.revision, record.state);
    if seen.get(&id) == Some(&fingerprint) {
        return true;
    }
    seen.insert(id.clone(), fingerprint);
    let notify = record
        .outcome
        .as_ref()
        .is_some_and(ExitOutcome::should_notify);
    if sender
        .send(Message::Session(Box::new(record)))
        .await
        .is_err()
    {
        return false;
    }
    if notify {
        let crash_root = root.to_owned();
        let crash = tokio::task::spawn_blocking(move || {
            let Some(guard) = session_host::claim_help(&crash_root, &id)? else {
                return Ok::<_, anyhow::Error>(None);
            };
            let report = session_host::read_crash(&crash_root, &id)?;
            let directory = session_host::report_directory(&crash_root, &id)?;
            Ok(Some((report, directory, guard)))
        })
        .await;
        if let Ok(Ok(Some((report, directory, guard)))) = crash {
            return sender
                .send(Message::Crash {
                    report: Box::new(report),
                    directory,
                    guard,
                })
                .await
                .is_ok();
        }
    }
    true
}

async fn send_catalog(root: &Path, sender: &Sender<Message>) {
    let root = root.to_owned();
    let result = tokio::task::spawn_blocking(move || Catalog::open(root)?.snapshot()).await;
    let result = match result {
        Ok(result) => result,
        Err(error) => Err(error.into()),
    };
    let _ = sender
        .send(Message::Catalog(
            result.map_err(|error| format!("{error:#}")),
        ))
        .await;
}

async fn finish(sender: &Sender<Message>, result: anyhow::Result<String>) {
    let _ = sender
        .send(Message::Finished(
            result.map_err(|error| format!("{error:#}")),
        ))
        .await;
}

fn load_accounts(root: &Path) -> anyhow::Result<Vec<Account>> {
    let path = root.join("accounts.json");
    match std::fs::read(&path) {
        Ok(bytes) => {
            serde_json::from_slice(&bytes).context("Saved account metadata could not be read")
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error).context("Saved account metadata could not be opened"),
    }
}

fn merge_signed_in_account(
    mut accounts: Vec<Account>,
    account: &Account,
) -> anyhow::Result<(Vec<Account>, bool)> {
    ensure!(
        !accounts
            .iter()
            .any(|saved| saved.reference == account.reference
                && saved.credential_ref != account.credential_ref),
        "This player already has a saved account for a different Microsoft application registration. Sign out that account before signing in again."
    );
    let existed = accounts
        .iter()
        .any(|saved| saved.credential_ref == account.credential_ref);
    accounts.retain(|saved| saved.reference != account.reference);
    accounts.push(account.clone());
    Ok((accounts, existed))
}

fn save_accounts(root: &Path, accounts: &[Account]) -> anyhow::Result<()> {
    use std::io::Write as _;
    std::fs::create_dir_all(root)?;
    let mut temporary = tempfile::NamedTempFile::new_in(root)?;
    temporary.write_all(&serde_json::to_vec_pretty(accounts)?)?;
    temporary.as_file().sync_all()?;
    temporary.persist(root.join("accounts.json"))?;
    Ok(())
}

async fn run_recipe(
    recipe: LaunchRecipe,
    root: PathBuf,
    sender: Sender<Message>,
    cancel: CancellationToken,
) -> anyhow::Result<Option<i32>> {
    let store = Store::new(root, false, Arc::new(|_| {}))?;
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
    let executor = Executor::new(store, Arc::new(move |event| forward_event(&events, event)));
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
        return Some(
            content
                .split("]]> ")
                .next()
                .unwrap_or(content)
                .trim_end_matches("]]>")
                .to_owned(),
        );
    }
    if trimmed.is_empty() || trimmed.starts_with('<') {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use block_launcher::session::{OfflineSession, SessionProvider as _};
    use std::time::{Duration, SystemTime};

    #[test]
    fn observer_restores_an_old_active_session_after_more_than_128_finished_runs()
    -> anyhow::Result<()> {
        use block_launcher::{
            crash::CrashContext,
            session_host::{RunRecord, RunState},
        };

        let directory = tempfile::tempdir()?;
        let id = |index| format!("00000000-0000-4000-8000-{index:012x}");
        let active_id = id(1);
        let template = RunRecord {
            schema_version: 1,
            context: CrashContext {
                run_id: active_id.clone(),
                profile_id: "profile".into(),
                profile_name: "Older game".into(),
                minecraft_version: "1.20.6".into(),
                loader: None,
            },
            instance_id: "instance".into(),
            game_directory: directory.path().join("game"),
            supervisor_pid: None,
            java_pid: None,
            state: RunState::Running,
            outcome: None,
            status: "Running".into(),
            revision: 1,
            recent_output: Vec::new(),
            capture_warnings: Vec::new(),
            started_at_unix_ms: 1,
            finished_at_unix_ms: None,
        };
        for index in 1_u64..=130 {
            let mut record = template.clone();
            record.context.run_id = id(index);
            record.started_at_unix_ms = index;
            if index > 1 {
                record.state = RunState::Finished;
                record.finished_at_unix_ms = Some(index + 1);
            }
            let path = directory.path().join("sessions").join(id(index));
            std::fs::create_dir_all(&path)?;
            std::fs::write(path.join("session.json"), serde_json::to_vec(&record)?)?;
        }
        let (engine, mut receiver) = super::Engine::with_root(directory.path().to_path_buf())?;
        let (finished, active) = engine.runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(3), async {
                let mut finished = Vec::new();
                while let Some(message) = receiver.recv().await {
                    if let super::Message::Session(record) = message {
                        if record.context.run_id == active_id {
                            return Ok::<_, anyhow::Error>((finished, record));
                        }
                        if !record.is_active() {
                            finished.push(record.context.run_id);
                        }
                    }
                }
                anyhow::bail!("session observer closed before restoring the active run")
            })
            .await
        })??;
        assert!(active.is_active());
        assert_eq!(active.started_at_unix_ms, 1);
        assert_eq!(finished.len(), 128);
        assert!(
            !finished.contains(&id(2)),
            "Only terminal history may be capped"
        );
        Ok(())
    }

    #[test]
    fn online_provider_rejects_expiry_before_command_binding() -> anyhow::Result<()> {
        let game = OfflineSession::Offline {
            username: "Synthetic".into(),
        }
        .session()?;
        let provider = super::OnlineSession {
            game,
            expires_at: SystemTime::now() + Duration::from_secs(15),
        };
        assert!(provider.session().is_err());
        let valid = super::OnlineSession {
            game: provider.game,
            expires_at: SystemTime::now() + Duration::from_secs(300),
        };
        assert!(valid.session().is_ok());
        Ok(())
    }

    #[test]
    fn account_metadata_is_atomic_and_corruption_is_not_reset() -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        assert_eq!(super::load_accounts(root.path())?, Vec::new());
        let account = block_auth::Account {
            reference: block_auth::AccountRef {
                player_id: "00000000-0000-4000-8000-000000000001".parse()?,
            },
            provider: block_auth::AccountProvider::Microsoft,
            player_name: "Synthetic".into(),
            client_id: "synthetic-public-client".into(),
            credential_ref: "synthetic-store-reference".into(),
        };
        super::save_accounts(root.path(), std::slice::from_ref(&account))?;
        assert_eq!(super::load_accounts(root.path())?, vec![account]);
        super::save_accounts(root.path(), &[])?;
        assert_eq!(super::load_accounts(root.path())?, Vec::new());
        std::fs::write(root.path().join("accounts.json"), b"invalid catalog")?;
        assert!(super::load_accounts(root.path()).is_err());
        assert_eq!(
            std::fs::read(root.path().join("accounts.json"))?,
            b"invalid catalog"
        );
        Ok(())
    }
    #[test]
    fn account_registration_changes_cannot_replace_metadata_and_orphan_old_credentials()
    -> anyhow::Result<()> {
        let old = block_auth::Account {
            reference: block_auth::AccountRef {
                player_id: "00000000-0000-4000-8000-000000000001".parse()?,
            },
            provider: block_auth::AccountProvider::Microsoft,
            player_name: "BeforeRename".into(),
            client_id: "old-public-client".into(),
            credential_ref: "old-store-reference".into(),
        };
        let mut renamed = old.clone();
        renamed.player_name = "AfterRename".into();
        let (accounts, existed) = super::merge_signed_in_account(vec![old.clone()], &renamed)?;
        assert!(existed);
        assert_eq!(accounts, vec![renamed.clone()]);
        renamed.client_id = "new-public-client".into();
        renamed.credential_ref = "new-store-reference".into();
        assert!(super::merge_signed_in_account(vec![old], &renamed).is_err());
        let (accounts, existed) = super::merge_signed_in_account(Vec::new(), &renamed)?;
        assert!(!existed);
        assert_eq!(accounts, vec![renamed]);
        Ok(())
    }
    #[test]
    fn corrupt_account_metadata_blocks_oauth_before_any_browser_or_credential_operation()
    -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        std::fs::write(
            directory.path().join("accounts.json"),
            b"corrupt account document",
        )?;
        let runtime = std::sync::Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .build()?,
        );
        let (sender, mut receiver) = tokio::sync::mpsc::channel(8);
        let engine = super::Engine {
            runtime: runtime.clone(),
            sender,
            root: directory.path().to_owned(),
        };
        engine.sign_in(block_launcher::CancellationToken::new());
        let message = runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(3), receiver.recv()).await
        })?;
        assert!(
            matches!(message, Some(super::Message::Finished(Err(error))) if error.contains("Saved account metadata could not be read"))
        );
        assert_eq!(
            std::fs::read(directory.path().join("accounts.json"))?,
            b"corrupt account document"
        );
        Ok(())
    }
}
