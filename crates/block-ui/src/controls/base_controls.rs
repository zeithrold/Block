//! Block's presentation adapter around gpui-base's unstyled controls. The base
//! primitives own native activation, focus and accessibility; no overlay Div or
//! second focus handle is added here.

#![expect(
    clippy::redundant_pub_crate,
    reason = "Controls are private to Block's shared design system."
)]

use std::time::Duration;

use gpui::{
    AnyElement, App, ClickEvent, Fill, FocusHandle, Hsla, InteractiveElement, Interactivity,
    IntoElement, ParentElement, RenderOnce, Role, SharedString, StatefulInteractiveElement,
    StyleRefinement, Styled, Window, rgb,
};
use gpui_base::motion::{Easing, Transition, transition};

#[derive(IntoElement)]
pub(super) enum Control {
    Button(Box<ButtonPresentation>),
    Checkbox(Box<gpui_base::Checkbox>),
    Radio(Box<gpui_base::Radio>),
    Selection(Box<super::selection_motion::SelectionPresentation>),
}

pub(super) struct ButtonPresentation {
    control: gpui_base::Button,
    id: SharedString,
    hover: Hsla,
    disabled: bool,
}

impl ButtonPresentation {
    fn render(mut self, window: &mut Window, cx: &mut App) -> gpui_base::Button {
        let hovered = window.use_keyed_state(
            gpui::ElementId::NamedChild(
                gpui::ElementId::from(self.id.clone()).into(),
                "hover".into(),
            ),
            cx,
            |_, _| false,
        );
        let progress = transition(
            (self.id, "hover-fill"),
            f32::from(!self.disabled && *hovered.read(cx)),
            Transition::new(if self.disabled {
                Duration::ZERO
            } else {
                Duration::from_millis(100)
            })
            .easing(Easing::CubicBezier {
                x1: 0.4,
                y1: 1.,
                x2: 0.6,
                y2: 1.,
            }),
            window,
            cx,
        );
        // Read the final consumer style, including transparent/custom fills.
        // Native active/focus styles still refine over this animated base fill.
        if let Some(background) = self
            .control
            .style()
            .background
            .as_ref()
            .and_then(Fill::color)
            .and_then(|background| background.as_solid())
        {
            let background = background.to_rgb();
            let hover = self.hover.to_rgb();
            let alpha = background.a + (hover.a - background.a) * progress;
            let weight = if alpha > 0. {
                hover.a * progress / alpha
            } else {
                0.
            };
            self.control = self
                .control
                .bg(background.blend(hover.alpha(weight)).alpha(alpha));
        }
        self.control.on_hover(move |is_hovered, _, cx| {
            hovered.update(cx, |hovered, cx| {
                *hovered = *is_hovered;
                cx.notify();
            });
        })
    }
}

/// The supplied label is the control's accessible name. It is applied through
/// `aria_label` rather than base's stored `accessibility_label`, which base
/// renders after the consumer's own `aria_label` and would rename an icon
/// control the consumer had already named. An empty label adds no name, matching
/// the visible-text guard in `components::styled_control`.
fn named<C: StatefulInteractiveElement>(control: C, label: SharedString) -> C {
    if label.is_empty() {
        control
    } else {
        control.aria_label(label)
    }
}

impl Control {
    /// The click event is forwarded rather than erased because a control may
    /// need the press geometry or modality to decide whether to activate. The
    /// primitive still owns role, focus and keyboard activation.
    pub(super) fn button(
        id: SharedString,
        label: SharedString,
        role: Role,
        disabled: bool,
        hover: u32,
        on_activate: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self::Button(Box::new(ButtonPresentation {
            control: named(
                gpui_base::Button::new(id.clone())
                    .role(role)
                    .disabled(disabled)
                    .on_click(move |event, window, cx| {
                        on_activate(event, window, cx);
                        cx.stop_propagation();
                    }),
                label,
            ),
            id,
            hover: rgb(hover).into(),
            disabled,
        }))
    }

    pub(super) fn checkbox(
        id: SharedString,
        label: SharedString,
        checked: bool,
        disabled: bool,
        on_activate: impl Fn(&(), &mut Window, &mut App) + 'static,
    ) -> Self {
        Self::Checkbox(Box::new(named(
            gpui_base::Checkbox::new(id)
                .checked(checked)
                .disabled(disabled)
                .on_change(move |_, _, window, cx| {
                    on_activate(&(), window, cx);
                    cx.stop_propagation();
                }),
            label,
        )))
    }

    pub(super) fn radio(
        id: SharedString,
        label: SharedString,
        checked: bool,
        disabled: bool,
        on_activate: impl Fn(&(), &mut Window, &mut App) + 'static,
    ) -> Self {
        let mut radio = named(
            gpui_base::Radio::new(id)
                .checked(checked)
                .disabled(disabled)
                .on_change(move |_, _, window, cx| {
                    on_activate(&(), window, cx);
                    cx.stop_propagation();
                }),
            label,
        );
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
            Self::Button(mut control) => {
                control.control = control.control.track_focus(handle);
                Self::Button(control)
            }
            Self::Checkbox(control) => Self::Checkbox(Box::new((*control).track_focus(handle))),
            Self::Radio(control) => Self::Radio(Box::new((*control).track_focus(handle))),
            Self::Selection(mut presentation) => {
                presentation.control = presentation.control.track_focus(handle);
                Self::Selection(presentation)
            }
        }
    }

    pub(super) fn tab_stop(self, tab_stop: bool) -> Self {
        match self {
            Self::Button(mut control) => {
                control.control = control.control.tab_stop(tab_stop);
                Self::Button(control)
            }
            Self::Checkbox(control) => Self::Checkbox(Box::new((*control).tab_stop(tab_stop))),
            Self::Radio(control) => Self::Radio(Box::new((*control).tab_stop(tab_stop))),
            Self::Selection(mut presentation) => {
                presentation.control = presentation.control.tab_stop(tab_stop);
                Self::Selection(presentation)
            }
        }
    }

    pub(super) fn tab_index(self, index: isize) -> Self {
        match self {
            Self::Button(mut control) => {
                control.control = control.control.tab_index(index).tab_stop(index >= 0);
                Self::Button(control)
            }
            Self::Checkbox(control) => {
                Self::Checkbox(Box::new((*control).tab_index(index).tab_stop(index >= 0)))
            }
            Self::Radio(control) => {
                Self::Radio(Box::new((*control).tab_index(index).tab_stop(index >= 0)))
            }
            Self::Selection(mut presentation) => {
                presentation.control = presentation.control.tab_index(index);
                Self::Selection(presentation)
            }
        }
    }
}

impl Styled for Control {
    fn style(&mut self) -> &mut StyleRefinement {
        match self {
            Self::Button(control) => control.control.style(),
            Self::Checkbox(control) => control.style(),
            Self::Radio(control) => control.style(),
            Self::Selection(presentation) => presentation.control.style(),
        }
    }
}

impl ParentElement for Control {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        match self {
            Self::Button(control) => control.control.extend(elements),
            Self::Checkbox(control) => control.extend(elements),
            Self::Radio(control) => control.extend(elements),
            Self::Selection(presentation) => presentation.children.extend(elements),
        }
    }
}

impl InteractiveElement for Control {
    fn interactivity(&mut self) -> &mut Interactivity {
        match self {
            Self::Button(control) => control.control.interactivity(),
            Self::Checkbox(control) => control.interactivity(),
            Self::Radio(control) => control.interactivity(),
            Self::Selection(presentation) => presentation.control.interactivity(),
        }
    }
}

impl StatefulInteractiveElement for Control {}

impl RenderOnce for Control {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        match self {
            Self::Button(control) => (*control).render(window, cx).into_any_element(),
            Self::Checkbox(control) => (*control).into_any_element(),
            Self::Radio(control) => (*control).into_any_element(),
            Self::Selection(presentation) => (*presentation).render(window, cx).into_any_element(),
        }
    }
}
