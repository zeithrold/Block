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
    game_wizard::WizardPage,
    import_games, motion, profile_editor, select,
    settings::SettingsDraft,
    shell::{Drawer, Surface},
};

/// One concrete launcher interaction.
#[derive(Clone, Debug)]
pub(super) enum UiAction {
    FirstRun(first_run::Action),
    ImportGames(import_games::Action),
    AuthOtherOptions,
    Providers,
    ProvidersBack,
    ProviderAdd,
    ProviderInspect(String),
    ProviderIdentify,
    ProviderConsent,
    ProviderSave,
    ProviderRemoveRequest(String),
    ProviderRemove(String),
    Tab(Tab),
    Drawer(Option<Drawer>),
    DownloadFiles(downloads::DownloadGroup),
    SetAppearance(Appearance),
    SetLanguage(crate::Language),
    ReduceMotion,
    SavePreferences,
    ResetPreferences,
    ConfirmSeparate(bool),
    SeparateProfiles,
    WizardNext,
    WizardBack,
    WizardSnapshots,
    WizardOptiFineFile,
    WizardOptiFineRemove,
    WizardKind(bool),
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
    CancelInstanceSettings,
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
    AuthNext,
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
            | UiAction::ProvidersBack
            | UiAction::WizardBack
            | UiAction::AuthCancel
            | UiAction::AuthBack
            | UiAction::AuthFinish
            | UiAction::ProfilePageCancel
            | UiAction::ProfilePageDone
            | UiAction::Tab(Tab::Home)
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
        self.motion.direction = motion_direction(&action);
        if self.composition_in_progress(&action, cx) || !self.action_allowed(&action) {
            return;
        }
        let focus_first = matches!(action, UiAction::CreateInstance | UiAction::EditProfile);
        let previous_page = (self.router.tab(), self.form.is_some());
        match action {
            UiAction::FirstRun(action) => self.handle_first_run(action, window, cx),
            UiAction::ImportGames(action) => self.handle_import_games(action, window, cx),
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
            UiAction::SavePreferences => self.save_settings(window, cx),
            UiAction::SetFullscreen => self.edit_settings_draft(|form| {
                form.fullscreen = !form.fullscreen;
            }),
            UiAction::ShowSnapshots => self.edit_settings_draft(|form| {
                form.show_snapshots = !form.show_snapshots;
            }),
            UiAction::SettingsPage(page) => self.select_settings_page(page, window, cx),
            UiAction::ProxyMode(mode) => self.edit_settings_draft(|form| {
                form.proxy_mode = mode;
            }),
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
            UiAction::CancelInstanceSettings => self.cancel_instance_editor(window, cx),
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
            UiAction::WizardKind(is_new) => self.wizard_kind(is_new, cx),
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
            UiAction::ProfileConnect => self.edit_profile_draft(|form| {
                form.original.server.connect_on_launch = !form.original.server.connect_on_launch;
            }),
            UiAction::WizardRuntime(value) => self.wizard_runtime(value),
            UiAction::WizardWindow { width, height } => self.wizard_window(width, height, cx),
            UiAction::WizardCustomWindow => self.wizard_custom_window(),
            UiAction::WizardProfileConnect => self.wizard_profile_connect(),
            UiAction::MinimizeWindow => window.minimize_window(),
            UiAction::CloseWindow => window.remove_window(),
            UiAction::SelectValue(action) => self.select_value(*action, window, cx),
            UiAction::ProfileIcon(icon) => self.edit_profile_draft(|form| {
                form.original.appearance.icon = icon;
            }),
            UiAction::ProfileColor(color) => self.edit_profile_draft(|form| {
                form.original.appearance.color = color;
            }),
            UiAction::WizardSnapshots => self.toggle_wizard_snapshots(),
            UiAction::WizardNext => self.advance_wizard_focus(window, cx),
            UiAction::WizardBack => self.back_game_wizard(cx),
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
            UiAction::CancelForm => self.cancel_form(window, cx),
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
            UiAction::ProvidersBack => self.close_providers(cx),
            UiAction::ProviderAdd => self.edit_provider(None, cx),
            UiAction::ProviderInspect(id) => self.edit_provider(Some(&id), cx),
            UiAction::ProviderIdentify => self.identify_provider(cx),
            UiAction::ProviderConsent => self.consent_provider(cx),
            UiAction::ProviderSave => self.save_provider(cx),
            UiAction::ProviderRemoveRequest(id) => self.request_remove_provider(id, window, cx),
            UiAction::ProviderRemove(id) => self.remove_provider(id, window, cx),
            UiAction::AuthNext => self.auth_next(cx),
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
            UiAction::Repair => {
                if let Some(id) = self.selected_profile.clone() {
                    self.repair(&id);
                }
            }
            UiAction::RepairProfile(id) => self.repair(&id),
            UiAction::Stop => self.stop(),
            UiAction::ToggleOutput => self.show_output = !self.show_output,
            UiAction::ToggleCommand => self.show_command = !self.show_command,
            UiAction::OpenLogs => self.open_launcher_logs(cx),
            UiAction::ExportLogs => self.export_launcher_logs(cx),
            UiAction::CancelLogExport => {
                if let Some(cancel) = &self.log_export_cancel {
                    cancel.cancel();
                }
            }
            UiAction::OpenRecipe => self.open_recipe(cx),
            UiAction::LaunchRecipe => self.launch_recipe(cx),
        }
        self.finish_action(previous_page, focus_first, window, cx);
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
            self.request_frame_resize(window, cx);
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

    /// A commit or page change must not interrupt an IME composition: the draft
    /// still holds the text being assembled.
    fn composition_in_progress(&self, action: &UiAction, cx: &Context<'_, Self>) -> bool {
        if matches!(action, UiAction::SavePreferences) {
            return self.settings_form.as_ref().is_some_and(|draft| {
                draft
                    .fields()
                    .iter()
                    .any(|field| field.read(cx).is_composing())
            });
        }
        matches!(
            action,
            UiAction::SaveInstance | UiAction::SaveProfile | UiAction::WizardNext
        ) && self
            .form_inputs()
            .iter()
            .any(|field| field.read(cx).is_composing())
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
                | UiAction::Tab(_)
                | UiAction::Providers
                | UiAction::ProvidersBack
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
        self.initialize_browsers(cx);
        self.router.navigate(tab);
        self.form = None;
        self.shell.instance_editor = None;
        self.shell.surfaces.clear(window, &self.focus, cx);
        self.request_frame_resize(window, cx);
        true
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
        } else if self
            .shell
            .surfaces
            .top()
            .is_some_and(|entry| matches!(entry.kind, Surface::WizardSection(_)))
        {
            self.cancel_component_draft(cx);
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
        if let Some(page) = page
            && self
                .shell
                .surfaces
                .layers()
                .any(|entry| matches!(entry.kind, Surface::WizardSection(_)))
        {
            self.shell
                .surfaces
                .open(Surface::WizardSection(page), window, cx);
            return false;
        }
        self.open_game_wizard_page(page, cx);
        if let Some(page) = page {
            self.shell
                .surfaces
                .open(Surface::WizardSection(page), window, cx);
        } else if self.shell.wizard.page.is_none() {
            self.shell.surfaces.close(window, &self.focus, cx);
        }
        true
    }

    /// Back out of the open form: leave the setup guide, step back through the
    /// wizard, close the profile editor or drop an unattached game draft.
    fn cancel_form(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.router.tab() == Tab::Onboarding && self.shell.first_run.enabled {
            self.first_run_overview(window, cx);
        } else if self.shell.editor.page.is_some() && matches!(self.form, Some(Form::Profile(_))) {
            self.finish_profile_page(true, window, cx);
        } else if matches!(self.form, Some(Form::Instance(_))) && self.shell.wizard.page.is_some() {
            self.back_game_wizard(cx);
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
        if let Some(profiles) = linked_tab {
            self.router.navigate(if profiles {
                Tab::Profiles
            } else {
                Tab::Instances
            });
            self.initialize_browsers(cx);
        }
        self.collection_mut().inspected = Some(id);
        self.form = None;
        self.router.leave_task();
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
        if self.setup_incomplete() && self.shell.first_run.enabled {
            self.handle_first_run(first_run::Action::Game, window, cx);
        } else {
            self.begin_game_wizard(cx);
        }
    }

    /// Start a profile for the inspected game without leaving the wizard's
    /// profile step.
    fn new_profile(&mut self, cx: &mut Context<'_, Self>) {
        let id = self
            .inspected_instance()
            .map(|instance| instance.id.clone());
        self.begin_game_wizard(cx);
        self.wizard_kind(false, cx);
        if let Some(id) = id {
            self.wizard_instance(id);
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

    fn repair(&mut self, id: &str) {
        if let Some(profile) = self
            .snapshot
            .profiles
            .iter()
            .find(|profile| profile.id == id)
            .cloned()
        {
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

    /// Apply an edit to the open settings draft, if one is open.
    fn edit_settings_draft(&mut self, apply: impl FnOnce(&mut SettingsDraft)) {
        if let Some(form) = &mut self.settings_form {
            apply(form);
        }
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
        self.advance_game_wizard(cx);
        self.focus_field(false, window, cx);
    }

    /// Commit the wizard's game step, then settle focus on the result.
    fn submit_game_and_focus(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.submit_game_wizard(cx);
        self.focus_field(false, window, cx);
    }
}
