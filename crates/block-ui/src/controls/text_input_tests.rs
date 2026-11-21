use gpui::{
    AppContext as _, Context, Element as _, Entity, Focusable as _, IntoElement, Render,
    TestAppContext, Window, accesskit,
};

use super::TextInput;
use crate::{components, test_support};

impl TextInput {
    pub(crate) fn edit_fixture(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
        edit: impl FnOnce(
            &mut gpui_base::input::InputState,
            &mut Window,
            &mut Context<'_, gpui_base::input::InputState>,
        ),
    ) {
        let editor = self.editor(window, cx);
        assert!(
            editor.is_some(),
            "a live form fixture must have a Kit editor"
        );
        if let Some(editor) = editor {
            editor.update(cx, |state, cx| edit(state, window, cx));
        }
    }
}

struct Field {
    input: Entity<TextInput>,
    validated: bool,
}

impl Render for Field {
    fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
        if self.validated {
            components::field_with(
                "Launch name",
                &self.input,
                crate::Palette::LIGHT,
                components::FieldOptions {
                    hint: Some("Shown in the library.".into()),
                    error: Some("Enter a name.".into()),
                    required: true,
                },
            )
        } else {
            components::field("Plain name", &self.input)
        }
    }
}

#[gpui::test]
fn labeled_search_field_updates_native_semantics_without_replacing_its_editor(
    cx: &mut TestAppContext,
) {
    let (view, cx) = test_support::window(cx, |_, cx| Field {
        input: cx.new(|cx| TextInput::new("trail", "Find a trail", cx)),
        validated: true,
    });
    let input = view.read_with(cx, |view, _| view.input.clone());
    for validated in [true, false] {
        view.update(cx, |view, cx| {
            view.validated = validated;
            view.input.update(cx, |field, cx| {
                field.set_read_only(!validated, cx);
                field.set_search(!validated, cx);
            });
            cx.notify();
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            input.update(cx, |field, cx| {
                let semantics = field.field_semantics.as_ref();
                let label = if validated {
                    "Launch name"
                } else {
                    "Plain name"
                };
                assert_eq!(
                    semantics.and_then(|s| s.description.as_deref()),
                    validated.then_some("Shown in the library. Enter a name.")
                );
                assert_eq!(semantics.map(|s| s.invalid), Some(validated));
                assert_eq!(semantics.map(|s| s.required), Some(validated));
                field.edit_fixture(window, cx, |state, window, cx| {
                    let element = state.render(window, cx).into_element();
                    let role = element.a11y_role().unwrap_or(accesskit::Role::Unknown);
                    assert_eq!(
                        role,
                        if validated {
                            accesskit::Role::TextInput
                        } else {
                            accesskit::Role::SearchInput
                        }
                    );
                    let mut node = accesskit::Node::new(role);
                    element.write_a11y_info(&mut node);
                    assert_eq!(node.label(), Some(label));
                    assert_eq!(node.value(), Some("trail"));
                    assert!(node.supports_action(accesskit::Action::Focus));
                    assert_eq!(node.supports_action(accesskit::Action::SetValue), validated);
                });
            });
        });
    }
}

#[gpui::test]
fn selection_clipboard_and_readonly_use_the_form_value(cx: &mut TestAppContext) {
    let text = "Trail 中文 👩🏽‍💻";
    let (input, cx) = test_support::window(cx, |_, cx| TextInput::new(text, "Name", cx));
    cx.update(|window, cx| input.focus_handle(cx).focus(window, cx));
    test_support::press(cx, "home shift-end secondary-c");
    assert_eq!(
        cx.read_from_clipboard()
            .and_then(|item| item.text())
            .as_deref(),
        Some(text)
    );
    cx.simulate_input("新");
    assert_eq!(input.read_with(cx, |field, cx| field.value(cx)), "新");
    input.update(cx, |field, cx| field.set_read_only(true, cx));
    cx.simulate_input("ignored");
    assert_eq!(input.read_with(cx, |field, cx| field.value(cx)), "新");
}
