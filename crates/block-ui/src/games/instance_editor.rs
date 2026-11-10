//! An Instance owns a draft. Detail subpages can commit to, or roll back to, that
//! draft without changing the catalog until the final Save action.
#![expect(
    clippy::redundant_pub_crate,
    reason = "Editor state is private to the product UI."
)]
use super::scroll_area::ScrollAreaExt as _;
use super::{
    Block, Form, InstanceEditForm, Language, TextInput, UiAction, account_id,
    components::{self, ControlState, FieldOptions},
    design::Icon,
    navigation::Screen,
    note, patterns,
};
use block_launcher::{
    catalog::{DataPolicy, Instance, TargetSource},
    install::optifine::OptiFineJar,
    recipe::RuntimeSelection,
};
use gpui::{
    Context, Entity, FocusHandle, Focusable as _, InteractiveElement as _, IntoElement,
    ParentElement as _, ScrollHandle, Styled as _, Window, div,
};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    fmt,
    path::PathBuf,
    rc::Rc,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum EditorPage {
    Appearance,
    Launch,
    Data,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum EditorField {
    Name,
    Memory,
    Java,
    Jvm,
    Game,
    Server,
    Width,
    Height,
    Directory,
    OptiFine,
}

/// A rejected draft value, kept with the draft until the user fixes the field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum EditorIssue {
    NameRequired,
    NameInvalid,
    Width,
    Height,
    Directory,
    Memory(components::MemoryError),
    Arguments(EditorField),
    /// A diagnostic from validation or argument parsing, kept verbatim.
    Reported(EditorField, String),
}

impl EditorIssue {
    const fn field(&self) -> EditorField {
        match self {
            Self::NameRequired | Self::NameInvalid => EditorField::Name,
            Self::Memory(_) => EditorField::Memory,
            Self::Width => EditorField::Width,
            Self::Height => EditorField::Height,
            Self::Directory => EditorField::Directory,
            Self::Arguments(field) | Self::Reported(field, _) => *field,
        }
    }

    /// Resolve the message in the language of the view that paints it.
    fn message(&self, language: Language) -> String {
        self.text().text(language)
    }

    fn text(&self) -> crate::i18n::UiText {
        match self {
            Self::NameRequired => msg!("games.instance_editor.error.name"),
            Self::NameInvalid => msg!("games.instance_editor.error.name_length"),
            Self::Memory(error) => msg!(error.key()),
            Self::Width => msg!("games.instance_editor.error.width"),
            Self::Height => msg!("games.instance_editor.error.height"),
            Self::Directory => msg!("games.instance_editor.error.directory"),
            Self::Arguments(_) => msg!("common.arguments_unclosed_quote"),
            Self::Reported(_, detail) => crate::i18n::UiText::Raw(detail.clone()),
        }
    }
}

/// Why the last save attempt did not reach the catalog.
#[derive(Clone, Debug, PartialEq, Eq)]
enum SaveFailure {
    /// The engine refused to start the save, so the draft stayed editable.
    Unavailable,
    /// A diagnostic from the catalog engine, kept verbatim.
    Reported(String),
}

impl SaveFailure {
    /// Resolve the message in the language of the view that paints it.
    fn message(&self, language: Language) -> String {
        match self {
            Self::Unavailable => tr!(language, "games.instance_editor.save_failed"),
            Self::Reported(detail) => detail.clone(),
        }
    }
}

/// Why the last OptiFine selection was not accepted; kept until the next attempt.
#[derive(Clone, Debug, PartialEq, Eq)]
enum OptiFineIssue {
    /// The system file chooser could not be opened.
    ChooserUnavailable,
    /// The selected JAR could not be imported; its diagnostic stays verbatim.
    ImportFailed(String),
    /// The JAR declares a different game version or loader; diagnostic verbatim.
    Rejected(String),
}

impl OptiFineIssue {
    /// Resolve the message in the language of the view that paints it.
    fn message(&self, language: Language) -> String {
        match self {
            Self::ChooserUnavailable => tr!(language, "games.optifine.chooser_failed"),
            Self::ImportFailed(detail) => {
                tr!(
                    language,
                    "games.optifine.import_failed",
                    error = detail.clone()
                )
            }
            Self::Rejected(detail) => detail.clone(),
        }
    }
}

#[derive(Clone, Debug)]
struct Checkpoint {
    original: Instance,
    values: [String; 9],
    managed: bool,
    window_custom: bool,
    optifine: Option<Box<OptiFineJar>>,
}
#[derive(Default)]
pub(super) struct EditorState {
    pub page: Option<EditorPage>,
    radio_focus: RefCell<HashMap<String, FocusHandle>>,
    checkpoint: Option<Checkpoint>,
    baseline: Option<Checkpoint>,
    error: Option<EditorIssue>,
    scroll: ScrollHandle,
    page_scroll: HashMap<Option<EditorPage>, gpui::Point<gpui::Pixels>>,
    reveal: Rc<Cell<Option<EditorField>>>,
    return_focus: Option<FocusHandle>,
    page_focus: Option<FocusHandle>,
    outer_focus: Option<FocusHandle>,
    outer_tab: Option<super::Tab>,
    optifine_error: Option<OptiFineIssue>,
    optifine_focus: Option<FocusHandle>,
    outer_instance: Option<String>,
    saving: bool,
    save_error: Option<SaveFailure>,
}

impl fmt::Debug for EditorState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EditorState")
            .field("page", &self.page)
            .field("saving", &self.saving)
            .field("error", &self.error)
            .field("save_error", &self.save_error)
            .finish_non_exhaustive()
    }
}

impl EditorState {
    pub(super) fn snapshot(&self) -> Self {
        let scroll = ScrollHandle::new();
        scroll.set_offset(self.scroll.offset());
        Self {
            page: self.page,
            optifine_error: self.optifine_error.clone(),
            checkpoint: self.checkpoint.clone(),
            baseline: self.baseline.clone(),
            error: self.error.clone(),
            save_error: self.save_error.clone(),
            saving: self.saving,
            page_scroll: self.page_scroll.clone(),
            outer_tab: self.outer_tab,
            outer_instance: self.outer_instance.clone(),
            scroll,
            reveal: Rc::new(Cell::new(self.reveal.get())),
            ..Default::default()
        }
    }
}

impl Block {
    pub(super) fn open_instance_editor(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.edit_instance(window, cx);
    }
    pub(super) fn open_instance_optifine_editor(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.edit_instance(window, cx);
        if matches!(self.form, Some(Form::EditInstance(_))) {
            let focus = cx.focus_handle().tab_stop(true);
            self.shell.editor.optifine_focus = Some(focus.clone());
            self.shell.editor.reveal.set(Some(EditorField::OptiFine));
            focus.focus(window, cx);
        }
    }
    pub(super) fn instance_runtime_automatic(&self) -> Option<bool> {
        match &self.form {
            Some(Form::EditInstance(form)) => Some(form.managed),
            _ => None,
        }
    }
    pub(super) fn instance_window_values(&self, cx: &gpui::App) -> Option<(String, String, bool)> {
        match &self.form {
            Some(Form::EditInstance(form)) => Some((
                form.width.read(cx).value(),
                form.height.read(cx).value(),
                form.window_custom,
            )),
            _ => None,
        }
    }
    pub(super) fn set_instance_window(
        &mut self,
        width: u32,
        height: u32,
        cx: &mut Context<'_, Self>,
    ) {
        if self.busy {
            return;
        }
        if let Some(Form::EditInstance(form)) = &mut self.form {
            form.width
                .update(cx, |field, cx| field.set_value(&width.to_string(), cx));
            form.height
                .update(cx, |field, cx| field.set_value(&height.to_string(), cx));
            form.window_custom = false;
        }
    }
    pub(super) fn custom_instance_window(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if let Some(Form::EditInstance(form)) = &mut self.form {
            form.window_custom = true;
            form.width.read(cx).focus_handle(cx).focus(window, cx);
            self.shell.editor.reveal.set(Some(EditorField::Width));
        }
    }
    pub(super) fn toggle_instance_fullscreen(&mut self) {
        if let Some(Form::EditInstance(form)) = &mut self.form {
            form.original.fullscreen = !form.original.fullscreen;
        }
    }
    fn instance_window_fields(&self, form: &InstanceEditForm, cx: &Context<'_, Self>) -> gpui::Div {
        let language = self.language();
        let size = format!(
            "{} × {}",
            form.width.read(cx).value(),
            form.height.read(cx).value()
        );
        let mut fields = div()
            .flex()
            .flex_col()
            .gap_3()
            .child(self.product_select_field(
                "instance-window",
                tr!(language, "select.window_size"),
                &size,
                super::select::ProductChoice::InstanceWindow,
                cx,
            ));
        if form.window_custom
            || self.shell.editor.error.as_ref().is_some_and(|issue| {
                matches!(issue.field(), EditorField::Width | EditorField::Height)
            })
        {
            fields = fields
                .child(self.instance_field(
                    EditorField::Width,
                    tr!(language, "games.instance_editor.window_width"),
                    &form.width,
                ))
                .child(self.instance_field(
                    EditorField::Height,
                    tr!(language, "games.instance_editor.window_height"),
                    &form.height,
                ));
        }
        fields.child(components::checkbox(
            "instance-fullscreen",
            tr!(language, "games.instance_editor.fullscreen"),
            form.original.fullscreen,
            self.palette,
            ControlState {
                disabled: self.busy,
                ..Default::default()
            },
            cx.listener(|view, (): &(), window, cx| {
                view.handle(UiAction::InstanceFullscreen, window, cx)
            }),
        ))
    }
    fn instance_optifine_section(
        &self,
        form: &InstanceEditForm,
        cx: &Context<'_, Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let language = self.language();
        let supported = form
            .original
            .targets
            .iter()
            .find(|target| target.id == form.original.target_id)
            .is_some_and(|target| {
                matches!(
                    target.kind,
                    block_launcher::catalog::LoaderKind::Vanilla
                        | block_launcher::catalog::LoaderKind::Forge
                )
            });
        let mut body = div()
            .flex()
            .flex_col()
            .gap_2()
            .child("OptiFine")
            .child(note(tr!(language, "games.optifine.intro")));
        if supported {
            let mut choose = self.button(
                "instance-optifine-select",
                tr!(
                    language,
                    if form.optifine.is_some() {
                        "games.optifine.replace"
                    } else {
                        "games.optifine.select"
                    }
                ),
                UiAction::InstanceOptiFineFile,
                cx,
            );
            if let Some(focus) = &self.shell.editor.optifine_focus {
                choose = choose.track_focus(focus);
            }
            body = body.child(choose);
            if let Some(selected) = &form.optifine {
                body = body
                    .child(note(tr!(
                        language,
                        "games.optifine.selected",
                        version = selected.version
                    )))
                    .child(self.button(
                        "instance-optifine-remove",
                        tr!(language, "games.optifine.remove"),
                        UiAction::InstanceOptiFineRemove,
                        cx,
                    ));
            }
        } else {
            body = body.child(note(tr!(
                language,
                "games.optifine.unsupported",
                loader = form
                    .original
                    .targets
                    .iter()
                    .find(|target| target.id == form.original.target_id)
                    .map_or("", |target| target.kind.name())
            )));
            if let Some(focus) = &self.shell.editor.optifine_focus {
                body = body.track_focus(focus);
            }
        }
        if let Some(error) = &self.shell.editor.optifine_error {
            body = body.child(
                note(error.message(language))
                    .id("instance-optifine-error")
                    .debug_selector(|| "instance-optifine-error".into()),
            );
        }
        patterns::reveal_field(
            body,
            self.shell.editor.scroll.clone(),
            self.shell.editor.reveal.clone(),
            EditorField::OptiFine,
        )
        .id("instance-optifine")
        .debug_selector(|| "instance-optifine".into())
    }
    pub(super) fn browse_instance_optifine(&mut self, cx: &mut Context<'_, Self>) {
        if self.busy {
            return;
        }
        let Some(Form::EditInstance(editor)) = &mut self.form else {
            return;
        };
        if editor.optifine_request.is_some()
            || !editor
                .original
                .targets
                .iter()
                .find(|target| target.id == editor.original.target_id)
                .is_some_and(|target| {
                    matches!(
                        target.kind,
                        block_launcher::catalog::LoaderKind::Vanilla
                            | block_launcher::catalog::LoaderKind::Forge
                    )
                })
        {
            return;
        }
        let identity = editor.original.id.as_str().to_owned();
        let target_id = editor.original.target_id.clone();
        let field = editor.name.entity_id();
        let Some(engine) = self.engine.clone() else {
            return;
        };
        let request = uuid::Uuid::new_v4();
        editor.optifine_request = Some(request);
        let prompt = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(tr!(self.language(), "games.optifine.prompt").into()),
        });
        cx.notify();
        cx.spawn(async move |view, cx| {
            let selection = match prompt.await {
                Ok(Ok(paths)) => Ok(paths.and_then(|paths| paths.into_iter().next())),
                _ => Err(()),
            };
            let path = view
                .update(cx, |view, cx| {
                    view.finish_instance_optifine_browse(
                        request, field, &identity, &target_id, selection, cx,
                    )
                })
                .ok()
                .flatten();
            let Some(path) = path else {
                return;
            };
            let result = engine.import_optifine(path).await;
            let _ = view.update(cx, |view, cx| {
                view.finish_instance_optifine_import(request, &identity, &target_id, result, cx);
            });
        })
        .detach();
    }

    fn finish_instance_optifine_browse(
        &mut self,
        request: uuid::Uuid,
        field: gpui::EntityId,
        identity: &str,
        target_id: &str,
        selection: Result<Option<PathBuf>, ()>,
        cx: &mut Context<'_, Self>,
    ) -> Option<PathBuf> {
        let Some(Form::EditInstance(editor)) = &mut self.form else {
            return None;
        };
        if editor.optifine_request != Some(request)
            || editor.name.entity_id() != field
            || editor.original.id.as_str() != identity
            || editor.original.target_id != target_id
        {
            return None;
        }
        if self.busy
            || !editor
                .original
                .targets
                .iter()
                .find(|target| target.id == editor.original.target_id)
                .is_some_and(|target| {
                    matches!(
                        target.kind,
                        block_launcher::catalog::LoaderKind::Vanilla
                            | block_launcher::catalog::LoaderKind::Forge
                    )
                })
        {
            editor.optifine_request = None;
            cx.notify();
            return None;
        }
        match selection {
            Ok(Some(path)) => {
                self.shell.editor.optifine_error = None;
                cx.notify();
                Some(path)
            }
            Ok(None) => {
                editor.optifine_request = None;
                cx.notify();
                None
            }
            Err(()) => {
                editor.optifine_request = None;
                self.shell.editor.optifine_error = Some(OptiFineIssue::ChooserUnavailable);
                cx.notify();
                None
            }
        }
    }

    fn finish_instance_optifine_import(
        &mut self,
        request: uuid::Uuid,
        identity: &str,
        target_id: &str,
        result: anyhow::Result<OptiFineJar>,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(Form::EditInstance(editor)) = &mut self.form else {
            return;
        };
        if editor.optifine_request != Some(request)
            || editor.original.id.as_str() != identity
            || editor.original.target_id != target_id
        {
            return;
        }
        editor.optifine_request = None;
        match result {
            Ok(selected) => {
                let Some(target) = editor
                    .original
                    .targets
                    .iter()
                    .find(|target| target.id == target_id)
                else {
                    return;
                };
                match selected.validate_for(
                    &editor.original.minecraft_version,
                    &target.kind,
                    target.loader_version.as_deref(),
                ) {
                    Ok(()) => {
                        editor.optifine = Some(Box::new(selected));
                        self.shell.editor.optifine_error = None;
                    }
                    Err(error) => {
                        self.shell.editor.optifine_error =
                            Some(OptiFineIssue::Rejected(error.to_string()))
                    }
                }
            }
            Err(error) => {
                self.shell.editor.optifine_error =
                    Some(OptiFineIssue::ImportFailed(format!("{error:#}")));
            }
        }
        cx.notify();
    }

    pub(super) fn remove_instance_optifine(&mut self, cx: &mut Context<'_, Self>) {
        if self.busy {
            return;
        }
        if let Some(Form::EditInstance(editor)) = &mut self.form {
            if editor.optifine_request.is_some() {
                return;
            }
            editor.optifine = None;
            self.shell.editor.optifine_error = None;
            cx.notify();
        }
    }

    pub(super) fn begin_instance_editor(
        &mut self,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> EditorState {
        let original = self.inspected_instance().cloned();
        let baseline = original.as_ref().map(|profile| Checkpoint {
            values: [
                profile.name.clone(),
                components::memory_gb_text(profile.memory_mib),
                match &profile.runtime {
                    RuntimeSelection::Managed => String::new(),
                    RuntimeSelection::Local { java } => java.display().to_string(),
                },
                super::format_arguments(&profile.jvm_args),
                super::format_arguments(&profile.game_args),
                profile.server.address.clone(),
                profile.width.to_string(),
                profile.height.to_string(),
                profile.game_directory.display().to_string(),
            ],
            managed: matches!(profile.runtime, RuntimeSelection::Managed),
            window_custom: !super::settings::WINDOW_PRESETS
                .contains(&(profile.width, profile.height)),
            optifine: profile
                .targets
                .iter()
                .find(|target| target.id == profile.target_id)
                .and_then(|target| target.optifine.clone())
                .map(Box::new),
            original: profile.clone(),
        });
        let state = EditorState {
            outer_focus: window
                .focused(cx)
                .or_else(|| Some(self.instance_editor_trigger_focus(cx))),
            outer_tab: Some(self.router.tab()),
            outer_instance: original.as_ref().map(|profile| profile.id.clone()),
            baseline,
            ..EditorState::default()
        };
        if let Some(profile) = original {
            if let Some(focus) = &state.outer_focus {
                self.shell
                    .return_focus
                    .insert(self.router.entry_id(), focus.clone());
            }
            self.router
                .open_screen(self.router.tab(), Screen::InstanceEditor(profile.id));
        }
        state
    }

    /// Compare the complete root draft, including invalid raw text and changes
    /// applied by a subsection, without validating or mutating that draft.
    pub(super) fn instance_editor_dirty(&self, cx: &gpui::App) -> bool {
        let (Some(Form::EditInstance(form)), Some(baseline)) =
            (&self.form, &self.shell.editor.baseline)
        else {
            return false;
        };
        let original = &baseline.original;
        form.managed != baseline.managed
            || [
                &form.name,
                &form.memory,
                &form.java,
                &form.jvm,
                &form.game,
                &form.server,
                &form.width,
                &form.height,
                &form.game_directory,
            ]
            .into_iter()
            .zip(&baseline.values)
            .any(|(field, value)| field.read(cx).value() != *value)
            || form.original.target_id != original.target_id
            || form.original.appearance != original.appearance
            || form.original.fullscreen != original.fullscreen
            || form.window_custom != baseline.window_custom
            || form.optifine != baseline.optifine
            || form.original.server != original.server
            || form.original.data_policy != original.data_policy
            || form.original.default_account != original.default_account
    }

    fn instance_editor_return_focus(
        &self,
        state: &EditorState,
        cx: &Context<'_, Self>,
    ) -> Option<FocusHandle> {
        // A user who navigated to another domain owns that domain's current focus.
        if state.outer_tab != Some(self.router.tab()) {
            return None;
        }
        let original_is_current = state.outer_instance.as_ref().is_some_and(|id| {
            self.snapshot
                .instances
                .iter()
                .any(|profile| &profile.id == id)
                && self
                    .inspected_instance()
                    .is_some_and(|profile| &profile.id == id)
        });
        Some(if original_is_current {
            state
                .outer_focus
                .clone()
                .unwrap_or_else(|| self.browser_detail_focus(cx))
        } else {
            self.browser_detail_focus(cx)
        })
    }

    pub(super) fn cancel_instance_editor(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.busy || !matches!(self.form, Some(Form::EditInstance(_))) {
            return;
        }
        let editor = std::mem::take(&mut self.shell.editor);
        self.form = None;
        if matches!(self.router.screen(), Screen::InstanceEditor(id) if Some(id) == editor.outer_instance.as_ref())
        {
            self.router.back();
            if let Some(focus) = self.instance_editor_return_focus(&editor, cx) {
                self.shell
                    .return_focus
                    .entry(self.router.entry_id())
                    .or_insert(focus);
            }
        }
        cx.notify();
    }

    pub(super) fn instance_edit_finished(
        &mut self,
        result: &Result<crate::i18n::UiText, String>,
        cx: &mut Context<'_, Self>,
    ) {
        if !self.shell.editor.saving {
            return;
        }
        if let Err(error) = result {
            self.shell.editor.saving = false;
            self.shell.editor.save_error = Some(SaveFailure::Reported(error.clone()));
        } else {
            let editor = std::mem::take(&mut self.shell.editor);
            if matches!(&self.form, Some(Form::EditInstance(form)) if Some(&form.original.id) == editor.outer_instance.as_ref())
            {
                self.form = None;
            }
            if matches!(self.router.screen(), Screen::InstanceEditor(id) if Some(id) == editor.outer_instance.as_ref())
            {
                self.router.back();
                if let Some(focus) = self.instance_editor_return_focus(&editor, cx) {
                    self.shell
                        .return_focus
                        .entry(self.router.entry_id())
                        .or_insert(focus);
                }
            }
        }
    }

    pub(super) fn open_instance_page(
        &mut self,
        page: EditorPage,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(Form::EditInstance(form)) = &self.form else {
            return;
        };
        self.shell.editor.checkpoint = Some(Checkpoint {
            original: form.original.clone(),
            managed: form.managed,
            window_custom: form.window_custom,
            optifine: form.optifine.clone(),
            values: [
                form.name.read(cx).value(),
                form.memory.read(cx).value(),
                form.java.read(cx).value(),
                form.jvm.read(cx).value(),
                form.game.read(cx).value(),
                form.server.read(cx).value(),
                form.width.read(cx).value(),
                form.height.read(cx).value(),
                form.game_directory.read(cx).value(),
            ],
        });
        self.shell
            .editor
            .page_scroll
            .insert(self.shell.editor.page, self.shell.editor.scroll.offset());
        self.shell.editor.page = Some(page);
        self.shell.editor.error = None;
        self.shell.editor.scroll.set_offset(
            self.shell
                .editor
                .page_scroll
                .get(&Some(page))
                .copied()
                .unwrap_or_default(),
        );
        self.shell.editor.return_focus = window.focused(cx);
        let focus = cx.focus_handle().tab_stop(true);
        if page == EditorPage::Launch {
            form.memory.read(cx).focus_handle(cx).focus(window, cx);
        } else {
            focus.focus(window, cx);
        }
        self.shell.editor.page_focus = Some(focus);
    }

    pub(super) fn finish_instance_page(
        &mut self,
        cancel: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if let Some(checkpoint) = self.shell.editor.checkpoint.take()
            && cancel
            && let Some(Form::EditInstance(form)) = &mut self.form
        {
            form.original = checkpoint.original;
            form.managed = checkpoint.managed;
            form.window_custom = checkpoint.window_custom;
            form.optifine = checkpoint.optifine;
            for (field, value) in [
                &form.name,
                &form.memory,
                &form.java,
                &form.jvm,
                &form.game,
                &form.server,
                &form.width,
                &form.height,
                &form.game_directory,
            ]
            .into_iter()
            .zip(checkpoint.values)
            {
                field.update(cx, |field, cx| field.set_value(&value, cx));
            }
        }
        self.shell
            .editor
            .page_scroll
            .insert(self.shell.editor.page, self.shell.editor.scroll.offset());
        self.shell.editor.page = None;
        self.shell.editor.scroll.set_offset(
            self.shell
                .editor
                .page_scroll
                .get(&None)
                .copied()
                .unwrap_or_default(),
        );
        self.shell.editor.error = None;
        if let Some(focus) = self.shell.editor.return_focus.take() {
            focus.focus(window, cx);
        }
        self.shell.editor.page_focus = None;
    }

    pub(super) fn set_instance_runtime_automatic(
        &mut self,
        automatic: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.busy {
            return;
        }
        if let Some(Form::EditInstance(form)) = &mut self.form {
            form.managed = automatic;
            if self
                .shell
                .editor
                .error
                .as_ref()
                .is_some_and(|issue| issue.field() == EditorField::Java)
            {
                self.shell.editor.error = None;
            }
            if !automatic {
                form.java.read(cx).focus_handle(cx).focus(window, cx);
                self.shell.editor.reveal.set(Some(EditorField::Java));
            }
        }
    }

    pub(super) fn set_instance_data_policy(&mut self, policy: DataPolicy) {
        if self.busy {
            return;
        }
        if let Some(Form::EditInstance(form)) = &mut self.form {
            form.original.data_policy = policy;
        }
    }

    pub(super) fn render_instance_editor(
        &self,
        cx: &Context<'_, Self>,
    ) -> super::page_host::ContentSlots {
        let Some(Form::EditInstance(form)) = &self.form else {
            return super::page_host::ContentSlots {
                body: div().into_any_element(),
                footer: div().into_any_element(),
            };
        };
        let language = self.language();
        let page = self.shell.editor.page;
        let body = div()
            .id("instance-editor-body")
            .debug_selector(|| "instance-editor-body".into())
            .flex_1()
            .min_h_0()
            .scroll_area(
                &self.shell.editor.scroll,
                "games.instance_editor",
                self.palette,
                false,
            )
            .p_4()
            .flex()
            .flex_col()
            .gap_5();
        let mut body = match page {
            None => self.instance_overview_fields(body, form, cx),
            Some(EditorPage::Appearance) => self.instance_appearance_fields(body, form, cx),
            Some(EditorPage::Launch) => self.instance_launch_fields(body, form, cx),
            Some(EditorPage::Data) => self.instance_data_fields(body, form, cx),
        };
        if let Some(issue) = &self.shell.editor.error {
            body = body.child(note(issue.message(language)));
        }
        let footer = patterns::management_footer(self.palette)
            .id("instance-editor-footer")
            .debug_selector(|| "instance-editor-footer".into())
            .flex_col()
            .items_stretch()
            .gap_2()
            .children(self.instance_save_error())
            .child(self.button(
                if page.is_some() {
                    "cancel-instance-page"
                } else {
                    "cancel-instance"
                },
                tr!(language, "common.cancel"),
                if page.is_some() {
                    UiAction::InstancePageCancel
                } else {
                    UiAction::NavigationBack
                },
                cx,
            ))
            .child(self.button(
                if page.is_some() {
                    "done-instance-page"
                } else {
                    "save-instance"
                },
                if page.is_some() {
                    tr!(language, "common.apply_draft")
                } else {
                    tr!(language, "games.action.save_changes")
                },
                if page.is_some() {
                    UiAction::InstancePageDone
                } else {
                    UiAction::SaveInstance
                },
                cx,
            ));
        super::page_host::ContentSlots {
            body: body.into_any_element(),
            footer: footer.into_any_element(),
        }
    }

    fn instance_save_error(&self) -> Option<gpui::Stateful<gpui::Div>> {
        let language = self.language();
        self.shell.editor.save_error.as_ref().map(|failure| {
            div()
                .id("instance-save-error")
                .debug_selector(|| "instance-save-error".into())
                .min_w_0()
                .flex_shrink_0()
                .px_4()
                .py_2()
                .text_sm()
                .text_color(gpui::rgb(self.palette.destructive))
                .whitespace_normal()
                .line_clamp(3)
                .child(failure.message(language))
        })
    }

    fn instance_account_group(
        &self,
        form: &InstanceEditForm,
        cx: &Context<'_, Self>,
    ) -> gpui_base::RadioGroup {
        let language = self.language();
        let mut options = vec![components::RadioOption {
            id: "current".into(),
            checked: form.original.default_account.is_none(),
            disabled: self.busy,
            action: UiAction::DefaultAccount(None),
        }];
        options.extend(self.accounts.iter().map(|account| {
            let id = account_id(account);
            components::RadioOption {
                checked: form.original.default_account.as_ref() == Some(&id),
                id: id.clone(),
                disabled: self.busy,
                action: UiAction::DefaultAccount(Some(id)),
            }
        }));
        let group = components::RadioGroup::new(
            "instance-default-account-group",
            tr!(language, "games.instance_editor.default_account_group"),
            options,
            &self.shell.editor.radio_focus,
            cx,
            cx.listener(|view, action: &UiAction, window, cx| {
                view.handle(action.clone(), window, cx);
            }),
        );
        group
            .container()
            .flex()
            .flex_col()
            .gap_2()
            .child(group.item(
                "current",
                components::radio(
                    "default-account-none",
                    tr!(self.language(), "games.instance_editor.use_current_account"),
                    form.original.default_account.is_none(),
                    self.palette,
                    ControlState {
                        disabled: self.busy,
                        ..Default::default()
                    },
                    cx.listener(|view, &(): &(), window, cx| {
                        view.handle(UiAction::DefaultAccount(None), window, cx);
                    }),
                ),
            ))
            .children(self.accounts.iter().map(|account| {
                let id = account_id(account);
                let key = id.clone();
                group.item(
                    &key,
                    components::radio(
                        format!("default-account-{id}"),
                        account.player_name.clone(),
                        form.original.default_account.as_deref() == Some(&id),
                        self.palette,
                        ControlState {
                            disabled: self.busy,
                            ..Default::default()
                        },
                        cx.listener(move |view, &(): &(), window, cx| {
                            view.handle(UiAction::DefaultAccount(Some(id.clone())), window, cx);
                        }),
                    ),
                )
            }))
    }

    fn instance_launch_fields(
        &self,
        body: super::scroll_area::ScrollArea<gpui::Stateful<gpui::Div>>,
        form: &InstanceEditForm,
        cx: &Context<'_, Self>,
    ) -> super::scroll_area::ScrollArea<gpui::Stateful<gpui::Div>> {
        let language = self.language();
        let mut fields = body
            .child(patterns::page_title(tr!(language, "games.launch.settings")))
            .child({
                let runtime = if form.managed {
                    tr!(language, "games.java.automatic")
                } else {
                    tr!(language, "games.java.custom_executable")
                };
                self.product_select_field(
                    "instance-runtime",
                    tr!(language, "select.java_runtime"),
                    &runtime,
                    super::select::ProductChoice::InstanceRuntime,
                    cx,
                )
            });
        if !form.managed {
            fields = fields.child(self.instance_field(
                EditorField::Java,
                tr!(language, "games.java.executable"),
                &form.java,
            ));
        }
        fields = fields.child(self.instance_window_fields(form, cx));
        fields = fields
            .child(self.instance_field(
                EditorField::Memory,
                tr!(language, "games.instance_editor.memory"),
                &form.memory,
            ))
            .child(self.instance_field(
                EditorField::Jvm,
                tr!(language, "games.instance_editor.jvm"),
                &form.jvm,
            ))
            .child(self.instance_field(
                EditorField::Game,
                tr!(language, "games.instance_editor.game"),
                &form.game,
            ))
            .child(note(tr!(language, "games.instance_editor.arguments_note")));
        if let Some(instance) = self
            .snapshot
            .instances
            .iter()
            .find(|instance| instance.id == form.original.id)
        {
            let group = components::RadioGroup::new(
                "instance-target-group",
                tr!(language, "games.instance_editor.launch_target"),
                instance
                    .targets
                    .iter()
                    .map(|target| components::RadioOption {
                        id: target.id.clone(),
                        checked: target.id == form.original.target_id,
                        disabled: self.busy,
                        action: UiAction::InstanceTarget(target.id.clone()),
                    })
                    .collect(),
                &self.shell.editor.radio_focus,
                cx,
                cx.listener(|view, action: &UiAction, window, cx| {
                    view.handle(action.clone(), window, cx);
                }),
            );
            fields = fields
                .child(div().child(tr!(language, "games.instance_editor.launch_target")))
                .child(group.container().flex().flex_col().gap_2().children(
                    instance.targets.iter().map(|target| {
                        let id = target.id.clone();
                        group.item(
                            &target.id,
                            components::radio(
                                format!("target-{id}"),
                                id.clone(),
                                target.id == form.original.target_id,
                                self.palette,
                                ControlState {
                                    disabled: self.busy,
                                    ..Default::default()
                                },
                                cx.listener(move |view, (): &(), window, cx| {
                                    view.handle(UiAction::InstanceTarget(id.clone()), window, cx);
                                }),
                            ),
                        )
                    }),
                ));
        }
        fields.child(self.instance_optifine_section(form, cx))
    }

    /// The root page: naming, server and the entries into the detail pages.
    fn instance_overview_fields(
        &self,
        body: super::scroll_area::ScrollArea<gpui::Stateful<gpui::Div>>,
        form: &InstanceEditForm,
        cx: &Context<'_, Self>,
    ) -> super::scroll_area::ScrollArea<gpui::Stateful<gpui::Div>> {
        let language = self.language();
        body.child(patterns::page_title(tr!(language, "games.launch.settings")))
            .child(self.instance_field(
                EditorField::Name,
                tr!(language, "games.instance_editor.name"),
                &form.name,
            ))
            .child(self.instance_field(
                EditorField::Server,
                tr!(language, "games.instance_editor.server"),
                &form.server,
            ))
            .child(self.instance_server_choice(form, cx))
            .child(
                self.button(
                    "edit-instance-appearance",
                    tr!(language, "games.instance_editor.appearance"),
                    UiAction::InstancePage(EditorPage::Appearance),
                    cx,
                )
                .w_full()
                .justify_between()
                .child(Icon::ChevronRight.render(16., self.palette.muted))
                .child(components::instance_avatar(
                    form.original.appearance,
                    self.palette,
                    32.,
                )),
            )
            .child(
                self.button(
                    "edit-instance-launch",
                    tr!(language, "games.launch.settings"),
                    UiAction::InstancePage(EditorPage::Launch),
                    cx,
                )
                .w_full()
                .justify_between()
                .child(Icon::ChevronRight.render(16., self.palette.muted)),
            )
            .child(
                self.button(
                    "edit-instance-data",
                    tr!(language, "games.instance_editor.data_and_account"),
                    UiAction::InstancePage(EditorPage::Data),
                    cx,
                )
                .w_full()
                .justify_between()
                .child(Icon::ChevronRight.render(16., self.palette.muted)),
            )
            .child(self.instance_optifine_section(form, cx))
            .child(note(tr!(language, "games.instance_editor.draft_note")))
    }

    /// The appearance page paints the draft avatar above both choice entries.
    fn instance_appearance_fields(
        &self,
        body: super::scroll_area::ScrollArea<gpui::Stateful<gpui::Div>>,
        form: &InstanceEditForm,
        cx: &Context<'_, Self>,
    ) -> super::scroll_area::ScrollArea<gpui::Stateful<gpui::Div>> {
        let language = self.language();
        body.child(patterns::page_title(tr!(
            language,
            "games.instance_editor.appearance_title"
        )))
        .child(components::instance_avatar(
            form.original.appearance,
            self.palette,
            80.,
        ))
        .child(self.instance_page_trigger(self.button(
            "instance-icon",
            crate::i18n::instance_icon_name(language, form.original.appearance.icon),
            UiAction::ChooseAppearance {
                wizard: false,
                color: false,
            },
            cx,
        )))
        .child(self.button(
            "instance-color",
            crate::i18n::instance_color_name(language, form.original.appearance.color),
            UiAction::ChooseAppearance {
                wizard: false,
                color: true,
            },
            cx,
        ))
    }

    /// The data page chooses the draft policy and default launch account.
    fn instance_data_fields(
        &self,
        body: super::scroll_area::ScrollArea<gpui::Stateful<gpui::Div>>,
        form: &InstanceEditForm,
        cx: &Context<'_, Self>,
    ) -> super::scroll_area::ScrollArea<gpui::Stateful<gpui::Div>> {
        let language = self.language();
        body.child(patterns::page_title(tr!(
            language,
            "games.instance_editor.data_and_account"
        )))
        .child({
            let policy = match form.original.data_policy {
                DataPolicy::Isolated => tr!(language, "games.instance_editor.data_isolated"),
                DataPolicy::Shared => tr!(language, "games.instance_editor.data_shared"),
            };
            self.product_select_field(
                "data-policy",
                tr!(language, "select.game_data"),
                &policy,
                super::select::ProductChoice::InstanceDataPolicy,
                cx,
            )
        })
        .child(self.instance_field(
            EditorField::Directory,
            tr!(language, "games.instance_editor.directory"),
            &form.game_directory,
        ))
        .child(note(tr!(language, "import.policy.files_remain")))
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(tr!(language, "games.instance_editor.default_account_group"))
                .child(self.instance_account_group(form, cx)),
        )
    }

    fn instance_server_choice(&self, form: &InstanceEditForm, cx: &Context<'_, Self>) -> gpui::Div {
        let language = self.language();
        let version = self
            .snapshot
            .instances
            .iter()
            .find(|instance| instance.id == form.original.id)
            .map(|instance| instance.minecraft_version.as_str());
        let connection = version.and_then(|version| self.automatic_connection(version));
        let supported = connection.is_some_and(|connection| {
            connection != block_launcher::catalog::AutomaticConnection::Unsupported
        });
        let address = form.server.read(cx).value();
        let mut choice = div().flex().flex_col().gap_2();
        if !address.trim().is_empty() {
            choice = choice.child(components::checkbox(
                "instance-connect",
                tr!(language, "games.instance_editor.connect"),
                form.original.server.connect_on_launch,
                self.palette,
                ControlState {
                    disabled: self.busy || !supported && !form.original.server.connect_on_launch,
                    ..Default::default()
                },
                cx.listener(|view, (): &(), window, cx| {
                    view.handle(UiAction::InstanceConnect, window, cx);
                }),
            ));
        }
        if !supported {
            choice = choice.child(note(match connection {
                Some(block_launcher::catalog::AutomaticConnection::Unsupported) => {
                    tr!(language, "games.instance_editor.server_unsupported")
                }
                _ => tr!(language, "games.instance_editor.server_unknown"),
            }));
        }
        choice
    }

    /// Which automatic connection form this version accepts. Installed official
    /// manifests are authoritative; otherwise the official catalog entry decides
    /// the family and the 1.20 Quick Play series boundary.
    pub(super) fn automatic_connection(
        &self,
        version: &str,
    ) -> Option<block_launcher::catalog::AutomaticConnection> {
        for instance in self
            .snapshot
            .instances
            .iter()
            .filter(|instance| instance.minecraft_version == version)
        {
            for target in &instance.targets {
                if let TargetSource::Managed {
                    manifest: Some(path),
                } = &target.source
                    && let Ok(bytes) = std::fs::read(path)
                    && let Ok(manifest) = block_launcher::manifest::decode::<
                        block_launcher::manifest::model::VersionManifest,
                    >(&bytes)
                {
                    return Some(block_launcher::catalog::automatic_connection(&manifest));
                }
            }
        }
        let entry = self
            .version_catalog
            .iter()
            .find(|entry| entry.id == version)?;
        Some(match entry.kind {
            block_launcher::manifest::model::VersionType::OldAlpha
            | block_launcher::manifest::model::VersionType::OldBeta => {
                block_launcher::catalog::AutomaticConnection::Unsupported
            }
            block_launcher::manifest::model::VersionType::Release
            | block_launcher::manifest::model::VersionType::Snapshot => {
                // Mojang's 1.20 release data introduced Quick Play. Numeric
                // release ids are `1.<series>.<patch>`; older clients use the
                // legacy server arguments.
                if version
                    .split('.')
                    .nth(1)
                    .and_then(|series| series.parse::<u32>().ok())
                    .is_some_and(|series| series >= 20)
                {
                    block_launcher::catalog::AutomaticConnection::QuickPlay
                } else {
                    block_launcher::catalog::AutomaticConnection::Legacy
                }
            }
        })
    }

    fn instance_page_trigger(&self, button: components::Control) -> components::Control {
        if let Some(focus) = &self.shell.editor.page_focus {
            button.track_focus(focus)
        } else {
            button
        }
    }

    fn instance_field(
        &self,
        field: EditorField,
        label: impl Into<gpui::SharedString>,
        input: &Entity<TextInput>,
    ) -> gpui::Stateful<gpui::Div> {
        let error = self
            .shell
            .editor
            .error
            .as_ref()
            .filter(|issue| issue.field() == field)
            .map(|issue| issue.message(self.language()).into());
        patterns::reveal_field(
            components::field_with(
                label,
                input,
                self.palette,
                FieldOptions {
                    error,
                    ..Default::default()
                },
            ),
            self.shell.editor.scroll.clone(),
            self.shell.editor.reveal.clone(),
            field,
        )
        .id(gpui::SharedString::from(format!(
            "instance-field-{field:?}"
        )))
        .debug_selector(move || format!("instance-field-{field:?}"))
    }

    pub(super) fn save_instance_draft(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.busy {
            return;
        }
        let Some(Form::EditInstance(form)) = &self.form else {
            return;
        };
        let optifine = form.optifine.clone();
        let component_changed = form
            .original
            .targets
            .iter()
            .find(|target| target.id == form.original.target_id)
            .is_some_and(|target| target.optifine.as_ref() != optifine.as_deref());
        if form.optifine_request.is_some() {
            return;
        }
        match parse_instance(form, cx) {
            Ok(profile) => {
                self.shell.editor.error = None;
                if component_changed {
                    self.begin_cancellable(
                        msg!("games.instance_editor.saving_optifine"),
                        super::ui_logging::Operation::SaveInstance,
                    );
                } else {
                    self.begin(
                        msg!("games.instance_editor.saving"),
                        super::ui_logging::Operation::SaveInstance,
                    );
                }
                self.shell.editor.saving = self.busy;
                self.shell.editor.save_error = (!self.busy).then_some(SaveFailure::Unavailable);
                if let Some(engine) = &self.engine {
                    let draft = super::engine::InstanceDraftValues {
                        name: profile.name.clone(),
                        defaults: super::settings::GameDefaults {
                            memory_mib: profile.memory_mib,
                            java: match &profile.runtime {
                                RuntimeSelection::Managed => String::new(),
                                RuntimeSelection::Local { java } => java.display().to_string(),
                            },
                            jvm_arguments: super::format_arguments(&profile.jvm_args),
                            width: profile.width,
                            height: profile.height,
                            fullscreen: profile.fullscreen,
                            show_snapshots: false,
                        },
                        appearance: profile.appearance,
                        target_id: Some(profile.target_id.clone()),
                        server: profile.server.clone(),
                        data_policy: profile.data_policy,
                        game_directory: profile.game_directory.clone(),
                        game_args: profile.game_args.clone(),
                        default_account: profile.default_account.clone(),
                    };
                    engine.save_instance(profile.id, draft, optifine.as_deref().cloned());
                }
            }
            Err(issue) => {
                let field = issue.field();
                self.shell.editor.page = if matches!(field, EditorField::Name | EditorField::Server)
                {
                    None
                } else if field == EditorField::Directory {
                    Some(EditorPage::Data)
                } else {
                    Some(EditorPage::Launch)
                };
                self.shell.editor.error = Some(issue.clone());
                self.shell.editor.reveal.set(Some(field));
                let input = match field {
                    EditorField::Name | EditorField::OptiFine => &form.name,
                    EditorField::Memory => &form.memory,
                    EditorField::Java => &form.java,
                    EditorField::Jvm => &form.jvm,
                    EditorField::Game => &form.game,
                    EditorField::Server => &form.server,
                    EditorField::Width => &form.width,
                    EditorField::Height => &form.height,
                    EditorField::Directory => &form.game_directory,
                };
                input.read(cx).focus_handle(cx).focus(window, cx);
                self.report(issue.text());
            }
        }
    }
}

pub(super) fn parse_instance(
    form: &InstanceEditForm,
    cx: &Context<'_, Block>,
) -> Result<Instance, EditorIssue> {
    let mut profile = form.original.clone();
    form.name
        .read(cx)
        .value()
        .trim()
        .clone_into(&mut profile.name);
    if profile.name.is_empty() {
        return Err(EditorIssue::NameRequired);
    }
    if profile.name.len() > 160 || profile.name.chars().any(char::is_control) {
        return Err(EditorIssue::NameInvalid);
    }
    form.server
        .read(cx)
        .value()
        .trim()
        .clone_into(&mut profile.server.address);
    if profile.server.address.is_empty() {
        profile.server.connect_on_launch = false;
    }
    profile
        .server
        .validate()
        .map_err(|error| EditorIssue::Reported(EditorField::Server, error.to_string()))?;
    profile.memory_mib =
        components::parse_memory_gb(&form.memory.read(cx).value()).map_err(EditorIssue::Memory)?;
    let java = form.java.read(cx).value();
    profile.runtime = if form.managed {
        RuntimeSelection::Managed
    } else {
        RuntimeSelection::Local {
            java: PathBuf::from(java.trim()),
        }
    };
    profile.width = form
        .width
        .read(cx)
        .value()
        .trim()
        .parse::<u32>()
        .ok()
        .filter(|value| (320..=16384).contains(value))
        .ok_or(EditorIssue::Width)?;
    profile.height = form
        .height
        .read(cx)
        .value()
        .trim()
        .parse::<u32>()
        .ok()
        .filter(|value| (240..=16384).contains(value))
        .ok_or(EditorIssue::Height)?;
    profile.game_directory = PathBuf::from(form.game_directory.read(cx).value().trim());
    if !profile.game_directory.is_absolute()
        || profile.game_directory.to_string_lossy().contains('\0')
    {
        return Err(EditorIssue::Directory);
    }
    profile.jvm_args.clear();
    profile.game_args.clear();
    profile.validate().map_err(|error| {
        EditorIssue::Reported(
            if form.managed {
                EditorField::Name
            } else {
                EditorField::Java
            },
            error.to_string(),
        )
    })?;
    profile.jvm_args = super::parse_arguments(&form.jvm.read(cx).value())
        .map_err(|_| EditorIssue::Arguments(EditorField::Jvm))?;
    profile
        .validate()
        .map_err(|error| EditorIssue::Reported(EditorField::Jvm, error.to_string()))?;
    profile.game_args = super::parse_arguments(&form.game.read(cx).value())
        .map_err(|_| EditorIssue::Arguments(EditorField::Game))?;
    profile
        .validate()
        .map_err(|error| EditorIssue::Reported(EditorField::Game, error.to_string()))?;
    Ok(profile)
}

#[cfg(test)]
#[path = "instance_editor_tests.rs"]
mod compact_tests;
#[cfg(test)]
#[path = "instance_component_tests.rs"]
mod component_tests;
#[cfg(test)]
#[path = "instance_draft_tests.rs"]
mod draft_tests;

#[cfg(test)]
mod tests {
    use super::{EditorField, EditorIssue, SaveFailure};
    use crate::Language;

    #[test]
    fn retained_profile_messages_resolve_in_the_view_language() {
        let required = EditorIssue::NameRequired;
        assert_eq!(required.field(), EditorField::Name);
        assert_eq!(
            required.message(Language::English),
            "Enter an instance name"
        );
        assert_eq!(
            required.message(Language::SimplifiedChinese),
            "请输入实例名称"
        );
        assert_eq!(
            required.message(Language::Japanese),
            "インスタンス名を入力してください"
        );

        assert_eq!(
            EditorIssue::NameInvalid.message(Language::English),
            "Use an instance name of at most 160 UTF-8 bytes without control characters"
        );

        // A diagnostic keeps its own field and wording in every view language.
        let reported =
            EditorIssue::Reported(EditorField::Jvm, "unterminated quoted argument".into());
        assert_eq!(reported.field(), EditorField::Jvm);
        assert_eq!(
            reported.message(Language::Japanese),
            "unterminated quoted argument"
        );

        let unavailable = SaveFailure::Unavailable;
        assert_eq!(
            unavailable.message(Language::English),
            "Instance settings could not be saved. Check Block’s data directory and restart."
        );
        assert_eq!(
            unavailable.message(Language::SimplifiedChinese),
            "无法保存实例设置。请检查 Block 的数据目录并重新启动。"
        );
        assert_eq!(
            SaveFailure::Reported("write failed".into()).message(Language::Japanese),
            "write failed"
        );
    }
}
