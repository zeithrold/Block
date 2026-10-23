//! Instance-owned settings use a local draft and a single real catalog save.

#![expect(
    clippy::redundant_pub_crate,
    reason = "Instance editing belongs to the parent shell."
)]

use super::scroll_area::ScrollAreaExt as _;
use super::{
    Block, TextInput, UiAction,
    components::{self, ButtonVariant, ControlState, FieldOptions},
    input, note, patterns,
};
use block_launcher::{
    catalog::{Instance, InstanceConfiguration, InstanceSettings, LaunchTarget},
    install::optifine::OptiFineJar,
    recipe::RuntimeSelection,
};
use gpui::{
    AnyElement, AnyWindowHandle, Context, Entity, FocusHandle, Focusable as _,
    InteractiveElement as _, IntoElement, ParentElement as _, ScrollHandle, Styled as _, Window,
    div,
};
use std::{cell::Cell, fmt, path::PathBuf, rc::Rc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Name,
    Java,
    Width,
    Height,
}

#[expect(
    clippy::struct_excessive_bools,
    reason = "Independent runtime, window editor, fullscreen and save state can coexist."
)]
pub(super) struct InstanceEditor {
    original: Instance,
    /// Launch target whose OptiFine selection this draft edits.
    target: LaunchTarget,
    name: Entity<TextInput>,
    java: Entity<TextInput>,
    width: Entity<TextInput>,
    height: Entity<TextInput>,
    managed: bool,
    window_custom: bool,
    fullscreen: bool,
    /// Draft OptiFine selection; `None` means not installed.
    optifine: Option<Box<OptiFineJar>>,
    /// Import that is currently being validated, so late results can be dropped.
    optifine_request: Option<uuid::Uuid>,
    optifine_error: Option<String>,
    saving: bool,
    error: Option<(Field, String)>,
    save_error: Option<String>,
    scroll: ScrollHandle,
    reveal: Rc<Cell<Option<Field>>>,
    return_focus: Option<FocusHandle>,
    window: AnyWindowHandle,
}

impl fmt::Debug for InstanceEditor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InstanceEditor")
            .field("instance", &self.original.id)
            .field("saving", &self.saving)
            .field("managed", &self.managed)
            .field("fullscreen", &self.fullscreen)
            .finish_non_exhaustive()
    }
}

impl InstanceEditor {
    pub(super) fn snapshot(&self, cx: &mut Context<'_, Block>) -> Self {
        let scroll = ScrollHandle::new();
        scroll.set_offset(self.scroll.offset());
        Self {
            original: self.original.clone(),
            target: self.target.clone(),
            name: super::frozen_input(&self.name, cx),
            java: super::frozen_input(&self.java, cx),
            width: super::frozen_input(&self.width, cx),
            height: super::frozen_input(&self.height, cx),
            managed: self.managed,
            window_custom: self.window_custom,
            fullscreen: self.fullscreen,
            optifine: self.optifine.clone(),
            optifine_request: self.optifine_request,
            optifine_error: self.optifine_error.clone(),
            saving: self.saving,
            error: self.error.clone(),
            save_error: self.save_error.clone(),
            scroll,
            reveal: Rc::new(Cell::new(None)),
            return_focus: None,
            window: self.window,
        }
    }

    pub(super) fn identity(&self) -> &str {
        &self.original.id
    }

    const fn field(&self, field: Field) -> &Entity<TextInput> {
        match field {
            Field::Name => &self.name,
            Field::Java => &self.java,
            Field::Width => &self.width,
            Field::Height => &self.height,
        }
    }

    fn read_only(&self, value: bool, cx: &mut Context<'_, Block>) {
        for field in [&self.name, &self.java, &self.width, &self.height] {
            field.update(cx, |field, cx| field.set_read_only(value, cx));
        }
    }

    fn configuration(
        &self,
        cx: &Context<'_, Block>,
    ) -> Result<InstanceConfiguration, (Field, String)> {
        let name = self.name.read(cx).value().trim().to_owned();
        if name.is_empty() || name.len() > 160 || name.chars().any(char::is_control) {
            return Err((
                Field::Name,
                "Enter an instance name using printable text".into(),
            ));
        }
        let width = self
            .width
            .read(cx)
            .value()
            .trim()
            .parse::<u32>()
            .ok()
            .filter(|value| (320..=16384).contains(value))
            .ok_or_else(|| {
                (
                    Field::Width,
                    "Window width must be between 320 and 16384 pixels".into(),
                )
            })?;
        let height = self
            .height
            .read(cx)
            .value()
            .trim()
            .parse::<u32>()
            .ok()
            .filter(|value| (240..=16384).contains(value))
            .ok_or_else(|| {
                (
                    Field::Height,
                    "Window height must be between 240 and 16384 pixels".into(),
                )
            })?;
        let runtime = if self.managed {
            RuntimeSelection::Managed
        } else {
            RuntimeSelection::Local {
                java: PathBuf::from(self.java.read(cx).value().trim()),
            }
        };
        let settings = InstanceSettings {
            runtime,
            width,
            height,
            fullscreen: self.fullscreen,
        };
        settings
            .validate()
            .map_err(|error| (Field::Java, error.to_string()))?;
        Ok(InstanceConfiguration { name, settings })
    }

    fn dirty(&self, cx: &Context<'_, Block>) -> bool {
        let Some(settings) = &self.original.settings else {
            return true;
        };
        let java = match &settings.runtime {
            RuntimeSelection::Managed => String::new(),
            RuntimeSelection::Local { java } => java.display().to_string(),
        };
        self.name.read(cx).value().trim() != self.original.name
            || self.width.read(cx).value().trim() != settings.width.to_string()
            || self.height.read(cx).value().trim() != settings.height.to_string()
            || self.fullscreen != settings.fullscreen
            || self.managed != matches!(settings.runtime, RuntimeSelection::Managed)
            || (!self.managed && self.java.read(cx).value().trim() != java)
            || self.optifine_changed()
    }

    const fn optifine_supported(&self) -> bool {
        matches!(
            self.target.kind,
            block_launcher::catalog::LoaderKind::Vanilla
                | block_launcher::catalog::LoaderKind::Forge
        )
    }

    fn optifine_changed(&self) -> bool {
        self.optifine.as_deref() != self.target.optifine.as_ref()
    }
}

impl Block {
    pub(super) fn open_instance_editor(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.busy || self.form.is_some() {
            return;
        }
        let Some(original) = self.inspected_instance().cloned() else {
            return;
        };
        let target = self
            .inspected_profile()
            .filter(|profile| profile.instance_id == original.id)
            .and_then(|profile| {
                original
                    .targets
                    .iter()
                    .find(|target| target.id == profile.target_id)
            })
            .or_else(|| original.targets.first())
            .cloned();
        let Some(target) = target else {
            return;
        };
        let optifine = target.optifine.clone().map(Box::new);
        let settings = original.settings.clone().unwrap_or_default();
        let java = match &settings.runtime {
            RuntimeSelection::Managed => String::new(),
            RuntimeSelection::Local { java } => java.display().to_string(),
        };
        let name = input(&original.name, "Instance name", cx);
        let return_focus = window.focused(cx);
        name.read(cx).focus_handle(cx).focus(window, cx);
        self.shell.instance_editor = Some(InstanceEditor {
            original,
            target,
            name,
            java: input(&java, "Absolute path to Java executable", cx),
            width: input(&settings.width.to_string(), "1280", cx),
            height: input(&settings.height.to_string(), "720", cx),
            managed: matches!(settings.runtime, RuntimeSelection::Managed),
            window_custom: !super::settings::WINDOW_PRESETS
                .contains(&(settings.width, settings.height)),
            fullscreen: settings.fullscreen,
            optifine,
            optifine_request: None,
            optifine_error: None,
            saving: false,
            error: None,
            save_error: None,
            scroll: ScrollHandle::new(),
            reveal: Rc::new(Cell::new(None)),
            return_focus,
            window: window.window_handle(),
        });
        self.shell.games.detail_visible = true;
        cx.notify();
    }

    pub(super) fn instance_runtime_automatic(&self) -> Option<bool> {
        self.shell
            .instance_editor
            .as_ref()
            .map(|editor| editor.managed)
    }

    pub(super) fn instance_window_values(&self, cx: &gpui::App) -> Option<(String, String, bool)> {
        self.shell.instance_editor.as_ref().map(|editor| {
            (
                editor.width.read(cx).value(),
                editor.height.read(cx).value(),
                editor.window_custom,
            )
        })
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
        if let Some(editor) = &mut self.shell.instance_editor {
            editor.managed = automatic;
            if editor
                .error
                .as_ref()
                .is_some_and(|(field, _)| *field == Field::Java)
            {
                editor.error = None;
            }
            if !automatic {
                editor.java.read(cx).focus_handle(cx).focus(window, cx);
                editor.reveal.set(Some(Field::Java));
            }
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
        if let Some(editor) = &mut self.shell.instance_editor {
            editor
                .width
                .update(cx, |input, cx| input.set_value(&width.to_string(), cx));
            editor
                .height
                .update(cx, |input, cx| input.set_value(&height.to_string(), cx));
            editor.window_custom = false;
            if editor
                .error
                .as_ref()
                .is_some_and(|(field, _)| matches!(field, Field::Width | Field::Height))
            {
                editor.error = None;
            }
        }
    }

    pub(super) fn custom_instance_window(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.busy {
            return;
        }
        if let Some(editor) = &mut self.shell.instance_editor {
            editor.window_custom = true;
            editor.width.read(cx).focus_handle(cx).focus(window, cx);
            editor.reveal.set(Some(Field::Width));
        }
    }

    pub(super) const fn toggle_instance_fullscreen(&mut self) {
        if self.busy {
            return;
        }
        if let Some(editor) = &mut self.shell.instance_editor {
            editor.fullscreen = !editor.fullscreen;
        }
    }

    pub(super) fn browse_instance_optifine(&self, cx: &Context<'_, Self>) {
        if self.busy {
            return;
        }
        let Some(editor) = &self.shell.instance_editor else {
            return;
        };
        if editor.optifine_request.is_some() || !editor.optifine_supported() {
            return;
        }
        let identity = editor.identity().to_owned();
        let target_id = editor.target.id.clone();
        let Some(engine) = self.engine.clone() else {
            return;
        };
        let prompt = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Select the official OptiFine installer JAR".into()),
        });
        cx.spawn(async move |view, cx| {
            let Ok(Ok(Some(paths))) = prompt.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let request = uuid::Uuid::new_v4();
            let started = view
                .update(cx, |view, cx| {
                    let Some(editor) = &mut view.shell.instance_editor else {
                        return false;
                    };
                    if view.busy
                        || editor.identity() != identity
                        || editor.target.id != target_id
                        || editor.optifine_request.is_some()
                        || !editor.optifine_supported()
                    {
                        return false;
                    }
                    editor.optifine_request = Some(request);
                    editor.optifine_error = None;
                    cx.notify();
                    true
                })
                .unwrap_or(false);
            if !started {
                return;
            }
            let result = engine.import_optifine(path).await;
            let _ = view.update(cx, |view, cx| {
                view.finish_instance_optifine_import(request, &identity, &target_id, result, cx);
            });
        })
        .detach();
    }

    fn finish_instance_optifine_import(
        &mut self,
        request: uuid::Uuid,
        identity: &str,
        target_id: &str,
        result: anyhow::Result<OptiFineJar>,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(editor) = &mut self.shell.instance_editor else {
            return;
        };
        if editor.optifine_request != Some(request)
            || editor.identity() != identity
            || editor.target.id != target_id
        {
            return;
        }
        editor.optifine_request = None;
        match result {
            Ok(selected) => match selected.validate_for(
                &editor.original.minecraft_version,
                &editor.target.kind,
                editor.target.loader_version.as_deref(),
            ) {
                Ok(()) => {
                    editor.optifine = Some(Box::new(selected));
                    editor.optifine_error = None;
                }
                Err(error) => editor.optifine_error = Some(error.to_string()),
            },
            Err(error) => {
                editor.optifine_error = Some(format!("Could not import OptiFine: {error:#}"));
            }
        }
        cx.notify();
    }

    pub(super) fn remove_instance_optifine(&mut self, cx: &mut Context<'_, Self>) {
        if self.busy {
            return;
        }
        if let Some(editor) = &mut self.shell.instance_editor {
            if editor.optifine_request.is_some() {
                return;
            }
            editor.optifine = None;
            editor.optifine_error = None;
            cx.notify();
        }
    }

    pub(super) fn cancel_instance_editor(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.busy {
            return;
        }
        if let Some(editor) = self.shell.instance_editor.take() {
            editor
                .return_focus
                .as_ref()
                .unwrap_or(&self.focus)
                .focus(window, cx);
        }
        cx.notify();
    }

    pub(super) fn save_instance_draft(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.busy {
            return;
        }
        let Some(editor) = &self.shell.instance_editor else {
            return;
        };
        let id = editor.original.id.clone();
        let target_id = editor.target.id.clone();
        let optifine_changed = editor.optifine_changed();
        let optifine = editor.optifine.clone();
        match editor.configuration(cx) {
            Ok(configuration) => {
                if optifine_changed {
                    self.begin_cancellable("Saving instance settings · applying OptiFine");
                } else {
                    self.begin("Saving instance settings");
                }
                if let Some(editor) = &mut self.shell.instance_editor {
                    editor.error = None;
                    editor.save_error = (!self.busy).then(|| "Instance settings could not be saved. Check Block’s data directory and restart.".into());
                    editor.saving = self.busy;
                    editor.read_only(self.busy, cx);
                }
                if self.busy
                    && let Some(engine) = &self.engine
                {
                    if optifine_changed {
                        engine.save_instance_with_optifine(
                            id,
                            configuration,
                            target_id,
                            optifine,
                            self.cancel.clone(),
                        );
                    } else {
                        engine.save_instance(id, configuration);
                    }
                }
            }
            Err((field, message)) => {
                if let Some(editor) = &mut self.shell.instance_editor {
                    editor.error = Some((field, message.clone()));
                    editor.save_error = None;
                    if matches!(field, Field::Width | Field::Height) {
                        editor.window_custom = true;
                    }
                    if field == Field::Java {
                        editor.managed = false;
                    }
                    editor.reveal.set(Some(field));
                    editor
                        .field(field)
                        .read(cx)
                        .focus_handle(cx)
                        .focus(window, cx);
                }
                self.report(message);
            }
        }
        cx.notify();
    }

    pub(super) fn instance_edit_finished(
        &mut self,
        result: &Result<String, String>,
        cx: &mut Context<'_, Self>,
    ) {
        if !self
            .shell
            .instance_editor
            .as_ref()
            .is_some_and(|editor| editor.saving)
        {
            return;
        }
        if let Err(error) = result {
            if let Some(editor) = &mut self.shell.instance_editor {
                editor.saving = false;
                editor.save_error = Some(error.clone());
                editor.read_only(false, cx);
            }
        } else if let Some(editor) = self.shell.instance_editor.take() {
            let focus = editor.return_focus.unwrap_or_else(|| self.focus.clone());
            cx.defer(move |cx| {
                let _ = editor
                    .window
                    .update(cx, |_, window, cx| focus.focus(window, cx));
            });
        }
    }

    pub(super) fn render_instance_editor(
        &self,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let Some(editor) = &self.shell.instance_editor else {
            return div().into_any_element();
        };
        let mut body = div().id(gpui::SharedString::from(format!("instance-editor-body-{}", editor.identity()))).debug_selector(|| "instance-editor-body".into())
            .flex_1().min_h_0().scroll_area(&editor.scroll, "Instance settings", self.palette, false).p_6().flex().flex_col().gap_5()
            .child(div().text_2xl().child("Instance configuration"))
            .child(self.instance_field(editor, Field::Name, "Instance name"))
            .child(note("These defaults apply to Profiles that inherit Instance settings. A Profile’s custom Java keeps priority."))
            .child(self.product_select_field("instance-runtime", "Java runtime", if editor.managed { "Automatic Java" } else { "Custom executable" }, super::select::ProductChoice::InstanceRuntime, cx));
        if !editor.managed {
            body = body.child(self.instance_field(editor, Field::Java, "Java executable"));
        }
        let size = format!(
            "{} × {}{}",
            editor.width.read(cx).value(),
            editor.height.read(cx).value(),
            if editor.window_custom {
                " · Custom"
            } else {
                ""
            }
        );
        body = body.child(self.product_select_field(
            "instance-window",
            "Window size",
            &size,
            super::select::ProductChoice::InstanceWindow,
            cx,
        ));
        if editor.window_custom {
            let mut dimensions = div().flex().gap_4();
            if patterns::is_narrow(self.page_viewport(window)) {
                dimensions = dimensions.flex_col();
            }
            body = body.child(
                dimensions
                    .child(
                        self.instance_field(editor, Field::Width, "Window width (px)")
                            .flex_1(),
                    )
                    .child(
                        self.instance_field(editor, Field::Height, "Window height (px)")
                            .flex_1(),
                    ),
            );
        }
        body = body
            .child(components::checkbox("instance-fullscreen", "Start in fullscreen", editor.fullscreen, self.palette,
                ControlState { disabled: self.busy, ..ControlState::default() },
                cx.listener(|view, (), window, cx| view.handle(UiAction::InstanceFullscreen, window, cx))))
            .child(self.instance_optifine_section(editor, cx))
            .child(div().flex().flex_col().gap_2().child("Installation directory").child(note(editor.original.directory.display().to_string()).min_w_0().w_full().truncate())
                .child(note("This location is read-only. Move the installation from Settings → Storage.")))
            .children(editor.original.settings.is_none().then(|| note("Existing Profiles keep their current Java and window choices. Enable inheritance in a Profile to use these Instance defaults.")))
            .children(editor.save_error.clone().map(note));
        let footer = patterns::management_footer(self.page_viewport(window), self.palette)
            .id("instance-editor-footer")
            .debug_selector(|| "instance-editor-footer".into())
            .justify_end()
            .child(self.button(
                "cancel-instance-settings",
                "Cancel",
                UiAction::CancelInstanceSettings,
                cx,
            ))
            .child(components::button(
                "save-instance-settings",
                "Save changes",
                self.palette,
                ButtonVariant::Primary,
                ControlState {
                    disabled: self.busy || editor.optifine_request.is_some() || !editor.dirty(cx),
                    busy: editor.saving,
                    ..ControlState::default()
                },
                cx.listener(|view, (), window, cx| {
                    view.handle(UiAction::SaveInstanceSettings, window, cx);
                }),
            ));
        div()
            .size_full()
            .min_h_0()
            .min_w_0()
            .flex()
            .flex_col()
            .child(body)
            .child(footer)
            .into_any_element()
    }

    fn instance_optifine_section(
        &self,
        editor: &InstanceEditor,
        cx: &Context<'_, Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let mut section = div()
            .id("instance-optifine")
            .debug_selector(|| "instance-optifine".into())
            .flex()
            .flex_col()
            .gap_2()
            .child(div().text_sm().child("OptiFine"))
            .child(note(
                "Choose the official OptiFine installer JAR downloaded from optifine.net. Block validates it and applies it to this game when you save.",
            ));
        if !editor.optifine_supported() {
            return section.child(note(format!(
                "OptiFine cannot be combined directly with {}.",
                editor.target.kind.name()
            )));
        }
        section = section.child(note(format!(
            "Applies to {}.",
            editor.target.display_name()
        )));
        let pending = editor.optifine_request.is_some();
        match &editor.optifine {
            Some(selected) => {
                section = section.child(note(format!("Selected: OptiFine {}", selected.version)));
                if let Some(forge) = &selected.forge {
                    section = section.child(note(format!(
                        "This release declares compatibility with Forge {forge}."
                    )));
                }
            }
            None => {
                section = section.child(note("Not installed."));
            }
        }
        let select_label = if pending {
            "Validating selected JAR…"
        } else if editor.optifine.is_some() {
            "Replace selected JAR"
        } else {
            "Select downloaded JAR"
        };
        let mut actions =
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_2()
                .child(components::button(
                    "instance-optifine-select",
                    select_label,
                    self.palette,
                    ButtonVariant::Outline,
                    ControlState {
                        disabled: self.busy || pending,
                        busy: pending,
                        ..ControlState::default()
                    },
                    cx.listener(|view, (), window, cx| {
                        view.handle(UiAction::InstanceOptiFineFile, window, cx);
                    }),
                ));
        if editor.optifine.is_some() {
            actions = actions.child(components::button(
                "instance-optifine-remove",
                "Remove OptiFine",
                self.palette,
                ButtonVariant::Ghost,
                ControlState {
                    disabled: self.busy || pending,
                    ..ControlState::default()
                },
                cx.listener(|view, (), window, cx| {
                    view.handle(UiAction::InstanceOptiFineRemove, window, cx);
                }),
            ));
        }
        section = section.child(actions);
        if pending {
            section = section.child(note(
                "Checking the JAR and its declared game compatibility…",
            ));
        }
        if let Some(error) = &editor.optifine_error {
            section = section.child(note(error.clone()));
        }
        section
    }

    fn instance_field(
        &self,
        editor: &InstanceEditor,
        field: Field,
        label: &'static str,
    ) -> gpui::Stateful<gpui::Div> {
        let error = editor
            .error
            .as_ref()
            .filter(|(invalid, _)| *invalid == field)
            .map(|(_, message)| message.clone().into());
        patterns::reveal_field(
            components::field_with(
                label,
                editor.field(field),
                self.palette,
                FieldOptions {
                    error,
                    ..FieldOptions::default()
                },
            ),
            editor.scroll.clone(),
            editor.reveal.clone(),
            field,
        )
        .id(gpui::SharedString::from(format!(
            "instance-field-{field:?}"
        )))
        .debug_selector(move || format!("instance-field-{field:?}"))
    }
}

#[cfg(test)]
mod tests {
    use crate::{Block, Tab, UiAction, test_support::ready_snapshot};
    use block_launcher::catalog::Catalog;
    use gpui::{Focusable as _, TestAppContext, VisualTestContext, px, size};
    use std::{
        fs,
        time::{Duration, Instant},
    };

    fn click(cx: &mut VisualTestContext, selector: &'static str) {
        cx.run_until_parked();
        let bounds = crate::test_support::control_bounds(cx, selector);
        assert!(bounds.is_some(), "missing {selector}");
        if let Some(bounds) = bounds {
            let window_count = cx.windows().len();
            cx.simulate_click(bounds.center(), gpui::Modifiers::none());
            cx.run_until_parked();
            if cx.windows().len() > window_count
                && let Some(window) = cx.windows().last()
            {
                *cx = VisualTestContext::from_window(*window, &cx.cx);
            }
            crate::test_support::ensure_live_window(cx);
            cx.run_until_parked();
            crate::test_support::ensure_live_window(cx);
        }
    }

    fn wait_for_save(view: &gpui::Entity<Block>, cx: &mut VisualTestContext) {
        let deadline = Instant::now() + Duration::from_secs(3);
        while view.read_with(cx, |view, _| view.busy) && Instant::now() < deadline {
            view.update(cx, Block::poll);
            std::thread::sleep(Duration::from_millis(5));
        }
        cx.run_until_parked();
        assert!(!view.read_with(cx, |view, _| view.busy));
    }

    #[gpui::test]
    fn cancelling_an_instance_draft_keeps_the_snapshot_and_restores_focus(cx: &mut TestAppContext) {
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.preferences.reduce_motion = true;
                view.tab = Tab::Instances;
                view.snapshot = ready_snapshot();
                view.focus.focus(window, cx);
                view.handle(UiAction::EditInstance, window, cx);
                if let Some(editor) = &mut view.shell.instance_editor {
                    editor
                        .name
                        .update(cx, |field, cx| field.set_value("Uncommitted 世界", cx));
                    editor.fullscreen = true;
                }
            });
        });
        click(cx, "cancel-instance-settings");
        cx.update(|window, cx| {
            view.read_with(cx, |view, _| {
                assert!(view.shell.instance_editor.is_none() && view.focus.is_focused(window));
                assert_eq!(view.snapshot.instances[0].name, "Test game");
                assert!(view.snapshot.instances[0].settings.is_none());
            });
        });
    }

    #[gpui::test]
    fn invalid_instance_dimensions_reveal_and_focus_the_field_in_a_short_window(
        cx: &mut TestAppContext,
    ) {
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        cx.simulate_resize(size(px(360.), px(540.)));
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.preferences.reduce_motion = true;
                view.tab = Tab::Instances;
                view.snapshot = ready_snapshot();
                view.handle(UiAction::EditInstance, window, cx);
                if let Some(editor) = &view.shell.instance_editor {
                    editor
                        .width
                        .update(cx, |field, cx| field.set_value("bad", cx));
                    editor
                        .name
                        .update(cx, |field, cx| field.set_value("Preserved draft", cx));
                }
                view.handle(UiAction::SaveInstanceSettings, window, cx);
            });
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            view.read_with(cx, |view, cx| {
                assert!(view.shell.instance_editor.as_ref().is_some_and(|editor| {
                    editor.width.read(cx).focus_handle(cx).is_focused(window)
                        && editor.name.read(cx).value() == "Preserved draft"
                }));
                assert!(!view.busy);
            });
        });
        let field = cx.debug_bounds("instance-field-Width");
        let body = cx.debug_bounds("instance-editor-body");
        let footer = cx.debug_bounds("instance-editor-footer");
        assert!(field.zip(body).is_some_and(
            |(field, body)| field.top() >= body.top() && field.bottom() <= body.bottom()
        ));
        assert!(
            footer
                .is_some_and(|footer| footer.size.height == px(56.) && footer.bottom() <= px(540.))
        );
    }

    #[gpui::test]
    fn a_real_save_failure_keeps_the_draft_and_retry_commits_complete_settings(
        cx: &mut TestAppContext,
    ) {
        let result = (|| -> anyhow::Result<()> {
            let root = tempfile::tempdir()?;
            let catalog = Catalog::open(root.path())?;
            let (instance, profile) =
                catalog.create_instance("Before".into(), "1.20.6".into(), None, None)?;
            let state = catalog.snapshot()?;
            let path = root.path().join("catalog.json");
            let bytes = fs::read(&path)?;
            let (engine, messages) = crate::engine::Engine::with_root(root.path().to_owned())?;
            let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
            cx.update(|window, cx| {
                view.update(cx, |view, cx| {
                    view.preferences.reduce_motion = true;
                    view.tab = Tab::Instances;
                    view.snapshot = state;
                    view.engine = Some(engine);
                    view.messages = Some(messages);
                    view.selected_instance = Some(instance.id.clone());
                    view.selected_profile = Some(profile.id.clone());
                    view.handle(UiAction::EditInstance, window, cx);
                    if let Some(editor) = &mut view.shell.instance_editor {
                        editor
                            .name
                            .update(cx, |field, cx| field.set_value("Saved instance", cx));
                        editor
                            .width
                            .update(cx, |field, cx| field.set_value("1536", cx));
                        editor.fullscreen = true;
                    }
                });
            });
            assert_eq!(fs::read(&path)?, bytes);
            fs::remove_file(&path)?;
            fs::create_dir(&path)?;
            click(cx, "save-instance-settings");
            wait_for_save(&view, cx);
            assert!(view.read_with(cx, |view, cx| {
                view.shell.instance_editor.as_ref().is_some_and(|editor| {
                    editor.save_error.is_some()
                        && editor.name.read(cx).value() == "Saved instance"
                        && editor.width.read(cx).value() == "1536"
                })
            }));
            fs::remove_dir(&path)?;
            fs::write(&path, &bytes)?;
            click(cx, "save-instance-settings");
            wait_for_save(&view, cx);
            assert!(view.read_with(cx, |view, _| view.shell.instance_editor.is_none()));
            let state = catalog.snapshot()?;
            let (saved, saved_profile, _) = state.selection(&profile.id)?;
            assert_eq!(saved.name, "Saved instance");
            assert_eq!(saved_profile.name, "Saved instance");
            let settings = saved
                .settings
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("settings missing"))?;
            assert_eq!(
                (settings.width, settings.height, settings.fullscreen),
                (1536, 720, true)
            );
            assert_eq!(saved.directory, instance.directory);
            Ok(())
        })();
        assert!(result.is_ok(), "{result:?}");
    }

    fn optifine_jar(minecraft: &str) -> block_launcher::install::optifine::OptiFineJar {
        block_launcher::install::optifine::OptiFineJar {
            minecraft: minecraft.into(),
            version: format!("{minecraft}_HD_U_I6"),
            launchwrapper: "2.3".into(),
            forge: None,
            resource: block_launcher::Resource {
                sha1: "a".repeat(40),
                size: 123,
                url: "block-import:synthetic".into(),
            },
        }
    }

    #[gpui::test]
    fn instance_optifine_draft_validates_removes_and_cancel_restores_the_catalog(
        cx: &mut TestAppContext,
    ) {
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.preferences.reduce_motion = true;
                view.tab = Tab::Instances;
                view.snapshot = ready_snapshot();
                view.handle(UiAction::EditInstance, window, cx);
            });
        });
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                let Some(editor) = view.shell.instance_editor.as_mut() else {
                    return;
                };
                assert_eq!(editor.target.id, "vanilla");
                let request = uuid::Uuid::new_v4();
                editor.optifine_request = Some(request);
                view.finish_instance_optifine_import(
                    request,
                    "instance-a",
                    "vanilla",
                    Ok(optifine_jar("1.20.6")),
                    cx,
                );
                let Some(editor) = view.shell.instance_editor.as_ref() else {
                    return;
                };
                assert!(
                    editor
                        .optifine
                        .as_ref()
                        .is_some_and(|selected| selected.version == "1.20.6_HD_U_I6")
                );
                assert!(editor.optifine_error.is_none());
                assert!(editor.optifine_changed());
                assert!(editor.dirty(cx));

                let request = uuid::Uuid::new_v4();
                let Some(editor) = view.shell.instance_editor.as_mut() else {
                    return;
                };
                editor.optifine_request = Some(request);
                view.finish_instance_optifine_import(
                    request,
                    "instance-a",
                    "vanilla",
                    Ok(optifine_jar("1.21.1")),
                    cx,
                );
                let Some(editor) = view.shell.instance_editor.as_ref() else {
                    return;
                };
                assert!(
                    editor
                        .optifine_error
                        .as_deref()
                        .is_some_and(|error| error.contains("1.21.1")),
                    "{:?}",
                    editor.optifine_error
                );
                assert_eq!(
                    editor
                        .optifine
                        .as_ref()
                        .map(|selected| selected.version.as_str()),
                    Some("1.20.6_HD_U_I6"),
                    "a rejected replacement keeps the previous draft"
                );

                view.handle(UiAction::InstanceOptiFineRemove, window, cx);
                let Some(editor) = view.shell.instance_editor.as_ref() else {
                    return;
                };
                assert!(editor.optifine.is_none());
                assert!(editor.optifine_error.is_none());
                assert!(
                    !editor.optifine_changed(),
                    "remove restores the original draft value"
                );

                let request = uuid::Uuid::new_v4();
                let Some(editor) = view.shell.instance_editor.as_mut() else {
                    return;
                };
                editor.optifine_request = Some(request);
                view.finish_instance_optifine_import(
                    request,
                    "instance-a",
                    "vanilla",
                    Ok(optifine_jar("1.20.6")),
                    cx,
                );
            });
        });
        click(cx, "cancel-instance-settings");
        cx.update(|window, cx| {
            view.read_with(cx, |view, _| {
                assert!(view.shell.instance_editor.is_none() && view.focus.is_focused(window));
                let target = &view.snapshot.instances[0].targets[0];
                assert!(
                    target.optifine.is_none(),
                    "Cancel never writes OptiFine to the catalog"
                );
                assert!(target.optifine_mod.is_none());
            });
        });
    }
}
