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
    catalog::{Catalog, LoaderKind, Profile, ProfileAppearance, ProfileConfiguration, Snapshot},
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

use super::{appearance::Preferences, global_config, settings::GameDefaults};

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
    Providers(Result<Vec<block_auth::AuthlibProvider>, String>),
    ProviderSaved(String),
    ForOperation {
        operation: u64,
        message: Box<Self>,
    },
    Versions(Result<Vec<VersionEntry>, String>),
    LoaderVersions {
        request: uuid::Uuid,
        result: Result<Vec<block_launcher::install::fabric::LoaderVersion>, String>,
    },
    GameDiscovery {
        request: uuid::Uuid,
        report: block_launcher::game_import::DiscoveryReport,
    },
    GameImported {
        request: uuid::Uuid,
        candidate: String,
        result: Result<(String, String), String>,
    },
    GameImportFinished {
        request: uuid::Uuid,
    },
    Catalog(Result<Snapshot, String>),
    Accounts(Result<Vec<Account>, String>),
    AccountImages(Vec<(String, block_launcher::avatars::SkinImage)>),
    ProviderImages(Vec<(String, block_launcher::avatars::ProviderIcon)>),
    AuthlibCharacters(Vec<block_auth::AuthlibProfile>),
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
    prepared_sign_in: MicrosoftSignInSlot,
    pending_authlib: Arc<Mutex<Option<PendingAuthlib>>>,
    prepared_authlib: AuthlibSignInSlot,
}

type PendingAuthlib = (CancellationToken, Arc<block_auth::PendingAuthlibSignIn>);
type MicrosoftSignInSlot = Arc<Mutex<Option<(CancellationToken, PreparedMicrosoftSignIn)>>>;
type AuthlibSignInSlot = Arc<Mutex<Option<(CancellationToken, block_auth::PreparedAuthlibSignIn)>>>;

impl Engine {
    pub(super) fn new() -> anyhow::Result<(Self, Receiver<Message>)> {
        let root = std::env::var_os("BLOCK_DATA_DIR").map_or_else(
            || {
                directories::BaseDirs::new()
                    .map(|dirs| {
                        dirs.data_dir()
                            .join(super::setup_storage::global_application_directory())
                    })
                    .context("cannot determine the data directory")
            },
            |path| Ok(PathBuf::from(path)),
        )?;
        Self::with_root(root)
    }

    pub(super) fn with_root(root: PathBuf) -> anyhow::Result<(Self, Receiver<Message>)> {
        block_launcher::storage::recover(&root)?;
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
            pending_authlib: Default::default(),
            prepared_authlib: Default::default(),
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
            let providers = load_providers(&root).map_err(|error| format!("{error:#}"));
            let _ = sender.send(Message::Providers(providers)).await;
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

    /// Discover existing game directories from only the roots selected by the UI.
    pub(super) fn discover_games(
        &self,
        request: uuid::Uuid,
        roots: block_launcher::game_import::DiscoveryRoots,
    ) {
        let sender = self.sender.unscoped();
        self.runtime.spawn(async move {
            let report =
                tokio::task::spawn_blocking(move || block_launcher::game_import::discover(&roots))
                    .await
                    .unwrap_or_default();
            let _ = sender
                .send(Message::GameDiscovery { request, report })
                .await;
        });
    }

    /// Copy and persist each selected game independently. One failure does not
    /// prevent later candidates from being imported; cancellation stops the
    /// remaining candidates while preserving already committed instances.
    pub(super) fn import_games(
        &self,
        request: uuid::Uuid,
        candidates: Vec<block_launcher::game_import::GameCandidate>,
        parent: PathBuf,
        defaults: GameDefaults,
        cancel: CancellationToken,
    ) {
        let root = self.root.clone();
        let sender = self.sender.unscoped();
        self.runtime.spawn(async move {
            for candidate in candidates {
                if cancel.is_cancelled() {
                    break;
                }
                let candidate_id = candidate.id.clone();
                let task_root = root.clone();
                let task_parent = parent.clone();
                let task_defaults = defaults.clone();
                let task_cancel = cancel.clone();
                let result = tokio::task::spawn_blocking(move || {
                    import_and_save_candidate(
                        &task_root,
                        candidate,
                        &task_parent,
                        task_defaults,
                        &task_cancel,
                    )
                })
                .await
                .map_err(|error| format!("Import task failed: {error}"))
                .and_then(|result| result.map_err(|error| format!("{error:#}")));
                match result {
                    Ok((instance, profile)) => {
                        let _ = sender
                            .send(Message::GameImported {
                                request,
                                candidate: candidate_id,
                                result: Ok((instance.id, profile.id)),
                            })
                            .await;
                        send_catalog(&root, &sender).await;
                    }
                    Err(error) => {
                        let _ = sender
                            .send(Message::GameImported {
                                request,
                                candidate: candidate_id,
                                result: Err(error),
                            })
                            .await;
                    }
                }
            }
            let _ = sender.send(Message::GameImportFinished { request }).await;
        });
    }

    /// Save a newly configured game without starting any installation or network work.
    #[expect(
        clippy::too_many_arguments,
        reason = "The setup-game commit passes explicit instance, target and profile boundaries."
    )]
    pub(super) fn save_setup_game(
        &self,
        name: String,
        version: String,
        directory: Option<PathBuf>,
        loader: Option<(LoaderKind, String)>,
        optifine: Option<block_launcher::install::optifine::OptiFineJar>,
        mut values: ProfileDraftValues,
        cancel: CancellationToken,
    ) {
        let root = self.root.clone();
        let sender = self.sender.clone();
        self.runtime.spawn(async move {
            let save_root = root.clone();
            let result = tokio::task::spawn_blocking(move || {
                ensure!(
                    !cancel.is_cancelled(),
                    "Cancelled before the game was saved."
                );
                let _storage = block_launcher::storage::acquire(&save_root)?;
                values.name = name.clone();
                let mut configuration = values.configuration()?;
                if let Some((kind, version)) = &loader {
                    configuration.target_id = format!("{}-{version}", kind.name().to_lowercase());
                }
                let catalog = Catalog::open(save_root)?;
                catalog.create_instance_with_components(
                    name,
                    version,
                    directory,
                    loader,
                    optifine,
                    configuration,
                )
            })
            .await
            .map_err(anyhow::Error::from)
            .and_then(|result| result);
            match result {
                Ok((instance, profile)) => {
                    let _ = sender
                        .send(Message::Created {
                            instance: instance.id,
                            profile: profile.id,
                        })
                        .await;
                    send_catalog(&root, &sender).await;
                    finish(&sender, Ok("Game saved".into())).await;
                }
                Err(error) => {
                    send_catalog(&root, &sender).await;
                    finish(&sender, Err(error)).await;
                }
            }
        });
    }

    /// Refresh real account avatars and provider icons without surfacing errors:
    /// every failure simply keeps the initials or monogram fallback.
    pub(super) fn load_images(&self) {
        let root = self.root.clone();
        let sender = self.sender.unscoped();
        self.runtime.spawn(async move {
            let cancel = CancellationToken::new();
            if let Ok(images) = account_images(&root, &cancel).await {
                let _ = sender.send(Message::AccountImages(images)).await;
            }
            if let Ok(icons) = provider_images(&root, &cancel).await {
                let _ = sender.send(Message::ProviderImages(icons)).await;
            }
        });
    }

    pub(super) fn test_connection(&self, request: super::connection_test::Request) {
        let root = self.root.clone();
        let sender = self.sender.unscoped();
        self.runtime.spawn(async move {
            let result = async {
                let storage = block_launcher::storage::acquire(&root)?;
                let store = Store::with_options(
                    storage.cache.clone(),
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

    pub(super) async fn import_optifine(
        &self,
        path: PathBuf,
    ) -> anyhow::Result<block_launcher::install::optifine::OptiFineJar> {
        let root = self.root.clone();
        self.runtime
            .spawn_blocking(move || {
                let storage = block_launcher::storage::acquire(&root)?;
                let context = ResolveContext {
                    store: Store::new(storage.cache.clone(), false, Arc::new(|_| {}))?,
                    cancel: CancellationToken::new(),
                };
                block_launcher::install::optifine::import(&path, &context)
            })
            .await?
    }

    pub(super) fn loader_versions(
        &self,
        request: uuid::Uuid,
        kind: LoaderKind,
        minecraft: String,
        cancel: CancellationToken,
    ) {
        let root = self.root.clone();
        let sender = self.sender.unscoped();
        self.runtime.spawn(async move {
            let result = async {
                let options = Preferences::load(&root)?.downloads;
                let storage = block_launcher::storage::acquire(&root)?;
                let store =
                    Store::with_options(storage.cache.clone(), false, Arc::new(|_| {}), options)?;
                let context = ResolveContext { store, cancel };
                match kind {
                    LoaderKind::Forge => {
                        block_launcher::install::forge::versions(&minecraft, &context).await
                    }
                    LoaderKind::NeoForge => {
                        block_launcher::install::neoforge::versions(&minecraft, &context).await
                    }
                    _ => {
                        block_launcher::install::fabric::versions(&kind, &minecraft, &context).await
                    }
                }
            }
            .await
            .map_err(|error| format!("{error:#}"));
            let _ = sender
                .send(Message::LoaderVersions { request, result })
                .await;
        });
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "The draft commit passes explicit instance and profile boundaries."
    )]
    pub(super) fn create_instance_with_profile(
        &self,
        pending_account: Option<Account>,
        name: String,
        version: String,
        directory: Option<PathBuf>,
        loader: Option<(LoaderKind, String)>,
        optifine: Option<block_launcher::install::optifine::OptiFineJar>,
        defaults: GameDefaults,
        profile: Option<ProfileDraftValues>,
        cancel: CancellationToken,
    ) {
        let root = self.root.clone();
        let sender = self.sender.clone();
        let prepared_sign_in = self.prepared_sign_in.clone();
        let prepared_authlib = self.prepared_authlib.clone();
        self.runtime.spawn(async move {
            let result = async {
                // First-run saves the verified identity and the game as one
                // commit: the identity is only written here, together with the
                // catalog entry, and is rolled back if the game cannot be saved.
                let committed = match pending_account {
                    None => None,
                    Some(account) => {
                        ensure!(
                            !cancel.is_cancelled(),
                            "Cancelled before the account was saved."
                        );
                        let (accounts, existed) = persist_prepared_identity(
                            &root,
                            &account,
                            &prepared_sign_in,
                            &prepared_authlib,
                        )
                        .await?;
                        let _ = sender
                            .unscoped()
                            .send(Message::Accounts(Ok(accounts)))
                            .await;
                        Some((account, existed))
                    }
                };
                let created = async {
                    // General catalog creation is unaffected by an already
                    // cancelled install token; only the first-run identity
                    // commit treats cancellation as a failure to roll back.
                    if committed.is_some() {
                        ensure!(
                            !cancel.is_cancelled(),
                            "Cancelled before the game was created."
                        );
                    }
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
                    if let Some((kind, version)) = &loader {
                        configuration.target_id =
                            format!("{}-{version}", kind.name().to_lowercase());
                    }
                    let catalog_root = root.clone();
                    let (instance, profile) = tokio::task::spawn_blocking(move || {
                        let catalog = Catalog::open(catalog_root)?;
                        catalog.create_instance_with_components(
                            name,
                            version,
                            directory,
                            loader,
                            optifine,
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
                match (created, committed) {
                    (Ok(status), _) => Ok(status),
                    (Err(error), Some((account, false))) => {
                        // The identity was saved for this commit only; remove its
                        // credential and metadata so a failed game save leaves
                        // neither half behind.
                        if let Err(rollback) = remove_persisted_account(&root, &account).await {
                            let _ = sender
                                .unscoped()
                                .send(Message::Accounts(Ok(
                                    load_accounts(&root).unwrap_or_default()
                                )))
                                .await;
                            return Err(error.context(format!(
                                "the new account could not be rolled back: {rollback:#}"
                            )));
                        }
                        let _ = sender
                            .unscoped()
                            .send(Message::Accounts(Ok(load_accounts(&root)?)))
                            .await;
                        Err(error)
                    }
                    (Err(error), _) => Err(error),
                }
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

    /// Save the Instance draft together with a Replace/Remove OptiFine choice,
    /// then reinstall that target so the saved JAR is actually applied.
    pub(super) fn save_instance_with_optifine(
        &self,
        id: String,
        configuration: block_launcher::catalog::InstanceConfiguration,
        target_id: String,
        optifine: Option<Box<block_launcher::install::optifine::OptiFineJar>>,
        cancel: CancellationToken,
    ) {
        let root = self.root.clone();
        let sender = self.sender.clone();
        self.runtime.spawn(async move {
            let catalog_root = root.clone();
            let write = {
                let id = id.clone();
                let target_id = target_id.clone();
                let selected = optifine.map(|selected| *selected);
                tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
                    Catalog::open(catalog_root)?.save_instance_with_optifine(
                        &id,
                        configuration,
                        &target_id,
                        selected,
                    )?;
                    Ok(())
                })
                .await
            };
            let result = match write {
                Ok(Ok(())) => {
                    async {
                        library(root.clone(), &sender)
                            .await?
                            .install(&id, &target_id, &cancel)
                            .await?;
                        Ok("Instance settings saved · OptiFine applied".to_owned())
                    }
                    .await
                }
                Ok(Err(error)) => Err(error),
                Err(error) => Err(error.into()),
            };
            send_catalog(&root, &sender).await;
            finish(&sender, result).await;
        });
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
            .pending_authlib
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            cancel.cancel();
        }
        if let Some((cancel, _)) = self
            .prepared_authlib
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            cancel.cancel();
        }
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

    pub(super) fn sign_in_authlib(
        &self,
        provider_id: uuid::Uuid,
        expected_api: String,
        login: String,
        password: block_auth::SecretString,
        cancel: CancellationToken,
    ) {
        let root = self.root.clone();
        let sender = self.sender.clone();
        let pending_slot = self.pending_authlib.clone();
        let prepared_slot = self.prepared_authlib.clone();
        self.runtime.spawn(async move {
            let result = async {
                let accounts = load_accounts(&root)?;
                let provider = provider_by_id(&root, provider_id)?;
                ensure!(provider.api_url==expected_api,"The provider changed before sign-in. Choose it again and re-enter your password.");
                let auth = block_auth::AuthlibAuth::new(&provider)?;
                let pending = auth.authenticate(&login, password, &cancel).await?;
                ensure!(!cancel.is_cancelled(), "Sign-in was cancelled.");
                if pending.selected_profile().is_some() || pending.profiles().len() == 1 {
                    let prepared = auth.prepare_sign_in(&pending, None, &cancel).await?;
                    let account = prepared.account().clone();
                    merge_signed_in_account(accounts, &account)?;
                    {
                        let mut slot = prepared_slot
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        ensure!(!cancel.is_cancelled(), "Sign-in was cancelled.");
                        *slot = Some((cancel.clone(), prepared));
                    }
                    let _ = sender.send(Message::AuthPrepared(account)).await;
                    Ok("External identity verified. Finish to save it securely.".into())
                } else {
                    let profiles = pending.profiles().to_vec();
                    {
                        let mut slot = pending_slot
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        ensure!(!cancel.is_cancelled(), "Sign-in was cancelled.");
                        *slot = Some((cancel.clone(), Arc::new(pending)));
                    }
                    let _ = sender.send(Message::AuthlibCharacters(profiles)).await;
                    Ok("Choose a character returned by your provider.".into())
                }
            }
            .await;
            finish(&sender, result).await;
        });
    }

    pub(super) fn select_authlib_character(
        &self,
        provider_id: uuid::Uuid,
        selected: uuid::Uuid,
        cancel: CancellationToken,
    ) {
        let root = self.root.clone();
        let sender = self.sender.clone();
        let pending_slot = self.pending_authlib.clone();
        let prepared_slot = self.prepared_authlib.clone();
        self.runtime.spawn(async move {
            let result = async {
                let pending = {
                    let slot = pending_slot
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    let (original, pending) = slot.as_ref().context(
                        "This provider sign-in is no longer available. Go back and sign in again.",
                    )?;
                    ensure!(!original.is_cancelled(), "Sign-in was cancelled.");
                    pending.clone()
                };
                let provider = provider_by_id(&root, provider_id)?;
                let prepared = block_auth::AuthlibAuth::new(&provider)?
                    .prepare_sign_in(&pending, Some(selected), &cancel)
                    .await?;
                let account = prepared.account().clone();
                merge_signed_in_account(load_accounts(&root)?, &account)?;
                {
                    let mut prepared_slot = prepared_slot
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    ensure!(!cancel.is_cancelled(), "Sign-in was cancelled.");
                    *prepared_slot = Some((cancel.clone(), prepared));
                }
                {
                    let mut slot = pending_slot
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    if slot
                        .as_ref()
                        .is_some_and(|(_, stored)| Arc::ptr_eq(stored, &pending))
                    {
                        slot.take();
                    }
                }
                let _ = sender.send(Message::AuthPrepared(account)).await;
                Ok("External character verified. Finish to save it securely.".into())
            }
            .await;
            finish(&sender, result).await;
        });
    }

    pub(super) fn save_authlib_sign_in(&self, expected: String) {
        let root = self.root.clone();
        let sender = self.sender.clone();
        let slot = self.prepared_authlib.clone();
        self.runtime.spawn(async move {
            let result=async {
                let (cancel,prepared)={
                    let mut slot=slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    ensure!(slot.as_ref().is_some_and(|(_,prepared)|prepared.account().identity_key()==expected),"This prepared character is no longer available. Sign in again.");
                    slot.take().context("Prepared external sign-in is unavailable")?
                };
                ensure!(!cancel.is_cancelled(),"Sign-in was cancelled.");
                let account=prepared.account().clone();
                let link=account.authlib.as_ref().context("Missing account provider")?;
                let provider=provider_by_id(&root,link.provider_id)?;
                let (_,existed)=merge_signed_in_account(load_accounts(&root)?,&account)?;
                let auth=block_auth::AuthlibAuth::new(&provider)?;
                // Finish authorizes the durable transaction. Closing the window
                // after this point cannot leave half-saved account metadata.
                auth.save_sign_in(prepared,&CancellationToken::new()).await?;
                let saved=(|| {
                    ensure!(provider_by_id(&root,link.provider_id)?.api_url==provider.api_url,"The provider API changed during sign-in. Sign in again.");
                    let (accounts,_)=merge_signed_in_account(load_accounts(&root)?,&account)?;
                    save_accounts(&root,&accounts)?; Ok::<_,anyhow::Error>(accounts)
                })();
                let accounts=match saved {
                    Ok(accounts)=>accounts,
                    Err(error)=> {if !existed {block_auth::AuthlibAuth::forget(&account).await.context("Account metadata could not be saved and its new credential could not be removed")?;}return Err(error);}
                };
                let name=account.player_name.clone();
                let _=sender.unscoped().send(Message::Accounts(Ok(accounts))).await;
                let _=sender.send(Message::SignedIn(account)).await;
                Ok(format!("External account {name} saved securely"))
            }.await;
            finish(&sender,result).await;
        });
    }

    pub(super) fn save_provider(
        &self,
        original: Option<block_auth::AuthlibProvider>,
        name: String,
        address: String,
        allow_http: bool,
        cancel: CancellationToken,
    ) {
        let root = self.root.clone();
        let sender = self.sender.clone();
        self.runtime.spawn(async move {
            let result = async {
                let address = block_auth::ProviderAddress::parse(&address)?;
                let provider = if let Some(provider) = &original
                    && address.url == provider.api_url
                    && (!address.insecure || allow_http)
                {
                    let mut provider = provider.clone();
                    provider.name = name;
                    provider.validate()?;
                    provider
                } else {
                    block_auth::AuthlibProvider::discover(
                        original.as_ref().map_or_else(uuid::Uuid::new_v4, |p| p.id),
                        &name,
                        &address.url,
                        allow_http,
                        &cancel,
                    )
                    .await?
                };
                ensure!(!cancel.is_cancelled(), "Provider setup was cancelled.");
                persist_provider(&root, original.as_ref(), &provider)?;
                let _ = sender
                    .unscoped()
                    .send(Message::Providers(Ok(load_providers(&root)?)))
                    .await;
                let _ = sender
                    .send(Message::ProviderSaved(provider.id.to_string()))
                    .await;
                Ok(format!("Provider {} saved", provider.name))
            }
            .await;
            finish(&sender, result).await;
        });
    }

    pub(super) fn remove_provider(&self, id: String) {
        let root = self.root.clone();
        let sender = self.sender.clone();
        self.runtime.spawn(async move {
            let result = async {
                let providers = delete_provider(&root, id.parse()?)?;
                let _ = sender
                    .unscoped()
                    .send(Message::Providers(Ok(providers)))
                    .await;
                Ok("Provider removed; game files are kept".into())
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
                let revocation_warning = remove_persisted_account(&root, &account).await?;
                let _ = sender
                    .send(Message::Accounts(Ok(load_accounts(&root)?)))
                    .await;
                Ok(if account.provider == AccountProvider::Offline {
                    "Offline identity removed · game files kept"
                } else if revocation_warning {
                    "Local credentials removed · provider revocation could not be confirmed"
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
                    AccountProvider::Authlib => {
                        load_accounts(&root)?;
                        let link = account
                            .authlib
                            .as_ref()
                            .context("The external account has no provider")?;
                        let saved = provider_by_id(&root, link.provider_id)?;
                        let session = block_auth::AuthlibAuth::new(&saved)?
                            .refresh(&account, &cancel)
                            .await?;
                        let fresh = block_auth::AuthlibProvider::discover(
                            saved.id,
                            &saved.name,
                            &saved.api_url,
                            saved.allow_http,
                            &cancel,
                        )
                        .await?;
                        ensure!(
                            fresh.api_url == saved.api_url,
                            "The provider API changed. Sign in again before launching."
                        );
                        let _ = sender
                            .send(Message::Status(
                                "Preparing the official Authlib agent · verifying SHA-256".into(),
                            ))
                            .await;
                        let tracker =
                            Arc::new(Mutex::new(super::tasks::DownloadTracker::default()));
                        let _ = sender.send(Message::Downloads(tracker.clone())).await;
                        let storage = block_launcher::storage::acquire(&root)?;
                        let store = Store::with_options(
                            storage.cache.clone(),
                            false,
                            Arc::new(move |event| {
                                tracker
                                    .lock()
                                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                                    .apply(event);
                            }),
                            Preferences::load(&root)?.downloads,
                        )?;
                        let agent =
                            block_launcher::install::authlib::prepare(&store, &cancel).await?;
                        ensure!(
                            provider_by_id(&root, link.provider_id)?.api_url == saved.api_url,
                            "The provider changed while preparing. Sign in again."
                        );
                        let (accounts, _) =
                            merge_signed_in_account(load_accounts(&root)?, &session.account)?;
                        save_accounts(&root, &accounts)?;
                        let _ = sender
                            .unscoped()
                            .send(Message::Accounts(Ok(accounts)))
                            .await;
                        let mut game = game_session(&session);
                        game.authlib = Some(block_launcher::session::AuthlibLaunch {
                            api_url: fresh.api_url,
                            metadata_json: fresh.metadata_json,
                            agent: agent.path,
                            sha256: agent.sha256,
                        });
                        OnlineSession {
                            game,
                            expires_at: session.expires_at,
                        }
                    }
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
        user_type: if session.account.provider == AccountProvider::Authlib {
            "mojang"
        } else {
            "msa"
        }
        .into(),
        xuid: String::new(),
        client_id: session.account.client_id.clone(),
        user_properties: session.expose_user_properties().to_owned(),
        authlib: None,
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
    let storage = block_launcher::storage::acquire(&root)?;
    let store = Store::with_options(storage.cache.clone(), false, Arc::new(|_| {}), options)?;
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

fn import_and_save_candidate(
    root: &Path,
    candidate: block_launcher::game_import::GameCandidate,
    parent: &Path,
    defaults: GameDefaults,
    cancel: &CancellationToken,
) -> anyhow::Result<(block_launcher::catalog::Instance, Profile)> {
    ensure!(
        candidate.importable,
        "{}",
        candidate
            .unsupported_reason
            .as_deref()
            .unwrap_or("candidate is not importable")
    );
    let minecraft_version = candidate
        .minecraft_version
        .clone()
        .context("Minecraft version could not be resolved from local metadata")?;
    ensure!(parent.is_dir(), "game destination parent does not exist");
    ensure!(!cancel.is_cancelled(), "Import cancelled");

    let loader = match candidate.loader.clone() {
        None
        | Some(block_launcher::game_import::DetectedLoader {
            kind: block_launcher::game_import::LoaderKind::Vanilla,
            ..
        }) => None,
        Some(detected) => {
            let kind = match detected.kind {
                block_launcher::game_import::LoaderKind::Vanilla => LoaderKind::Vanilla,
                block_launcher::game_import::LoaderKind::Fabric => LoaderKind::Fabric,
                block_launcher::game_import::LoaderKind::Quilt => LoaderKind::Quilt,
                block_launcher::game_import::LoaderKind::Forge => LoaderKind::Forge,
                block_launcher::game_import::LoaderKind::NeoForge => LoaderKind::NeoForge,
            };
            let mut version = detected
                .version
                .context("Loader version could not be resolved from local metadata")?;
            if kind == LoaderKind::Forge && !version.starts_with(&format!("{minecraft_version}-")) {
                version = format!("{minecraft_version}-{version}");
            } else if kind == LoaderKind::NeoForge
                && minecraft_version == "1.20.1"
                && !version.starts_with("1.20.1-")
            {
                version = format!("1.20.1-{version}");
            }
            Some((kind, version))
        }
    };
    let values = ProfileDraftValues {
        name: candidate.name.clone(),
        defaults,
        appearance: ProfileAppearance::default(),
        target_id: None,
        server: block_launcher::catalog::ServerPreferences::default(),
    };
    let mut configuration = values.configuration()?;
    if let Some((kind, version)) = &loader {
        configuration.target_id = format!("{}-{version}", kind.name().to_lowercase());
    }
    // Hold the shared storage lease from before copy publication through the
    // catalog transaction so relocation cannot race the imported directory.
    let _storage = block_launcher::storage::acquire(root)?;
    ensure!(!cancel.is_cancelled(), "Import cancelled");
    let destination = parent.join(uuid::Uuid::new_v4().to_string());
    let copied = block_launcher::game_import::copy_candidate(&candidate, &destination, cancel)?;
    if cancel.is_cancelled() {
        let _ = std::fs::remove_dir_all(&copied.target_directory);
        anyhow::bail!("Import cancelled");
    }
    let saved = (|| {
        let catalog = Catalog::open(root)?;
        catalog.create_instance_with_components(
            candidate.name,
            minecraft_version,
            Some(copied.target_directory.clone()),
            loader,
            None,
            configuration,
        )
    })();
    if saved.is_err() {
        // The target is a fresh UUID path published by this operation. Never
        // remove an existing destination or any source data on rollback.
        let _ = std::fs::remove_dir_all(&copied.target_directory);
    }
    saved
}

async fn finish(sender: &EventSender, result: anyhow::Result<String>) {
    let _ = sender
        .send(Message::Finished(
            result.map_err(|error| format!("{error:#}")),
        ))
        .await;
}

fn load_accounts(root: &Path) -> anyhow::Result<Vec<Account>> {
    match global_config::read(root) {
        Ok(Some(config)) => Ok(config.accounts),
        Ok(None) => Ok(Vec::new()),
        Err(error) => Err(error).context("Saved account metadata could not be read"),
    }
}

/// Resolve every saved account's real skin through its own provider. Any
/// failure is skipped: the UI keeps the initials fallback, silently.
async fn account_images(
    root: &Path,
    cancel: &CancellationToken,
) -> anyhow::Result<Vec<(String, block_launcher::avatars::SkinImage)>> {
    let accounts = load_accounts(root)?;
    // Avatars are display-only: a damaged configuration must not prevent the
    // UI from rendering, so this path uses the lenient loader instead of the
    // strict account/provider readers.
    let providers = global_config::load(root).providers;
    let fetch = block_launcher::avatars::ImageFetch::new()?;
    let cache = block_launcher::avatars::ImageCache::new(root);
    let mut images = Vec::new();
    for account in accounts {
        let identity = account.identity_key();
        let Some((url, domains)) = skin_source(&account, &providers, cancel).await else {
            continue;
        };
        if let Some(image) = cache.skin(&identity, &url) {
            images.push((identity, image));
            continue;
        }
        let Ok(image) = fetch.skin(&url, &domains, cancel).await else {
            continue;
        };
        let _ = cache.store_skin(&identity, &url, &image);
        images.push((identity, image));
    }
    Ok(images)
}

async fn skin_source(
    account: &Account,
    providers: &[block_auth::AuthlibProvider],
    cancel: &CancellationToken,
) -> Option<(String, Vec<String>)> {
    match account.provider {
        AccountProvider::Offline => None,
        AccountProvider::Microsoft => {
            let auth = auth().ok()?;
            let session = auth.refresh(account, cancel).await.ok()?;
            let skin = auth.profile_skin(&session, cancel).await.ok()??;
            Some((skin.url, vec!["textures.minecraft.net".to_owned()]))
        }
        AccountProvider::Authlib => {
            let link = account.authlib.as_ref()?;
            let provider = providers
                .iter()
                .find(|provider| provider.id == link.provider_id)?;
            let auth = block_auth::AuthlibAuth::new(provider).ok()?;
            let session = auth.refresh(account, cancel).await.ok()?;
            let skin = block_auth::skin_texture(&session, provider)?;
            Some((skin.url, provider.metadata.skin_domains.clone()))
        }
    }
}

/// Resolve provider icons from the discovered icon URL, bounded to its API host.
async fn provider_images(
    root: &Path,
    cancel: &CancellationToken,
) -> anyhow::Result<Vec<(String, block_launcher::avatars::ProviderIcon)>> {
    let providers = load_providers(root)?;
    let fetch = block_launcher::avatars::ImageFetch::new()?;
    let cache = block_launcher::avatars::ImageCache::new(root);
    let mut icons = Vec::new();
    for provider in providers {
        let id = provider.id.to_string();
        let source = provider.metadata.icon_url.clone();
        if let Some(icon) = cache.icon(&id, &source) {
            icons.push((id, icon));
            continue;
        }
        let Ok(icon) = fetch.icon(&source, &provider.api_url, cancel).await else {
            continue;
        };
        let _ = cache.store_icon(&id, &source, &icon);
        icons.push((id, icon));
    }
    Ok(icons)
}

fn load_providers(root: &Path) -> anyhow::Result<Vec<block_auth::AuthlibProvider>> {
    match global_config::read(root) {
        Ok(Some(config)) => {
            validate_providers(&config.providers)?;
            Ok(config.providers)
        }
        Ok(None) => Ok(Vec::new()),
        Err(error) => Err(error).context("Saved provider metadata could not be read"),
    }
}

fn provider_by_id(root: &Path, id: uuid::Uuid) -> anyhow::Result<block_auth::AuthlibProvider> {
    load_providers(root)?
        .into_iter()
        .find(|provider| provider.id == id)
        .context("This provider is no longer saved. Add it again before signing in.")
}

fn validate_providers(providers: &[block_auth::AuthlibProvider]) -> anyhow::Result<()> {
    ensure!(providers.len() <= 64, "At most 64 providers may be saved");
    for (index, provider) in providers.iter().enumerate() {
        provider.validate()?;
        ensure!(
            !providers[..index]
                .iter()
                .any(|p| p.id == provider.id || p.api_url == provider.api_url),
            "A provider with this API address is already saved"
        );
    }
    Ok(())
}

fn delete_provider(
    root: &Path,
    id: uuid::Uuid,
) -> anyhow::Result<Vec<block_auth::AuthlibProvider>> {
    let mut providers = load_providers(root)?;
    ensure!(
        !load_accounts(root)?.iter().any(|a| a
            .authlib
            .as_ref()
            .is_some_and(|link| link.provider_id == id)),
        "This provider is used by saved accounts. Remove those accounts before removing the provider."
    );
    let count = providers.len();
    providers.retain(|provider| provider.id != id);
    ensure!(
        providers.len() != count,
        "This provider is no longer saved."
    );
    save_providers(root, &providers)?;
    Ok(providers)
}

fn save_providers(root: &Path, providers: &[block_auth::AuthlibProvider]) -> anyhow::Result<()> {
    validate_providers(providers)?;
    ensure!(
        serde_json::to_vec_pretty(providers)?.len() <= 16 * 1024 * 1024,
        "Saved providers are too large"
    );
    global_config::update(root, |config| config.providers = providers.to_vec())
}

fn persist_provider(
    root: &Path,
    original: Option<&block_auth::AuthlibProvider>,
    provider: &block_auth::AuthlibProvider,
) -> anyhow::Result<()> {
    let mut providers = load_providers(root)?;
    let saved = providers.iter().find(|p| p.id == provider.id);
    ensure!(
        saved == original,
        "The provider changed while editing. Open it again before saving."
    );
    if original.is_some_and(|old| old.api_url != provider.api_url) {
        ensure!(
            !load_accounts(root)?.iter().any(|a| a
                .authlib
                .as_ref()
                .is_some_and(|link| link.provider_id == provider.id)),
            "This provider is used by saved accounts. Its API address cannot change while accounts are linked."
        );
    }
    providers.retain(|p| p.id != provider.id);
    providers.push(provider.clone());
    save_providers(root, &providers)
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
        authlib: None,
    })
}

fn merge_signed_in_account(
    mut accounts: Vec<Account>,
    account: &Account,
) -> anyhow::Result<(Vec<Account>, bool)> {
    ensure!(
        !accounts
            .iter()
            .any(|saved| saved.identity_key() == account.identity_key()
                && (saved.provider != account.provider
                    || saved.credential_ref != account.credential_ref)),
        "This player already has a saved account for a different provider or Microsoft application registration. Remove that account before adding it again."
    );
    let existed = accounts.iter().any(|saved| {
        saved.identity_key() == account.identity_key()
            && saved.provider == account.provider
            && saved.credential_ref == account.credential_ref
    });
    accounts.retain(|saved| saved.identity_key() != account.identity_key());
    accounts.push(account.clone());
    Ok((accounts, existed))
}

fn save_accounts(root: &Path, accounts: &[Account]) -> anyhow::Result<()> {
    global_config::update(root, |config| config.accounts = accounts.to_vec())
}

/// Durably saves one verified identity and returns the updated account list.
/// Preparation alone never reaches secure storage; only this call does.
async fn persist_prepared_identity(
    root: &Path,
    account: &Account,
    prepared_sign_in: &MicrosoftSignInSlot,
    prepared_authlib: &AuthlibSignInSlot,
) -> anyhow::Result<(Vec<Account>, bool)> {
    match account.provider {
        AccountProvider::Offline => {
            ensure!(
                account == &offline_account(&account.player_name)?,
                "The offline identity is invalid."
            );
            let (accounts, existed) = merge_signed_in_account(load_accounts(root)?, account)?;
            save_accounts(root, &accounts)?;
            Ok((accounts, existed))
        }
        AccountProvider::Microsoft => {
            let (cancel, prepared) = {
                let mut pending = prepared_sign_in
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                ensure!(
                    pending.as_ref().is_some_and(|(_, prepared)| {
                        prepared.account().reference == account.reference
                    }),
                    "The prepared identity is no longer available. Sign in again."
                );
                pending.take().context("Prepared sign-in is unavailable")?
            };
            ensure!(!cancel.is_cancelled(), "Sign-in was cancelled.");
            let signed_in = prepared.account().clone();
            let (accounts, existed) = merge_signed_in_account(load_accounts(root)?, &signed_in)?;
            let auth = auth()?;
            auth.save_sign_in(prepared).await?;
            if let Err(error) = save_accounts(root, &accounts) {
                if !existed {
                    auth.sign_out(&signed_in).await.context("Account metadata could not be saved, and the new credential could not be removed from secure storage")?;
                }
                return Err(error);
            }
            Ok((accounts, existed))
        }
        AccountProvider::Authlib => {
            let link = account
                .authlib
                .as_ref()
                .context("Missing account provider")?;
            let (cancel, prepared) = {
                let mut slot = prepared_authlib
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                ensure!(
                    slot.as_ref().is_some_and(|(_, prepared)| {
                        prepared.account().identity_key() == account.identity_key()
                    }),
                    "This prepared character is no longer available. Sign in again."
                );
                slot.take()
                    .context("Prepared external sign-in is unavailable")?
            };
            ensure!(!cancel.is_cancelled(), "Sign-in was cancelled.");
            let signed_in = prepared.account().clone();
            let provider = provider_by_id(root, link.provider_id)?;
            let (_, existed) = merge_signed_in_account(load_accounts(root)?, &signed_in)?;
            let auth = block_auth::AuthlibAuth::new(&provider)?;
            auth.save_sign_in(prepared, &CancellationToken::new())
                .await?;
            let saved = (|| {
                ensure!(
                    provider_by_id(root, link.provider_id)?.api_url == provider.api_url,
                    "The provider API changed during sign-in. Sign in again."
                );
                let (accounts, _) = merge_signed_in_account(load_accounts(root)?, &signed_in)?;
                save_accounts(root, &accounts)?;
                Ok::<_, anyhow::Error>(accounts)
            })();
            match saved {
                Ok(accounts) => Ok((accounts, existed)),
                Err(error) => {
                    if !existed {
                        block_auth::AuthlibAuth::forget(&signed_in).await.context("Account metadata could not be saved and its new credential could not be removed")?;
                    }
                    Err(error)
                }
            }
        }
    }
}

/// Removes an account's durable credentials and metadata. Returns whether a
/// provider-side revocation could not be confirmed.
async fn remove_persisted_account(root: &Path, account: &Account) -> anyhow::Result<bool> {
    let mut accounts = load_accounts(root)?;
    let mut revocation_warning = false;
    if account.provider == AccountProvider::Authlib {
        if let Some(link) = &account.authlib {
            let remote = async {
                let provider = provider_by_id(root, link.provider_id)?;
                block_auth::AuthlibAuth::new(&provider)?
                    .invalidate(account, &CancellationToken::new())
                    .await?;
                Ok::<_, anyhow::Error>(())
            }
            .await;
            revocation_warning = remote.is_err();
        }
        block_auth::AuthlibAuth::forget(account).await?;
    } else if account.provider == AccountProvider::Microsoft {
        MicrosoftAuth::new(&account.client_id)?
            .sign_out(account)
            .await?;
    }
    accounts.retain(|saved| saved.identity_key() != account.identity_key());
    save_accounts(root, &accounts)?;
    Ok(revocation_warning)
}

async fn run_recipe(
    recipe: LaunchRecipe,
    root: PathBuf,
    sender: EventSender,
    cancel: CancellationToken,
) -> anyhow::Result<Option<i32>> {
    let _storage = block_launcher::storage::acquire(&root)?;
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
    fn provider_persistence_preserves_drafts_duplicates_links_and_corrupt_files()
    -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let provider = crate::providers::fixture_provider(false)?;
        super::persist_provider(root.path(), None, &provider)?;
        assert_eq!(super::load_providers(root.path())?, vec![provider.clone()]);
        let original_bytes = std::fs::read(root.path().join(crate::global_config::FILE))?;
        let mut duplicate = provider.clone();
        duplicate.id = uuid::Uuid::new_v4();
        assert!(super::persist_provider(root.path(), None, &duplicate).is_err());
        assert_eq!(
            std::fs::read(root.path().join(crate::global_config::FILE))?,
            original_bytes
        );
        let mut account = super::offline_account("Player")?;
        account.provider = block_auth::AccountProvider::Authlib;
        account.authlib = Some(block_auth::AuthlibAccount {
            provider_id: provider.id,
            login: "fixture-user".into(),
        });
        super::save_accounts(root.path(), &[account])?;
        assert!(super::delete_provider(root.path(), provider.id).is_err());
        let mut moved = crate::providers::fixture_provider(true)?;
        moved.id = provider.id;
        assert!(super::persist_provider(root.path(), Some(&provider), &moved).is_err());
        let mut renamed = provider.clone();
        renamed.name = "Renamed community".into();
        super::persist_provider(root.path(), Some(&provider), &renamed)?;
        assert!(super::persist_provider(root.path(), Some(&provider), &provider).is_err());
        assert_eq!(super::load_providers(root.path())?, vec![renamed]);
        super::save_accounts(root.path(), &[])?;
        assert!(super::delete_provider(root.path(), provider.id)?.is_empty());
        std::fs::write(
            root.path().join(crate::global_config::FILE),
            b"broken cache",
        )?;
        assert!(super::load_providers(root.path()).is_err());
        assert!(super::persist_provider(root.path(), None, &provider).is_err());
        assert_eq!(
            std::fs::read(root.path().join(crate::global_config::FILE))?,
            b"broken cache"
        );
        Ok(())
    }

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
            pending_authlib: Default::default(),
            prepared_authlib: Default::default(),
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
        let original = std::fs::read(root.path().join(crate::global_config::FILE))?;
        let mut forged = account.clone();
        forged.client_id = "must-not-be-treated-as-microsoft".into();
        engine.save_offline_account(forged);
        assert!(finish(&mut receiver).is_err());
        assert_eq!(
            std::fs::read(root.path().join(crate::global_config::FILE))?,
            original
        );
        engine.sign_out(account.clone());
        finish(&mut receiver)?;
        assert!(super::load_accounts(root.path())?.is_empty());
        std::fs::write(root.path().join(crate::global_config::FILE), b"corrupt")?;
        engine.save_offline_account(account);
        assert!(finish(&mut receiver).is_err());
        assert_eq!(
            std::fs::read(root.path().join(crate::global_config::FILE))?,
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
            authlib: None,
        };
        super::save_accounts(root.path(), std::slice::from_ref(&account))?;
        assert_eq!(super::load_accounts(root.path())?, vec![account]);
        super::save_accounts(root.path(), &[])?;
        assert_eq!(super::load_accounts(root.path())?, Vec::new());
        std::fs::write(
            root.path().join(crate::global_config::FILE),
            b"invalid catalog",
        )?;
        assert!(super::load_accounts(root.path()).is_err());
        assert_eq!(
            std::fs::read(root.path().join(crate::global_config::FILE))?,
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
            authlib: None,
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
    fn external_password_exchange_rechecks_the_exact_provider_address_before_io()
    -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let mut provider = crate::providers::fixture_provider(true)?;
        provider.api_url = format!("http://{}/api/", listener.local_addr()?);
        provider.metadata.icon_url = format!("http://{}/favicon.ico", listener.local_addr()?);
        provider.validate()?;
        super::save_providers(directory.path(), &[provider.clone()])?;
        let (engine, mut receiver) = super::Engine::with_root(directory.path().into())?;
        engine.sign_in_authlib(
            provider.id,
            "https://old.example.test/api/".into(),
            "User".into(),
            block_auth::SecretString::new("SYNTHETIC_PASSWORD".into()),
            block_launcher::CancellationToken::new(),
        );
        let message = engine.runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(3), receiver.recv()).await
        })?;
        assert!(
            matches!(message,Some(super::Message::Finished(Err(error))) if error.contains("provider changed before sign-in")&&!error.contains("SYNTHETIC_PASSWORD"))
        );
        assert!(
            listener
                .accept()
                .is_err_and(|error| error.kind() == std::io::ErrorKind::WouldBlock)
        );
        assert!(super::load_accounts(directory.path())?.is_empty());
        Ok(())
    }

    #[test]
    fn corrupt_account_metadata_blocks_oauth_before_any_browser_or_credential_operation()
    -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        std::fs::write(
            directory.path().join(crate::global_config::FILE),
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
            pending_authlib: Default::default(),
            prepared_authlib: Default::default(),
        };
        engine.sign_in(block_launcher::CancellationToken::new(), false);
        let message = runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(3), receiver.recv()).await
        })?;
        assert!(
            matches!(message, Some(super::Message::Finished(Err(error))) if error.contains("Saved account metadata could not be read"))
        );
        assert_eq!(
            std::fs::read(directory.path().join(crate::global_config::FILE))?,
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
            pending_authlib: Default::default(),
            prepared_authlib: Default::default(),
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
            None,
            "Configured".into(),
            "26.3".into(),
            None,
            None,
            None,
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
    fn selected_loader_reaches_atomic_creation_instead_of_falling_back_to_vanilla()
    -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
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
            root: root.path().into(),
            prepared_sign_in: Default::default(),
            pending_authlib: Default::default(),
            prepared_authlib: Default::default(),
        };
        for (kind, version, minecraft) in [
            (super::LoaderKind::Fabric, "0.16.14", "1.20.6"),
            (super::LoaderKind::Quilt, "0.29.2", "1.20.6"),
            (super::LoaderKind::NeoForge, "21.1.219", "1.21.1"),
            (super::LoaderKind::Forge, "1.20.1-47.4.13", "1.20.1"),
        ] {
            let cancel = super::CancellationToken::new();
            cancel.cancel();
            engine.create_instance_with_profile(
                None,
                kind.name().into(),
                minecraft.into(),
                None,
                Some((kind.clone(), version.into())),
                None,
                super::GameDefaults::default(),
                None,
                cancel,
            );
            let profile = runtime.block_on(async {
                tokio::time::timeout(Duration::from_secs(5), async {
                    let mut profile = None;
                    while let Some(message) = receiver.recv().await {
                        match message {
                            super::Message::Created { profile: id, .. } => profile = Some(id),
                            super::Message::Finished(result) => {
                                assert!(result.is_err());
                                return profile
                                    .ok_or_else(|| anyhow::anyhow!("Missing saved profile"));
                            }
                            _ => {}
                        }
                    }
                    anyhow::bail!("Creation channel closed")
                })
                .await
            })??;
            let state = super::Catalog::open(root.path())?.snapshot()?;
            let (instance, saved, target) = state.selection(&profile)?;
            assert_eq!(instance.minecraft_version, minecraft);
            assert_eq!(
                saved.target_id,
                format!("{}-{version}", kind.name().to_lowercase())
            );
            assert_eq!(target.kind, kind);
            assert_eq!(target.loader_version.as_deref(), Some(version));
            assert!(matches!(
                target.status,
                block_launcher::catalog::InstallStatus::Failed { .. }
            ));
        }
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

    fn fixture_engine(
        root: &std::path::Path,
    ) -> anyhow::Result<(
        super::Engine,
        std::sync::Arc<tokio::runtime::Runtime>,
        tokio::sync::mpsc::Receiver<super::Message>,
    )> {
        let runtime = std::sync::Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .build()?,
        );
        let (sender, receiver) = tokio::sync::mpsc::channel(64);
        let engine = super::Engine {
            runtime: runtime.clone(),
            sender: sender.into(),
            root: root.to_owned(),
            prepared_sign_in: Default::default(),
            pending_authlib: Default::default(),
            prepared_authlib: Default::default(),
        };
        Ok((engine, runtime, receiver))
    }

    #[test]
    fn imported_games_commit_independently_and_remain_pending() -> anyhow::Result<()> {
        use block_launcher::game_import::{GameCandidate, SourceKind};
        let root = tempfile::tempdir()?;
        let destination_parent = root.path().join("imported");
        std::fs::create_dir(&destination_parent)?;
        let source_one = root.path().join("source-one");
        let source_three = root.path().join("source-three");
        std::fs::create_dir(&source_one)?;
        std::fs::create_dir(&source_three)?;
        std::fs::write(source_one.join("options.txt"), b"one")?;
        std::fs::write(source_three.join("options.txt"), b"three")?;
        let candidate =
            |id: &str, name: &str, version: &str, path: &std::path::Path| GameCandidate {
                id: id.into(),
                source: SourceKind::AdjacentMinecraft,
                source_root: path.to_owned(),
                name: name.into(),
                game_directory: path.to_owned(),
                configured_version: Some(version.into()),
                minecraft_version: Some(version.into()),
                loader: None,
                latest_release: None,
                latest_snapshot: None,
                importable: true,
                unsupported_reason: None,
                metadata_warning: None,
            };
        let first = candidate("one", "Imported One", "1.20.1", &source_one);
        let mut failed = candidate("failed", "Missing", "1.20.1", &source_one);
        failed.importable = false;
        failed.unsupported_reason = Some("synthetic unsupported fixture".into());
        let third = candidate("three", "Imported Three", "1.21.1", &source_three);
        let (engine, runtime, mut receiver) = fixture_engine(root.path())?;
        let request = uuid::Uuid::new_v4();
        engine.import_games(
            request,
            vec![first, failed, third],
            destination_parent,
            super::GameDefaults::default(),
            super::CancellationToken::new(),
        );
        let (successes, failures, finished) = runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(5), async {
                let mut successes = Vec::new();
                let mut failures = Vec::new();
                let mut finished = false;
                while let Some(message) = receiver.recv().await {
                    match message {
                        super::Message::GameImported {
                            request: got,
                            candidate,
                            result,
                        } if got == request => match result {
                            Ok((instance, profile)) => {
                                successes.push((candidate, instance, profile))
                            }
                            Err(error) => failures.push((candidate, error)),
                        },
                        super::Message::GameImportFinished { request: got } if got == request => {
                            finished = true;
                            break;
                        }
                        _ => {}
                    }
                }
                Ok::<_, anyhow::Error>((successes, failures, finished))
            })
            .await
        })??;
        assert!(finished);
        assert_eq!(successes.len(), 2);
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].0, "failed");
        let state = super::Catalog::open(root.path())?.snapshot()?;
        assert_eq!(state.instances.len(), 2);
        for (_, instance_id, profile_id) in successes {
            let instance = state
                .instances
                .iter()
                .find(|entry| entry.id == instance_id)
                .ok_or_else(|| anyhow::anyhow!("imported instance missing from catalog"))?;
            assert!(instance.directory.join("options.txt").is_file());
            assert!(instance.targets.iter().all(|target| matches!(
                &target.status,
                block_launcher::catalog::InstallStatus::Pending
            )));
            assert_eq!(state.selection(&profile_id)?.0.id, instance_id);
        }
        Ok(())
    }

    #[test]
    fn setup_game_save_persists_pending_profile_without_network_install() -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let (engine, runtime, mut receiver) = fixture_engine(root.path())?;
        engine.save_setup_game(
            "Setup Game".into(),
            "1.21.1".into(),
            None,
            None,
            None,
            super::ProfileDraftValues {
                name: "Draft name is replaced".into(),
                defaults: super::GameDefaults::default(),
                appearance: super::ProfileAppearance::default(),
                target_id: None,
                server: block_launcher::catalog::ServerPreferences::default(),
            },
            super::CancellationToken::new(),
        );
        let (created, catalog, finished) = runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(5), async {
                let mut created = false;
                let mut catalog = false;
                let mut finished = false;
                while let Some(message) = receiver.recv().await {
                    match message {
                        super::Message::Created { .. } => created = true,
                        super::Message::Catalog(Ok(_)) => catalog = true,
                        super::Message::Finished(Ok(_)) => {
                            finished = true;
                            break;
                        }
                        super::Message::Finished(Err(error)) => anyhow::bail!("{error}"),
                        _ => {}
                    }
                }
                Ok::<_, anyhow::Error>((created, catalog, finished))
            })
            .await
        })??;
        assert!(created && catalog && finished);
        let state = super::Catalog::open(root.path())?.snapshot()?;
        assert_eq!(state.instances.len(), 1);
        assert_eq!(state.instances[0].name, "Setup Game");
        assert!(state.instances[0].targets.iter().all(|target| matches!(
            &target.status,
            block_launcher::catalog::InstallStatus::Pending
        )));
        assert!(!root.path().join("versions").exists());
        Ok(())
    }
}
