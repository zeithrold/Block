//! A form field binds its draft to the Kit editor in the window that presents it.
use gpui::{
    App, AppContext as _, Context, Entity, EntityInputHandler as _, FocusHandle, Focusable,
    IntoElement, Render, SharedString, Window,
};
use gpui_base::input::{InputEvent, InputState};

#[path = "text_input_view.rs"]
mod view;
pub(super) use view::LabeledEditor;

#[cfg(test)]
#[path = "text_input_tests.rs"]
mod tests;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct FieldSemantics {
    pub label: SharedString,
    pub description: Option<SharedString>,
    pub required: bool,
    pub invalid: bool,
}

#[derive(Debug)]
enum Content {
    Draft(SharedString),
    Mounted {
        editor: Entity<InputState>,
        _ime_keys: gpui::Subscription,
        // Async model callbacks have no window. Consume their explicit reset
        // before rendering or delivering another edit to this editor.
        reset: Option<SharedString>,
    },
    Frozen(SharedString),
}

#[derive(Debug)]
pub(super) struct TextInput {
    content: Content,
    focus: FocusHandle,
    placeholder: SharedString,
    field_semantics: Option<FieldSemantics>,
    read_only: bool,
    search: bool,
    composing: bool,
}

impl TextInput {
    pub(super) fn new(value: &str, placeholder: &str, cx: &Context<'_, Self>) -> Self {
        Self {
            content: Content::Draft(value.to_owned().into()),
            focus: cx.focus_handle().tab_stop(true),
            placeholder: placeholder.to_owned().into(),
            field_semantics: None,
            read_only: false,
            search: false,
            composing: false,
        }
    }

    fn text(&self, cx: &App) -> SharedString {
        match &self.content {
            Content::Draft(value) | Content::Frozen(value) => value.clone(),
            Content::Mounted { editor, reset, .. } => {
                reset.clone().unwrap_or_else(|| editor.read(cx).value())
            }
        }
    }

    pub(super) fn value(&self, cx: &App) -> String {
        self.text(cx).to_string()
    }

    pub(super) fn set_value(&mut self, value: &str, cx: &mut Context<'_, Self>) {
        let value = value.to_owned().into();
        match &mut self.content {
            Content::Draft(text) => *text = value,
            Content::Mounted { reset, .. } => *reset = Some(value),
            Content::Frozen(_) => return,
        }
        self.composing = false;
        cx.notify();
    }

    pub(super) fn frozen(&self, cx: &Context<'_, Self>) -> Self {
        Self {
            content: Content::Frozen(self.text(cx)),
            focus: cx.focus_handle(),
            placeholder: self.placeholder.clone(),
            field_semantics: self.field_semantics.clone(),
            read_only: self.read_only,
            search: self.search,
            composing: false,
        }
    }

    pub(super) fn set_placeholder(&mut self, placeholder: &str, cx: &mut Context<'_, Self>) {
        if self.placeholder != placeholder {
            self.placeholder = placeholder.to_owned().into();
            cx.notify();
        }
    }

    pub(super) fn set_read_only(&mut self, read_only: bool, cx: &mut Context<'_, Self>) {
        if self.read_only != read_only {
            self.read_only = read_only;
            if let Content::Mounted { editor, .. } = &self.content {
                editor.update(cx, |state, cx| state.set_readonly(read_only, cx));
            }
            cx.notify();
        }
    }

    pub(super) fn set_search(&mut self, search: bool, cx: &mut Context<'_, Self>) {
        if self.search != search {
            self.search = search;
            cx.notify();
        }
    }

    pub(super) fn set_field_semantics(
        &mut self,
        semantics: FieldSemantics,
        cx: &mut Context<'_, Self>,
    ) {
        if self.field_semantics.as_ref() != Some(&semantics) {
            self.field_semantics = Some(semantics);
            cx.notify();
        }
    }

    pub(super) fn has_marked_text(field: &Entity<Self>, window: &mut Window, cx: &mut App) -> bool {
        let editor = match &field.read(cx).content {
            Content::Mounted {
                editor,
                reset: None,
                ..
            } => editor.clone(),
            Content::Draft(_) | Content::Frozen(_) | Content::Mounted { reset: Some(_), .. } => {
                return false;
            }
        };
        editor.update(cx, |state, cx| {
            state.marked_text_range(window, cx).is_some()
        })
    }

    pub(super) const fn is_composing(&self) -> bool {
        self.composing
    }

    pub(super) fn editor(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Option<Entity<InputState>> {
        match &mut self.content {
            Content::Frozen(_) => None,
            Content::Mounted { editor, reset, .. } => {
                if let Some(value) = reset.take() {
                    editor.update(cx, |state, cx| state.set_value(value, window, cx));
                }
                Some(editor.clone())
            }
            Content::Draft(value) => {
                let editor = cx.new(|cx| {
                    let mut state =
                        InputState::new(window, cx).placeholder(self.placeholder.clone());
                    state.set_value(value.clone(), window, cx);
                    state.set_readonly(self.read_only, cx);
                    state
                });
                let focus = editor.focus_handle(cx);
                if self.focus.is_focused(window) {
                    focus.focus(window, cx);
                }
                self.focus = focus.clone();
                cx.subscribe(&editor, |field, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        field.composing = false;
                        cx.notify();
                    }
                })
                .detach();
                cx.observe_in(&editor, window, |field, state, window, cx| {
                    let marked = state.update(cx, |state, cx| {
                        state.marked_text_range(window, cx).is_some()
                    });
                    if marked || field.composing {
                        field.composing = marked;
                        cx.notify();
                    }
                })
                .detach();
                let weak = editor.downgrade();
                let ime_keys = cx.intercept_keystrokes(move |event, window, cx| {
                    if focus.is_focused(window)
                        && matches!(
                            event.keystroke.key.as_str(),
                            "enter" | "escape" | "up" | "down"
                        )
                        && weak.upgrade().is_some_and(|editor| {
                            editor.update(cx, |state, cx| {
                                state.marked_text_range(window, cx).is_some()
                            })
                        })
                    {
                        cx.stop_propagation();
                    }
                });
                self.content = Content::Mounted {
                    editor: editor.clone(),
                    reset: None,
                    _ime_keys: ime_keys,
                };
                Some(editor)
            }
        }
    }
}

impl Focusable for TextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for TextInput {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        self.render_field(window, cx)
    }
}
