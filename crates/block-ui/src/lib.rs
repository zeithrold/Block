//! A native launcher with persistent installations, launch profiles and Microsoft accounts.

mod engine;
mod text_input;

use std::{collections::VecDeque, path::PathBuf, time::Duration};

use block_auth::Account;
use block_launcher::{
    CancellationToken,
    catalog::{DataPolicy, InstallStatus, Instance, LoaderKind, Profile, Snapshot},
    recipe::{LaunchRecipe, RecipeSource, RuntimeSelection},
};
use gpui::{
    AppContext as _, Context, Entity, FocusHandle, Focusable as _, InteractiveElement as _,
    IntoElement, KeyBinding, ParentElement as _, PathPromptOptions, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, px, rgb,
};
use tokio::sync::mpsc::Receiver;

use engine::{Engine, Message};
use text_input::TextInput;

/// Explicit development recipe input. Normal startup uses the persisted library.
#[derive(Debug, Default)]
pub struct LaunchOptions {
    pub recipe: Option<PathBuf>,
    pub launch: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Tab {
    #[default]
    Instances,
    Profiles,
    Accounts,
}

#[derive(Clone, Debug)]
enum UiAction {
    Tab(Tab),
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
enum Form {
    Instance(InstanceForm),
    Profile(Box<ProfileForm>),
    Delete(String),
}

/// The GPUI view owns only public account metadata and editable settings.
#[derive(Debug)]
pub struct Block {
    snapshot: Snapshot,
    accounts: Vec<Account>,
    versions: Vec<String>,
    selected_instance: Option<String>,
    selected_profile: Option<String>,
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
        let (engine, messages, status) = match Engine::new() {
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
        let mut view = Self {
            snapshot: Snapshot::default(),
            accounts: Vec::new(),
            versions: vec!["1.20.6".into(), "1.20.1".into(), "26.3".into()],
            selected_instance: None,
            selected_profile: None,
            selected_account: None,
            tab: Tab::Instances,
            form: None,
            engine,
            messages,
            focus,
            status,
            completed: 0,
            busy: false,
            cancel: CancellationToken::new(),
            logs: VecDeque::new(),
            show_output: false,
            command: None,
            show_command: false,
            recipe: None,
            recipe_load: RecipeLoadState::default(),
        };
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
                Message::Catalog(Ok(snapshot)) => {
                    let previous = self
                        .profile()
                        .map(|profile| (profile.id.clone(), profile.default_account.clone()));
                    self.snapshot = snapshot;
                    self.normalize_selection();
                    if previous
                        != self
                            .profile()
                            .map(|profile| (profile.id.clone(), profile.default_account.clone()))
                    {
                        self.apply_profile_account();
                    }
                }
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
                        .filter(|version| format!("{:?}", version.kind) == "Release")
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
                    self.form = None;
                }
                Message::Status(status) => self.status = status,
                Message::Progress(count) => self.completed = count,
                Message::Command(command) => self.command = Some(command),
                Message::Log(line) => self.push_log(line),
                Message::Finished(result) => {
                    self.busy = false;
                    match result {
                        Ok(status) => {
                            self.status = status;
                            self.form = None;
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
        self.push_log(error.clone());
        self.status = error;
    }

    fn begin(&mut self, status: &str) {
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
        if self.busy
            && !matches!(
                action,
                UiAction::Stop
                    | UiAction::Tab(_)
                    | UiAction::ToggleOutput
                    | UiAction::ToggleCommand
            )
        {
            return;
        }
        let focus_first = matches!(action, UiAction::CreateInstance | UiAction::EditProfile);
        match action {
            UiAction::Tab(tab) => {
                self.tab = tab;
                self.form = None;
            }
            UiAction::SelectInstance(id) => {
                self.selected_instance = Some(id);
                self.selected_profile = None;
                self.form = None;
                self.normalize_selection();
            }
            UiAction::SelectProfile(id) => {
                self.selected_profile = Some(id);
                self.form = None;
                self.apply_profile_account();
            }
            UiAction::SelectAccount(id) => self.selected_account = Some(id),
            UiAction::CreateInstance => {
                self.tab = Tab::Instances;
                self.form = Some(Form::Instance(InstanceForm {
                    name: input("My Minecraft", "Instance name", cx),
                    version: input("1.20.6", "Minecraft version", cx),
                    directory: input("", "Automatic installation location", cx),
                    neoforge: false,
                }));
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
            UiAction::CancelForm => self.form = None,
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
                    self.cancel.cancel();
                    self.status = "Stopping · waiting for the current operation".into();
                }
            }
            UiAction::ToggleOutput => self.show_output = !self.show_output,
            UiAction::ToggleCommand => self.show_command = !self.show_command,
            UiAction::OpenRecipe => self.open_recipe(cx),
            UiAction::LaunchRecipe => self.launch_recipe(cx),
        }
        if self.form.is_none() {
            self.focus.focus(window);
        } else if focus_first {
            self.focus_field(false, window, cx);
        }
        cx.notify();
    }

    fn focus_field(&self, backwards: bool, window: &mut Window, cx: &Context<'_, Self>) {
        let fields = match &self.form {
            Some(Form::Instance(form)) => vec![&form.name, &form.version, &form.directory],
            Some(Form::Profile(form)) => {
                let mut fields = vec![&form.name, &form.memory];
                if !form.managed {
                    fields.push(&form.java);
                }
                fields.extend([&form.jvm, &form.game]);
                fields
            }
            _ => return,
        };
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
            engine.create_instance(name, version, directory, neoforge, self.cancel.clone());
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
        if let Some(engine) = &self.engine {
            engine.play(profile, account, self.cancel.clone());
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
            .px_3()
            .py_2()
            .rounded_md()
            .bg(rgb(if self.busy { 0x0022_303a } else { 0x0029_3c4a }))
            .text_sm()
            .cursor_pointer()
            .hover(|style| style.bg(rgb(0x0036_5363)))
            .on_click(
                cx.listener(move |view, _, window, cx| view.handle(action.clone(), window, cx)),
            )
            .child(label.into())
    }

    fn sidebar(&self, cx: &Context<'_, Self>) -> impl IntoElement + use<> {
        div()
            .flex()
            .flex_col()
            .w(px(230.))
            .flex_shrink_0()
            .bg(rgb(0x0010_1923))
            .p_4()
            .gap_3()
            .child(div().text_xl().child("Block"))
            .child(note("Minecraft: Java Edition"))
            .child(self.button(
                "tab-instances",
                "Instances",
                UiAction::Tab(Tab::Instances),
                cx,
            ))
            .child(self.button("tab-profiles", "Profiles", UiAction::Tab(Tab::Profiles), cx))
            .child(self.button("tab-accounts", "Accounts", UiAction::Tab(Tab::Accounts), cx))
            .child(div().mt_4().text_xs().child("YOUR INSTALLATIONS"))
            .child(
                div()
                    .id("instance-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .children(self.snapshot.instances.iter().map(|instance| {
                        let label = format!(
                            "{}{}",
                            if Some(&instance.id) == self.selected_instance.as_ref() {
                                "● "
                            } else {
                                ""
                            },
                            instance.name
                        );
                        self.button(
                            format!("instance-{}", instance.id),
                            label,
                            UiAction::SelectInstance(instance.id.clone()),
                            cx,
                        )
                    })),
            )
            .child(self.button(
                "create-instance",
                "+ New instance",
                UiAction::CreateInstance,
                cx,
            ))
            .child(note(format!(
                "{} / {}",
                std::env::consts::OS,
                std::env::consts::ARCH
            )))
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
                panel().gap_2().bg(rgb(0x0010_1923)).child(div().text_lg().child(account.player_name.clone()))
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
            .bg(rgb(0x0010_1923))
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
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(self.button("new-profile", "+ New profile", UiAction::NewProfile, cx))
                    .child(self.button("edit-profile", "Edit", UiAction::EditProfile, cx))
                    .child(self.button("copy-profile", "Copy settings", UiAction::CopyProfile, cx))
                    .child(self.button("delete-profile", "Delete", UiAction::DeleteProfile, cx)),
            )
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
            .bg(rgb(0x0017_352f))
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
        panel().gap_3().child(div().text_2xl().child("Create an instance"))
            .child(field("Name", &form.name))
            .child(field("Minecraft version", &form.version))
            .child(div().flex().flex_wrap().gap_2().children(self.versions.iter().take(8).enumerate().map(|(index, version)| self.button(format!("version-{index}"), version.clone(), UiAction::SelectVersion(version.clone()), cx))))
            .child(note("Choose a release above, or type an exact Minecraft version. Native platform compatibility is checked before installation."))
            .child(self.button("loader-toggle", if form.neoforge { "NeoForge 20.6.144 · Minecraft 1.20.6" } else { "Vanilla · click to choose NeoForge" }, UiAction::ToggleLoader, cx))
            .child(field("Installation location (optional)", &form.directory))
            .child(self.button("browse-install", "Choose folder…", UiAction::BrowseInstall, cx))
            .child(note("Leave the location empty to use Block's data folder. Downloads are verified and shared safely; the default profile uses isolated game data."))
            .child(div().flex().flex_wrap().gap_2()
                .child(self.button("install-instance", "Create and install", UiAction::SaveInstance, cx))
                .child(self.button("cancel-instance", "Cancel", UiAction::CancelForm, cx)))
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
                    .bg(rgb(0x000b_1219))
                    .p_3()
                    .text_xs()
                    .children(lines.into_iter().map(|line| div().mb_1().child(line))),
            )
    }
}

impl Render for Block {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let main = match &self.form {
            Some(Form::Instance(form)) => self.instance_form(form, cx).into_any_element(),
            Some(Form::Profile(form)) => self.profile_form(form, cx).into_any_element(),
            Some(Form::Delete(_)) => panel().gap_4().child(div().text_xl().child("Delete this profile?"))
                .child(note("Only launch settings will be removed. All worlds, game data and shared installation files will remain on disk. Copying a profile copies settings only; isolated profiles start with an empty game directory."))
                .child(div().flex().flex_wrap().gap_2().child(self.button("confirm-delete", "Delete profile", UiAction::ConfirmDelete, cx))
                    .child(self.button("cancel-delete", "Keep profile", UiAction::CancelForm, cx))).into_any_element(),
            None => match self.tab { Tab::Instances => self.instance_page(cx).into_any_element(), Tab::Profiles => self.profiles_page(cx).into_any_element(), Tab::Accounts => self.accounts_page(cx).into_any_element() },
        };
        let mut content = div()
            .id("main-content")
            .flex_1()
            .min_w_0()
            .overflow_y_scroll()
            .p_6()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                panel()
                    .gap_1()
                    .child(div().text_sm().child(self.status.clone()))
                    .child(note(if self.busy {
                        format!(
                            "{} resource objects verified · Stop cancels the current operation",
                            self.completed
                        )
                    } else {
                        "Ready · account credentials are never stored in profiles".into()
                    })),
            )
            .child(main)
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(self.button(
                        "show-output",
                        if self.show_output {
                            "Hide output"
                        } else {
                            "View output"
                        },
                        UiAction::ToggleOutput,
                        cx,
                    ))
                    .child(self.button(
                        "open-recipe",
                        "Open development recipe…",
                        UiAction::OpenRecipe,
                        cx,
                    )),
            );
        if self.busy {
            content = content.child(self.button(
                "operation-stop",
                "Stop current operation",
                UiAction::Stop,
                cx,
            ));
        }
        if let Some(recipe) = &self.recipe {
            let title = match &recipe.source {
                RecipeSource::Mojang { version } => version.as_str(),
                RecipeSource::LocalManifest { .. } => "local manifest",
            };
            content = content.child(self.button(
                "launch-recipe",
                format!("Launch development recipe · {title}"),
                UiAction::LaunchRecipe,
                cx,
            ));
        }
        if self.show_output {
            content = content.child(self.output(cx));
        }
        div()
            .key_context("LauncherForm")
            .on_action(
                cx.listener(|view, _: &NextField, window, cx| view.focus_field(false, window, cx)),
            )
            .on_action(cx.listener(|view, _: &PreviousField, window, cx| {
                view.focus_field(true, window, cx);
            }))
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .bg(rgb(0x0017_232d))
            .text_color(rgb(0x00e9_f0f4))
            .child(self.sidebar(cx))
            .child(content)
    }
}

fn panel() -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .p_4()
        .rounded_md()
        .bg(rgb(0x001d_2d39))
}
fn note(text: impl Into<SharedString>) -> gpui::Div {
    div()
        .text_xs()
        .text_color(rgb(0x00a3_b7c7))
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
    fn test_view(
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
            snapshot: block_launcher::catalog::Snapshot::default(),
            accounts: Vec::new(),
            versions: vec!["1.20.6".into()],
            selected_instance: None,
            selected_profile: None,
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
            logs: std::collections::VecDeque::new(),
            show_output: false,
            command: None,
            show_command: false,
            recipe: None,
            recipe_load: RecipeLoadState::default(),
        }
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
        cx.simulate_keystrokes("cmd-a");
        cx.simulate_input("世界 🧱");
        assert_eq!(
            view.read_with(cx, |view, cx| match &view.form {
                Some(super::Form::Instance(form)) => form.name.read(cx).value(),
                _ => String::new(),
            }),
            "世界 🧱"
        );
        cx.simulate_keystrokes("tab cmd-a");
        cx.simulate_input("1.20.1");
        assert_eq!(
            view.read_with(cx, |view, cx| match &view.form {
                Some(super::Form::Instance(form)) => form.version.read(cx).value(),
                _ => String::new(),
            }),
            "1.20.1"
        );
        let bounds = cx.debug_bounds("loader-toggle");
        assert!(bounds.is_some());
        let Some(bounds) = bounds else {
            return;
        };
        cx.simulate_click(bounds.center(), gpui::Modifiers::none());
        assert!(view.read_with(cx, |view, cx| matches!(&view.form, Some(super::Form::Instance(form)) if form.neoforge && form.version.read(cx).value() == "1.20.6")));
        assert!(!view.read_with(cx, |view, _| view.busy));
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
