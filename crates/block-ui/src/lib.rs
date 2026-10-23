//! A native launcher with persistent installations, launch profiles and Microsoft accounts.

mod about;
mod appearance;
mod authentication;
mod base_controls;
mod base_overlay;
mod browser;
mod components;
mod connection_test;
mod crash_view;
mod design;
mod downloads;
mod drawer;
mod drawer_gestures;
mod engine;
mod feedback;
mod game_wizard;
mod instance_editor;
#[cfg(test)]
mod interaction_tests;
mod motion;
#[cfg(test)]
mod native_regressions;
mod onboarding;
mod overlays;
mod patterns;
mod platform;
mod profile_editor;
#[cfg(test)]
mod profile_editor_tests;
mod providers;
#[cfg(debug_assertions)]
mod scenarios;
mod scroll_area;
mod search_selector;
mod select;
mod settings;
mod shell;
mod tasks;
#[cfg(test)]
mod test_support;
mod text_input;
mod tooltip;
mod transitions;
mod window_geometry;

pub use about::BuildInfo;
pub use crash_view::CrashView;
pub use design::{UiAssets, load_fonts};
pub use gpui_base::Root as ProductRoot;
#[cfg(debug_assertions)]
pub use scenarios::{SCENARIOS, ScenarioOptions};

/// Register the shared unstyled foundation once before opening product windows.
pub fn initialize(cx: &mut gpui::App) {
    if !cx.has_global::<gpui_base::GlobalState>() {
        gpui_base::init(cx);
    }
    // Modal semantics stay in Base Sheet/Dialog. Product drawer motion sits
    // below dialogs, and transient feedback sits above both.
    ProductRoot::register_plugin(cx, drawer::DrawerLayer::new);
    gpui_component::init(cx);
    ProductRoot::register_plugin(cx, feedback::ToastLayer::new);
}

fn bind_navigation_keys(cx: &mut gpui::App) {
    // Settle the product entrance before forwarding once to Base Root. There
    // is no second traversal or containment loop in this binding.
    cx.bind_keys([
        KeyBinding::new("tab", NextField, Some("LauncherForm")),
        KeyBinding::new("shift-tab", PreviousField, Some("LauncherForm")),
    ]);
}

use std::{
    collections::{HashMap, VecDeque},
    fs::File,
    path::PathBuf,
    time::Duration,
};

use block_auth::Account;
use block_launcher::{
    CancellationToken,
    catalog::{DataPolicy, Instance, Profile, Snapshot},
    crash::CrashReport,
    platform::Architecture,
    recipe::{LaunchRecipe, RuntimeSelection},
    session_host::{self, RunRecord, RunState},
};
use gpui::{
    AppContext as _, Bounds, Context, Entity, FocusHandle, Focusable as _, Image, ImageFormat,
    InteractiveElement as _, IntoElement, KeyBinding, ParentElement as _, PathPromptOptions,
    Pixels, Render, SharedString, Styled as _, WeakEntity, Window, div, px, rgb,
};
#[cfg(any(windows, target_os = "linux", target_os = "macos"))]
use raw_window_handle::RawWindowHandle;
use tokio::sync::mpsc::Receiver;

use appearance::{Appearance, Palette, Preferences};
use engine::{Engine, Message};
use game_wizard::{WizardPage, WizardStep};
use scroll_area::ScrollAreaExt as _;
use shell::{Drawer, ShellState};
use text_input::TextInput;

/// Explicit development recipe input. Normal startup uses the persisted library.
#[derive(Debug, Default)]
pub struct LaunchOptions {
    pub recipe: Option<PathBuf>,
    pub launch: bool,
    pub profile: Option<String>,
    pub data_dir: Option<PathBuf>,
    pub build_info: BuildInfo,
    #[cfg(debug_assertions)]
    pub scenario: Option<ScenarioOptions>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Tab {
    #[default]
    Home,
    Onboarding,
    Instances,
    Profiles,
    Accounts,
    Settings,
    About,
}

#[derive(Clone, Debug)]
enum UiAction {
    Providers,
    ProvidersBack,
    ProviderAdd,
    ProviderList,
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
    ReduceMotion,
    SavePreferences,
    ResetPreferences,
    StorageMoveGames,
    StorageReviewCleanup,
    StorageCleanup(uuid::Uuid, bool),
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
    OnboardingNext,
    OnboardingBack,
    OnboardingSkip,
    OnboardingAccountLater,
    OnboardingSection(WizardPage),
    OnboardingSectionDone,
    OnboardingSectionCancel,
    OnboardingChange(onboarding::Step),
    OnboardingChooseAccount,
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
    SaveInstanceSettings,
    CancelInstanceSettings,
    InstanceFullscreen,
    InstanceOptiFineFile,
    InstanceOptiFineRemove,
    ProfileInheritRuntime,
    ProfileInheritWindow,
    BrowserBack,
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
    Stop,
    ToggleOutput,
    ToggleCommand,
    OpenRecipe,
    LaunchRecipe,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, gpui::Action)]
#[action(namespace = launcher_form)]
struct NextField;
#[derive(Clone, Debug, Default, PartialEq, Eq, gpui::Action)]
#[action(namespace = launcher_form)]
struct PreviousField;

#[derive(Debug)]
struct InstanceForm {
    name: Entity<TextInput>,
    version: Entity<TextInput>,
    directory: Entity<TextInput>,
    loader: block_launcher::catalog::LoaderKind,
    loader_version: Option<String>,
    optifine: Option<Box<block_launcher::install::optifine::OptiFineJar>>,
}

impl InstanceForm {
    #[cfg(test)]
    fn is_neoforge(&self) -> bool {
        self.loader == block_launcher::catalog::LoaderKind::NeoForge
    }

    fn loader_label(&self) -> String {
        let loader = self.loader_version.as_ref().map_or_else(
            || self.loader.name().to_owned(),
            |version| format!("{} {version}", self.loader.name()),
        );
        self.optifine.as_ref().map_or(loader.clone(), |selected| {
            if self.loader == block_launcher::catalog::LoaderKind::Vanilla {
                format!("OptiFine {}", selected.version)
            } else {
                format!("{loader} + OptiFine {}", selected.version)
            }
        })
    }

    fn selected_loader(&self) -> Option<(block_launcher::catalog::LoaderKind, String)> {
        self.loader_version
            .as_ref()
            .map(|version| (self.loader.clone(), version.clone()))
    }
}

#[derive(Debug)]
struct ProfileForm {
    original: Profile,
    name: Entity<TextInput>,
    memory: Entity<TextInput>,
    java: Entity<TextInput>,
    jvm: Entity<TextInput>,
    game: Entity<TextInput>,
    server: Entity<TextInput>,
    managed: bool,
}

#[derive(Debug)]
struct PendingCrash {
    report: CrashReport,
    directory: PathBuf,
    guard: File,
}

#[derive(Debug)]
enum Form {
    Instance(InstanceForm),
    Profile(Box<ProfileForm>),
}

/// The GPUI view owns only public account metadata and editable settings.
#[derive(Debug)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "Product visibility, lifecycle and cancellability are independent state flags."
)]
pub struct Block {
    build_info: BuildInfo,
    shell: ShellState,
    motion: transitions::MotionState,
    motion_dirty: bool,
    visual_only: bool,
    pending_window_size: Option<gpui::Size<Pixels>>,
    onboarding_progress: onboarding::Progress,
    settings_form: Option<settings::SettingsDraft>,
    preferences: Preferences,
    saved_preferences: Preferences,
    palette: Palette,
    snapshot: Snapshot,
    accounts: Vec<Account>,
    account_avatars: HashMap<String, components::AccountAvatar>,
    provider_icons: HashMap<String, components::ProviderBadge>,
    versions: Vec<String>,
    /// Test-only architecture override for version-availability policy.
    host_arch_override: Option<Architecture>,
    version_load: browser::CollectionLoad,
    version_catalog: Vec<minecraft_manifest::model::VersionEntry>,
    selected_instance: Option<String>,
    selected_profile: Option<String>,
    pending_profile: Option<String>,
    selected_account: Option<String>,
    tab: Tab,
    form: Option<Form>,
    engine: Option<Engine>,
    messages: Option<Receiver<Message>>,
    focus: FocusHandle,
    status: String,
    completed: usize,
    operation_kind: OperationKind,
    operation_revision: u64,
    pending_creation: Option<PendingCreation>,
    downloads: Option<std::sync::Arc<std::sync::Mutex<tasks::DownloadTracker>>>,
    downloads_revision: u64,
    busy: bool,
    operation_cancellable: bool,
    cancel: CancellationToken,
    play_stop: CancellationToken,
    active_run: Option<String>,
    active_started_at: u64,
    session_revisions: HashMap<String, (u64, RunState)>,
    pending_crashes: VecDeque<PendingCrash>,
    crash_help: Option<Entity<CrashView>>,
    crash_guard: Option<File>,
    logs: VecDeque<String>,
    show_output: bool,
    command: Option<String>,
    show_command: bool,
    recipe: Option<LaunchRecipe>,
    recipe_load: RecipeLoadState,
    /// Owned native drop target (OLE on Windows, AppKit on macOS, XDND on
    /// Linux X11) over the rendered provider drop zone.
    #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
    provider_drop: Option<ProviderDrop>,
    /// A Play requested while another operation was running, started on completion.
    pending_play: bool,
}

#[derive(Debug, Default)]
struct RecipeLoadState {
    launch_after_load: Option<bool>,
}

impl RecipeLoadState {
    const fn start(&mut self, busy: bool, launch: bool) -> bool {
        if busy || self.is_loading() {
            return false;
        }
        self.launch_after_load = Some(launch);
        true
    }
    const fn is_loading(&self) -> bool {
        self.launch_after_load.is_some()
    }
    fn finish(&mut self, success: bool) -> bool {
        self.launch_after_load
            .take()
            .is_some_and(|launch| launch && success)
    }
}

/// Owns the native child window that receives provider quick-link drops.
#[cfg(any(windows, target_os = "linux", target_os = "macos"))]
#[derive(Debug)]
struct ProviderDrop {
    target: block_drop::DropTarget,
}

impl Drop for Block {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum OperationKind {
    #[default]
    General,
    Install,
    Launch,
    Account,
}

#[derive(Clone, Copy, Debug)]
struct PendingCreation {
    operation: u64,
    tab: Tab,
    draft: u64,
}

impl Block {
    #[must_use]
    #[expect(
        clippy::too_many_lines,
        reason = "Initial product state and startup tasks are constructed together."
    )]
    pub fn new(window: &mut Window, cx: &mut Context<'_, Self>, options: LaunchOptions) -> Self {
        initialize(cx);
        text_input::bind_keys(cx);
        bind_navigation_keys(cx);
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        #[cfg(not(debug_assertions))]
        let scenario_mode = false;
        #[cfg(debug_assertions)]
        let scenario_mode = options.scenario.is_some();
        let engine = if scenario_mode {
            None
        } else {
            Some(
                options
                    .data_dir
                    .clone()
                    .map_or_else(Engine::new, Engine::with_root),
            )
        };
        let (engine, messages, status) = match engine {
            Some(Ok((engine, messages))) => {
                engine.load_catalog();
                engine.load_versions();
                engine.load_images();
                (Some(engine), Some(messages), "Welcome to Block".into())
            }
            Some(Err(error)) => (None, None, format!("Engine unavailable: {error:#}")),
            None => (None, None, "UI scenario fixture".into()),
        };
        if !scenario_mode {
            cx.spawn(async move |view, cx| {
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(100))
                        .await;
                    if view.update(cx, Self::poll).is_err() {
                        break;
                    }
                }
            })
            .detach();
        }
        let preferences = engine
            .as_ref()
            .map(|engine| Preferences::load(&engine.root))
            .transpose();
        let (preferences, status) = match preferences {
            Ok(value) => (value.unwrap_or_default(), status),
            Err(error) => (
                Preferences::default(),
                format!("Could not read preferences: {error}"),
            ),
        };
        let tab = if preferences.onboarding_complete {
            Tab::Home
        } else {
            Tab::Onboarding
        };
        if tab == Tab::Home {
            window.resize(window_geometry::home_size(window, cx));
        } else {
            window.resize(window_geometry::management_size(window, cx));
        }
        let mut view = Self {
            build_info: options.build_info,
            motion: transitions::MotionState::default(),
            motion_dirty: true,
            visual_only: false,
            shell: ShellState {
                ..ShellState::default()
            },
            pending_window_size: None,
            onboarding_progress: onboarding::Progress::Draft,
            settings_form: None,
            saved_preferences: preferences.clone(),
            palette: preferences.palette(window.appearance()),
            preferences,
            snapshot: Snapshot::default(),
            accounts: Vec::new(),
            account_avatars: HashMap::new(),
            provider_icons: HashMap::new(),
            versions: Vec::new(),
            host_arch_override: None,
            version_load: if engine.is_some() {
                browser::CollectionLoad::Loading
            } else {
                browser::CollectionLoad::Failed("Engine unavailable".into())
            },
            version_catalog: Vec::new(),
            selected_instance: None,
            selected_profile: None,
            pending_profile: options.profile,
            selected_account: None,
            tab,
            form: None,
            engine,
            messages,
            focus,
            status,
            completed: 0,
            operation_kind: OperationKind::General,
            operation_revision: 0,
            pending_creation: None,
            downloads: None,
            downloads_revision: 0,
            busy: false,
            operation_cancellable: false,
            cancel: CancellationToken::new(),
            play_stop: CancellationToken::new(),
            active_run: None,
            active_started_at: 0,
            session_revisions: HashMap::new(),
            pending_crashes: VecDeque::new(),
            crash_help: None,
            crash_guard: None,
            logs: VecDeque::new(),
            show_output: false,
            command: None,
            show_command: false,
            recipe: None,
            recipe_load: RecipeLoadState::default(),
            #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
            provider_drop: None,
            pending_play: false,
        };
        view.initialize_browsers(cx);
        let load = if view.engine.is_some() {
            browser::CollectionLoad::Loading
        } else {
            browser::CollectionLoad::Failed(view.status.clone())
        };
        view.shell.games.load = load.clone();
        view.shell.profiles.load = load.clone();
        view.shell.accounts.load = load;
        if tab == Tab::Onboarding {
            view.begin_onboarding(cx);
            view.focus_field(false, window, cx);
        }
        if let Some(path) = options.recipe {
            view.load_recipe(path, options.launch, cx);
        } else if options.launch {
            view.status = "Select a profile and account before playing".into();
        }
        #[cfg(debug_assertions)]
        if let Some(scenario) = options.scenario {
            let owner = cx.entity();
            window.on_next_frame(move |window, cx| {
                owner.update(cx, |view, cx| {
                    if let Err(error) = view.apply_scenario(&scenario, window, cx) {
                        view.status = format!("Could not prepare UI scenario: {error:#}");
                    }
                });
            });
        }
        view
    }

    #[expect(
        clippy::too_many_lines,
        reason = "Typed engine messages share one lifecycle and stale-operation gate."
    )]
    fn poll(&mut self, cx: &mut Context<'_, Self>) {
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
        if messages.is_empty() && !download_changed && !toast_expired {
            return;
        }
        self.motion_dirty = true;
        for message in messages {
            let message = match message {
                Message::ForOperation { operation, message }
                    if operation == self.operation_revision =>
                {
                    *message
                }
                Message::ForOperation { .. } => continue,
                message => message,
            };
            match message {
                Message::ForOperation { .. } => {}
                Message::Catalog(Ok(snapshot)) => {
                    self.apply_catalog(snapshot, cx);
                    self.shell.games.load = browser::CollectionLoad::Ready;
                    self.shell.profiles.load = browser::CollectionLoad::Ready;
                }
                Message::Catalog(Err(error)) => {
                    self.shell.games.load = browser::CollectionLoad::Failed(error.clone());
                    self.shell.profiles.load = browser::CollectionLoad::Failed(error.clone());
                    self.report(error);
                }
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
                Message::Accounts(Err(error)) => {
                    self.shell.accounts.load = browser::CollectionLoad::Failed(error.clone());
                    self.report(error);
                }
                Message::Accounts(Ok(accounts)) => {
                    self.shell.accounts.load = browser::CollectionLoad::Ready;
                    self.reconcile_account_browser(&accounts);
                    self.accounts = accounts;
                    if self.tab != Tab::Onboarding
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
                Message::AccountImages(images) => {
                    self.account_avatars = images
                        .into_iter()
                        .map(|(identity, skin)| {
                            (
                                identity,
                                components::AccountAvatar {
                                    image: std::sync::Arc::new(Image::from_bytes(
                                        ImageFormat::Png,
                                        skin.bytes,
                                    )),
                                    width: skin.width,
                                    height: skin.height,
                                },
                            )
                        })
                        .collect();
                }
                Message::ProviderImages(icons) => {
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
                Message::Versions(Ok(versions)) => {
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
                Message::Versions(Err(error)) => {
                    self.version_load = browser::CollectionLoad::Failed(error.clone());
                    self.push_log(format!("Version list unavailable: {error}"));
                }
                Message::LoaderVersions { request, result } => {
                    self.receive_loader_versions(request, result, cx)
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
                Message::StorageProgress(id, progress) => self.storage_progress(id, progress),
                Message::StorageFinished(finished) => self.storage_finished(*finished, cx),
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
                    self.session_updated(*record);
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
                Message::Finished(result) => {
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
                            if self.tab != Tab::Onboarding
                                && self.shell.auth.is_none()
                                && self.operation_kind != OperationKind::Account
                            {
                                self.form = None;
                            }
                        }
                        Err(_) if self.cancel.is_cancelled() => {
                            self.discard_uncommitted_onboarding_identity();
                            self.status = "Cancelled · retry when ready".into();
                        }
                        Err(error) => {
                            self.discard_uncommitted_onboarding_identity();
                            self.report(format!("Failed · {error}"));
                        }
                    }
                    self.start_pending_play(cx);
                }
                Message::Recipe(result) => {
                    let launch = self.recipe_load.finish(result.is_ok());
                    match result {
                        Ok(recipe) => {
                            self.recipe = Some(recipe);
                            self.status =
                                "Development recipe loaded · offline identity is explicit".into();
                            if launch {
                                self.launch_recipe(cx);
                            }
                        }
                        Err(error) => self.report(format!("Invalid recipe: {error}")),
                    }
                }
            }
        }
        self.sync_form_inputs(cx);
        cx.notify();
    }

    fn apply_catalog(&mut self, snapshot: Snapshot, cx: &mut Context<'_, Self>) {
        let previous = self
            .profile()
            .map(|profile| (profile.id.clone(), profile.default_account.clone()));
        self.snapshot = snapshot;
        if let Some(profile) = self.pending_profile.take() {
            self.select_requested_profile(&profile);
        }
        self.normalize_selection();
        // The catalog decides Bundled vs Separated browsing, so the selector
        // labels must follow the loaded mode instead of their creation-time one.
        self.initialize_browsers(cx);
        if previous
            != self
                .profile()
                .map(|profile| (profile.id.clone(), profile.default_account.clone()))
        {
            self.apply_profile_account();
        }
    }

    fn select_requested_profile(&mut self, id: &str) {
        if let Some(profile) = self
            .snapshot
            .profiles
            .iter()
            .find(|profile| profile.id == id)
        {
            self.selected_instance = Some(profile.instance_id.clone());
            self.selected_profile = Some(profile.id.clone());
            if self.form.is_none()
                && self.shell.auth.is_none()
                && self.shell.instance_editor.is_none()
            {
                if self.tab == Tab::Home {
                    self.pending_window_size = Some(gpui::size(px(960.), px(600.)));
                }
                self.tab = Tab::Instances;
            }
            self.apply_profile_account();
        } else {
            self.status =
                "The requested profile is no longer available. Select another profile.".into();
        }
    }

    fn session_updated(&mut self, record: RunRecord) {
        let id = record.context.run_id.clone();
        let fingerprint = (record.revision, record.state);
        if let Some(previous) = self.session_revisions.get(&id)
            && (previous.0 > record.revision || previous == &fingerprint)
        {
            return;
        }
        self.session_revisions.insert(id.clone(), fingerprint);
        if self.session_revisions.len() > 256 {
            self.session_revisions.retain(|id, _| {
                Some(id) == self.active_run.as_ref() || id == &record.context.run_id
            });
        }
        if record.is_active() {
            if self.active_run.as_ref() != Some(&id)
                && record.started_at_unix_ms < self.active_started_at
            {
                return;
            }
            self.active_started_at = record.started_at_unix_ms;
            if self.active_run.is_none() {
                if self
                    .snapshot
                    .profiles
                    .iter()
                    .any(|profile| profile.id == record.context.profile_id)
                {
                    self.select_requested_profile(&record.context.profile_id);
                } else {
                    self.pending_profile = Some(record.context.profile_id.clone());
                }
            }
            self.active_run = Some(id);
            self.busy = true;
            self.status = record.status;
            self.logs.clear();
            for line in record.recent_output {
                self.push_log(line);
            }
            for warning in record.capture_warnings {
                self.push_log(warning);
            }
        } else if self.active_run.as_ref() == Some(&id) {
            self.active_run = None;
            self.busy = false;
            self.status = record.status;
            for line in record.recent_output {
                self.push_log(line);
            }
        }
    }

    fn prepare_crash_help(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.crash_help.is_some() {
            return;
        }
        let Some(pending) = self.pending_crashes.pop_front() else {
            return;
        };
        let profile = pending.report.context.profile_id.clone();
        let run = pending.report.context.run_id.clone();
        let launcher = cx.weak_entity();
        let view = cx.new(|cx| {
            CrashView::new(
                pending.report,
                pending.directory,
                move |window, cx| {
                    let _ = launcher.update(cx, |view, cx| {
                        view.crash_help = None;
                        view.crash_guard = None;
                        view.select_requested_profile(&profile);
                        view.focus.focus(window, cx);
                        cx.notify();
                    });
                },
                window,
                cx,
            )
        });
        self.crash_guard = Some(pending.guard);
        self.crash_help = Some(view);
        if let Some(engine) = &self.engine
            && let Err(error) = session_host::mark_seen(&engine.root, &run)
        {
            self.push_log(format!(
                "Could not mark this diagnostic report as read: {error:#}"
            ));
        }
    }

    fn mark_creation(&mut self) {
        if self.operation_cancellable {
            self.operation_kind = OperationKind::Install;
        }
        if let Some(Form::Instance(form)) = &self.form {
            self.pending_creation = Some(PendingCreation {
                operation: self.operation_revision,
                tab: self.tab,
                draft: form.name.entity_id().as_u64(),
            });
        }
    }

    /// A failed or cancelled first-run commit rolls its new identity back, so
    /// the draft no longer holds a prepared account and must add one again.
    fn discard_uncommitted_onboarding_identity(&mut self) {
        if self.tab == Tab::Onboarding
            && self.operation_kind == OperationKind::Install
            && self.shell.onboarding.pending_identity.take().is_some()
        {
            self.shell.onboarding.account = None;
            self.shell.onboarding.account_decided = false;
        }
    }

    fn creation_saved(&mut self, instance: String, profile: String, cx: &mut Context<'_, Self>) {
        let context = self
            .pending_creation
            .take()
            .filter(|context| context.operation == self.operation_revision);
        let owns_draft = context.is_some_and(|context| context.tab == self.tab && matches!(&self.form, Some(Form::Instance(form)) if form.name.entity_id().as_u64() == context.draft));
        self.selected_instance = Some(instance.clone());
        self.selected_profile = Some(profile.clone());
        self.shell.games.inspected = Some(
            if self.snapshot.profile_mode == block_launcher::catalog::ProfileMode::Bundled {
                profile.clone()
            } else {
                instance
            },
        );
        self.shell.profiles.inspected = Some(profile);
        self.shell.games.detail_visible = true;
        self.shell.profiles.detail_visible = true;
        if owns_draft {
            if self.tab == Tab::Onboarding {
                self.onboarding_progress = onboarding::Progress::Created;
                self.complete_onboarding();
            } else {
                self.form = None;
                if let Some(query) = &self.collection().query {
                    query.update(cx, |field, cx| field.set_value("", cx));
                }
            }
        }
    }

    fn normalize_selection(&mut self) {
        if self.tab == Tab::Onboarding {
            return;
        }
        let previous_profile = self.selected_profile.clone();
        if !self
            .snapshot
            .instances
            .iter()
            .any(|i| Some(&i.id) == self.selected_instance.as_ref())
        {
            self.selected_instance = self.snapshot.instances.last().map(|i| i.id.clone());
        }
        if !self.snapshot.profiles.iter().any(|p| {
            Some(&p.id) == self.selected_profile.as_ref()
                && Some(&p.instance_id) == self.selected_instance.as_ref()
        }) {
            self.selected_profile = self
                .snapshot
                .profiles
                .iter()
                .rev()
                .find(|p| Some(&p.instance_id) == self.selected_instance.as_ref())
                .map(|p| p.id.clone());
        }
        if previous_profile != self.selected_profile {
            self.apply_profile_account();
        }
    }

    fn apply_profile_account(&mut self) {
        if self.tab == Tab::Onboarding {
            return;
        }
        if let Some(default) = self
            .profile()
            .and_then(|profile| profile.default_account.as_ref())
        {
            self.selected_account = self
                .accounts
                .iter()
                .find(|account| account_id(account) == *default)
                .map(account_id);
        } else if !self
            .accounts
            .iter()
            .any(|account| Some(account_id(account)) == self.selected_account)
        {
            self.selected_account = self.accounts.first().map(account_id);
        }
    }

    fn push_log(&mut self, line: String) {
        if self.logs.len() >= 200 {
            self.logs.pop_front();
        }
        self.logs.push_back(line);
    }

    fn push_toast(&mut self, message: String, kind: feedback::ToastKind) {
        let toast = feedback::Toast::new(message, kind);
        self.shell.toast = Some(toast.clone());
        self.shell.pending_toasts.push(toast);
    }

    /// True while the first-run setup draft is active (the setup page or its
    /// Settings/About preview), so menu routes cannot bypass setup.
    fn setup_incomplete(&self) -> bool {
        !self.preferences.onboarding_complete
            && (self.tab == Tab::Onboarding || self.shell.guide_preview.is_some())
    }

    fn report(&mut self, error: String) {
        self.push_toast(error.clone(), feedback::ToastKind::Error);
        if self.tab == Tab::Onboarding {
            self.shell.onboarding.error = Some(error.clone());
        }
        self.push_log(error.clone());
        self.status = error;
    }

    fn begin_cancellable(&mut self, status: &str) {
        self.begin(status);
        self.operation_cancellable = self.busy;
    }

    const fn can_stop(&self) -> bool {
        self.busy && (self.active_run.is_some() || self.operation_cancellable)
    }

    fn begin(&mut self, status: &str) {
        self.operation_cancellable = false;
        self.operation_kind = OperationKind::General;
        if self.tab == Tab::Onboarding {
            self.shell.onboarding.error = None;
        }
        if self.engine.is_none() {
            self.status =
                "Engine unavailable · restart Block after checking its data directory".into();
            return;
        }
        self.operation_revision = self.operation_revision.wrapping_add(1);
        self.pending_creation = None;
        if let Some(engine) = &mut self.engine {
            engine.begin_operation(self.operation_revision);
        }
        self.busy = true;
        self.completed = 0;
        self.downloads = None;
        self.downloads_revision = 0;
        self.command = None;
        self.show_command = false;
        self.logs.clear();
        self.status = status.into();
        self.cancel = CancellationToken::new();
    }

    /// Leave the Welcome Settings/About preview and restore the setup page,
    /// keeping its draft and undoing an unsaved settings preview.
    fn leave_guide_preview(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some((tab, size)) = self.shell.guide_preview.take() else {
            return;
        };
        if tab == Tab::Settings {
            self.preferences.clone_from(&self.saved_preferences);
            self.settings_form = None;
        }
        self.tab = Tab::Onboarding;
        window.resize(size);
        self.shell.surfaces.clear(window, &self.focus, cx);
        cx.notify();
    }

    fn instance(&self) -> Option<&Instance> {
        self.snapshot
            .instances
            .iter()
            .find(|i| Some(&i.id) == self.selected_instance.as_ref())
    }

    fn profile(&self) -> Option<&Profile> {
        self.snapshot
            .profiles
            .iter()
            .find(|p| Some(&p.id) == self.selected_profile.as_ref())
    }

    #[expect(
        clippy::too_many_lines,
        reason = "Concrete UI actions are kept in one dispatcher without a controller framework."
    )]
    fn handle(&mut self, action: UiAction, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.motion_dirty = true;
        self.motion.direction = if matches!(
            action,
            UiAction::CancelForm
                | UiAction::ProvidersBack
                | UiAction::ProviderList
                | UiAction::BrowserBack
                | UiAction::WizardBack
                | UiAction::OnboardingBack
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
        };
        if matches!(
            action,
            UiAction::SaveInstance
                | UiAction::SaveProfile
                | UiAction::WizardNext
                | UiAction::OnboardingNext
                | UiAction::OnboardingSectionDone
        ) && self
            .form_inputs()
            .iter()
            .any(|field| field.read(cx).is_composing())
        {
            return;
        }
        let retry_setup_save = matches!(action, UiAction::OnboardingNext)
            && self.tab == Tab::Onboarding
            && self.onboarding_progress == onboarding::Progress::Created;
        if self.busy && !retry_setup_save && !self.can_activate(&action) {
            return;
        }
        let focus_first = matches!(action, UiAction::CreateInstance | UiAction::EditProfile);
        let previous_page = (self.tab, self.form.is_some());
        match action {
            UiAction::WizardOptiFineFile => self.browse_optifine(cx),
            UiAction::WizardOptiFineRemove => {
                if let Some(Form::Instance(form)) = &mut self.form {
                    form.optifine = None;
                }
                self.shell.wizard.cancel_optifine_import();
            }
            UiAction::OnboardingSection(page) => self.open_onboarding_section(page, window, cx),
            UiAction::OnboardingSectionDone => self.finish_onboarding_section(false, window, cx),
            UiAction::OnboardingSectionCancel => self.finish_onboarding_section(true, window, cx),
            UiAction::OnboardingChange(step) => self.change_onboarding_step(step, window, cx),
            UiAction::OnboardingChooseAccount => {
                self.shell
                    .surfaces
                    .open(shell::Surface::Drawer(Drawer::Accounts), window, cx);
                self.open_selector(Drawer::Accounts, window, cx);
            }
            UiAction::OnboardingAccountLater | UiAction::OnboardingNext => {
                self.advance_onboarding(window, cx);
            }
            UiAction::OnboardingBack => self.back_onboarding(window, cx),
            UiAction::OnboardingSkip => {
                if self.tab == Tab::Onboarding
                    && self.shell.onboarding.step == onboarding::Step::Review
                    && self.busy
                    && self.onboarding_progress == onboarding::Progress::Draft
                {
                    return;
                }
                if self.busy {
                    self.cancel.cancel();
                }
                self.complete_onboarding();
            }
            UiAction::Tab(tab) => {
                if self.shell.providers.visible {
                    self.close_providers(cx);
                }
                if self.shell.auth.is_some() {
                    self.cancel_auth(window, cx);
                }
                if self.setup_incomplete()
                    && !matches!(tab, Tab::Onboarding | Tab::Settings | Tab::About)
                {
                    // Setup owns navigation until it is finished: keep the draft
                    // and surface why the requested page is unavailable.
                    self.leave_guide_preview(window, cx);
                    let message = if matches!(tab, Tab::Instances | Tab::Profiles) {
                        "Finish setup to see your games."
                    } else {
                        "Finish setup to continue."
                    };
                    self.push_toast(message.into(), feedback::ToastKind::Info);
                    self.status = message.into();
                    cx.notify();
                    return;
                }
                if (self.tab == Tab::Onboarding || self.shell.guide_preview.is_some())
                    && matches!(tab, Tab::Settings | Tab::About)
                {
                    if self.shell.guide_preview.is_some()
                        && self.tab == Tab::Settings
                        && tab != Tab::Settings
                    {
                        self.preferences.clone_from(&self.saved_preferences);
                        self.settings_form = None;
                    }
                    if tab == Tab::Settings {
                        self.open_settings(cx);
                    }
                    self.shell.guide_preview = Some((tab, window.viewport_size()));
                    self.tab = tab;
                    self.shell.surfaces.clear(window, &self.focus, cx);
                    window.resize(window_geometry::management_size(window, cx));
                    cx.notify();
                    return;
                }
                if self.tab == Tab::Settings && tab != Tab::Settings {
                    self.preferences = self.saved_preferences.clone();
                    self.settings_form = None;
                }
                if tab == Tab::Settings && self.tab != Tab::Settings {
                    self.open_settings(cx);
                }
                self.initialize_browsers(cx);
                self.tab = tab;
                self.form = None;
                self.shell.instance_editor = None;
                if tab == Tab::Onboarding {
                    self.begin_onboarding(cx);
                }
                self.shell.surfaces.clear(window, &self.focus, cx);
                if tab == Tab::Home {
                    window.resize(window_geometry::home_size(window, cx));
                } else if previous_page.0 == Tab::Home {
                    window.resize(window_geometry::management_size(window, cx));
                }
            }
            UiAction::DownloadFiles(group) => {
                self.shell
                    .surfaces
                    .open(shell::Surface::DownloadFiles(group), window, cx);
            }
            UiAction::Drawer(drawer) => {
                if drawer == Some(Drawer::Menu) {
                    self.open_app_menu_at(None, window, cx);
                } else if let Some(drawer) = drawer {
                    if self
                        .shell
                        .surfaces
                        .open(shell::Surface::Drawer(drawer), window, cx)
                    {
                        self.open_selector(drawer, window, cx);
                    }
                } else if self.shell.guide_preview.is_some() {
                    self.leave_guide_preview(window, cx);
                } else if self
                    .shell
                    .surfaces
                    .top()
                    .is_some_and(|entry| matches!(entry.kind, shell::Surface::AuthProviderAdd))
                {
                    self.cancel_auth_provider(cx);
                    self.shell.surfaces.close(window, &self.focus, cx);
                } else if self
                    .shell
                    .surfaces
                    .top()
                    .is_some_and(|entry| matches!(entry.kind, shell::Surface::WizardSection(_)))
                {
                    self.cancel_component_draft(cx);
                    self.shell.surfaces.close(window, &self.focus, cx);
                } else if self
                    .shell
                    .surfaces
                    .top()
                    .is_some_and(|layer| matches!(layer.kind, shell::Surface::OnboardingSection(_)))
                {
                    self.finish_onboarding_section(true, window, cx);
                } else {
                    self.shell.surfaces.close(window, &self.focus, cx);
                }
            }
            UiAction::SetAppearance(appearance) => self.preview_appearance(appearance),
            UiAction::ReduceMotion => self.preview_reduce_motion(),
            UiAction::ResetPreferences => self.reset_settings(cx),
            UiAction::SavePreferences => self.save_settings(window, cx),
            UiAction::StorageMoveGames => self.move_storage_games(window, cx),
            UiAction::StorageReviewCleanup => {
                if let Some(cleanup) = &self.shell.storage.cleanup {
                    self.shell.surfaces.open(
                        shell::Surface::StorageCleanup(cleanup.id),
                        window,
                        cx,
                    );
                }
            }
            UiAction::StorageCleanup(id, remove) => self.clean_storage(id, remove, window, cx),
            UiAction::SetFullscreen => {
                if let Some(form) = &mut self.settings_form {
                    form.fullscreen = !form.fullscreen;
                }
            }
            UiAction::ShowSnapshots => {
                if let Some(form) = &mut self.settings_form {
                    form.show_snapshots = !form.show_snapshots;
                }
            }
            UiAction::SettingsPage(page) => {
                self.select_settings_page(page, window, cx);
            }
            UiAction::ProxyMode(mode) => {
                if let Some(form) = &mut self.settings_form {
                    form.proxy_mode = mode;
                }
            }
            UiAction::ConfirmSeparate(confirm) => {
                if confirm {
                    self.shell
                        .surfaces
                        .open(shell::Surface::SeparateProfiles, window, cx);
                } else {
                    self.shell.surfaces.close(window, &self.focus, cx);
                }
            }
            UiAction::SeparateProfiles => {
                self.shell.surfaces.close(window, &self.focus, cx);
                self.begin("Enabling separated profiles");
                if let Some(engine) = &self.engine {
                    engine.separate_profiles();
                }
            }
            UiAction::Inspect(id) => {
                self.shell.instance_editor = None;
                self.collection_mut().inspected = Some(id);
                self.collection_mut().detail_visible = true;
                self.form = None;
            }
            UiAction::InspectLinked { profiles, id } => {
                self.shell.instance_editor = None;
                self.tab = if profiles {
                    Tab::Profiles
                } else {
                    Tab::Instances
                };
                self.initialize_browsers(cx);
                self.collection_mut().inspected = Some(id);
                self.collection_mut().detail_visible = true;
                self.form = None;
            }
            UiAction::GamesHelp => {
                self.shell
                    .surfaces
                    .open(shell::Surface::GamesHelp, window, cx);
            }
            UiAction::ProfileModeSettings => {
                self.shell.surfaces.clear(window, &self.focus, cx);
                self.open_settings(cx);
                self.tab = Tab::Settings;
                self.select_settings_page(2, window, cx);
            }
            UiAction::ReloadVersions => {
                if let Some(engine) = &self.engine {
                    self.version_load = browser::CollectionLoad::Loading;
                    engine.load_versions();
                } else {
                    self.version_load = browser::CollectionLoad::Failed(
                        "Engine unavailable · restart Block to retry".into(),
                    );
                }
            }
            UiAction::ReloadLibrary => {
                if let Some(engine) = &self.engine {
                    self.shell.games.load = browser::CollectionLoad::Loading;
                    self.shell.profiles.load = browser::CollectionLoad::Loading;
                    self.shell.accounts.load = browser::CollectionLoad::Loading;
                    engine.load_catalog();
                }
            }
            UiAction::EditInstance => self.open_instance_editor(window, cx),
            UiAction::SaveInstanceSettings => self.save_instance_draft(window, cx),
            UiAction::CancelInstanceSettings => self.cancel_instance_editor(window, cx),
            UiAction::InstanceFullscreen => self.toggle_instance_fullscreen(),
            UiAction::InstanceOptiFineFile => self.browse_instance_optifine(cx),
            UiAction::InstanceOptiFineRemove => self.remove_instance_optifine(cx),
            UiAction::ProfileInheritRuntime => {
                if let Some(Form::Profile(form)) = &mut self.form {
                    form.original.inheritance.runtime = !form.original.inheritance.runtime;
                }
            }
            UiAction::ProfileInheritWindow => {
                if let Some(Form::Profile(form)) = &mut self.form {
                    form.original.inheritance.window = !form.original.inheritance.window;
                }
            }
            UiAction::BrowserBack => {
                self.shell.instance_editor = None;
                self.collection_mut().detail_visible = false;
                self.form = None;
                self.restore_browser_focus(window, cx);
            }
            UiAction::SelectProfile(id) => {
                if let Some(profile) = self
                    .snapshot
                    .profiles
                    .iter()
                    .find(|profile| profile.id == id)
                {
                    self.selected_instance = Some(profile.instance_id.clone());
                    self.selected_profile = Some(id);
                }
                self.push_toast(
                    "Game selected · ready on Home".into(),
                    feedback::ToastKind::Info,
                );
                self.shell.surfaces.clear(window, &self.focus, cx);
                self.form = None;
                self.apply_profile_account();
            }
            UiAction::SelectAccount(id) => {
                if self.tab == Tab::Onboarding {
                    self.choose_onboarding_account(id);
                } else {
                    self.selected_account = Some(id);
                }
                self.push_toast("Account selected".into(), feedback::ToastKind::Info);
                self.shell.surfaces.clear(window, &self.focus, cx);
            }
            UiAction::CreateInstance => self.begin_game_wizard(cx),
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
            UiAction::ProfileConnect => {
                if let Some(Form::Profile(form)) = &mut self.form {
                    form.original.server.connect_on_launch =
                        !form.original.server.connect_on_launch;
                }
            }
            UiAction::WizardRuntime(value) => self.wizard_runtime(value),
            UiAction::WizardWindow { width, height } => self.wizard_window(width, height, cx),
            UiAction::WizardCustomWindow => self.wizard_custom_window(),
            UiAction::WizardProfileConnect => self.wizard_profile_connect(),
            UiAction::MinimizeWindow => window.minimize_window(),
            UiAction::CloseWindow => window.remove_window(),
            UiAction::SelectValue(action) => {
                self.shell.surfaces.close(window, &self.focus, cx);
                self.handle(*action, window, cx);
            }
            UiAction::ProfileIcon(icon) => {
                if let Some(Form::Profile(form)) = &mut self.form {
                    form.original.appearance.icon = icon;
                }
            }
            UiAction::ProfileColor(color) => {
                if let Some(Form::Profile(form)) = &mut self.form {
                    form.original.appearance.color = color;
                }
            }
            UiAction::WizardSnapshots => {
                self.shell.wizard.include_snapshots = !self.shell.wizard.include_snapshots;
            }
            UiAction::WizardNext => {
                self.advance_game_wizard(cx);
                self.focus_field(false, window, cx);
            }
            UiAction::WizardBack => self.back_game_wizard(cx),
            UiAction::WizardPage(page) => {
                if self.tab == Tab::Onboarding {
                    if let Some(page) = page {
                        self.open_onboarding_section(page, window, cx);
                    } else {
                        self.finish_onboarding_section(false, window, cx);
                    }
                } else {
                    if page.is_none()
                        && self
                            .shell
                            .wizard
                            .page
                            .is_some_and(|page| !self.validate_wizard_section(page, cx))
                    {
                        self.focus_wizard_error(window, cx);
                        return;
                    }
                    if let Some(page) = page
                        && self
                            .shell
                            .surfaces
                            .layers()
                            .any(|entry| matches!(entry.kind, shell::Surface::WizardSection(_)))
                    {
                        self.shell
                            .surfaces
                            .open(shell::Surface::WizardSection(page), window, cx);
                        return;
                    }
                    self.open_game_wizard_page(page, cx);
                    if let Some(page) = page {
                        self.shell
                            .surfaces
                            .open(shell::Surface::WizardSection(page), window, cx);
                    } else if self.shell.wizard.page.is_none() {
                        self.shell.surfaces.close(window, &self.focus, cx);
                    }
                }
            }
            UiAction::WizardLoader(kind) => self.choose_wizard_loader(kind, cx),
            UiAction::WizardLoaderBuild(version) => self.choose_wizard_loader_build(version, cx),
            UiAction::ReloadLoaderVersions => self.load_wizard_loader_versions(true, cx),
            UiAction::SelectVersion(version) => self.set_wizard_version(version, cx),
            UiAction::BrowseInstall => self.browse_install(cx),
            UiAction::SaveInstance => {
                self.submit_game_wizard(cx);
                self.focus_field(false, window, cx);
            }
            UiAction::EditProfile => self.edit_profile(window, cx),
            UiAction::SaveProfile => self.save_profile_draft(window, cx),
            UiAction::ProfilePage(page) => self.open_profile_page(page, window, cx),
            UiAction::ProfilePageDone => self.finish_profile_page(false, window, cx),
            UiAction::ProfilePageCancel => self.finish_profile_page(true, window, cx),
            UiAction::CancelForm => {
                if self.shell.editor.page.is_some() && matches!(self.form, Some(Form::Profile(_))) {
                    self.finish_profile_page(true, window, cx);
                } else if matches!(self.form, Some(Form::Instance(_)))
                    && self.shell.wizard.page.is_some()
                {
                    self.back_game_wizard(cx);
                } else if matches!(self.form, Some(Form::Profile(_))) {
                    self.cancel_profile_editor(window, cx);
                } else if self.tab != Tab::Onboarding {
                    self.form = None;
                    self.shell.wizard.cancel_loader_request();
                }
            }
            UiAction::NewProfile => {
                let id = self
                    .inspected_instance()
                    .map(|instance| instance.id.clone());
                self.begin_game_wizard(cx);
                self.wizard_kind(false, cx);
                if let Some(id) = id {
                    self.wizard_instance(id);
                }
            }
            UiAction::ProfileActions => self.open_profile_actions_at(None, window, cx),
            UiAction::CopyProfileId(id) => {
                if let Some(profile) = self
                    .snapshot
                    .profiles
                    .iter()
                    .find(|profile| profile.id == id)
                    .cloned()
                {
                    self.begin("Copying profile settings");
                    if let Some(engine) = &self.engine {
                        engine.copy_profile(profile.id, format!("{} copy", profile.name));
                    }
                }
            }
            UiAction::DeleteProfileId(id) => {
                if self
                    .snapshot
                    .profiles
                    .iter()
                    .any(|profile| profile.id == id)
                {
                    self.shell
                        .surfaces
                        .open(shell::Surface::DeleteProfile(id), window, cx);
                } else {
                    self.report(
                        "This Profile is no longer available. Refresh the game library.".into(),
                    );
                }
            }
            UiAction::ConfirmDelete => {
                let id = self
                    .shell
                    .surfaces
                    .top()
                    .and_then(|layer| match &layer.kind {
                        shell::Surface::DeleteProfile(id) => Some(id.clone()),
                        _ => None,
                    });
                if let Some(id) = id {
                    self.shell.surfaces.close(window, &self.focus, cx);
                    self.begin("Deleting profile metadata");
                    if let Some(engine) = &self.engine {
                        engine.delete_profile(id);
                    }
                }
            }
            UiAction::ProfileTarget(target) => {
                if let Some(Form::Profile(form)) = &mut self.form {
                    form.original.target_id = target;
                }
            }
            UiAction::DefaultAccount(account) => {
                if let Some(Form::Profile(form)) = &mut self.form {
                    form.original.default_account = account;
                }
            }
            UiAction::SignIn => self.open_auth_wizard(window, cx),
            UiAction::Authenticate | UiAction::AuthRetry => self.start_auth(window, cx),
            UiAction::AuthCancel => self.cancel_auth(window, cx),
            UiAction::AuthFinish => self.finish_auth(window, cx),
            UiAction::Providers => self.open_providers(cx),
            UiAction::ProvidersBack => self.close_providers(cx),
            UiAction::ProviderAdd => self.edit_provider(None, cx),
            UiAction::ProviderList => self.show_provider_list(cx),
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
            UiAction::AuthProviderCancel => {
                self.cancel_auth_provider(cx);
                self.shell.surfaces.close(window, &self.focus, cx);
            }
            UiAction::AuthCharacter(id) => self.choose_auth_character(id, cx),
            UiAction::AuthMethod(method) => self.change_auth_method(method, window, cx),
            UiAction::CopyDeviceCode => self.copy_device_code(cx),
            UiAction::AuthReopen => self.reopen_auth(cx),
            UiAction::RequestSignOut(id) => {
                self.shell
                    .surfaces
                    .open(shell::Surface::RemoveAccount(id), window, cx);
            }
            UiAction::SignOut(id) => {
                if let Some(account) = self.accounts.iter().find(|a| account_id(a) == id).cloned() {
                    self.shell.surfaces.close(window, &self.focus, cx);
                    self.begin("Removing saved credentials");
                    self.operation_kind = OperationKind::Account;
                    if let Some(engine) = &self.engine {
                        engine.sign_out(account);
                    }
                }
            }
            UiAction::Play => self.play(cx),
            UiAction::Repair => {
                if let Some(profile) = self.profile().cloned() {
                    self.begin_cancellable("Repairing installation · verifying shared resources");
                    self.operation_kind = OperationKind::Install;
                    if let Some(engine) = &self.engine {
                        engine.install(profile.instance_id, profile.target_id, self.cancel.clone());
                    }
                }
            }
            UiAction::Stop => {
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
                    self.status = "Stopping · waiting for the current operation".into();
                }
            }
            UiAction::ToggleOutput => self.show_output = !self.show_output,
            UiAction::ToggleCommand => self.show_command = !self.show_command,
            UiAction::OpenRecipe => self.open_recipe(cx),
            UiAction::LaunchRecipe => self.launch_recipe(cx),
        }
        if previous_page != (self.tab, self.form.is_some()) {
            if self.form.is_some() && previous_page.0 == Tab::Home {
                window.resize(window_geometry::management_size(window, cx));
            }
            self.shell.backwards = self.tab == Tab::Home || self.form.is_none() && previous_page.1;
            self.shell.revision = self.shell.revision.wrapping_add(1);
        }
        if focus_first && !self.shell.surfaces.is_open() {
            self.focus_field(false, window, cx);
        }
        self.sync_form_inputs(cx);
        cx.notify();
    }

    fn form_inputs(&self) -> Vec<Entity<TextInput>> {
        let mut fields = match &self.form {
            Some(Form::Instance(form)) => vec![
                form.name.clone(),
                form.version.clone(),
                form.directory.clone(),
            ],
            Some(Form::Profile(form)) => vec![
                form.name.clone(),
                form.memory.clone(),
                form.java.clone(),
                form.jvm.clone(),
                form.game.clone(),
                form.server.clone(),
            ],
            None => Vec::new(),
        };
        fields.extend(self.shell.wizard.loader_search.iter().cloned());
        if let Some(profile) = &self.shell.wizard.profile {
            fields.extend([
                profile.name.clone(),
                profile.memory.clone(),
                profile.java.clone(),
                profile.jvm.clone(),
                profile.width.clone(),
                profile.height.clone(),
                profile.server.clone(),
            ]);
        }
        fields
    }

    fn sync_form_inputs(&self, cx: &mut Context<'_, Self>) {
        for field in self.form_inputs() {
            field.update(cx, |field, cx| field.set_read_only(self.busy, cx));
        }
    }

    fn move_focus(&mut self, backwards: bool, window: &mut Window, cx: &mut Context<'_, Self>) {
        base_overlay::advance_focus(backwards, window, cx);
    }

    fn focus_field(&self, backwards: bool, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.tab != Tab::Onboarding
            && matches!(self.form, Some(Form::Instance(_)))
            && self.focus_wizard_error(window, cx)
        {
            return;
        }
        if self.tab == Tab::Onboarding && self.shell.onboarding.step != onboarding::Step::Game {
            if backwards {
                window.focus_prev(cx);
            } else {
                window.focus_next(cx);
            }
            return;
        }
        if self.tab == Tab::Settings {
            if let Some(form) = &self.settings_form {
                let fields = form.fields();
                if let Some(index) = fields
                    .iter()
                    .position(|field| field.read(cx).focus_handle(cx).is_focused(window))
                {
                    let next = if backwards {
                        (index + fields.len() - 1) % fields.len()
                    } else {
                        (index + 1) % fields.len()
                    };
                    fields[next].read(cx).focus_handle(cx).focus(window, cx);
                } else if let Some(first) = fields.first() {
                    first.read(cx).focus_handle(cx).focus(window, cx);
                }
            }
            return;
        }
        let fields = match &self.form {
            Some(Form::Instance(form)) => match self.shell.wizard.page {
                Some(WizardPage::Version) => vec![
                    self.shell
                        .wizard
                        .version_search
                        .as_ref()
                        .unwrap_or(&form.version),
                ],
                Some(WizardPage::Location) => vec![&form.directory],
                None if self.shell.wizard.step == WizardStep::Setup
                    && self.shell.wizard.kind == game_wizard::CreationKind::NewInstance =>
                {
                    vec![&form.name]
                }
                None if self.shell.wizard.step == WizardStep::Profile => self
                    .shell
                    .wizard
                    .profile
                    .as_ref()
                    .map_or_else(Vec::new, |profile| vec![&profile.name]),
                Some(WizardPage::Loader)
                    if form.loader != block_launcher::catalog::LoaderKind::Vanilla =>
                {
                    self.shell
                        .wizard
                        .loader_search
                        .as_ref()
                        .into_iter()
                        .collect()
                }
                Some(WizardPage::Loader | WizardPage::OptiFine) | None => Vec::new(),
            },
            Some(Form::Profile(form)) => {
                let mut fields = vec![&form.name, &form.memory];
                if !form.managed {
                    fields.push(&form.java);
                }
                fields.extend([&form.jvm, &form.game]);
                fields
            }
            _ => {
                if backwards {
                    window.focus_prev(cx);
                } else {
                    window.focus_next(cx);
                }
                return;
            }
        };
        if fields.is_empty() {
            if backwards {
                window.focus_prev(cx);
            } else {
                window.focus_next(cx);
            }
            return;
        }
        let selected = fields
            .iter()
            .position(|field| field.read(cx).focus_handle(cx).is_focused(window));
        let index = selected.map_or(0, |index| {
            if backwards {
                (index + fields.len() - 1) % fields.len()
            } else {
                (index + 1) % fields.len()
            }
        });
        fields[index].read(cx).focus_handle(cx).focus(window, cx);
    }

    fn validate_instance_draft(&mut self, cx: &Context<'_, Self>) -> bool {
        let Some(Form::Instance(form)) = &self.form else {
            return false;
        };
        let directory = form.directory.read(cx).value();
        let name = form.name.read(cx).value();
        let version = form.version.read(cx).value();
        if self.wizard_version_restriction(version.trim()).is_some() {
            self.report(block_launcher::support::legacy_lwjgl_message(
                version.trim(),
            ));
            return false;
        }
        let message = if name.trim().is_empty() {
            Some("Enter a game name")
        } else if name.len() > 160 || name.chars().any(char::is_control) {
            Some("Use a game name of at most 160 UTF-8 bytes without control characters")
        } else if version.trim().is_empty() {
            Some("Choose a Minecraft version")
        } else if version.len() > 80 || version.chars().any(char::is_control) {
            Some("Use a Minecraft version of at most 80 UTF-8 bytes without control characters")
        } else if let Some(issue) = self.wizard_loader_issue(cx) {
            self.report(issue);
            return false;
        } else if !directory.trim().is_empty() && !PathBuf::from(directory.trim()).is_absolute() {
            Some("Choose an absolute installation directory")
        } else {
            None
        };
        message.is_none_or(|message| {
            self.report(message.into());
            false
        })
    }

    fn edit_profile(&mut self, window: &Window, cx: &mut Context<'_, Self>) {
        self.shell.instance_editor = None;
        let Some(profile) = self.inspected_profile().cloned() else {
            return;
        };
        let java = match &profile.runtime {
            RuntimeSelection::Managed => String::new(),
            RuntimeSelection::Local { java } => java.display().to_string(),
        };
        self.shell.editor = self.begin_profile_editor(window, cx);
        self.form = Some(Form::Profile(Box::new(ProfileForm {
            name: input(&profile.name, "Profile name", cx),
            memory: input(&components::memory_gb_text(profile.memory_mib), "2", cx),
            java: input(&java, "Absolute path to Java executable", cx),
            jvm: input(
                &format_arguments(&profile.jvm_args),
                "Optional JVM arguments",
                cx,
            ),
            game: input(
                &format_arguments(&profile.game_args),
                "Optional game arguments",
                cx,
            ),
            server: input(&profile.server.address, "play.example.com", cx),
            managed: matches!(profile.runtime, RuntimeSelection::Managed),
            original: profile,
        })));
    }

    fn play(&mut self, cx: &mut Context<'_, Self>) {
        if self.busy {
            if self.operation_kind != OperationKind::Install {
                return;
            }
            self.pending_play = true;
            self.status =
                "Launch queued · it starts as soon as the current operation finishes".into();
            cx.notify();
            return;
        }
        self.pending_play = false;
        let Some(profile) = self.selected_profile.clone() else {
            self.report("Create or select a profile first".into());
            return;
        };
        let Some(account) = self
            .accounts
            .iter()
            .find(|a| Some(account_id(a)) == self.selected_account)
            .cloned()
        else {
            self.tab = Tab::Accounts;
            self.status = if self
                .profile()
                .is_some_and(|profile| profile.default_account.is_some())
            {
                "This profile's default account is unavailable. Choose an account explicitly or sign in again.".into()
            } else {
                "Add and select an account before playing".into()
            };
            return;
        };
        self.begin_cancellable("Preparing · validating profile and account");
        self.operation_kind = OperationKind::Launch;
        self.play_stop = CancellationToken::new();
        if let Some(engine) = &self.engine {
            engine.play(
                profile,
                account,
                self.cancel.clone(),
                self.play_stop.clone(),
            );
        }
    }

    /// Play may be queued while an install/repair is keeping the app busy.
    fn play_queue_eligible(&self) -> bool {
        self.busy
            && self.operation_kind == OperationKind::Install
            && self.active_run.is_none()
            && self.profile().is_some()
    }

    /// Whether an action may run while another operation is in progress.
    fn can_activate(&self, action: &UiAction) -> bool {
        Self::available_while_busy(action)
            || matches!(action, UiAction::Play) && self.play_queue_eligible()
    }

    /// Start a Play that was queued while another operation was in progress.
    fn start_pending_play(&mut self, cx: &mut Context<'_, Self>) {
        if ready_for_pending_launch(self.pending_play, self.busy, self.active_run.is_some()) {
            self.pending_play = false;
            self.play(cx);
        }
    }

    fn browse_install(&self, cx: &Context<'_, Self>) {
        let Some(Form::Instance(form)) = &self.form else {
            return;
        };
        let field = form.directory.downgrade();
        let selection = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose an installation directory".into()),
        });
        cx.spawn(async move |view, cx| {
            if let Ok(Ok(Some(paths))) = selection.await
                && let Some(path) = paths.first()
            {
                let _ = view.update(cx, |view, cx| {
                    if !view.busy
                        && let Some(Form::Instance(form)) = &view.form
                        && Some(form.directory.entity_id())
                            == field.upgrade().map(|field| field.entity_id())
                    {
                        form.directory.update(cx, |field, cx| {
                            field.set_value(&path.display().to_string(), cx);
                        });
                    }
                });
            }
        })
        .detach();
    }

    fn load_recipe(&mut self, path: PathBuf, launch: bool, cx: &mut Context<'_, Self>) {
        if let Some(engine) = &self.engine
            && self.recipe_load.start(self.busy, launch)
        {
            engine.load_recipe(path);
            self.status = "Reading development recipe".into();
            cx.notify();
        }
    }

    fn open_recipe(&self, cx: &Context<'_, Self>) {
        if self.busy || self.recipe_load.is_loading() {
            return;
        }
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Select a development launch recipe (.json)".into()),
        });
        cx.spawn(async move |view, cx| {
            if let Ok(Ok(Some(paths))) = prompt.await
                && let Some(path) = paths.into_iter().next()
            {
                let _ = view.update(cx, |view, cx| view.load_recipe(path, false, cx));
            }
        })
        .detach();
    }

    fn launch_recipe(&mut self, cx: &mut Context<'_, Self>) {
        if self.busy || self.recipe_load.is_loading() {
            return;
        }
        if let Some(recipe) = self.recipe.clone() {
            self.begin_cancellable("Preparing explicit development recipe");
            self.operation_kind = OperationKind::Launch;
            if let Some(engine) = &self.engine {
                engine.launch(recipe, self.cancel.clone());
            }
            cx.notify();
        }
    }

    fn button(
        &self,
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        action: UiAction,
        cx: &Context<'_, Self>,
    ) -> components::Control {
        let variant = match &action {
            UiAction::SaveInstance
            | UiAction::SaveProfile
            | UiAction::SavePreferences
            | UiAction::Play
            | UiAction::WizardNext
            | UiAction::OnboardingNext
            | UiAction::SignIn
            | UiAction::Drawer(Some(Drawer::Profiles)) => components::ButtonVariant::Primary,
            UiAction::SignOut(_) | UiAction::ConfirmDelete | UiAction::StorageCleanup(_, true) => {
                components::ButtonVariant::Destructive
            }
            UiAction::Tab(_)
            | UiAction::Drawer(_)
            | UiAction::CancelForm
            | UiAction::MinimizeWindow
            | UiAction::CloseWindow => components::ButtonVariant::Ghost,
            _ => components::ButtonVariant::Outline,
        };
        self.button_with_variant(id, label, action, variant, cx)
    }

    fn button_with_variant(
        &self,
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        action: UiAction,
        variant: components::ButtonVariant,
        cx: &Context<'_, Self>,
    ) -> components::Control {
        let disabled = self.busy && !self.can_activate(&action)
            || matches!(action, UiAction::SignIn) && self.busy && self.shell.auth.is_none()
            || matches!(action, UiAction::Stop) && !self.can_stop();
        let anchor = std::rc::Rc::new(std::cell::Cell::new(None));
        let captured_anchor = anchor.clone();
        let app_menu = matches!(action, UiAction::Drawer(Some(Drawer::Menu)));
        let profile_actions = matches!(action, UiAction::ProfileActions);
        let product_choice = match &action {
            UiAction::OpenProductSelect(kind) => Some(*kind),
            _ => None,
        };
        let choice = match action {
            UiAction::ChooseAppearance { wizard, color } => Some((wizard, color)),
            _ => None,
        };
        let id = id.into();
        let visible_label = label.into();
        let on_activate = cx.listener(move |view, (): &(), window, cx| {
            if view.busy && !Self::available_while_busy(&action) {
                return;
            }
            if let Some((wizard, color)) = choice {
                view.open_appearance_select_at(wizard, color, captured_anchor.get(), window, cx);
                view.motion_dirty = true;
                cx.notify();
            } else if let Some(kind) = product_choice {
                view.open_product_select(kind, captured_anchor.get(), window, cx);
                view.motion_dirty = true;
                cx.notify();
            } else if app_menu {
                view.open_app_menu_at(captured_anchor.get(), window, cx);
                view.motion_dirty = true;
                cx.notify();
            } else if profile_actions {
                view.open_profile_actions_at(captured_anchor.get(), window, cx);
                view.motion_dirty = true;
                cx.notify();
            } else {
                view.handle(action.clone(), window, cx);
            }
        });
        let state = components::ControlState {
            disabled,
            ..Default::default()
        };
        let control = if choice.is_some() || product_choice.is_some() {
            components::select_trigger(id, visible_label, self.palette, state, on_activate)
        } else {
            components::button_role(
                id.clone(),
                visible_label,
                self.palette,
                variant,
                state,
                if id.starts_with("menu-") {
                    gpui::accesskit::Role::MenuItem
                } else {
                    gpui::accesskit::Role::Button
                },
                on_activate,
            )
        };
        if choice.is_some() || profile_actions || product_choice.is_some() {
            control.child(
                gpui::canvas(
                    move |bounds, _, _| anchor.set(Some(bounds)),
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
        } else {
            control
        }
    }

    fn available_while_busy(action: &UiAction) -> bool {
        if let UiAction::SelectValue(action) = action {
            return Self::available_while_busy(action);
        }
        if let UiAction::OpenProductSelect(choice) = action {
            return matches!(
                choice,
                select::ProductChoice::Appearance
                    | select::ProductChoice::Proxy
                    | select::ProductChoice::DefaultJava
                    | select::ProductChoice::DefaultWindow
            );
        }
        matches!(
            action,
            UiAction::Stop
                | UiAction::OnboardingSkip
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
                | UiAction::BrowserBack
                | UiAction::ToggleOutput
                | UiAction::ToggleCommand
        )
    }

    fn instance_form(
        &self,
        form: &InstanceForm,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement + use<> {
        self.render_game_wizard(form, window, cx)
    }

    fn output(&self, cx: &Context<'_, Self>) -> impl IntoElement + use<> {
        let lines = if self.show_command {
            self.command.as_deref().unwrap_or("A redacted command preview is available for development recipes after preparation.").lines().map(str::to_owned).collect::<Vec<_>>()
        } else {
            self.logs.iter().cloned().collect()
        };
        panel()
            .gap_2()
            .child(self.button(
                "toggle-command",
                if self.show_command {
                    "Show game output"
                } else {
                    "Show redacted command"
                },
                UiAction::ToggleCommand,
                cx,
            ))
            .child(
                div()
                    .id("game-output")
                    .h(px(210.))
                    .scroll_area(&self.shell.output_scroll, "Game output", self.palette, true)
                    .bg(rgb(self.palette.surface))
                    .p_3()
                    .text_xs()
                    .children(
                        lines
                            .into_iter()
                            .map(|line| div().mb_1().whitespace_nowrap().child(line)),
                    ),
            )
    }
}

/// Keep the component library's theme mode in step with the product palette so
/// overlay chrome, scrims and inputs render in the same appearance.
fn sync_component_theme(palette: Palette, cx: &mut gpui::App) {
    let dark = palette.background == Palette::DARK.background;
    let mode = if dark {
        gpui_component::ThemeMode::Dark
    } else {
        gpui_component::ThemeMode::Light
    };
    if gpui_component::Theme::global(cx).mode != mode {
        gpui_component::Theme::change(mode, None, cx);
    }
    let theme = gpui_component::Theme::global_mut(cx);
    theme.font_family = design::FONT_FAMILY.into();
    // The library's default scrim is too faint for the prototype's modal
    // treatment; the product palette decides the intended dimming.
    theme.colors.overlay = if dark {
        gpui::hsla(0., 0., 0., 0.6)
    } else {
        gpui::hsla(0., 0., 0., 0.3)
    };
}

impl Render for Block {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        self.motion
            .apply_policy(self.motion_preferences().reduced(), cx);
        self.prepare_crash_help(window, cx);
        self.sync_provider_drop(window, cx);
        if let Some(help) = &self.crash_help {
            return div().size_full().child(help.clone()).into_any_element();
        }
        if self.pending_window_size.take().is_some() {
            window.resize(if self.tab == Tab::Home {
                window_geometry::home_size(window, cx)
            } else {
                window_geometry::management_size(window, cx)
            });
        }
        self.palette = self.preferences.palette(window.appearance());
        cx.set_global(self.palette);
        sync_component_theme(self.palette, cx);
        let action_rail = if self.tab == Tab::Home && self.form.is_none() {
            104.
        } else if patterns::is_narrow(window.viewport_size()) {
            design::NARROW_FOOTER
        } else {
            design::MANAGEMENT_FOOTER
        };
        feedback::ToastLayer::reserve_actions(px(action_rail + 12.), window, cx);
        feedback::ToastLayer::enqueue(std::mem::take(&mut self.shell.pending_toasts), window, cx);
        self.prepare_motion(window, cx);
        let title = self.navigation_title();
        window.set_window_title(&if self.tab == Tab::Home && self.shell.auth.is_none() {
            "Block".to_owned()
        } else {
            format!("Block · {title}")
        });
        self.render_shell(window, cx).into_any_element()
    }
}

impl Block {
    /// Attach or release the owned native drop target as the provider draft
    /// appears or disappears. Platforms without a drop backend keep this a no-op.
    #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
    fn sync_provider_drop(&mut self, window: &Window, cx: &mut Context<'_, Self>) {
        let drafting = self
            .shell
            .auth
            .as_ref()
            .is_some_and(|state| state.is_provider_drafting());
        if !drafting {
            self.provider_drop = None;
            return;
        }
        if self.provider_drop.is_some() {
            return;
        }
        let Ok(handle) = raw_window_handle::HasWindowHandle::window_handle(window) else {
            return;
        };
        let parent = match handle.as_raw() {
            #[cfg(windows)]
            RawWindowHandle::Win32(win32) => Some(win32.hwnd.get()),
            #[cfg(target_os = "linux")]
            RawWindowHandle::Xcb(xcb) => isize::try_from(xcb.window.get()).ok(),
            #[cfg(target_os = "linux")]
            RawWindowHandle::Xlib(xlib) => isize::try_from(xlib.window).ok(),
            #[cfg(target_os = "linux")]
            RawWindowHandle::Wayland(_) => None,
            #[cfg(target_os = "macos")]
            RawWindowHandle::AppKit(appkit) => Some(appkit.ns_view.as_ptr() as isize),
            _ => None,
        };
        let Some(parent) = parent else {
            if let Some(draft) = self
                .shell
                .auth
                .as_mut()
                .and_then(|state| state.provider_draft_mut())
                && draft.error.is_none()
            {
                draft.error = Some(if cfg!(target_os = "linux") {
                    "Drag and drop is unavailable: Wayland windows are not supported by this build; use an X11 session.".to_owned()
                } else {
                    "Drag and drop is unavailable: unsupported windowing system.".to_owned()
                });
            }
            return;
        };
        let (sender, receiver) = async_channel::bounded::<String>(4);
        let zone = block_drop::DropZone::new(0.0, 0.0, 0.0, 0.0, window.scale_factor());
        match block_drop::DropTarget::attach(parent, zone, sender) {
            Ok(target) => {
                self.provider_drop = Some(ProviderDrop { target });
                cx.spawn(async move |view, cx| {
                    while let Ok(payload) = receiver.recv().await {
                        let _ = view.update(cx, |view, cx| {
                            view.apply_provider_drop(payload, cx);
                        });
                    }
                })
                .detach();
            }
            Err(error) => {
                if let Some(draft) = self
                    .shell
                    .auth
                    .as_mut()
                    .and_then(|state| state.provider_draft_mut())
                    && draft.error.is_none()
                {
                    draft.error = Some(format!("Drag and drop is unavailable: {error}"));
                }
            }
        }
    }

    /// Keep the owned native drop surface aligned with the rendered zone.
    #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
    fn position_provider_drop(&mut self, bounds: &[Bounds<Pixels>], scale: f32) {
        let Some(first) = bounds.first() else {
            return;
        };
        let mut left = first.left();
        let mut top = first.top();
        let mut right = first.right();
        let mut bottom = first.bottom();
        for bounds in &bounds[1..] {
            left = left.min(bounds.left());
            top = top.min(bounds.top());
            right = right.max(bounds.right());
            bottom = bottom.max(bounds.bottom());
        }
        if let Some(drop) = &self.provider_drop {
            drop.target.set_zone(block_drop::DropZone::new(
                f32::from(left),
                f32::from(top),
                f32::from(right - left),
                f32::from(bottom - top),
                scale,
            ));
        }
    }

    /// Feed an accepted quick link through the same recognise/normalise path as
    /// pasting it and pressing "Recognize address".
    #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
    fn apply_provider_drop(&mut self, payload: String, cx: &mut Context<'_, Self>) {
        if let Some(draft) = self
            .shell
            .auth
            .as_mut()
            .and_then(|state| state.provider_draft_mut())
        {
            draft
                .address
                .update(cx, |field, cx| field.set_value(&payload, cx));
            draft.error = None;
        }
        self.identify_auth_provider(cx);
        cx.notify();
    }

    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    fn sync_provider_drop(&mut self, _window: &Window, _cx: &mut Context<'_, Self>) {}

    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    #[expect(
        dead_code,
        reason = "the drop zone only drives real bounds and payloads where a native target exists"
    )]
    fn position_provider_drop(&mut self, _bounds: &[Bounds<Pixels>], _scale: f32) {}

    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    #[expect(
        dead_code,
        reason = "quick-link drops need an owned native target; the stub platforms have none"
    )]
    fn apply_provider_drop(&mut self, _payload: String, _cx: &mut Context<'_, Self>) {}
}

/// Subscribe the rendered drop zone to its real window bounds (Windows, macOS
/// and X11).
#[cfg(any(windows, target_os = "linux", target_os = "macos"))]
fn attach_provider_drop_bounds(zone: gpui::Div, view: WeakEntity<Block>) -> gpui::Div {
    zone.on_children_prepainted(move |bounds, window, cx| {
        if let Some(view) = view.upgrade() {
            let scale = window.scale_factor();
            view.update(cx, |view, _| {
                view.position_provider_drop(&bounds, scale);
            });
        }
    })
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn attach_provider_drop_bounds(zone: gpui::Div, _view: WeakEntity<Block>) -> gpui::Div {
    zone
}

/// Whether a queued Play may start now that the previous operation ended.
const fn ready_for_pending_launch(pending: bool, busy: bool, has_active_run: bool) -> bool {
    pending && !busy && !has_active_run
}

#[cfg(test)]
mod queued_play_ui_tests {
    use super::{OperationKind, Tab, UiAction};
    use gpui::TestAppContext;

    #[gpui::test]
    fn play_queues_only_while_installing_and_drains_after_the_install(cx: &mut TestAppContext) {
        let result = (|| -> anyhow::Result<()> {
            let root = tempfile::tempdir()?;
            let catalog = block_launcher::catalog::Catalog::open(root.path())?;
            let (_, profile) =
                catalog.create_instance("Queued".into(), "1.20.6".into(), None, None)?;
            let snapshot = catalog.snapshot()?;
            let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
            view.update(cx, |view, _| {
                view.snapshot = snapshot;
                view.selected_profile = Some(profile.id.clone());
                view.busy = true;
                view.operation_kind = OperationKind::Launch;
            });
            // A launch already running does not queue another one.
            cx.update(|window, cx| {
                view.update(cx, |view, cx| view.handle(UiAction::Play, window, cx));
            });
            assert!(!view.read_with(cx, |view, _| view.pending_play));
            assert!(!view.read_with(cx, |view, _| view.status.contains("Launch queued")));

            // An install may be asked to launch as soon as it finishes.
            view.update(cx, |view, _| view.operation_kind = OperationKind::Install);
            cx.update(|window, cx| {
                view.update(cx, |view, cx| view.handle(UiAction::Play, window, cx));
            });
            assert!(view.read_with(cx, |view, _| view.pending_play));
            assert!(view.read_with(cx, |view, _| view.status.contains("Launch queued")));
            assert!(view.read_with(cx, |view, _| view.busy));

            // Stop clears a queued launch.
            cx.update(|window, cx| {
                view.update(cx, |view, cx| view.handle(UiAction::Stop, window, cx));
            });
            assert!(!view.read_with(cx, |view, _| view.pending_play));

            // Pressing Play again after Stop still queues while installing.
            cx.update(|window, cx| {
                view.update(cx, |view, cx| view.handle(UiAction::Play, window, cx));
            });
            assert!(view.read_with(cx, |view, _| view.pending_play));

            // Completion drains the queue through the normal Play path (no
            // account in this fixture, so it lands on the Accounts tab).
            view.update(cx, |view, cx| {
                view.busy = false;
                view.operation_kind = OperationKind::General;
                view.start_pending_play(cx);
            });
            assert!(!view.read_with(cx, |view, _| view.pending_play));
            assert_eq!(view.read_with(cx, |view, _| view.tab), Tab::Accounts);
            Ok(())
        })();
        assert!(result.is_ok(), "{result:?}");
    }
}

#[cfg(test)]
mod pending_launch_tests {
    use super::ready_for_pending_launch;

    #[test]
    fn pending_launch_waits_for_the_running_operation() {
        assert!(!ready_for_pending_launch(true, true, false));
        assert!(!ready_for_pending_launch(true, false, true));
        assert!(!ready_for_pending_launch(false, false, false));
        assert!(ready_for_pending_launch(true, false, false));
    }
}

fn panel() -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .p_4()
        .rounded_md()
        .border_1()
        .border_color(rgb(0x0073_7373))
        .bg(gpui::transparent_black())
}
#[track_caller]
fn note(text: impl Into<SharedString>) -> gpui::Stateful<gpui::Div> {
    use std::hash::{Hash as _, Hasher as _};
    let text = text.into();
    let mut identity = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut identity);
    // Equal descriptions in different parts of a review are distinct semantic
    // nodes. Keep a stable call-site identity rather than merging by text.
    let caller = std::panic::Location::caller();
    caller.file().hash(&mut identity);
    caller.line().hash(&mut identity);
    caller.column().hash(&mut identity);

    div()
        .id(("description", identity.finish()))
        .text_xs()
        .opacity(0.65)
        .child(text)
}

fn input(value: &str, placeholder: &str, cx: &mut Context<'_, Block>) -> Entity<TextInput> {
    let field = cx.new(|cx| TextInput::new(value, placeholder, cx));
    cx.observe(&field, |view, _, cx| {
        view.motion_dirty = true;
        cx.notify();
    })
    .detach();
    field
}

fn frozen_input(input: &Entity<TextInput>, cx: &mut Context<'_, Block>) -> Entity<TextInput> {
    cx.new(|cx| input.read(cx).frozen(cx))
}
fn account_id(account: &Account) -> String {
    account.identity_key()
}
/// Parse explicit quoted arguments without invoking a shell or interpreting backslashes in paths.
fn parse_arguments(value: &str) -> anyhow::Result<Vec<String>> {
    let mut result = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut started = false;
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\'
            && quoted
            && chars
                .peek()
                .is_some_and(|next| *next == '"' || *next == '\\')
        {
            if let Some(next) = chars.next() {
                current.push(next);
            }
            started = true;
        } else if ch == '"' {
            quoted = !quoted;
            started = true;
        } else if ch.is_whitespace() && !quoted {
            if started {
                result.push(std::mem::take(&mut current));
                started = false;
            }
        } else {
            current.push(ch);
            started = true;
        }
    }
    anyhow::ensure!(
        !quoted,
        "Close the double quote in the additional arguments"
    );
    if started {
        result.push(current);
    }
    Ok(result)
}

fn format_arguments(arguments: &[String]) -> String {
    arguments
        .iter()
        .map(|argument| {
            if argument.is_empty() || argument.chars().any(|ch| ch.is_whitespace() || ch == '"') {
                format!(
                    "\"{}\"",
                    argument.replace('\\', "\\\\").replace('"', "\\\"")
                )
            } else {
                argument.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::{RecipeLoadState, parse_arguments};
    #[test]
    fn pending_recipe_blocks_another_import_and_autolaunches_once_after_success() {
        let mut state = RecipeLoadState::default();
        assert!(state.start(false, true));
        assert!(!state.start(false, false));
        assert!(state.finish(true));
        assert!(!state.finish(true));
    }
    #[test]
    fn failed_startup_recipe_does_not_autolaunch_a_later_manual_import() {
        let mut state = RecipeLoadState::default();
        assert!(state.start(false, true));
        assert!(!state.finish(false));
        assert!(state.start(false, false));
        assert!(!state.finish(true));
    }
    #[test]
    fn file_picker_result_cannot_start_recipe_loading_during_a_run() {
        let mut state = RecipeLoadState::default();
        assert!(!state.start(true, false));
        assert!(!state.is_loading());
    }
    #[test]
    fn arguments_keep_quoted_values_and_windows_paths_without_running_a_shell() {
        assert_eq!(
            parse_arguments(r#"-Dname="My world" -Dpath=C:\Games """#).ok(),
            Some(vec![
                "-Dname=My world".into(),
                r"-Dpath=C:\Games".into(),
                String::new()
            ])
        );
        assert!(parse_arguments("\"unclosed").is_err());
    }
    #[expect(
        clippy::redundant_pub_crate,
        reason = "Share the network-free view fixture with sibling UI tests only."
    )]
    pub(super) fn test_view(
        window: &mut gpui::Window,
        cx: &mut gpui::Context<'_, super::Block>,
    ) -> super::Block {
        super::initialize(cx);
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        super::text_input::bind_keys(cx);
        super::bind_navigation_keys(cx);
        super::Block {
            build_info: super::BuildInfo::default(),
            motion: super::transitions::MotionState::default(),
            motion_dirty: true,
            visual_only: false,
            shell: super::ShellState::default(),
            pending_window_size: None,
            onboarding_progress: super::onboarding::Progress::Draft,
            settings_form: None,
            preferences: super::Preferences::default(),
            saved_preferences: super::Preferences::default(),
            palette: super::Palette::LIGHT,
            snapshot: block_launcher::catalog::Snapshot::default(),
            accounts: Vec::new(),
            account_avatars: std::collections::HashMap::new(),
            provider_icons: std::collections::HashMap::new(),
            versions: vec!["1.20.6".into()],
            host_arch_override: None,
            version_load: super::browser::CollectionLoad::Ready,
            version_catalog: Vec::new(),
            selected_instance: None,
            selected_profile: None,
            pending_profile: None,
            selected_account: None,
            tab: super::Tab::Instances,
            form: None,
            engine: None,
            messages: None,
            focus,
            status: "Welcome".into(),
            completed: 0,
            operation_kind: super::OperationKind::General,
            operation_revision: 0,
            pending_creation: None,
            downloads: None,
            downloads_revision: 0,
            busy: false,
            operation_cancellable: false,
            cancel: block_launcher::CancellationToken::new(),
            play_stop: block_launcher::CancellationToken::new(),
            active_run: None,
            active_started_at: 0,
            session_revisions: std::collections::HashMap::new(),
            pending_crashes: std::collections::VecDeque::new(),
            crash_help: None,
            crash_guard: None,
            logs: std::collections::VecDeque::new(),
            show_output: false,
            command: None,
            show_command: false,
            recipe: None,
            recipe_load: RecipeLoadState::default(),
            #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
            provider_drop: None,
            pending_play: false,
        }
    }

    fn click_selector(cx: &mut gpui::VisualTestContext, selector: &'static str) {
        cx.run_until_parked();
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(500));
        // TestWindow has no display-link callback. Paint the settled transition
        // so the hitboxes belong to the current view rather than its first frame.
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        let mut bounds = None;
        // A dialog can mount on the frame after the caller's click, and its
        // entrance animation shifts the target while the hitbox is located.
        // Locate, settle, re-locate until the painted bounds stop moving.
        for _ in 0..3 {
            let current = crate::test_support::control_bounds(cx, selector);
            assert!(current.is_some(), "missing selector: {selector}");
            if bounds == current {
                break;
            }
            bounds = current;
            cx.executor()
                .advance_clock(std::time::Duration::from_millis(500));
            cx.update(|window, _| window.refresh());
            cx.run_until_parked();
        }
        let Some(bounds) = bounds else {
            return;
        };
        {
            let window_count = cx.windows().len();
            cx.simulate_click(bounds.center(), gpui::Modifiers::none());
            cx.run_until_parked();
            if cx.windows().len() > window_count
                && let Some(window) = cx.windows().last()
            {
                *cx = gpui::VisualTestContext::from_window(*window, &cx.cx);
            }
            crate::test_support::ensure_live_window(cx);
        }
    }

    #[gpui::test]
    fn first_run_onboarding_saves_only_on_completion_and_default_reset_keeps_it(
        cx: &mut gpui::TestAppContext,
    ) {
        let result = (|| -> anyhow::Result<()> {
            let root = tempfile::tempdir()?;
            let (engine, _messages) = super::Engine::with_root(root.path().into())?;
            let (view, cx) = crate::test_support::window(cx, test_view);
            cx.simulate_resize(gpui::size(gpui::px(960.), gpui::px(600.)));
            view.update(cx, |view, cx| {
                view.engine = Some(engine);
                view.begin_onboarding(cx);
                cx.notify();
            });
            cx.run_until_parked();
            if let Some(bounds) = cx.debug_bounds("welcome-next") {
                assert!(
                    bounds.right() > gpui::px(860.),
                    "primary action must align with the right form edge: {bounds:?}"
                );
                assert!(bounds.bottom() <= gpui::px(600.));
            }
            click_selector(cx, "welcome-next");
            assert_eq!(
                view.read_with(cx, |view, _| view.shell.onboarding.step),
                super::onboarding::Step::Account
            );
            assert!(!root.path().join("preferences.json").exists());
            assert!(view.read_with(cx, |view, _| view.form.is_some()));
            assert!(cx.debug_bounds("onboarding-next-disabled").is_some());
            click_selector(cx, "onboarding-account-later");
            assert_eq!(
                view.read_with(cx, |view, _| view.shell.onboarding.step),
                super::onboarding::Step::Review
            );
            cx.simulate_resize(gpui::size(gpui::px(360.), gpui::px(540.)));
            cx.run_until_parked();
            let action = cx
                .debug_bounds("welcome-next")
                .ok_or_else(|| anyhow::anyhow!("missing compact primary action"))?;
            let footer = cx
                .debug_bounds("onboarding-footer")
                .ok_or_else(|| anyhow::anyhow!("missing compact footer"))?;
            assert!(
                action.right() <= footer.right() && action.bottom() <= footer.bottom(),
                "compact action must stay inside its animated footer: {action:?}, {footer:?}"
            );
            cx.simulate_resize(gpui::size(gpui::px(960.), gpui::px(600.)));
            click_selector(cx, "welcome-back");
            click_selector(cx, "welcome-back");
            assert_eq!(
                view.read_with(cx, |view, _| view.shell.onboarding.step),
                super::onboarding::Step::Game
            );
            assert!(view.read_with(cx, |view, _| view.shell.wizard.backwards));
            click_selector(cx, "welcome-next");
            assert!(!view.read_with(cx, |view, _| view.shell.wizard.backwards));
            assert_eq!(
                view.read_with(cx, |view, _| view.tab),
                super::Tab::Onboarding
            );
            assert!(!root.path().join("preferences.json").exists());
            click_selector(cx, "onboarding-skip");
            assert_eq!(view.read_with(cx, |view, _| view.tab), super::Tab::Home);
            let saved = super::Preferences::load(root.path())?;
            assert!(saved.onboarding_complete);
            assert_eq!(saved.appearance, super::Appearance::System);
            cx.update(|window, cx| {
                view.update(cx, |view, cx| {
                    view.handle(super::UiAction::Tab(super::Tab::Settings), window, cx);
                    view.handle(super::UiAction::ResetPreferences, window, cx);
                });
            });
            assert!(view.read_with(cx, |view, _| view.preferences.onboarding_complete));
            assert!(
                block_launcher::catalog::Catalog::open(root.path())?
                    .snapshot()?
                    .instances
                    .is_empty()
            );
            Ok(())
        })();
        assert!(result.is_ok(), "{result:?}");
    }

    #[gpui::test]
    fn first_run_cannot_exit_while_catalog_creation_is_pending(cx: &mut gpui::TestAppContext) {
        let result = (|| -> anyhow::Result<()> {
            let root = tempfile::tempdir()?;
            let (engine, _messages) = super::Engine::with_root(root.path().into())?;
            let (view, cx) = crate::test_support::window(cx, test_view);
            view.update(cx, |view, cx| {
                view.engine = Some(engine);
                view.begin_onboarding(cx);
                view.shell.onboarding.step = super::onboarding::Step::Review;
                view.busy = true;
                cx.notify();
            });
            cx.run_until_parked();
            assert!(cx.debug_bounds("onboarding-skip").is_none());
            cx.update(|window, cx| {
                view.update(cx, |view, cx| {
                    view.handle(super::UiAction::OnboardingSkip, window, cx);
                });
            });
            assert!(
                view.read_with(cx, |view, _| view.tab == super::Tab::Onboarding
                    && view.form.is_some()
                    && view.busy
                    && !view.cancel.is_cancelled())
            );
            assert!(!root.path().join("preferences.json").exists());
            Ok(())
        })();
        assert!(result.is_ok(), "{result:?}");
    }

    #[gpui::test]
    fn first_run_save_retry_during_download_does_not_create_a_second_game(
        cx: &mut gpui::TestAppContext,
    ) {
        let result = (|| -> anyhow::Result<()> {
            let root = tempfile::tempdir()?;
            let catalog = block_launcher::catalog::Catalog::open(root.path())?;
            let (instance, profile) =
                catalog.create_instance("First game".into(), "1.20.6".into(), None, None)?;
            let (engine, _messages) = super::Engine::with_root(root.path().into())?;
            let (view, cx) = crate::test_support::window(cx, test_view);
            cx.simulate_resize(gpui::size(gpui::px(960.), gpui::px(600.)));
            std::fs::create_dir(root.path().join("preferences.json"))?;
            view.update(cx, |view, cx| {
                view.engine = Some(engine);
                view.begin_onboarding(cx);
                view.selected_instance = Some(instance.id.clone());
                view.selected_profile = Some(profile.id.clone());
                view.onboarding_progress = super::onboarding::Progress::Created;
                view.shell.onboarding.step = super::onboarding::Step::Review;
                view.busy = true;
                assert!(!view.complete_onboarding());
                cx.notify();
            });
            cx.run_until_parked();
            assert!(cx.debug_bounds("onboarding-error").is_some());
            click_selector(cx, "welcome-back");
            assert_eq!(
                view.read_with(cx, |view, _| view.shell.onboarding.step),
                super::onboarding::Step::Review
            );
            assert_eq!(
                view.read_with(cx, |view, _| view.onboarding_progress),
                super::onboarding::Progress::Created
            );
            std::fs::remove_dir(root.path().join("preferences.json"))?;
            click_selector(cx, "welcome-next");
            assert_eq!(view.read_with(cx, |view, _| view.tab), super::Tab::Home);
            assert!(view.read_with(cx, |view, _| view.busy));
            let snapshot = catalog.snapshot()?;
            assert_eq!(snapshot.instances.len(), 1);
            assert_eq!(snapshot.instances[0].id, instance.id);
            assert_eq!(
                view.read_with(cx, |view, _| view.selected_profile.clone()),
                Some(profile.id)
            );
            assert!(super::Preferences::load(root.path())?.onboarding_complete);
            Ok(())
        })();
        assert!(result.is_ok(), "{result:?}");
    }

    #[gpui::test]
    fn native_home_menu_settings_preview_cancel_and_escape(cx: &mut gpui::TestAppContext) {
        let (view, cx) = crate::test_support::window(cx, test_view);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                let fixture = super::test_support::test_account();
                assert!(fixture.is_ok(), "{fixture:?}");
                let Ok(account) = fixture else {
                    return;
                };
                view.selected_account = Some(super::account_id(&account));
                view.accounts = vec![account];
                view.handle(super::UiAction::Tab(super::Tab::Home), window, cx);
            });
        });
        cx.run_until_parked();
        cx.simulate_resize(gpui::size(gpui::px(432.), gpui::px(768.)));
        click_selector(cx, "app-menu");
        assert_eq!(
            view.read_with(cx, |view, _| view.shell.drawer()),
            Some(super::Drawer::Menu)
        );
        click_selector(cx, "menu-settings");
        assert_eq!(view.read_with(cx, |view, _| view.tab), super::Tab::Settings);
        cx.simulate_resize(gpui::size(gpui::px(960.), gpui::px(600.)));
        click_selector(cx, "settings-appearance");
        click_selector(cx, "select-choice-2");
        assert_eq!(
            view.read_with(cx, |view, _| view.preferences.appearance),
            super::Appearance::Dark
        );
        click_selector(cx, "reduce-motion");
        assert!(view.read_with(cx, |view, _| view.preferences.reduce_motion));
        click_selector(cx, "back-home");
        assert_eq!(
            view.read_with(cx, |view, _| view.preferences.clone()),
            super::Preferences::default()
        );
        click_selector(cx, "home-profiles");
        assert_eq!(
            view.read_with(cx, |view, _| view.shell.drawer()),
            Some(super::Drawer::Profiles)
        );
        crate::test_support::press(cx, "escape");
        assert!(view.read_with(cx, |view, _| view.shell.drawer().is_none()));
        assert!(!view.read_with(cx, |view, _| view.busy));
    }

    #[gpui::test]
    fn native_menu_is_keyboard_operable_without_replaying_page_animation(
        cx: &mut gpui::TestAppContext,
    ) {
        let (view, cx) = crate::test_support::window(cx, test_view);
        let revision = view.read_with(cx, |view, _| view.shell.revision);
        crate::test_support::press(cx, "tab enter");
        assert_eq!(
            view.read_with(cx, |view, _| view.shell.drawer()),
            Some(super::Drawer::Menu)
        );
        assert_eq!(view.read_with(cx, |view, _| view.shell.revision), revision);
        crate::test_support::press(cx, "escape");
        assert!(view.read_with(cx, |view, _| view.shell.drawer().is_none()));
    }

    fn supervised_record() -> block_launcher::session_host::RunRecord {
        block_launcher::session_host::RunRecord {
            schema_version: 1,
            context: block_launcher::crash::CrashContext {
                run_id: "synthetic-run".into(),
                profile_id: "synthetic-profile".into(),
                profile_name: "Synthetic profile".into(),
                minecraft_version: "1.20.6".into(),
                loader: None,
            },
            instance_id: "synthetic-instance".into(),
            game_directory: std::env::temp_dir().join("synthetic-game"),
            supervisor_pid: Some(123),
            java_pid: Some(456),
            state: block_launcher::session_host::RunState::Running,
            outcome: None,
            status: "Running · independent game session".into(),
            revision: 1,
            recent_output: vec!["Game is running".into()],
            capture_warnings: Vec::new(),
            started_at_unix_ms: 1,
            finished_at_unix_ms: None,
        }
    }

    #[gpui::test]
    fn reopened_ui_restores_session_and_ignores_an_old_handoff_after_exit(
        cx: &mut gpui::TestAppContext,
    ) {
        let (view, cx) = crate::test_support::window(cx, test_view);
        let record = supervised_record();
        view.update(cx, |view, cx| {
            view.session_updated(record.clone());
            cx.notify();
        });
        assert!(view.read_with(cx, |view, _| view.busy));
        assert_eq!(
            view.read_with(cx, |view, _| view.active_run.clone()),
            Some("synthetic-run".into())
        );
        let mut finished = record.clone();
        finished.state = block_launcher::session_host::RunState::Finished;
        finished.revision = 3;
        finished.status = "Exited normally".into();
        view.update(cx, |view, cx| {
            view.session_updated(finished);
            // The observer can see the exit before the initial spawn RPC returns.
            view.session_updated(record);
            cx.notify();
        });
        assert!(!view.read_with(cx, |view, _| view.busy));
        assert!(view.read_with(cx, |view, _| view.active_run.is_none()));
        assert_eq!(
            view.read_with(cx, |view, _| view.status.clone()),
            "Exited normally"
        );
    }

    #[gpui::test]
    fn monitoring_interruption_preserves_the_session_and_explicit_stop_is_separate(
        cx: &mut gpui::TestAppContext,
    ) {
        let (view, cx) = crate::test_support::window(cx, test_view);
        let mut record = supervised_record();
        view.update(cx, |view, _| view.session_updated(record.clone()));
        record.state = block_launcher::session_host::RunState::MonitoringInterrupted;
        record.status = "Monitoring interrupted · Minecraft may still be running".into();
        view.update(cx, |view, cx| {
            view.session_updated(record);
            cx.notify();
        });
        assert!(view.read_with(cx, |view, _| view.busy));
        assert!(view.read_with(cx, |view, _| view.active_run.is_some()));
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.handle(super::UiAction::Stop, window, cx);
            });
        });
        assert!(view.read_with(cx, |view, _| view.play_stop.is_cancelled()));
        assert!(!view.read_with(cx, |view, _| view.cancel.is_cancelled()));
    }

    #[gpui::test]
    fn requested_profile_waits_for_the_initial_catalog(cx: &mut gpui::TestAppContext) {
        let result = (|| -> anyhow::Result<()> {
            let root = tempfile::tempdir()?;
            let catalog = block_launcher::catalog::Catalog::open(root.path())?;
            let (instance, profile) =
                catalog.create_instance("Requested".into(), "1.20.6".into(), None, None)?;
            let _ =
                catalog.create_instance("Last in catalog".into(), "1.20.6".into(), None, None)?;
            let (sender, receiver) = tokio::sync::mpsc::channel(1);
            sender.try_send(super::Message::Catalog(Ok(catalog.snapshot()?)))?;
            let (view, cx) = crate::test_support::window(cx, test_view);
            view.update(cx, |view, cx| {
                view.tab = super::Tab::Home;
                view.pending_profile = Some(profile.id.clone());
                view.messages = Some(receiver);
                view.poll(cx);
            });
            assert_eq!(
                view.read_with(cx, |view, _| view.selected_profile.clone()),
                Some(profile.id)
            );
            assert_eq!(
                view.read_with(cx, |view, _| view.selected_instance.clone()),
                Some(instance.id)
            );
            assert!(view.read_with(cx, |view, _| view.pending_profile.is_none()));
            assert_eq!(
                view.read_with(cx, |view, _| view.tab),
                super::Tab::Instances
            );
            Ok(())
        })();
        assert!(result.is_ok(), "{result:?}");
    }

    #[gpui::test]
    fn inline_crash_help_returns_to_the_existing_launcher_without_launching(
        cx: &mut gpui::TestAppContext,
    ) {
        let result = (|| -> anyhow::Result<()> {
            let root = tempfile::tempdir()?;
            let guard = std::fs::File::create(root.path().join("presentation.lock"))?;
            let context = supervised_record().context;
            let report = block_launcher::crash::CrashReport::new(
                context,
                block_launcher::crash::ExitOutcome::start_failed(),
                "Java could not start",
                Vec::new(),
            );
            let (view, cx) = crate::test_support::window(cx, test_view);
            view.update(cx, |view, cx| {
                view.pending_crashes.push_back(super::PendingCrash {
                    report,
                    directory: root.path().to_path_buf(),
                    guard,
                });
                cx.notify();
            });
            cx.run_until_parked();
            assert!(view.read_with(cx, |view, _| view.crash_help.is_some()));
            assert!(view.read_with(cx, |view, _| view.crash_guard.is_some()));
            assert!(cx.debug_bounds("crash-return").is_some());
            crate::test_support::press(cx, "enter");
            assert!(view.read_with(cx, |view, _| view.crash_help.is_none()));
            assert!(view.read_with(cx, |view, _| view.crash_guard.is_none()));
            assert!(!view.read_with(cx, |view, _| view.busy));
            assert!(view.read_with(cx, |view, _| view.active_run.is_none()));
            Ok(())
        })();
        assert!(result.is_ok(), "{result:?}");
    }

    #[gpui::test]
    fn actual_block_form_routes_unicode_input_tab_and_compatible_loader_selection(
        cx: &mut gpui::TestAppContext,
    ) {
        let (view, cx) = crate::test_support::window(cx, test_view);
        let bounds = cx.debug_bounds("create-instance");
        assert!(bounds.is_some());
        let Some(bounds) = bounds else {
            return;
        };
        let window_count = cx.windows().len();
        cx.simulate_click(bounds.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        if cx.windows().len() > window_count
            && let Some(window) = cx.windows().last()
        {
            *cx = gpui::VisualTestContext::from_window(*window, &cx.cx);
        }
        crate::test_support::ensure_live_window(cx);
        cx.simulate_resize(gpui::size(gpui::px(1100.), gpui::px(1300.)));
        crate::test_support::press(cx, "cmd-a");
        cx.simulate_input("世界 🧱");
        assert_eq!(
            view.read_with(cx, |view, cx| match &view.form {
                Some(super::Form::Instance(form)) => form.name.read(cx).value(),
                _ => String::new(),
            }),
            "世界 🧱"
        );
        click_selector(cx, "wizard-component-version");
        crate::test_support::press(cx, "tab tab");
        crate::test_support::press(cx, "cmd-a");
        cx.simulate_input("1.20.1");
        assert_eq!(
            view.read_with(cx, |view, cx| match &view.form {
                Some(super::Form::Instance(form)) => form.version.read(cx).value(),
                _ => String::new(),
            }),
            "1.20.1"
        );
        click_selector(cx, "wizard-next");
        click_selector(cx, "wizard-component-loader");
        // The section now lives in a deferred library dialog; assert its row is
        // painted, then route the selection through the same action the row
        // dispatches. Pointer coordinates are unreliable while a dialog
        // entrance is still settling on slow CI runners.
        click_selector(cx, "wizard-loader-neoforge");
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.handle(
                    super::UiAction::WizardLoader(block_launcher::catalog::LoaderKind::NeoForge),
                    window,
                    cx,
                );
            });
        });
        cx.run_until_parked();
        let state = view.read_with(cx, |view, cx| {
            view.form.as_ref().map(|form| match form {
                super::Form::Instance(form) => (
                    form.is_neoforge(),
                    form.version.read(cx).value(),
                    form.loader.clone(),
                    form.loader_version.clone(),
                ),
                super::Form::Profile(_) => (
                    false,
                    String::new(),
                    block_launcher::catalog::LoaderKind::Vanilla,
                    None,
                ),
            })
        });
        assert!(
            state.as_ref().is_some_and(|(neoforge, version, _, build)| {
                *neoforge && version == "1.20.1" && build.is_none()
            }),
            "form after loader selection: {state:?}"
        );
        assert!(!view.read_with(cx, |view, _| view.busy));
    }

    #[gpui::test]
    fn game_wizard_four_square_tiles_and_footer_fit_default_window(cx: &mut gpui::TestAppContext) {
        let (view, cx) = crate::test_support::window(cx, test_view);
        cx.simulate_resize(gpui::size(gpui::px(960.), gpui::px(600.)));
        cx.run_until_parked();
        assert!(cx.debug_bounds("nav-profiles").is_none());
        click_selector(cx, "create-instance");
        let footer = cx.debug_bounds("wizard-footer");
        assert!(footer.is_some());
        let Some(footer) = footer else {
            return;
        };
        assert!(footer.bottom() <= gpui::px(600.), "footer: {footer:?}");
        assert_eq!(footer.size.height, gpui::px(72.));
        let mut previous_right = gpui::px(0.);
        let mut tile_top = None;
        for selector in [
            "wizard-component-version",
            "wizard-component-loader",
            "wizard-component-optifine",
            "wizard-component-location",
        ] {
            let bounds = crate::test_support::control_bounds(cx, selector);
            assert!(bounds.is_some(), "missing {selector}");
            let Some(bounds) = bounds else {
                return;
            };
            assert!((f32::from(bounds.size.width - bounds.size.height)).abs() < 1.);
            assert!(bounds.size.width >= gpui::px(120.));
            assert!(bounds.bottom() < footer.top(), "{selector}: {bounds:?}");
            assert!(bounds.left() >= previous_right);
            if let Some(top) = tile_top {
                assert_eq!(bounds.top(), top);
            }
            tile_top = Some(bounds.top());
            previous_right = bounds.right();
        }
        assert!(cx.debug_bounds("nav-profiles").is_none());
        click_selector(cx, "wizard-next");
        assert_eq!(
            view.read_with(cx, |view, _| view.shell.wizard.step),
            super::game_wizard::WizardStep::Review
        );
        let final_action = cx.debug_bounds("wizard-next");
        assert!(final_action.is_some());
        if let Some(bounds) = final_action {
            assert!(bounds.bottom() <= gpui::px(600.));
        }
        assert!(!view.read_with(cx, |view, _| view.busy));
    }

    #[gpui::test]
    fn game_wizard_keeps_component_drafts_and_cancel_never_writes_catalog(
        cx: &mut gpui::TestAppContext,
    ) {
        let result = (|| -> anyhow::Result<()> {
            let root = tempfile::tempdir()?;
            let catalog = block_launcher::catalog::Catalog::open(root.path())?;
            catalog.create_instance("Existing game".into(), "1.20.6".into(), None, None)?;
            let original = std::fs::read(root.path().join("catalog.json"))?;
            let snapshot = catalog.snapshot()?;
            let (engine, messages) = super::Engine::with_root(root.path().to_path_buf())?;
            let (view, cx) = crate::test_support::window(cx, test_view);
            view.update(cx, |view, cx| {
                view.engine = Some(engine);
                view.messages = Some(messages);
                view.apply_catalog(snapshot, cx);
                cx.notify();
            });
            click_selector(cx, "create-instance");
            cx.simulate_resize(gpui::size(gpui::px(1100.), gpui::px(1300.)));
            crate::test_support::press(cx, "cmd-a");
            cx.simulate_input("世界 draft");
            click_selector(cx, "wizard-component-version");
            crate::test_support::press(cx, "tab tab");
            crate::test_support::press(cx, "cmd-a");
            cx.simulate_input("1.20.1");
            click_selector(cx, "wizard-next");
            click_selector(cx, "wizard-component-location");
            let directory = root.path().join("future-game");
            cx.simulate_input(&directory.to_string_lossy());
            click_selector(cx, "wizard-next");
            click_selector(cx, "wizard-component-version");
            assert!(view.read_with(cx, |view, cx| {
                matches!(&view.form, Some(super::Form::Instance(form))
                    if form.name.read(cx).value() == "世界 draft"
                        && form.version.read(cx).value() == "1.20.1"
                        && form.directory.read(cx).value() == directory.to_string_lossy())
            }));
            assert!(!view.read_with(cx, |view, _| view.busy));
            click_selector(cx, "cancel-instance");
            assert!(view.read_with(cx, |view, _| view.form.is_some()
                && view.shell.wizard.page.is_none()));
            click_selector(cx, "cancel-instance");
            assert!(view.read_with(cx, |view, _| view.form.is_none()));
            assert!(!view.read_with(cx, |view, _| view.busy));
            assert_eq!(std::fs::read(root.path().join("catalog.json"))?, original);
            assert!(!directory.exists());
            Ok(())
        })();
        assert!(result.is_ok(), "{result:?}");
    }

    #[gpui::test]
    fn game_wizard_review_starts_real_install_only_after_final_confirmation(
        cx: &mut gpui::TestAppContext,
    ) {
        let result = (|| -> anyhow::Result<()> {
            let root = tempfile::tempdir()?;
            let catalog = block_launcher::catalog::Catalog::open(root.path())?;
            let (existing, _) =
                catalog.create_instance("Existing game".into(), "1.20.6".into(), None, None)?;
            let original = std::fs::read(root.path().join("catalog.json"))?;
            let snapshot = catalog.snapshot()?;
            let (engine, messages) = super::Engine::with_root(root.path().to_path_buf())?;
            let (view, cx) = crate::test_support::window(cx, test_view);
            view.update(cx, |view, cx| {
                view.engine = Some(engine);
                view.messages = Some(messages);
                view.apply_catalog(snapshot, cx);
                cx.notify();
            });
            click_selector(cx, "create-instance");
            cx.simulate_resize(gpui::size(gpui::px(1100.), gpui::px(1300.)));
            view.update(cx, |view, cx| {
                if let Some(super::Form::Instance(form)) = &view.form {
                    form.directory.update(cx, |field, cx| {
                        field.set_value(&existing.directory.to_string_lossy(), cx);
                    });
                }
            });
            click_selector(cx, "wizard-next");
            assert_eq!(
                view.read_with(cx, |view, _| view.shell.wizard.step),
                super::game_wizard::WizardStep::Review
            );
            assert!(!view.read_with(cx, |view, _| view.busy));
            assert_eq!(std::fs::read(root.path().join("catalog.json"))?, original);
            click_selector(cx, "wizard-back");
            click_selector(cx, "wizard-next");
            click_selector(cx, "wizard-next");
            assert!(view.read_with(cx, |view, _| view.busy));
            // The real engine rejects this overlapping location before network
            // installation. That failure proves the final action reached it.
            for _ in 0..200 {
                view.update(cx, super::Block::poll);
                if !view.read_with(cx, |view, _| view.busy) {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            assert!(!view.read_with(cx, |view, _| view.busy));
            assert!(view.read_with(cx, |view, _| {
                view.status
                    .contains("installation directory overlaps another instance")
            }));
            assert_eq!(std::fs::read(root.path().join("catalog.json"))?, original);
            assert_eq!(catalog.snapshot()?.instances.len(), 1);
            Ok(())
        })();
        assert!(result.is_ok(), "{result:?}");
    }

    #[gpui::test]
    fn actual_block_play_requires_an_account_instead_of_silent_offline_identity(
        cx: &mut gpui::TestAppContext,
    ) {
        let (view, cx) = crate::test_support::window(cx, test_view);
        view.update(cx, |view, cx| {
            view.selected_profile = Some("profile".into());
            view.play(cx);
        });
        assert_eq!(view.read_with(cx, |view, _| view.tab), super::Tab::Accounts);
        assert!(!view.read_with(cx, |view, _| view.busy));
        assert!(view.read_with(cx, |view, _| {
            view.status.contains("Add and select an account")
        }));
    }

    #[test]
    fn edited_argument_text_round_trips_spaces_quotes_and_path_separators() {
        let args = vec![
            r"-Dpath=C:\My Games\".to_owned(),
            "-Dquoted=a\"b".into(),
            String::new(),
            "--demo".into(),
        ];
        assert_eq!(
            parse_arguments(&super::format_arguments(&args)).ok(),
            Some(args)
        );
    }
    #[gpui::test]
    fn actual_profile_editor_keeps_invalid_drafts_and_preserves_saved_settings(
        cx: &mut gpui::TestAppContext,
    ) {
        use block_launcher::{
            catalog::{DataPolicy, InstallStatus, Instance, LaunchTarget, LoaderKind, Profile},
            recipe::{Compatibility, RuntimeSelection},
        };
        let (view, cx) = crate::test_support::window(cx, test_view);
        cx.simulate_resize(gpui::size(gpui::px(1300.), gpui::px(1600.)));
        view.update(cx, |view, cx| {
            view.snapshot.instances.push(Instance {
                settings: None,
                id: "instance".into(),
                name: "Worlds".into(),
                minecraft_version: "1.20.6".into(),
                directory: std::env::temp_dir().join("block-ui-synthetic"),
                targets: vec![LaunchTarget {
                    id: "vanilla".into(),
                    kind: LoaderKind::Vanilla,
                    loader_version: None,
                    manifest: None,
                    status: InstallStatus::Pending,
                    optifine: None,
                    optifine_mod: None,
                }],
            });
            view.snapshot.profiles.push(Profile {
                id: "profile".into(),
                instance_id: "instance".into(),
                name: "Default".into(),
                target_id: "vanilla".into(),
                runtime: RuntimeSelection::Managed,
                memory_mib: 2048,
                jvm_args: Vec::new(),
                game_args: Vec::new(),
                data_policy: DataPolicy::Isolated,
                appearance: block_launcher::catalog::ProfileAppearance::default(),
                inheritance: block_launcher::catalog::ProfileInheritance::default(),
                server: block_launcher::catalog::ServerPreferences::default(),
                default_account: None,
                compatibility: Compatibility::Official,
            });
            view.normalize_selection();
            cx.notify();
        });
        cx.run_until_parked();
        let bounds = cx.debug_bounds("edit-profile");
        assert!(bounds.is_some());
        let Some(bounds) = bounds else {
            return;
        };
        let window_count = cx.windows().len();
        cx.simulate_click(bounds.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        if cx.windows().len() > window_count
            && let Some(window) = cx.windows().last()
        {
            *cx = gpui::VisualTestContext::from_window(*window, &cx.cx);
        }
        crate::test_support::ensure_live_window(cx);
        view.update(cx, |view, cx| {
            if let Some(super::Form::Profile(form)) = &view.form {
                form.memory
                    .update(cx, |field, cx| field.set_value("invalid", cx));
            }
        });
        click_selector(cx, "save-profile");
        assert!(view.read_with(cx, |view, _| {
            view.status
                .contains("Choose a memory limit between 1 and 32 GB.")
        }));
        assert_eq!(
            view.read_with(cx, |view, _| view.profile().map(|p| p.memory_mib)),
            Some(2048)
        );
        assert!(view.read_with(cx, |view, _| matches!(
            view.form,
            Some(super::Form::Profile(_))
        )));
    }
    #[gpui::test]
    #[expect(
        clippy::too_many_lines,
        reason = "Exercise startup, missing defaults and explicit account overrides in one unchanged catalog workflow."
    )]
    fn profile_defaults_apply_on_start_and_switch_without_substituting_a_missing_identity(
        cx: &mut gpui::TestAppContext,
    ) {
        let result = (|| -> anyhow::Result<()> {
            let root = tempfile::tempdir()?;
            let catalog = block_launcher::catalog::Catalog::open(root.path())?;
            let (first, mut first_profile) =
                catalog.create_instance("First".into(), "1.20.6".into(), None, None)?;
            let (second, mut second_profile) =
                catalog.create_instance("Second".into(), "1.20.6".into(), None, None)?;
            let account_a = block_auth::Account {
                reference: block_auth::AccountRef {
                    player_id: "00000000-0000-4000-8000-000000000001".parse()?,
                },
                provider: block_auth::AccountProvider::Microsoft,
                player_name: "AccountA".into(),
                client_id: "public-client".into(),
                credential_ref: "synthetic-a".into(),
                authlib: None,
            };
            let mut account_b = account_a.clone();
            account_b.reference.player_id = "00000000-0000-4000-8000-000000000002".parse()?;
            account_b.player_name = "AccountB".into();
            account_b.credential_ref = "synthetic-b".into();
            first_profile.default_account = Some(super::account_id(&account_a));
            second_profile.default_account = Some("missing-account".into());
            catalog.save_profile(first_profile.clone())?;
            catalog.save_profile(second_profile.clone())?;
            let snapshot = catalog.snapshot()?;
            let (view, cx) = crate::test_support::window(cx, test_view);
            view.update(cx, |view, cx| {
                view.accounts = vec![account_b.clone(), account_a.clone()];
                view.snapshot = snapshot;
                view.selected_instance = Some(first.id.clone());
                view.normalize_selection();
                cx.notify();
            });
            assert_eq!(
                view.read_with(cx, |view, _| view.selected_account.clone()),
                Some(super::account_id(&account_a))
            );
            cx.update(|window, cx| {
                view.update(cx, |view, cx| {
                    view.handle(super::UiAction::Inspect(second.id), window, cx);
                    assert_eq!(view.selected_account, Some(super::account_id(&account_a)));
                    assert_eq!(
                        view.selected_profile.as_deref(),
                        Some(first_profile.id.as_str())
                    );
                    view.handle(
                        super::UiAction::SelectProfile(second_profile.id.clone()),
                        window,
                        cx,
                    );
                });
            });
            assert_eq!(
                view.read_with(cx, |view, _| view.selected_account.clone()),
                None
            );
            view.update(cx, |view, cx| view.play(cx));
            assert!(view.read_with(cx, |view, _| {
                view.status.contains("default account is unavailable")
            }));
            cx.update(|window, cx| {
                view.update(cx, |view, cx| {
                    view.handle(
                        super::UiAction::SelectAccount(super::account_id(&account_b)),
                        window,
                        cx,
                    );
                });
            });
            assert_eq!(
                view.read_with(cx, |view, _| view.selected_account.clone()),
                Some(super::account_id(&account_b))
            );
            cx.update(|window, cx| {
                view.update(cx, |view, cx| {
                    view.handle(
                        super::UiAction::SelectProfile(first_profile.id.clone()),
                        window,
                        cx,
                    );
                });
            });
            assert_eq!(
                view.read_with(cx, |view, _| view.selected_account.clone()),
                Some(super::account_id(&account_a))
            );
            cx.update(|window, cx| {
                view.update(cx, |view, cx| {
                    view.handle(
                        super::UiAction::SelectAccount(super::account_id(&account_b)),
                        window,
                        cx,
                    );
                });
            });
            cx.update(|window, cx| {
                view.update(cx, |view, cx| {
                    view.handle(super::UiAction::SelectProfile(first_profile.id), window, cx);
                });
            });
            assert_eq!(
                view.read_with(cx, |view, _| view.selected_account.clone()),
                Some(super::account_id(&account_a))
            );
            Ok(())
        })();
        assert!(result.is_ok(), "{result:?}");
    }
    #[gpui::test]
    fn orphan_browser_requests_do_not_become_authorized_by_an_unrelated_new_cancel_token(
        cx: &mut gpui::TestAppContext,
    ) {
        let (view, cx) = crate::test_support::window(cx, test_view);
        let (sender, receiver) = tokio::sync::mpsc::channel(4);
        assert!(
            sender
                .try_send(super::Message::Browser(
                    "https://example.invalid/synthetic-authorization".into()
                ))
                .is_ok()
        );
        view.update(cx, |view, cx| {
            view.messages = Some(receiver);
            view.cancel.cancel();
            view.poll(cx);
        });
        assert_eq!(cx.opened_url(), None);
        assert!(
            sender
                .try_send(super::Message::Browser(
                    "https://example.invalid/new-synthetic-authorization".into()
                ))
                .is_ok()
        );
        view.update(cx, |view, cx| {
            view.cancel = block_launcher::CancellationToken::new();
            view.poll(cx);
        });
        assert_eq!(cx.opened_url(), None);
    }
}
