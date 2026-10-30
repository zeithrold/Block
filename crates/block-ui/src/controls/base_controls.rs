//! Block's presentation adapter around gpui-base's unstyled controls. The base
//! primitives own native activation, focus and accessibility; no overlay Div or
//! second focus handle is added here.

#![expect(
    clippy::redundant_pub_crate,
    reason = "Controls are private to Block's shared design system."
)]

use gpui::{
    AnyElement, App, FocusHandle, InteractiveElement, Interactivity, IntoElement, ParentElement,
    RenderOnce, Role, SharedString, StatefulInteractiveElement, StyleRefinement, Styled, Window,
};

#[derive(IntoElement)]
pub(super) enum Control {
    Button(Box<gpui_base::Button>),
    Checkbox(Box<gpui_base::Checkbox>),
    Radio(Box<gpui_base::Radio>),
}

impl Control {
    pub(super) fn button(
        id: SharedString,
        _label: SharedString,
        role: Role,
        disabled: bool,
        on_activate: impl Fn(&(), &mut Window, &mut App) + 'static,
    ) -> Self {
        Self::Button(Box::new(
            gpui_base::Button::new(id)
                .role(role)
                .disabled(disabled)
                .on_click(move |_, window, cx| {
                    on_activate(&(), window, cx);
                    cx.stop_propagation();
                }),
        ))
    }

    pub(super) fn checkbox(
        id: SharedString,
        _label: SharedString,
        checked: bool,
        disabled: bool,
        on_activate: impl Fn(&(), &mut Window, &mut App) + 'static,
    ) -> Self {
        Self::Checkbox(Box::new(
            gpui_base::Checkbox::new(id)
                .checked(checked)
                .disabled(disabled)
                .on_change(move |_, _, window, cx| {
                    on_activate(&(), window, cx);
                    cx.stop_propagation();
                }),
        ))
    }

    pub(super) fn radio(
        id: SharedString,
        _label: SharedString,
        checked: bool,
        disabled: bool,
        on_activate: impl Fn(&(), &mut Window, &mut App) + 'static,
    ) -> Self {
        let mut radio = gpui_base::Radio::new(id)
            .checked(checked)
            .disabled(disabled)
            .on_change(move |_, _, window, cx| {
                on_activate(&(), window, cx);
                cx.stop_propagation();
            });
        // Base deliberately emits no change for an already checked radio.
        // Consume its native click at the control boundary so an enclosing
        // actionable row does not mistake that no-op for its own activation.
        if checked && !disabled {
            radio = radio.on_click(|_, _, cx| cx.stop_propagation());
        }
        Self::Radio(Box::new(radio))
    }

    // These are inherent forwards because base primitives carry the provided
    // handle/tab policy through RenderOnce. Mutating only Interactivity would
    // be overwritten by their default keyed focus state at render time.
    pub(super) fn track_focus(self, handle: &FocusHandle) -> Self {
        match self {
            Self::Button(control) => Self::Button(Box::new((*control).track_focus(handle))),
            Self::Checkbox(control) => Self::Checkbox(Box::new((*control).track_focus(handle))),
            Self::Radio(control) => Self::Radio(Box::new((*control).track_focus(handle))),
        }
    }

    pub(super) fn tab_stop(self, tab_stop: bool) -> Self {
        match self {
            Self::Button(control) => Self::Button(Box::new((*control).tab_stop(tab_stop))),
            Self::Checkbox(control) => Self::Checkbox(Box::new((*control).tab_stop(tab_stop))),
            Self::Radio(control) => Self::Radio(Box::new((*control).tab_stop(tab_stop))),
        }
    }

    pub(super) fn tab_index(self, index: isize) -> Self {
        match self {
            Self::Button(control) => {
                Self::Button(Box::new((*control).tab_index(index).tab_stop(index >= 0)))
            }
            Self::Checkbox(control) => {
                Self::Checkbox(Box::new((*control).tab_index(index).tab_stop(index >= 0)))
            }
            Self::Radio(control) => {
                Self::Radio(Box::new((*control).tab_index(index).tab_stop(index >= 0)))
            }
        }
    }
}

impl Styled for Control {
    fn style(&mut self) -> &mut StyleRefinement {
        match self {
            Self::Button(control) => control.style(),
            Self::Checkbox(control) => control.style(),
            Self::Radio(control) => control.style(),
        }
    }
}

impl ParentElement for Control {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        match self {
            Self::Button(control) => control.extend(elements),
            Self::Checkbox(control) => control.extend(elements),
            Self::Radio(control) => control.extend(elements),
        }
    }
}

impl InteractiveElement for Control {
    fn interactivity(&mut self) -> &mut Interactivity {
        match self {
            Self::Button(control) => control.interactivity(),
            Self::Checkbox(control) => control.interactivity(),
            Self::Radio(control) => control.interactivity(),
        }
    }
}

impl StatefulInteractiveElement for Control {}

impl RenderOnce for Control {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        match self {
            Self::Button(control) => (*control).into_any_element(),
            Self::Checkbox(control) => (*control).into_any_element(),
            Self::Radio(control) => (*control).into_any_element(),
        }
    }
}
