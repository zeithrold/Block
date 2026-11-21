//! Product bindings for the shared anchored selector surface.
#![expect(
    clippy::redundant_pub_crate,
    reason = "Selectors are internal to the product shell."
)]
use super::{Block, Form, Language, UiAction, scroll_area::ScrollAreaExt as _, shell::Surface};
use block_launcher::catalog::{InstanceColor, InstanceIcon};
use gpui::{
    Bounds, Context, FocusHandle, InteractiveElement as _, IntoElement, KeyDownEvent,
    ParentElement as _, Pixels, Point, ScrollHandle, Styled as _, Window, div, point, px,
};
use std::time::{Duration, Instant};
use unicode_segmentation::UnicodeSegmentation as _;

#[derive(Clone, Debug)]
pub(super) struct SelectChoice<A> {
    pub label: String,
    pub selected: bool,
    pub action: A,
}

#[derive(Clone, Debug)]
pub(super) struct SelectState<A> {
    /// Semantic catalog key; the label is resolved whenever the popup paints.
    title: &'static str,
    pub choices: Vec<SelectChoice<A>>,
    pub anchor: Point<Pixels>,
    pub width: Pixels,
    pub active: usize,
    pub focus: FocusHandle,
    /// The options viewport. Long catalogs (Minecraft versions, loader builds)
    /// scroll inside the popup instead of growing past it.
    pub scroll: ScrollHandle,
    identity: SelectIdentity,
    typeahead: String,
    typed_at: Option<Instant>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SelectIdentity {
    Appearance { wizard: bool, color: bool },
}

impl SelectIdentity {
    const fn action(&self) -> UiAction {
        match self {
            Self::Appearance { wizard, color } => UiAction::ChooseAppearance {
                wizard: *wizard,
                color: *color,
            },
        }
    }
}

impl<A> SelectState<A> {
    /// The selector's own name in the language of the view that presents it.
    pub(super) fn title(&self, language: Language) -> String {
        tr!(language, self.title)
    }

    fn typeahead(&mut self, text: &str, now: Instant) -> Option<usize> {
        if self
            .typed_at
            .is_none_or(|last| now.saturating_duration_since(last) >= Duration::from_millis(700))
        {
            self.typeahead.clear();
        }
        self.typed_at = Some(now);
        let text = text.to_lowercase();
        let repeated =
            !self.typeahead.is_empty() && self.typeahead.graphemes(true).all(|part| part == text);
        if !repeated {
            self.typeahead.push_str(&text);
        }
        let start = if self.typeahead == text {
            self.active + 1
        } else {
            self.active
        };
        (0..self.choices.len())
            .map(|offset| (start + offset) % self.choices.len())
            .find(|index| {
                self.choices[*index]
                    .label
                    .to_lowercase()
                    .starts_with(&self.typeahead)
            })
    }
}

impl Block {
    /// Refresh only option labels, retaining the popup, active row and focus.
    pub(super) fn refresh_select_language(&mut self) {
        let language = self.language();
        let Some(layer) = self.shell.surfaces.top_mut() else {
            return;
        };
        let Surface::Select(state) = &mut layer.kind else {
            return;
        };
        for choice in &mut state.choices {
            choice.label = match &choice.action {
                UiAction::InstanceIcon(icon) | UiAction::WizardInstanceIcon(icon) => {
                    crate::i18n::instance_icon_name(language, *icon)
                }
                UiAction::InstanceColor(color) | UiAction::WizardInstanceColor(color) => {
                    crate::i18n::instance_color_name(language, *color)
                }
                UiAction::DeleteInstanceId(_) => tr!(language, "choice.delete_from_library"),
                _ => continue,
            };
        }
    }

    fn can_open_select(&self, identity: &SelectIdentity) -> bool {
        (!self.busy || Self::available_while_busy(&identity.action()))
            && !self.shell.surfaces.layers().any(|layer| {
                matches!(&layer.kind, Surface::Select(state) if &state.identity == identity)
            })
    }

    fn open_select_at(
        &mut self,
        title: &'static str,
        choices: Vec<SelectChoice<UiAction>>,
        identity: SelectIdentity,
        anchor: Option<Bounds<Pixels>>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if !self.can_open_select(&identity) {
            return;
        }
        let width = anchor.map_or(px(224.), |bounds| bounds.size.width.max(px(144.)));
        let active = choices
            .iter()
            .position(|choice| choice.selected)
            .unwrap_or(0);
        let focus = cx.focus_handle().tab_stop(true);
        let scroll = ScrollHandle::new();
        if !choices.is_empty() {
            scroll.scroll_to_item(active);
        }
        self.shell.surfaces.open(
            Surface::Select(SelectState {
                title,
                choices,
                active,
                focus: focus.clone(),
                scroll,
                identity,
                typeahead: String::new(),
                typed_at: None,
                anchor: anchor.map_or_else(
                    || point(px(16.), px(56.)),
                    |bounds| point(bounds.left(), bounds.bottom() + px(6.)),
                ),
                width,
            }),
            window,
            cx,
        );
        focus.focus(window, cx);
        let owner = cx.weak_entity();
        // The first callback precedes the initial layout. The second sees the
        // scroll viewport's bounds, so the selected item can be revealed.
        window.on_next_frame(move |window, _| {
            window.on_next_frame(move |window, cx| {
                let _ = owner.update(cx, |view, cx| {
                    let Some(layer) = view.shell.surfaces.top_mut() else {
                        return;
                    };
                    let Surface::Select(state) = &mut layer.kind else {
                        return;
                    };
                    if state.focus == focus {
                        state.scroll.scroll_to_item(state.active);
                        window.refresh();
                        cx.notify();
                    }
                });
            });
        });
    }

    pub(super) fn open_appearance_select(
        &mut self,
        wizard: bool,
        color: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.open_appearance_select_at(wizard, color, None, window, cx);
    }

    pub(super) fn open_appearance_select_at(
        &mut self,
        wizard: bool,
        color: bool,
        anchor: Option<Bounds<Pixels>>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let language = self.language();
        let appearance = if wizard {
            self.wizard_appearance()
        } else if let Some(Form::EditInstance(form)) = &self.form {
            form.original.appearance
        } else {
            return;
        };
        let choices: Vec<_> = if color {
            InstanceColor::ALL
                .into_iter()
                .map(|value| SelectChoice {
                    label: crate::i18n::instance_color_name(language, value),
                    selected: value == appearance.color,
                    action: if wizard {
                        UiAction::WizardInstanceColor(value)
                    } else {
                        UiAction::InstanceColor(value)
                    },
                })
                .collect()
        } else {
            InstanceIcon::ALL
                .into_iter()
                .map(|value| SelectChoice {
                    label: crate::i18n::instance_icon_name(language, value),
                    selected: value == appearance.icon,
                    action: if wizard {
                        UiAction::WizardInstanceIcon(value)
                    } else {
                        UiAction::InstanceIcon(value)
                    },
                })
                .collect()
        };
        self.open_select_at(
            if color {
                "select.instance_color"
            } else {
                "select.instance_icon"
            },
            choices,
            SelectIdentity::Appearance { wizard, color },
            anchor,
            window,
            cx,
        );
    }

    pub(super) fn select_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        let Some(layer) = self.shell.surfaces.top_mut() else {
            return false;
        };
        let Surface::Select(state) = &mut layer.kind else {
            return false;
        };
        let modifiers = event.keystroke.modifiers;
        if modifiers.control
            || modifiers.alt
            || modifiers.platform
            || modifiers.function
            || state.choices.is_empty()
        {
            return false;
        }
        let count = state.choices.len();
        let inspection = match event.keystroke.key.as_str() {
            "down" => (state.active + 1) % count,
            "up" => (state.active + count - 1) % count,
            "home" => 0,
            "end" => count - 1,
            _ => {
                let text = event
                    .keystroke
                    .key_char
                    .as_deref()
                    .unwrap_or(&event.keystroke.key);
                if text.graphemes(true).count() != 1
                    || text.trim().is_empty()
                    || text.chars().any(char::is_control)
                {
                    return false;
                }
                if let Some(index) = state.typeahead(text, cx.background_executor().now()) {
                    state.active = index;
                    state.scroll.scroll_to_item(index);
                }
                state.focus.focus(window, cx);
                self.motion_dirty = true;
                cx.notify();
                return true;
            }
        };
        state.active = inspection;
        state.scroll.scroll_to_item(inspection);
        state.typeahead.clear();
        state.typed_at = None;
        state.focus.focus(window, cx);
        self.motion_dirty = true;
        cx.notify();
        true
    }

    pub(super) fn select_options(
        &self,
        state: &SelectState<UiAction>,
        max_height: Pixels,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement + use<> {
        let revision = self.shell.surfaces.layers().find_map(|layer| {
            matches!(&layer.kind, Surface::Select(current) if current.focus == state.focus)
                .then_some(layer.revision)
        });
        // Version and loader-build catalogs are longer than the popup: the
        // options scroll inside it, and the active row stays reachable.
        let options = state.choices.iter().enumerate().map(|(index, choice)| {
            let disabled = self.busy && !Self::available_while_busy(&choice.action);
            let control_state = super::components::ControlState {
                disabled,
                ..Default::default()
            };
            let action = choice.action.clone();
            let on_activate = cx.listener(move |view, (): &(), window, cx| {
                if !view.shell.surfaces.top().is_some_and(|layer| {
                    Some(layer.revision) == revision && matches!(layer.kind, Surface::Select(_))
                }) {
                    return;
                }
                view.handle(UiAction::SelectValue(Box::new(action.clone())), window, cx);
            });
            let mut option = super::components::select_option(
                format!("select-choice-{index}"),
                choice.label.clone(),
                choice.selected,
                self.palette,
                control_state,
                on_activate,
            )
            .tab_stop(index == state.active && !disabled);
            if index == state.active {
                option = option.track_focus(&state.focus.clone().tab_stop(!disabled));
            }
            div().child(option)
        });
        div()
            .id("select-options")
            .max_h(max_height)
            .children(options)
            .scroll_list_area(&state.scroll, "select.options", self.palette)
    }
}

#[cfg(test)]
#[path = "select_tests.rs"]
mod tests;
