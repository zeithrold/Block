//! The engine-message pump: one place decides which typed message may still
//! affect the current operation, and the per-group handlers apply it to the
//! launcher state.

use block_auth::Account;
use block_launcher::manifest::model::VersionEntry;
use block_launcher::{
    avatars::{ProviderIcon, SkinImage},
    catalog::Snapshot,
    game_import::DiscoveryReport,
    recipe::LaunchRecipe,
};
use gpui::{Context, Image, ImageFormat};
use uuid::Uuid;

use crate::{
    Block, OperationKind, PendingCrash, Tab, account_id, browser, components, engine::Message,
    feedback, import_games,
};

impl Block {
    /// Drain the engine channel and apply every message that still belongs to the
    /// current operation.
    pub(super) fn poll(&mut self, cx: &mut Context<'_, Self>) {
        let logging_failed = self.notify_logging_failure(block_logging::failure().is_some(), cx);
        self.window_changes
            .flush_size(self.view_id, cx.background_executor().now());
        let mut messages = Vec::new();
        if let Some(receiver) = &mut self.messages {
            while let Ok(message) = receiver.try_recv() {
                messages.push(message);
            }
        }
        let toast_expired = self
            .shell
            .toast
            .as_ref()
            .is_some_and(|toast| toast.expires <= std::time::Instant::now());
        if toast_expired {
            self.shell.toast = None;
        }
        let download_changed = self.downloads.as_ref().is_some_and(|tracker| {
            let tracker = tracker
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if tracker.revision == self.downloads_revision {
                return false;
            }
            self.downloads_revision = tracker.revision;
            self.completed = tracker.verified();
            true
        });
        if messages.is_empty() && !download_changed && !toast_expired && !logging_failed {
            return;
        }
        self.motion_dirty = true;
        for message in messages {
            let (message, scoped) = match message {
                Message::ForOperation { operation, message }
                    if operation == self.operation_revision =>
                {
                    (*message, true)
                }
                Message::ForOperation { .. } => continue,
                message => (message, false),
            };
            match message {
                Message::GameDiscovery { request, report } => {
                    self.receive_discovery(request, report, cx);
                }
                Message::GameImported {
                    request,
                    candidate,
                    result,
                } => self.receive_game_import(request, candidate, result),
                Message::GameImportFinished { request } => self.finish_game_import(request),
                Message::ForOperation { .. } => {}
                Message::Catalog(result) => self.receive_catalog(result, cx),
                Message::Providers(result) => {
                    self.receive_providers(result);
                    if let Some(engine) = &self.engine {
                        engine.load_images();
                    }
                }
                Message::ProviderSaved(id) => {
                    if !self.auth_provider_saved(&id, cx) {
                        self.provider_saved(&id, cx);
                    }
                    if let Some(engine) = &self.engine {
                        engine.load_images();
                    }
                }
                Message::Accounts(result) => self.receive_accounts(result),
                Message::AccountImages(images) => self.store_account_images(images),
                Message::ProviderImages(icons) => self.store_provider_images(icons),
                Message::Versions(result) => self.receive_versions(result),
                Message::LoaderVersions { request, result } => {
                    self.receive_loader_versions(request, result, cx);
                }
                Message::Browser(url) => {
                    if !self.cancel.is_cancelled() {
                        self.auth_browser(url, cx);
                    }
                }
                Message::AuthPrepared(account) => self.auth_prepared(account),
                Message::AuthlibCharacters(characters) => self.auth_characters(characters),
                Message::SignedIn(account) => self.auth_saved(account, cx),
                Message::DeviceCode {
                    user_code,
                    verification_uri,
                    expires_at,
                } => self.auth_device_code(user_code, verification_uri, expires_at),
                Message::AuthStage(stage) => self.auth_stage(stage),
                Message::ConnectionTest(request, result) => {
                    self.connection_finished(request, result, cx);
                }
                Message::Created { instance, profile } => {
                    self.creation_saved(instance, profile, cx);
                }
                Message::Status(status) => self.status = status,
                Message::Downloads(tracker) => {
                    self.downloads = Some(tracker);
                    self.downloads_revision = 0;
                }
                Message::Command(command) => self.command = Some(command),
                Message::Log(line) => self.push_log(line),
                Message::Session(record) => {
                    self.session_updated(*record, cx);
                    self.start_pending_play(cx);
                }
                Message::SessionAccepted { operation, record } => {
                    if operation == Some(self.operation_revision)
                        && self.logged_operation == Some(crate::ui_logging::Operation::Launch)
                    {
                        if let Some(id) = crate::ui_logging::session_id(&record.context.run_id) {
                            crate::ui_logging::session_handoff(
                                self.view_id,
                                self.operation_revision,
                                id,
                            );
                        }
                        self.finish_logged_operation(crate::ui_logging::Outcome::Accepted);
                    }
                    self.session_updated(*record, cx);
                    self.start_pending_play(cx);
                }
                Message::Crash {
                    report,
                    directory,
                    guard,
                } => {
                    self.pending_crashes.push_back(PendingCrash {
                        report: *report,
                        directory,
                        guard,
                    });
                }
                Message::Finished(result) => self.finish_operation(result, scoped, cx),
                Message::Recipe(result) => self.receive_recipe(result, cx),
            }
        }
        self.sync_form_inputs(cx);
        cx.notify();
    }

    /// A discovery that needs the import window is deferred to a later frame:
    /// the window is borrowed while its own message is being handled.
    fn receive_discovery(
        &mut self,
        request: Uuid,
        report: DiscoveryReport,
        cx: &mut Context<'_, Self>,
    ) {
        let decision = self.resolve_game_discovery(request, report);
        let deferred = matches!(
            decision,
            import_games::DeferredDiscovery::OpenDrawer
                | import_games::DeferredDiscovery::ContinueToOverview
        );
        if deferred && let Some(handle) = self.shell.import_games.window {
            let owner = cx.entity();
            cx.defer(move |cx| {
                let _ = handle.update(cx, |_, window, cx| {
                    owner.update(cx, |view, cx| {
                        view.apply_deferred_discovery(decision, window, cx);
                        cx.notify();
                    });
                });
            });
        }
    }

    fn receive_catalog(&mut self, result: Result<Snapshot, String>, cx: &mut Context<'_, Self>) {
        match result {
            Ok(snapshot) => {
                self.apply_catalog(snapshot, cx);
                self.shell.games.load = browser::CollectionLoad::Ready;
                self.shell.profiles.load = browser::CollectionLoad::Ready;
            }
            Err(error) => {
                self.shell.games.load = browser::CollectionLoad::Failed(error.clone().into());
                self.shell.profiles.load = browser::CollectionLoad::Failed(error.clone().into());
                self.report(crate::i18n::UiText::Raw(error));
            }
        }
    }

    /// Accounts arrive as one snapshot: the browser labels, the onboarding
    /// default and the profile's own default account all follow it.
    fn receive_accounts(&mut self, result: Result<Vec<Account>, String>) {
        let accounts = match result {
            Ok(accounts) => accounts,
            Err(error) => {
                self.shell.accounts.load = browser::CollectionLoad::Failed(error.clone().into());
                self.report(crate::i18n::UiText::Raw(error));
                return;
            }
        };
        self.shell.accounts.load = browser::CollectionLoad::Ready;
        self.reconcile_account_browser(&accounts);
        self.accounts = accounts;
        if self.shell.first_run.enabled
            && !self.preferences.onboarding_complete
            && self.selected_account.is_none()
        {
            self.selected_account = self.accounts.first().map(account_id);
            self.shell.onboarding.account = self.selected_account.clone();
        }
        if self.router.tab() != Tab::Onboarding
            && !self
                .accounts
                .iter()
                .any(|a| Some(account_id(a)) == self.selected_account)
        {
            self.apply_profile_account();
        }
        if let Some(engine) = &self.engine {
            engine.load_images();
        }
    }

    fn store_account_images(&mut self, images: Vec<(String, SkinImage)>) {
        self.account_avatars = images
            .into_iter()
            .map(|(identity, skin)| {
                (
                    identity,
                    components::AccountAvatar {
                        image: std::sync::Arc::new(Image::from_bytes(ImageFormat::Png, skin.bytes)),
                        width: skin.width,
                        height: skin.height,
                    },
                )
            })
            .collect();
    }

    fn store_provider_images(&mut self, icons: Vec<(String, ProviderIcon)>) {
        self.provider_icons = icons
            .into_iter()
            .map(|(provider, icon)| {
                (
                    provider,
                    components::ProviderBadge {
                        image: std::sync::Arc::new(Image::from_bytes(
                            components::icon_image_format(icon.kind),
                            icon.bytes,
                        )),
                    },
                )
            })
            .collect();
    }

    fn receive_versions(&mut self, result: Result<Vec<VersionEntry>, String>) {
        let versions = match result {
            Ok(versions) => versions,
            Err(error) => {
                self.version_load = browser::CollectionLoad::Failed(error.clone().into());
                self.push_log(format!("Version list unavailable: {error}"));
                return;
            }
        };
        self.version_load = browser::CollectionLoad::Ready;
        self.version_catalog.clone_from(&versions);
        self.versions = versions
            .into_iter()
            .filter(|version| {
                self.saved_preferences.game.show_snapshots
                    || format!("{:?}", version.kind) == "Release"
            })
            .map(|version| version.id)
            .collect();
    }

    /// Completion epilogue shared by every operation: apply the per-feature
    /// results, report the outcome and drain a queued Play.
    fn finish_operation(
        &mut self,
        result: Result<crate::i18n::UiText, String>,
        scoped: bool,
        cx: &mut Context<'_, Self>,
    ) {
        if scoped {
            let outcome = if result.is_ok() {
                crate::ui_logging::Outcome::Succeeded
            } else if self.cancel.is_cancelled() {
                crate::ui_logging::Outcome::Cancelled
            } else {
                crate::ui_logging::Outcome::Failed
            };
            self.finish_logged_operation(outcome);
        }
        self.auth_provider_finished(&result, cx);
        self.providers_finished(&result);
        self.instance_edit_finished(&result, cx);
        self.profile_edit_finished(&result, cx);
        self.auth_finished(&result);
        self.busy = self.active_run.is_some();
        self.operation_cancellable = false;
        match result {
            Ok(status) => {
                self.push_toast(status.clone(), feedback::ToastKind::Success);
                self.status = status;
                if self.router.tab() != Tab::Onboarding
                    && self.shell.auth.is_none()
                    && self.operation_kind != OperationKind::Account
                {
                    self.form = None;
                    self.router.leave_task();
                }
            }
            Err(_) if self.cancel.is_cancelled() => {
                self.status = msg!("runtime.cancelled");
            }
            Err(error) => {
                self.report(msg!("runtime.failed", error = error));
            }
        }
        self.start_pending_play(cx);
    }

    fn receive_recipe(&mut self, result: Result<LaunchRecipe, String>, cx: &mut Context<'_, Self>) {
        let launch = self.recipe_load.finish(result.is_ok());
        match result {
            Ok(recipe) => {
                self.recipe = Some(recipe);
                self.status = msg!("runtime.recipe_loaded");
                if launch {
                    self.launch_recipe(cx);
                }
            }
            Err(error) => self.report(msg!("runtime.invalid_recipe", error = error)),
        }
    }
}
