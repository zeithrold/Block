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
    CancellationToken, catalog::LoaderKind, install::fabric::LoaderVersion, platform::Architecture,
    support::legacy_lwjgl_reason,
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
    design::Icon,
    input, note, patterns,
    scroll_area::ScrollAreaExt as _,
    text_input::TextInput,
};
use crate::Language;

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

enum WizardBodyOwner {
    Outer,
    Section(WizardPage),
}

use crate::i18n::UiText as WizardText;

/// A wizard failure a shared owner raises and clears by identity, never by the
/// text it happens to paint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum WizardFailure {
    /// The installation-folder chooser could not open.
    ChooserFailed,
}

impl WizardFailure {
    const fn key(self) -> &'static str {
        match self {
            Self::ChooserFailed => "wizard.error.chooserFailed",
        }
    }

    fn from_key(key: &str) -> Option<Self> {
        [Self::ChooserFailed]
            .into_iter()
            .find(|failure| failure.key() == key)
    }
}

/// Resolve one product message with its arguments in `language`.
///
/// Painted copy and retained causes share this one substitution rule, so a
/// whole message never leaves an unresolved marker behind.
fn text(language: Language, key: &'static str, args: &[(&'static str, &str)]) -> String {
    let mut message = WizardText::message(key);
    for &(name, value) in args {
        message = message.arg(name, value);
    }
    message.text(language)
}

/// The wizard's frozen and live draft state.
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
    /// A retained product message or verbatim diagnostic.
    pub error: Option<WizardText>,
    version_notice: Option<WizardText>,
    pub separated: bool,
    pub kind: CreationKind,
    pub existing_instance: Option<String>,
    pub profile: Option<ProfileDraft>,
    pub body_scroll: ScrollHandle,
    pub section_scroll: ScrollHandle,
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
        language: Language,
        cx: &mut Context<'_, Block>,
    ) -> Self {
        Self {
            name: input(
                &tr!(language, "wizard.profile.defaultName"),
                &tr!(language, "wizard.profile.namePlaceholder"),
                cx,
            ),
            server: input("", "play.example.com:25565", cx),
            connect_on_launch: false,
            memory: input(&components::memory_gb_text(defaults.memory_mib), "2", cx),
            java: input(
                &defaults.java,
                &tr!(language, "wizard.profile.javaPlaceholder"),
                cx,
            ),
            jvm: input(
                &defaults.jvm_arguments,
                &tr!(language, "wizard.profile.jvmPlaceholder"),
                cx,
            ),
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
        } else if components::parse_memory_gb(&self.memory.read(cx).value()).is_err() {
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
    ) -> Result<super::engine::ProfileDraftValues, WizardText> {
        let name = self.name.read(cx).value().trim().to_owned();
        if name.is_empty() || name.len() > 80 || name.chars().any(char::is_control) {
            return Err(msg!("wizard.profile.defaultError"));
        }
        let defaults = super::settings::GameDefaults {
            memory_mib: components::parse_memory_gb(&self.memory.read(cx).value())
                .map_err(|error| WizardText::message(error.key()))?,
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
                .map_err(|_| msg!("wizard.profile.widthError"))?,
            height: self
                .height
                .read(cx)
                .value()
                .trim()
                .parse()
                .map_err(|_| msg!("wizard.profile.heightError"))?,
            fullscreen: self.fullscreen,
            show_snapshots: false,
        };
        if !self.managed && defaults.java.is_empty() {
            return Err(msg!("wizard.profile.javaError"));
        }
        for (value, minimum, key) in [
            (defaults.width, 320, "wizard.error.widthRange"),
            (defaults.height, 240, "wizard.error.heightRange"),
        ] {
            if !(minimum..=16384).contains(&value) {
                return Err(msg!(key, minimum = minimum, maximum = 16384));
            }
        }
        super::parse_arguments(&defaults.jvm_arguments)
            .map_err(|_| msg!("common.arguments_unclosed_quote"))?;
        defaults
            .validate()
            .map_err(|error| msg!("wizard.profile.validationDetail", detail = error))?;
        let server = self.server_preferences(cx);
        server
            .validate()
            .map_err(|error| msg!("wizard.profile.validationDetail", detail = error))?;
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
    version_notice: Option<WizardText>,
    profile: Option<ProfileCheckpoint>,
}

impl WizardState {
    pub(super) fn refresh_placeholders(&self, language: Language, cx: &mut Context<'_, Block>) {
        for (input, key) in [
            (
                self.version_search.as_ref(),
                "wizard.version.searchPlaceholder",
            ),
            (
                self.loader_search.as_ref(),
                "wizard.loader.searchPlaceholder",
            ),
        ] {
            if let Some(input) = input {
                let placeholder = tr!(language, key);
                input.update(cx, |input, cx| input.set_placeholder(&placeholder, cx));
            }
        }
        if let Some(profile) = &self.profile {
            for (input, key) in [
                (&profile.name, "wizard.profile.namePlaceholder"),
                (&profile.java, "wizard.profile.javaPlaceholder"),
                (&profile.jvm, "wizard.profile.jvmPlaceholder"),
            ] {
                let placeholder = tr!(language, key);
                input.update(cx, |input, cx| input.set_placeholder(&placeholder, cx));
            }
        }
    }

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
        let section_scroll = ScrollHandle::new();
        section_scroll.set_offset(self.section_scroll.offset());
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
            section_scroll,
            rail_scroll,
            error_field: self.error_field,
            reveal: Rc::new(Cell::new(None)),
        }
    }

    /// Clear the painted validation, its field and any pending reveal.
    pub(super) fn clear_error(&mut self) {
        self.error = None;
        self.error_field = None;
        self.reveal.set(None);
    }

    /// Raise a failure by identity, replacing whatever painted before it.
    pub(super) fn set_failure(&mut self, failure: WizardFailure) {
        self.error = Some(WizardText::message(failure.key()));
    }

    /// Clear `failure` if it is still the painted cause, leaving any other
    /// message alone.
    pub(super) fn clear_failure(&mut self, failure: WizardFailure) {
        if self.failure() == Some(failure) {
            self.error = None;
        }
    }

    /// The raised failure, identified by cause rather than by painted text.
    pub(super) fn failure(&self) -> Option<WizardFailure> {
        self.error
            .as_ref()
            .and_then(WizardText::key)
            .and_then(WizardFailure::from_key)
    }

    /// Whether any error would paint, whichever owner set it.
    fn has_error(&self) -> bool {
        self.error.is_some()
    }

    /// Resolve the retained error in the language of the view.
    fn error_text(&self, language: Language) -> Option<String> {
        self.error.as_ref().map(|cause| cause.text(language))
    }

    pub(super) fn new(language: Language, cx: &mut Context<'_, Block>) -> Self {
        Self {
            version_search: Some(input(
                "",
                &tr!(language, "wizard.version.searchPlaceholder"),
                cx,
            )),
            loader_search: Some(input(
                "",
                &tr!(language, "wizard.loader.searchPlaceholder"),
                cx,
            )),
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
            // The Review leads with the decisions; its folded defaults open
            // only when the user asks for them there. Setup shares the wizard's
            // one expansion state, so entering the Review closes that state
            // instead of letting a Setup expansion arrive already open.
            if self.step == WizardStep::Review
                && let Some(profile) = &mut self.profile
            {
                profile.options = false;
            }
            self.transition(false);
            self.body_scroll.set_offset(gpui::point(px(0.), px(0.)));
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
            self.body_scroll.set_offset(gpui::point(px(0.), px(0.)));
        }
    }

    pub(super) fn open_page(&mut self, page: Option<WizardPage>) {
        if self.set_page(page) {
            self.body_scroll.set_offset(gpui::point(px(0.), px(0.)));
        }
    }

    fn open_section_page(&mut self, page: Option<WizardPage>) {
        if self.set_page(page) {
            self.section_scroll.set_offset(gpui::point(px(0.), px(0.)));
        }
    }

    fn set_page(&mut self, page: Option<WizardPage>) -> bool {
        if self.page != page {
            self.page = page;
            self.transition(page.is_none());
            true
        } else {
            false
        }
    }

    fn transition(&mut self, backwards: bool) {
        self.backwards = backwards;
        self.revision = self.revision.wrapping_add(1);
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

    pub(super) fn browse_optifine(&mut self, cx: &mut Context<'_, Self>) {
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
        let request = uuid::Uuid::new_v4();
        self.shell.wizard.optifine_request = Some(request);
        let prompt = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(tr!(self.language(), "wizard.optifine.chooserPrompt").into()),
        });
        cx.notify();
        cx.spawn(async move |view, cx| {
            let selection = match prompt.await {
                Ok(Ok(paths)) => Ok(paths.and_then(|paths| paths.into_iter().next())),
                _ => Err(()),
            };
            let path = view
                .update(cx, |view, cx| {
                    view.finish_optifine_browse(request, field, selection, cx)
                })
                .ok()
                .flatten();
            let Some(path) = path else {
                return;
            };
            let result = engine.import_optifine(path).await;
            let _ = view.update(cx, |view, cx| {
                view.finish_optifine_import(request, field, result, cx)
            });
        })
        .detach();
    }

    fn finish_optifine_browse(
        &mut self,
        request: uuid::Uuid,
        field: gpui::EntityId,
        selection: Result<Option<std::path::PathBuf>, ()>,
        cx: &mut Context<'_, Self>,
    ) -> Option<std::path::PathBuf> {
        if self.shell.wizard.optifine_request != Some(request) {
            return None;
        }
        let current = !self.busy
            && self.shell.wizard.page == Some(WizardPage::OptiFine)
            && matches!(&self.form, Some(Form::Instance(form))
                if form.version.entity_id() == field
                    && matches!(form.loader, LoaderKind::Vanilla | LoaderKind::Forge));
        if !current {
            self.shell.wizard.optifine_request = None;
            cx.notify();
            return None;
        }
        match selection {
            Ok(Some(path)) => {
                self.shell.wizard.clear_error();
                cx.notify();
                Some(path)
            }
            Ok(None) => {
                self.shell.wizard.optifine_request = None;
                cx.notify();
                None
            }
            Err(()) => {
                self.shell.wizard.optifine_request = None;
                self.shell.wizard.error =
                    Some(WizardText::message("wizard.optifine.chooserFailed"));
                cx.notify();
                None
            }
        }
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
                Err(error) => self.shell.wizard.error = Some(error.to_string().into()),
            },
            Err(error) => {
                self.shell.wizard.error = Some(
                    WizardText::message("wizard.optifine.importFailed")
                        .arg("error", format!("{error:#}")),
                );
            }
        }
        cx.notify();
    }

    fn wizard_optifine_card(
        &self,
        form: &InstanceForm,
        cx: &Context<'_, Self>,
    ) -> gpui::AnyElement {
        let language = self.language();
        if !matches!(form.loader, LoaderKind::Vanilla | LoaderKind::Forge) {
            return self.wizard_unavailable(
                "optifine",
                "OptiFine",
                &text(
                    language,
                    "wizard.optifine.cannotCombine",
                    &[("loader", form.loader.name())],
                ),
            );
        }
        self.wizard_card(
            "optifine",
            "OptiFine",
            (
                &form.optifine.as_ref().map_or_else(
                    || tr!(language, "wizard.optifine.notSelected"),
                    |jar| jar.version.clone(),
                ),
                &if form.optifine.is_some() {
                    tr!(language, "wizard.optifine.verified")
                } else {
                    tr!(language, "wizard.optifine.downloadThenSelect")
                },
            ),
            WizardPage::OptiFine,
            cx,
        )
    }

    pub(super) fn wizard_optifine(&self, form: &InstanceForm, cx: &Context<'_, Self>) -> gpui::Div {
        let language = self.language();
        let pending = self.shell.wizard.optifine_request.is_some();
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(patterns::page_heading(
                tr!(language, "wizard.optifine.heading"),
                tr!(language, "wizard.optifine.body"),
            ))
            .child(components::button(
                "optifine-official-download",
                tr!(language, "wizard.optifine.openDownloads"),
                self.palette,
                ButtonVariant::Outline,
                ControlState::default(),
                |&(), _, cx| cx.open_url("https://optifine.net/downloads"),
            ))
            .child(components::button(
                "optifine-select-jar",
                if pending {
                    tr!(language, "wizard.optifine.choosing")
                } else {
                    tr!(language, "wizard.optifine.selectJar")
                },
                self.palette,
                ButtonVariant::Outline,
                ControlState {
                    disabled: self.busy || pending,
                    busy: pending,
                    ..Default::default()
                },
                cx.listener(|view, (), window, cx| {
                    view.handle(UiAction::WizardOptiFineFile, window, cx)
                }),
            ))
            .children(form.optifine.as_ref().map(|selected| {
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(div().text_sm().child(text(
                        language,
                        "wizard.optifine.selected",
                        &[("version", &selected.version)],
                    )))
                    .child(note(selected.forge.as_ref().map_or_else(
                        || tr!(language, "wizard.optifine.standalone"),
                        |version| text(language, "wizard.optifine.forge", &[("version", version)]),
                    )))
                    .child(self.button(
                        "optifine-remove",
                        tr!(language, "wizard.optifine.useWithout"),
                        UiAction::WizardOptiFineRemove,
                        cx,
                    ))
            }))
            .children(self.optifine_issue(cx).map(|reason| note(reason)))
            .children(self.shell.wizard.error_text(language).map(|reason| {
                note(reason)
                    .id("wizard-optifine-error")
                    .debug_selector(|| "wizard-optifine-error".into())
            }))
            .child(note(tr!(language, "wizard.optifine.cacheNote")))
    }
    pub(super) fn set_wizard_error(&mut self, message: impl Into<WizardText>, cx: &gpui::App) {
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
            } else if self.wizard_loader_text(cx).is_some() {
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
        self.shell.wizard.error = Some(message.into());
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
        if !matches!(self.router.tab(), Tab::Instances | Tab::Onboarding)
            || !matches!(self.form, Some(Form::Instance(_)))
            || !self.shell.wizard.has_error()
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
        label: &str,
        input: &Entity<TextInput>,
        hint: Option<&str>,
    ) -> gpui::Stateful<gpui::Div> {
        let section_field = self.wizard_section_page() == Some(WizardPage::Location)
            && (matches!(
                field,
                WizardField::Directory
                    | WizardField::Java
                    | WizardField::Width
                    | WizardField::Height
            ) || self.router.tab() == Tab::Onboarding
                && matches!(field, WizardField::Memory | WizardField::Jvm));
        let scroll = if section_field {
            &self.shell.wizard.section_scroll
        } else {
            &self.shell.wizard.body_scroll
        };
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
                        .error_text(self.language())
                        .filter(|_| self.shell.wizard.error_field == Some(field))
                        .map(Into::into),
                },
            ),
            scroll.clone(),
            self.shell.wizard.reveal.clone(),
            field,
        )
        .id(gpui::SharedString::from(format!("wizard-field-{field:?}")))
        .debug_selector(move || format!("wizard-field-{field:?}"))
    }

    pub(super) fn begin_game_wizard(&mut self, cx: &mut Context<'_, Self>) {
        let language = self.language();
        self.shell.wizard = WizardState::new(language, cx);
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
        self.shell.wizard.profile = Some(ProfileDraft::new(
            &self.saved_preferences.game.clone(),
            language,
            cx,
        ));
        self.router.navigate(Tab::Instances);
        self.router.enter_game_wizard();
        let version = self
            .versions
            .first()
            .map_or("1.20.6", String::as_str)
            .to_owned();
        self.form = Some(Form::Instance(InstanceForm {
            name: input(
                &tr!(language, "wizard.setup.defaultGameName"),
                &tr!(language, "wizard.setup.gameNamePlaceholder"),
                cx,
            ),
            version: input(
                &version,
                &tr!(language, "wizard.setup.versionPlaceholder"),
                cx,
            ),
            directory: input("", &tr!(language, "wizard.setup.directoryPlaceholder"), cx),
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
        self.shell.wizard.open_section_page(page);
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
            self.shell.wizard.version_notice = Some(
                WizardText::message("wizard.notice.optifineRemoved").arg("loader", kind.name()),
            );
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
                Err(tr!(self.language(), "wizard.loader.serviceUnavailable")),
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
                        self.shell.wizard.version_notice =
                            Some(WizardText::message("wizard.notice.buildGone"));
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
            Err(error) => catalog.load = CollectionLoad::Failed(crate::i18n::UiText::Raw(error)),
        }
        if self.shell.wizard.error_field == Some(WizardField::Loader) {
            self.shell.wizard.clear_error();
        }
    }

    /// Retain the loader problem by identity for shared owners.
    pub(super) fn wizard_loader_issue(&self, cx: &gpui::App) -> Option<WizardText> {
        self.wizard_loader_text(cx)
    }

    fn wizard_loader_text(&self, cx: &gpui::App) -> Option<WizardText> {
        if let Some(issue) = self.optifine_issue(cx) {
            return Some(issue.into());
        }
        if self.shell.wizard.optifine_request.is_some() {
            return Some(WizardText::message("wizard.loader.issue.waitOptifine"));
        }
        let Some(Form::Instance(form)) = &self.form else {
            return None;
        };
        match form.loader {
            LoaderKind::Vanilla => None,
            _ => {
                let Some(catalog) = self.shell.wizard.loader_catalog.as_ref() else {
                    return Some(WizardText::message("wizard.loader.issue.loadCatalog"));
                };
                if catalog.kind != form.loader || catalog.minecraft != form.version.read(cx).value()
                {
                    return Some(WizardText::message("wizard.loader.issue.reload"));
                }
                match &catalog.load {
                    CollectionLoad::Loading => {
                        Some(WizardText::message("wizard.loader.issue.loading"))
                    }
                    CollectionLoad::Failed(_) => {
                        Some(WizardText::message("wizard.loader.issue.failed"))
                    }
                    CollectionLoad::Ready if catalog.builds.is_empty() => {
                        Some(WizardText::message("wizard.loader.issue.noBuild"))
                    }
                    CollectionLoad::Ready
                        if !catalog
                            .builds
                            .iter()
                            .any(|build| Some(&build.version) == form.loader_version.as_ref()) =>
                    {
                        Some(WizardText::message("wizard.loader.issue.chooseBuild"))
                    }
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

    pub(super) fn set_wizard_version(&mut self, version: String, cx: &mut Context<'_, Self>) {
        let version = if version.trim().len() == version.len() {
            version
        } else {
            version.trim().to_owned()
        };
        if let Some(reason) = self.wizard_version_restriction(&version) {
            self.shell.wizard.version_notice = Some(msg!(
                "wizard.version.unsupported_arm",
                version = version,
                reason = reason
            ));
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
                self.shell.wizard.version_notice =
                    Some(WizardText::message("wizard.notice.minecraftChanged"));
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
    ) -> Option<(WizardField, WizardText)> {
        let Some(Form::Instance(form)) = &self.form else {
            return Some((
                WizardField::GameName,
                WizardText::message("wizard.error.reopen"),
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
                WizardText::message("wizard.error.chooseVersion"),
            ));
        }
        if page == WizardPage::Version
            && let Some(reason) = self.wizard_version_restriction(version.trim())
        {
            return Some((
                WizardField::Version,
                msg!(
                    "wizard.version.unsupported_arm",
                    version = version.trim(),
                    reason = reason
                ),
            ));
        }
        if matches!(page, WizardPage::Version | WizardPage::Loader)
            && let Some(issue) = self.wizard_loader_text(cx)
        {
            return Some((WizardField::Loader, issue));
        }
        if let Some(selected) = &form.optifine
            && let Err(error) =
                selected.validate_for(&version, &form.loader, form.loader_version.as_deref())
        {
            return Some((WizardField::Loader, error.to_string().into()));
        }
        if self.shell.wizard.optifine_request.is_some() {
            return Some((
                WizardField::Loader,
                WizardText::message("wizard.error.optifinePending"),
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
                WizardText::message("wizard.error.absoluteFolder"),
            ));
        }
        let Some(profile) = &self.shell.wizard.profile else {
            return Some((
                WizardField::Java,
                WizardText::message("wizard.error.draftUnavailable"),
            ));
        };
        if !profile.managed
            && (!std::path::Path::new(profile.java.read(cx).value().trim()).is_absolute()
                || profile.java.read(cx).value().chars().any(char::is_control))
        {
            return Some((
                WizardField::Java,
                WizardText::message("wizard.error.absoluteJava"),
            ));
        }
        for (field, key, input, minimum, maximum) in [
            (
                WizardField::Width,
                "wizard.error.widthRange",
                &profile.width,
                320,
                16384,
            ),
            (
                WizardField::Height,
                "wizard.error.heightRange",
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
                    WizardText::message(key)
                        .arg("minimum", minimum.to_string())
                        .arg("maximum", maximum.to_string()),
                ));
            }
        }
        if self.router.tab() == Tab::Onboarding {
            if components::parse_memory_gb(&profile.memory.read(cx).value()).is_err() {
                return Some((
                    WizardField::Memory,
                    WizardText::message("wizard.error.memoryRange"),
                ));
            }
            if super::parse_arguments(&profile.jvm.read(cx).value()).is_err() {
                return Some((WizardField::Jvm, msg!("common.arguments_unclosed_quote")));
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
                .then(|| WizardText::message("wizard.error.chooseInstance"))
            } else if state.step == WizardStep::Profile
                || (!state.separated && state.step == WizardStep::Setup && state.page.is_none())
            {
                state.profile.as_ref().map_or_else(
                    || Some(WizardText::message("wizard.error.profileDraftUnavailable")),
                    |profile| profile.values(cx).err(),
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
                self.set_wizard_error(error, cx);
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
                self.shell.wizard.error = Some(WizardText::message("wizard.error.instanceGone"));
                return;
            };
            self.begin(
                msg!("wizard.status.savingProfile"),
                super::ui_logging::Operation::CreateProfile,
            );
            self.mark_creation();
            if let Some(engine) = &self.engine {
                engine.create_profile_draft(instance, values);
            }
        } else {
            let directory =
                (!directory.trim().is_empty()).then(|| std::path::PathBuf::from(directory.trim()));
            let defaults = values.defaults.clone();
            // One decision routes the work and names it, so the busy status can
            // never promise preparation this branch does not start: first-run
            // Create only saves the setup, and Finish prepares game files.
            let saves_setup = self.router.tab() == Tab::Onboarding && self.shell.first_run.enabled;
            self.begin_cancellable(
                if saves_setup {
                    msg!("wizard.status.savingSetup")
                } else {
                    msg!("wizard.status.creatingGame")
                },
                if saves_setup {
                    super::ui_logging::Operation::SaveSetup
                } else {
                    super::ui_logging::Operation::CreateGame
                },
            );
            self.mark_creation();
            if let Some(engine) = &self.engine {
                if saves_setup {
                    engine.save_setup_game(
                        name,
                        version,
                        directory,
                        loader,
                        optifine,
                        values,
                        self.cancel.clone(),
                    );
                } else {
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
        let section = self.shell.wizard.section_snapshot.is_some();
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
                    catalog.load = CollectionLoad::Failed(msg!("wizard.loader.cancelled"));
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
        // A rejected Done leaves its message, field and pending reveal on the
        // shared wizard error owner. Cancelling the draft discards that
        // validation together with the edits it described: the restored
        // checkpoint is valid and the outer page must not inherit the banner.
        self.shell.wizard.clear_error();
        if section {
            self.shell.wizard.open_section_page(None);
        } else {
            self.shell.wizard.open_page(None);
        }
        self.shell.wizard.loader_request = None;
        self.shell.wizard.optifine_request = None;
    }

    pub(super) fn render_game_wizard(
        &self,
        form: &InstanceForm,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement + use<> {
        let language = self.language();
        let body = self.game_wizard_transition_body(form, WizardBodyOwner::Outer, window, cx);
        let (steps, current) = self.wizard_rail_progress(language);
        div()
            .id("game-wizard")
            .debug_selector(|| "game-wizard".into())
            .size_full()
            .min_h_0()
            .child(self.host_wizard(
                super::page_host::WizardSlots {
                    rail: self.wizard_step_rail(&steps, current),
                    body,
                    footer: self.game_wizard_footer(language, cx),
                },
                super::page_host::WizardLayout::Game,
                window,
                cx,
            ))
    }

    /// The wizard footer: a section page commits that page, Review installs the
    /// draft, and every earlier step continues to the next one.
    fn game_wizard_footer(&self, language: Language, cx: &Context<'_, Self>) -> gpui::AnyElement {
        let state = &self.shell.wizard;
        let page = self.wizard_outer_page();
        let review = state.step == WizardStep::Review;
        let show_back = page.is_some()
            || if state.separated {
                state.step != WizardStep::GetStarted
            } else {
                review
            };
        let next_label = if page.is_some() {
            tr!(language, "wizard.footer.done")
        } else if review {
            if state.kind == CreationKind::ExistingInstance {
                tr!(language, "wizard.footer.createProfile")
            } else {
                tr!(language, "wizard.footer.createGame")
            }
        } else {
            tr!(language, "wizard.footer.continue")
        };
        let next_action = if review && page.is_none() {
            UiAction::SaveInstance
        } else {
            UiAction::WizardNext
        };
        div()
            .w_full()
            .flex()
            .justify_between()
            .items_center()
            .gap_3()
            .child(self.button(
                "cancel-instance",
                tr!(language, "common.cancel"),
                UiAction::CancelForm,
                cx,
            ))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .children(show_back.then(|| {
                        self.button(
                            "wizard-back",
                            tr!(language, "wizard.footer.back"),
                            UiAction::WizardBack,
                            cx,
                        )
                    }))
                    .child(self.button("wizard-next", next_label, next_action, cx)),
            )
            .into_any_element()
    }

    /// The rail's step copy and the index of the step currently in progress.
    /// A separated wizard walks Get Started, instance, profile and review; the
    /// joined one folds everything into Setup.
    fn wizard_rail_progress(&self, language: Language) -> (Vec<(String, String)>, usize) {
        let state = &self.shell.wizard;
        let review_detail = tr!(language, "wizard.rail.reviewDetail");
        let steps = if state.separated {
            vec![
                (
                    tr!(language, "wizard.rail.getStarted"),
                    tr!(language, "wizard.rail.getStartedDetail"),
                ),
                (
                    if state.kind == CreationKind::NewInstance {
                        tr!(language, "wizard.rail.instance")
                    } else {
                        tr!(language, "wizard.rail.chooseInstance")
                    },
                    tr!(language, "wizard.rail.instanceDetail"),
                ),
                (
                    tr!(language, "wizard.rail.profile"),
                    tr!(language, "wizard.rail.profileDetail"),
                ),
                (tr!(language, "wizard.rail.review"), review_detail.clone()),
            ]
        } else {
            vec![
                (
                    tr!(language, "wizard.rail.setup"),
                    tr!(language, "wizard.rail.setupDetail"),
                ),
                (tr!(language, "wizard.rail.review"), review_detail),
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
        (steps, current)
    }

    pub(super) fn render_game_wizard_transition_body(
        &self,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> gpui::AnyElement {
        match &self.form {
            Some(Form::Instance(form)) => {
                self.game_wizard_transition_body(form, WizardBodyOwner::Outer, window, cx)
            }
            _ => div().into_any_element(),
        }
    }

    pub(super) fn render_game_wizard_section_body(
        &self,
        page: WizardPage,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> gpui::AnyElement {
        match &self.form {
            Some(Form::Instance(form)) => {
                self.game_wizard_transition_body(form, WizardBodyOwner::Section(page), window, cx)
            }
            _ => div().into_any_element(),
        }
    }

    fn wizard_section_page(&self) -> Option<WizardPage> {
        self.shell
            .surfaces
            .layers()
            .find_map(|layer| match layer.kind {
                super::shell::Surface::WizardSection(page) => Some(page),
                _ => None,
            })
    }

    pub(super) fn wizard_outer_page(&self) -> Option<WizardPage> {
        if self.wizard_section_page().is_some() {
            None
        } else {
            self.shell.wizard.page
        }
    }

    fn game_wizard_transition_body(
        &self,
        form: &InstanceForm,
        owner: WizardBodyOwner,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> gpui::AnyElement {
        let state = &self.shell.wizard;
        let language = self.language();
        let (page, scroll, id) = match owner {
            WizardBodyOwner::Outer => (self.wizard_outer_page(), &state.body_scroll, "wizard-body"),
            WizardBodyOwner::Section(page) => {
                (Some(page), &state.section_scroll, "wizard-section-body")
            }
        };
        let body = self.game_wizard_body(form, page, window, cx);
        let body = div()
            .flex()
            .flex_col()
            .gap_4()
            .child(body)
            .children(state.error_text(language).map(|error| {
                div()
                    .id("wizard-error")
                    .debug_selector(|| "wizard-error".into())
                    .flex()
                    .items_start()
                    .gap_2()
                    .text_sm()
                    .text_color(rgb(self.palette.destructive))
                    .child(Icon::Error.render(16., self.palette.destructive))
                    .child(error)
            }))
            .children(self.busy.then(|| note(self.status.text(self.language()))))
            .into_any_element();
        div()
            .id(id)
            .debug_selector(move || id.into())
            .size_full()
            .min_h_0()
            .scroll_area(scroll, id, self.palette, false)
            .child(div().w_full().max_w(px(608.)).mx_auto().p_8().child(body))
            .into_any_element()
    }

    fn game_wizard_body(
        &self,
        form: &InstanceForm,
        page: Option<WizardPage>,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> gpui::AnyElement {
        let state = &self.shell.wizard;
        let review = state.step == WizardStep::Review;
        match page {
            Some(WizardPage::Version) => self.wizard_version(form, cx).into_any_element(),
            Some(WizardPage::Loader) => self.wizard_loader(form, cx).into_any_element(),
            Some(WizardPage::OptiFine) => self.wizard_optifine(form, cx).into_any_element(),
            Some(WizardPage::Location) => self.wizard_location(form, cx).into_any_element(),
            // The Review is ordinary content of the outer `wizard-body` viewport.
            // A second viewport here would claim the shared `body_scroll` while
            // sizing itself to its own content, so the outer page would keep the
            // space and report no remaining range: the facts below the fold could
            // never be scrolled above the fixed footer.
            None if review => div()
                .id("wizard-review-content")
                .debug_selector(|| "wizard-review-content".into())
                .w_full()
                .min_w_0()
                .flex()
                .flex_col()
                .child(self.wizard_review(form, window, cx))
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

    fn wizard_step_rail(&self, steps: &[(String, String)], current: usize) -> gpui::AnyElement {
        let language = self.language();
        let current_step = (current + 1).to_string();
        let total_steps = steps.len().to_string();
        let rail = div()
            .id("wizard-rail")
            .debug_selector(|| "wizard-rail".into())
            .min_w_0()
            .h_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .w(px(super::design::SIDEBAR_WIDTH))
            .border_r_1()
            .border_color(rgb(self.palette.border))
            .bg(rgb(self.palette.sidebar));
        rail.child(
            div()
                .id("wizard-steps")
                .flex_1()
                .min_h_0()
                .scroll_area(
                    &self.shell.wizard.rail_scroll,
                    "wizard.rail.steps",
                    self.palette,
                    false,
                )
                .px_6()
                .py_6()
                .flex()
                .flex_col()
                .gap_6()
                .child(note(tr!(language, "wizard.rail.section")))
                .child(components::stepper(
                    "game-wizard-steps",
                    &tr!(language, "wizard.rail.stepperLabel"),
                    &steps
                        .iter()
                        .enumerate()
                        .map(|(index, (title, description))| components::StepperStep {
                            title: title.as_str(),
                            description: description.as_str(),
                            complete: index < current,
                        })
                        .collect::<Vec<_>>(),
                    current,
                    self.palette,
                )),
        )
        .child(
            patterns::management_footer(self.palette)
                .px_6()
                .child(note(text(
                    language,
                    "wizard.rail.stepOf",
                    &[
                        ("current", current_step.as_str()),
                        ("total", total_steps.as_str()),
                    ],
                ))),
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
        let language = self.language();
        let group = self.wizard_radio_group(
            "wizard-creation-kind",
            &tr!(language, "wizard.start.creationType"),
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
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(patterns::page_heading(
                tr!(language, "wizard.start.heading"),
                tr!(language, "wizard.start.subtitle"),
            ))
            .child(components::badge(
                tr!(language, "wizard.start.badge"),
                self.palette,
                BadgeVariant::Outline,
            ))
            .child(
                group
                    .container()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(
                        group.item(
                            "instance",
                            components::radio(
                                "wizard-kind-instance",
                                tr!(language, "wizard.start.newInstance"),
                                state.kind == CreationKind::NewInstance,
                                self.palette,
                                ControlState {
                                    disabled: self.busy,
                                    ..ControlState::default()
                                },
                                cx.listener(|view, &(): &(), window, cx| {
                                    view.handle(UiAction::WizardKind(true), window, cx)
                                }),
                            )
                            .w_full()
                            .rounded(px(14.))
                            .border_color(rgb(self.palette.border))
                            .p_4()
                            .child(Icon::Blocks.render(20., self.palette.muted)),
                        ),
                    )
                    .child(note(tr!(language, "wizard.start.newInstanceNote")))
                    .child(
                        group.item(
                            "profile",
                            components::radio(
                                "wizard-kind-profile",
                                tr!(language, "wizard.start.newProfile"),
                                state.kind == CreationKind::ExistingInstance,
                                self.palette,
                                ControlState {
                                    disabled: self.busy || self.snapshot.instances.is_empty(),
                                    ..ControlState::default()
                                },
                                cx.listener(|view, &(): &(), window, cx| {
                                    view.handle(UiAction::WizardKind(false), window, cx)
                                }),
                            )
                            .w_full()
                            .rounded(px(14.))
                            .border_color(rgb(self.palette.border))
                            .p_4()
                            .child(Icon::Layers.render(20., self.palette.muted)),
                        ),
                    )
                    .child(note(if self.snapshot.instances.is_empty() {
                        tr!(language, "wizard.start.noInstancesNote")
                    } else {
                        tr!(language, "wizard.start.reuseNote")
                    })),
            )
    }

    fn wizard_instance_picker(&self, cx: &Context<'_, Self>) -> gpui::Div {
        let language = self.language();
        let group = self.wizard_radio_group(
            "wizard-instance-group",
            &tr!(language, "wizard.instance.group"),
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
            .child(patterns::page_heading(
                tr!(language, "wizard.instance.heading"),
                tr!(language, "wizard.instance.subtitle"),
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
                    let count_text = count.to_string();
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
                        .child(note(text(
                            language,
                            "wizard.instance.profiles",
                            &[("count", count_text.as_str())],
                        ))),
                    )
                }),
            ))
            .children(self.snapshot.instances.is_empty().then(|| {
                patterns::empty_state(
                    tr!(language, "wizard.instance.emptyTitle"),
                    tr!(language, "wizard.instance.emptyDetail"),
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
        // First-run Configuration contains these same draft editors. While its
        // section owns them, keep the disclosure and the rest of the profile
        // mounted without mounting a second copy of the inputs.
        let section_owns_advanced = self.router.tab() == Tab::Onboarding
            && self.shell.wizard.kind == CreationKind::NewInstance
            && self.wizard_section_page() == Some(WizardPage::Location);
        let language = self.language();
        div()
            .flex()
            .flex_col()
            .gap_4()
            .children(
                show_name.then(|| patterns::page_title(tr!(language, "wizard.profile.title"))),
            )
            .children(show_name.then(|| {
                self.wizard_field(
                    WizardField::ProfileName,
                    &tr!(language, "wizard.profile.name"),
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
                        &tr!(language, "wizard.profile.icon"),
                        crate::i18n::profile_icon_name(language, profile.appearance.icon),
                        false,
                        cx,
                    ))
                    .child(self.wizard_appearance_field(
                        "wizard-profile-color",
                        &tr!(language, "wizard.profile.color"),
                        crate::i18n::profile_color_name(language, profile.appearance.color),
                        true,
                        cx,
                    )),
            )
            .child(self.wizard_server_preferences(profile, cx))
            .children(
                (self.shell.wizard.kind == CreationKind::ExistingInstance)
                    .then(|| self.wizard_profile_targets(profile, cx)),
            )
            .children(show_name.then(|| {
                self.wizard_profile_options_trigger(
                    "wizard-profile-options",
                    &tr!(language, "wizard.profile.advanced"),
                    cx,
                )
            }))
            .children(show_name.then(|| {
                super::motion::disclosure(
                    "wizard-advanced",
                    &self.shell.wizard.advanced.borrow(),
                    self.motion_preferences(),
                    if section_owns_advanced {
                        div().into_any_element()
                    } else {
                        self.wizard_profile_advanced(profile, cx).into_any_element()
                    },
                    window,
                    cx,
                )
            }))
            .children(
                (!show_name && !section_owns_advanced)
                    .then(|| self.wizard_profile_advanced(profile, cx)),
            )
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
        id: &'static str,
        label: &str,
        cx: &Context<'_, Self>,
    ) -> components::Control {
        super::motion::disclosure_trigger(
            id,
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
        let language = self.language();
        let has_address = !profile.server.read(cx).value().trim().is_empty();
        let mut fields = div()
            .flex()
            .flex_col()
            .gap_3()
            .child(self.wizard_field(
                WizardField::Server,
                &tr!(language, "wizard.profile.server"),
                &profile.server,
                Some(&tr!(language, "wizard.profile.serverHint")),
            ))
            .child(components::checkbox(
                "wizard-profile-connect",
                tr!(language, "wizard.profile.connect"),
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
                    tr!(language, "wizard.profile.connectionUnsupported")
                }
                _ => tr!(language, "wizard.profile.connectionUnknown"),
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
            let error = match self.wizard_automatic_connection(cx) {
                Some(block_launcher::catalog::AutomaticConnection::Unsupported) => {
                    WizardText::message("wizard.error.connectionUnsupported")
                }
                _ => WizardText::message("wizard.error.connectionUnknown"),
            };
            self.set_wizard_error(error, cx);
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
        label: &str,
        value: String,
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
        let language = self.language();
        let group = self.wizard_radio_group(
            "wizard-target-group",
            &tr!(language, "wizard.profile.launchTarget"),
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
            .child(note(tr!(language, "wizard.profile.launchTarget")))
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
                                block_launcher::catalog::InstallStatus::Ready => {
                                    tr!(language, "wizard.profile.installReady")
                                }
                                block_launcher::catalog::InstallStatus::Pending => {
                                    tr!(language, "wizard.profile.installPending")
                                }
                                block_launcher::catalog::InstallStatus::Installing => {
                                    tr!(language, "wizard.profile.installInstalling")
                                }
                                block_launcher::catalog::InstallStatus::Failed { .. } => {
                                    tr!(language, "wizard.profile.installFailed")
                                }
                            })),
                        )
                    })),
            )
    }

    fn wizard_profile_advanced(&self, profile: &ProfileDraft, cx: &Context<'_, Self>) -> gpui::Div {
        let language = self.language();
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(self.wizard_field(
                WizardField::Memory,
                &tr!(language, "wizard.profile.memory"),
                &profile.memory,
                Some(&tr!(language, "wizard.profile.memoryHint")),
            ))
            .child(self.wizard_field(
                WizardField::Jvm,
                &tr!(language, "wizard.profile.jvm"),
                &profile.jvm,
                None,
            ))
            .children(
                (self.shell.wizard.kind == CreationKind::ExistingInstance)
                    .then(|| self.wizard_runtime_window(profile, cx)),
            )
    }

    fn wizard_runtime_window(&self, profile: &ProfileDraft, cx: &Context<'_, Self>) -> gpui::Div {
        let language = self.language();
        let width = profile.width.read(cx).value();
        let height = profile.height.read(cx).value();
        let size = text(
            language,
            if profile.window_custom {
                "wizard.profile.windowSizeCustom"
            } else {
                "wizard.profile.windowSizeValue"
            },
            &[("width", &width), ("height", &height)],
        );
        let runtime = if profile.managed {
            tr!(language, "wizard.profile.automaticJava")
        } else {
            tr!(language, "wizard.profile.customExecutable")
        };
        div()
            .flex()
            .flex_col()
            .gap_4()
            // The shared selector owns its label as `&'static str`, so this
            // trigger keeps the shared English label until the selector accepts
            // localized copy.
            .child(self.product_select_field(
                "wizard-java-runtime",
                tr!(language, "select.java_runtime"),
                &runtime,
                super::select::ProductChoice::WizardRuntime,
                cx,
            ))
            .children((!profile.managed).then(|| {
                self.wizard_field(
                    WizardField::Java,
                    &tr!(language, "wizard.profile.javaExecutable"),
                    &profile.java,
                    None,
                )
            }))
            .child(self.product_select_field(
                "wizard-window-size",
                tr!(language, "select.window_size"),
                &size,
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
                            &tr!(language, "wizard.profile.windowWidth"),
                            &profile.width,
                            None,
                        )
                        .flex_1(),
                    )
                    .child(
                        self.wizard_field(
                            WizardField::Height,
                            &tr!(language, "wizard.profile.windowHeight"),
                            &profile.height,
                            None,
                        )
                        .flex_1(),
                    )
            }))
            .child(components::checkbox(
                "wizard-fullscreen",
                tr!(language, "wizard.profile.fullscreen"),
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
        let language = self.language();
        let version = form.version.read(cx).value();
        let version_kind = self
            .version_catalog
            .iter()
            .find(|entry| entry.id == version.trim())
            .map_or_else(
                || tr!(language, "wizard.version.exactId"),
                |entry| Self::wizard_version_kind(entry.kind, language),
            );
        let configuration = self.wizard_configuration_summary(cx);
        let loader_original = tr!(language, "wizard.setup.originalGame");
        let loader_version = form.loader_version.as_deref().unwrap_or(&loader_original);
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(patterns::page_heading(
                tr!(language, "wizard.setup.heading"),
                tr!(language, "wizard.setup.subtitle"),
            ))
            .child(self.wizard_field(
                WizardField::GameName,
                &tr!(language, "wizard.setup.gameName"),
                &form.name,
                None,
            ))
            .child(
                div()
                    .id("wizard-components")
                    .debug_selector(|| "wizard-components".into())
                    .grid()
                    .grid_cols(2)
                    .gap_3()
                    .child(self.wizard_card(
                        "version",
                        &tr!(language, "wizard.setup.versionCard"),
                        (&version, &version_kind),
                        WizardPage::Version,
                        cx,
                    ))
                    .child(self.wizard_card(
                        "loader",
                        &tr!(language, "wizard.setup.loaderCard"),
                        (form.loader.name(), loader_version),
                        WizardPage::Loader,
                        cx,
                    ))
                    .child(self.wizard_optifine_card(form, cx))
                    .child(self.wizard_card(
                        "location",
                        &tr!(language, "wizard.setup.configurationCard"),
                        (&configuration.0, &configuration.1),
                        WizardPage::Location,
                        cx,
                    )),
            )
            .child(note(tr!(language, "wizard.setup.openCard")))
            .children(
                self.shell
                    .wizard
                    .version_notice
                    .as_ref()
                    .map(|notice| self.wizard_version_notice(notice, language)),
            )
            .children((!self.shell.wizard.separated).then(|| {
                self.wizard_profile_options_trigger(
                    "wizard-profile-options",
                    &tr!(language, "wizard.setup.appearanceLaunch"),
                    cx,
                )
            }))
            .children((!self.shell.wizard.separated).then(|| {
                let content = self.wizard_profile(window, cx, false).into_any_element();
                super::motion::disclosure(
                    "wizard-appearance-launch",
                    &self.shell.wizard.advanced.borrow(),
                    self.motion_preferences(),
                    content,
                    window,
                    cx,
                )
            }))
    }

    pub(super) fn wizard_configuration_summary(&self, cx: &gpui::App) -> (String, String) {
        let language = self.language();
        self.shell.wizard.profile.as_ref().map_or_else(
            || {
                (
                    tr!(language, "wizard.profile.automaticJava"),
                    tr!(language, "wizard.config.defaultWindow"),
                )
            },
            |profile| {
                (
                    if profile.managed {
                        tr!(language, "wizard.profile.automaticJava")
                    } else {
                        tr!(language, "wizard.config.customJava")
                    },
                    if profile.fullscreen {
                        tr!(language, "wizard.config.fullscreenValue")
                    } else {
                        text(
                            language,
                            "wizard.profile.windowSizeValue",
                            &[
                                ("width", &profile.width.read(cx).value()),
                                ("height", &profile.height.read(cx).value()),
                            ],
                        )
                    },
                )
            },
        )
    }

    fn wizard_card(
        &self,
        id: &str,
        title: &str,
        summary: (&str, &str),
        page: WizardPage,
        cx: &Context<'_, Self>,
    ) -> gpui::AnyElement {
        let (value, detail) = summary;
        let icon = match page {
            WizardPage::Version => Icon::Games,
            WizardPage::Loader => Icon::Download,
            WizardPage::OptiFine => Icon::Experiment,
            WizardPage::Location => Icon::Settings,
        };
        let value_id = format!("wizard-component-{id}-value");
        let detail_id = format!("wizard-component-{id}-detail");
        let card = components::button(
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
        .w_auto()
        .h_auto()
        .min_w_0()
        .flex()
        .flex_col()
        .items_start()
        .justify_between()
        .min_h(px(96.))
        .gap_3()
        .p_4();
        let card = card
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
                    .child({
                        let debug_id = value_id.clone();
                        div()
                            .id(value_id)
                            .debug_selector(move || debug_id)
                            .w_full()
                            .min_w_0()
                            .text_sm()
                            .whitespace_normal()
                            .child(value.to_owned())
                    })
                    .child({
                        let debug_id = detail_id.clone();
                        div()
                            .id(detail_id)
                            .debug_selector(move || debug_id)
                            .w_full()
                            .min_w_0()
                            .text_xs()
                            .text_color(rgb(self.palette.muted))
                            .whitespace_normal()
                            .child(detail.to_owned())
                    }),
            );
        super::tooltip::trigger(
            card,
            text(
                self.language(),
                "wizard.card.tooltip",
                &[("title", title), ("value", value), ("detail", detail)],
            ),
            cx,
        )
        .into_any_element()
    }

    fn wizard_unavailable(&self, id: &str, title: &str, reason: &str) -> gpui::AnyElement {
        let card = components::button(
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
        .min_h(px(96.))
        .flex()
        .flex_col()
        .items_start()
        .justify_between()
        .gap_3()
        .p_4()
        .child(Icon::Info.render(20., self.palette.muted))
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
                        .whitespace_normal()
                        .child(tr!(self.language(), "wizard.setup.unavailable")),
                )
                .child(
                    div()
                        .w_full()
                        .min_w_0()
                        .text_xs()
                        .text_color(rgb(self.palette.muted))
                        .whitespace_normal()
                        .child(reason.to_owned()),
                ),
        );
        card.into_any_element()
    }

    fn matching_wizard_versions(
        &self,
        form: &InstanceForm,
        cx: &gpui::App,
    ) -> Vec<(String, Option<block_launcher::manifest::model::VersionType>)> {
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
                    || entry.kind == block_launcher::manifest::model::VersionType::Release
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
        let count_text = count.to_string();
        let language = self.language();
        let result_key = if count == 1 {
            "wizard.version.resultsOne"
        } else {
            "wizard.version.resultsMany"
        };

        div()
            .id("wizard-version-picker")
            .flex()
            .flex_col()
            .gap_4()
            .on_key_down(cx.listener(Self::move_wizard_version))
            .child(patterns::page_heading(
                tr!(language, "wizard.version.heading"),
                tr!(language, "wizard.version.subtitle"),
            ))
            .child(self.wizard_version_catalog_status(cx))
            .children(self.shell.wizard.version_search.as_ref().map(|search| {
                components::field_with(
                    tr!(language, "wizard.version.search"),
                    search,
                    self.palette,
                    FieldOptions {
                        hint: Some(tr!(language, "wizard.version.searchHint").into()),
                        ..FieldOptions::default()
                    },
                )
            }))
            .child(components::checkbox(
                "wizard-snapshots",
                tr!(language, "wizard.version.snapshots"),
                self.shell.wizard.include_snapshots,
                self.palette,
                ControlState {
                    disabled: self.busy || self.version_catalog.is_empty(),
                    ..ControlState::default()
                },
                cx.listener(|view, &(): &(), window, cx| {
                    view.handle(UiAction::WizardSnapshots, window, cx)
                }),
            ))
            .child(
                div()
                    .id("wizard-version-result-count")
                    .text_xs()
                    .text_color(rgb(self.palette.muted))
                    .child(text(language, result_key, &[("count", &count_text)])),
            )
            .child(self.wizard_version_list(form, versions, cx))
            .child(components::field_with(
                tr!(language, "wizard.version.selectedLabel"),
                &form.version,
                self.palette,
                FieldOptions {
                    required: true,
                    hint: Some(tr!(language, "wizard.version.selectedHint").into()),
                    ..FieldOptions::default()
                },
            ))
            .child(self.wizard_version_metadata(form, cx))
            .children(
                self.shell
                    .wizard
                    .version_notice
                    .as_ref()
                    .map(|notice| self.wizard_version_notice(notice, language)),
            )
    }

    fn wizard_version_catalog_status(&self, cx: &Context<'_, Self>) -> gpui::Stateful<gpui::Div> {
        use super::browser::CollectionLoad;
        let language = self.language();
        let previous = !self.version_catalog.is_empty();
        let loaded = self.version_catalog.len().to_string();
        let (title, detail, retry) = match &self.version_load {
            CollectionLoad::Loading => (
                tr!(language, "wizard.catalog.loadingTitle"),
                if previous {
                    tr!(language, "wizard.catalog.loadingRefresh")
                } else {
                    tr!(language, "wizard.catalog.loadingFirst")
                },
                None,
            ),
            CollectionLoad::Failed(error) => (
                tr!(language, "wizard.catalog.failedTitle"),
                text(
                    language,
                    if previous {
                        "wizard.catalog.failedRefresh"
                    } else {
                        "wizard.catalog.failedFirst"
                    },
                    &[("error", &error.text(language))],
                ),
                Some(tr!(language, "wizard.catalog.retry")),
            ),
            CollectionLoad::Ready if !previous => (
                tr!(language, "wizard.catalog.emptyTitle"),
                tr!(language, "wizard.catalog.emptyDetail"),
                Some(tr!(language, "wizard.catalog.reload")),
            ),
            CollectionLoad::Ready => (
                tr!(language, "wizard.catalog.readyTitle"),
                text(
                    language,
                    "wizard.catalog.readyDetail",
                    &[("count", &loaded)],
                ),
                Some(tr!(language, "wizard.catalog.refresh")),
            ),
        };

        let loading = matches!(self.version_load, CollectionLoad::Loading);
        if matches!(self.version_load, CollectionLoad::Ready) && previous {
            return div()
                .id("wizard-version-catalog-status")
                .flex()
                .items_center()
                .justify_between()
                .gap_2()
                .child(note(text(
                    language,
                    "wizard.catalog.summary",
                    &[("count", &loaded)],
                )))
                .child(self.button(
                    "wizard-retry-versions",
                    tr!(language, "wizard.catalog.refresh"),
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
        let language = self.language();
        let selected = form.version.read(cx).value();
        let entry = self
            .version_catalog
            .iter()
            .find(|entry| entry.id == selected.trim());
        let source = entry.map_or_else(
            || {
                if selected.trim().is_empty() {
                    tr!(language, "wizard.version.source.empty")
                } else if self.version_catalog.is_empty() && selected.trim() == "1.20.6" {
                    tr!(language, "wizard.version.source.installerDefault")
                } else if self.version_catalog.is_empty() {
                    tr!(language, "wizard.version.source.unverified")
                } else {
                    tr!(language, "wizard.version.source.notFound")
                }
            },
            |entry| {
                text(
                    language,
                    "wizard.version.source.catalog",
                    &[("kind", &Self::wizard_version_kind(entry.kind, language))],
                )
            },
        );

        div()
            .id("wizard-selected-version-source")
            .flex()
            .flex_col()
            .gap_2()
            .children(entry.map(|entry| {
                components::badge(
                    Self::wizard_version_kind(entry.kind, language),
                    self.palette,
                    if entry.kind == block_launcher::manifest::model::VersionType::Release {
                        BadgeVariant::Success
                    } else {
                        BadgeVariant::Warning
                    },
                )
            }))
            .child(note(source))
    }

    fn wizard_version_kind(
        kind: block_launcher::manifest::model::VersionType,
        language: Language,
    ) -> String {
        use block_launcher::manifest::model::VersionType;
        let key = match kind {
            VersionType::Release => "wizard.version.kind.release",
            VersionType::Snapshot => "wizard.version.kind.snapshot",
            VersionType::OldBeta => "wizard.version.kind.beta",
            VersionType::OldAlpha => "wizard.version.kind.alpha",
        };
        tr!(language, key)
    }

    fn wizard_version_notice(
        &self,
        notice: &WizardText,
        language: Language,
    ) -> gpui::Stateful<gpui::Div> {
        div()
            .id("wizard-version-change-notice")
            .rounded_lg()
            .p_3()
            .bg(rgb(self.palette.secondary))
            .child(note(notice.text(language)))
    }

    fn wizard_version_list(
        &self,
        form: &InstanceForm,
        versions: Vec<(String, Option<block_launcher::manifest::model::VersionType>)>,
        cx: &Context<'_, Self>,
    ) -> super::scroll_area::ScrollArea<gpui_base::RadioGroup> {
        let selected = form.version.read(cx).value();
        let language = self.language();
        let empty = versions.is_empty();
        let group = self
            .wizard_radio_group(
                "wizard-version-list",
                &tr!(language, "wizard.version.listLabel"),
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
                                        .child(tr!(
                                            language,
                                            "wizard.version.unsupported_reason",
                                            reason = reason
                                        )),
                                )
                            } else {
                                div().flex_1()
                            })
                            .children(kind.map(|kind| {
                                components::badge(
                                    Self::wizard_version_kind(kind, language),
                                    self.palette,
                                    if kind == block_launcher::manifest::model::VersionType::Release
                                    {
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
                    tr!(language, "wizard.version.emptyTitle"),
                    tr!(language, "wizard.version.emptyDetail"),
                    self.palette,
                )
            }))
            .scroll_area(
                &self.shell.wizard.version_scroll,
                "wizard.version.list",
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
        let language = self.language();
        let group = self.wizard_radio_group(
            "wizard-loader-group",
            &tr!(language, "wizard.loader.group"),
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
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(patterns::page_heading(
                tr!(language, "wizard.loader.heading"),
                tr!(language, "wizard.loader.subtitle"),
            ))
            .child(
                div()
                    .id("wizard-loader-picker")
                    .debug_selector(|| "wizard-loader-picker".into())
                    .h(px(336.))
                    .min_w_0()
                    .flex()
                    .overflow_hidden()
                    .rounded_xl()
                    .border_1()
                    .border_color(rgb(self.palette.border))
                    .child(
                        group
                            .container()
                            .w(gpui::relative(0.32))
                            .min_w(px(76.))
                            .max_w(px(144.))
                            .flex_none()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .p_2()
                            .border_r_1()
                            .border_color(rgb(self.palette.border))
                            .bg(rgb(self.palette.secondary))
                            .child(note(tr!(language, "wizard.loader.column")).px_1())
                            .children(names.into_iter().map(|(name, choice)| {
                                group.item(name, self.wizard_loader_choice(name, choice, form, cx))
                            })),
                    )
                    .child(self.wizard_loader_builds(form, cx)),
            )
            .child(note(tr!(language, "wizard.loader.note")))
    }

    fn wizard_loader_builds(&self, form: &InstanceForm, cx: &Context<'_, Self>) -> gpui::Div {
        let language = self.language();
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
            return pane.child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_3()
                    .child(Icon::Games.render(28., self.palette.muted))
                    .child(
                        div()
                            .text_sm()
                            .whitespace_normal()
                            .child(tr!(language, "wizard.loader.noneTitle")),
                    )
                    .child(note(tr!(language, "wizard.loader.noneDetail")).whitespace_normal()),
            );
        }
        let catalog = self
            .shell
            .wizard
            .loader_catalog
            .as_ref()
            .filter(|catalog| catalog.kind == form.loader && catalog.minecraft == minecraft);
        let Some(catalog) = catalog else {
            return pane.child(self.wizard_loader_problem(
                language,
                tr!(language, "wizard.loader.loadCatalog"),
                "wizard.loader.retry",
                cx,
            ));
        };
        let builds = catalog.builds.as_slice();
        let load = catalog.load.clone();
        let ready = matches!(load, CollectionLoad::Ready);
        pane = pane.child(self.wizard_loader_builds_head(form, language, builds.len(), &minecraft));
        match &load {
            CollectionLoad::Loading => {
                pane = pane.child(
                    div()
                        .id("wizard-loader-loading")
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(Icon::Loading.render(14., self.palette.muted))
                        .child(note(tr!(language, "wizard.loader.loading"))),
                );
            }
            CollectionLoad::Failed(error) => {
                pane = pane.child(self.wizard_loader_problem(
                    language,
                    error.text(language),
                    "wizard.loader.retry",
                    cx,
                ));
            }
            CollectionLoad::Ready if builds.is_empty() => {
                return pane
                    .child(patterns::empty_state(
                        tr!(language, "wizard.loader.noBuildsTitle"),
                        tr!(language, "wizard.loader.noBuildsDetail"),
                        self.palette,
                    ))
                    .child(self.button(
                        "wizard-loader-retry",
                        tr!(language, "wizard.loader.refresh"),
                        UiAction::ReloadLoaderVersions,
                        cx,
                    ));
            }
            CollectionLoad::Ready => {}
        }
        pane = pane.children(self.shell.wizard.loader_search.as_ref().map(|input| {
            components::field_with(
                tr!(language, "wizard.loader.searchLabel"),
                input,
                self.palette,
                FieldOptions::default(),
            )
        }));
        pane.child(self.wizard_loader_build_rows(form, builds, language, !ready, cx))
    }

    /// The loader and build counts above the build list.
    fn wizard_loader_builds_head(
        &self,
        form: &InstanceForm,
        language: Language,
        count: usize,
        minecraft: &str,
    ) -> gpui::Div {
        let count = count.to_string();
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .child(div().text_sm().child(text(
                language,
                "wizard.loader.versions",
                &[("loader", form.loader.name())],
            )))
            .child(components::badge(
                text(language, "wizard.loader.buildsCount", &[("count", &count)]),
                self.palette,
                BadgeVariant::Secondary,
            ))
            .child(note(text(
                language,
                "wizard.loader.minecraft",
                &[("version", minecraft)],
            )))
    }

    /// A loader problem with the catalog action that resolves it.
    fn wizard_loader_problem(
        &self,
        language: Language,
        message: String,
        retry: &'static str,
        cx: &Context<'_, Self>,
    ) -> gpui::Stateful<gpui::Div> {
        div()
            .id("wizard-loader-error")
            .flex()
            .flex_col()
            .gap_2()
            .child(note(message))
            .child(self.button(
                "wizard-loader-retry",
                tr!(language, retry),
                UiAction::ReloadLoaderVersions,
                cx,
            ))
    }

    /// The filtered build rows, their empty search state and their viewport.
    fn wizard_loader_build_rows(
        &self,
        form: &InstanceForm,
        builds: &[LoaderVersion],
        language: Language,
        unloaded: bool,
        cx: &Context<'_, Self>,
    ) -> gpui::Div {
        let disabled = self.busy || unloaded;
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
        let group = self
            .wizard_radio_group(
                "wizard-loader-build-group",
                &tr!(language, "wizard.loader.buildGroup"),
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
        let rows = filtered
            .iter()
            .map(|build| {
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
                            view.handle(UiAction::WizardLoaderBuild(version.clone()), window, cx)
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
                        if build.stable {
                            tr!(language, "wizard.loader.stable")
                        } else {
                            tr!(language, "wizard.loader.preview")
                        },
                        self.palette,
                        if build.stable {
                            BadgeVariant::Secondary
                        } else {
                            BadgeVariant::Warning
                        },
                    )),
                )
            })
            .collect::<Vec<_>>();
        let empty_search = filtered.is_empty() && !builds.is_empty();
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                group
                    .container()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .children(rows)
                    .children(empty_search.then(|| {
                        patterns::empty_state(
                            tr!(language, "wizard.loader.searchEmptyTitle"),
                            tr!(language, "wizard.loader.searchEmptyDetail"),
                            self.palette,
                        )
                    }))
                    .scroll_area(
                        &self.shell.wizard.loader_scroll,
                        "wizard.loader.builds",
                        self.palette,
                        false,
                    ),
            )
            .child(note(form.loader_version.as_ref().map_or_else(
                || tr!(language, "wizard.loader.chooseBuild"),
                |version| {
                    text(
                        language,
                        "wizard.loader.selectedBuild",
                        &[("version", version)],
                    )
                },
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
        let language = self.language();
        let mut fields = div()
            .flex()
            .flex_col()
            .gap_4()
            .child(patterns::page_heading(
                tr!(language, "wizard.config.heading"),
                tr!(language, "wizard.config.subtitle"),
            ))
            .child(self.wizard_field(
                WizardField::Directory,
                &tr!(language, "wizard.config.directory"),
                &form.directory,
                Some(&tr!(language, "wizard.config.directoryHint")),
            ))
            .child(self.button(
                "browse-install",
                tr!(language, "wizard.config.browse"),
                UiAction::BrowseInstall,
                cx,
            ))
            .children(self.shell.wizard.error_text(language).map(|error| {
                note(error)
                    .id("wizard-location-error")
                    .debug_selector(|| "wizard-location-error".into())
            }));
        if let Some(profile) = &self.shell.wizard.profile {
            fields = fields.child(self.wizard_runtime_window(profile, cx));
            if self.router.tab() == Tab::Onboarding {
                fields = fields.child(self.wizard_profile_advanced(profile, cx));
            }
        }
        fields.child(note(tr!(language, "wizard.config.draftNote")))
    }

    /// The draft values the review states, resolved from the form for a new game
    /// or from the Instance whose new Profile it describes.
    fn review_values(&self, form: &InstanceForm, cx: &Context<'_, Self>) -> ReviewValues {
        let state = &self.shell.wizard;
        let language = self.language();
        let existing = state.kind == CreationKind::ExistingInstance;
        let chosen = self
            .snapshot
            .instances
            .iter()
            .find(|instance| Some(&instance.id) == state.existing_instance.as_ref());
        let game_name = if existing {
            chosen.map_or_else(
                || tr!(language, "wizard.review.unavailableInstance"),
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
        let launch_target = if existing {
            chosen
                .and_then(|instance| {
                    instance.targets.iter().find(|target| {
                        profile.is_some_and(|profile| target.id == profile.target_id)
                    })
                })
                .map_or_else(
                    || tr!(language, "wizard.review.chooseLaunchTarget"),
                    |target| target.display_name(),
                )
        } else {
            form.loader_label()
        };
        let defaults = &self.saved_preferences.game;
        let memory_input = profile.map_or_else(
            || components::memory_gb_text(defaults.memory_mib),
            |profile| profile.memory.read(cx).value(),
        );
        let memory = components::parse_memory_gb(&memory_input)
            .map_or_else(|_| memory_input.clone(), components::memory_label);
        let java = profile.map_or_else(
            || tr!(language, "wizard.config.automaticJavaManaged"),
            |profile| {
                if profile.managed {
                    tr!(language, "wizard.config.automaticJavaManaged")
                } else {
                    profile.java.read(cx).value()
                }
            },
        );
        let server = profile.map_or_else(String::new, |profile| profile.server.read(cx).value());
        let connect = profile.is_some_and(|profile| profile.connect_on_launch);
        let jvm = profile.map_or_else(String::new, |profile| profile.jvm.read(cx).value());
        let resolution = profile.map_or_else(
            || {
                text(
                    language,
                    "wizard.profile.windowSizeValue",
                    &[
                        ("width", &defaults.width.to_string()),
                        ("height", &defaults.height.to_string()),
                    ],
                )
            },
            |profile| {
                text(
                    language,
                    if profile.fullscreen {
                        "wizard.review.windowFullscreen"
                    } else {
                        "wizard.profile.windowSizeValue"
                    },
                    &[
                        ("width", &profile.width.read(cx).value()),
                        ("height", &profile.height.read(cx).value()),
                    ],
                )
            },
        );
        // Show an effective window choice when it differs from the built-in
        // default or deliberately overrides the saved preference.
        let ordinary_window = super::settings::GameDefaults::default();
        let window_visible = profile.is_some_and(|profile| {
            let width = profile.width.read(cx).value();
            let height = profile.height.read(cx).value();
            profile.fullscreen
                || profile.window_custom
                || profile.fullscreen != defaults.fullscreen
                || width.trim() != defaults.width.to_string()
                || height.trim() != defaults.height.to_string()
                || width.trim() != ordinary_window.width.to_string()
                || height.trim() != ordinary_window.height.to_string()
        });
        ReviewValues {
            game_name,
            version,
            directory,
            profile_name,
            appearance: self.wizard_appearance(),
            launch_target,
            memory,
            java,
            server,
            connect,
            jvm,
            resolution,
            window_visible,
        }
    }

    /// Identity first: what is being created and, for a separated Profile, whose
    /// Profile it is.
    fn wizard_review_identity(
        &self,
        values: &ReviewValues,
        existing: bool,
        separated: bool,
    ) -> gpui::Stateful<gpui::Div> {
        let eyebrow = if existing {
            tr!(self.language(), "wizard.review.eyebrowExisting")
        } else if separated {
            tr!(self.language(), "wizard.review.eyebrowNew")
        } else {
            tr!(self.language(), "wizard.review.eyebrowGame")
        };
        components::card(self.palette)
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap_3()
                    .child(Icon::Blocks.render(24., self.palette.muted))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(note(eyebrow))
                            .child(
                                div()
                                    .text_sm()
                                    .whitespace_normal()
                                    .child(values.game_name.clone()),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_2()
                                    .child(components::badge(
                                        values.version.clone(),
                                        self.palette,
                                        BadgeVariant::Secondary,
                                    ))
                                    .child(
                                        components::badge(
                                            values.launch_target.clone(),
                                            self.palette,
                                            BadgeVariant::Outline,
                                        )
                                        .flex_col()
                                        .items_stretch()
                                        .max_w_full()
                                        .min_w_0()
                                        .h_auto()
                                        .min_h(px(20.))
                                        .whitespace_normal()
                                        .line_height(px(18.))
                                        .debug_selector(|| "wizard-review-launch-target".into()),
                                    ),
                            ),
                    ),
            )
            .children(separated.then(|| {
                div()
                    .border_t_1()
                    .border_color(rgb(self.palette.border))
                    .pt_3()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(components::profile_avatar(
                        values.appearance,
                        self.palette,
                        40.,
                    ))
                    .child(
                        div()
                            .min_w_0()
                            .child(note(tr!(self.language(), "wizard.review.newProfile")))
                            .child(values.profile_name.clone()),
                    )
            }))
            .id("wizard-review-identity")
            .debug_selector(|| "wizard-review-identity".into())
    }

    fn wizard_review(
        &self,
        form: &InstanceForm,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> gpui::Stateful<gpui::Div> {
        self.reconcile_wizard_disclosure(cx);
        let state = &self.shell.wizard;
        let language = self.language();
        let existing = state.kind == CreationKind::ExistingInstance;
        let values = self.review_values(form, cx);
        let profile = state.profile.as_ref();
        let summary = review_summary_facts(
            &values,
            self.automatic_connection(&values.version),
            language,
        );
        let details = review_folded_facts(&values, language);
        let title = if existing {
            tr!(language, "wizard.review.titleExisting")
        } else {
            tr!(language, "wizard.review.titleNew")
        };
        let consequence = if existing {
            tr!(language, "wizard.review.consequenceExisting")
        } else if self.router.tab() == Tab::Onboarding && self.shell.first_run.enabled {
            tr!(language, "wizard.review.consequenceFirstRun")
        } else {
            tr!(language, "wizard.review.consequence")
        };
        let fixed = if existing {
            tr!(language, "wizard.review.fixedExisting")
        } else {
            tr!(language, "wizard.review.fixedNew")
        };
        // Identity first, then what Create does, then the runtime choices the
        // user must weigh, and only then the folded unchanged defaults.
        div()
            .id("wizard-review")
            .debug_selector(|| "wizard-review".into())
            .flex()
            .flex_col()
            .gap_4()
            .child(patterns::page_title(title))
            .child(self.wizard_review_identity(&values, existing, state.separated))
            .child(
                div()
                    .id("wizard-review-consequence")
                    .debug_selector(|| "wizard-review-consequence".into())
                    .w_full()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(note(consequence))
                    .child(note(fixed)),
            )
            .child(
                div()
                    .id("wizard-review-summary")
                    .debug_selector(|| "wizard-review-summary".into())
                    .w_full()
                    .min_w_0()
                    .flex()
                    .flex_wrap()
                    .gap_4()
                    .child(review_fact(language, Fact::Java, values.java.clone()))
                    .child(review_fact(language, Fact::Memory, values.memory.clone()))
                    .children(
                        summary
                            .into_iter()
                            .map(|(fact, value)| review_fact(language, fact, value)),
                    ),
            )
            .children(profile.is_some().then(|| {
                self.wizard_profile_options_trigger(
                    "wizard-review-options",
                    &tr!(language, "wizard.review.details"),
                    cx,
                )
            }))
            .children(profile.is_some().then(|| {
                let content = div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(note(tr!(language, "wizard.review.detailsNote")))
                    .child(
                        div().flex().flex_wrap().gap_4().children(
                            details
                                .into_iter()
                                .map(|(fact, value)| review_fact(language, fact, value)),
                        ),
                    )
                    .into_any_element();
                super::motion::disclosure(
                    "wizard-review-details",
                    &state.advanced.borrow(),
                    self.motion_preferences(),
                    content,
                    window,
                    cx,
                )
            }))
            .children(existing.then(|| note(tr!(language, "wizard.review.existingNote"))))
    }
}

/// The values the review states, resolved from the draft or from the existing
/// Instance whose new Profile it describes.
struct ReviewValues {
    game_name: String,
    version: String,
    directory: String,
    profile_name: String,
    appearance: block_launcher::catalog::ProfileAppearance,
    launch_target: String,
    memory: String,
    java: String,
    server: String,
    connect: bool,
    jvm: String,
    resolution: String,
    window_visible: bool,
}

/// The identity of one review fact. Its debug selector comes from this stable
/// name, never from the translated label.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fact {
    Server,
    AutomaticConnection,
    JvmArguments,
    GameWindow,
    GameFiles,
    GameData,
    Java,
    Memory,
}

impl Fact {
    const fn id(self) -> &'static str {
        match self {
            Self::Server => "server",
            Self::AutomaticConnection => "automatic-connection",
            Self::JvmArguments => "jvm-arguments",
            Self::GameWindow => "game-window",
            Self::GameFiles => "game-files",
            Self::GameData => "game-data",
            Self::Java => "java",
            Self::Memory => "memory",
        }
    }

    fn label(self, language: Language) -> String {
        let key = match self {
            Self::Server => "wizard.fact.server",
            Self::AutomaticConnection => "wizard.fact.automaticConnection",
            Self::JvmArguments => "wizard.fact.jvm",
            Self::GameWindow => "wizard.fact.gameWindow",
            Self::GameFiles => "wizard.fact.gameFiles",
            Self::GameData => "wizard.fact.gameData",
            Self::Java => "wizard.fact.java",
            Self::Memory => "wizard.fact.memory",
        };
        tr!(language, key)
    }
}

/// The summary carries every submitted value whose consequence the user must
/// weigh, including values inherited from saved defaults: arguments, a server,
/// an automatic connection, a location or a window choice all change what Create
/// writes. The fold keeps only values that decide nothing.
fn review_summary_facts(
    values: &ReviewValues,
    automatic_connection: Option<block_launcher::catalog::AutomaticConnection>,
    language: Language,
) -> Vec<(Fact, String)> {
    let mut facts = Vec::new();
    if !values.server.trim().is_empty() {
        facts.push((Fact::Server, values.server.clone()));
    }
    if values.connect {
        facts.push((
            Fact::AutomaticConnection,
            match automatic_connection {
                Some(block_launcher::catalog::AutomaticConnection::Legacy) => {
                    tr!(language, "wizard.review.automaticConnectionLegacy")
                }
                _ => tr!(language, "wizard.review.automaticConnectionQuickPlay"),
            },
        ));
    }
    if !values.jvm.trim().is_empty() {
        facts.push((Fact::JvmArguments, values.jvm.clone()));
    }
    if values.window_visible {
        facts.push((Fact::GameWindow, values.resolution.clone()));
    }
    if !values.directory.trim().is_empty() {
        facts.push((Fact::GameFiles, values.directory.clone()));
    }
    facts
}

/// The values that need no decision, and never a non-empty value claimed as
/// empty.
fn review_folded_facts(values: &ReviewValues, language: Language) -> Vec<(Fact, String)> {
    let mut facts = Vec::new();
    if values.server.trim().is_empty() {
        facts.push((Fact::Server, tr!(language, "wizard.review.chooseInGame")));
    }
    if !values.connect {
        facts.push((
            Fact::AutomaticConnection,
            tr!(language, "wizard.review.connectionOff"),
        ));
    }
    if values.jvm.trim().is_empty() {
        facts.push((
            Fact::JvmArguments,
            tr!(language, "wizard.review.noArguments"),
        ));
    }
    if !values.window_visible {
        facts.push((Fact::GameWindow, values.resolution.clone()));
    }
    if values.directory.trim().is_empty() {
        facts.push((
            Fact::GameFiles,
            tr!(language, "wizard.review.separateFolder"),
        ));
    }
    // Every Profile this wizard creates owns an isolated data folder; it is a
    // fixed policy rather than a choice, so it stays in the fold.
    facts.push((
        Fact::GameData,
        tr!(language, "wizard.review.isolatedFolder"),
    ));
    facts
}

/// One review fact: a muted 12 px label above a wrapping 14 px value.
fn review_fact(
    language: Language,
    fact: Fact,
    value: impl Into<gpui::SharedString>,
) -> gpui::Stateful<gpui::Div> {
    let id = format!("wizard-review-{}", fact.id());
    div()
        .id(id.clone())
        .debug_selector(move || id)
        .w(px(240.))
        // A fact longer than its column wraps instead of pushing out of view.
        .max_w_full()
        .min_w_0()
        .flex()
        .flex_col()
        .gap_1()
        .child(note(fact.label(language)))
        .child(div().text_sm().whitespace_normal().child(value.into()))
}
