//! A bordered list of full-width native action controls.

use gpui::{
    App, Div, FontWeight, InteractiveElement as _, ParentElement as _, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, px, rgb,
};

use super::{
    Palette,
    components::{Control, ControlState, interactive_control},
    design::{BODY_FONT_SIZE, BODY_LINE_HEIGHT, Icon, SMALL_FONT_SIZE},
};

/// Groups related actions in one surface, with dividers only between rows.
pub(super) struct ActionListCard;

impl ActionListCard {
    pub(super) fn render(palette: Palette, rows: impl IntoIterator<Item = ActionRow>) -> Div {
        div()
            .w_full()
            .min_w_0()
            .rounded(px(14.))
            .border_1()
            .border_color(rgb(palette.border))
            .overflow_hidden()
            .children(rows.into_iter().enumerate().map(|(index, row)| {
                div()
                    .children((index > 0).then(|| div().h(px(1.)).bg(rgb(palette.border))))
                    .child(row.control)
            }))
    }
}

/// The trailing affordance also determines the native control role.
pub(super) enum ActionDestination {
    External,
    Forward,
}

pub(super) struct ActionRow {
    control: Control,
}

/// The visible and accessible text of a row.
pub(super) struct ActionCopy {
    pub title: SharedString,
    pub description: SharedString,
}

/// Presentation that follows the destination and availability of the action.
pub(super) struct ActionAppearance {
    pub palette: Palette,
    pub destination: ActionDestination,
    pub state: ControlState,
}

impl ActionRow {
    // The ID, copy, presentation policy, and callback are independent inputs;
    // the callback is not part of the presentation data.
    pub(super) fn new(
        id: &'static str,
        copy: ActionCopy,
        appearance: ActionAppearance,
        on_activate: impl Fn(&(), &mut Window, &mut App) + 'static,
    ) -> Self {
        let ActionAppearance {
            palette,
            destination,
            state,
        } = appearance;
        let (icon, role) = match destination {
            ActionDestination::External => (Icon::ExternalLink, Role::Link),
            ActionDestination::Forward => (Icon::ChevronRight, Role::Button),
        };
        let control = interactive_control(
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
        .items_center()
        .gap(px(14.))
        .p(px(13.))
        .border_1()
        .border_color(gpui::transparent_black())
        .bg(rgb(palette.background))
        .text_color(rgb(palette.foreground))
        .whitespace_normal()
        .focus_visible(move |style| {
            style
                .border_color(rgb(palette.ring))
                .bg(rgb(palette.accent))
        })
        .child(
            div()
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
                )
                .child(
                    div()
                        .text_size(px(SMALL_FONT_SIZE))
                        .line_height(px(19.5))
                        .text_color(rgb(palette.muted))
                        .child(copy.description),
                ),
        )
        .child(div().flex_shrink_0().child(icon.render(16., palette.muted)));
        Self {
            control: if state.interactive() {
                control.active(move |style| style.bg(rgb(palette.accent)).opacity(0.85))
            } else {
                control
            },
        }
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
        fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl gpui::IntoElement {
            let palette = Palette::LIGHT;
            div()
                .id("action-list-fixture")
                .tab_group()
                .child(ActionListCard::render(
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
                            "action-next",
                            ActionCopy {
                                title: "Next step".into(),
                                description: "Proceed to the next step.".into(),
                            },
                            ActionAppearance {
                                palette,
                                destination: ActionDestination::Forward,
                                state: ControlState::default(),
                            },
                            cx.listener(|view, (), _, _| {
                                view.chosen = Some(ActionDestination::Forward)
                            }),
                        ),
                    ],
                ))
        }
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
