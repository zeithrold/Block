use std::{
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
    executor::{ExecutionEvent, Executor},
    library::Library,
    plan::compile,
    recipe::LaunchRecipe,
    resolver::{ResolveContext, resolve},
    session::{GameSession, SessionProvider},
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
    Created { instance: String, profile: String },
    Recipe(Result<LaunchRecipe, String>),
    Status(String),
    Progress(usize),
    Log(String),
    Command(String),
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

    pub(super) fn play(&self, profile: String, account: Account, cancel: CancellationToken) {
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
                let code = library(root.clone(), &sender)?
                    .launch(&profile, &provider, &cancel)
                    .await?;
                ensure!(
                    cancel.is_cancelled() || code == Some(0),
                    "Minecraft exited unexpectedly ({code:?}). Open game output for details."
                );
                Ok(if cancel.is_cancelled() {
                    "Stopped".into()
                } else {
                    "Exited normally · ready to play again".into()
                })
            }
            .await;
            send_catalog(&root, &sender).await;
            finish(&sender, result).await;
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
        ExecutionEvent::Exited { .. } => return,
        ExecutionEvent::Log { line, .. } => {
            let Some(line) = readable_log(&line) else {
                return;
            };
            Message::Log(line)
        }
    };
    let _ = sender.try_send(message);
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
