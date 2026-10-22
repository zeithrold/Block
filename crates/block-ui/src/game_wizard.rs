#![expect(
    clippy::redundant_pub_crate,
    reason = "Wizard state is shared only with the parent view and shell."
)]
//! Draft-only game setup. The final review is the only installation action.

#[cfg(test)]
#[path = "game_wizard_tests.rs"]
mod tests;

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
};

use super::browser::CollectionLoad;
use block_launcher::{
    CancellationToken,
    catalog::LoaderKind,
    install::fabric::LoaderVersion,
    platform::Architecture,
    support::{legacy_lwjgl_message, legacy_lwjgl_reason},
};
use gpui::{
    AppContext as _, Context, Entity, Focusable as _, InteractiveElement as _, IntoElement,
    ParentElement as _, ScrollHandle, Styled as _, Window, div, px, rgb,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct LoaderCatalog {
    pub request: uuid::Uuid,
    pub minecraft: String,
    pub kind: LoaderKind,
    pub builds: Vec<LoaderVersion>,
    pub load: CollectionLoad,
}

#[derive(Debug)]
struct LoaderRequest(CancellationToken);

impl Drop for LoaderRequest {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

use super::{
    Block, Form, InstanceForm, Tab, UiAction,
    components::{self, BadgeVariant, ButtonVariant, ControlState, FieldOptions},
    design::{BROWSER_FRACTION, Icon},
    input, note, patterns,
    scroll_area::ScrollAreaExt as _,
    text_input::TextInput,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum WizardStep {
    GetStarted,
    #[default]
    Setup,
    Profile,
    Review,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum CreationKind {
    #[default]
    NewInstance,
    ExistingInstance,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum WizardPage {
    Version,
    Loader,
    OptiFine,
    Location,
}

#[derive(Debug, Default)]
pub(super) struct WizardState {
    pub step: WizardStep,
    pub page: Option<WizardPage>,
    pub revision: usize,
    pub backwards: bool,
    pub version_search: Option<Entity<TextInput>>,
    pub loader_search: Option<Entity<TextInput>>,
    pub loader_catalog: Option<LoaderCatalog>,
    loader_request: Option<LoaderRequest>,
    optifine_request: Option<uuid::Uuid>,
    loader_scroll: ScrollHandle,
    version_focus: Rc<RefCell<HashMap<String, gpui::FocusHandle>>>,
    advanced: RefCell<super::motion::Disclosure>,
    pub include_snapshots: bool,
    pub version_scroll: ScrollHandle,
    section_snapshot: Option<ComponentDraft>,
    pub error: Option<String>,
    version_notice: Option<String>,
    pub separated: bool,
    pub kind: CreationKind,
    pub existing_instance: Option<String>,
    pub profile: Option<ProfileDraft>,
    pub body_scroll: ScrollHandle,
    pub rail_scroll: ScrollHandle,
    error_field: Option<WizardField>,
    reveal: Rc<Cell<Option<WizardField>>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum WizardField {
    GameName,
    Version,
    Loader,
    Directory,
    ProfileName,
    Server,
    Memory,
    Java,
    Jvm,
    Width,
    Height,
}

#[derive(Debug)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "Runtime, window, disclosure and server preferences are independent draft values."
)]
pub(super) struct ProfileDraft {
    pub name: Entity<TextInput>,
    pub server: Entity<TextInput>,
    pub connect_on_launch: bool,
    pub memory: Entity<TextInput>,
    pub java: Entity<TextInput>,
    pub jvm: Entity<TextInput>,
    pub width: Entity<TextInput>,
    pub height: Entity<TextInput>,
    pub managed: bool,
    pub fullscreen: bool,
    pub window_custom: bool,
    pub options: bool,
    pub appearance: block_launcher::catalog::ProfileAppearance,
    pub target_id: String,
}

impl ProfileDraft {
    pub(super) fn snapshot(&self, cx: &mut Context<'_, Block>) -> Self {
        Self {
            name: cx.new(|cx| self.name.read(cx).frozen(cx)),
            server: cx.new(|cx| self.server.read(cx).frozen(cx)),
            connect_on_launch: self.connect_on_launch,
            memory: cx.new(|cx| self.memory.read(cx).frozen(cx)),
            java: cx.new(|cx| self.java.read(cx).frozen(cx)),
            jvm: cx.new(|cx| self.jvm.read(cx).frozen(cx)),
            width: cx.new(|cx| self.width.read(cx).frozen(cx)),
            height: cx.new(|cx| self.height.read(cx).frozen(cx)),
            managed: self.managed,
            fullscreen: self.fullscreen,
            window_custom: self.window_custom,
            options: self.options,
            appearance: self.appearance,
            target_id: self.target_id.clone(),
        }
    }

    pub(super) fn new(
        defaults: &super::settings::GameDefaults,
        cx: &mut Context<'_, Block>,
    ) -> Self {
        Self {
            name: input("New Profile", "Profile name", cx),
            server: input("", "play.example.com:25565", cx),
            connect_on_launch: false,
            memory: input(&defaults.memory_mib.to_string(), "2048", cx),
            java: input(&defaults.java, "Absolute path to Java", cx),
            jvm: input(&defaults.jvm_arguments, "Extra JVM arguments", cx),
            width: input(&defaults.width.to_string(), "1280", cx),
            height: input(&defaults.height.to_string(), "720", cx),
            managed: defaults.java.trim().is_empty(),
            fullscreen: defaults.fullscreen,
            window_custom: !matches!(
                (defaults.width, defaults.height),
                (854, 480) | (1280, 720) | (1920, 1080)
            ),
            options: false,
            appearance: block_launcher::catalog::ProfileAppearance::default(),
            target_id: String::new(),
        }
    }

    pub(super) fn invalid_field(&self, cx: &gpui::App) -> Option<WizardField> {
        let name = self.name.read(cx).value();
        let valid_number = |input: &Entity<TextInput>, minimum, maximum| {
            input
                .read(cx)
                .value()
                .trim()
                .parse::<u32>()
                .is_ok_and(|value| (minimum..=maximum).contains(&value))
        };
        if name.trim().is_empty() || name.len() > 80 || name.chars().any(char::is_control) {
            Some(WizardField::ProfileName)
        } else if self.server_preferences(cx).validate().is_err() {
            Some(WizardField::Server)
        } else if !valid_number(&self.memory, 256, 65536) {
            Some(WizardField::Memory)
        } else if !self.managed
            && !std::path::Path::new(self.java.read(cx).value().trim()).is_absolute()
        {
            Some(WizardField::Java)
        } else if super::parse_arguments(&self.jvm.read(cx).value()).is_err() {
            Some(WizardField::Jvm)
        } else if !valid_number(&self.width, 320, 16384) {
            Some(WizardField::Width)
        } else if !valid_number(&self.height, 240, 16384) {
            Some(WizardField::Height)
        } else {
            None
        }
    }

    pub(super) const fn input(&self, field: WizardField) -> Option<&Entity<TextInput>> {
        match field {
            WizardField::ProfileName => Some(&self.name),
            WizardField::Server => Some(&self.server),
            WizardField::Memory => Some(&self.memory),
            WizardField::Java => Some(&self.java),
            WizardField::Jvm => Some(&self.jvm),
            WizardField::Width => Some(&self.width),
            WizardField::Height => Some(&self.height),
            WizardField::GameName
            | WizardField::Version
            | WizardField::Loader
            | WizardField::Directory => None,
        }
    }

    fn server_preferences(&self, cx: &gpui::App) -> block_launcher::catalog::ServerPreferences {
        block_launcher::catalog::ServerPreferences {
            address: self.server.read(cx).value().trim().to_owned(),
            connect_on_launch: self.connect_on_launch,
        }
    }

    pub(super) fn values(
        &self,
        cx: &gpui::App,
    ) -> anyhow::Result<super::engine::ProfileDraftValues> {
        use anyhow::Context as _;
        let name = self.name.read(cx).value().trim().to_owned();
        anyhow::ensure!(
            !name.is_empty() && name.len() <= 80 && !name.chars().any(char::is_control),
            "Give the Profile a name of 1–80 characters."
        );
        let defaults = super::settings::GameDefaults {
            memory_mib: self
                .memory
                .read(cx)
                .value()
                .trim()
                .parse()
                .context("Enter maximum memory as a whole number of MiB.")?,
            java: if self.managed {
                String::new()
            } else {
                self.java.read(cx).value().trim().to_owned()
            },
            jvm_arguments: self.jvm.read(cx).value(),
            width: self
                .width
                .read(cx)
                .value()
                .trim()
                .parse()
                .context("Enter a whole number for window width.")?,
            height: self
                .height
                .read(cx)
                .value()
                .trim()
                .parse()
                .context("Enter a whole number for window height.")?,
            fullscreen: self.fullscreen,
            show_snapshots: false,
        };
        if !self.managed {
            anyhow::ensure!(
                !defaults.java.is_empty(),
                "Choose a Java executable or use managed Java."
            );
        }
        defaults.validate()?;
        let server = self.server_preferences(cx);
        server.validate()?;
        Ok(super::engine::ProfileDraftValues {
            server,
            name,
            defaults,
            appearance: self.appearance,
            target_id: (!self.target_id.is_empty()).then(|| self.target_id.clone()),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "A checkpoint preserves each independent draft preference for section cancellation."
)]
struct ProfileCheckpoint {
    values: [String; 7],
    managed: bool,
    fullscreen: bool,
    window_custom: bool,
    options: bool,
    connect_on_launch: bool,
    appearance: block_launcher::catalog::ProfileAppearance,
    target_id: String,
}

impl ProfileDraft {
    fn checkpoint(&self, cx: &gpui::App) -> ProfileCheckpoint {
        ProfileCheckpoint {
            values: [
                &self.name,
                &self.server,
                &self.memory,
                &self.java,
                &self.jvm,
                &self.width,
                &self.height,
            ]
            .map(|field| field.read(cx).value()),
            managed: self.managed,
            fullscreen: self.fullscreen,
            window_custom: self.window_custom,
            options: self.options,
            connect_on_launch: self.connect_on_launch,
            appearance: self.appearance,
            target_id: self.target_id.clone(),
        }
    }
    fn restore_checkpoint(&mut self, checkpoint: ProfileCheckpoint, cx: &mut Context<'_, Block>) {
        for (input, value) in [
            &self.name,
            &self.server,
            &self.memory,
            &self.java,
            &self.jvm,
            &self.width,
            &self.height,
        ]
        .into_iter()
        .zip(checkpoint.values)
        {
            input.update(cx, |input, cx| {
                if input.value() != value {
                    input.set_value(&value, cx);
                }
            });
        }
        self.managed = checkpoint.managed;
        self.fullscreen = checkpoint.fullscreen;
        self.window_custom = checkpoint.window_custom;
        self.options = checkpoint.options;
        self.connect_on_launch = checkpoint.connect_on_launch;
        self.appearance = checkpoint.appearance;
        self.target_id = checkpoint.target_id;
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ComponentDraft {
    version: String,
    directory: String,
    loader: LoaderKind,
    loader_version: Option<String>,
    loader_catalog: Option<LoaderCatalog>,
    optifine: Option<Box<block_launcher::install::optifine::OptiFineJar>>,
    version_notice: Option<String>,
    profile: Option<ProfileCheckpoint>,
}

impl WizardState {
    pub(super) fn cancel_optifine_import(&mut self) {
        self.optifine_request = None;
        self.clear_error();
    }
    pub(super) fn cancel_loader_request(&mut self) {
        self.loader_request = None;
        self.loader_catalog = None;
    }
    /// Freeze the rendered draft, including independent editor and scroll state.
    /// This copy creates no subscriptions or requests and cannot mutate the live draft.
    pub(super) fn snapshot(&self, cx: &mut Context<'_, Block>) -> Self {
        let version_scroll = ScrollHandle::new();
        version_scroll.set_offset(self.version_scroll.offset());
        let body_scroll = ScrollHandle::new();
        body_scroll.set_offset(self.body_scroll.offset());
        let rail_scroll = ScrollHandle::new();
        rail_scroll.set_offset(self.rail_scroll.offset());
        let loader_scroll = ScrollHandle::new();
        loader_scroll.set_offset(self.loader_scroll.offset());
        Self {
            step: self.step,
            page: self.page,
            revision: self.revision,
            backwards: self.backwards,
            version_search: self
                .version_search
                .as_ref()
                .map(|input| cx.new(|cx| input.read(cx).frozen(cx))),
            loader_search: self
                .loader_search
                .as_ref()
                .map(|input| cx.new(|cx| input.read(cx).frozen(cx))),
            loader_catalog: self.loader_catalog.clone(),
            loader_request: None,
            optifine_request: self.optifine_request,
            loader_scroll,
            version_focus: Rc::default(),
            advanced: RefCell::new(self.advanced.borrow().clone()),
            include_snapshots: self.include_snapshots,
            version_scroll,
            section_snapshot: self.section_snapshot.clone(),
            error: self.error.clone(),
            version_notice: self.version_notice.clone(),
            separated: self.separated,
            kind: self.kind,
            existing_instance: self.existing_instance.clone(),
            profile: self.profile.as_ref().map(|profile| profile.snapshot(cx)),
            body_scroll,
            rail_scroll,
            error_field: self.error_field,
            reveal: Rc::new(Cell::new(None)),
        }
    }

    fn clear_error(&mut self) {
        self.error = None;
        self.error_field = None;
        self.reveal.set(None);
    }

    pub(super) fn new(cx: &mut Context<'_, Block>) -> Self {
        Self {
            version_search: Some(input("", "Search versions…", cx)),
            loader_search: Some(input("", "Search loader builds…", cx)),
            ..Self::default()
        }
    }

    pub(super) fn next(&mut self) {
        if self.page.is_some() {
            self.open_page(None);
        } else if self.step != WizardStep::Review {
            self.step = match self.step {
                WizardStep::GetStarted => WizardStep::Setup,
                WizardStep::Setup if self.separated => WizardStep::Profile,
                WizardStep::Setup | WizardStep::Profile | WizardStep::Review => WizardStep::Review,
            };
            self.transition(false);
        }
    }

    pub(super) fn back(&mut self) {
        if self.page.is_some() {
            self.open_page(None);
        } else if self.step != WizardStep::GetStarted
            && (self.separated || self.step != WizardStep::Setup)
        {
            self.step = match self.step {
                WizardStep::Review if self.separated => WizardStep::Profile,
                WizardStep::Review | WizardStep::Profile => WizardStep::Setup,
                WizardStep::Setup | WizardStep::GetStarted => WizardStep::GetStarted,
            };
            self.transition(true);
        }
    }

    pub(super) fn open_page(&mut self, page: Option<WizardPage>) {
        if self.page != page {
            self.page = page;
            self.transition(page.is_none());
        }
    }

    fn transition(&mut self, backwards: bool) {
        self.backwards = backwards;
        self.revision = self.revision.wrapping_add(1);
        self.body_scroll.set_offset(gpui::point(px(0.), px(0.)));
    }
}

impl Block {
    fn optifine_issue(&self, cx: &gpui::App) -> Option<String> {
        let Some(Form::Instance(form)) = &self.form else {
            return None;
        };
        form.optifine
            .as_ref()
            .and_then(|selected| {
                selected
                    .validate_for(
                        &form.version.read(cx).value(),
                        &form.loader,
                        form.loader_version.as_deref(),
                    )
                    .err()
            })
            .map(|error| error.to_string())
    }

    pub(super) fn browse_optifine(&self, cx: &Context<'_, Self>) {
        if self.busy
            || self.shell.wizard.optifine_request.is_some()
            || self.shell.wizard.page != Some(WizardPage::OptiFine)
        {
            return;
        }
        let Some(Form::Instance(form)) = &self.form else {
            return;
        };
        if !matches!(form.loader, LoaderKind::Vanilla | LoaderKind::Forge) {
            return;
        }
        let Some(engine) = self.engine.clone() else {
            return;
        };
        let field = form.version.entity_id();
        let prompt = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Select the official OptiFine installer JAR".into()),
        });
        cx.spawn(async move |view, cx| {
            let Ok(Ok(Some(paths))) = prompt.await else { return; };
            let Some(path) = paths.into_iter().next() else { return; };
            let request = uuid::Uuid::new_v4();
            let started = view.update(cx, |view, cx| {
                if view.busy || view.shell.wizard.page != Some(WizardPage::OptiFine) || !matches!(&view.form, Some(Form::Instance(form)) if form.version.entity_id() == field) { return false; }
                view.shell.wizard.optifine_request = Some(request);
                view.shell.wizard.clear_error();
                cx.notify();
                true
            }).unwrap_or(false);
            if !started { return; }
            let result = engine.import_optifine(path).await;
            let _ = view.update(cx, |view, cx| view.finish_optifine_import(request, field, result, cx));
        }).detach();
    }

    fn finish_optifine_import(
        &mut self,
        request: uuid::Uuid,
        field: gpui::EntityId,
        result: anyhow::Result<block_launcher::install::optifine::OptiFineJar>,
        cx: &mut Context<'_, Self>,
    ) {
        if self.shell.wizard.optifine_request != Some(request)
            || self.shell.wizard.page != Some(WizardPage::OptiFine)
        {
            return;
        }
        self.shell.wizard.optifine_request = None;
        let Some(Form::Instance(form)) = &mut self.form else {
            return;
        };
        if form.version.entity_id() != field {
            return;
        }
        match result {
            Ok(selected) => match selected.validate_for(
                &form.version.read(cx).value(),
                &form.loader,
                form.loader_version.as_deref(),
            ) {
                Ok(()) => form.optifine = Some(Box::new(selected)),
                Err(error) => self.shell.wizard.error = Some(error.to_string()),
            },
            Err(error) => {
                self.shell.wizard.error = Some(format!("Could not import OptiFine: {error:#}"))
            }
        }
        cx.notify();
    }

    pub(super) fn wizard_optifine_component(
        &self,
        form: &InstanceForm,
        cx: &Context<'_, Self>,
    ) -> gpui::AnyElement {
        if !matches!(form.loader, LoaderKind::Vanilla | LoaderKind::Forge) {
            return self
                .wizard_unavailable(
                    "optifine",
                    "OptiFine",
                    &format!("Cannot combine directly with {}", form.loader.name()),
                    true,
                )
                .into_any_element();
        }
        self.wizard_component(
            "optifine",
            "OptiFine",
            (
                form.optifine
                    .as_ref()
                    .map_or("Not selected", |jar| jar.version.as_str()),
                if form.optifine.is_some() {
                    "JAR identity checked"
                } else {
                    "Download, then select JAR"
                },
            ),
            WizardPage::OptiFine,
            cx,
        )
        .into_any_element()
    }

    pub(super) fn wizard_optifine(&self, form: &InstanceForm, cx: &Context<'_, Self>) -> gpui::Div {
        let pending = self.shell.wizard.optifine_request.is_some();
        div().flex().flex_col().gap_4()
            .child(div().text_xl().child("Install OptiFine"))
            .child(note("Download the installer JAR for your Minecraft version from optifine.net, then select it here. Block validates it and performs the remaining installation automatically."))
            .child(components::button("optifine-official-download", "Open official downloads", self.palette, ButtonVariant::Outline, ControlState::default(), |&(), _, cx| cx.open_url("https://optifine.net/downloads")))
            .child(self.button("optifine-select-jar", if pending { "Validating selected JAR…" } else { "Select downloaded JAR" }, UiAction::WizardOptiFineFile, cx))
            .children(form.optifine.as_ref().map(|selected| div().flex().flex_col().gap_2()
                .child(div().text_sm().child(format!("Selected: {}", selected.version)))
                .child(note(selected.forge.as_ref().map_or_else(|| "Standalone installation; this release does not declare Forge compatibility.".into(), |version| format!("Standalone or Forge {version}. Other loaders cannot be combined directly."))))
                .child(self.button("optifine-remove", "Use without OptiFine", UiAction::WizardOptiFineRemove, cx))))
            .children(self.optifine_issue(cx).map(|reason| note(reason)))
            .children(self.shell.wizard.error.as_ref().map(|reason| note(reason.clone())))
            .child(note("No extraction or manual file placement is required. The selected JAR is kept only in your private launcher cache."))
    }
    pub(super) fn set_wizard_error(&mut self, message: String, cx: &gpui::App) {
        let mut field = self
            .shell
            .wizard
            .profile
            .as_ref()
            .and_then(|profile| profile.invalid_field(cx));
        if self.shell.wizard.kind == CreationKind::NewInstance
            && self.shell.wizard.page.is_none()
            && let Some(Form::Instance(form)) = &self.form
        {
            if form.name.read(cx).value().trim().is_empty() {
                field = Some(WizardField::GameName);
            } else if form.version.read(cx).value().trim().is_empty() {
                field = Some(WizardField::Version);
            } else if self.wizard_loader_issue(cx).is_some() {
                field = Some(WizardField::Loader);
            } else {
                let directory = form.directory.read(cx).value();
                if !directory.trim().is_empty()
                    && !std::path::Path::new(directory.trim()).is_absolute()
                {
                    field = Some(WizardField::Directory);
                }
            }
        }
        if self.shell.wizard.page.is_none()
            && self.shell.wizard.kind == CreationKind::NewInstance
            && field.is_some_and(|field| {
                matches!(
                    field,
                    WizardField::Java
                        | WizardField::Width
                        | WizardField::Height
                        | WizardField::Directory
                )
            })
        {
            self.shell.wizard.open_page(Some(WizardPage::Location));
        }
        if matches!(field, Some(WizardField::Width | WizardField::Height))
            && let Some(profile) = &mut self.shell.wizard.profile
        {
            profile.window_custom = true;
        }
        self.shell.wizard.error = Some(message);
        self.shell.wizard.error_field = field;
        self.shell.wizard.reveal.set(field);
        if let Some(profile) = &mut self.shell.wizard.profile
            && field.is_some_and(|field| {
                !matches!(
                    field,
                    WizardField::GameName
                        | WizardField::Version
                        | WizardField::Directory
                        | WizardField::ProfileName
                )
            })
        {
            profile.options = true;
        }
    }

    pub(super) fn focus_wizard_error(&self, window: &mut Window, cx: &mut gpui::App) -> bool {
        if !matches!(self.tab, Tab::Instances | Tab::Onboarding)
            || !matches!(self.form, Some(Form::Instance(_)))
            || self.shell.wizard.error.is_none()
        {
            return false;
        }
        let Some(field) = self.shell.wizard.error_field else {
            return false;
        };
        let instance_input = match &self.form {
            Some(Form::Instance(form)) => match field {
                WizardField::GameName => Some(&form.name),
                WizardField::Version if self.shell.wizard.page == Some(WizardPage::Version) => {
                    self.shell.wizard.version_search.as_ref()
                }
                WizardField::Version => Some(&form.version),
                WizardField::Directory => Some(&form.directory),
                _ => None,
            },
            _ => None,
        };
        let input = instance_input.or_else(|| {
            self.shell
                .wizard
                .profile
                .as_ref()
                .and_then(|profile| profile.input(field))
        });
        input.is_some_and(|input| {
            input.read(cx).focus_handle(cx).focus(window, cx);
            true
        })
    }

    fn wizard_field(
        &self,
        field: WizardField,
        label: &'static str,
        input: &Entity<TextInput>,
        hint: Option<&'static str>,
    ) -> gpui::Stateful<gpui::Div> {
        patterns::reveal_field(
            components::field_with(
                label,
                input,
                self.palette,
                FieldOptions {
                    hint: hint.map(Into::into),
                    required: matches!(field, WizardField::GameName | WizardField::ProfileName),
                    error: self
                        .shell
                        .wizard
                        .error
                        .as_ref()
                        .filter(|_| self.shell.wizard.error_field == Some(field))
                        .map(|error| error.clone().into()),
                },
            ),
            self.shell.wizard.body_scroll.clone(),
            self.shell.wizard.reveal.clone(),
            field,
        )
        .id(gpui::SharedString::from(format!("wizard-field-{field:?}")))
        .debug_selector(move || format!("wizard-field-{field:?}"))
    }

    pub(super) fn begin_game_wizard(&mut self, cx: &mut Context<'_, Self>) {
        self.shell.wizard = WizardState::new(cx);
        self.shell.wizard.include_snapshots = self.saved_preferences.game.show_snapshots;
        self.shell.wizard.separated =
            self.snapshot.profile_mode == block_launcher::catalog::ProfileMode::Separated;
        if self.shell.wizard.separated {
            self.shell.wizard.step = WizardStep::GetStarted;
        }
        self.shell.wizard.existing_instance = self.selected_instance.clone().or_else(|| {
            self.snapshot
                .instances
                .first()
                .map(|instance| instance.id.clone())
        });
        self.shell.wizard.profile =
            Some(ProfileDraft::new(&self.saved_preferences.game.clone(), cx));
        self.tab = Tab::Instances;
        let version = self
            .versions
            .first()
            .map_or("1.20.6", String::as_str)
            .to_owned();
        self.form = Some(Form::Instance(InstanceForm {
            name: input("My Minecraft", "Game name", cx),
            version: input(&version, "Minecraft version", cx),
            directory: input("", "Automatic installation location", cx),
            loader: LoaderKind::Vanilla,
            loader_version: None,
            optifine: None,
        }));
    }

    pub(super) fn wizard_kind(&mut self, is_new: bool, _: &mut Context<'_, Self>) {
        if is_new || !self.snapshot.instances.is_empty() {
            self.shell.wizard.kind = if is_new {
                CreationKind::NewInstance
            } else {
                CreationKind::ExistingInstance
            };
            self.shell.wizard.clear_error();
            if !is_new {
                self.normalize_wizard_target();
            }
        }
    }

    pub(super) fn wizard_instance(&mut self, id: String) {
        if self
            .snapshot
            .instances
            .iter()
            .any(|instance| instance.id == id)
        {
            self.shell.wizard.existing_instance = Some(id);
            self.shell.wizard.clear_error();
            self.normalize_wizard_target();
        }
    }

    fn normalize_wizard_target(&mut self) {
        if let Some(instance) = self
            .snapshot
            .instances
            .iter()
            .find(|instance| Some(&instance.id) == self.shell.wizard.existing_instance.as_ref())
            && let Some(profile) = &mut self.shell.wizard.profile
            && !instance
                .targets
                .iter()
                .any(|target| target.id == profile.target_id)
        {
            profile.target_id = instance
                .targets
                .first()
                .map_or_else(String::new, |target| target.id.clone());
        }
    }

    pub(super) fn wizard_profile_target(&mut self, id: String) {
        if self.snapshot.instances.iter().any(|instance| {
            Some(&instance.id) == self.shell.wizard.existing_instance.as_ref()
                && instance.targets.iter().any(|target| target.id == id)
        }) && let Some(profile) = &mut self.shell.wizard.profile
        {
            profile.target_id = id;
        }
    }

    pub(super) const fn wizard_profile_icon(&mut self, icon: block_launcher::catalog::ProfileIcon) {
        if let Some(profile) = &mut self.shell.wizard.profile {
            profile.appearance.icon = icon;
        }
    }
    pub(super) fn wizard_appearance(&self) -> block_launcher::catalog::ProfileAppearance {
        self.shell
            .wizard
            .profile
            .as_ref()
            .map_or_else(Default::default, |profile| profile.appearance)
    }
    pub(super) const fn wizard_profile_color(
        &mut self,
        color: block_launcher::catalog::ProfileColor,
    ) {
        if let Some(profile) = &mut self.shell.wizard.profile {
            profile.appearance.color = color;
        }
    }
    pub(super) const fn wizard_profile_connect(&mut self) {
        if let Some(profile) = &mut self.shell.wizard.profile {
            profile.connect_on_launch = !profile.connect_on_launch;
        }
    }

    pub(super) const fn wizard_profile_fullscreen(&mut self) {
        if let Some(profile) = &mut self.shell.wizard.profile {
            profile.fullscreen = !profile.fullscreen;
        }
    }
    pub(super) const fn wizard_runtime(&mut self, managed: bool) {
        if let Some(profile) = &mut self.shell.wizard.profile {
            profile.managed = managed;
        }
    }

    pub(super) fn wizard_window(&mut self, width: u32, height: u32, cx: &mut Context<'_, Self>) {
        if let Some(profile) = &mut self.shell.wizard.profile {
            profile
                .width
                .update(cx, |field, cx| field.set_value(&width.to_string(), cx));
            profile
                .height
                .update(cx, |field, cx| field.set_value(&height.to_string(), cx));
            profile.window_custom = false;
        }
    }

    pub(super) const fn wizard_custom_window(&mut self) {
        if let Some(profile) = &mut self.shell.wizard.profile {
            profile.window_custom = true;
        }
    }
    pub(super) const fn wizard_profile_options(&mut self) {
        if let Some(profile) = &mut self.shell.wizard.profile {
            profile.options = !profile.options;
        }
    }

    pub(super) fn open_game_wizard_page(
        &mut self,
        page: Option<WizardPage>,
        cx: &Context<'_, Self>,
    ) {
        if page != Some(WizardPage::OptiFine) {
            self.shell.wizard.optifine_request = None;
        }
        if page.is_some()
            && self.shell.wizard.page.is_none()
            && let Some(Form::Instance(form)) = &self.form
        {
            self.shell.wizard.section_snapshot = Some(ComponentDraft {
                version: form.version.read(cx).value(),
                directory: form.directory.read(cx).value(),
                loader: form.loader.clone(),
                loader_version: form.loader_version.clone(),
                loader_catalog: self.shell.wizard.loader_catalog.clone(),
                optifine: form.optifine.clone(),
                version_notice: self.shell.wizard.version_notice.clone(),
                profile: self
                    .shell
                    .wizard
                    .profile
                    .as_ref()
                    .map(|profile| profile.checkpoint(cx)),
            });
        }
        if page.is_none() {
            self.shell.wizard.section_snapshot = None;
            self.shell.wizard.optifine_request = None;
        }
        self.shell.wizard.clear_error();
        self.shell.wizard.open_page(page);
        if page == Some(WizardPage::Loader) {
            self.load_wizard_loader_versions(false, cx);
        } else if page.is_none() {
            self.shell.wizard.loader_request = None;
        }
    }

    pub(super) fn choose_wizard_loader(&mut self, kind: LoaderKind, cx: &mut Context<'_, Self>) {
        if self.busy {
            return;
        }
        let Some(Form::Instance(form)) = &mut self.form else {
            return;
        };
        if form.loader == kind {
            return;
        }
        form.loader = kind.clone();
        form.loader_version = None;
        self.shell.wizard.loader_catalog = None;
        self.shell.wizard.loader_request = None;
        self.shell.wizard.version_notice = None;
        if !matches!(kind, LoaderKind::Vanilla | LoaderKind::Forge)
            && form.optifine.take().is_some()
        {
            self.shell.wizard.version_notice = Some(format!(
                "OptiFine was removed because it cannot be combined directly with {}.",
                kind.name()
            ));
        }
        self.shell
            .wizard
            .loader_scroll
            .set_offset(gpui::point(px(0.), px(0.)));
        self.shell.wizard.clear_error();
        if let Some(search) = &self.shell.wizard.loader_search {
            search.update(cx, |field, cx| field.set_value("", cx));
        }
        self.load_wizard_loader_versions(false, cx);
        cx.notify();
    }

    pub(super) fn choose_wizard_loader_build(
        &mut self,
        version: String,
        cx: &mut Context<'_, Self>,
    ) {
        if self.busy {
            return;
        }
        let Some(Form::Instance(form)) = &mut self.form else {
            return;
        };
        let valid = self
            .shell
            .wizard
            .loader_catalog
            .as_ref()
            .is_some_and(|catalog| {
                catalog.kind == form.loader
                    && catalog.minecraft == form.version.read(cx).value()
                    && matches!(catalog.load, CollectionLoad::Ready)
                    && catalog.builds.iter().any(|build| build.version == version)
            });
        if valid {
            form.loader_version = Some(version);
            self.shell.wizard.clear_error();
            self.shell.wizard.version_notice = None;
            cx.notify();
        }
    }

    pub(super) fn load_wizard_loader_versions(&mut self, force: bool, cx: &Context<'_, Self>) {
        let Some(Form::Instance(form)) = &self.form else {
            return;
        };
        if form.loader == LoaderKind::Vanilla {
            return;
        }
        let minecraft = form.version.read(cx).value();
        let kind = form.loader.clone();
        if !force
            && self
                .shell
                .wizard
                .loader_catalog
                .as_ref()
                .is_some_and(|catalog| {
                    catalog.kind == kind
                        && catalog.minecraft == minecraft
                        && (!matches!(catalog.load, CollectionLoad::Loading)
                            || self.shell.wizard.loader_request.is_some())
                })
        {
            return;
        }
        let request = uuid::Uuid::new_v4();
        let cancel = CancellationToken::new();
        let builds = self
            .shell
            .wizard
            .loader_catalog
            .as_ref()
            .filter(|catalog| catalog.kind == kind && catalog.minecraft == minecraft)
            .map_or_else(Vec::new, |catalog| catalog.builds.clone());
        self.shell.wizard.loader_catalog = Some(LoaderCatalog {
            request,
            minecraft: minecraft.clone(),
            kind: kind.clone(),
            builds,
            load: CollectionLoad::Loading,
        });
        self.shell.wizard.loader_request = Some(LoaderRequest(cancel.clone()));
        if let Some(engine) = &self.engine {
            engine.loader_versions(request, kind, minecraft, cancel);
        } else if !self.visual_only {
            self.receive_loader_versions(
                request,
                Err(
                    "The launcher service is unavailable. Retry after reopening this window."
                        .into(),
                ),
                cx,
            );
        }
    }

    pub(super) fn receive_loader_versions(
        &mut self,
        request: uuid::Uuid,
        result: Result<Vec<LoaderVersion>, String>,
        cx: &gpui::App,
    ) {
        let Some(Form::Instance(form)) = &mut self.form else {
            return;
        };
        let Some(catalog) = &mut self.shell.wizard.loader_catalog else {
            return;
        };
        if catalog.request != request
            || catalog.kind != form.loader
            || catalog.minecraft != form.version.read(cx).value()
        {
            return;
        }
        self.shell.wizard.loader_request = None;
        match result {
            Ok(mut builds) => {
                if !matches!(form.loader, LoaderKind::NeoForge | LoaderKind::Forge) {
                    builds.sort_by(|left, right| {
                        match (
                            semver::Version::parse(&left.version),
                            semver::Version::parse(&right.version),
                        ) {
                            (Ok(left), Ok(right)) => right.cmp(&left),
                            (Ok(_), Err(_)) => std::cmp::Ordering::Less,
                            (Err(_), Ok(_)) => std::cmp::Ordering::Greater,
                            (Err(_), Err(_)) => right.version.cmp(&left.version),
                        }
                    });
                }
                if let Some(selected) = &form.loader_version {
                    if !builds.iter().any(|build| build.version == *selected) {
                        form.loader_version = None;
                        self.shell.wizard.version_notice = Some("The selected loader build is no longer listed for this game. Choose another build.".into());
                    }
                } else {
                    form.loader_version = builds
                        .iter()
                        .find(|build| build.stable)
                        .or_else(|| builds.first())
                        .map(|build| build.version.clone());
                }
                catalog.builds = builds;
                catalog.load = CollectionLoad::Ready;
            }
            Err(error) => catalog.load = CollectionLoad::Failed(error),
        }
        if self.shell.wizard.error_field == Some(WizardField::Loader) {
            self.shell.wizard.clear_error();
        }
    }

    pub(super) fn wizard_loader_issue(&self, cx: &gpui::App) -> Option<String> {
        if let Some(issue) = self.optifine_issue(cx) {
            return Some(issue);
        }
        if self.shell.wizard.optifine_request.is_some() {
            return Some("Wait for OptiFine JAR validation to finish.".into());
        }
        let Some(Form::Instance(form)) = &self.form else {
            return None;
        };
        match form.loader {
            LoaderKind::Vanilla => None,
            _ => {
                let Some(catalog) = self.shell.wizard.loader_catalog.as_ref() else {
                    return Some(
                        "Load the official loader catalog and choose a compatible build.".into(),
                    );
                };
                if catalog.kind != form.loader || catalog.minecraft != form.version.read(cx).value()
                {
                    return Some("Reload the loader catalog for this Minecraft version.".into());
                }
                match &catalog.load {
                    CollectionLoad::Loading => Some("Wait for the official loader versions to load.".into()),
                    CollectionLoad::Failed(_) => Some("The official loader catalog could not be loaded. Retry before continuing.".into()),
                    CollectionLoad::Ready if catalog.builds.is_empty() => Some("This loader has no official build for the selected Minecraft version. Choose another loader or game version.".into()),
                    CollectionLoad::Ready if !catalog.builds.iter().any(|build| Some(&build.version) == form.loader_version.as_ref()) => Some("Choose a compatible loader build.".into()),
                    CollectionLoad::Ready => None,
                }
            }
        }
    }

    /// Host architecture used by the native version-availability policy.
    fn host_architecture(&self) -> Architecture {
        if let Some(arch) = self.host_arch_override {
            return arch;
        }
        if cfg!(target_arch = "aarch64") {
            Architecture::Aarch64
        } else if cfg!(target_arch = "x86") {
            Architecture::X86
        } else {
            Architecture::X86_64
        }
    }

    /// LWJGL 2-era restriction reason for a catalog version, if unavailable on
    /// this host. Unknown or not-yet-loaded versions are left to installation
    /// verification, which applies the same official-metadata rule.
    pub(super) fn wizard_version_restriction(&self, version: &str) -> Option<&'static str> {
        let version = version.trim();
        let entry = self
            .version_catalog
            .iter()
            .find(|entry| entry.id == version)?;
        legacy_lwjgl_reason(self.host_architecture(), entry, &self.version_catalog)
    }

    /// Catalog-level note explaining the ARM64 LWJGL 2 restriction.
    fn wizard_version_legacy_note(&self) -> Option<gpui::Div> {
        let restricted = self
            .version_catalog
            .iter()
            .any(|entry| self.wizard_version_restriction(&entry.id).is_some());
        restricted.then(|| {
            div()
                .debug_selector(|| "wizard-version-legacy-note".into())
                .child(note(format!(
                    "1.12.2 and earlier are unavailable on this ARM64 host: {}",
                    block_launcher::support::LWJGL2_ARM64_REASON
                )))
        })
    }

    pub(super) fn set_wizard_version(&mut self, version: String, cx: &mut Context<'_, Self>) {
        let version = if version.trim().len() == version.len() {
            version
        } else {
            version.trim().to_owned()
        };
        if self.wizard_version_restriction(&version).is_some() {
            self.shell.wizard.version_notice = Some(legacy_lwjgl_message(&version));
            cx.notify();
            return;
        }
        let Some(Form::Instance(form)) = &mut self.form else {
            return;
        };
        let changed = form.version.read(cx).value() != version;
        if changed {
            form.version
                .update(cx, |field, cx| field.set_value(&version, cx));
        }
        if changed {
            self.shell.wizard.version_notice = None;
        }
        if changed {
            self.shell.wizard.loader_catalog = None;
            self.shell.wizard.loader_request = None;
            if form.loader != LoaderKind::Vanilla {
                form.loader_version = None;
                self.shell.wizard.version_notice = Some("Minecraft changed. Choose a compatible build from the refreshed loader catalog.".into());
            }
            self.load_wizard_loader_versions(false, cx);
        }
        if self.shell.wizard.error_field == Some(WizardField::Version) {
            self.shell.wizard.clear_error();
        }
        cx.notify();
    }

    pub(super) fn validate_wizard_section(
        &mut self,
        page: WizardPage,
        cx: &Context<'_, Self>,
    ) -> bool {
        if let Some((field, message)) = self.wizard_section_issue(page, cx) {
            self.set_wizard_error(message, cx);
            self.shell.wizard.error_field = Some(field);
            self.shell.wizard.reveal.set(Some(field));
            false
        } else {
            self.shell.wizard.clear_error();
            true
        }
    }

    fn wizard_section_issue(
        &self,
        page: WizardPage,
        cx: &gpui::App,
    ) -> Option<(WizardField, String)> {
        let Some(Form::Instance(form)) = &self.form else {
            return Some((
                WizardField::GameName,
                "Reopen game setup to continue.".into(),
            ));
        };
        let version = form.version.read(cx).value();
        if page == WizardPage::Version
            && (version.trim().is_empty()
                || version.len() > 80
                || version.chars().any(char::is_control))
        {
            return Some((
                WizardField::Version,
                "Choose a Minecraft version from the list.".into(),
            ));
        }
        if page == WizardPage::Version && self.wizard_version_restriction(version.trim()).is_some()
        {
            return Some((WizardField::Version, legacy_lwjgl_message(version.trim())));
        }
        if matches!(page, WizardPage::Version | WizardPage::Loader)
            && let Some(issue) = self.wizard_loader_issue(cx)
        {
            return Some((WizardField::Loader, issue));
        }
        if let Some(selected) = &form.optifine
            && let Err(error) =
                selected.validate_for(&version, &form.loader, form.loader_version.as_deref())
        {
            return Some((WizardField::Loader, error.to_string()));
        }
        if self.shell.wizard.optifine_request.is_some() {
            return Some((
                WizardField::Loader,
                "Wait for the selected OptiFine JAR to finish validation.".into(),
            ));
        }
        if page != WizardPage::Location {
            return None;
        }
        let directory = form.directory.read(cx).value();
        if !directory.trim().is_empty()
            && (!std::path::Path::new(directory.trim()).is_absolute()
                || directory.chars().any(char::is_control))
        {
            return Some((
                WizardField::Directory,
                "Choose an absolute installation folder or leave it empty.".into(),
            ));
        }
        let Some(profile) = &self.shell.wizard.profile else {
            return Some((
                WizardField::Java,
                "The configuration draft is unavailable. Reopen game setup.".into(),
            ));
        };
        if !profile.managed
            && (!std::path::Path::new(profile.java.read(cx).value().trim()).is_absolute()
                || profile.java.read(cx).value().chars().any(char::is_control))
        {
            return Some((
                WizardField::Java,
                "Choose an absolute Java executable path or use automatic Java.".into(),
            ));
        }
        for (field, label, input, minimum, maximum) in [
            (
                WizardField::Width,
                "Window width",
                &profile.width,
                320,
                16384,
            ),
            (
                WizardField::Height,
                "Window height",
                &profile.height,
                240,
                16384,
            ),
        ] {
            if !input
                .read(cx)
                .value()
                .trim()
                .parse::<u32>()
                .is_ok_and(|value| (minimum..=maximum).contains(&value))
            {
                return Some((
                    field,
                    format!("{label} must be a whole number between {minimum} and {maximum}."),
                ));
            }
        }
        if self.tab == Tab::Onboarding {
            if !profile
                .memory
                .read(cx)
                .value()
                .trim()
                .parse::<u32>()
                .is_ok_and(|value| (256..=65536).contains(&value))
            {
                return Some((
                    WizardField::Memory,
                    "Maximum memory must be between 256 and 65536 MiB.".into(),
                ));
            }
            if let Err(error) = super::parse_arguments(&profile.jvm.read(cx).value()) {
                return Some((WizardField::Jvm, error.to_string()));
            }
        }
        None
    }

    pub(super) fn advance_game_wizard(&mut self, cx: &Context<'_, Self>) {
        if let Some(page) = self.shell.wizard.page {
            if self.validate_wizard_section(page, cx) {
                self.shell.wizard.section_snapshot = None;
                self.shell.wizard.clear_error();
                self.shell.wizard.next();
            }
            return;
        }
        let needs_instance = self.shell.wizard.step == WizardStep::Setup
            && self.shell.wizard.kind == CreationKind::NewInstance;
        if needs_instance && !self.validate_instance_draft(cx) {
            self.set_wizard_error(self.status.clone(), cx);
            return;
        }
        if (self.shell.wizard.step == WizardStep::Profile
            || !self.shell.wizard.separated && self.shell.wizard.step == WizardStep::Setup)
            && !self.validate_wizard_connection(cx)
        {
            return;
        }
        let state = &self.shell.wizard;
        let issue =
            if state.step == WizardStep::Setup && state.kind == CreationKind::ExistingInstance {
                (!self
                    .snapshot
                    .instances
                    .iter()
                    .any(|instance| Some(&instance.id) == state.existing_instance.as_ref()))
                .then(|| "Choose an existing Instance.".to_owned())
            } else if state.step == WizardStep::Profile
                || (!state.separated && state.step == WizardStep::Setup && state.page.is_none())
            {
                state.profile.as_ref().map_or_else(
                    || Some("Profile draft is unavailable. Reopen game setup.".to_owned()),
                    |profile| profile.values(cx).err().map(|error| error.to_string()),
                )
            } else {
                None
            };
        if let Some(issue) = issue {
            self.set_wizard_error(issue, cx);
        } else {
            self.shell.wizard.section_snapshot = None;
            self.shell.wizard.clear_error();
            self.shell.wizard.next();
        }
    }

    pub(super) fn submit_game_wizard(&mut self, cx: &Context<'_, Self>) {
        if self.busy || self.shell.wizard.step != WizardStep::Review {
            return;
        }
        if !self.validate_wizard_connection(cx) {
            self.shell.wizard.step = if self.shell.wizard.separated {
                WizardStep::Profile
            } else {
                WizardStep::Setup
            };
            return;
        }
        let Some(Form::Instance(form)) = &self.form else {
            return;
        };
        let name = form.name.read(cx).value();
        let version = form.version.read(cx).value();
        let directory = form.directory.read(cx).value();
        let loader = form.selected_loader();
        let optifine = form.optifine.as_deref().cloned();
        let kind = self.shell.wizard.kind;
        if kind == CreationKind::NewInstance && !self.validate_instance_draft(cx) {
            return;
        }
        let Some(profile) = &self.shell.wizard.profile else {
            return;
        };
        let values = profile.values(cx);
        let mut values = match values {
            Ok(values) => values,
            Err(error) => {
                self.set_wizard_error(error.to_string(), cx);
                self.shell.wizard.step = if self.shell.wizard.separated {
                    WizardStep::Profile
                } else {
                    WizardStep::Setup
                };
                self.shell.wizard.transition(true);
                return;
            }
        };
        if !self.shell.wizard.separated {
            values.name.clone_from(&name);
        }
        if kind == CreationKind::NewInstance {
            values.target_id = None;
        }
        if kind == CreationKind::ExistingInstance {
            let Some(instance) = self.shell.wizard.existing_instance.clone().filter(|id| {
                self.snapshot
                    .instances
                    .iter()
                    .any(|instance| &instance.id == id)
            }) else {
                self.shell.wizard.error = Some(
                    "The chosen Instance is no longer available. Go back and choose another."
                        .into(),
                );
                return;
            };
            self.begin("Saving your new Profile");
            self.mark_creation();
            if let Some(engine) = &self.engine {
                engine.create_profile_draft(instance, values);
            }
        } else {
            let directory =
                (!directory.trim().is_empty()).then(|| std::path::PathBuf::from(directory.trim()));
            let defaults = values.defaults.clone();
            self.begin_cancellable("Creating your game and preparing verified downloads");
            self.mark_creation();
            if let Some(engine) = &self.engine {
                engine.create_instance_with_profile(
                    None,
                    name,
                    version,
                    directory,
                    loader,
                    optifine,
                    defaults,
                    Some(values),
                    self.cancel.clone(),
                );
            }
        }
    }

    pub(super) fn back_game_wizard(&mut self, cx: &mut Context<'_, Self>) {
        if self.shell.wizard.page.is_some() {
            self.cancel_component_draft(cx);
        } else {
            self.shell.wizard.back();
        }
        self.shell.wizard.clear_error();
    }

    pub(super) fn cancel_component_draft(&mut self, cx: &mut Context<'_, Self>) {
        if let Some(snapshot) = self.shell.wizard.section_snapshot.take()
            && let Some(Form::Instance(form)) = &mut self.form
        {
            form.version
                .update(cx, |field, cx| field.set_value(&snapshot.version, cx));
            form.directory
                .update(cx, |field, cx| field.set_value(&snapshot.directory, cx));
            form.loader = snapshot.loader;
            form.loader_version = snapshot.loader_version;
            form.optifine = snapshot.optifine;
            self.shell.wizard.loader_catalog = snapshot.loader_catalog.map(|mut catalog| {
                catalog.request = uuid::Uuid::new_v4();
                if matches!(catalog.load, CollectionLoad::Loading) {
                    catalog.load = CollectionLoad::Failed(
                        "The previous request was cancelled. Retry to reload builds.".into(),
                    );
                }
                catalog
            });
            self.shell.wizard.version_notice = snapshot.version_notice;
            if let (Some(profile), Some(checkpoint)) =
                (&mut self.shell.wizard.profile, snapshot.profile)
            {
                profile.restore_checkpoint(checkpoint, cx);
            }
        }
        self.shell.wizard.open_page(None);
        self.shell.wizard.loader_request = None;
        self.shell.wizard.optifine_request = None;
    }

    pub(super) fn render_game_wizard(
        &self,
        form: &InstanceForm,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement + use<> {
        let state = &self.shell.wizard;
        let review = state.step == WizardStep::Review;
        let body = self.transition_content(
            super::motion::Scope::Wizard,
            self.game_wizard_transition_body(form, window, cx),
            window,
            cx,
        );
        let show_back = state.page.is_some()
            || if state.separated {
                state.step != WizardStep::GetStarted
            } else {
                review
            };
        let next_label = if state.page.is_some() {
            "Done"
        } else if review {
            if state.kind == CreationKind::ExistingInstance {
                "Create Profile"
            } else {
                "Create game"
            }
        } else {
            "Continue"
        };
        let next_action = if review && state.page.is_none() {
            UiAction::SaveInstance
        } else {
            UiAction::WizardNext
        };
        let footer = div()
            .w_full()
            .flex()
            .justify_between()
            .items_center()
            .gap_3()
            .child(self.button("cancel-instance", "Cancel", UiAction::CancelForm, cx))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .children(
                        show_back
                            .then(|| self.button("wizard-back", "Back", UiAction::WizardBack, cx)),
                    )
                    .child(self.button("wizard-next", next_label, next_action, cx)),
            )
            .into_any_element();
        let steps = if state.separated {
            vec![
                ("Get started", "Choose what to create"),
                (
                    if state.kind == CreationKind::NewInstance {
                        "Instance"
                    } else {
                        "Choose Instance"
                    },
                    "Game files and components",
                ),
                ("Profile", "Make it yours"),
                ("Review", "Check the details"),
            ]
        } else {
            vec![
                ("Game setup", "Name and configure"),
                ("Review", "Check the details"),
            ]
        };
        let current = match state.step {
            WizardStep::GetStarted => 0,
            WizardStep::Setup => usize::from(state.separated),
            WizardStep::Profile => 2,
            WizardStep::Review => {
                if state.separated {
                    3
                } else {
                    1
                }
            }
        };
        div()
            .id("game-wizard")
            .debug_selector(|| "game-wizard".into())
            .size_full()
            .min_h_0()
            .child(self.render_wizard_frame(window, &steps, current, body, footer))
    }

    pub(super) fn render_game_wizard_transition_body(
        &self,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> gpui::AnyElement {
        match &self.form {
            Some(Form::Instance(form)) => self.game_wizard_transition_body(form, window, cx),
            _ => div().into_any_element(),
        }
    }

    fn game_wizard_transition_body(
        &self,
        form: &InstanceForm,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> gpui::AnyElement {
        let state = &self.shell.wizard;
        let narrow = patterns::is_narrow(window.viewport_size());
        let body = self.game_wizard_body(form, window, cx);
        let body = div()
            .flex()
            .flex_col()
            .gap_4()
            .child(body)
            .children(state.error.as_ref().map(|error| {
                div()
                    .flex()
                    .items_start()
                    .gap_2()
                    .text_sm()
                    .text_color(rgb(self.palette.destructive))
                    .child(Icon::Error.render(16., self.palette.destructive))
                    .child(error.clone())
            }))
            .children(self.busy.then(|| note(self.status.clone())))
            .into_any_element();
        div()
            .id("wizard-body")
            .debug_selector(|| "wizard-body".into())
            .size_full()
            .min_h_0()
            .scroll_area(&state.body_scroll, "Game setup", self.palette, false)
            .child(
                div()
                    .w_full()
                    .max_w(px(608.))
                    .mx_auto()
                    .p(px(if narrow { 16. } else { 32. }))
                    .child(body),
            )
            .into_any_element()
    }

    pub(super) fn game_wizard_body(
        &self,
        form: &InstanceForm,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> gpui::AnyElement {
        let state = &self.shell.wizard;
        let review = state.step == WizardStep::Review;
        let page = state.page.filter(|_| {
            !self.shell.surfaces.layers().any(|entry| {
                matches!(
                    entry.kind,
                    super::shell::Surface::WizardSection(_)
                        | super::shell::Surface::OnboardingSection(_)
                )
            }) || self.shell.surfaces.top().is_some_and(|entry| {
                matches!(
                    entry.kind,
                    super::shell::Surface::WizardSection(_)
                        | super::shell::Surface::OnboardingSection(_)
                )
            })
        });
        match page {
            Some(WizardPage::Version) => self.wizard_version(form, cx).into_any_element(),
            Some(WizardPage::Loader) => self.wizard_loader(form, cx).into_any_element(),
            Some(WizardPage::OptiFine) => self.wizard_optifine(form, cx).into_any_element(),
            Some(WizardPage::Location) => self.wizard_location(form, cx).into_any_element(),
            None if review => div()
                .size_full()
                .min_h_0()
                .child(super::scroll_area::viewport(
                    div()
                        .id("wizard-review-scroll")
                        .w_full()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(self.wizard_review(form, cx)),
                    &self.shell.wizard.body_scroll,
                    "Review",
                    self.palette,
                    false,
                ))
                .into_any_element(),
            None if state.step == WizardStep::GetStarted => {
                self.wizard_start(cx).into_any_element()
            }
            None if state.step == WizardStep::Profile => {
                self.wizard_profile(window, cx, true).into_any_element()
            }
            None if state.kind == CreationKind::ExistingInstance => {
                self.wizard_instance_picker(cx).into_any_element()
            }
            None => self.wizard_setup(form, window, cx).into_any_element(),
        }
    }

    /// Shared fixed rail and action footer for game setup and first-run setup.
    pub(super) fn render_wizard_frame(
        &self,
        window: &Window,
        steps: &[(&str, &str)],
        current: usize,
        body: gpui::AnyElement,
        footer: gpui::AnyElement,
    ) -> gpui::AnyElement {
        let viewport = window.viewport_size();
        let rail = self.wizard_step_rail(steps, current, window);
        div()
            .id("wizard-frame")
            .size_full()
            .min_h_0()
            .flex()
            .child(rail)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(div().flex_1().min_h_0().child(body))
                    .child(
                        patterns::management_footer(viewport, self.palette)
                            .id("wizard-footer")
                            .debug_selector(|| "wizard-footer".into())
                            .child(
                                div()
                                    .w_full()
                                    .max_w(px(608.))
                                    .mx_auto()
                                    .flex()
                                    .flex_col()
                                    .child(footer),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn wizard_step_rail(
        &self,
        steps: &[(&str, &str)],
        current: usize,
        window: &Window,
    ) -> gpui::AnyElement {
        let narrow = patterns::is_narrow(window.viewport_size());
        let mut rail = div()
            .id("wizard-rail")
            .debug_selector(|| "wizard-rail".into())
            .min_w_0()
            .h_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .border_r_1()
            .border_color(rgb(self.palette.border))
            .bg(rgb(self.palette.sidebar));
        if narrow {
            rail = rail.w(px(48.));
        } else {
            rail = rail.w(gpui::relative(BROWSER_FRACTION));
        }
        rail.child(
            div()
                .id("wizard-steps")
                .flex_1()
                .min_h_0()
                .scroll_area(
                    &self.shell.wizard.rail_scroll,
                    "Game setup steps",
                    self.palette,
                    false,
                )
                .px(px(if narrow { 9. } else { 24. }))
                .py_6()
                .flex()
                .flex_col()
                .gap_6()
                .children((!narrow).then(|| note("Game setup")))
                .child(components::stepper(
                    "game-wizard-steps",
                    "Create a game",
                    &steps
                        .iter()
                        .enumerate()
                        .map(|(index, (title, description))| components::StepperStep {
                            title,
                            description,
                            complete: index < current,
                        })
                        .collect::<Vec<_>>(),
                    current,
                    narrow,
                    self.palette,
                )),
        )
        .child(
            patterns::management_footer(window.viewport_size(), self.palette)
                .px(px(if narrow { 8. } else { 24. }))
                .child(note(if narrow {
                    format!("{}/{}", current + 1, steps.len())
                } else {
                    format!("Step {} of {}", current + 1, steps.len())
                })),
        )
        .into_any_element()
    }

    fn wizard_radio_group(
        &self,
        id: &'static str,
        label: &str,
        options: Vec<components::RadioOption<UiAction>>,
        cx: &Context<'_, Self>,
    ) -> components::RadioGroup<UiAction> {
        components::RadioGroup::new(
            id,
            label.to_owned(),
            options,
            &self.shell.wizard.version_focus,
            cx,
            cx.listener(|view, action: &UiAction, window, cx| {
                view.handle(action.clone(), window, cx);
            }),
        )
    }

    fn wizard_start(&self, cx: &Context<'_, Self>) -> gpui::Div {
        let state = &self.shell.wizard;
        let group = self.wizard_radio_group(
            "wizard-creation-kind",
            "Creation type",
            vec![
                components::RadioOption {
                    id: "instance".into(),
                    checked: state.kind == CreationKind::NewInstance,
                    disabled: self.busy,
                    action: UiAction::WizardKind(true),
                },
                components::RadioOption {
                    id: "profile".into(),
                    checked: state.kind == CreationKind::ExistingInstance,
                    disabled: self.busy || self.snapshot.instances.is_empty(),
                    action: UiAction::WizardKind(false),
                },
            ],
            cx,
        );
        div().flex().flex_col().gap_4()
            .child(div().text_xl().child("What would you like to create?"))
            .child(note("Start fresh or add a new way to play an existing game."))
            .child(components::badge("Separated Profile", self.palette, BadgeVariant::Outline))
            .child(group.container().flex().flex_col().gap_4()
                .child(group.item("instance", components::radio("wizard-kind-instance", "New Instance + Profile", state.kind == CreationKind::NewInstance,
                    self.palette, ControlState { disabled: self.busy, ..ControlState::default() },
                    cx.listener(|view, &(): &(), window, cx| view.handle(UiAction::WizardKind(true), window, cx)))
                    .w_full().rounded(px(14.)).border_color(rgb(self.palette.border)).p_4().child(Icon::Blocks.render(20., self.palette.muted))))
                .child(note("A fresh Minecraft setup, with your first Profile ready to go."))
                .child(group.item("profile", components::radio("wizard-kind-profile", "New Profile", state.kind == CreationKind::ExistingInstance,
                    self.palette, ControlState { disabled: self.busy || self.snapshot.instances.is_empty(), ..ControlState::default() },
                    cx.listener(|view, &(): &(), window, cx| view.handle(UiAction::WizardKind(false), window, cx)))
                    .w_full().rounded(px(14.)).border_color(rgb(self.palette.border)).p_4().child(Icon::Layers.render(20., self.palette.muted))))
                .child(note(if self.snapshot.instances.is_empty() { "Create an Instance first. New Profiles can reuse it later without duplicating game files." } else { "Reuse an existing Instance with independent appearance and launch settings. Game files are shared." })))
    }

    fn wizard_instance_picker(&self, cx: &Context<'_, Self>) -> gpui::Div {
        let group = self.wizard_radio_group(
            "wizard-instance-group",
            "Existing game Instance",
            self.snapshot
                .instances
                .iter()
                .map(|instance| components::RadioOption {
                    id: instance.id.clone(),
                    checked: Some(&instance.id) == self.shell.wizard.existing_instance.as_ref(),
                    disabled: self.busy,
                    action: UiAction::WizardInstance(instance.id.clone()),
                })
                .collect(),
            cx,
        );
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(div().text_xl().child("Choose a game Instance"))
            .child(note(
                "Your new Profile uses these existing game files and components.",
            ))
            .child(group.container().flex().flex_col().gap_4().children(
                self.snapshot.instances.iter().map(|instance| {
                    let id = instance.id.clone();
                    let count = self
                        .snapshot
                        .profiles
                        .iter()
                        .filter(|profile| profile.instance_id == id)
                        .count();
                    group.item(
                        &instance.id,
                        components::radio(
                            format!("wizard-instance-{id}"),
                            instance.name.clone(),
                            Some(&id) == self.shell.wizard.existing_instance.as_ref(),
                            self.palette,
                            ControlState {
                                disabled: self.busy,
                                ..ControlState::default()
                            },
                            cx.listener(move |view, &(): &(), window, cx| {
                                view.handle(UiAction::WizardInstance(id.clone()), window, cx);
                            }),
                        )
                        .w_full()
                        .rounded(px(14.))
                        .border_color(rgb(self.palette.border))
                        .p_3()
                        .child(div().flex_1())
                        .child(components::badge(
                            instance.minecraft_version.clone(),
                            self.palette,
                            BadgeVariant::Secondary,
                        ))
                        .child(note(format!("{count} Profiles"))),
                    )
                }),
            ))
            .children(self.snapshot.instances.is_empty().then(|| {
                patterns::empty_state(
                    "No Instances",
                    "Go back and create a new Instance with its first Profile.",
                    self.palette,
                )
            }))
    }

    fn wizard_profile(
        &self,
        window: &Window,
        cx: &Context<'_, Self>,
        show_name: bool,
    ) -> gpui::Div {
        self.reconcile_wizard_disclosure(cx);
        let Some(profile) = &self.shell.wizard.profile else {
            return div();
        };
        div()
            .flex()
            .flex_col()
            .gap_4()
            .children(show_name.then(|| div().text_xl().child("Your way to play")))
            .children(show_name.then(|| {
                self.wizard_field(
                    WizardField::ProfileName,
                    "Profile name",
                    &profile.name,
                    None,
                )
            }))
            .child(
                div()
                    .flex()
                    .items_end()
                    .gap_3()
                    .child(components::profile_avatar(
                        profile.appearance,
                        self.palette,
                        40.,
                    ))
                    .child(self.wizard_appearance_field(
                        "wizard-profile-icon",
                        "Icon",
                        profile.appearance.icon.label(),
                        false,
                        cx,
                    ))
                    .child(self.wizard_appearance_field(
                        "wizard-profile-color",
                        "Color",
                        profile.appearance.color.label(),
                        true,
                        cx,
                    )),
            )
            .child(self.wizard_server_preferences(profile, cx))
            .children(
                (self.shell.wizard.kind == CreationKind::ExistingInstance)
                    .then(|| self.wizard_profile_targets(profile, cx)),
            )
            .children(
                show_name
                    .then(|| self.wizard_profile_options_trigger("Advanced launch settings", cx)),
            )
            .children(show_name.then(|| {
                super::motion::disclosure(
                    "wizard-advanced",
                    &self.shell.wizard.advanced.borrow(),
                    self.motion_preferences(),
                    self.wizard_profile_advanced(profile, cx).into_any_element(),
                    window,
                    cx,
                )
            }))
            .children((!show_name).then(|| self.wizard_profile_advanced(profile, cx)))
    }

    fn reconcile_wizard_disclosure(&self, cx: &Context<'_, Self>) {
        let open = self
            .shell
            .wizard
            .profile
            .as_ref()
            .is_some_and(|profile| profile.options);
        self.shell.wizard.advanced.borrow_mut().set_open(
            open,
            cx.background_executor().now(),
            self.motion_preferences(),
        );
    }

    fn wizard_profile_options_trigger(
        &self,
        label: &'static str,
        cx: &Context<'_, Self>,
    ) -> components::Control {
        super::motion::disclosure_trigger(
            "wizard-profile-options",
            label,
            &self.shell.wizard.advanced.borrow(),
            self.palette,
            self.motion_preferences(),
            cx,
            cx.listener(|view, (): &(), window, cx| {
                view.handle(UiAction::WizardProfileOptions, window, cx);
            }),
        )
    }

    /// The selected version's automatic connection form, resolved from its real
    /// official manifest or the official catalog entry.
    fn wizard_automatic_connection(
        &self,
        cx: &gpui::App,
    ) -> Option<block_launcher::catalog::AutomaticConnection> {
        let version = if self.shell.wizard.kind == CreationKind::ExistingInstance {
            self.snapshot
                .instances
                .iter()
                .find(|instance| Some(&instance.id) == self.shell.wizard.existing_instance.as_ref())
                .map(|instance| instance.minecraft_version.clone())
        } else if let Some(Form::Instance(form)) = &self.form {
            Some(form.version.read(cx).value())
        } else {
            None
        };
        version.and_then(|version| self.automatic_connection(&version))
    }

    fn wizard_automatic_connection_supported(&self, cx: &gpui::App) -> bool {
        self.wizard_automatic_connection(cx)
            .is_some_and(|connection| {
                connection != block_launcher::catalog::AutomaticConnection::Unsupported
            })
    }

    fn wizard_server_preferences(
        &self,
        profile: &ProfileDraft,
        cx: &Context<'_, Self>,
    ) -> gpui::Div {
        let connection = self.wizard_automatic_connection(cx);
        let supported = self.wizard_automatic_connection_supported(cx);
        let has_address = !profile.server.read(cx).value().trim().is_empty();
        let mut fields = div()
            .flex()
            .flex_col()
            .gap_3()
            .child(self.wizard_field(
                WizardField::Server,
                "Server address (optional)",
                &profile.server,
                Some("A hostname or IP address, with an optional port. Saving does not connect."),
            ))
            .child(components::checkbox(
                "wizard-profile-connect",
                "Connect to this server automatically on launch",
                profile.connect_on_launch,
                self.palette,
                ControlState {
                    disabled: self.busy
                        || (!supported || !has_address) && !profile.connect_on_launch,
                    ..Default::default()
                },
                cx.listener(|view, (): &(), window, cx| {
                    view.handle(UiAction::WizardProfileConnect, window, cx);
                }),
            ));
        if !supported {
            fields = fields.child(note(match connection {
                Some(block_launcher::catalog::AutomaticConnection::Unsupported) => {
                    "This version’s client does not accept an automatic server connection. The address can still be saved."
                }
                _ => "The address can be saved. Automatic connection stays unavailable until this version’s official manifest is available.",
            }));
        }
        fields
    }

    fn validate_wizard_connection(&mut self, cx: &Context<'_, Self>) -> bool {
        if self
            .shell
            .wizard
            .profile
            .as_ref()
            .is_some_and(|profile| profile.connect_on_launch)
            && !self.wizard_automatic_connection_supported(cx)
        {
            let reason = match self.wizard_automatic_connection(cx) {
                Some(block_launcher::catalog::AutomaticConnection::Unsupported) => {
                    "This version does not accept an automatic server connection."
                }
                _ => {
                    "This version’s official manifest is unavailable, so automatic connection cannot start."
                }
            };
            self.set_wizard_error(
                format!("{reason} Turn off automatic connection to continue."),
                cx,
            );
            self.shell.wizard.error_field = Some(WizardField::Server);
            self.shell.wizard.reveal.set(Some(WizardField::Server));
            false
        } else {
            true
        }
    }

    fn wizard_appearance_field(
        &self,
        id: &'static str,
        label: &'static str,
        value: &'static str,
        color: bool,
        cx: &Context<'_, Self>,
    ) -> gpui::Div {
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_1()
            .child(note(label))
            .child(
                self.button(
                    id,
                    value,
                    UiAction::ChooseAppearance {
                        wizard: true,
                        color,
                    },
                    cx,
                )
                .w_full()
                .min_w_0(),
            )
    }

    fn wizard_profile_targets(&self, profile: &ProfileDraft, cx: &Context<'_, Self>) -> gpui::Div {
        let instance =
            self.snapshot.instances.iter().find(|instance| {
                Some(&instance.id) == self.shell.wizard.existing_instance.as_ref()
            });
        let targets = instance
            .into_iter()
            .flat_map(|instance| instance.targets.iter())
            .collect::<Vec<_>>();
        let group = self.wizard_radio_group(
            "wizard-target-group",
            "Launch target",
            targets
                .iter()
                .map(|target| components::RadioOption {
                    id: target.id.clone(),
                    checked: profile.target_id == target.id,
                    disabled: self.busy,
                    action: UiAction::WizardProfileTarget(target.id.clone()),
                })
                .collect(),
            cx,
        );
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(note("Launch target"))
            .child(
                group
                    .container()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .children(targets.into_iter().map(|target| {
                        let id = target.id.clone();
                        let name = target.display_name();
                        group.item(
                            &target.id,
                            components::radio(
                                format!("wizard-target-{id}"),
                                name,
                                profile.target_id == id,
                                self.palette,
                                ControlState {
                                    disabled: self.busy,
                                    ..ControlState::default()
                                },
                                cx.listener(move |view, &(): &(), window, cx| {
                                    view.handle(
                                        UiAction::WizardProfileTarget(id.clone()),
                                        window,
                                        cx,
                                    );
                                }),
                            )
                            .child(note(match &target.status {
                                block_launcher::catalog::InstallStatus::Ready => "Installed",
                                block_launcher::catalog::InstallStatus::Pending => {
                                    "Installs before play"
                                }
                                block_launcher::catalog::InstallStatus::Installing => "Installing",
                                block_launcher::catalog::InstallStatus::Failed { .. } => {
                                    "Installation needs retry"
                                }
                            })),
                        )
                    })),
            )
    }

    fn wizard_profile_advanced(&self, profile: &ProfileDraft, cx: &Context<'_, Self>) -> gpui::Div {
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(self.wizard_field(
                WizardField::Memory,
                "Maximum memory (MiB)",
                &profile.memory,
                Some("1024 MiB = 1 GiB. Supported range: 256–65536 MiB."),
            ))
            .child(self.wizard_field(WizardField::Jvm, "Extra JVM arguments", &profile.jvm, None))
            .children(
                (self.shell.wizard.kind == CreationKind::ExistingInstance)
                    .then(|| self.wizard_runtime_window(profile, cx)),
            )
    }

    fn wizard_runtime_window(&self, profile: &ProfileDraft, cx: &Context<'_, Self>) -> gpui::Div {
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(self.product_select_field(
                "wizard-java-runtime",
                "Java runtime",
                if profile.managed {
                    "Automatic Java"
                } else {
                    "Custom executable"
                },
                super::select::ProductChoice::WizardRuntime,
                cx,
            ))
            .children((!profile.managed).then(|| {
                self.wizard_field(WizardField::Java, "Java executable", &profile.java, None)
            }))
            .child(self.product_select_field(
                "wizard-window-size",
                "Window size",
                &format!(
                    "{} × {}{}",
                    profile.width.read(cx).value(),
                    profile.height.read(cx).value(),
                    if profile.window_custom {
                        " · Custom"
                    } else {
                        ""
                    }
                ),
                super::select::ProductChoice::WizardWindow,
                cx,
            ))
            .children(profile.window_custom.then(|| {
                div()
                    .flex()
                    .gap_3()
                    .child(
                        self.wizard_field(
                            WizardField::Width,
                            "Window width (px)",
                            &profile.width,
                            None,
                        )
                        .flex_1(),
                    )
                    .child(
                        self.wizard_field(
                            WizardField::Height,
                            "Window height (px)",
                            &profile.height,
                            None,
                        )
                        .flex_1(),
                    )
            }))
            .child(components::checkbox(
                "wizard-fullscreen",
                "Start in fullscreen",
                profile.fullscreen,
                self.palette,
                ControlState {
                    disabled: self.busy,
                    ..ControlState::default()
                },
                cx.listener(|view, &(): &(), window, cx| {
                    view.handle(UiAction::WizardProfileFullscreen, window, cx);
                }),
            ))
    }

    fn wizard_setup(
        &self,
        form: &InstanceForm,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement + use<> {
        self.reconcile_wizard_disclosure(cx);
        let version = form.version.read(cx).value();
        let version_kind = self
            .version_catalog
            .iter()
            .find(|entry| entry.id == version.trim())
            .map_or("Exact version ID", |entry| {
                Self::wizard_version_kind(entry.kind)
            });
        let configuration = self.wizard_configuration_summary(cx);
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(div().text_xl().child("Create your game"))
            .child(note(
                "Choose the game components. Review everything before installation begins.",
            ))
            .child(self.wizard_field(
                WizardField::GameName,
                "Game name",
                &form.name,
                Some("Required. Up to 160 UTF-8 bytes; no control characters."),
            ))
            .child(
                div()
                    .id("wizard-components")
                    .debug_selector(|| "wizard-components".into())
                    .grid()
                    .grid_cols(if window.viewport_size().width < px(760.) {
                        2
                    } else {
                        4
                    })
                    .gap_3()
                    .child(self.wizard_component(
                        "version",
                        "Minecraft version",
                        (&version, version_kind),
                        WizardPage::Version,
                        cx,
                    ))
                    .child(self.wizard_component(
                        "loader",
                        "Mod loader",
                        (form.loader.name(), form.loader_version.as_deref().unwrap_or("Original game")),
                        WizardPage::Loader,
                        cx,
                    ))
                    .child(self.wizard_optifine_component(form, cx))
                    .child(self.wizard_component(
                        "location",
                        "Configuration",
                        (&configuration.0, &configuration.1),
                        WizardPage::Location,
                        cx,
                    )),
            )
            .child(note("Open a tile to configure it. Minecraft version and Mod loader become fixed when this Instance is created."))
            .children(self.shell.wizard.version_notice.as_ref().map(|notice| self.wizard_version_notice(notice)))
            .children((!self.shell.wizard.separated).then(|| self.wizard_profile_options_trigger("Appearance & launch settings", cx)))
            .children((!self.shell.wizard.separated).then(|| {
                let content = self.wizard_profile(window, cx, false).into_any_element();
                super::motion::disclosure("wizard-appearance-launch", &self.shell.wizard.advanced.borrow(), self.motion_preferences(), content, window, cx)
            }))
    }

    pub(super) fn wizard_configuration_summary(&self, cx: &gpui::App) -> (String, String) {
        self.shell.wizard.profile.as_ref().map_or_else(
            || ("Automatic Java".to_owned(), "Default window".to_owned()),
            |profile| {
                (
                    if profile.managed {
                        "Automatic Java"
                    } else {
                        "Custom Java"
                    }
                    .to_owned(),
                    if profile.fullscreen {
                        "Fullscreen".to_owned()
                    } else {
                        format!(
                            "{} × {}",
                            profile.width.read(cx).value(),
                            profile.height.read(cx).value()
                        )
                    },
                )
            },
        )
    }

    pub(super) fn wizard_component(
        &self,
        id: &str,
        title: &str,
        summary: (&str, &str),
        page: WizardPage,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement + use<> {
        let (value, detail) = summary;
        let icon = match page {
            WizardPage::Version => Icon::Games,
            WizardPage::Loader => Icon::Download,
            WizardPage::OptiFine => Icon::Experiment,
            WizardPage::Location => Icon::Settings,
        };
        let mut card = components::button(
            format!("wizard-component-{id}"),
            "",
            self.palette,
            ButtonVariant::Outline,
            ControlState {
                disabled: self.busy,
                ..ControlState::default()
            },
            cx.listener(move |view, &(): &(), window, cx| {
                view.handle(UiAction::WizardPage(Some(page)), window, cx);
            }),
        )
        // Grid stretch supplies the column width. A percentage width makes
        // Taffy's intrinsic aspect-ratio row use the entire grid's width.
        .w_auto()
        .h_auto()
        .min_w_0()
        .min_h_0()
        .flex()
        .flex_col()
        .items_start()
        .justify_between()
        .gap_2()
        .p_3()
        .child(
            div()
                .w_full()
                .flex()
                .items_center()
                .justify_between()
                .child(icon.render(20., self.palette.muted))
                .child(Icon::ChevronRight.render(14., self.palette.muted)),
        )
        .child(
            div()
                .w_full()
                .min_w_0()
                .flex()
                .flex_col()
                .gap_1()
                .child(note(title.to_owned()))
                .child(
                    div()
                        .w_full()
                        .min_w_0()
                        .text_sm()
                        .truncate()
                        .child(value.to_owned()),
                )
                .child(
                    div()
                        .w_full()
                        .min_w_0()
                        .text_size(px(10.))
                        .line_height(px(14.))
                        .text_color(rgb(self.palette.muted))
                        .truncate()
                        .child(detail.to_owned()),
                ),
        );
        card.style().aspect_ratio = Some(1.);
        super::tooltip::trigger(card, format!("{title}: {value} · {detail}"), cx)
    }

    pub(super) fn wizard_unavailable(
        &self,
        id: &str,
        title: &str,
        reason: &str,
        square: bool,
    ) -> impl IntoElement + use<> {
        let mut card = components::button(
            format!("wizard-unavailable-{id}"),
            "",
            self.palette,
            ButtonVariant::Outline,
            ControlState {
                disabled: true,
                ..ControlState::default()
            },
            |&(), _, _| {},
        )
        .w_auto()
        .h_auto()
        .min_w_0()
        .min_h_0()
        .rounded_md()
        .border_1()
        .border_color(rgb(self.palette.border))
        .p_3()
        .flex()
        .flex_col()
        .justify_between()
        .gap_2()
        .child(Icon::Info.render(20., self.palette.muted))
        .child(
            div()
                .min_w_0()
                .w_full()
                .whitespace_normal()
                .child(title.to_owned())
                .child(note(reason.to_owned()).w_full().line_clamp(3)),
        );
        if square {
            card.style().aspect_ratio = Some(1.);
        }
        card
    }

    fn matching_wizard_versions(
        &self,
        form: &InstanceForm,
        cx: &gpui::App,
    ) -> Vec<(String, Option<minecraft_manifest::model::VersionType>)> {
        let selected = form.version.read(cx).value();
        let query = self
            .shell
            .wizard
            .version_search
            .as_ref()
            .map_or_else(String::new, |search| search.read(cx).value())
            .trim()
            .to_lowercase();
        if self.version_catalog.is_empty() {
            return self
                .versions
                .iter()
                .filter(|id| id.to_lowercase().contains(&query))
                .map(|id| (id.clone(), None))
                .collect();
        }
        self.version_catalog
            .iter()
            .filter(|entry| {
                (self.shell.wizard.include_snapshots
                    || entry.kind == minecraft_manifest::model::VersionType::Release
                    || entry.id == selected)
                    && entry.id.to_lowercase().contains(&query)
            })
            .map(|entry| (entry.id.clone(), Some(entry.kind)))
            .collect()
    }

    fn move_wizard_version(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.busy || event.keystroke.modifiers.modified() {
            return;
        }
        if self
            .shell
            .wizard
            .version_search
            .as_ref()
            .is_some_and(|search| search.read(cx).is_composing())
        {
            return;
        }
        if !matches!(event.keystroke.key.as_str(), "up" | "down" | "home" | "end") {
            return;
        }
        let Some(Form::Instance(form)) = &self.form else {
            return;
        };
        let versions: Vec<_> = self
            .matching_wizard_versions(form, cx)
            .into_iter()
            .filter(|(version, _)| self.wizard_version_restriction(version).is_none())
            .collect();
        if versions.is_empty() {
            return;
        }
        let selected = form.version.read(cx).value();
        let from_row = self
            .shell
            .wizard
            .version_focus
            .borrow()
            .values()
            .any(|focus| focus.is_focused(window));
        let current = versions
            .iter()
            .position(|(version, _)| *version == selected);
        let index = match event.keystroke.key.as_str() {
            "home" => 0,
            "end" => versions.len() - 1,
            "up" => current.map_or(versions.len() - 1, |index| index.saturating_sub(1)),
            _ => current.map_or(0, |index| (index + 1).min(versions.len() - 1)),
        };
        let selected = versions[index].0.clone();
        self.handle(UiAction::SelectVersion(selected.clone()), window, cx);
        if from_row
            && let Some(focus) = self
                .shell
                .wizard
                .version_focus
                .borrow()
                .get(&format!("wizard-version-list:{selected}"))
        {
            focus.focus(window, cx);
        }
        self.shell.wizard.version_scroll.scroll_to_item(index);
        cx.stop_propagation();
    }

    pub(super) fn wizard_version(
        &self,
        form: &InstanceForm,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement + use<> {
        let versions = self.matching_wizard_versions(form, cx);
        let count = versions.len();
        let result = if count == 1 { "result" } else { "results" };

        div().id("wizard-version-picker").flex().flex_col().gap_4()
            .on_key_down(cx.listener(Self::move_wizard_version))
            .child(div().text_xl().child("Minecraft version"))
            .child(note("Version and loader become fixed when this Instance is created."))
            .child(self.wizard_version_catalog_status(cx))
            .children(self.shell.wizard.version_search.as_ref().map(|search| components::field_with(
                "Search versions", search, self.palette,
                FieldOptions { hint: Some("Use Up/Down to choose a result. Tab into the list to use Home/End; the search field keeps normal text editing.".into()), ..FieldOptions::default() },
            )))
            .child(components::checkbox("wizard-snapshots", "Include snapshots and early builds", self.shell.wizard.include_snapshots,
                self.palette, ControlState { disabled: self.busy || self.version_catalog.is_empty(), ..ControlState::default() },
                cx.listener(|view, &(): &(), window, cx| view.handle(UiAction::WizardSnapshots, window, cx))))
            .child(div().id("wizard-version-result-count")
                .text_xs().text_color(rgb(self.palette.muted)).child(format!("{count} {result}")))
            .children(self.wizard_version_legacy_note())
            .child(self.wizard_version_list(form, versions, cx))
            .child(components::field_with("Selected version ID", &form.version, self.palette, FieldOptions {
                required: true,
                hint: Some("Enter an exact official version ID. Installation verifies the catalog before downloading. Loader builds are filtered for this version.".into()), ..FieldOptions::default()
            }))
            .child(self.wizard_version_metadata(form, cx))
            .children(self.shell.wizard.version_notice.as_ref().map(|notice| self.wizard_version_notice(notice)))
    }

    fn wizard_version_catalog_status(&self, cx: &Context<'_, Self>) -> gpui::Stateful<gpui::Div> {
        use super::browser::CollectionLoad;
        let previous = !self.version_catalog.is_empty();
        let (title, detail, retry) = match &self.version_load {
            CollectionLoad::Loading => ("Loading official version catalog", if previous {
                "Previously loaded versions remain available while refreshing. Your draft selection is preserved.".to_owned()
            } else { "Official metadata is loading. Your draft is preserved; the selected version’s source is shown below.".to_owned() }, None),
            CollectionLoad::Failed(error) => ("Version catalog unavailable", format!("{error}. {}", if previous {
                "Previously loaded versions remain available; the current draft is preserved."
            } else { "Your current draft is kept. Retry to fetch official version metadata." }), Some("Retry catalog")),
            CollectionLoad::Ready if !previous => ("Official metadata is unavailable", "No official metadata is available yet. Release types remain unverified until it loads.".to_owned(), Some("Reload catalog")),
            CollectionLoad::Ready => ("Official version catalog", format!("{} versions loaded. The selected version’s type comes from this metadata.", self.version_catalog.len()), Some("Refresh catalog")),
        };

        let loading = matches!(self.version_load, CollectionLoad::Loading);
        if matches!(self.version_load, CollectionLoad::Ready) && previous {
            return div()
                .id("wizard-version-catalog-status")
                .flex()
                .items_center()
                .justify_between()
                .gap_2()
                .child(note(format!(
                    "Official catalog · {} versions",
                    self.version_catalog.len()
                )))
                .child(self.button(
                    "wizard-retry-versions",
                    "Refresh catalog",
                    UiAction::ReloadVersions,
                    cx,
                ));
        }
        components::card(self.palette)
            .id("wizard-version-catalog-status")
            .p_3()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        if loading { Icon::Loading } else { Icon::Info }
                            .render(14., self.palette.muted),
                    )
                    .child(div().text_sm().child(title)),
            )
            .child(note(detail))
            .children(retry.map(|label| {
                self.button("wizard-retry-versions", label, UiAction::ReloadVersions, cx)
            }))
    }

    fn wizard_version_metadata(
        &self,
        form: &InstanceForm,
        cx: &gpui::App,
    ) -> gpui::Stateful<gpui::Div> {
        let selected = form.version.read(cx).value();
        let entry = self
            .version_catalog
            .iter()
            .find(|entry| entry.id == selected.trim());
        let source = entry.map_or_else(|| {
            if selected.trim().is_empty() { "Choose or enter a Minecraft version.".into() }
            else if self.version_catalog.is_empty() && selected.trim() == "1.20.6" {
                "Compatible installer default · official release metadata not loaded".into()
            } else if self.version_catalog.is_empty() {
                "Exact version ID · release type unverified until official metadata loads".into()
            } else { "Exact version ID · not found in the loaded official catalog; installation will verify it".into() }
        }, |entry| format!("Official catalog · {}", Self::wizard_version_kind(entry.kind)));

        div()
            .id("wizard-selected-version-source")
            .flex()
            .flex_col()
            .gap_2()
            .children(entry.map(|entry| {
                components::badge(
                    Self::wizard_version_kind(entry.kind),
                    self.palette,
                    if entry.kind == minecraft_manifest::model::VersionType::Release {
                        BadgeVariant::Success
                    } else {
                        BadgeVariant::Warning
                    },
                )
            }))
            .child(note(source))
    }

    pub(super) const fn wizard_version_kind(
        kind: minecraft_manifest::model::VersionType,
    ) -> &'static str {
        use minecraft_manifest::model::VersionType;
        match kind {
            VersionType::Release => "Stable release",
            VersionType::Snapshot => "Snapshot",
            VersionType::OldBeta => "Old beta",
            VersionType::OldAlpha => "Old alpha",
        }
    }

    fn wizard_version_notice(&self, notice: &str) -> gpui::Stateful<gpui::Div> {
        div()
            .id("wizard-version-change-notice")
            .rounded_lg()
            .p_3()
            .bg(rgb(self.palette.secondary))
            .child(note(notice.to_owned()))
    }

    fn wizard_version_list(
        &self,
        form: &InstanceForm,
        versions: Vec<(String, Option<minecraft_manifest::model::VersionType>)>,
        cx: &Context<'_, Self>,
    ) -> super::scroll_area::ScrollArea<gpui_base::RadioGroup> {
        let selected = form.version.read(cx).value();
        let empty = versions.is_empty();
        let group = self
            .wizard_radio_group(
                "wizard-version-list",
                "Minecraft versions",
                versions
                    .iter()
                    .map(|(version, _)| components::RadioOption {
                        id: version.clone(),
                        checked: *version == selected,
                        disabled: self.busy || self.wizard_version_restriction(version).is_some(),
                        action: UiAction::SelectVersion(version.clone()),
                    })
                    .collect(),
                cx,
            )
            .with_scroll(self.shell.wizard.version_scroll.clone());
        group
            .container()
            .debug_selector(|| "wizard-version-list".into())
            .flex_shrink_0()
            .min_h(px(180.))
            .max_h(px(280.))
            .flex()
            .flex_col()
            .gap_2()
            .children(
                versions
                    .into_iter()
                    .enumerate()
                    .map(|(index, (version, kind))| {
                        let active = version == selected;
                        let restriction = self.wizard_version_restriction(&version);
                        let id = version.clone();
                        group.item(
                            &id,
                            components::radio(
                                format!("wizard-version-{index}"),
                                version.clone(),
                                active,
                                self.palette,
                                ControlState {
                                    disabled: self.busy || restriction.is_some(),
                                    ..ControlState::default()
                                },
                                cx.listener(move |view, &(): &(), window, cx| {
                                    view.handle(
                                        UiAction::SelectVersion(version.clone()),
                                        window,
                                        cx,
                                    )
                                }),
                            )
                            .w_full()
                            .min_w_0()
                            .px_3()
                            .py_2()
                            .border_color(rgb(if active {
                                self.palette.primary
                            } else {
                                self.palette.border
                            }))
                            .bg(rgb(if active {
                                self.palette.secondary
                            } else {
                                self.palette.background
                            }))
                            .child(if let Some(reason) = restriction {
                                div().flex_1().min_w_0().child(
                                    div()
                                        .debug_selector(move || {
                                            format!("wizard-version-reason-{index}")
                                        })
                                        .text_xs()
                                        .text_color(rgb(self.palette.muted))
                                        .child(reason),
                                )
                            } else {
                                div().flex_1()
                            })
                            .children(kind.map(|kind| {
                                components::badge(
                                    Self::wizard_version_kind(kind),
                                    self.palette,
                                    if kind == minecraft_manifest::model::VersionType::Release {
                                        BadgeVariant::Success
                                    } else {
                                        BadgeVariant::Warning
                                    },
                                )
                            })),
                        )
                    }),
            )
            .children(empty.then(|| {
                patterns::empty_state(
                    "No matching versions",
                    "Try another search. Your selected version is preserved.",
                    self.palette,
                )
            }))
            .scroll_area(
                &self.shell.wizard.version_scroll,
                "Minecraft versions",
                self.palette,
                false,
            )
            .tab_index(-1)
            .tab_stop(false)
    }

    fn wizard_loader_choice(
        &self,
        name: &'static str,
        kind: Option<LoaderKind>,
        form: &InstanceForm,
        cx: &Context<'_, Self>,
    ) -> components::Control {
        let selected = kind.as_ref() == Some(&form.loader);
        components::radio(
            format!("wizard-loader-{}", name.to_lowercase()),
            name,
            selected,
            self.palette,
            ControlState {
                disabled: self.busy || kind.is_none(),
                ..ControlState::default()
            },
            cx.listener(move |view, &(): &(), window, cx| {
                if let Some(kind) = &kind {
                    view.handle(UiAction::WizardLoader(kind.clone()), window, cx);
                }
            }),
        )
        .w_full()
        .min_w_0()
        .h_auto()
        .min_h(px(40.))
        .px_2()
        .py_2()
        .border_color(rgb(if selected {
            self.palette.primary
        } else {
            self.palette.border
        }))
        .bg(rgb(if selected {
            self.palette.secondary
        } else {
            self.palette.background
        }))
    }

    pub(super) fn wizard_loader(
        &self,
        form: &InstanceForm,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement + use<> {
        let names = [
            ("Vanilla", Some(LoaderKind::Vanilla)),
            ("NeoForge", Some(LoaderKind::NeoForge)),
            ("Fabric", Some(LoaderKind::Fabric)),
            ("Forge", Some(LoaderKind::Forge)),
            ("Quilt", Some(LoaderKind::Quilt)),
        ];
        let group = self.wizard_radio_group(
            "wizard-loader-group",
            "Mod loader",
            names
                .iter()
                .map(|(name, choice)| components::RadioOption {
                    id: (*name).into(),
                    checked: choice.as_ref() == Some(&form.loader),
                    disabled: self.busy || choice.is_none(),
                    action: UiAction::WizardLoader(choice.clone().unwrap_or(LoaderKind::Vanilla)),
                })
                .collect(),
            cx,
        );
        div().flex().flex_col().gap_3()
            .child(div().text_xl().child("Mod loader"))
            .child(note("Choose a loader and build. These choices become fixed when this Instance is created."))
            .child(div().id("wizard-loader-picker").debug_selector(|| "wizard-loader-picker".into())
                .h(px(336.)).min_w_0().flex().overflow_hidden()
                .rounded_xl().border_1().border_color(rgb(self.palette.border))
                .child(group.container().w(gpui::relative(0.32)).min_w(px(76.)).max_w(px(144.))
                    .flex_none().flex().flex_col().gap_2().p_2()
                    .border_r_1().border_color(rgb(self.palette.border)).bg(rgb(self.palette.secondary))
                    .child(note("Loader").px_1())
                    .children(names.into_iter().map(|(name, choice)| group.item(name, self.wizard_loader_choice(name, choice, form, cx)))))
                .child(self.wizard_loader_builds(form, cx)))
            .child(note("Official builds for the selected Minecraft version. Installation verifies every required file."))
    }

    fn wizard_loader_builds(&self, form: &InstanceForm, cx: &Context<'_, Self>) -> gpui::Div {
        let minecraft = form.version.read(cx).value();
        let mut pane = div()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .gap_3()
            .p_3();
        if form.loader == LoaderKind::Vanilla {
            return pane.child(div().flex_1().min_h_0().flex().flex_col().items_center().justify_center().gap_3()
                .child(Icon::Games.render(28., self.palette.muted))
                .child(div().text_sm().whitespace_normal().child("No mod loader needed"))
                .child(note("Play the original game, or choose a supported loader on the left to use mods.").whitespace_normal()));
        }
        let catalog = self
            .shell
            .wizard
            .loader_catalog
            .as_ref()
            .filter(|catalog| catalog.kind == form.loader && catalog.minecraft == minecraft);
        let builds = catalog.map_or(&[][..], |catalog| catalog.builds.as_slice());
        let load = catalog.map_or_else(
            || CollectionLoad::Failed("Load the official catalog to choose a build.".into()),
            |catalog| catalog.load.clone(),
        );
        pane = pane
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .child(format!("{} versions", form.loader.name())),
                    )
                    .child(components::badge(
                        format!("{} builds", builds.len()),
                        self.palette,
                        BadgeVariant::Secondary,
                    )),
            )
            .child(note(format!("Minecraft {minecraft}")));
        match &load {
            CollectionLoad::Loading => {
                pane = pane.child(
                    div()
                        .id("wizard-loader-loading")
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(Icon::Loading.render(14., self.palette.muted))
                        .child(note("Loading official builds…")),
                );
            }
            CollectionLoad::Failed(error) => {
                pane = pane.child(
                    div()
                        .id("wizard-loader-error")
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(note(error.clone()))
                        .child(self.button(
                            "wizard-loader-retry",
                            "Retry",
                            UiAction::ReloadLoaderVersions,
                            cx,
                        )),
                );
            }
            CollectionLoad::Ready if builds.is_empty() => {
                return pane
                    .child(patterns::empty_state(
                        "No compatible builds",
                        "Choose another Minecraft version or loader.",
                        self.palette,
                    ))
                    .child(self.button(
                        "wizard-loader-retry",
                        "Refresh catalog",
                        UiAction::ReloadLoaderVersions,
                        cx,
                    ));
            }
            CollectionLoad::Ready => {}
        }
        pane = pane.children(self.shell.wizard.loader_search.as_ref().map(|input| {
            components::field_with(
                "Search loader builds",
                input,
                self.palette,
                FieldOptions::default(),
            )
        }));
        let query = self
            .shell
            .wizard
            .loader_search
            .as_ref()
            .map_or_else(String::new, |input| input.read(cx).value())
            .trim()
            .to_lowercase();
        let filtered: Vec<_> = builds
            .iter()
            .filter(|build| build.version.to_lowercase().contains(&query))
            .collect();
        let disabled = self.busy || !matches!(load, CollectionLoad::Ready);
        let group = self
            .wizard_radio_group(
                "wizard-loader-build-group",
                "Loader build",
                filtered
                    .iter()
                    .map(|build| components::RadioOption {
                        id: build.version.clone(),
                        checked: form.loader_version.as_deref() == Some(build.version.as_str()),
                        disabled,
                        action: UiAction::WizardLoaderBuild(build.version.clone()),
                    })
                    .collect(),
                cx,
            )
            .with_scroll(self.shell.wizard.loader_scroll.clone());
        pane = pane.child(
            group
                .container()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .gap_2()
                .children(filtered.iter().map(|build| {
                    let version = build.version.clone();
                    let selected = form.loader_version.as_deref() == Some(version.as_str());
                    group.item(
                        &build.version,
                        components::radio(
                            format!("wizard-loader-build-{version}"),
                            version.clone(),
                            selected,
                            self.palette,
                            ControlState {
                                disabled,
                                ..ControlState::default()
                            },
                            cx.listener(move |view, &(): &(), window, cx| {
                                view.handle(
                                    UiAction::WizardLoaderBuild(version.clone()),
                                    window,
                                    cx,
                                )
                            }),
                        )
                        .w_full()
                        .min_w_0()
                        .min_h(px(40.))
                        .px_2()
                        .bg(rgb(if selected {
                            self.palette.secondary
                        } else {
                            self.palette.background
                        }))
                        .child(div().flex_1())
                        .child(components::badge(
                            if build.stable { "Stable" } else { "Preview" },
                            self.palette,
                            if build.stable {
                                BadgeVariant::Secondary
                            } else {
                                BadgeVariant::Warning
                            },
                        )),
                    )
                }))
                .children((filtered.is_empty() && !builds.is_empty()).then(|| {
                    patterns::empty_state(
                        "No matching builds",
                        "Try another search. Your selected build is preserved.",
                        self.palette,
                    )
                }))
                .scroll_area(
                    &self.shell.wizard.loader_scroll,
                    "Loader builds",
                    self.palette,
                    false,
                ),
        );
        pane.child(note(form.loader_version.as_ref().map_or_else(
            || "Choose a compatible build.".into(),
            |version| format!("Selected build: {version}"),
        )))
    }

    pub(super) fn wizard_location(
        &self,
        form: &InstanceForm,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement + use<> {
        self.wizard_configuration(form, cx)
    }

    pub(super) fn wizard_configuration(
        &self,
        form: &InstanceForm,
        cx: &Context<'_, Self>,
    ) -> gpui::Div {
        let mut fields = div()
            .flex()
            .flex_col()
            .gap_4()
            .child(div().text_2xl().child("Configuration"))
            .child(note(
                "Choose this Instance’s game folder, Java runtime and window settings.",
            ))
            .child(self.wizard_field(
                WizardField::Directory,
                "Installation folder (optional)",
                &form.directory,
                Some("Leave empty for automatic storage. Existing player folders are not moved."),
            ))
            .child(self.button(
                "browse-install",
                "Choose folder…",
                UiAction::BrowseInstall,
                cx,
            ));
        if let Some(profile) = &self.shell.wizard.profile {
            fields = fields.child(self.wizard_runtime_window(profile, cx));
            if self.tab == Tab::Onboarding {
                fields = fields.child(self.wizard_profile_advanced(profile, cx));
            }
        }
        fields.child(note(
            "These settings stay in your draft until the final review is saved.",
        ))
    }

    #[expect(
        clippy::too_many_lines,
        reason = "The confirmation keeps all committed values together in one review."
    )]
    fn wizard_review(&self, form: &InstanceForm, cx: &Context<'_, Self>) -> gpui::Div {
        let state = &self.shell.wizard;
        let existing = state.kind == CreationKind::ExistingInstance;
        let chosen = self
            .snapshot
            .instances
            .iter()
            .find(|instance| Some(&instance.id) == state.existing_instance.as_ref());
        let game_name = if existing {
            chosen.map_or_else(
                || "Unavailable Instance".to_owned(),
                |instance| instance.name.clone(),
            )
        } else {
            form.name.read(cx).value()
        };
        let version = if existing {
            chosen.map_or_else(String::new, |instance| instance.minecraft_version.clone())
        } else {
            form.version.read(cx).value()
        };
        let directory = if existing {
            chosen.map_or_else(String::new, |instance| {
                instance.directory.display().to_string()
            })
        } else {
            form.directory.read(cx).value()
        };
        let profile = state.profile.as_ref();
        let profile_name = if state.separated {
            profile.map_or_else(String::new, |profile| profile.name.read(cx).value())
        } else {
            game_name.clone()
        };
        let appearance = self.wizard_appearance();
        let launch_target = if existing {
            chosen
                .and_then(|instance| {
                    instance.targets.iter().find(|target| {
                        profile.is_some_and(|profile| target.id == profile.target_id)
                    })
                })
                .map_or_else(
                    || "Choose a launch target".to_owned(),
                    |target| target.display_name(),
                )
        } else {
            form.loader_label()
        };
        let memory = profile.map_or_else(
            || self.saved_preferences.game.memory_mib.to_string(),
            |profile| profile.memory.read(cx).value(),
        );
        let java = profile.map_or_else(
            || "Automatic · managed by Block".to_owned(),
            |profile| {
                if profile.managed {
                    "Automatic · managed by Block".to_owned()
                } else {
                    profile.java.read(cx).value()
                }
            },
        );
        let resolution = profile.map_or_else(
            || {
                format!(
                    "{} × {}",
                    self.saved_preferences.game.width, self.saved_preferences.game.height
                )
            },
            |profile| {
                format!(
                    "{} × {}{}",
                    profile.width.read(cx).value(),
                    profile.height.read(cx).value(),
                    if profile.fullscreen {
                        " · Fullscreen"
                    } else {
                        ""
                    }
                )
            },
        );
        let automatic_connection = self.automatic_connection(&version);
        div().flex().flex_col().gap_4()
            .child(div().text_xl().child("Ready for your next world?"))
            .child(note(if existing { "Review the new Profile. Existing game files and other Profiles stay in place." } else { "Create game saves your setup and begins verified downloads. You can follow installation on Home." }))
            .child(components::card(self.palette)
                .child(div().flex().items_center().gap_3().child(Icon::Blocks.render(24., self.palette.muted))
                    .child(div().flex_1().min_w_0().child(note(if existing { "EXISTING INSTANCE" } else if state.separated { "NEW INSTANCE" } else { "NEW GAME" }))
                        .child(div().text_sm().whitespace_normal().child(game_name)))
                    .child(components::badge(version, self.palette, BadgeVariant::Secondary)))
                .children(state.separated.then(|| div().border_t_1().border_color(rgb(self.palette.border)).pt_3().flex().items_center().gap_3()
                    .child(components::profile_avatar(appearance, self.palette, 40.))
                    .child(div().min_w_0().child(note("NEW PROFILE")).child(profile_name)))) )
            .children([
                ("Launch target", launch_target),
                ("Maximum memory", memory.parse::<u32>().map_or_else(|_| memory.clone(), components::memory_label)),
                ("Server", profile.map_or_else(|| "Choose in game".to_owned(), |profile| { let value = profile.server.read(cx).value(); if value.trim().is_empty() { "Choose in game".into() } else { value } })),
                ("Automatic connection", if profile.is_some_and(|profile| profile.connect_on_launch) {
                    match automatic_connection {
                        Some(block_launcher::catalog::AutomaticConnection::Legacy) => "On · server arguments",
                        _ => "On · Quick Play",
                    }
                } else { "Off" }.to_owned()),
                ("JVM arguments", profile.map_or_else(|| "Default arguments".to_owned(), |profile| { let value = profile.jvm.read(cx).value(); if value.trim().is_empty() { "Default arguments".into() } else { value } })),
                ("Java runtime", java),
                ("Game window", resolution),
                ("Game files", if directory.is_empty() { "A separate folder in Block’s data directory".to_owned() } else { directory }),
                ("Game data", "Isolated folder for this Profile".to_owned()),
            ].map(|(label,value)| div().flex().flex_col().gap_1().child(note(label)).child(div().text_sm().whitespace_normal().child(value))))
            .child(components::badge("Version and loader fixed after creation", self.palette, BadgeVariant::Outline))
            .children(existing.then(|| note("Only the new Profile is saved. Any missing files for its selected target are installed before play.")))
    }
}
