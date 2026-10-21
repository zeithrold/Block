#![expect(
    clippy::redundant_pub_crate,
    reason = "Engine types are exposed only within this private UI module."
)]
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime},
};

use anyhow::{Context as _, ensure};
use block_auth::{
    Account, AccountProvider, AccountRef, AuthenticatedSession, MicrosoftAuth,
    PreparedMicrosoftSignIn,
};
use block_launcher::{
    CancellationToken, Store,
    catalog::{Catalog, Profile, ProfileAppearance, ProfileConfiguration, Snapshot},
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

use super::{appearance::Preferences, settings::GameDefaults};

#[derive(Clone, Debug)]
pub(super) struct ProfileDraftValues {
    pub name: String,
    pub defaults: GameDefaults,
    pub appearance: ProfileAppearance,
    pub target_id: Option<String>,
    pub server: block_launcher::catalog::ServerPreferences,
}

impl ProfileDraftValues {
    fn configuration(self) -> anyhow::Result<ProfileConfiguration> {
        self.defaults.validate()?;
        let mut game_args = vec![
            "--width".into(),
            self.defaults.width.to_string(),
            "--height".into(),
            self.defaults.height.to_string(),
        ];
        if self.defaults.fullscreen {
            game_args.push("--fullscreen".into());
        }
        Ok(ProfileConfiguration {
            name: self.name.trim().to_owned(),
            runtime: if self.defaults.java.trim().is_empty() {
                block_launcher::recipe::RuntimeSelection::Managed
            } else {
                block_launcher::recipe::RuntimeSelection::Local {
                    java: PathBuf::from(self.defaults.java.trim()),
                }
            },
            memory_mib: self.defaults.memory_mib,
            jvm_args: super::parse_arguments(&self.defaults.jvm_arguments)?,
            game_args,
            appearance: self.appearance,
            inheritance: block_launcher::catalog::ProfileInheritance::default(),
            server: self.server,
            target_id: self.target_id.unwrap_or_else(|| "vanilla".into()),
        })
    }
}

// Browser URLs contain transient authorization state and deliberately have no Debug representation.
#[expect(
    clippy::redundant_pub_crate,
    reason = "This private module exposes items only to its parent view."
)]
pub(crate) enum Message {
    ForOperation {
        operation: u64,
        message: Box<Self>,
    },
    Versions(Result<Vec<VersionEntry>, String>),
    Catalog(Result<Snapshot, String>),
    Accounts(Result<Vec<Account>, String>),
    AuthPrepared(Account),
    SignedIn(Account),
    DeviceCode {
        user_code: String,
        verification_uri: String,
        expires_at: std::time::Instant,
    },
    AuthStage(block_auth::SignInStage),
    ConnectionTest(super::connection_test::Request, Result<u16, String>),
    Browser(String),
    Created {
        instance: String,
        profile: String,
    },
    Recipe(Result<LaunchRecipe, String>),
    Status(String),
    Downloads(Arc<Mutex<super::tasks::DownloadTracker>>),
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

/// A task captures its identity when it starts. Late results cannot adopt the
/// identity of a later task that happens to use the same channel.
#[derive(Clone, Debug)]
struct EventSender {
    sender: Sender<Message>,
    operation: Option<u64>,
}

impl From<Sender<Message>> for EventSender {
    fn from(sender: Sender<Message>) -> Self {
        Self {
            sender,
            operation: None,
        }
    }
}

impl EventSender {
    fn wrap(&self, message: Message) -> Message {
        // Detached sessions and crash presentation have their own run identity.
        if matches!(message, Message::Session(_) | Message::Crash { .. }) {
            return message;
        }
        match self.operation {
            Some(operation) => Message::ForOperation {
                operation,
                message: Box::new(message),
            },
            None => message,
        }
    }

    async fn send(
        &self,
        message: Message,
    ) -> Result<(), Box<tokio::sync::mpsc::error::SendError<Message>>> {
        self.sender.send(self.wrap(message)).await.map_err(Box::new)
    }

    fn try_send(
        &self,
        message: Message,
    ) -> Result<(), Box<tokio::sync::mpsc::error::TrySendError<Message>>> {
        self.sender.try_send(self.wrap(message)).map_err(Box::new)
    }

    fn is_closed(&self) -> bool {
        self.sender.is_closed()
    }

    fn unscoped(&self) -> Self {
        Self {
            sender: self.sender.clone(),
            operation: None,
        }
    }
}

#[derive(Debug, Clone)]
#[expect(
    clippy::redundant_pub_crate,
    reason = "This private module exposes items only to its parent view."
)]
pub(crate) struct Engine {
    runtime: Arc<tokio::runtime::Runtime>,
    sender: EventSender,
    pub root: PathBuf,
    prepared_sign_in: Arc<Mutex<Option<(CancellationToken, PreparedMicrosoftSignIn)>>>,
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
            sender: sender.into(),
            root,
            prepared_sign_in: Default::default(),
        };
        engine.watch_sessions();
        Ok((engine, receiver))
    }

    pub(super) const fn begin_operation(&mut self, operation: u64) {
        self.sender.operation = Some(operation);
    }

    fn watch_sessions(&self) {
        let root = self.root.clone();
        let sender = self.sender.unscoped();
        self.runtime.spawn(observe_sessions(root, sender));
    }

    pub(super) fn load_catalog(&self) {
        let root = self.root.clone();
        let sender = self.sender.unscoped();
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

    pub(super) fn test_connection(&self, request: super::connection_test::Request) {
        let root = self.root.clone();
        let sender = self.sender.unscoped();
        self.runtime.spawn(async move {
            let result = async {
                let store = Store::with_options(
                    root.join("cache"),
                    false,
                    Arc::new(|_| {}),
                    request.options.clone(),
                )?;
                store
                    .probe_connection(super::connection_test::ENDPOINT)
                    .await
            }
            .await
            .map_err(|error: anyhow::Error| format!("{error:#}"));
            let _ = sender.send(Message::ConnectionTest(request, result)).await;
        });
    }

    pub(super) fn load_versions(&self) {
        let root = self.root.clone();
        let sender = self.sender.unscoped();
        self.runtime.spawn(async move {
            let result = version_index(
                root,
                "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json",
            )
            .await;
            let _ = sender
                .send(Message::Versions(
                    result.map_err(|error| format!("{error:#}")),
                ))
                .await;
        });
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "The draft commit passes explicit instance and profile boundaries."
    )]
    pub(super) fn create_instance_with_profile(
        &self,
        name: String,
        version: String,
        directory: Option<PathBuf>,
        neoforge: bool,
        defaults: GameDefaults,
        profile: Option<ProfileDraftValues>,
        cancel: CancellationToken,
    ) {
        let root = self.root.clone();
        let sender = self.sender.clone();
        self.runtime.spawn(async move {
            let result = async {
                let configuration = profile
                    .unwrap_or_else(|| ProfileDraftValues {
                        name: name.clone(),
                        defaults,
                        appearance: ProfileAppearance::default(),
                        target_id: None,
                        server: block_launcher::catalog::ServerPreferences::default(),
                    })
                    .configuration()?;
                let mut configuration = configuration;
                if neoforge && configuration.target_id == "vanilla" {
                    configuration.target_id = "neoforge-20.6.144".into();
                }
                let catalog_root = root.clone();
                let (instance, profile) = tokio::task::spawn_blocking(move || {
                    let catalog = Catalog::open(catalog_root)?;
                    catalog.create_instance_configured(
                        name,
                        version,
                        directory,
                        neoforge.then(|| "20.6.144".to_owned()),
                        configuration,
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
                let library = library(root.clone(), &sender).await?;
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

    pub(super) fn create_profile_draft(&self, instance: String, values: ProfileDraftValues) {
        let root = self.root.clone();
        let sender = self.sender.clone();
        self.runtime.spawn(async move {
            let catalog_root = root.clone();
            let result = tokio::task::spawn_blocking(move || {
                Catalog::open(catalog_root)?
                    .create_profile_configured(&instance, values.configuration()?)
            })
            .await;
            let result = match result {
                Ok(Ok(profile)) => {
                    let _ = sender
                        .send(Message::Created {
                            instance: profile.instance_id,
                            profile: profile.id,
                        })
                        .await;
                    Ok("Profile created · existing installation preserved".into())
                }
                Ok(Err(error)) => Err(error),
                Err(error) => Err(error.into()),
            };
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

    pub(super) fn separate_profiles(&self) {
        self.catalog_change(
            |catalog| catalog.separate_profiles(),
            "Separated profile mode enabled",
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

    pub(super) fn save_instance(
        &self,
        id: String,
        configuration: block_launcher::catalog::InstanceConfiguration,
    ) {
        self.catalog_change(
            move |catalog| catalog.save_instance(&id, configuration),
            "Instance settings saved",
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
                library(root.clone(), &sender)
                    .await?
                    .install(&instance, &target, &cancel)
                    .await?;
                Ok("Repair complete · ready to play".to_owned())
            }
            .await;
            send_catalog(&root, &sender).await;
            finish(&sender, result).await;
        });
    }

    pub(super) fn discard_prepared_sign_in(&self) {
        if let Some((cancel, _)) = self
            .prepared_sign_in
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            cancel.cancel();
        }
    }

    pub(super) fn sign_in(&self, cancel: CancellationToken, device: bool) {
        let root = self.root.clone();
        let sender = self.sender.clone();
        let pending_account = self.prepared_sign_in.clone();
        self.runtime.spawn(async move {
            let result = async {
                // Reject unreadable/corrupt metadata before creating an OS credential.
                let accounts = load_accounts(&root)?;
                let observed = sender.clone();
                let auth = auth()?.with_stage_observer(Arc::new(move |stage| {
                    let _ = observed.try_send(Message::AuthStage(stage));
                }));
                let prepared = if device {
                    let pending = auth.begin_device_sign_in(&cancel).await?;
                    ensure!(!cancel.is_cancelled(), "Sign-in was cancelled.");
                    let _ = sender
                        .send(Message::DeviceCode {
                            user_code: pending.user_code().to_owned(),
                            verification_uri: pending.verification_uri().to_owned(),
                            expires_at: std::time::Instant::now() + pending.remaining(),
                        })
                        .await;
                    auth.complete_device_sign_in(pending, &cancel).await?
                } else {
                    let pending = auth.begin_sign_in().await?;
                    ensure!(!cancel.is_cancelled(), "Sign-in was cancelled.");
                    let _ = sender
                        .send(Message::Browser(pending.authorization_url().to_owned()))
                        .await;
                    auth.complete_sign_in(pending, &cancel).await?
                };
                let account = prepared.account().clone();
                // Validate registration conflicts before offering final confirmation.
                merge_signed_in_account(accounts, &account)?;
                {
                    let mut pending = pending_account
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    ensure!(!cancel.is_cancelled(), "Sign-in was cancelled.");
                    *pending = Some((cancel.clone(), prepared));
                }
                let _ = sender.send(Message::AuthPrepared(account)).await;
                Ok("Identity verified. Finish setup to save this account.".into())
            }
            .await;
            finish(&sender, result).await;
        });
    }

    pub(super) fn save_sign_in(&self, expected: AccountRef) {
        let root = self.root.clone();
        let sender = self.sender.clone();
        let pending_account = self.prepared_sign_in.clone();
        self.runtime.spawn(async move {
            let result = async {
                let (cancel, prepared) = {
                    let mut pending = pending_account.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    ensure!(pending.as_ref().is_some_and(|(_, prepared)| prepared.account().reference == expected), "The prepared identity is no longer available. Sign in again.");
                    pending.take().context("Prepared sign-in is unavailable")?
                };
                ensure!(!cancel.is_cancelled(), "Sign-in was cancelled.");
                let account = prepared.account().clone();
                let (accounts, existed) = merge_signed_in_account(load_accounts(&root)?, &account)?;
                let auth = auth()?.with_stage_observer({
                    let observed = sender.clone();
                    Arc::new(move |stage| { let _ = observed.try_send(Message::AuthStage(stage)); })
                });
                auth.save_sign_in(prepared).await?;
                let _ = sender.send(Message::AuthStage(block_auth::SignInStage::SaveMetadata)).await;
                if let Err(error) = save_accounts(&root, &accounts) {
                    if !existed {
                        auth.sign_out(&account).await.context("Account metadata could not be saved, and the new credential could not be removed from secure storage")?;
                    }
                    return Err(error);
                }
                let name = account.player_name.clone();
                // Durable state remains authoritative even if its child window closed.
                let _ = sender.unscoped().send(Message::Accounts(Ok(accounts))).await;
                let _ = sender.send(Message::SignedIn(account)).await;
                Ok(format!(
                    "Signed in as {name} · Java Edition ownership verified"
                ))
            }
            .await;
            finish(&sender, result).await;
        });
    }

    pub(super) fn save_offline_account(&self, account: Account) {
        let root = self.root.clone();
        let sender = self.sender.clone();
        self.runtime.spawn(async move {
            let result = async {
                ensure!(
                    account == offline_account(&account.player_name)?,
                    "The offline identity is invalid."
                );
                let (accounts, _) = merge_signed_in_account(load_accounts(&root)?, &account)?;
                save_accounts(&root, &accounts)?;
                let _ = sender
                    .unscoped()
                    .send(Message::Accounts(Ok(accounts)))
                    .await;
                let name = account.player_name.clone();
                let _ = sender.send(Message::SignedIn(account)).await;
                Ok(format!("Offline identity {name} saved for local play"))
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
                if account.provider == AccountProvider::Microsoft {
                    MicrosoftAuth::new(&account.client_id)?
                        .sign_out(&account)
                        .await?;
                }
                let mut accounts = load_accounts(&root)?;
                accounts.retain(|saved| saved.reference != account.reference);
                save_accounts(&root, &accounts)?;
                let _ = sender.send(Message::Accounts(Ok(accounts))).await;
                Ok(if account.provider == AccountProvider::Offline {
                    "Offline identity removed · game files kept"
                } else {
                    "Signed out · saved credentials removed"
                }
                .to_owned())
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
                let provider = match account.provider {
                    AccountProvider::Microsoft => {
                        let session = auth()?.refresh(&account, &cancel).await?;
                        OnlineSession {
                            game: game_session(&session),
                            expires_at: session.expires_at,
                        }
                    }
                    AccountProvider::Offline => {
                        ensure!(
                            account == offline_account(&account.player_name)?,
                            "The saved offline identity is invalid. Remove it and add it again."
                        );
                        OnlineSession {
                            game: block_launcher::session::OfflineSession::Offline {
                                username: account.player_name.clone(),
                            }
                            .session()?,
                            // This bounds the private supervisor bootstrap, not account validity.
                            expires_at: SystemTime::now() + Duration::from_secs(24 * 60 * 60),
                        }
                    }
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
        let sender = self.sender.unscoped();
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
        let sender = self.sender.unscoped();
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

async fn library(root: PathBuf, sender: &EventSender) -> anyhow::Result<Library> {
    let options = Preferences::load(&root)?.downloads;
    let tracker = Arc::new(Mutex::new(super::tasks::DownloadTracker::default()));
    sender
        .send(Message::Downloads(tracker.clone()))
        .await
        .map_err(|_| anyhow::anyhow!("launcher window closed before downloads started"))?;
    let download = Arc::new(move |event| {
        tracker
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .apply(event);
    });
    let events = sender.clone();
    Library::with_options(
        root,
        download,
        Arc::new(move |event| forward_event(&events, event)),
        options,
    )
}

async fn version_index(root: PathBuf, url: &str) -> anyhow::Result<Vec<VersionEntry>> {
    let options = Preferences::load(&root)?.downloads;
    let store = Store::with_options(root, false, Arc::new(|_| {}), options)?;
    let bytes = store.metadata("version_manifest_v2.json", url).await?;
    let index: VersionIndex = decode(&bytes)?;
    Ok(index.versions)
}

fn forward_event(sender: &EventSender, event: ExecutionEvent) {
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

async fn observe_sessions(root: PathBuf, sender: EventSender) {
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
    sender: &EventSender,
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
    sender: &EventSender,
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

async fn send_catalog(root: &Path, sender: &EventSender) {
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

async fn finish(sender: &EventSender, result: anyhow::Result<String>) {
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

pub(super) fn offline_account(username: &str) -> anyhow::Result<Account> {
    let game = block_launcher::session::OfflineSession::Offline {
        username: username.to_owned(),
    }
    .session()?;
    Ok(Account {
        reference: AccountRef {
            player_id: game.uuid.parse()?,
        },
        provider: AccountProvider::Offline,
        player_name: game.username,
        client_id: String::new(),
        credential_ref: String::new(),
    })
}

fn merge_signed_in_account(
    mut accounts: Vec<Account>,
    account: &Account,
) -> anyhow::Result<(Vec<Account>, bool)> {
    ensure!(
        !accounts
            .iter()
            .any(|saved| saved.reference == account.reference
                && (saved.provider != account.provider
                    || saved.credential_ref != account.credential_ref)),
        "This player already has a saved account for a different provider or Microsoft application registration. Remove that account before adding it again."
    );
    let existed = accounts.iter().any(|saved| {
        saved.reference == account.reference
            && saved.provider == account.provider
            && saved.credential_ref == account.credential_ref
    });
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
    sender: EventSender,
    cancel: CancellationToken,
) -> anyhow::Result<Option<i32>> {
    let options = Preferences::load(&root)?.downloads;
    let store = Store::with_options(root, false, Arc::new(|_| {}), options)?;
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
    fn offline_identity_matches_java_and_rejects_invalid_or_colliding_metadata()
    -> anyhow::Result<()> {
        let steve = super::offline_account("Steve")?;
        assert_eq!(
            steve.reference.player_id.simple().to_string(),
            "5627dd98e6be3c21b8a8e92344183641"
        );
        assert_ne!(steve.reference, super::offline_account("steve")?.reference);
        assert!(steve.client_id.is_empty() && steve.credential_ref.is_empty());
        for name in [
            "",
            "bad name",
            "name-with-dash",
            "中文",
            "abcdefghijklmnopq",
            "line\n",
        ] {
            assert!(super::offline_account(name).is_err(), "{name:?}");
        }
        let alex = super::offline_account("Alex_123")?;
        let (accounts, existed) = super::merge_signed_in_account(vec![steve.clone()], &alex)?;
        assert!(!existed);
        let (accounts, existed) = super::merge_signed_in_account(accounts, &steve)?;
        assert!(existed);
        assert_eq!(accounts.len(), 2);
        let mut microsoft = steve.clone();
        microsoft.provider = block_auth::AccountProvider::Microsoft;
        microsoft.credential_ref = "synthetic-keychain-reference".into();
        assert!(super::merge_signed_in_account(vec![microsoft], &steve).is_err());
        Ok(())
    }

    #[test]
    fn offline_identity_persists_reopens_and_removes_without_a_credential_store()
    -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let runtime = std::sync::Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()?,
        );
        let (sender, mut receiver) = tokio::sync::mpsc::channel(32);
        let engine = super::Engine {
            runtime: runtime.clone(),
            sender: sender.into(),
            root: root.path().to_owned(),
            prepared_sign_in: Default::default(),
        };
        let account = super::offline_account("Local_Player")?;
        let finish = |receiver: &mut tokio::sync::mpsc::Receiver<super::Message>| {
            runtime.block_on(async {
                tokio::time::timeout(Duration::from_secs(3), async {
                    while let Some(message) = receiver.recv().await {
                        if let super::Message::Finished(result) = message {
                            return result.map_err(anyhow::Error::msg);
                        }
                    }
                    anyhow::bail!("The account operation ended without completion")
                })
                .await?
            })
        };
        engine.save_offline_account(account.clone());
        finish(&mut receiver)?;
        assert_eq!(super::load_accounts(root.path())?, vec![account.clone()]);
        engine.save_offline_account(account.clone());
        finish(&mut receiver)?;
        assert_eq!(super::load_accounts(root.path())?.len(), 1);
        let original = std::fs::read(root.path().join("accounts.json"))?;
        let mut forged = account.clone();
        forged.client_id = "must-not-be-treated-as-microsoft".into();
        engine.save_offline_account(forged);
        assert!(finish(&mut receiver).is_err());
        assert_eq!(std::fs::read(root.path().join("accounts.json"))?, original);
        engine.sign_out(account.clone());
        finish(&mut receiver)?;
        assert!(super::load_accounts(root.path())?.is_empty());
        std::fs::write(root.path().join("accounts.json"), b"corrupt")?;
        engine.save_offline_account(account);
        assert!(finish(&mut receiver).is_err());
        assert_eq!(
            std::fs::read(root.path().join("accounts.json"))?,
            b"corrupt"
        );
        Ok(())
    }

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
            sender: sender.into(),
            root: directory.path().to_owned(),
            prepared_sign_in: Default::default(),
        };
        engine.sign_in(block_launcher::CancellationToken::new(), false);
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

    #[test]
    fn new_game_settings_reach_the_profile_without_changing_existing_games() -> anyhow::Result<()> {
        use super::GameDefaults;
        use block_launcher::{CancellationToken, catalog::Catalog, recipe::RuntimeSelection};
        let root = tempfile::tempdir()?;
        let catalog = Catalog::open(root.path())?;
        let (_, existing) =
            catalog.create_instance("Existing".into(), "26.3".into(), None, None)?;
        let runtime = std::sync::Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .build()?,
        );
        let (sender, mut receiver) = tokio::sync::mpsc::channel(32);
        let engine = super::Engine {
            runtime: runtime.clone(),
            sender: sender.into(),
            root: root.path().to_owned(),
            prepared_sign_in: Default::default(),
        };
        let java = std::env::current_exe()?;
        let defaults = GameDefaults {
            memory_mib: 4096,
            java: java.display().to_string(),
            jvm_arguments: "-Dlabel=\"My world\"".into(),
            width: 1600,
            height: 900,
            fullscreen: true,
            show_snapshots: true,
        };
        let cancel = CancellationToken::new();
        cancel.cancel(); // Test profile creation, with installation prevented from networking.
        engine.create_instance_with_profile(
            "Configured".into(),
            "26.3".into(),
            None,
            false,
            defaults,
            None,
            cancel,
        );
        let profile_id = runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(3), async {
                while let Some(message) = receiver.recv().await {
                    if let super::Message::Created { profile, .. } = message {
                        return Ok::<_, anyhow::Error>(profile);
                    }
                }
                anyhow::bail!("instance creation did not report a saved profile")
            })
            .await
        })??;
        let state = catalog.snapshot()?;
        let (_, saved, _) = state.selection(&profile_id)?;
        assert_eq!(saved.memory_mib, 4096);
        assert!(matches!(&saved.runtime, RuntimeSelection::Local { java: path } if path == &java));
        assert_eq!(saved.jvm_args, ["-Dlabel=My world"]);
        assert_eq!(
            saved.game_args,
            ["--width", "1600", "--height", "900", "--fullscreen"]
        );
        assert_eq!(
            state.selection(&existing.id)?.1.memory_mib,
            existing.memory_mib
        );
        assert_eq!(
            state.selection(&existing.id)?.1.game_args,
            existing.game_args
        );
        Ok(())
    }

    #[test]
    fn saved_network_settings_route_version_metadata_through_a_real_loopback_proxy()
    -> anyhow::Result<()> {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let root = tempfile::tempdir()?;
        runtime.block_on(async {
            let proxy = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
            let origin = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
            let url = format!("http://{}/index.json", origin.local_addr()?);
            let mut preferences = super::Preferences::default();
            preferences.downloads.max_concurrent = 3;
            preferences.downloads.max_retries = 0;
            preferences.downloads.proxy = block_launcher::ProxyMode::Http {
                url: format!("http://{}", proxy.local_addr()?),
            };
            preferences.save(root.path())?;
            let (sender, _receiver) = tokio::sync::mpsc::channel(2);
            assert_eq!(
                super::library(root.path().to_owned(), &sender.into())
                    .await?
                    .download_options(),
                &preferences.downloads
            );
            let server = tokio::spawn(async move {
                let (mut stream, _) = proxy.accept().await?;
                let mut bytes = [0; 4096];
                let length = stream.read(&mut bytes).await?;
                let request = String::from_utf8_lossy(&bytes[..length]).into_owned();
                let body = br#"{"latest":{"release":"test","snapshot":"test"},"versions":[]}"#;
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                stream.write_all(header.as_bytes()).await?;
                stream.write_all(body).await?;
                Ok::<_, anyhow::Error>(request)
            });
            let versions = tokio::time::timeout(
                Duration::from_secs(2),
                super::version_index(root.path().to_owned(), &url),
            )
            .await??;
            assert_eq!(versions.len(), 0);
            assert!(server.await??.starts_with(&format!("GET {url} HTTP/1.1")));
            assert!(
                tokio::time::timeout(Duration::from_millis(30), origin.accept())
                    .await
                    .is_err()
            );
            Ok::<_, anyhow::Error>(())
        })
    }
}
