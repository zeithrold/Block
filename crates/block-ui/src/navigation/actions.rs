//! The launcher's action vocabulary, its dispatcher and its post-action
//! epilogue.
//!
//! `handle` is deliberately one exhaustive match: every interaction has exactly
//! one route, and the busy/composition guards are applied once, before it.
//! Branches that own real work — navigation, drawer dismissal, wizard page
//! validation, selection and session control — delegate to the named helpers
//! below instead of growing the match.

use block_launcher::catalog::DataPolicy;
use gpui::{Context, Window};

use crate::{
    Block, Form, OperationKind, ProfileForm, Tab, account_id,
    appearance::Appearance,
    authentication, browser, downloads, feedback, first_run,
    game_wizard::{CreationKind, WizardPage, WizardStep},
    import_games, modpack_import, motion, profile_editor, select,
    settings::SettingsDraft,
    shell::{Drawer, EditorExit, EditorKind, PendingEditorExit, Surface},
};

/// One concrete launcher interaction.
#[derive(Clone, Debug)]
pub(super) enum UiAction {
    FirstRun(first_run::Action),
    ImportGames(import_games::Action),
    PackImport(modpack_import::Action),
    AuthOtherOptions,
    Providers,
    ProviderAdd,
    ProviderInspect(String),
    ProviderEdit(String),
    ProviderCancel,
    ProviderIdentify,
    ProviderConsent,
    ProviderSave,
    ProviderRemoveRequest(String),
    ProviderRemove(String),
    Tab(Tab),
    NavigationBack,
    EditorExitSave,
    EditorExitDiscard,
    Drawer(Option<Drawer>),
    DownloadFiles(downloads::DownloadGroup),
    SetAppearance(Appearance),
    SetLanguage(crate::Language),
    ReduceMotion,
    SavePreferences,
    SettingsBack,
    SettingsExitSave,
    SettingsExitDiscard,
    ResetPreferences,
    ConfirmSeparate(bool),
    SeparateProfiles,
    WizardNext,
    WizardBack,
    WizardSnapshots,
    WizardOptiFineFile,
    WizardOptiFineRemove,
    WizardKind(CreationKind),
    ConfirmPackLeave,
    ConfirmDiscardGame,
    WizardInstance(String),
    WizardProfileIcon(block_launcher::catalog::ProfileIcon),
    WizardProfileColor(block_launcher::catalog::ProfileColor),
    WizardProfileFullscreen,
    WizardRuntime(bool),
    WizardWindow { width: u32, height: u32 },
    WizardCustomWindow,
    WizardProfileOptions,
    WizardProfileTarget(String),
    ChooseAppearance { wizard: bool, color: bool },
    OpenProductSelect(select::ProductChoice),
    DefaultJavaAutomatic(bool),
    DefaultWindow { width: u32, height: u32 },
    CustomDefaultWindow,
    ProfileRuntimeAutomatic(bool),
    ProfileDataPolicy(DataPolicy),
    InstanceRuntimeAutomatic(bool),
    InstanceWindow { width: u32, height: u32 },
    CustomInstanceWindow,
    ProfileConnect,
    WizardProfileConnect,
    ProfileIcon(block_launcher::catalog::ProfileIcon),
    ProfileColor(block_launcher::catalog::ProfileColor),
    SelectValue(Box<Self>),
    MinimizeWindow,
    CloseWindow,
    QuitApplication,
    WizardPage(Option<WizardPage>),
    SetFullscreen,
    ShowSnapshots,
    ProxyMode(u8),
    SettingsPage(u8),
    Inspect(String),
    InspectLinked { profiles: bool, id: String },
    GamesHelp,
    ProfileModeSettings,
    ReloadLibrary,
    ReloadVersions,
    EditInstance,
    EditInstanceOptiFine,
    SaveInstanceSettings,
    InstanceFullscreen,
    InstanceOptiFineFile,
    InstanceOptiFineRemove,
    ProfileInheritRuntime,
    ProfileInheritWindow,
    SelectProfile(String),
    SelectAccount(String),
    CreateInstance,
    SaveInstance,
    WizardLoader(block_launcher::catalog::LoaderKind),
    WizardLoaderBuild(String),
    ReloadLoaderVersions,
    BrowseInstall,
    SelectVersion(String),
    EditProfile,
    SaveProfile,
    ProfilePage(profile_editor::EditorPage),
    ProfilePageDone,
    ProfilePageCancel,
    CancelForm,
    NewProfile,
    CopyProfileId(String),
    DeleteProfileId(String),
    ProfileActions,
    ConfirmDelete,
    ProfileTarget(String),
    DefaultAccount(Option<String>),
    SignIn,
    Authenticate,
    AuthBack,
    AuthMethod(authentication::Method),
    AuthProvider(block_auth::AccountProvider),
    AuthExternalProvider(uuid::Uuid),
    AuthHttpConsent,
    AuthProviderAdd,
    AuthProviderIdentify,
    AuthProviderConsent,
    AuthProviderSave,
    AuthProviderCancel,
    AuthCharacter(uuid::Uuid),
    CopyDeviceCode,
    AuthCancel,
    AuthFinish,
    AuthReopen,
    AuthRetry,
    RequestSignOut(String),
    SignOut(String),
    Play,
    Repair,
    RepairProfile(String),
    Stop,
    ToggleOutput,
    ToggleCommand,
    OpenLogs,
    ExportLogs,
    CancelLogExport,
    OpenRecipe,
    LaunchRecipe,
}

/// Whether the action leaves the current page against its forward direction, so
/// the shared motion state replays in reverse.
fn motion_direction(action: &UiAction) -> motion::Direction {
    if matches!(
        action,
        UiAction::CancelForm
            | UiAction::NavigationBack
            | UiAction::SettingsBack
            | UiAction::WizardBack
            | UiAction::AuthCancel
            | UiAction::AuthBack
            | UiAction::AuthFinish
            | UiAction::ProfilePageCancel
            | UiAction::ProfilePageDone
            | UiAction::Tab(Tab::Home)
            | UiAction::FirstRun(first_run::Action::Back | first_run::Action::BackToWelcome)
    ) {
        motion::Direction::Back
    } else {
        motion::Direction::Forward
    }
}

impl Block {
    /// Route one interaction: apply the ordering guards once, dispatch
    /// exhaustively, then settle the frame.
    pub(super) fn handle(
        &mut self,
        action: UiAction,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.motion_dirty = true;
        self.shell.focus_generation = self.shell.focus_generation.wrapping_add(1);
        self.motion.direction = motion_direction(&action);
        if self.intercept_action(&action, window, cx) {
            return;
        }
        self.capture_motion_origin(cx);
        let focus_first = matches!(action, UiAction::CreateInstance | UiAction::EditProfile);
        let previous_page = (self.router.tab(), self.form.is_some());
        let previous_entry = self.router.entry_id();
        let previous_focus = window.focused(cx);
        match action {
            UiAction::NavigationBack => self.navigation_back(window, cx),
            UiAction::EditorExitSave => self.confirm_editor_exit(true, window, cx),
            UiAction::EditorExitDiscard => self.confirm_editor_exit(false, window, cx),
            UiAction::FirstRun(action) => self.route_first_run(action, window, cx),
            UiAction::ImportGames(action) => self.handle_import_games(action, window, cx),
            UiAction::PackImport(action) => self.handle_pack_import(action, window, cx),
            UiAction::ConfirmPackLeave => self.confirm_pack_leave(window, cx),
            UiAction::ConfirmDiscardGame => self.confirm_game_discard(window, cx),
            UiAction::AuthOtherOptions => self.toggle_auth_other_options(cx),
            UiAction::WizardOptiFineFile => self.browse_optifine(cx),
            UiAction::WizardOptiFineRemove => self.clear_wizard_optifine(),
            UiAction::Tab(tab) => {
                if !self.navigate(tab, window, cx) {
                    return;
                }
            }
            UiAction::DownloadFiles(group) => {
                self.shell
                    .surfaces
                    .open(Surface::DownloadFiles(group), window, cx);
            }
            UiAction::Drawer(drawer) => {
                if !self.set_drawer(drawer, window, cx) {
                    return;
                }
            }
            UiAction::SetAppearance(appearance) => self.preview_appearance(appearance),
            UiAction::SetLanguage(language) => {
                self.preferences.language = Some(language);
                self.refresh_language(window, cx);
            }
            UiAction::ReduceMotion => self.preview_reduce_motion(),
            UiAction::ResetPreferences => self.reset_settings(window, cx),
            UiAction::SavePreferences => {
                self.save_settings(window, cx);
            }
            UiAction::SettingsBack => self.settings_back(window, cx),
            UiAction::SettingsExitSave => self.confirm_settings_exit(true, window, cx),
            UiAction::SettingsExitDiscard => self.confirm_settings_exit(false, window, cx),
            UiAction::SetFullscreen => self.toggle_settings_fullscreen(),
            UiAction::ShowSnapshots => self.toggle_settings_snapshots(),
            UiAction::SettingsPage(page) => self.select_settings_page(page, window, cx),
            UiAction::ProxyMode(mode) => self.edit_settings_draft(|form| form.proxy_mode = mode),
            UiAction::ConfirmSeparate(confirm) => self.confirm_separate(confirm, window, cx),
            UiAction::SeparateProfiles => self.separate_profiles(window, cx),
            UiAction::Inspect(id) => self.inspect(id, None, cx),
            UiAction::InspectLinked { profiles, id } => self.inspect(id, Some(profiles), cx),
            UiAction::GamesHelp => {
                self.shell.surfaces.open(Surface::GamesHelp, window, cx);
            }
            UiAction::ProfileModeSettings => self.open_profile_mode_settings(window, cx),
            UiAction::ReloadVersions => self.reload_versions(),
            UiAction::ReloadLibrary => self.reload_library(),
            UiAction::EditInstance => self.open_instance_editor(window, cx),
            UiAction::EditInstanceOptiFine => self.open_instance_optifine_editor(window, cx),
            UiAction::SaveInstanceSettings => self.save_instance_draft(window, cx),
            UiAction::InstanceFullscreen => self.toggle_instance_fullscreen(),
            UiAction::InstanceOptiFineFile => self.browse_instance_optifine(cx),
            UiAction::InstanceOptiFineRemove => self.remove_instance_optifine(cx),
            UiAction::ProfileInheritRuntime => self.edit_profile_draft(|form| {
                form.original.inheritance.runtime = !form.original.inheritance.runtime;
            }),
            UiAction::ProfileInheritWindow => self.edit_profile_draft(|form| {
                form.original.inheritance.window = !form.original.inheritance.window;
            }),
            UiAction::SelectProfile(id) => self.select_profile(id, window, cx),
            UiAction::SelectAccount(id) => self.select_account(id, window, cx),
            UiAction::CreateInstance => self.start_creation(window, cx),
            UiAction::WizardKind(kind) => self.start_creation_kind(kind, window, cx),
            UiAction::WizardInstance(id) => self.wizard_instance(id),
            UiAction::WizardProfileIcon(icon) => self.wizard_profile_icon(icon),
            UiAction::WizardProfileColor(color) => self.wizard_profile_color(color),
            UiAction::WizardProfileFullscreen => self.wizard_profile_fullscreen(),
            UiAction::WizardProfileOptions => self.wizard_profile_options(),
            UiAction::WizardProfileTarget(id) => self.wizard_profile_target(id),
            UiAction::ChooseAppearance { wizard, color } => {
                self.open_appearance_select(wizard, color, window, cx);
            }
            UiAction::OpenProductSelect(kind) => self.open_product_select(kind, None, window, cx),
            UiAction::DefaultJavaAutomatic(value) => {
                self.set_default_java_automatic(value, window, cx);
            }
            UiAction::DefaultWindow { width, height } => self.set_default_window(width, height, cx),
            UiAction::CustomDefaultWindow => self.custom_default_window(window, cx),
            UiAction::ProfileRuntimeAutomatic(value) => {
                self.set_profile_runtime_automatic(value, window, cx);
            }
            UiAction::ProfileDataPolicy(value) => self.set_profile_data_policy(value),
            UiAction::InstanceRuntimeAutomatic(value) => {
                self.set_instance_runtime_automatic(value, window, cx);
            }
            UiAction::InstanceWindow { width, height } => {
                self.set_instance_window(width, height, cx);
            }
            UiAction::CustomInstanceWindow => self.custom_instance_window(window, cx),
            UiAction::ProfileConnect => self.toggle_profile_connect(),
            UiAction::WizardRuntime(value) => self.wizard_runtime(value),
            UiAction::WizardWindow { width, height } => self.wizard_window(width, height, cx),
            UiAction::WizardCustomWindow => self.wizard_custom_window(),
            UiAction::WizardProfileConnect => self.wizard_profile_connect(),
            UiAction::MinimizeWindow => window.minimize_window(),
            UiAction::CloseWindow => self.close_launcher_window(window),
            UiAction::QuitApplication => Self::defer_application_quit(cx),
            UiAction::SelectValue(action) => self.select_value(*action, window, cx),
            UiAction::ProfileIcon(icon) => self.edit_profile_draft(|form| {
                form.original.appearance.icon = icon;
            }),
            UiAction::ProfileColor(color) => self.edit_profile_draft(|form| {
                form.original.appearance.color = color;
            }),
            UiAction::WizardSnapshots => self.toggle_wizard_snapshots(),
            UiAction::WizardNext => self.advance_wizard_focus(window, cx),
            UiAction::WizardBack => self.back_wizard_focus(window, cx),
            UiAction::WizardPage(page) => {
                if !self.select_wizard_page(page, window, cx) {
                    return;
                }
            }
            UiAction::WizardLoader(kind) => self.choose_wizard_loader(kind, cx),
            UiAction::WizardLoaderBuild(version) => self.choose_wizard_loader_build(version, cx),
            UiAction::ReloadLoaderVersions => self.load_wizard_loader_versions(true, cx),
            UiAction::SelectVersion(version) => self.set_wizard_version(version, cx),
            UiAction::BrowseInstall => self.browse_install(cx),
            UiAction::SaveInstance => self.submit_game_and_focus(window, cx),
            UiAction::EditProfile => self.edit_profile(window, cx),
            UiAction::SaveProfile => self.save_profile_draft(window, cx),
            UiAction::ProfilePage(page) => self.open_profile_page(page, window, cx),
            UiAction::ProfilePageDone => self.finish_profile_page(false, window, cx),
            UiAction::ProfilePageCancel => self.finish_profile_page(true, window, cx),
            UiAction::CancelForm => self.cancel_or_finish_form(window, cx),
            UiAction::NewProfile => self.new_profile(cx),
            UiAction::ProfileActions => self.open_profile_actions_at(None, window, cx),
            UiAction::CopyProfileId(id) => self.copy_profile(id),
            UiAction::DeleteProfileId(id) => self.request_profile_delete(id, window, cx),
            UiAction::ConfirmDelete => self.confirm_profile_delete(window, cx),
            UiAction::ProfileTarget(target) => self.edit_profile_draft(|form| {
                form.original.target_id = target;
            }),
            UiAction::DefaultAccount(account) => self.edit_profile_draft(|form| {
                form.original.default_account = account;
            }),
            UiAction::SignIn => self.open_auth_wizard(window, cx),
            UiAction::Authenticate | UiAction::AuthRetry => self.start_auth(window, cx),
            UiAction::AuthCancel => self.cancel_auth(window, cx),
            UiAction::AuthFinish => self.finish_auth(window, cx),
            UiAction::Providers => self.open_providers(cx),
            UiAction::ProviderAdd => self.edit_provider(None, window, cx),
            UiAction::ProviderInspect(id) => self.inspect_provider(&id, cx),
            UiAction::ProviderEdit(id) => self.edit_provider(Some(&id), window, cx),
            UiAction::ProviderCancel => self.cancel_provider_editor(cx),
            UiAction::ProviderIdentify => self.identify_provider(window, cx),
            UiAction::ProviderConsent => self.consent_provider(cx),
            UiAction::ProviderSave => self.save_provider(cx),
            UiAction::ProviderRemoveRequest(id) => self.request_remove_provider(id, window, cx),
            UiAction::ProviderRemove(id) => self.remove_provider(id, window, cx),
            UiAction::AuthProvider(provider) => self.choose_auth_provider(provider, cx),
            UiAction::AuthBack => self.auth_back(window, cx),
            UiAction::AuthExternalProvider(id) => self.choose_external_provider(id, window, cx),
            UiAction::AuthHttpConsent => self.consent_external_auth(cx),
            UiAction::AuthProviderAdd => self.open_auth_provider_add(window, cx),
            UiAction::AuthProviderIdentify => self.identify_auth_provider(cx),
            UiAction::AuthProviderConsent => self.consent_auth_provider(cx),
            UiAction::AuthProviderSave => self.save_auth_provider(window, cx),
            UiAction::AuthProviderCancel => self.close_auth_provider_add(window, cx),
            UiAction::AuthCharacter(id) => self.choose_auth_character(id, cx),
            UiAction::AuthMethod(method) => self.change_auth_method(method, window, cx),
            UiAction::CopyDeviceCode => self.copy_device_code(cx),
            UiAction::AuthReopen => self.reopen_auth(cx),
            UiAction::RequestSignOut(id) => {
                self.shell
                    .surfaces
                    .open(Surface::RemoveAccount(id), window, cx);
            }
            UiAction::SignOut(id) => self.sign_out(id, window, cx),
            UiAction::Play => self.play(cx),
            UiAction::Repair => self.repair_selected(window, cx),
            UiAction::RepairProfile(id) => self.repair(&id, window, cx),
            UiAction::Stop => self.stop(),
            UiAction::ToggleOutput => self.show_output = !self.show_output,
            UiAction::ToggleCommand => self.show_command = !self.show_command,
            UiAction::OpenLogs => self.open_launcher_logs(cx),
            UiAction::ExportLogs => self.export_launcher_logs(cx),
            UiAction::CancelLogExport => self.cancel_log_export(),
            UiAction::OpenRecipe => self.open_recipe(cx),
            UiAction::LaunchRecipe => self.launch_recipe(cx),
        }
        self.restore_navigation_focus(previous_entry, previous_focus);
        self.finish_action(previous_page, focus_first, window, cx);
    }

    fn restore_navigation_focus(
        &mut self,
        previous_entry: u64,
        previous_focus: Option<gpui::FocusHandle>,
    ) {
        let entry = self.router.entry_id();
        if previous_entry != entry {
            if self.shell.focus_entry != Some(entry) {
                self.shell.focus_entry = None;
            }
            if let Some(focus) = previous_focus {
                self.shell.return_focus.insert(previous_entry, focus);
            }
            self.shell
                .return_focus
                .retain(|entry, _| self.router.contains_entry(*entry));
        }
    }

    pub(super) fn schedule_navigation_focus(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let entry = self.router.entry_id();
        if self.shell.focus_entry == Some(entry) {
            return;
        }
        self.shell.focus_entry = Some(entry);
        self.shell
            .return_focus
            .retain(|entry, _| self.router.contains_entry(*entry));
        let fallback = match self.router.screen() {
            super::navigation::Screen::Record(_) => self.browser_detail_focus(cx),
            super::navigation::Screen::Provider(_) => self.provider_detail_focus(cx),
            _ => self.focus.clone(),
        };
        let focus = self.shell.return_focus.get(&entry).cloned();
        // Fresh editors already focus their initial field through their owner.
        if focus.is_none()
            && (matches!(
                self.router.screen(),
                super::navigation::Screen::InstanceEditor(_)
                    | super::navigation::Screen::ProfileEditor(_)
                    | super::navigation::Screen::NewProvider
                    | super::navigation::Screen::EditProvider(_)
            ) || self.page_spec().task)
        {
            return;
        }
        // Keep Back in the live shell until destination controls are mounted.
        // Explicit component owners mark focus_entry before this scheduler.
        if !self.shell.surfaces.is_open() {
            self.focus.focus(window, cx);
        }
        let scene = self.page_spec().detail;
        let generation = self.shell.focus_generation;
        let overlays: Vec<_> = self
            .shell
            .surfaces
            .layers()
            .map(|layer| layer.revision)
            .collect();
        let owner = cx.weak_entity();
        window.on_next_frame(move |window, _| {
            window.on_next_frame(move |window, cx| {
                let _ = owner.update(cx, |view, cx| {
                    if view.router.entry_id() != entry
                        || view.page_spec().detail != scene
                        || view.shell.focus_generation != generation
                        || view.shell.surfaces.is_open()
                        || !view
                            .shell
                            .surfaces
                            .layers()
                            .map(|layer| layer.revision)
                            .eq(overlays)
                    {
                        return;
                    }
                    focus
                        .filter(|focus| view.focus.contains(focus, window))
                        .or(Some(fallback))
                        .filter(|focus| view.focus.contains(focus, window))
                        .unwrap_or_else(|| view.focus.clone())
                        .focus(window, cx);
                });
            });
        });
    }

    fn route_first_run(
        &mut self,
        action: first_run::Action,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if matches!(action, first_run::Action::Back) && self.finish_completed_pack(window, cx) {
            return;
        }
        if matches!(action, first_run::Action::Back) && self.router.game_wizard() {
            if self.return_from_pack_shortcut(window, cx) {
                return;
            }
            self.pack_import_closed();
        }
        self.handle_first_run(action, window, cx);
    }

    fn close_launcher_window(&mut self, window: &mut Window) {
        self.select_completed_pack();
        window.remove_window();
    }

    fn defer_application_quit(cx: &mut Context<'_, Self>) {
        cx.defer(|cx| {
            crate::request_application_quit(cx);
        });
    }

    /// Native close and application quit must use the same save and draft
    /// barriers as actions dispatched by the page.
    fn intercept_action(
        &mut self,
        action: &UiAction,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        self.exit_blocked_by_auth_save(action)
            || self.composition_in_progress(action, cx)
            || !self.action_allowed(action)
            || self.confirm_pack_leave_if_needed(action, window, cx)
            || self.confirm_game_discard_if_needed(action, window, cx)
            || self.confirm_settings_exit_if_needed(action, window, cx)
            || self.confirm_editor_exit_if_needed(action, window, cx)
    }

    pub(super) fn allow_native_window_close(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        if self.intercept_action(&UiAction::CloseWindow, window, cx) {
            return false;
        }
        self.select_completed_pack();
        true
    }

    pub(super) fn allow_application_quit(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        !self.intercept_action(&UiAction::QuitApplication, window, cx)
    }

    fn cancel_log_export(&self) {
        if let Some(cancel) = &self.log_export_cancel {
            cancel.cancel();
        }
    }

    fn cancel_or_finish_form(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.router.game_wizard()
            && let Some(page) = self.shell.wizard.page
        {
            self.cancel_component_draft(cx);
            self.focus_wizard_card(page, window, cx);
            return;
        }
        if !self.finish_completed_pack(window, cx) {
            self.cancel_form(window, cx);
        }
    }

    fn back_wizard_focus(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let page = self.shell.wizard.page;
        self.back_game_wizard(cx);
        if self.shell.wizard.page.is_none()
            && let Some(page) = page
        {
            self.focus_wizard_card(page, window, cx);
        } else {
            self.focus_field(false, window, cx);
        }
    }

    /// Settle the frame after an action that changed launcher state: resize for a
    /// new page shape, replay the page transition when the page changed, restore
    /// focus for a newly opened form and keep busy form inputs in step.
    fn finish_action(
        &mut self,
        previous_page: (Tab, bool),
        focus_first: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if previous_page != (self.router.tab(), self.form.is_some()) {
            self.shell.backwards =
                self.router.tab() == Tab::Home || self.form.is_none() && previous_page.1;
            self.shell.revision = self.shell.revision.wrapping_add(1);
        }
        if focus_first && !self.shell.surfaces.is_open() {
            self.focus_field(false, window, cx);
        }
        self.sync_form_inputs(cx);
        cx.notify();
    }

    /// An authorized account save cannot be presented as a cancelled write.
    fn exit_blocked_by_auth_save(&self, action: &UiAction) -> bool {
        self.auth_save_pending()
            && matches!(
                action,
                UiAction::CloseWindow
                    | UiAction::QuitApplication
                    | UiAction::NavigationBack
                    | UiAction::Tab(_)
                    | UiAction::FirstRun(_)
            )
    }

    /// A commit or page change must not interrupt an IME composition: the draft
    /// still holds the text being assembled.
    pub(super) fn composition_in_progress(
        &self,
        action: &UiAction,
        cx: &Context<'_, Self>,
    ) -> bool {
        if matches!(
            action,
            UiAction::SavePreferences
                | UiAction::SettingsExitSave
                | UiAction::SettingsBack
                | UiAction::NavigationBack
                | UiAction::SettingsPage(_)
                | UiAction::Tab(_)
        ) && self.settings_form.as_ref().is_some_and(|draft| {
            draft
                .fields()
                .iter()
                .any(|field| field.read(cx).is_composing())
        }) {
            return true;
        }
        (matches!(
            action,
            UiAction::SaveInstance
                | UiAction::SaveProfile
                | UiAction::WizardNext
                | UiAction::NavigationBack
                | UiAction::CancelForm
                | UiAction::Tab(_)
        ) && self
            .form_inputs()
            .iter()
            .any(|field| field.read(cx).is_composing()))
            || matches!(
                action,
                UiAction::PackImport(
                    modpack_import::Action::Confirm | modpack_import::Action::Retry
                )
            ) && self
                .shell
                .pack_import
                .name
                .as_ref()
                .is_some_and(|field| field.read(cx).is_composing())
    }

    /// Whether the dispatcher may run this action now. Everything waits for a
    /// busy operation except the actions the busy policy allows, and the one
    /// action that retries the save its own operation is holding.
    fn action_allowed(&self, action: &UiAction) -> bool {
        if !self.busy {
            return true;
        }
        self.can_activate(action)
    }

    /// An active pack installation must be explicitly stopped before leaving.
    fn confirm_pack_leave_if_needed(
        &mut self,
        action: &UiAction,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        let leaving = matches!(
            action,
            UiAction::CancelForm
                | UiAction::NavigationBack
                | UiAction::Tab(_)
                | UiAction::CloseWindow
                | UiAction::QuitApplication
                | UiAction::FirstRun(first_run::Action::Back)
        );
        if !leaving
            || !self.pack_wizard_active()
            || self.shell.pack_import.phase != modpack_import::Phase::Installing
        {
            return false;
        }
        if let UiAction::Tab(tab) = action
            && !self.router.allows(*tab, self.setup_incomplete())
        {
            return false;
        }
        let Some(request) = self.shell.pack_import.request else {
            return false;
        };
        if !self
            .shell
            .surfaces
            .layers()
            .any(|entry| matches!(entry.kind, Surface::StopPackImport { .. }))
        {
            self.shell.surfaces.open(
                Surface::StopPackImport {
                    request,
                    exit: Box::new(action.clone()),
                    window: window.window_handle(),
                },
                window,
                cx,
            );
        }
        cx.notify();
        true
    }

    fn confirm_pack_leave(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some((request, exit)) = self
            .shell
            .surfaces
            .top()
            .and_then(|entry| match &entry.kind {
                Surface::StopPackImport { request, exit, .. } => Some((*request, exit.clone())),
                _ => None,
            })
        else {
            return;
        };
        self.shell.surfaces.close(window, &self.focus, cx);
        if self.pack_wizard_active()
            && self.shell.pack_import.phase == modpack_import::Phase::Installing
            && self.shell.pack_import.accepts(request)
        {
            self.pack_import_closed();
            self.handle(*exit, window, cx);
        }
    }

    /// Returning to first-run keeps the live draft. Only exits that actually
    /// destroy it require confirmation, and an untouched draft exits directly.
    fn confirm_game_discard_if_needed(
        &mut self,
        action: &UiAction,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        if let Some(draft) = self.shell.wizard.discard_confirmed.take()
            && self.changed_game_draft_id(action, cx) == Some(draft)
        {
            return false;
        }
        if self.shell.auth.is_some()
            && !matches!(
                action,
                UiAction::CloseWindow | UiAction::QuitApplication | UiAction::Tab(_)
            )
        {
            return false;
        }
        let leaving = match action {
            UiAction::CancelForm | UiAction::NavigationBack => {
                self.pack_wizard_active()
                    || self.router.game_wizard()
                        && self.shell.wizard.page.is_none()
                        && self.router.tab() != Tab::Onboarding
            }
            UiAction::CloseWindow
            | UiAction::QuitApplication
            | UiAction::FirstRun(first_run::Action::Skip) => true,
            UiAction::Tab(tab) => {
                (self.router.game_wizard() || self.shell.auth.is_some())
                    && self.router.allows(*tab, self.setup_incomplete())
            }
            UiAction::FirstRun(first_run::Action::Continue) => self.first_run_continue_exits(),
            UiAction::FirstRun(first_run::Action::Back) => {
                self.pack_wizard_active()
                    || self.router.game_wizard() && self.changed_pack_draft_id(cx).is_some()
            }
            _ => false,
        };
        let Some(draft) = leaving
            .then(|| self.changed_game_draft_id(action, cx))
            .flatten()
        else {
            return false;
        };
        self.shell.surfaces.open(
            Surface::DiscardGameDraft {
                draft,
                exit: Box::new(action.clone()),
            },
            window,
            cx,
        );
        cx.notify();
        true
    }

    fn confirm_game_discard(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some((draft, exit)) = self
            .shell
            .surfaces
            .top()
            .and_then(|entry| match &entry.kind {
                Surface::DiscardGameDraft { draft, exit } => Some((*draft, exit.clone())),
                _ => None,
            })
        else {
            return;
        };
        self.shell.surfaces.close(window, &self.focus, cx);
        if self.changed_game_draft_id(&exit, cx) != Some(draft) {
            return;
        }
        self.shell.wizard.discard_confirmed = Some(draft);
        if matches!(*exit, UiAction::QuitApplication) {
            Self::defer_application_quit(cx);
        } else {
            self.handle(*exit, window, cx);
        }
    }

    fn changed_game_draft_id(&self, action: &UiAction, cx: &gpui::App) -> Option<gpui::EntityId> {
        let pack_draft = self.router.game_wizard().then(|| {
            self.changed_pack_draft_id(cx).or_else(|| {
                matches!(
                    action,
                    UiAction::CloseWindow
                        | UiAction::QuitApplication
                        | UiAction::Tab(_)
                        | UiAction::FirstRun(first_run::Action::Skip | first_run::Action::Continue)
                )
                .then(|| self.held_pack_parent_draft_id(cx))
                .flatten()
            })
        });
        pack_draft.flatten().or_else(|| {
            let returns_to_source = self.pack_shortcut_preserves_current_draft()
                && matches!(
                    action,
                    UiAction::CancelForm | UiAction::FirstRun(first_run::Action::Back)
                );
            if returns_to_source || !self.game_wizard_changed(cx) {
                return None;
            }
            match &self.form {
                Some(Form::Instance(form)) => Some(form.name.entity_id()),
                _ => None,
            }
        })
    }

    /// Whether an action may run while another operation is in progress.
    pub(super) fn can_activate(&self, action: &UiAction) -> bool {
        Self::available_while_busy(action)
            || matches!(action, UiAction::Play) && self.play_queue_eligible()
    }

    /// Play may be queued while an install/repair is keeping the app busy.
    pub(super) fn play_queue_eligible(&self) -> bool {
        self.busy
            && self.operation_kind == OperationKind::Install
            && self.active_run.is_none()
            && self.profile().is_some()
    }

    pub(super) fn available_while_busy(action: &UiAction) -> bool {
        if let UiAction::SelectValue(action) = action {
            return Self::available_while_busy(action);
        }
        if let UiAction::OpenProductSelect(choice) = action {
            return matches!(
                choice,
                select::ProductChoice::Language
                    | select::ProductChoice::Appearance
                    | select::ProductChoice::Proxy
                    | select::ProductChoice::DefaultJava
                    | select::ProductChoice::DefaultWindow
            );
        }
        matches!(
            action,
            UiAction::Stop
                | UiAction::PackImport(modpack_import::Action::Stop)
                | UiAction::QuitApplication
                | UiAction::Tab(_)
                | UiAction::NavigationBack
                | UiAction::Providers
                | UiAction::DownloadFiles(_)
                | UiAction::Drawer(_)
                | UiAction::AuthCancel
                | UiAction::AuthProviderCancel
                | UiAction::AuthReopen
                | UiAction::MinimizeWindow
                | UiAction::CloseWindow
                | UiAction::SignIn
                | UiAction::AuthBack
                | UiAction::AuthMethod(_)
                | UiAction::CopyDeviceCode
                | UiAction::SetAppearance(_)
                | UiAction::SetLanguage(_)
                | UiAction::ReduceMotion
                | UiAction::DefaultJavaAutomatic(_)
                | UiAction::DefaultWindow { .. }
                | UiAction::CustomDefaultWindow
                | UiAction::SavePreferences
                | UiAction::SettingsBack
                | UiAction::SettingsExitSave
                | UiAction::SettingsExitDiscard
                | UiAction::ResetPreferences
                | UiAction::SettingsPage(_)
                | UiAction::SetFullscreen
                | UiAction::ShowSnapshots
                | UiAction::ProxyMode(_)
                | UiAction::Inspect(_)
                | UiAction::InspectLinked { .. }
                | UiAction::GamesHelp
                | UiAction::ProfileModeSettings
                | UiAction::ReloadVersions
                | UiAction::ReloadLibrary
                | UiAction::ToggleOutput
                | UiAction::ToggleCommand
                | UiAction::OpenLogs
                | UiAction::ExportLogs
                | UiAction::CancelLogExport
        )
    }

    /// Switch pages. Returns whether the shared epilogue still applies: a route
    /// that setup refuses, or a Settings/About preview opened over the setup
    /// page, settles the frame itself.
    fn navigate(&mut self, tab: Tab, window: &mut Window, cx: &mut Context<'_, Self>) -> bool {
        if !self.router.allows(tab, self.setup_incomplete()) {
            self.push_toast(msg!("runtime.setup_required"), feedback::ToastKind::Info);
            cx.notify();
            return false;
        }
        self.select_completed_pack();
        if self.shell.providers.visible {
            self.close_providers(cx);
        }
        if self.shell.auth.is_some() {
            self.cancel_auth(window, cx);
        }
        if self.router.tab() == Tab::Settings && tab != Tab::Settings {
            self.leave_settings(cx);
        }
        if tab == Tab::Settings && self.router.tab() != Tab::Settings {
            self.open_settings(cx);
        }
        if self.router.game_wizard() {
            self.pack_import_closed();
        }
        self.initialize_browsers(cx);
        self.router.navigate(tab);
        self.form = None;
        self.shell.instance_editor = None;
        self.shell.surfaces.clear(window, &self.focus, cx);
        if tab == Tab::Settings {
            self.focus_settings_heading(window, cx);
        }
        true
    }

    fn navigation_back(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.shell.auth.is_some() {
            self.auth_back(window, cx);
        } else if self.router.tab() == Tab::Settings {
            self.settings_back(window, cx);
        } else if self.shell.instance_editor.is_some() {
            self.cancel_instance_editor(window, cx);
        } else if matches!(self.form, Some(Form::Profile(_))) {
            if self.shell.editor.page.is_some() {
                self.finish_profile_page(false, window, cx);
            } else {
                self.cancel_or_finish_form(window, cx);
            }
        } else if self.router.game_wizard() {
            if self.shell.wizard.page.is_some() {
                self.back_wizard_focus(window, cx);
            } else if self.router.tab() == Tab::Onboarding {
                self.route_first_run(first_run::Action::Back, window, cx);
            } else {
                self.cancel_or_finish_form(window, cx);
            }
        } else if matches!(
            self.router.screen(),
            super::navigation::Screen::NewProvider | super::navigation::Screen::EditProvider(_)
        ) {
            self.cancel_provider_editor(cx);
        } else {
            self.router.back();
            self.shell.providers.visible = matches!(
                self.router.screen(),
                super::navigation::Screen::Providers
                    | super::navigation::Screen::Provider(_)
                    | super::navigation::Screen::NewProvider
                    | super::navigation::Screen::EditProvider(_)
            );
        }
    }

    fn confirm_editor_exit_if_needed(
        &mut self,
        action: &UiAction,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        let leaving = matches!(
            action,
            UiAction::NavigationBack
                | UiAction::Tab(_)
                | UiAction::CloseWindow
                | UiAction::QuitApplication
        );
        if !leaving {
            return false;
        }
        if self.shell.pending_editor_exit.is_some() {
            return true;
        }
        let kind = match self.router.screen() {
            super::navigation::Screen::InstanceEditor(_) => self
                .shell
                .instance_editor
                .as_ref()
                .is_some_and(|editor| editor.dirty(cx))
                .then_some(EditorKind::Instance),
            super::navigation::Screen::ProfileEditor(_)
                if !matches!(action, UiAction::NavigationBack)
                    || self.shell.editor.page.is_none() =>
            {
                self.profile_editor_dirty(cx).then_some(EditorKind::Profile)
            }
            super::navigation::Screen::NewProvider | super::navigation::Screen::EditProvider(_) => {
                self.shell
                    .providers
                    .dirty(cx)
                    .then_some(EditorKind::Provider)
            }
            _ => None,
        };
        let Some(kind) = kind else {
            return false;
        };
        if !self
            .shell
            .surfaces
            .layers()
            .any(|layer| matches!(layer.kind, Surface::EditorExit(_)))
        {
            if self.shell.surfaces.is_open() {
                self.shell.surfaces.clear(window, &self.focus, cx);
            }
            self.shell.surfaces.open(
                Surface::EditorExit(EditorExit {
                    entry: self.router.entry_id(),
                    kind,
                    exit: Box::new(action.clone()),
                }),
                window,
                cx,
            );
        }
        cx.notify();
        true
    }

    fn confirm_editor_exit(&mut self, save: bool, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(exit) = self.shell.surfaces.top().and_then(|layer| {
            if let Surface::EditorExit(exit) = &layer.kind {
                Some(exit.clone())
            } else {
                None
            }
        }) else {
            return;
        };
        if exit.entry != self.router.entry_id() {
            self.shell.surfaces.close(window, &self.focus, cx);
            return;
        }
        self.shell.surfaces.close(window, &self.focus, cx);
        if save {
            let operation = self.operation_revision;
            match exit.kind {
                EditorKind::Instance => self.save_instance_draft(window, cx),
                EditorKind::Profile => self.save_profile_draft(window, cx),
                EditorKind::Provider => self.save_provider(cx),
            }
            if self.busy && self.operation_revision != operation {
                self.shell.pending_editor_exit = Some(PendingEditorExit {
                    operation: self.operation_revision,
                    exit: exit.exit,
                    window: window.window_handle(),
                });
            }
        } else {
            match exit.kind {
                EditorKind::Instance => self.cancel_instance_editor(window, cx),
                EditorKind::Profile => self.cancel_profile_editor(window, cx),
                EditorKind::Provider => self.cancel_provider_editor(cx),
            }
            if !matches!(exit.exit.as_ref(), UiAction::NavigationBack) {
                self.handle(*exit.exit, window, cx);
            }
        }
    }

    fn confirm_settings_exit_if_needed(
        &mut self,
        action: &UiAction,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        let leaving = match action {
            UiAction::Tab(tab) => {
                *tab != Tab::Settings && self.router.allows(*tab, self.setup_incomplete())
            }
            UiAction::CloseWindow | UiAction::QuitApplication => true,
            _ => false,
        };
        if !leaving
            || self.router.tab() != Tab::Settings
            || !self
                .settings_form
                .as_ref()
                .is_some_and(|draft| self.settings_dirty(draft, cx))
        {
            return false;
        }
        if !self
            .shell
            .surfaces
            .layers()
            .any(|entry| matches!(entry.kind, Surface::SettingsExit(_)))
        {
            self.shell.surfaces.clear(window, &self.focus, cx);
            self.shell
                .surfaces
                .open(Surface::SettingsExit(Box::new(action.clone())), window, cx);
        }
        cx.notify();
        true
    }

    fn confirm_settings_exit(
        &mut self,
        save: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(exit) = self
            .shell
            .surfaces
            .top()
            .and_then(|entry| match &entry.kind {
                Surface::SettingsExit(exit) => Some(exit.clone()),
                _ => None,
            })
        else {
            return;
        };
        self.shell.surfaces.close(window, &self.focus, cx);
        if save && !self.save_settings(window, cx) {
            return;
        }
        self.leave_settings(cx);
        if matches!(*exit, UiAction::QuitApplication) {
            Self::defer_application_quit(cx);
        } else {
            self.handle(*exit, window, cx);
        }
    }

    /// Open, replace or dismiss a drawer. Returns whether the shared epilogue
    /// still applies: a drawer that setup refuses leaves the frame untouched.
    fn set_drawer(
        &mut self,
        drawer: Option<Drawer>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        if self.setup_incomplete()
            && self.shell.first_run.enabled
            && drawer
                .is_some_and(|drawer| !matches!(drawer, Drawer::ImportGames | Drawer::Language))
        {
            return false;
        }
        if drawer == Some(Drawer::Menu) {
            self.open_app_menu_at(None, window, cx);
        } else if let Some(drawer) = drawer {
            if self
                .shell
                .surfaces
                .open(Surface::Drawer(drawer), window, cx)
            {
                self.open_selector(drawer, window, cx);
            }
        } else if self
            .shell
            .surfaces
            .top()
            .is_some_and(|entry| matches!(entry.kind, Surface::AuthProviderAdd))
        {
            self.cancel_auth_provider(cx);
            self.shell.surfaces.close(window, &self.focus, cx);
        } else {
            self.shell.surfaces.close(window, &self.focus, cx);
        }
        true
    }

    /// Route a wizard page request. Returns whether the shared epilogue still
    /// applies: a rejected section, or a section surface that was replaced,
    /// has already settled focus and repaint.
    fn select_wizard_page(
        &mut self,
        page: Option<WizardPage>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        if page.is_none()
            && self
                .shell
                .wizard
                .page
                .is_some_and(|page| !self.validate_wizard_section(page, cx))
        {
            self.focus_wizard_error(window, cx);
            return false;
        }
        self.open_game_wizard_page(page, cx);
        if page.is_some() {
            self.focus_field(false, window, cx);
        }
        true
    }

    /// Back out of the open form: leave the setup guide, step back through the
    /// wizard, close the profile editor or drop an unattached game draft.
    fn cancel_form(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.router.game_wizard() && self.shell.wizard.page.is_none() {
            if self.return_from_pack_shortcut(window, cx) {
                return;
            }
            self.pack_import_closed();
        }
        if matches!(self.form, Some(Form::Instance(_))) && self.shell.wizard.page.is_some() {
            let page = self.shell.wizard.page;
            self.back_game_wizard(cx);
            if let Some(page) = page {
                self.focus_wizard_card(page, window, cx);
            }
        } else if self.router.tab() == Tab::Onboarding && self.shell.first_run.enabled {
            self.first_run_overview(window, cx);
        } else if self.shell.editor.page.is_some() && matches!(self.form, Some(Form::Profile(_))) {
            self.finish_profile_page(true, window, cx);
        } else if matches!(self.form, Some(Form::Profile(_))) {
            self.cancel_profile_editor(window, cx);
        } else if self.router.tab() != Tab::Onboarding {
            self.form = None;
            self.router.leave_task();
            self.shell.wizard.cancel_loader_request();
        }
    }

    /// Show an inspected item in the browser that owns it. A linked inspection
    /// moves the page to the other collection first.
    fn inspect(&mut self, id: String, linked_tab: Option<bool>, cx: &mut Context<'_, Self>) {
        self.shell.instance_editor = None;
        let tab = if let Some(profiles) = linked_tab {
            if profiles {
                Tab::Profiles
            } else {
                Tab::Instances
            }
        } else {
            self.router.tab()
        };
        let record = if tab == Tab::Accounts {
            super::navigation::RecordRef::Account(id.clone())
        } else if tab == Tab::Profiles
            || self.snapshot.profile_mode == block_launcher::catalog::ProfileMode::Bundled
        {
            super::navigation::RecordRef::Profile(id.clone())
        } else {
            super::navigation::RecordRef::Instance(id.clone())
        };
        self.router.open_record(tab, record);
        self.initialize_browsers(cx);
        self.collection_mut().inspected = Some(id);
        self.form = None;
    }

    fn open_profile_mode_settings(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.shell.surfaces.clear(window, &self.focus, cx);
        self.open_settings(cx);
        self.router.navigate(Tab::Settings);
        self.select_settings_page(2, window, cx);
    }

    fn confirm_separate(&mut self, confirm: bool, window: &mut Window, cx: &mut Context<'_, Self>) {
        if confirm {
            self.shell
                .surfaces
                .open(Surface::SeparateProfiles, window, cx);
        } else {
            self.shell.surfaces.close(window, &self.focus, cx);
        }
    }

    fn separate_profiles(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.shell.surfaces.close(window, &self.focus, cx);
        self.begin(
            msg!("runtime.enabling_separated"),
            crate::ui_logging::Operation::SeparateProfiles,
        );
        if let Some(engine) = &self.engine {
            engine.separate_profiles();
        }
    }

    fn reload_versions(&mut self) {
        if let Some(engine) = &self.engine {
            self.version_load = browser::CollectionLoad::Loading;
            engine.load_versions();
        } else {
            self.version_load = browser::CollectionLoad::Failed(msg!("runtime.engine_retry"));
        }
    }

    fn reload_library(&mut self) {
        if let Some(engine) = &self.engine {
            self.shell.games.load = browser::CollectionLoad::Loading;
            self.shell.profiles.load = browser::CollectionLoad::Loading;
            self.shell.accounts.load = browser::CollectionLoad::Loading;
            engine.load_catalog();
        }
    }

    fn select_profile(&mut self, id: String, window: &mut Window, cx: &mut Context<'_, Self>) {
        if let Some(profile) = self
            .snapshot
            .profiles
            .iter()
            .find(|profile| profile.id == id)
        {
            self.selected_instance = Some(profile.instance_id.clone());
            self.selected_profile = Some(id);
        }
        self.push_toast(msg!("runtime.game_selected"), feedback::ToastKind::Info);
        self.shell.surfaces.clear(window, &self.focus, cx);
        self.form = None;
        self.router.leave_task();
        self.apply_profile_account();
    }

    fn select_account(&mut self, id: String, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.selected_account = Some(id);
        self.push_toast(msg!("runtime.account_selected"), feedback::ToastKind::Info);
        self.shell.surfaces.clear(window, &self.focus, cx);
    }

    /// Opening creation during setup enters the first-run game step instead of
    /// the free-standing wizard.
    fn start_creation(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.router.game_wizard() {
            return;
        }
        if self.setup_incomplete() && self.shell.first_run.enabled {
            self.handle_first_run(first_run::Action::Game, window, cx);
        } else {
            self.begin_game_wizard(cx);
            self.shell.wizard.step = WizardStep::GetStarted;
        }
    }

    fn start_creation_kind(
        &mut self,
        kind: CreationKind,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.wizard_kind(kind, cx);
        if self.shell.wizard.step != WizardStep::GetStarted || self.shell.wizard.kind != kind {
            return;
        }
        self.shell.wizard.next();
        if kind == CreationKind::ImportPack {
            self.handle_pack_import(modpack_import::Action::Choose, window, cx);
        } else {
            self.focus_field(false, window, cx);
        }
    }

    /// Start a Profile directly for the inspected Instance.
    fn new_profile(&mut self, cx: &mut Context<'_, Self>) {
        let id = self
            .inspected_instance()
            .map(|instance| instance.id.clone());
        self.begin_game_wizard(cx);
        self.wizard_kind(CreationKind::ExistingInstance, cx);
        if let Some(id) = id {
            self.wizard_instance(id);
            self.shell.wizard.context_instance = true;
            self.shell.wizard.step = WizardStep::Setup;
            self.mark_game_wizard_clean(cx);
        }
    }

    fn copy_profile(&mut self, id: String) {
        if let Some(profile) = self
            .snapshot
            .profiles
            .iter()
            .find(|profile| profile.id == id)
            .cloned()
        {
            self.begin(
                msg!("runtime.copying_profile"),
                crate::ui_logging::Operation::CopyProfile,
            );
            if let Some(engine) = &self.engine {
                engine.copy_profile(profile.id, format!("{} copy", profile.name));
            }
        }
    }

    /// Ask before deleting, unless the profile already left the catalog.
    fn request_profile_delete(
        &mut self,
        id: String,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self
            .snapshot
            .profiles
            .iter()
            .any(|profile| profile.id == id)
        {
            self.shell
                .surfaces
                .open(Surface::DeleteProfile(id), window, cx);
        } else {
            self.report(msg!("runtime.profile_unavailable"));
        }
    }

    fn confirm_profile_delete(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let id = self
            .shell
            .surfaces
            .top()
            .and_then(|layer| match &layer.kind {
                Surface::DeleteProfile(id) => Some(id.clone()),
                _ => None,
            });
        if let Some(id) = id {
            self.shell.surfaces.close(window, &self.focus, cx);
            self.begin(
                msg!("runtime.deleting_profile"),
                crate::ui_logging::Operation::DeleteProfile,
            );
            if let Some(engine) = &self.engine {
                engine.delete_profile(id);
            }
        }
    }

    fn sign_out(&mut self, id: String, window: &mut Window, cx: &mut Context<'_, Self>) {
        if let Some(account) = self.accounts.iter().find(|a| account_id(a) == id).cloned() {
            self.shell.surfaces.close(window, &self.focus, cx);
            self.begin(
                msg!("runtime.removing_credentials"),
                crate::ui_logging::Operation::RemoveAccount,
            );
            self.operation_kind = OperationKind::Account;
            if let Some(engine) = &self.engine {
                engine.sign_out(account);
            }
        }
    }

    fn repair_selected(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if let Some(id) = self.selected_profile.clone() {
            self.repair(&id, window, cx);
        }
    }

    /// Repair the selected profile, or resume the pack installation that never
    /// completed for its instance. A pending pack owns its files, so repairing
    /// it means finishing that installation rather than reinstalling a target.
    fn repair(&mut self, id: &str, window: &mut Window, cx: &mut Context<'_, Self>) {
        if let Some(profile) = self
            .snapshot
            .profiles
            .iter()
            .find(|profile| profile.id == id)
            .cloned()
        {
            let pending_pack = self
                .snapshot
                .instances
                .iter()
                .find(|instance| instance.id == profile.instance_id)
                .filter(|instance| {
                    instance.pack.as_ref().is_some_and(|pack| {
                        pack.status == block_launcher::catalog::PackStatus::Pending
                    })
                });
            if let Some(instance) = pending_pack {
                self.resume_pack_import(instance.id.clone(), window, cx);
                return;
            }
            self.begin_cancellable(
                msg!("runtime.repairing"),
                crate::ui_logging::Operation::Repair,
            );
            self.operation_kind = OperationKind::Install;
            if let Some(engine) = &self.engine {
                engine.install(profile.instance_id, profile.target_id, self.cancel.clone());
            }
        }
    }

    /// Explicit Stop: end the accepted session's launch, or cancel the
    /// cancellable operation that is still running. A queued Play is dropped
    /// either way.
    fn stop(&mut self) {
        self.pending_play = false;
        if self.can_stop() {
            self.play_stop.cancel();
            if let Some(id) = &self.active_run {
                if let Some(engine) = &self.engine {
                    engine.stop_session(id.clone());
                }
            } else {
                self.cancel.cancel();
            }
            self.status = msg!("runtime.stopping");
        }
    }

    /// Apply an edit to the open profile draft, if one is open.
    fn edit_profile_draft(&mut self, apply: impl FnOnce(&mut ProfileForm)) {
        if let Some(Form::Profile(form)) = &mut self.form {
            apply(form);
        }
    }

    fn toggle_profile_connect(&mut self) {
        self.edit_profile_draft(|form| {
            form.original.server.connect_on_launch = !form.original.server.connect_on_launch;
        });
    }

    /// Apply an edit to the open settings draft, if one is open.
    fn edit_settings_draft(&mut self, apply: impl FnOnce(&mut SettingsDraft)) {
        if let Some(form) = &mut self.settings_form {
            apply(form);
        }
    }

    fn toggle_settings_fullscreen(&mut self) {
        self.edit_settings_draft(|form| form.fullscreen = !form.fullscreen);
    }

    fn toggle_settings_snapshots(&mut self) {
        self.edit_settings_draft(|form| form.show_snapshots = !form.show_snapshots);
    }

    /// Drop the OptiFine jar recorded in the wizard draft and its import request.
    fn clear_wizard_optifine(&mut self) {
        if let Some(Form::Instance(form)) = &mut self.form {
            form.optifine = None;
        }
        self.shell.wizard.cancel_optifine_import();
    }

    fn toggle_wizard_snapshots(&mut self) {
        self.shell.wizard.include_snapshots = !self.shell.wizard.include_snapshots;
    }

    /// Apply a select-popup choice once its surface has closed.
    fn select_value(&mut self, action: UiAction, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.shell.surfaces.close(window, &self.focus, cx);
        self.handle(action, window, cx);
    }

    fn close_auth_provider_add(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.cancel_auth_provider(cx);
        self.shell.surfaces.close(window, &self.focus, cx);
    }

    /// Move the wizard forward, then settle focus on the page it opened.
    fn advance_wizard_focus(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let page = self.shell.wizard.page;
        self.advance_game_wizard(cx);
        if self.shell.wizard.page.is_none()
            && let Some(page) = page
        {
            self.focus_wizard_card(page, window, cx);
        } else {
            self.focus_field(false, window, cx);
        }
    }

    /// Commit the wizard's game step, then settle focus on the result.
    fn submit_game_and_focus(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.submit_game_wizard(cx);
        self.focus_field(false, window, cx);
    }
}
