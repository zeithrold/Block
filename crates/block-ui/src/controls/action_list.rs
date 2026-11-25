//! A bordered list of full-width native action controls.

use gpui::{
    AnyElement, App, Div, FocusHandle, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, RenderOnce, Role, SharedString, StatefulInteractiveElement as _,
    Styled as _, Window, div, px, rgb,
};

use super::{
    Palette,
    components::{Control, ControlState, interactive_control},
    design::{BODY_FONT_SIZE, BODY_LINE_HEIGHT, Icon, SMALL_FONT_SIZE},
};

/// Groups related actions in one surface, with dividers only between rows.
pub(super) struct ActionMenuList;

impl ActionMenuList {
    pub(super) fn render(
        palette: Palette,
        rows: impl IntoIterator<Item = impl IntoElement>,
    ) -> Div {
        div()
            .w_full()
            .min_w_0()
            .flex_shrink_0()
            .rounded(px(14.))
            .border_1()
            .border_color(rgb(palette.border))
            .overflow_hidden()
            .children(rows.into_iter().enumerate().map(|(index, row)| {
                div()
                    .children((index > 0).then(|| div().h(px(1.)).bg(rgb(palette.border))))
                    .child(row)
            }))
    }
}

/// The trailing affordance also determines the native control role.
pub(super) enum ActionDestination {
    External,
    Forward,
    Disclosure(bool),
    Custom,
}

#[derive(IntoElement)]
pub(super) struct ActionRow {
    control: Control,
    copy: Div,
    description: Option<AnyElement>,
    leading: AnyElement,
    trailing: Option<AnyElement>,
}

/// The visible and accessible text of a row.
pub(super) struct ActionCopy {
    pub title: SharedString,
    pub description: SharedString,
}

/// Presentation that follows the destination and availability of the action.
pub(super) struct ActionAppearance {
    pub palette: Palette,
    pub icon: Icon,
    pub destination: ActionDestination,
    pub state: ControlState,
}

impl ActionRow {
    // The ID, copy, presentation policy, and callback are independent inputs;
    // the callback is not part of the presentation data.
    pub(super) fn new(
        id: impl Into<SharedString>,
        copy: ActionCopy,
        appearance: ActionAppearance,
        on_activate: impl Fn(&(), &mut Window, &mut App) + 'static,
    ) -> Self {
        let ActionAppearance {
            palette,
            icon,
            destination,
            state,
        } = appearance;
        let role = match destination {
            ActionDestination::External => Role::Link,
            ActionDestination::Forward
            | ActionDestination::Disclosure(_)
            | ActionDestination::Custom => Role::Button,
        };
        let mut control = interactive_control(
            id,
            copy.title.clone(),
            state,
            role,
            palette.accent,
            on_activate,
        )
        .aria_description(copy.description.clone())
        .w_full()
        .min_w_0()
        .min_h(px(44.))
        .flex()
        .items_start()
        .gap(px(10.))
        // The focus stroke is inside the prototype's 13px inset.
        .p(px(12.))
        .border_1()
        .border_color(gpui::transparent_black())
        .bg(rgb(palette.background))
        .text_color(rgb(palette.foreground))
        .whitespace_normal()
        .focus_visible(move |style| {
            style
                .border_color(rgb(palette.ring))
                .bg(rgb(palette.accent))
        });
        if state.interactive() {
            control = control.active(move |style| style.bg(rgb(palette.accent)).opacity(0.85));
        }
        let trailing = match destination {
            ActionDestination::External => Some(Icon::ExternalLink.render(16., palette.muted)),
            ActionDestination::Forward => Some(Icon::ChevronRight.render(16., palette.muted)),
            ActionDestination::Disclosure(expanded) => {
                control = control.aria_expanded(expanded);
                let chevron = Icon::ChevronDown.render(16., palette.muted);
                Some(if expanded {
                    chevron.with_transformation(gpui::Transformation::rotate(gpui::radians(
                        std::f32::consts::PI,
                    )))
                } else {
                    chevron
                })
            }
            ActionDestination::Custom => None,
        };
        let title = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(3.))
            .child(
                div()
                    .text_size(px(BODY_FONT_SIZE))
                    .line_height(px(BODY_LINE_HEIGHT))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(copy.title),
            );
        let description = (!copy.description.is_empty()).then(|| {
            div()
                .text_size(px(SMALL_FONT_SIZE))
                .line_height(px(19.5))
                .text_color(rgb(palette.muted))
                .child(copy.description)
                .into_any_element()
        });
        Self {
            control,
            copy: title,
            description,
            leading: icon.render(18., palette.foreground).into_any_element(),
            trailing: trailing.map(IntoElement::into_any_element),
        }
    }

    /// Use a brand mark in the same column as ordinary action icons.
    pub(super) fn leading(mut self, element: impl IntoElement) -> Self {
        self.leading = element.into_any_element();
        self
    }

    /// Replace the destination mark (e.g. with a badge or a loading ring).
    pub(super) fn trailing(mut self, element: impl IntoElement) -> Self {
        self.trailing = Some(element.into_any_element());
        self
    }

    pub(super) fn track_focus(mut self, focus: &FocusHandle) -> Self {
        self.control = self.control.track_focus(focus);
        self
    }

    /// The plain description supplied in `ActionCopy` remains the native description.
    pub(super) fn description(mut self, element: impl IntoElement) -> Self {
        self.description = Some(element.into_any_element());
        self
    }

    pub(super) fn map_control(mut self, decorate: impl FnOnce(Control) -> Control) -> Self {
        self.control = decorate(self.control);
        self
    }
}

impl RenderOnce for ActionRow {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        self.control
            .child(
                div()
                    .mt(px(1.5))
                    .size(px(18.))
                    .flex_shrink_0()
                    .child(self.leading),
            )
            .child(self.copy.children(self.description))
            .children(self.trailing.map(|tail| {
                div()
                    .flex_shrink_0()
                    .self_center()
                    .flex()
                    .items_center()
                    .child(tail)
            }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Context, Modifiers, Render, TestAppContext, point};

    #[derive(Default)]
    struct Actions {
        chosen: Option<ActionDestination>,
    }

    impl Render for Actions {
        fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
            let palette = Palette::LIGHT;
            div()
                .id("action-list-fixture")
                .w(px(240.))
                .tab_group()
                .child(ActionMenuList::render(
                    palette,
                    [
                        ActionRow::new(
                            "action-unavailable",
                            ActionCopy {
                                title: "Unavailable".into(),
                                description: "This browser action cannot run yet.".into(),
                            },
                            ActionAppearance {
                                palette,
                                icon: Icon::Browser,
                                destination: ActionDestination::External,
                                state: ControlState {
                                    disabled: true,
                                    ..ControlState::default()
                                },
                            },
                            cx.listener(|view, (), _, _| {
                                view.chosen = Some(ActionDestination::External)
                            }),
                        ),
                        ActionRow::new(
                            format!("action-{}", "next"),
                            ActionCopy {
                                title: "Next step".into(),
                                description: "Proceed to the next step.".into(),
                            },
                            ActionAppearance {
                                palette,
                                icon: Icon::DeviceCode,
                                destination: ActionDestination::Forward,
                                state: ControlState::default(),
                            },
                            cx.listener(|view, (), _, _| {
                                view.chosen = Some(ActionDestination::Forward)
                            }),
                        )
                        .leading(div().size(px(18.)).debug_selector(|| "brand-mark".into()))
                        .description(
                            div()
                                .debug_selector(|| "rich-description".into())
                                .child("选择一种方式继续设置，然后返回上一步查看当前的配置。"),
                        )
                        .trailing(div().size(px(16.)).debug_selector(|| "end-mark".into())),
                    ],
                ))
        }
    }

    #[gpui::test]
    fn rich_copy_wraps_between_a_top_aligned_brand_and_centered_end_mark(cx: &mut TestAppContext) {
        let (_, cx) = crate::test_support::window(cx, |_, _| Actions::default());
        let elements = ["action-next", "brand-mark", "rich-description", "end-mark"]
            .map(|id| crate::test_support::control_bounds(cx, id));
        let [Some(row), Some(brand), Some(description), Some(end)] = elements else {
            assert!(elements.iter().all(Option::is_some), "{elements:?}");
            return;
        };
        assert!(brand.top() < description.top());
        assert!(brand.right() < description.left());
        assert!(description.right() < end.left());
        assert!(description.size.height > px(BODY_LINE_HEIGHT));
        assert!((f32::from(end.center().y - row.center().y)).abs() <= 1.);
    }

    #[gpui::test]
    fn disabled_rows_ignore_pointer_and_are_skipped_by_keyboard(cx: &mut TestAppContext) {
        let (view, cx) = crate::test_support::window(cx, |_, _| Actions::default());
        let disabled = crate::test_support::control_bounds(cx, "action-unavailable");
        assert!(disabled.is_some(), "the disabled row remains visible");
        if let Some(disabled) = disabled {
            cx.simulate_click(disabled.center(), Modifiers::none());
        }
        assert!(view.read_with(cx, |view, _| view.chosen.is_none()));

        // The application shell routes Tab to GPUI's traversal. This fixture
        // mounts the controls without that shell.
        cx.update(|window, cx| window.focus_next(cx));
        cx.update(|window, cx| {
            assert!(
                window.focused(cx).is_some(),
                "the enabled row is a tab stop"
            );
            window.draw(cx).clear(cx);
        });
        crate::test_support::press(cx, "enter");

        assert!(view.read_with(cx, |view, _| {
            matches!(view.chosen, Some(ActionDestination::Forward))
        }));
    }

    #[gpui::test]
    fn clicking_the_trailing_padding_activates_the_row(cx: &mut TestAppContext) {
        let (view, cx) = crate::test_support::window(cx, |_, _| Actions::default());
        let row = crate::test_support::control_bounds(cx, "action-next");
        assert!(row.is_some(), "the action row is visible");
        if let Some(row) = row {
            cx.simulate_click(
                point(row.right() - px(4.), row.center().y),
                Modifiers::none(),
            );
        }

        assert!(view.read_with(cx, |view, _| {
            matches!(view.chosen, Some(ActionDestination::Forward))
        }));
    }
}
