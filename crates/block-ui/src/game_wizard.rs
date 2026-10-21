#![expect(
    clippy::redundant_pub_crate,
    reason = "Wizard state is shared only with the parent view and shell."
)]
//! Draft-only game setup. The final review is the only installation action.

use std::time::Duration;

use gpui::{
    Animation, AnimationExt as _, Context, InteractiveElement as _, IntoElement,
    ParentElement as _, StatefulInteractiveElement as _, Styled as _, div, px, rgb,
};

use super::{Block, InstanceForm, UiAction, field, note};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum WizardStep {
    #[default]
    Setup,
    Review,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum WizardPage {
    Version,
    Loader,
    Location,
}

#[derive(Debug, Default)]
pub(super) struct WizardState {
    pub step: WizardStep,
    pub page: Option<WizardPage>,
    pub revision: usize,
    pub backwards: bool,
}

impl WizardState {
    pub(super) fn next(&mut self) {
        if self.page.is_some() {
            self.open_page(None);
        } else if self.step == WizardStep::Setup {
            self.step = WizardStep::Review;
            self.transition(false);
        }
    }

    pub(super) fn back(&mut self) {
        if self.page.is_some() {
            self.open_page(None);
        } else if self.step == WizardStep::Review {
            self.step = WizardStep::Setup;
            self.transition(true);
        }
    }

    pub(super) fn open_page(&mut self, page: Option<WizardPage>) {
        if self.page != page {
            self.page = page;
            self.transition(page.is_none());
        }
    }

    const fn transition(&mut self, backwards: bool) {
        self.backwards = backwards;
        self.revision = self.revision.wrapping_add(1);
    }
}

impl Block {
    pub(super) fn render_game_wizard(
        &self,
        form: &InstanceForm,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement + use<> {
        let state = &self.shell.wizard;
        let review = state.step == WizardStep::Review;
        let body = self.game_wizard_body(form, cx);
        let show_back = review || state.page.is_some();
        let next_label = if state.page.is_some() {
            "Done"
        } else if review {
            "Create and install"
        } else {
            "Continue"
        };
        let next_action = if review && state.page.is_none() {
            UiAction::SaveInstance
        } else {
            UiAction::WizardNext
        };
        let footer = div()
            .w_full()
            .flex()
            .justify_between()
            .items_center()
            .gap_3()
            .child(self.button("cancel-instance", "Cancel", UiAction::CancelForm, cx))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .children(
                        show_back
                            .then(|| self.button("wizard-back", "Back", UiAction::WizardBack, cx)),
                    )
                    .child(self.button("wizard-next", next_label, next_action, cx)),
            )
            .into_any_element();
        let body = div()
            .flex()
            .flex_col()
            .gap_4()
            .child(body)
            .child(note(self.status.clone()))
            .into_any_element();
        div()
            .id("game-wizard")
            .debug_selector(|| "game-wizard".into())
            .size_full()
            .min_h_0()
            .child(self.render_wizard_frame(
                &[
                    ("Game setup", "Name and configure"),
                    ("Review", "Check the details"),
                ],
                usize::from(review),
                body,
                footer,
            ))
    }

    pub(super) fn game_wizard_body(
        &self,
        form: &InstanceForm,
        cx: &Context<'_, Self>,
    ) -> gpui::AnyElement {
        let state = &self.shell.wizard;
        let review = state.step == WizardStep::Review;
        let mut body = match state.page {
            Some(WizardPage::Version) => self.wizard_version(form, cx).into_any_element(),
            Some(WizardPage::Loader) => self.wizard_loader(form, cx).into_any_element(),
            Some(WizardPage::Location) => self.wizard_location(form, cx).into_any_element(),
            None if review => self.wizard_review(form, cx).into_any_element(),
            None => self.wizard_setup(form, cx).into_any_element(),
        };
        if !self.preferences.reduce_motion {
            let backwards = state.backwards;
            body = div()
                .relative()
                .w_full()
                .child(body)
                .with_animation(
                    ("game-wizard-step", state.revision),
                    Animation::new(Duration::from_millis(460))
                        .with_easing(|t| 1. - (1. - t).powi(4)),
                    move |element, progress| {
                        element
                            .left(px((1. - progress) * if backwards { -48. } else { 48. }))
                            .opacity(progress)
                    },
                )
                .into_any_element();
        }
        body
    }

    /// Shared fixed rail and action footer for game setup and first-run setup.
    pub(super) fn render_wizard_frame(
        &self,
        steps: &[(&str, &str)],
        current: usize,
        body: gpui::AnyElement,
        footer: gpui::AnyElement,
    ) -> gpui::AnyElement {
        let rail = self.wizard_step_rail(steps, current);
        div()
            .id("wizard-frame")
            .size_full()
            .min_h_0()
            .flex()
            .child(rail)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .id("wizard-body")
                            .debug_selector(|| "wizard-body".into())
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .child(div().w_full().max_w(px(608.)).mx_auto().p_8().child(body)),
                    )
                    .child(
                        div()
                            .id("wizard-footer")
                            .debug_selector(|| "wizard-footer".into())
                            .h(px(72.))
                            .flex_shrink_0()
                            .border_t_1()
                            .border_color(rgb(self.palette.border))
                            .flex()
                            .items_center()
                            .child(
                                div()
                                    .w_full()
                                    .max_w(px(608.))
                                    .mx_auto()
                                    .px_8()
                                    .flex()
                                    .flex_col()
                                    .child(footer),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn wizard_step_rail(&self, steps: &[(&str, &str)], current: usize) -> gpui::AnyElement {
        div()
            .id("wizard-rail")
            .debug_selector(|| "wizard-rail".into())
            .w(gpui::relative(0.32))
            .min_w_0()
            .h_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .border_r_1()
            .border_color(rgb(self.palette.border))
            .bg(rgb(self.palette.surface))
            .child(
                div()
                    .id("wizard-steps")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_6()
                    .flex()
                    .flex_col()
                    .gap_6()
                    .child(note("Setup"))
                    .children(
                        steps
                            .iter()
                            .enumerate()
                            .map(|(index, (title, description))| {
                                let active = index == current;
                                div()
                                    .flex()
                                    .gap_3()
                                    .child(
                                        div()
                                            .size(px(28.))
                                            .flex_shrink_0()
                                            .rounded_full()
                                            .border_1()
                                            .border_color(rgb(if active {
                                                self.palette.primary
                                            } else {
                                                self.palette.border
                                            }))
                                            .bg(rgb(if active {
                                                self.palette.primary
                                            } else {
                                                self.palette.background
                                            }))
                                            .text_color(rgb(if active {
                                                self.palette.on_primary
                                            } else {
                                                self.palette.muted
                                            }))
                                            .text_sm()
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .child((index + 1).to_string()),
                                    )
                                    .child(
                                        div()
                                            .min_w_0()
                                            .flex()
                                            .flex_col()
                                            .gap_2()
                                            .text_sm()
                                            .text_color(rgb(if active {
                                                self.palette.foreground
                                            } else {
                                                self.palette.muted
                                            }))
                                            .child((*title).to_owned())
                                            .child(note((*description).to_owned())),
                                    )
                            }),
                    ),
            )
            .child(
                div()
                    .h(px(72.))
                    .flex_shrink_0()
                    .border_t_1()
                    .border_color(rgb(self.palette.border))
                    .px_6()
                    .flex()
                    .items_center()
                    .child(note(format!("Step {} of {}", current + 1, steps.len()))),
            )
            .into_any_element()
    }

    fn wizard_setup(
        &self,
        form: &InstanceForm,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement + use<> {
        let version = form.version.read(cx).value();
        let directory = form.directory.read(cx).value();
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(div().text_xl().child("Create your game"))
            .child(note(
                "Choose the game components. Review everything before installation begins.",
            ))
            .child(field("Game name", &form.name))
            .child(
                div()
                    .grid()
                    .grid_cols(4)
                    .gap_3()
                    .child(self.wizard_component(
                        "version",
                        "Minecraft version",
                        &version,
                        WizardPage::Version,
                        cx,
                    ))
                    .child(self.wizard_component(
                        "loader",
                        "Mod loader",
                        if form.neoforge {
                            "NeoForge 20.6.144"
                        } else {
                            "Vanilla"
                        },
                        WizardPage::Loader,
                        cx,
                    ))
                    .child(self.wizard_unavailable("optifine", "OptiFine", "Unavailable", true))
                    .child(self.wizard_component(
                        "location",
                        "Installation location",
                        if directory.is_empty() {
                            "Automatic · Block data folder"
                        } else {
                            &directory
                        },
                        WizardPage::Location,
                        cx,
                    )),
            )
    }

    fn wizard_component(
        &self,
        id: &str,
        title: &str,
        value: &str,
        page: WizardPage,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement + use<> {
        let mut card = self
            .button(
                format!("wizard-component-{id}"),
                title.to_owned(),
                UiAction::WizardPage(Some(page)),
                cx,
            )
            .w_full()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .items_start()
            .justify_between()
            .gap_2()
            .p_3()
            .child(note(value.to_owned()).w_full().line_clamp(3));
        card.style().aspect_ratio = Some(1.);
        card
    }

    fn wizard_unavailable(
        &self,
        id: &str,
        title: &str,
        reason: &str,
        square: bool,
    ) -> impl IntoElement + use<> {
        let mut card = div()
            .id(gpui::SharedString::from(format!("wizard-unavailable-{id}")))
            .debug_selector({
                let selector = format!("wizard-unavailable-{id}");
                move || selector
            })
            .w_full()
            .min_w_0()
            .min_h_0()
            .rounded_md()
            .border_1()
            .border_color(rgb(self.palette.border))
            .p_3()
            .opacity(0.6)
            .flex()
            .flex_col()
            .justify_between()
            .gap_2()
            .child(title.to_owned())
            .child(note(reason.to_owned()).w_full().line_clamp(3));
        if square {
            card.style().aspect_ratio = Some(1.);
        }
        card
    }

    pub(super) fn wizard_version(
        &self,
        form: &InstanceForm,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement + use<> {
        let selected = form.version.read(cx).value();
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(div().text_2xl().child("Minecraft version"))
            .child(note(
                "Choose a version from the official catalog, or enter an exact version ID.",
            ))
            .child(field("Minecraft version", &form.version))
            .child(
                div()
                    .id("wizard-version-list")
                    .max_h(px(240.))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .children(self.versions.iter().enumerate().map(|(index, version)| {
                        self.button(
                            format!("wizard-version-{index}"),
                            if *version == selected {
                                format!("Selected · {version}")
                            } else {
                                version.clone()
                            },
                            UiAction::SelectVersion(version.clone()),
                            cx,
                        )
                    })),
            )
            .child(note(
                "Changing away from Minecraft 1.20.6 resets the mod loader to Vanilla.",
            ))
    }

    pub(super) fn wizard_loader(
        &self,
        form: &InstanceForm,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement + use<> {
        let option = |id: &str, name: &str, selected: bool| {
            if selected {
                div()
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(self.palette.primary))
                    .bg(rgb(self.palette.surface))
                    .px_3()
                    .py_2()
                    .child(format!("Selected · {name}"))
                    .into_any_element()
            } else {
                self.button(id.to_owned(), name.to_owned(), UiAction::ToggleLoader, cx)
                    .into_any_element()
            }
        };
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(div().text_2xl().child("Mod loader"))
            .child(note("Vanilla is available for official versions. NeoForge currently supports one verified version pair."))
            .child(option("wizard-loader-vanilla", "Vanilla", !form.neoforge))
            .child(option("wizard-loader-neoforge", "NeoForge 20.6.144 · Minecraft 1.20.6", form.neoforge))
            .child(note("Selecting NeoForge also selects Minecraft 1.20.6."))
            .children(["Fabric", "Forge", "Quilt"].map(|loader| {
                self.wizard_unavailable(loader, loader, "This loader does not have an installer in Block yet", false)
            }))
    }

    pub(super) fn wizard_location(
        &self,
        form: &InstanceForm,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement + use<> {
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(div().text_2xl().child("Installation location"))
            .child(note("Choose a new folder for this game, or leave the location empty for automatic storage."))
            .child(field("Installation folder (optional)", &form.directory))
            .child(self.button("browse-install", "Choose folder…", UiAction::BrowseInstall, cx))
            .child(note("Game data uses an isolated profile folder. Downloads are verified and cached for reuse."))
    }

    fn wizard_review(
        &self,
        form: &InstanceForm,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement + use<> {
        let directory = form.directory.read(cx).value();
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(div().text_2xl().child("Ready for your next world?"))
            .child(note("Review your choices. Create and install will save this game and download the required files."))
            .children([
                ("Game name", form.name.read(cx).value()),
                ("Minecraft", form.version.read(cx).value()),
                ("Mod loader", if form.neoforge { "NeoForge 20.6.144" } else { "Vanilla" }.to_owned()),
                ("Installation location", if directory.is_empty() { "Automatic · Block data folder".to_owned() } else { directory }),
                ("Game data", "Isolated profile folder".to_owned()),
                ("Java runtime", if self.saved_preferences.game.java.is_empty() { "Automatic · managed by Block".to_owned() } else { self.saved_preferences.game.java.clone() }),
                ("Maximum memory", format!("{} MiB · editable after creation", self.saved_preferences.game.memory_mib)),
                ("Game window", format!("{} × {}{}", self.saved_preferences.game.width, self.saved_preferences.game.height, if self.saved_preferences.game.fullscreen { " · Fullscreen" } else { "" })),
            ].map(|(label, value)| {
                div()
                    .rounded_md()
                    .bg(rgb(self.palette.surface))
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(note(label))
                    .child(value)
            }))
    }
}
