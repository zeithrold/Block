//! A Profile owns a draft. Detail subpages can commit to, or roll back to, that
//! draft without changing the catalog until the final Save action.
#![expect(
    clippy::redundant_pub_crate,
    reason = "Editor state is private to the product UI."
)]
use super::scroll_area::ScrollAreaExt as _;
use super::{
    Block, Form, Language, ProfileForm, TextInput, UiAction, account_id,
    components::{self, ControlState, FieldOptions},
    note, patterns,
};
use block_launcher::{
    catalog::{DataPolicy, Profile},
    recipe::RuntimeSelection,
};
use gpui::{
    AnyWindowHandle, Context, Entity, FocusHandle, Focusable as _, InteractiveElement as _,
    IntoElement, ParentElement as _, ScrollHandle, Styled as _, Window, div,
};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    fmt,
    path::PathBuf,
    rc::Rc,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
}

/// A rejected draft value, kept with the draft until the user fixes the field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum EditorIssue {
    NameRequired,
    NameInvalid,
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
            Self::Arguments(field) | Self::Reported(field, _) => *field,
        }
    }

    /// Resolve the message in the language of the view that paints it.
    fn message(&self, language: Language) -> String {
        self.text().text(language)
    }

    fn text(&self) -> crate::i18n::UiText {
        match self {
            Self::NameRequired => msg!("games.profile_editor.error.name"),
            Self::NameInvalid => msg!("games.profile_editor.error.name_length"),
            Self::Memory(error) => msg!(error.key()),
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
            Self::Unavailable => tr!(language, "games.profile_editor.save_failed"),
            Self::Reported(detail) => detail.clone(),
        }
    }
}

#[derive(Debug)]
struct Checkpoint {
    original: Profile,
    values: [String; 6],
    managed: bool,
}
#[derive(Default)]
pub(super) struct EditorState {
    pub page: Option<EditorPage>,
    radio_focus: RefCell<HashMap<String, FocusHandle>>,
    checkpoint: Option<Checkpoint>,
    error: Option<EditorIssue>,
    scroll: ScrollHandle,
    reveal: Rc<Cell<Option<EditorField>>>,
    return_focus: Option<FocusHandle>,
    page_focus: Option<FocusHandle>,
    outer_focus: Option<FocusHandle>,
    outer_window: Option<AnyWindowHandle>,
    outer_tab: Option<super::Tab>,
    outer_profile: Option<String>,
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
            error: self.error.clone(),
            save_error: self.save_error.clone(),
            scroll,
            reveal: Rc::new(Cell::new(self.reveal.get())),
            ..Default::default()
        }
    }
}

impl Block {
    pub(super) fn begin_profile_editor(
        &self,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> EditorState {
        EditorState {
            outer_focus: Some(self.profile_editor_trigger_focus(cx)),
            outer_window: Some(window.window_handle()),
            outer_tab: Some(self.router.tab()),
            outer_profile: self.inspected_profile().map(|profile| profile.id.clone()),
            ..EditorState::default()
        }
    }

    fn profile_editor_return_focus(
        &self,
        state: &EditorState,
        cx: &Context<'_, Self>,
    ) -> Option<FocusHandle> {
        // A user who navigated to another domain owns that domain's current focus.
        if state.outer_tab != Some(self.router.tab()) {
            return None;
        }
        let original_is_current = state.outer_profile.as_ref().is_some_and(|id| {
            self.snapshot
                .profiles
                .iter()
                .any(|profile| &profile.id == id)
                && self
                    .inspected_profile()
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

    pub(super) fn cancel_profile_editor(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.busy || !matches!(self.form, Some(Form::Profile(_))) {
            return;
        }
        let editor = std::mem::take(&mut self.shell.editor);
        self.form = None;
        if let Some(focus) = self.profile_editor_return_focus(&editor, cx) {
            focus.focus(window, cx);
        }
        cx.notify();
    }

    pub(super) fn profile_edit_finished(
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
            if matches!(&self.form, Some(Form::Profile(form)) if Some(&form.original.id) == editor.outer_profile.as_ref())
            {
                self.form = None;
            }
            let focus = self.profile_editor_return_focus(&editor, cx);
            if let (Some(window), Some(focus)) = (editor.outer_window, focus) {
                cx.defer(move |cx| {
                    let _ = window.update(cx, |_, window, cx| focus.focus(window, cx));
                });
            }
        }
    }

    pub(super) fn open_profile_page(
        &mut self,
        page: EditorPage,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(Form::Profile(form)) = &self.form else {
            return;
        };
        self.shell.editor.checkpoint = Some(Checkpoint {
            original: form.original.clone(),
            managed: form.managed,
            values: [
                form.name.read(cx).value(),
                form.memory.read(cx).value(),
                form.java.read(cx).value(),
                form.jvm.read(cx).value(),
                form.game.read(cx).value(),
                form.server.read(cx).value(),
            ],
        });
        self.shell.editor.page = Some(page);
        self.shell.editor.error = None;
        self.shell
            .editor
            .scroll
            .set_offset(gpui::point(gpui::px(0.), gpui::px(0.)));
        self.shell.editor.return_focus = window.focused(cx);
        let focus = cx.focus_handle().tab_stop(true);
        if page == EditorPage::Launch {
            form.memory.read(cx).focus_handle(cx).focus(window, cx);
        } else {
            focus.focus(window, cx);
        }
        self.shell.editor.page_focus = Some(focus);
    }

    pub(super) fn finish_profile_page(
        &mut self,
        cancel: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if let Some(checkpoint) = self.shell.editor.checkpoint.take()
            && cancel
            && let Some(Form::Profile(form)) = &mut self.form
        {
            form.original = checkpoint.original;
            form.managed = checkpoint.managed;
            for (field, value) in [
                &form.name,
                &form.memory,
                &form.java,
                &form.jvm,
                &form.game,
                &form.server,
            ]
            .into_iter()
            .zip(checkpoint.values)
            {
                field.update(cx, |field, cx| field.set_value(&value, cx));
            }
        }
        self.shell.editor.page = None;
        self.shell.editor.error = None;
        if let Some(focus) = self.shell.editor.return_focus.take() {
            focus.focus(window, cx);
        }
        self.shell.editor.page_focus = None;
    }

    pub(super) fn set_profile_runtime_automatic(
        &mut self,
        automatic: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.busy {
            return;
        }
        if let Some(Form::Profile(form)) = &mut self.form {
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

    pub(super) fn set_profile_data_policy(&mut self, policy: DataPolicy) {
        if self.busy {
            return;
        }
        if let Some(Form::Profile(form)) = &mut self.form {
            form.original.data_policy = policy;
        }
    }

    pub(super) fn render_profile_editor(&self, cx: &Context<'_, Self>) -> gpui::AnyElement {
        let Some(Form::Profile(form)) = &self.form else {
            return div().into_any_element();
        };
        let language = self.language();
        let page = self.shell.editor.page;
        let body = div()
            .id("profile-editor-body")
            .flex_1()
            .min_h_0()
            .scroll_area(
                &self.shell.editor.scroll,
                "games.profile_editor",
                self.palette,
                false,
            )
            .p_6()
            .flex()
            .flex_col()
            .gap_5();
        let mut body = match page {
            None => self.profile_overview_fields(body, form, cx),
            Some(EditorPage::Appearance) => self.profile_appearance_fields(body, form, cx),
            Some(EditorPage::Launch) => self.profile_launch_fields(body, form, cx),
            Some(EditorPage::Data) => self.profile_data_fields(body, form, cx),
        };
        if let Some(issue) = &self.shell.editor.error {
            body = body.child(note(issue.message(language)));
        }
        let footer = patterns::management_footer(self.palette)
            .justify_end()
            .child(self.button(
                if page.is_some() {
                    "cancel-profile-page"
                } else {
                    "cancel-profile"
                },
                tr!(language, "common.cancel"),
                if page.is_some() {
                    UiAction::ProfilePageCancel
                } else {
                    UiAction::CancelForm
                },
                cx,
            ))
            .child(self.button(
                if page.is_some() {
                    "done-profile-page"
                } else {
                    "save-profile"
                },
                if page.is_some() {
                    tr!(language, "games.action.done")
                } else {
                    tr!(language, "games.action.save_changes")
                },
                if page.is_some() {
                    UiAction::ProfilePageDone
                } else {
                    UiAction::SaveProfile
                },
                cx,
            ));
        div()
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .child(body)
            .children(self.profile_save_error())
            .child(footer)
            .into_any_element()
    }

    fn profile_save_error(&self) -> Option<gpui::Stateful<gpui::Div>> {
        let language = self.language();
        self.shell.editor.save_error.as_ref().map(|failure| {
            div()
                .id("profile-save-error")
                .debug_selector(|| "profile-save-error".into())
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

    fn profile_account_group(
        &self,
        form: &ProfileForm,
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
            "profile-default-account-group",
            tr!(language, "games.profile_editor.default_account_group"),
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
                    tr!(self.language(), "games.profile_editor.use_current_account"),
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

    fn profile_launch_fields(
        &self,
        body: super::scroll_area::ScrollArea<gpui::Stateful<gpui::Div>>,
        form: &ProfileForm,
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
                    "profile-runtime",
                    tr!(language, "select.java_runtime"),
                    &runtime,
                    super::select::ProductChoice::ProfileRuntime,
                    cx,
                )
            });
        if !form.managed {
            fields = fields.child(self.profile_field(
                EditorField::Java,
                tr!(language, "games.java.executable"),
                &form.java,
            ));
        }
        fields = fields.child(self.profile_inheritance_fields(form, cx));
        fields = fields
            .child(self.profile_field(
                EditorField::Memory,
                tr!(language, "games.profile_editor.memory"),
                &form.memory,
            ))
            .child(self.profile_field(
                EditorField::Jvm,
                tr!(language, "games.profile_editor.jvm"),
                &form.jvm,
            ))
            .child(self.profile_field(
                EditorField::Game,
                tr!(language, "games.profile_editor.game"),
                &form.game,
            ))
            .child(note(tr!(language, "games.profile_editor.arguments_note")));
        if let Some(instance) = self
            .snapshot
            .instances
            .iter()
            .find(|instance| instance.id == form.original.instance_id)
        {
            let group = components::RadioGroup::new(
                "profile-target-group",
                tr!(language, "games.profile_editor.launch_target"),
                instance
                    .targets
                    .iter()
                    .map(|target| components::RadioOption {
                        id: target.id.clone(),
                        checked: target.id == form.original.target_id,
                        disabled: self.busy,
                        action: UiAction::ProfileTarget(target.id.clone()),
                    })
                    .collect(),
                &self.shell.editor.radio_focus,
                cx,
                cx.listener(|view, action: &UiAction, window, cx| {
                    view.handle(action.clone(), window, cx);
                }),
            );
            fields = fields
                .child(div().child(tr!(language, "games.profile_editor.launch_target")))
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
                                    view.handle(UiAction::ProfileTarget(id.clone()), window, cx);
                                }),
                            ),
                        )
                    }),
                ));
        }
        fields
    }

    /// The root page: naming, server and the entries into the detail pages.
    fn profile_overview_fields(
        &self,
        body: super::scroll_area::ScrollArea<gpui::Stateful<gpui::Div>>,
        form: &ProfileForm,
        cx: &Context<'_, Self>,
    ) -> super::scroll_area::ScrollArea<gpui::Stateful<gpui::Div>> {
        let language = self.language();
        body.child(patterns::page_title(tr!(language, "games.launch.settings")))
            .child(self.profile_field(
                EditorField::Name,
                tr!(language, "games.profile_editor.name"),
                &form.name,
            ))
            .child(self.profile_field(
                EditorField::Server,
                tr!(language, "games.profile_editor.server"),
                &form.server,
            ))
            .child(self.profile_server_choice(form, cx))
            .child(
                self.button(
                    "edit-profile-appearance",
                    tr!(language, "games.profile_editor.appearance"),
                    UiAction::ProfilePage(EditorPage::Appearance),
                    cx,
                )
                .child(components::profile_avatar(
                    form.original.appearance,
                    self.palette,
                    32.,
                )),
            )
            .child(self.button(
                "edit-profile-launch",
                tr!(language, "games.launch.settings"),
                UiAction::ProfilePage(EditorPage::Launch),
                cx,
            ))
            .child(self.button(
                "edit-profile-data",
                tr!(language, "games.profile_editor.data_and_account"),
                UiAction::ProfilePage(EditorPage::Data),
                cx,
            ))
            .child(note(tr!(language, "games.profile_editor.draft_note")))
    }

    /// The appearance page paints the draft avatar above both choice entries.
    fn profile_appearance_fields(
        &self,
        body: super::scroll_area::ScrollArea<gpui::Stateful<gpui::Div>>,
        form: &ProfileForm,
        cx: &Context<'_, Self>,
    ) -> super::scroll_area::ScrollArea<gpui::Stateful<gpui::Div>> {
        let language = self.language();
        body.child(patterns::page_title(tr!(
            language,
            "games.profile_editor.appearance_title"
        )))
        .child(components::profile_avatar(
            form.original.appearance,
            self.palette,
            80.,
        ))
        .child(self.profile_page_trigger(self.button(
            "profile-icon",
            crate::i18n::profile_icon_name(language, form.original.appearance.icon),
            UiAction::ChooseAppearance {
                wizard: false,
                color: false,
            },
            cx,
        )))
        .child(self.button(
            "profile-color",
            crate::i18n::profile_color_name(language, form.original.appearance.color),
            UiAction::ChooseAppearance {
                wizard: false,
                color: true,
            },
            cx,
        ))
    }

    /// The data page chooses the draft policy and default launch account.
    fn profile_data_fields(
        &self,
        body: super::scroll_area::ScrollArea<gpui::Stateful<gpui::Div>>,
        form: &ProfileForm,
        cx: &Context<'_, Self>,
    ) -> super::scroll_area::ScrollArea<gpui::Stateful<gpui::Div>> {
        let language = self.language();
        body.child(patterns::page_title(tr!(
            language,
            "games.profile_editor.data_and_account"
        )))
        .child({
            let policy = match form.original.data_policy {
                DataPolicy::Isolated => tr!(language, "games.profile_editor.data_isolated"),
                DataPolicy::Shared => tr!(language, "games.profile_editor.data_shared"),
            };
            self.product_select_field(
                "data-policy",
                tr!(language, "select.game_data"),
                &policy,
                super::select::ProductChoice::ProfileDataPolicy,
                cx,
            )
        })
        .child(note(tr!(language, "games.profile_editor.data_note")))
        .child(self.profile_account_group(form, cx))
    }

    fn profile_inheritance_fields(&self, form: &ProfileForm, cx: &Context<'_, Self>) -> gpui::Div {
        let mut fields = div().flex().flex_col().gap_5();
        let settings = self
            .snapshot
            .instances
            .iter()
            .find(|instance| instance.id == form.original.instance_id)
            .and_then(|instance| instance.settings.as_ref());
        let language = self.language();
        fields = fields
            .child(components::checkbox(
                "profile-inherit-runtime",
                tr!(language, "games.profile_editor.inherit_runtime"),
                form.original.inheritance.runtime,
                self.palette,
                ControlState {
                    disabled: self.busy || !form.managed || settings.is_none(),
                    ..Default::default()
                },
                cx.listener(|view, (): &(), window, cx| {
                    view.handle(UiAction::ProfileInheritRuntime, window, cx);
                }),
            ))
            .child(components::checkbox(
                "profile-inherit-window",
                tr!(language, "games.profile_editor.inherit_window"),
                form.original.inheritance.window,
                self.palette,
                ControlState {
                    disabled: self.busy || settings.is_none(),
                    ..Default::default()
                },
                cx.listener(|view, (): &(), window, cx| {
                    view.handle(UiAction::ProfileInheritWindow, window, cx);
                }),
            ));
        if !form.managed {
            fields = fields.child(note(tr!(language, "games.profile_editor.custom_java_note")));
        }
        if let Some(settings) = settings {
            fields = fields.child(note(format!(
                "{}{}",
                tr!(
                    language,
                    "games.profile_editor.instance_window",
                    width = settings.width,
                    height = settings.height
                ),
                if settings.fullscreen {
                    tr!(language, "games.profile_editor.fullscreen_suffix")
                } else {
                    String::new()
                }
            )));
        } else {
            fields = fields.child(note(tr!(
                language,
                "games.profile_editor.inheritance_unavailable"
            )));
        }
        if form.original.inheritance.window {
            fields = fields.child(note(tr!(
                language,
                "games.profile_editor.window_inheritance_note"
            )));
        }
        fields
    }

    fn profile_server_choice(&self, form: &ProfileForm, cx: &Context<'_, Self>) -> gpui::Div {
        let language = self.language();
        let version = self
            .snapshot
            .instances
            .iter()
            .find(|instance| instance.id == form.original.instance_id)
            .map(|instance| instance.minecraft_version.as_str());
        let connection = version.and_then(|version| self.automatic_connection(version));
        let supported = connection.is_some_and(|connection| {
            connection != block_launcher::catalog::AutomaticConnection::Unsupported
        });
        let address = form.server.read(cx).value();
        let mut choice = div().flex().flex_col().gap_2();
        if !address.trim().is_empty() {
            choice = choice.child(components::checkbox(
                "profile-connect",
                tr!(language, "games.profile_editor.connect"),
                form.original.server.connect_on_launch,
                self.palette,
                ControlState {
                    disabled: self.busy || !supported && !form.original.server.connect_on_launch,
                    ..Default::default()
                },
                cx.listener(|view, (): &(), window, cx| {
                    view.handle(UiAction::ProfileConnect, window, cx);
                }),
            ));
        }
        if !supported {
            choice = choice.child(note(match connection {
                Some(block_launcher::catalog::AutomaticConnection::Unsupported) => {
                    tr!(language, "games.profile_editor.server_unsupported")
                }
                _ => tr!(language, "games.profile_editor.server_unknown"),
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
                if let Some(path) = &target.manifest
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

    fn profile_page_trigger(&self, button: components::Control) -> components::Control {
        if let Some(focus) = &self.shell.editor.page_focus {
            button.track_focus(focus)
        } else {
            button
        }
    }

    fn profile_field(
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
        .id(gpui::SharedString::from(format!("profile-field-{field:?}")))
        .debug_selector(move || format!("profile-field-{field:?}"))
    }

    pub(super) fn save_profile_draft(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.busy {
            return;
        }
        let Some(Form::Profile(form)) = &self.form else {
            return;
        };
        match parse_profile(form, cx) {
            Ok(profile) => {
                self.shell.editor.error = None;
                self.begin(
                    msg!("games.profile_editor.saving"),
                    super::ui_logging::Operation::SaveProfile,
                );
                self.shell.editor.saving = self.busy;
                self.shell.editor.save_error = (!self.busy).then_some(SaveFailure::Unavailable);
                if let Some(engine) = &self.engine {
                    engine.save_profile(profile);
                }
            }
            Err(issue) => {
                let field = issue.field();
                self.shell.editor.page = if matches!(field, EditorField::Name | EditorField::Server)
                {
                    None
                } else {
                    Some(EditorPage::Launch)
                };
                self.shell.editor.error = Some(issue.clone());
                self.shell.editor.reveal.set(Some(field));
                let input = match field {
                    EditorField::Name => &form.name,
                    EditorField::Memory => &form.memory,
                    EditorField::Java => &form.java,
                    EditorField::Jvm => &form.jvm,
                    EditorField::Game => &form.game,
                    EditorField::Server => &form.server,
                };
                input.read(cx).focus_handle(cx).focus(window, cx);
                self.report(issue.text());
            }
        }
    }
}

pub(super) fn parse_profile(
    form: &ProfileForm,
    cx: &Context<'_, Block>,
) -> Result<Profile, EditorIssue> {
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
mod tests {
    use super::{EditorField, EditorIssue, SaveFailure};
    use crate::Language;

    #[test]
    fn retained_profile_messages_resolve_in_the_view_language() {
        let required = EditorIssue::NameRequired;
        assert_eq!(required.field(), EditorField::Name);
        assert_eq!(required.message(Language::English), "Enter a profile name");
        assert_eq!(
            required.message(Language::SimplifiedChinese),
            "请输入配置名称"
        );
        assert_eq!(
            required.message(Language::Japanese),
            "プロファイル名を入力してください"
        );

        assert_eq!(
            EditorIssue::NameInvalid.message(Language::English),
            "Use a profile name of at most 160 UTF-8 bytes without control characters"
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
            "Profile settings could not be saved. Check Block’s data directory and restart."
        );
        assert_eq!(
            unavailable.message(Language::SimplifiedChinese),
            "无法保存配置设置。请检查 Block 的数据目录并重新启动。"
        );
        assert_eq!(
            SaveFailure::Reported("write failed".into()).message(Language::Japanese),
            "write failed"
        );
    }
}
