//! A native launcher with persistent installations, launch profiles and Microsoft accounts.

mod appearance;
mod crash_view;
mod engine;
mod game_wizard;
mod onboarding;
mod settings;
mod shell;
mod text_input;
mod window_geometry;

pub use crash_view::CrashView;

use std::{
    collections::{HashMap, VecDeque},
    fs::File,
    path::PathBuf,
    time::Duration,
};

use block_auth::Account;
use block_launcher::{
    CancellationToken,
    catalog::{DataPolicy, InstallStatus, Instance, LoaderKind, Profile, Snapshot},
    crash::CrashReport,
    recipe::{LaunchRecipe, RuntimeSelection},
    session_host::{self, RunRecord, RunState},
};
use gpui::{
    AppContext as _, Context, Entity, FocusHandle, Focusable as _, InteractiveElement as _,
    IntoElement, KeyBinding, ParentElement as _, PathPromptOptions, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, px, rgb,
};
use tokio::sync::mpsc::Receiver;

use appearance::{Appearance, Palette, Preferences};
use engine::{Engine, Message};
use game_wizard::{WizardPage, WizardStep};
use shell::{Drawer, ShellState};
use text_input::TextInput;

/// Explicit development recipe input. Normal startup uses the persisted library.
#[derive(Debug, Default)]
pub struct LaunchOptions {
    pub recipe: Option<PathBuf>,
    pub launch: bool,
    pub profile: Option<String>,
    pub data_dir: Option<PathBuf>,
    pub system_window_chrome: bool,
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
    Tab(Tab),
    Drawer(Option<Drawer>),
    SetAppearance(Appearance),
    ReduceMotion,
    SavePreferences,
    ResetPreferences,
    ConfirmSeparate(bool),
    SeparateProfiles,
    WizardNext,
    WizardBack,
    WizardPage(Option<WizardPage>),
    OnboardingNext,
    OnboardingBack,
    OnboardingSkip,
    OnboardingAccountLater,
    OnboardingOptions,
    SetFullscreen,
    ShowSnapshots,
    ProxyMode(u8),
    SettingsPage(u8),
    SelectInstance(String),
    SelectProfile(String),
    SelectAccount(String),
    CreateInstance,
    SaveInstance,
    ToggleLoader,
    BrowseInstall,
    SelectVersion(String),
    EditProfile,
    SaveProfile,
    CancelForm,
    NewProfile,
    CopyProfile,
    DeleteProfile,
    ConfirmDelete,
    TogglePolicy,
    ToggleRuntime,
    ProfileTarget(String),
    DefaultAccount(Option<String>),
    SignIn,
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
    neoforge: bool,
}

#[derive(Debug)]
struct ProfileForm {
    original: Profile,
    name: Entity<TextInput>,
    memory: Entity<TextInput>,
    java: Entity<TextInput>,
    jvm: Entity<TextInput>,
    game: Entity<TextInput>,
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
    Delete(String),
}

/// The GPUI view owns only public account metadata and editable settings.
#[derive(Debug)]
pub struct Block {
    shell: ShellState,
    pending_window_size: Option<gpui::Size<gpui::Pixels>>,
    onboarding_progress: onboarding::Progress,
    settings_form: Option<settings::SettingsDraft>,
    preferences: Preferences,
    saved_preferences: Preferences,
    palette: Palette,
    snapshot: Snapshot,
    accounts: Vec<Account>,
    versions: Vec<String>,
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
    busy: bool,
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

impl Drop for Block {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

impl Block {
    #[must_use]
    pub fn new(window: &mut Window, cx: &mut Context<'_, Self>, options: LaunchOptions) -> Self {
        text_input::bind_keys(cx);
        cx.bind_keys([
            KeyBinding::new("tab", NextField, Some("LauncherForm")),
            KeyBinding::new("shift-tab", PreviousField, Some("LauncherForm")),
        ]);
        let focus = cx.focus_handle();
        focus.focus(window);
        let engine = options
            .data_dir
            .clone()
            .map_or_else(Engine::new, Engine::with_root);
        let (engine, messages, status) = match engine {
            Ok((engine, messages)) => {
                engine.load_catalog();
                engine.load_versions();
                (Some(engine), Some(messages), "Welcome to Block".into())
            }
            Err(error) => (None, None, format!("Engine unavailable: {error:#}")),
        };
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
        }
        let mut view = Self {
            shell: ShellState {
                system_window_chrome: options.system_window_chrome,
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
            versions: vec!["1.20.6".into(), "1.20.1".into(), "26.3".into()],
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
            busy: false,
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
        };
        if tab == Tab::Onboarding {
            view.begin_onboarding(cx);
        }
        if let Some(path) = options.recipe {
            view.load_recipe(path, options.launch, cx);
        } else if options.launch {
            view.status = "Select a profile and Microsoft account before playing".into();
        }
        view
    }

    fn poll(&mut self, cx: &mut Context<'_, Self>) {
        let mut messages = Vec::new();
        if let Some(receiver) = &mut self.messages {
            while let Ok(message) = receiver.try_recv() {
                messages.push(message);
            }
        }
        if messages.is_empty() {
            return;
        }
        for message in messages {
            match message {
                Message::Catalog(Ok(snapshot)) => self.apply_catalog(snapshot),
                Message::Catalog(Err(error)) | Message::Accounts(Err(error)) => self.report(error),
                Message::Accounts(Ok(accounts)) => {
                    self.accounts = accounts;
                    if !self
                        .accounts
                        .iter()
                        .any(|a| Some(account_id(a)) == self.selected_account)
                    {
                        self.apply_profile_account();
                    }
                }
                Message::Versions(Ok(versions)) => {
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
                    self.push_log(format!("Version list unavailable: {error}"));
                }
                Message::Browser(url) => {
                    if !self.cancel.is_cancelled() {
                        cx.open_url(&url);
                    }
                }
                Message::Created { instance, profile } => {
                    self.selected_instance = Some(instance);
                    self.selected_profile = Some(profile);
                    if self.tab == Tab::Onboarding {
                        self.onboarding_progress = onboarding::Progress::Created;
                        self.complete_onboarding();
                    } else {
                        self.form = None;
                    }
                }
                Message::Status(status) => self.status = status,
                Message::Progress(count) => self.completed = count,
                Message::Command(command) => self.command = Some(command),
                Message::Log(line) => self.push_log(line),
                Message::Session(record) => self.session_updated(*record),
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
                    self.busy = self.active_run.is_some();
                    match result {
                        Ok(status) => {
                            self.status = status;
                            if self.tab != Tab::Onboarding {
                                self.form = None;
                            }
                        }
                        Err(_) if self.cancel.is_cancelled() => {
                            self.status = "Cancelled · retry when ready".into();
                        }
                        Err(error) => self.report(format!("Failed · {error}")),
                    }
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
        cx.notify();
    }

    fn apply_catalog(&mut self, snapshot: Snapshot) {
        let previous = self
            .profile()
            .map(|profile| (profile.id.clone(), profile.default_account.clone()));
        self.snapshot = snapshot;
        if let Some(profile) = self.pending_profile.take() {
            self.select_requested_profile(&profile);
        }
        self.normalize_selection();
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
            self.tab = Tab::Instances;
            self.form = None;
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
                        view.focus.focus(window);
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

    fn normalize_selection(&mut self) {
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

    fn report(&mut self, error: String) {
        if self.tab == Tab::Onboarding {
            self.shell.onboarding.error = Some(error.clone());
        }
        self.push_log(error.clone());
        self.status = error;
    }

    fn begin(&mut self, status: &str) {
        if self.tab == Tab::Onboarding {
            self.shell.onboarding.error = None;
        }
        if self.engine.is_none() {
            self.status =
                "Engine unavailable · restart Block after checking its data directory".into();
            return;
        }
        self.busy = true;
        self.completed = 0;
        self.command = None;
        self.show_command = false;
        self.logs.clear();
        self.status = status.into();
        self.cancel = CancellationToken::new();
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
        let retry_setup_save = matches!(action, UiAction::OnboardingNext)
            && self.tab == Tab::Onboarding
            && self.onboarding_progress == onboarding::Progress::Created;
        if self.busy
            && !retry_setup_save
            && !matches!(
                action,
                UiAction::Stop
                    | UiAction::OnboardingSkip
                    | UiAction::Tab(_)
                    | UiAction::Drawer(_)
                    | UiAction::SetAppearance(_)
                    | UiAction::ReduceMotion
                    | UiAction::SavePreferences
                    | UiAction::ResetPreferences
                    | UiAction::ToggleOutput
                    | UiAction::ToggleCommand
            )
        {
            return;
        }
        let focus_first = matches!(action, UiAction::CreateInstance | UiAction::EditProfile);
        let previous_page = (self.tab, self.form.is_some());
        match action {
            UiAction::OnboardingAccountLater => self.advance_onboarding(cx),
            UiAction::OnboardingOptions => {
                self.shell.onboarding.options = !self.shell.onboarding.options;
            }
            UiAction::OnboardingNext => {
                self.advance_onboarding(cx);
                self.focus_field(false, window, cx);
            }
            UiAction::OnboardingBack => {
                self.back_onboarding();
                self.focus_field(false, window, cx);
            }
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
                if self.tab == Tab::Settings && tab != Tab::Settings {
                    self.preferences = self.saved_preferences.clone();
                    self.settings_form = None;
                }
                if tab == Tab::Settings && self.tab != Tab::Settings {
                    self.open_settings(cx);
                }
                self.tab = tab;
                self.form = None;
                self.shell.drawer = None;
                window.resize(if tab == Tab::Home {
                    window_geometry::home_size(window, cx)
                } else {
                    gpui::size(px(960.), px(600.))
                });
            }
            UiAction::Drawer(drawer) => {
                if drawer.is_some() {
                    self.shell.return_focus = window.focused(cx);
                    let focus = cx.focus_handle();
                    focus.focus(window);
                    self.shell.drawer_focus = Some(focus);
                }
                self.shell.drawer = drawer;
                self.shell.drawer_revision = self.shell.drawer_revision.wrapping_add(1);
            }
            UiAction::SetAppearance(appearance) => self.preferences.appearance = appearance,
            UiAction::ReduceMotion => {
                self.preferences.reduce_motion = !self.preferences.reduce_motion;
            }
            UiAction::ResetPreferences => self.reset_settings(cx),
            UiAction::SavePreferences => self.save_settings(cx),
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
                if let Some(form) = &mut self.settings_form {
                    form.page = page;
                }
            }
            UiAction::ProxyMode(mode) => {
                if let Some(form) = &mut self.settings_form {
                    form.proxy_mode = mode;
                }
            }
            UiAction::ConfirmSeparate(confirm) => self.shell.confirm_separation = confirm,
            UiAction::SeparateProfiles => {
                self.shell.confirm_separation = false;
                self.begin("Enabling separated profiles");
                if let Some(engine) = &self.engine {
                    engine.separate_profiles();
                }
            }
            UiAction::SelectInstance(id) => {
                self.selected_instance = Some(id);
                self.selected_profile = None;
                self.form = None;
                self.normalize_selection();
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
                self.shell.drawer = None;
                self.form = None;
                self.apply_profile_account();
            }
            UiAction::SelectAccount(id) => {
                self.selected_account = Some(id);
                self.shell.drawer = None;
            }
            UiAction::CreateInstance => {
                self.shell.wizard = game_wizard::WizardState::default();
                self.tab = Tab::Instances;
                self.form = Some(Form::Instance(InstanceForm {
                    name: input("My Minecraft", "Instance name", cx),
                    version: input("1.20.6", "Minecraft version", cx),
                    directory: input("", "Automatic installation location", cx),
                    neoforge: false,
                }));
            }
            UiAction::WizardNext => {
                if self.validate_instance_draft(cx) {
                    self.shell.wizard.next();
                    self.focus_field(false, window, cx);
                }
            }
            UiAction::WizardBack => self.shell.wizard.back(),
            UiAction::WizardPage(page) => {
                self.shell.wizard.open_page(page);
                self.focus_field(false, window, cx);
            }
            UiAction::ToggleLoader => {
                if let Some(Form::Instance(form)) = &mut self.form {
                    form.neoforge = !form.neoforge;
                    if form.neoforge {
                        form.version
                            .update(cx, |field, cx| field.set_value("1.20.6", cx));
                    }
                }
            }
            UiAction::SelectVersion(version) => {
                if let Some(Form::Instance(form)) = &mut self.form {
                    form.version
                        .update(cx, |field, cx| field.set_value(&version, cx));
                    if version != "1.20.6" {
                        form.neoforge = false;
                    }
                }
            }
            UiAction::BrowseInstall => self.browse_install(cx),
            UiAction::SaveInstance => self.save_instance(cx),
            UiAction::EditProfile => self.edit_profile(cx),
            UiAction::SaveProfile => self.save_profile(cx),
            UiAction::CancelForm => {
                if self.tab != Tab::Onboarding {
                    self.form = None;
                }
            }
            UiAction::NewProfile => {
                if let Some(id) = self.selected_instance.clone() {
                    self.begin("Creating profile");
                    self.selected_profile = None;
                    if let Some(engine) = &self.engine {
                        engine.create_profile(id, "New profile".into());
                    }
                }
            }
            UiAction::CopyProfile => {
                if let Some(profile) = self.profile().cloned() {
                    self.begin("Copying profile settings");
                    self.selected_profile = None;
                    if let Some(engine) = &self.engine {
                        engine.copy_profile(profile.id, format!("{} copy", profile.name));
                    }
                }
            }
            UiAction::DeleteProfile => {
                if let Some(id) = self.selected_profile.clone() {
                    self.form = Some(Form::Delete(id));
                }
            }
            UiAction::ConfirmDelete => {
                if let Some(Form::Delete(id)) = self.form.take() {
                    self.begin("Deleting profile metadata");
                    if let Some(engine) = &self.engine {
                        engine.delete_profile(id);
                    }
                }
            }
            UiAction::TogglePolicy => {
                if let Some(Form::Profile(form)) = &mut self.form {
                    form.original.data_policy = match form.original.data_policy {
                        DataPolicy::Shared => DataPolicy::Isolated,
                        DataPolicy::Isolated => DataPolicy::Shared,
                    };
                }
            }
            UiAction::ToggleRuntime => {
                if let Some(Form::Profile(form)) = &mut self.form {
                    form.managed = !form.managed;
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
            UiAction::SignIn => {
                self.begin("Opening Microsoft sign-in");
                if let Some(engine) = &self.engine {
                    engine.sign_in(self.cancel.clone());
                }
            }
            UiAction::SignOut(id) => {
                if let Some(account) = self.accounts.iter().find(|a| account_id(a) == id).cloned() {
                    self.begin("Removing saved credentials");
                    if let Some(engine) = &self.engine {
                        engine.sign_out(account);
                    }
                }
            }
            UiAction::Play => self.play(),
            UiAction::Repair => {
                if let Some(profile) = self.profile().cloned() {
                    self.begin("Repairing installation · verifying shared resources");
                    if let Some(engine) = &self.engine {
                        engine.install(profile.instance_id, profile.target_id, self.cancel.clone());
                    }
                }
            }
            UiAction::Stop => {
                if self.busy {
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
            if self.form.is_some() {
                window.resize(gpui::size(px(960.), px(600.)));
            }
            self.shell.backwards = self.tab == Tab::Home || self.form.is_none() && previous_page.1;
            self.shell.revision = self.shell.revision.wrapping_add(1);
        }
        if let Some(focus) = self
            .shell
            .drawer
            .as_ref()
            .and(self.shell.drawer_focus.as_ref())
        {
            focus.focus(window);
        } else if self.form.is_none() {
            self.shell
                .return_focus
                .take()
                .unwrap_or_else(|| self.focus.clone())
                .focus(window);
        } else if focus_first {
            self.focus_field(false, window, cx);
        }
        cx.notify();
    }

    fn move_focus(&self, backwards: bool, window: &mut Window, cx: &Context<'_, Self>) {
        if backwards {
            window.focus_prev();
        } else {
            window.focus_next();
        }
        if let Some(focus) = self
            .shell
            .drawer
            .as_ref()
            .and(self.shell.drawer_focus.as_ref())
            && !focus.contains_focused(window, cx)
        {
            focus.focus(window);
            if backwards {
                window.focus_prev();
            } else {
                window.focus_next();
            }
        }
    }

    fn focus_field(&self, backwards: bool, window: &mut Window, cx: &Context<'_, Self>) {
        if self.tab == Tab::Onboarding && self.shell.onboarding.step != onboarding::Step::Game {
            if backwards {
                window.focus_prev();
            } else {
                window.focus_next();
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
                    fields[next].read(cx).focus_handle(cx).focus(window);
                } else if let Some(first) = fields.first() {
                    first.read(cx).focus_handle(cx).focus(window);
                }
            }
            return;
        }
        let fields = match &self.form {
            Some(Form::Instance(form)) => match self.shell.wizard.page {
                Some(WizardPage::Version) => vec![&form.version],
                Some(WizardPage::Location) => vec![&form.directory],
                None if self.shell.wizard.step == WizardStep::Setup => vec![&form.name],
                Some(WizardPage::Loader) | None => Vec::new(),
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
                    window.focus_prev();
                } else {
                    window.focus_next();
                }
                return;
            }
        };
        if fields.is_empty() {
            if backwards {
                window.focus_prev();
            } else {
                window.focus_next();
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
        fields[index].read(cx).focus_handle(cx).focus(window);
    }

    fn validate_instance_draft(&mut self, cx: &Context<'_, Self>) -> bool {
        let Some(Form::Instance(form)) = &self.form else {
            return false;
        };
        let directory = form.directory.read(cx).value();
        let message = if form.name.read(cx).value().trim().is_empty() {
            Some("Enter a game name")
        } else if form.version.read(cx).value().trim().is_empty() {
            Some("Choose a Minecraft version")
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

    fn save_instance(&mut self, cx: &Context<'_, Self>) {
        let Some(Form::Instance(form)) = &self.form else {
            return;
        };
        let name = form.name.read(cx).value();
        let version = form.version.read(cx).value();
        let directory = form.directory.read(cx).value();
        let neoforge = form.neoforge;
        if name.trim().is_empty() || version.trim().is_empty() {
            self.report("Enter an instance name and Minecraft version".into());
            return;
        }
        if neoforge && version != "1.20.6" {
            self.report("NeoForge 20.6.144 requires Minecraft 1.20.6".into());
            return;
        }
        let directory = (!directory.trim().is_empty()).then(|| PathBuf::from(directory.trim()));
        if directory.as_ref().is_some_and(|path| !path.is_absolute()) {
            self.report("Choose an absolute installation directory".into());
            return;
        }
        self.begin("Installing · resolving Minecraft, Java and native libraries");
        self.selected_instance = None;
        self.selected_profile = None;
        if let Some(engine) = &self.engine {
            engine.create_instance(
                name,
                version,
                directory,
                neoforge,
                self.saved_preferences.game.clone(),
                self.cancel.clone(),
            );
        }
    }

    fn edit_profile(&mut self, cx: &mut Context<'_, Self>) {
        let Some(profile) = self.profile().cloned() else {
            return;
        };
        let java = match &profile.runtime {
            RuntimeSelection::Managed => String::new(),
            RuntimeSelection::Local { java } => java.display().to_string(),
        };
        self.form = Some(Form::Profile(Box::new(ProfileForm {
            name: input(&profile.name, "Profile name", cx),
            memory: input(&profile.memory_mib.to_string(), "2048", cx),
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
            managed: matches!(profile.runtime, RuntimeSelection::Managed),
            original: profile,
        })));
    }

    fn save_profile(&mut self, cx: &Context<'_, Self>) {
        let Some(Form::Profile(form)) = &self.form else {
            return;
        };
        let mut profile = form.original.clone();
        profile.name = form.name.read(cx).value();
        let result = (|| -> anyhow::Result<()> {
            profile.memory_mib = form.memory.read(cx).value().trim().parse().map_err(|_| {
                anyhow::anyhow!("Memory must be a number between 256 and 65536 MiB")
            })?;
            profile.jvm_args = parse_arguments(&form.jvm.read(cx).value())?;
            profile.game_args = parse_arguments(&form.game.read(cx).value())?;
            profile.runtime = if form.managed {
                RuntimeSelection::Managed
            } else {
                RuntimeSelection::Local {
                    java: PathBuf::from(form.java.read(cx).value()),
                }
            };
            profile.validate()?;
            Ok(())
        })();
        if let Err(error) = result {
            self.report(error.to_string());
            return;
        }
        self.begin("Saving profile");
        if let Some(engine) = &self.engine {
            engine.save_profile(profile);
        }
    }

    fn play(&mut self) {
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
                "Sign in with Microsoft and select an account before playing".into()
            };
            return;
        };
        self.begin("Preparing · validating profile and account");
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
            self.begin("Preparing explicit development recipe");
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
    ) -> gpui::Stateful<gpui::Div> {
        let id = id.into();
        let selector = id.to_string();
        div()
            .id(id)
            .debug_selector(move || selector)
            .tab_index(0)
            .focus(|style| style.border_color(rgb(0x0080_8080)))
            .on_key_down({
                let action = action.clone();
                cx.listener(move |view, event: &gpui::KeyDownEvent, window, cx| {
                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                        view.handle(action.clone(), window, cx);
                        cx.stop_propagation();
                    }
                })
            })
            .px_3()
            .py_2()
            .rounded_md()
            .bg(rgb(self.palette.surface))
            .border_1()
            .border_color(rgb(self.palette.border))
            .text_sm()
            .cursor_pointer()
            .hover(|style| style.opacity(0.75))
            .on_click(
                cx.listener(move |view, _, window, cx| view.handle(action.clone(), window, cx)),
            )
            .child(label.into())
    }

    fn welcome(&self, cx: &Context<'_, Self>) -> impl IntoElement + use<> {
        panel().gap_4().child(div().text_2xl().child("Your next world starts here"))
            .child(note("Sign in with Microsoft, create an installation, and choose a profile to play."))
            .child(div().flex().flex_wrap().gap_2()
                .child(self.button("welcome-sign-in", "Sign in with Microsoft", UiAction::SignIn, cx))
                .child(self.button("welcome-create", "Create an instance", UiAction::CreateInstance, cx)))
            .child(note("Java, libraries and assets are installed automatically. Your account credentials stay in the operating system's secure credential store."))
    }

    fn accounts_page(&self, cx: &Context<'_, Self>) -> impl IntoElement + use<> {
        panel().gap_4().child(div().text_2xl().child("Accounts"))
            .child(note("Sign in through your system browser. Java Edition ownership is checked before an account can play. Saved accounts are checked and refreshed when you launch."))
            .child(self.button("sign-in", "Sign in with Microsoft", UiAction::SignIn, cx))
            .children(self.accounts.iter().map(|account| {
                let id = account_id(account);
                panel().gap_2().bg(rgb(self.palette.surface)).child(div().text_lg().child(account.player_name.clone()))
                    .child(note(format!("Microsoft · {}", account.reference.player_id)))
                    .child(note("Ownership verified at sign-in · credentials in system secure storage"))
                    .child(div().flex().flex_wrap().gap_2()
                        .child(self.button(format!("account-select-{id}"), if Some(&id) == self.selected_account.as_ref() { "Selected" } else { "Use this account" }, UiAction::SelectAccount(id.clone()), cx))
                        .child(self.button(format!("account-logout-{id}"), "Sign out", UiAction::SignOut(id), cx)))
            }))
            .child(if self.accounts.is_empty() { "No saved accounts. Sign in to play." } else { "Signing out removes credentials and leaves your instances and worlds intact." })
    }

    fn instance_page(&self, cx: &Context<'_, Self>) -> impl IntoElement + use<> {
        let Some(instance) = self.instance() else {
            return self.welcome(cx).into_any_element();
        };
        panel()
            .gap_4()
            .child(div().text_2xl().child(instance.name.clone()))
            .child(note(format!(
                "Minecraft {} · {}",
                instance.minecraft_version,
                instance.directory.display()
            )))
            .children(instance.targets.iter().map(|target| {
                let kind = match target.kind {
                    LoaderKind::Vanilla => "Vanilla".into(),
                    LoaderKind::NeoForge => format!(
                        "NeoForge {}",
                        target.loader_version.as_deref().unwrap_or("")
                    ),
                };
                note(format!("{kind} · {}", install_status(&target.status)))
            }))
            .child(self.profile_choices(cx))
            .child(self.launch_controls(cx))
            .into_any_element()
    }

    fn profile_choices(&self, cx: &Context<'_, Self>) -> impl IntoElement + use<> {
        panel()
            .gap_2()
            .bg(rgb(self.palette.surface))
            .child(div().text_sm().child("Profiles"))
            .children(
                self.snapshot
                    .profiles
                    .iter()
                    .filter(|p| Some(&p.instance_id) == self.selected_instance.as_ref())
                    .map(|profile| {
                        let label = format!(
                            "{}{} · {}",
                            if Some(&profile.id) == self.selected_profile.as_ref() {
                                "● "
                            } else {
                                ""
                            },
                            profile.name,
                            profile.target_id
                        );
                        self.button(
                            format!("profile-{}", profile.id),
                            label,
                            UiAction::SelectProfile(profile.id.clone()),
                            cx,
                        )
                    }),
            )
            .child({
                let mut actions = div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(self.button("edit-profile", "Edit", UiAction::EditProfile, cx))
                    .child(self.button("delete-profile", "Delete", UiAction::DeleteProfile, cx));
                if self.snapshot.profile_mode == block_launcher::catalog::ProfileMode::Separated {
                    actions = actions
                        .child(self.button(
                            "new-profile",
                            "+ New profile",
                            UiAction::NewProfile,
                            cx,
                        ))
                        .child(self.button(
                            "copy-profile",
                            "Copy settings",
                            UiAction::CopyProfile,
                            cx,
                        ));
                }
                actions
            })
    }

    fn profiles_page(&self, cx: &Context<'_, Self>) -> impl IntoElement + use<> {
        let mut content = panel().gap_4().child(div().text_2xl().child("Launch profiles"))
            .child(note("Profiles share an installation and keep independent launch settings. Select an installation in the sidebar."))
            .child(self.profile_choices(cx));
        if let Some(profile) = self.profile() {
            content = content.child(note(format!(
                "{} MiB · {} · {:?}",
                profile.memory_mib,
                match profile.runtime {
                    RuntimeSelection::Managed => "Automatic Java",
                    RuntimeSelection::Local { .. } => "Custom Java",
                },
                profile.data_policy
            )));
            if let Some(instance) = self.instance()
                && let Ok(directory) = profile.game_directory(instance)
            {
                content = content.child(note(format!("Game data: {}", directory.display())));
            }
        }
        content.child(self.launch_controls(cx))
    }

    fn launch_controls(&self, cx: &Context<'_, Self>) -> impl IntoElement + use<> {
        panel()
            .gap_3()
            .bg(rgb(self.palette.surface))
            .child(div().text_sm().child(format!(
                    "Account: {}",
                    self.accounts
                        .iter()
                        .find(|a| Some(account_id(a)) == self.selected_account)
                        .map_or("Sign in to play", |a| a.player_name.as_str())
                )))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(self.button(
                        "play",
                        if self.busy {
                            "Preparing / running…"
                        } else {
                            "Play"
                        },
                        UiAction::Play,
                        cx,
                    ))
                    .child(self.button("stop", "Stop", UiAction::Stop, cx))
                    .child(self.button("repair", "Repair", UiAction::Repair, cx))
                    .child(self.button(
                        "choose-account",
                        "Choose account",
                        UiAction::Tab(Tab::Accounts),
                        cx,
                    )),
            )
    }

    fn instance_form(
        &self,
        form: &InstanceForm,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement + use<> {
        self.render_game_wizard(form, cx)
    }

    fn profile_form(&self, form: &ProfileForm, cx: &Context<'_, Self>) -> impl IntoElement + use<> {
        let mut content = panel()
            .gap_3()
            .child(div().text_2xl().child("Edit profile"))
            .child(field("Name", &form.name))
            .child(div().text_sm().child("Launch target"));
        if let Some(instance) = self.instance() {
            content = content.children(instance.targets.iter().map(|target| {
                self.button(
                    format!("target-{}", target.id),
                    format!(
                        "{}{}",
                        if target.id == form.original.target_id {
                            "● "
                        } else {
                            ""
                        },
                        target.id
                    ),
                    UiAction::ProfileTarget(target.id.clone()),
                    cx,
                )
            }));
            if let Ok(path) = form.original.game_directory(instance) {
                content = content.child(note(format!("Game data: {}", path.display())));
            }
        }
        content = content.child(self.button("data-policy", format!("Data: {:?} · click to change", form.original.data_policy), UiAction::TogglePolicy, cx))
            .child(note("Isolated keeps saves, mods, settings and logs in this profile's directory. Shared uses the instance's game directory; only one game may write to it at a time. Changing this setting selects a different directory and keeps all existing files."))
            .child(self.button("runtime-policy", if form.managed { "Java: automatic · click for custom" } else { "Java: custom · click for automatic" }, UiAction::ToggleRuntime, cx));
        if !form.managed {
            content = content.child(field("Java executable", &form.java));
        }
        content = content.child(field("Maximum memory (MiB)", &form.memory))
            .child(field("Additional JVM arguments", &form.jvm)).child(field("Additional game arguments", &form.game))
            .child(note("Optional arguments are separated by spaces; use double quotes around values containing spaces."))
            .child(div().text_sm().child("Default account"))
            .child(self.button("default-account-none", if form.original.default_account.is_none() { "● Use current account" } else { "Use current account" }, UiAction::DefaultAccount(None), cx))
            .children(self.accounts.iter().map(|account| {
                let id = account_id(account);
                self.button(format!("default-account-{id}"), format!("{}{}", if form.original.default_account.as_deref() == Some(&id) { "● " } else { "" }, account.player_name), UiAction::DefaultAccount(Some(id)), cx)
            }))
            .child(div().flex().flex_wrap().gap_2().child(self.button("save-profile", "Save profile", UiAction::SaveProfile, cx))
                .child(self.button("cancel-profile", "Cancel", UiAction::CancelForm, cx)));
        content
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
                    .overflow_y_scroll()
                    .bg(rgb(self.palette.surface))
                    .p_3()
                    .text_xs()
                    .children(lines.into_iter().map(|line| div().mb_1().child(line))),
            )
    }
}

impl Render for Block {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        self.prepare_crash_help(window, cx);
        if let Some(help) = &self.crash_help {
            return div().size_full().child(help.clone()).into_any_element();
        }
        if let Some(size) = self.pending_window_size.take() {
            window.resize(if self.tab == Tab::Home {
                window_geometry::home_size(window, cx)
            } else {
                size
            });
        }
        self.palette = self.preferences.palette(window.appearance());
        cx.set_global(self.palette);
        self.render_shell(window, cx).into_any_element()
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
fn note(text: impl Into<SharedString>) -> gpui::Div {
    div()
        .text_xs()
        .text_color(rgb(0x0080_8080))
        .child(text.into())
}
fn field(label: &'static str, input: &Entity<TextInput>) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(div().text_sm().child(label))
        .child(input.clone())
}
fn input(value: &str, placeholder: &str, cx: &mut Context<'_, Block>) -> Entity<TextInput> {
    cx.new(|cx| TextInput::new(value, placeholder, cx))
}
fn account_id(account: &Account) -> String {
    account.reference.player_id.to_string()
}
fn install_status(status: &InstallStatus) -> String {
    match status {
        InstallStatus::Pending => "Not installed · Repair to install".into(),
        InstallStatus::Installing => "Installing".into(),
        InstallStatus::Ready => "Ready".into(),
        InstallStatus::Failed { message } => format!("Failed · {message} · Repair to retry"),
    }
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
        let focus = cx.focus_handle();
        focus.focus(window);
        super::text_input::bind_keys(cx);
        cx.bind_keys([gpui::KeyBinding::new(
            "tab",
            super::NextField,
            Some("LauncherForm"),
        )]);
        super::Block {
            shell: super::ShellState::default(),
            pending_window_size: None,
            onboarding_progress: super::onboarding::Progress::Draft,
            settings_form: None,
            preferences: super::Preferences::default(),
            saved_preferences: super::Preferences::default(),
            palette: super::Palette::LIGHT,
            snapshot: block_launcher::catalog::Snapshot::default(),
            accounts: Vec::new(),
            versions: vec!["1.20.6".into()],
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
            busy: false,
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
        }
    }

    fn click_selector(cx: &mut gpui::VisualTestContext, selector: &'static str) {
        cx.run_until_parked();
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(500));
        cx.run_until_parked();
        let bounds = cx.debug_bounds(selector);
        assert!(bounds.is_some(), "missing selector: {selector}");
        if let Some(bounds) = bounds {
            cx.simulate_click(bounds.center(), gpui::Modifiers::none());
        }
    }

    #[gpui::test]
    fn first_run_onboarding_saves_only_on_completion_and_default_reset_keeps_it(
        cx: &mut gpui::TestAppContext,
    ) {
        let result = (|| -> anyhow::Result<()> {
            let root = tempfile::tempdir()?;
            let (engine, _messages) = super::Engine::with_root(root.path().into())?;
            let (view, cx) = cx.add_window_view(test_view);
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
            let (view, cx) = cx.add_window_view(test_view);
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
            let (view, cx) = cx.add_window_view(test_view);
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
            assert!(cx.debug_bounds("welcome-back").is_none());
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
        let (view, cx) = cx.add_window_view(test_view);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.handle(super::UiAction::Tab(super::Tab::Home), window, cx);
            });
        });
        cx.run_until_parked();
        cx.simulate_resize(gpui::size(gpui::px(432.), gpui::px(768.)));
        click_selector(cx, "app-menu");
        assert_eq!(
            view.read_with(cx, |view, _| view.shell.drawer),
            Some(super::Drawer::Menu)
        );
        click_selector(cx, "menu-settings");
        assert_eq!(view.read_with(cx, |view, _| view.tab), super::Tab::Settings);
        cx.simulate_resize(gpui::size(gpui::px(960.), gpui::px(600.)));
        click_selector(cx, "appearance-dark");
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
            view.read_with(cx, |view, _| view.shell.drawer),
            Some(super::Drawer::Profiles)
        );
        cx.simulate_keystrokes("escape");
        assert!(view.read_with(cx, |view, _| view.shell.drawer.is_none()));
        assert!(!view.read_with(cx, |view, _| view.busy));
    }

    #[gpui::test]
    fn native_menu_is_keyboard_operable_without_replaying_page_animation(
        cx: &mut gpui::TestAppContext,
    ) {
        let (view, cx) = cx.add_window_view(test_view);
        let revision = view.read_with(cx, |view, _| view.shell.revision);
        cx.simulate_keystrokes("tab enter");
        assert_eq!(
            view.read_with(cx, |view, _| view.shell.drawer),
            Some(super::Drawer::Menu)
        );
        assert_eq!(view.read_with(cx, |view, _| view.shell.revision), revision);
        cx.simulate_keystrokes("escape");
        assert!(view.read_with(cx, |view, _| view.shell.drawer.is_none()));
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
        let (view, cx) = cx.add_window_view(test_view);
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
        let (view, cx) = cx.add_window_view(test_view);
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
            let (view, cx) = cx.add_window_view(test_view);
            view.update(cx, |view, cx| {
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
            let (view, cx) = cx.add_window_view(test_view);
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
            cx.simulate_keystrokes("enter");
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
        let (view, cx) = cx.add_window_view(test_view);
        let bounds = cx.debug_bounds("create-instance");
        assert!(bounds.is_some());
        let Some(bounds) = bounds else {
            return;
        };
        cx.simulate_click(bounds.center(), gpui::Modifiers::none());
        cx.simulate_resize(gpui::size(gpui::px(1100.), gpui::px(1300.)));
        cx.simulate_keystrokes("cmd-a");
        cx.simulate_input("世界 🧱");
        assert_eq!(
            view.read_with(cx, |view, cx| match &view.form {
                Some(super::Form::Instance(form)) => form.name.read(cx).value(),
                _ => String::new(),
            }),
            "世界 🧱"
        );
        click_selector(cx, "wizard-component-version");
        cx.simulate_keystrokes("cmd-a");
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
        click_selector(cx, "wizard-loader-neoforge");
        assert!(view.read_with(cx, |view, cx| matches!(&view.form, Some(super::Form::Instance(form)) if form.neoforge && form.version.read(cx).value() == "1.20.6")));
        assert!(!view.read_with(cx, |view, _| view.busy));
    }

    #[gpui::test]
    fn game_wizard_four_square_tiles_and_footer_fit_default_window(cx: &mut gpui::TestAppContext) {
        let (view, cx) = cx.add_window_view(test_view);
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
            "wizard-unavailable-optifine",
            "wizard-component-location",
        ] {
            let bounds = cx.debug_bounds(selector);
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
            let (view, cx) = cx.add_window_view(test_view);
            view.update(cx, |view, cx| {
                view.engine = Some(engine);
                view.messages = Some(messages);
                view.apply_catalog(snapshot);
                cx.notify();
            });
            click_selector(cx, "create-instance");
            cx.simulate_resize(gpui::size(gpui::px(1100.), gpui::px(1300.)));
            cx.simulate_keystrokes("cmd-a");
            cx.simulate_input("世界 draft");
            click_selector(cx, "wizard-component-version");
            cx.simulate_keystrokes("cmd-a");
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
            let (view, cx) = cx.add_window_view(test_view);
            view.update(cx, |view, cx| {
                view.engine = Some(engine);
                view.messages = Some(messages);
                view.apply_catalog(snapshot);
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
        let (view, cx) = cx.add_window_view(test_view);
        view.update(cx, |view, _| {
            view.selected_profile = Some("profile".into());
            view.play();
        });
        assert_eq!(view.read_with(cx, |view, _| view.tab), super::Tab::Accounts);
        assert!(!view.read_with(cx, |view, _| view.busy));
        assert!(view.read_with(cx, |view, _| view.status.contains("Sign in with Microsoft")));
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
        let (view, cx) = cx.add_window_view(test_view);
        cx.simulate_resize(gpui::size(gpui::px(1300.), gpui::px(1600.)));
        view.update(cx, |view, cx| {
            view.snapshot.instances.push(Instance {
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
        cx.simulate_click(bounds.center(), gpui::Modifiers::none());
        view.update(cx, |view, cx| {
            if let Some(super::Form::Profile(form)) = &view.form {
                form.memory
                    .update(cx, |field, cx| field.set_value("invalid", cx));
            }
        });
        let bounds = cx.debug_bounds("save-profile");
        assert!(bounds.is_some());
        let Some(bounds) = bounds else {
            return;
        };
        cx.simulate_click(bounds.center(), gpui::Modifiers::none());
        assert!(view.read_with(cx, |view, _| {
            view.status.contains("Memory must be a number")
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
            };
            let mut account_b = account_a.clone();
            account_b.reference.player_id = "00000000-0000-4000-8000-000000000002".parse()?;
            account_b.player_name = "AccountB".into();
            account_b.credential_ref = "synthetic-b".into();
            first_profile.default_account = Some(super::account_id(&account_a));
            second_profile.default_account = Some("missing-account".into());
            catalog.save_profile(first_profile.clone())?;
            catalog.save_profile(second_profile)?;
            let snapshot = catalog.snapshot()?;
            let (view, cx) = cx.add_window_view(test_view);
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
                    view.handle(super::UiAction::SelectInstance(second.id), window, cx);
                });
            });
            assert_eq!(
                view.read_with(cx, |view, _| view.selected_account.clone()),
                None
            );
            view.update(cx, |view, _| view.play());
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
                    view.handle(super::UiAction::SelectInstance(first.id), window, cx);
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
    fn cancelling_sign_in_discards_an_already_queued_browser_request(
        cx: &mut gpui::TestAppContext,
    ) {
        let (view, cx) = cx.add_window_view(test_view);
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
        assert_eq!(
            cx.opened_url().as_deref(),
            Some("https://example.invalid/new-synthetic-authorization")
        );
    }
}
