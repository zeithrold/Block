use gpui::{
    Context, Focusable as _, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, Window, accesskit, div, px, rgb,
};
use gpui_component::input::Input;

use super::TextInput;
use crate::{Palette, design};

#[derive(gpui::IntoElement)]
pub(crate) struct LabeledEditor {
    pub input: gpui::Entity<TextInput>,
    pub semantics: super::FieldSemantics,
}

impl gpui::RenderOnce for LabeledEditor {
    fn render(self, _: &mut Window, cx: &mut gpui::App) -> impl IntoElement {
        self.input.update(cx, |input, cx| {
            input.set_field_semantics(self.semantics, cx)
        });
        self.input
    }
}

impl TextInput {
    pub(super) fn render_field(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> gpui::AnyElement {
        let palette = cx
            .try_global::<Palette>()
            .copied()
            .unwrap_or(Palette::LIGHT);
        let Some(editor) = self.editor(window, cx) else {
            let text = self.text(cx);
            return div()
                .w_full()
                .h(px(design::CONTROL_HEIGHT))
                .px_3()
                .flex()
                .items_center()
                .overflow_hidden()
                .rounded(px(design::CONTROL_RADIUS))
                .border_1()
                .border_color(rgb(palette.input))
                .bg(rgb(palette.input_background))
                .opacity(if self.read_only { 0.65 } else { 1. })
                .child(if text.is_empty() {
                    self.placeholder.clone()
                } else {
                    text
                })
                .into_any_element();
        };
        if editor.read(cx).presentation().placeholder() != &self.placeholder {
            editor.update(cx, |state, cx| {
                state.set_placeholder(self.placeholder.clone(), window, cx)
            });
        }
        let semantics = self.field_semantics.as_ref();
        let invalid = semantics.is_some_and(|semantics| semantics.invalid);
        let required = semantics.is_some_and(|semantics| semantics.required);
        let label = semantics.map_or_else(|| self.placeholder.clone(), |s| s.label.clone());
        let mut input = Input::new(&editor)
            .id("text-input")
            .aria_label(label.clone())
            .role(if self.search {
                accesskit::Role::SearchInput
            } else {
                accesskit::Role::TextInput
            })
            .readonly(self.read_only)
            .w_full()
            .min_w_0()
            .h(px(design::CONTROL_HEIGHT))
            .rounded(px(design::CONTROL_RADIUS))
            .text_size(px(design::BODY_FONT_SIZE))
            .line_height(px(design::BODY_LINE_HEIGHT))
            .border_color(rgb(if invalid {
                palette.destructive
            } else if editor.focus_handle(cx).is_focused(window) {
                palette.ring
            } else {
                palette.input
            }))
            .bg(rgb(if self.search {
                palette.secondary
            } else {
                palette.input_background
            }));
        if self.search {
            input = input.prefix(design::Icon::Search.render(14., palette.muted));
        }
        let mut group = div().id("field-validation").w_full().min_w_0();
        if let Some(semantics) = semantics {
            let read_only = self.read_only;
            group = group
                .role(accesskit::Role::Group)
                .aria_label(label)
                .a11y_synthetic_children(move |builder| {
                    let node = builder.parent_node();
                    if invalid {
                        node.set_invalid(accesskit::Invalid::True);
                    }
                    if required {
                        node.set_required();
                    }
                    if read_only {
                        node.set_read_only();
                    }
                });
            if let Some(description) = &semantics.description {
                group = group.aria_description(description.clone());
            }
        }
        group.child(input).into_any_element()
    }
}
