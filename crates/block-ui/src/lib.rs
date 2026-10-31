//! A native launcher with persistent installations, launch profiles and Microsoft accounts.

#[path = "settings/about.rs"]
mod about;
#[path = "navigation/actions.rs"]
mod actions;
#[path = "settings/appearance.rs"]
mod appearance;
#[path = "accounts/authentication.rs"]
mod authentication;
#[path = "controls/base_controls.rs"]
mod base_controls;
#[path = "controls/base_overlay.rs"]
mod base_overlay;
#[path = "games/browser.rs"]
mod browser;
#[path = "controls/components.rs"]
mod components;
#[path = "feedback/connection_test.rs"]
mod connection_test;
#[path = "feedback/crash_view.rs"]
mod crash_view;
#[path = "settings/design.rs"]
mod design;
#[path = "games/downloads.rs"]
mod downloads;
#[path = "overlays/drawer.rs"]
mod drawer;
#[path = "overlays/drawer_gestures.rs"]
mod drawer_gestures;
#[path = "runtime/engine.rs"]
mod engine;
#[path = "runtime/events.rs"]
mod events;
#[path = "feedback/feedback.rs"]
mod feedback;
#[path = "setup/first_run.rs"]
mod first_run;
#[cfg(test)]
#[path = "setup/first_run_tests.rs"]
mod first_run_tests;
#[path = "navigation/frame_motion.rs"]
mod frame_motion;
#[path = "games/game_wizard.rs"]
mod game_wizard;
#[path = "settings/global_config.rs"]
mod global_config;
#[path = "games/import_games.rs"]
mod import_games;
#[path = "games/instance_editor.rs"]
mod instance_editor;
#[cfg(test)]
#[path = "runtime/interaction_tests.rs"]
mod interaction_tests;
#[path = "navigation/motion.rs"]
mod motion;
#[cfg(test)]
#[path = "runtime/native_regressions.rs"]
mod native_regressions;
#[path = "navigation/navigation.rs"]
mod navigation;
#[path = "accounts/onboarding.rs"]
mod onboarding;
#[path = "overlays/overlays.rs"]
mod overlays;
#[path = "navigation/page_host.rs"]
mod page_host;
#[path = "controls/patterns.rs"]
mod patterns;
#[path = "runtime/platform.rs"]
mod platform;
#[path = "games/profile_editor.rs"]
mod profile_editor;
#[cfg(test)]
#[path = "games/profile_editor_tests.rs"]
mod profile_editor_tests;
#[path = "accounts/providers.rs"]
mod providers;
#[cfg(debug_assertions)]
#[path = "runtime/scenarios.rs"]
mod scenarios;
#[path = "controls/scroll_area.rs"]
mod scroll_area;
#[path = "controls/search_selector.rs"]
mod search_selector;
#[path = "controls/select.rs"]
mod select;
#[path = "settings/settings.rs"]
mod settings;
#[path = "setup/setup_storage.rs"]
mod setup_storage;
#[path = "navigation/shell.rs"]
mod shell;
#[path = "runtime/tasks.rs"]
mod tasks;
#[cfg(test)]
#[path = "runtime/test_support.rs"]
mod test_support;
#[path = "controls/text_input.rs"]
mod text_input;
#[path = "controls/tooltip.rs"]
mod tooltip;
#[path = "navigation/transitions.rs"]
mod transitions;
#[path = "runtime/ui_logging.rs"]
mod ui_logging;
#[path = "setup/welcome_button.rs"]
mod welcome_button;
#[path = "navigation/window_geometry.rs"]
mod window_geometry;
#[path = "navigation/wizard_entrance.rs"]
mod wizard_entrance;

pub use about::BuildInfo;
pub use crash_view::CrashView;
pub use design::{UiAssets, load_fonts};
pub use gpui_base::Root as ProductRoot;
#[cfg(debug_assertions)]
pub use scenarios::{SCENARIOS, ScenarioOptions};
pub use window_geometry::{initial_window_bounds, preferred_management_size};

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
    catalog::{Instance, Profile, Snapshot},
    crash::CrashReport,
    platform::Architecture,
    recipe::{LaunchRecipe, RuntimeSelection},
    session_host::{self, RunRecord, RunState},
};
use gpui::{
    AppContext as _, Bounds, Context, Entity, FocusHandle, Focusable as _, InteractiveElement as _,
    IntoElement, KeyBinding, ParentElement as _, PathPromptOptions, Pixels, Render, SharedString,
    Styled as _, WeakEntity, Window, div, px, rgb,
};
use gpui_component::WindowExt as _;
#[cfg(any(windows, target_os = "linux", target_os = "macos"))]
use raw_window_handle::RawWindowHandle;
use tokio::sync::mpsc::Receiver;

use actions::UiAction;
use appearance::{Appearance, Palette, Preferences};
use engine::{Engine, Message};
use game_wizard::{WizardPage, WizardStep};
use navigation::{Router, Tab};
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
    view_id: u64,
    window_changes: ui_logging::WindowChanges,
    shell: ShellState,
    motion: transitions::MotionState,
    motion_dirty: bool,
    visual_only: bool,
    pending_window_size: Option<gpui::Size<Pixels>>,
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
    version_catalog: Vec<block_launcher::manifest::model::VersionEntry>,
    selected_instance: Option<String>,
    selected_profile: Option<String>,
    pending_profile: Option<String>,
    selected_account: Option<String>,
    router: Router,
    form: Option<Form>,
    engine: Option<Engine>,
    messages: Option<Receiver<Message>>,
    focus: FocusHandle,
    status: String,
    completed: usize,
    operation_kind: OperationKind,
    operation_revision: u64,
    logged_operation: Option<ui_logging::Operation>,
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
    exporting_logs: bool,
    log_export_cancel: Option<CancellationToken>,
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

const INSTALL_CHOOSER_ERROR: &str =
    "Could not open the installation folder chooser. Check your system file picker and try again.";

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
        if let Some(cancel) = &self.log_export_cancel {
            cancel.cancel();
        }
        self.finish_logged_operation(ui_logging::Outcome::ViewClosed);
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
        window.resize(window_geometry::home_size(window));
        let mut view = Self {
            build_info: options.build_info,
            view_id: cx.entity_id().as_u64(),
            window_changes: ui_logging::WindowChanges::default(),
            motion: transitions::MotionState::default(),
            motion_dirty: true,
            visual_only: false,
            shell: ShellState {
                ..ShellState::default()
            },
            pending_window_size: None,
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
            router: Router::new(tab),
            form: None,
            engine,
            messages,
            focus,
            status,
            completed: 0,
            operation_kind: OperationKind::General,
            operation_revision: 0,
            logged_operation: None,
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
            exporting_logs: false,
            log_export_cancel: None,
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
            view.begin_first_run(cx);
            if view.shell.first_run.stage != first_run::Stage::Welcome {
                window.resize(window_geometry::management_size(window));
            }
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
            self.shell.games.inspected = Some(
                if self.snapshot.profile_mode == block_launcher::catalog::ProfileMode::Bundled {
                    profile.id.clone()
                } else {
                    profile.instance_id.clone()
                },
            );
            self.shell.profiles.inspected = Some(profile.id.clone());
            if self.form.is_none()
                && self.shell.auth.is_none()
                && self.shell.instance_editor.is_none()
                && !self.setup_incomplete()
            {
                if self.router.tab() == Tab::Home {
                    self.pending_window_size = Some(preferred_management_size());
                }
                self.router.navigate(Tab::Instances);
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
        let previous = self.session_revisions.get(&id).copied();
        if let Some(previous) = previous
            && (previous.0 > record.revision || previous == fingerprint)
        {
            return;
        }
        if (previous.is_some() || record.is_active())
            && previous.is_none_or(|previous| previous.1 != record.state)
            && let Some(id) = ui_logging::session_id(&id)
        {
            ui_logging::session_state(
                self.view_id,
                id,
                record.state,
                record.outcome.as_ref().map(|outcome| outcome.kind),
            );
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
        // Do not create the focus-taking view until the current modal has released input.
        if self.crash_help.is_some()
            || self.pending_crashes.is_empty()
            || self.shell.surfaces.is_open()
            || window.has_active_dialog(cx)
            || window.has_active_sheet(cx)
            || drawer::DrawerLayer::is_open(window, cx)
        {
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
                tab: self.router.tab(),
                draft: form.name.entity_id().as_u64(),
            });
        }
    }

    fn creation_saved(&mut self, instance: String, profile: String, cx: &mut Context<'_, Self>) {
        let context = self
            .pending_creation
            .take()
            .filter(|context| context.operation == self.operation_revision);
        let owns_draft = context.is_some_and(|context| context.tab == self.router.tab() && matches!(&self.form, Some(Form::Instance(form)) if form.name.entity_id().as_u64() == context.draft));
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
        if owns_draft {
            if self.router.tab() == Tab::Onboarding {
                self.form = None;
                self.router.leave_task();
                self.shell.first_run.stage = first_run::Stage::Overview;
            } else {
                self.form = None;
                self.router.leave_task();
                if let Some(query) = &self.collection().query {
                    query.update(cx, |field, cx| field.set_value("", cx));
                }
            }
        }
    }

    fn normalize_selection(&mut self) {
        if self.router.tab() == Tab::Onboarding {
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
        if self.router.tab() == Tab::Onboarding {
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

    fn notify_logging_failure(&mut self, failed: bool, cx: &mut Context<'_, Self>) -> bool {
        if !ui_logging::claim_failure_notice(failed, cx) {
            return false;
        }
        self.push_toast(
            ui_logging::FAILURE_MESSAGE.into(),
            feedback::ToastKind::Error,
        );
        true
    }

    /// Setup owns navigation until the first-run draft is complete.
    fn setup_incomplete(&self) -> bool {
        !self.preferences.onboarding_complete && self.shell.first_run.enabled
    }

    fn report(&mut self, error: String) {
        self.push_toast(error.clone(), feedback::ToastKind::Error);
        if self.router.tab() == Tab::Onboarding && self.shell.first_run.enabled {
            self.shell.first_run.error = Some(error.clone());
            if self.shell.first_run.stage == first_run::Stage::GameWizard {
                self.shell.wizard.error = Some(error.clone());
            }
        }
        self.push_log(error.clone());
        self.status = error;
    }

    fn begin_cancellable(&mut self, status: &str, operation: ui_logging::Operation) {
        self.begin(status, operation);
        self.operation_cancellable = self.busy;
    }

    const fn can_stop(&self) -> bool {
        self.busy && (self.active_run.is_some() || self.operation_cancellable)
    }

    fn begin(&mut self, status: &str, operation: ui_logging::Operation) {
        self.operation_cancellable = false;
        self.operation_kind = OperationKind::General;
        if self.engine.is_none() {
            self.status =
                "Engine unavailable · restart Block after checking its data directory".into();
            return;
        }
        self.operation_revision = self.operation_revision.wrapping_add(1);
        self.logged_operation = Some(operation);
        ui_logging::operation_started(self.view_id, self.operation_revision, operation);
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

    fn finish_logged_operation(&mut self, outcome: ui_logging::Outcome) {
        if let Some(operation) = self.logged_operation.take() {
            ui_logging::operation_finished(
                self.view_id,
                self.operation_revision,
                operation,
                outcome,
            );
        }
    }

    fn abandon_operation(&mut self, revision: u64) -> bool {
        if revision != self.operation_revision {
            return false;
        }
        self.finish_logged_operation(ui_logging::Outcome::Cancelled);
        self.operation_revision = self.operation_revision.wrapping_add(1);
        if let Some(engine) = &mut self.engine {
            engine.begin_operation(self.operation_revision);
        }
        self.busy = self.active_run.is_some();
        self.operation_cancellable = false;
        true
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
        self.advance_navigation_focus(backwards, window, cx);
    }

    fn focus_field(&self, backwards: bool, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.router.tab() != Tab::Onboarding
            && matches!(self.form, Some(Form::Instance(_)))
            && self.focus_wizard_error(window, cx)
        {
            return;
        }
        if self.router.tab() == Tab::Settings {
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
            self.router.navigate(Tab::Accounts);
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
        self.begin_cancellable(
            "Preparing · validating profile and account",
            ui_logging::Operation::Launch,
        );
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
        let field = form.directory.entity_id();
        let revision = self.shell.wizard.revision;
        let selection = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose an installation directory".into()),
        });
        cx.spawn(async move |view, cx| {
            let selection = match selection.await {
                Ok(Ok(paths)) => Ok(paths.and_then(|paths| paths.into_iter().next())),
                _ => Err(()),
            };
            let _ = view.update(cx, |view, cx| {
                view.finish_install_browse(field, revision, selection, cx);
            });
        })
        .detach();
    }

    fn finish_install_browse(
        &mut self,
        field: gpui::EntityId,
        revision: usize,
        selection: Result<Option<PathBuf>, ()>,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(Form::Instance(form)) = &self.form else {
            return;
        };
        if self.busy
            || self.shell.wizard.page != Some(WizardPage::Location)
            || self.shell.wizard.revision != revision
            || form.directory.entity_id() != field
        {
            return;
        }
        match selection {
            Ok(Some(path)) => {
                form.directory.update(cx, |field, cx| {
                    field.set_value(&path.display().to_string(), cx);
                });
                if self.shell.wizard.error.as_deref() == Some(INSTALL_CHOOSER_ERROR) {
                    self.shell.wizard.error = None;
                }
                if self.shell.first_run.error.as_deref() == Some(INSTALL_CHOOSER_ERROR) {
                    self.shell.first_run.error = None;
                }
                if self.status == INSTALL_CHOOSER_ERROR {
                    self.status = "Installation folder selected.".into();
                }
            }
            Ok(None) => return,
            Err(()) => {
                self.shell.wizard.error = Some(INSTALL_CHOOSER_ERROR.into());
                self.report(INSTALL_CHOOSER_ERROR.into());
            }
        }
        cx.notify();
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
            let selection = match prompt.await {
                Ok(Ok(paths)) => Ok(paths.and_then(|paths| paths.into_iter().next())),
                _ => Err(()),
            };
            let _ = view.update(cx, |view, cx| view.finish_recipe_browse(selection, cx));
        })
        .detach();
    }

    fn finish_recipe_browse(
        &mut self,
        selection: Result<Option<PathBuf>, ()>,
        cx: &mut Context<'_, Self>,
    ) {
        if self.busy || self.recipe_load.is_loading() {
            return;
        }
        match selection {
            Ok(Some(path)) => self.load_recipe(path, false, cx),
            Ok(None) => return,
            Err(()) => self.report(
                "Could not open the recipe chooser. Check your system file picker and try again."
                    .into(),
            ),
        }
        cx.notify();
    }

    fn launch_recipe(&mut self, cx: &mut Context<'_, Self>) {
        if self.busy || self.recipe_load.is_loading() {
            return;
        }
        if let Some(recipe) = self.recipe.clone() {
            self.begin_cancellable(
                "Preparing explicit development recipe",
                ui_logging::Operation::LaunchRecipe,
            );
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
            | UiAction::SignIn
            | UiAction::Drawer(Some(Drawer::Profiles)) => components::ButtonVariant::Primary,
            UiAction::SignOut(_) | UiAction::ConfirmDelete => {
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
            || matches!(action, UiAction::Stop) && !self.can_stop()
            || self.import_action_disabled(&action);
        let disabled = disabled
            || matches!(action, UiAction::AuthCancel) && self.auth_save_pending()
            || matches!(action, UiAction::ExportLogs) && self.exporting_logs;
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
            if view.busy && !view.can_activate(&action) {
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
        self.notify_logging_failure(block_logging::failure().is_some(), cx);
        let reduced_motion = self.motion_preferences().reduced();
        self.motion.apply_policy(reduced_motion, cx);
        self.window_changes.observe(
            self.view_id,
            f32::from(window.viewport_size().width),
            f32::from(window.viewport_size().height),
            window.scale_factor(),
            reduced_motion,
            cx.background_executor().now(),
        );
        self.prepare_crash_help(window, cx);
        self.sync_provider_drop(window, cx);
        feedback::ToastLayer::enqueue(std::mem::take(&mut self.shell.pending_toasts), window, cx);
        if let Some(help) = &self.crash_help {
            return div().size_full().child(help.clone()).into_any_element();
        }
        if self.pending_window_size.take().is_some() {
            self.request_frame_resize(window, cx);
        }
        self.prepare_window_frame(window, cx);
        self.palette = self.preferences.palette(window.appearance());
        cx.set_global(self.palette);
        sync_component_theme(self.palette, cx);
        let action_rail = if self.router.tab() == Tab::Home && self.form.is_none() {
            104.
        } else {
            design::MANAGEMENT_FOOTER
        };
        feedback::ToastLayer::reserve_actions(px(action_rail + 12.), window, cx);
        self.prepare_motion(window, cx);
        let title = self.navigation_title();
        window.set_window_title(
            &if self.router.tab() == Tab::Home && self.shell.auth.is_none() {
                "Block".to_owned()
            } else {
                format!("Block · {title}")
            },
        );
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
            assert_eq!(
                view.read_with(cx, |view, _| view.router.tab()),
                Tab::Accounts
            );
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
            view_id: cx.entity_id().as_u64(),
            window_changes: super::ui_logging::WindowChanges::default(),
            motion: super::transitions::MotionState::default(),
            motion_dirty: true,
            visual_only: false,
            shell: super::ShellState::default(),
            pending_window_size: None,
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
            router: super::Router::new(super::Tab::Instances),
            form: None,
            engine: None,
            messages: None,
            focus,
            status: "Welcome".into(),
            completed: 0,
            operation_kind: super::OperationKind::General,
            operation_revision: 0,
            logged_operation: None,
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
            exporting_logs: false,
            log_export_cancel: None,
            command: None,
            show_command: false,
            recipe: None,
            recipe_load: RecipeLoadState::default(),
            #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
            provider_drop: None,
            pending_play: false,
        }
    }

    #[gpui::test]
    fn logging_storage_failure_notification_is_shown_only_once(cx: &mut gpui::TestAppContext) {
        let (view, cx) = crate::test_support::window(cx, test_view);
        view.update(cx, |view, cx| {
            assert!(!view.notify_logging_failure(false, cx));
            assert!(view.notify_logging_failure(true, cx));
            assert!(!view.notify_logging_failure(true, cx));
            assert_eq!(view.shell.pending_toasts.len(), 1);
            assert_eq!(
                view.shell.pending_toasts[0].message,
                "Launcher logs are unavailable. Check disk space and permissions."
            );
        });
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
    fn folder_and_recipe_chooser_failures_are_visible_and_cancellation_is_quiet(
        cx: &mut gpui::TestAppContext,
    ) {
        let result = (|| -> anyhow::Result<()> {
            let root = tempfile::tempdir()?;
            let (engine, messages) = super::Engine::with_root(root.path().to_path_buf())?;
            let (view, cx) = crate::test_support::window(cx, test_view);
            view.update(cx, |view, cx| {
                view.engine = Some(engine);
                view.messages = Some(messages);
                view.preferences.reduce_motion = true;
                cx.notify();
            });
            cx.simulate_resize(gpui::size(gpui::px(1100.), gpui::px(1300.)));
            click_selector(cx, "create-instance");
            click_selector(cx, "wizard-component-location");
            let state = view.update(cx, |view, cx| {
                let Some(super::Form::Instance(form)) = &view.form else {
                    return None;
                };
                form.directory
                    .update(cx, |field, cx| field.set_value("/draft/game", cx));
                Some((form.directory.entity_id(), view.shell.wizard.revision))
            });
            let Some((field, revision)) = state else {
                anyhow::bail!("missing instance draft");
            };
            let previous_status = view.read_with(cx, |view, _| view.status.clone());
            click_selector(cx, "browse-install");
            assert!(cx.did_prompt_for_paths());
            cx.simulate_path_prompt_response(|_| None);
            cx.run_until_parked();
            assert_eq!(
                view.read_with(cx, |view, _| view.status.clone()),
                previous_status
            );
            click_selector(cx, "browse-install");
            cx.simulate_path_prompt_response(|_| {
                Some(vec![std::path::PathBuf::from("/chosen/game")])
            });
            cx.run_until_parked();
            assert!(view.read_with(cx, |view, cx| {
                matches!(&view.form, Some(super::Form::Instance(form))
                    if form.directory.read(cx).value() == "/chosen/game")
            }));
            view.update(cx, |view, cx| {
                if let Some(super::Form::Instance(form)) = &view.form {
                    form.directory
                        .update(cx, |field, cx| field.set_value("/draft/game", cx));
                }
            });
            view.update(cx, |view, cx| {
                view.finish_install_browse(field, revision, Err(()), cx)
            });
            assert!(crate::test_support::control_bounds(cx, "wizard-location-error").is_some());
            assert!(view.read_with(cx, |view, cx| {
                view.status.contains("installation folder chooser")
                    && matches!(&view.form, Some(super::Form::Instance(form))
                        if form.directory.read(cx).value() == "/draft/game")
            }));
            click_selector(cx, "browse-install");
            cx.simulate_path_prompt_response(|_| {
                Some(vec![std::path::PathBuf::from("/chosen/after-retry")])
            });
            cx.run_until_parked();
            assert!(view.read_with(cx, |view, cx| {
                view.status == "Installation folder selected."
                    && view.shell.wizard.error.is_none()
                    && matches!(&view.form, Some(super::Form::Instance(form))
                        if form.directory.read(cx).value() == "/chosen/after-retry")
            }));
            view.update(cx, |view, cx| {
                view.router.navigate(super::Tab::Onboarding);
                view.shell.first_run.enabled = true;
                view.shell.first_run.stage = super::first_run::Stage::GameWizard;
                view.finish_install_browse(field, revision, Err(()), cx);
                assert_eq!(
                    view.shell.first_run.error.as_deref(),
                    Some(super::INSTALL_CHOOSER_ERROR)
                );
                view.finish_install_browse(
                    field,
                    revision,
                    Ok(Some(std::path::PathBuf::from("/chosen/first-run"))),
                    cx,
                );
                assert!(view.shell.first_run.error.is_none());
                view.shell.first_run.enabled = false;
                view.router.navigate(super::Tab::Instances);
                view.open_game_wizard_page(None, cx);
                view.open_game_wizard_page(Some(super::WizardPage::Location), cx);
                view.status = "another wizard section".into();
                view.finish_install_browse(field, revision, Err(()), cx);
                assert_eq!(view.status, "another wizard section");
                view.begin_game_wizard(cx);
                view.status = "new draft".into();
                view.finish_install_browse(field, revision, Err(()), cx);
                assert_eq!(view.status, "new draft");
            });

            let (engine, messages) = super::Engine::with_root(root.path().to_path_buf())?;
            let (about, cx) = crate::test_support::window(cx, |window, cx| {
                let mut view = test_view(window, cx);
                view.engine = Some(engine);
                view.messages = Some(messages);
                view.preferences.reduce_motion = true;
                view.router.navigate(super::Tab::About);
                view.show_output = true;
                view
            });
            cx.simulate_resize(gpui::size(gpui::px(1100.), gpui::px(1300.)));
            cx.run_until_parked();
            about.update(cx, |view, cx| {
                view.shell.about_scroll.scroll_to_bottom();
                cx.notify();
            });
            let previous_status = about.read_with(cx, |view, _| view.status.clone());
            click_selector(cx, "open-recipe");
            assert!(cx.did_prompt_for_paths());
            cx.simulate_path_prompt_response(|_| None);
            cx.run_until_parked();
            assert_eq!(
                about.read_with(cx, |view, _| view.status.clone()),
                previous_status
            );
            about.update(cx, |view, cx| view.finish_recipe_browse(Err(()), cx));
            assert!(about.read_with(cx, |view, _| {
                view.status.contains("recipe chooser") && view.recipe.is_none()
            }));
            about.update(cx, |view, cx| {
                view.busy = true;
                view.status = "another operation".into();
                view.finish_recipe_browse(Err(()), cx);
                assert_eq!(view.status, "another operation");
            });
            Ok(())
        })();
        assert!(result.is_ok(), "{result:?}");
    }

    #[gpui::test]
    fn first_run_onboarding_saves_only_on_completion_and_default_reset_keeps_it(
        cx: &mut gpui::TestAppContext,
    ) {
        let result = (|| -> anyhow::Result<()> {
            let root = tempfile::tempdir()?;
            let (engine, _messages) = super::Engine::with_root(root.path().into())?;
            let (view, cx) = crate::test_support::window(cx, test_view);
            cx.update(|window, cx| {
                view.update(cx, |view, cx| {
                    view.engine = Some(engine);
                    view.begin_first_run(cx);
                    view.handle_first_run(super::first_run::Action::Custom, window, cx);
                });
            });
            assert_eq!(
                super::Preferences::load(root.path())?,
                super::Preferences::default()
            );
            assert!(view.read_with(cx, |view, _| view.form.is_some()));
            cx.update(|window, cx| {
                view.update(cx, |view, cx| {
                    view.handle_first_run(super::first_run::Action::Skip, window, cx);
                });
            });
            assert_eq!(
                view.read_with(cx, |view, _| view.router.tab()),
                super::Tab::Home
            );
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
            cx.update(|window, cx| {
                view.update(cx, |view, cx| {
                    view.engine = Some(engine);
                    view.begin_first_run(cx);
                    view.handle_first_run(super::first_run::Action::Custom, window, cx);
                    view.busy = true;
                    cx.notify();
                });
            });
            cx.run_until_parked();
            cx.update(|window, cx| {
                view.update(cx, |view, cx| {
                    view.handle(
                        super::UiAction::FirstRun(super::first_run::Action::Skip),
                        window,
                        cx,
                    );
                });
            });
            assert!(
                view.read_with(cx, |view, _| view.router.tab() == super::Tab::Onboarding
                    && view.form.is_some()
                    && view.busy
                    && !view.cancel.is_cancelled())
            );
            assert_eq!(
                super::Preferences::load(root.path())?,
                super::Preferences::default()
            );
            Ok(())
        })();
        assert!(result.is_ok(), "{result:?}");
    }

    #[gpui::test]
    fn first_run_save_retry_does_not_create_a_second_game(cx: &mut gpui::TestAppContext) {
        let result = (|| -> anyhow::Result<()> {
            let root = tempfile::tempdir()?;
            let catalog = block_launcher::catalog::Catalog::open(root.path())?;
            let (instance, profile) =
                catalog.create_instance("First game".into(), "1.20.6".into(), None, None)?;
            let snapshot = catalog.snapshot()?;
            let (engine, _messages) = super::Engine::with_root(root.path().into())?;
            let (view, cx) = crate::test_support::window(cx, test_view);
            std::fs::create_dir(root.path().join(super::global_config::FILE))?;
            view.update(cx, |view, cx| {
                view.engine = Some(engine);
                view.begin_first_run(cx);
                view.snapshot = snapshot;
                view.selected_instance = Some(instance.id.clone());
                view.selected_profile = Some(profile.id.clone());
                assert!(!view.complete_onboarding());
                cx.notify();
            });
            assert!(!view.read_with(cx, |view, _| view.preferences.onboarding_complete));
            std::fs::remove_dir(root.path().join(super::global_config::FILE))?;
            view.update(cx, |view, _| assert!(view.complete_onboarding()));
            assert_eq!(
                view.read_with(cx, |view, _| view.router.tab()),
                super::Tab::Home
            );
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
        assert_eq!(
            view.read_with(cx, |view, _| view.router.tab()),
            super::Tab::Settings
        );
        cx.simulate_resize(gpui::size(gpui::px(960.), gpui::px(600.)));
        click_selector(cx, "settings-appearance");
        click_selector(cx, "select-choice-2");
        assert_eq!(
            view.read_with(cx, |view, _| view.preferences.appearance),
            super::Appearance::Dark
        );
        click_selector(cx, "reduce-motion");
        assert!(view.read_with(cx, |view, _| view.preferences.reduce_motion));
        click_selector(cx, "app-menu");
        click_selector(cx, "menu-home");
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
                view.router.navigate(super::Tab::Home);
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
                view.read_with(cx, |view, _| view.router.tab()),
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
    fn game_wizard_cards_scroll_without_hiding_the_footer(cx: &mut gpui::TestAppContext) {
        let (view, cx) = crate::test_support::window(cx, test_view);
        cx.simulate_resize(gpui::size(gpui::px(960.), gpui::px(600.)));
        view.update(cx, |view, _| view.preferences.reduce_motion = true);
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
        let viewport = cx.debug_bounds("wizard-body");
        let location = crate::test_support::control_bounds(cx, "wizard-component-location");
        assert!(viewport.is_some() && location.is_some());
        if let (Some(viewport), Some(location)) = (viewport, location)
            && location.bottom() > viewport.bottom() + gpui::px(1.)
        {
            view.update(cx, |view, cx| {
                let scroll = &view.shell.wizard.body_scroll;
                assert!(scroll.max_offset().y > gpui::px(0.));
                scroll.set_offset(gpui::point(gpui::px(0.), -scroll.max_offset().y));
                cx.notify();
            });
            cx.run_until_parked();
            cx.executor()
                .advance_clock(std::time::Duration::from_millis(32));
            cx.update(|window, _| window.refresh());
            cx.run_until_parked();
            let scrolled = crate::test_support::control_bounds(cx, "wizard-component-location");
            assert!(
                scrolled.is_some_and(|bounds| bounds.bottom() <= viewport.bottom()),
                "scrolled={scrolled:?} viewport={viewport:?}"
            );
            assert_eq!(
                cx.debug_bounds("wizard-footer")
                    .map(|bounds| bounds.bottom()),
                Some(footer.bottom()),
                "the footer does not scroll away"
            );
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
            let original = serde_json::to_value(catalog.snapshot()?)?;
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
            assert_eq!(serde_json::to_value(catalog.snapshot()?)?, original);
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
            let original = serde_json::to_value(catalog.snapshot()?)?;
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
            assert_eq!(serde_json::to_value(catalog.snapshot()?)?, original);
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
            assert_eq!(serde_json::to_value(catalog.snapshot()?)?, original);
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
        assert_eq!(
            view.read_with(cx, |view, _| view.router.tab()),
            super::Tab::Accounts
        );
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
