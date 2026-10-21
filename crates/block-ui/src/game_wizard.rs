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
        div()
            .id("game-wizard")
            .debug_selector(|| "game-wizard".into())
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_6()
            .child(div().flex().gap_3().children(
                [(1, "Game setup", !review), (2, "Review", review)].map(
                    |(number, title, current)| {
                        div()
                            .flex_1()
                            .rounded_lg()
                            .border_1()
                            .border_color(rgb(if current {
                                self.palette.primary
                            } else {
                                self.palette.border
                            }))
                            .bg(rgb(self.palette.surface))
                            .p_3()
                            .text_sm()
                            .child(format!("{number}  {title}"))
                            .child(note(if current { "Current step" } else { "" }))
                    },
                ),
            ))
            .child(div().min_h(px(310.)).overflow_hidden().child(body))
            .child(
                div()
                    .border_t_1()
                    .border_color(rgb(self.palette.border))
                    .pt_4()
                    .flex()
                    .flex_wrap()
                    .justify_between()
                    .items_center()
                    .gap_3()
                    .child(self.button("cancel-instance", "Cancel", UiAction::CancelForm, cx))
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .children(show_back.then(|| {
                                self.button("wizard-back", "Back", UiAction::WizardBack, cx)
                            }))
                            .child(self.button("wizard-next", next_label, next_action, cx)),
                    ),
            )
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
            .child(div().text_2xl().child("Create your game"))
            .child(note(
                "Choose the game components. Review everything before installation begins.",
            ))
            .child(field("Game name", &form.name))
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
            .child(self.wizard_unavailable(
                "optifine",
                "OptiFine",
                "Installation is not available yet",
            ))
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
            ))
    }

    fn wizard_component(
        &self,
        id: &str,
        title: &str,
        value: &str,
        page: WizardPage,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement + use<> {
        self.button(
            format!("wizard-component-{id}"),
            title.to_owned(),
            UiAction::WizardPage(Some(page)),
            cx,
        )
        .w_full()
        .flex()
        .flex_col()
        .items_start()
        .gap_2()
        .p_4()
        .child(note(value.to_owned()))
    }

    fn wizard_unavailable(&self, id: &str, title: &str, reason: &str) -> impl IntoElement + use<> {
        div()
            .id(gpui::SharedString::from(format!("wizard-unavailable-{id}")))
            .w_full()
            .rounded_md()
            .border_1()
            .border_color(rgb(self.palette.border))
            .p_4()
            .opacity(0.6)
            .flex()
            .flex_col()
            .gap_2()
            .child(format!("{title} · Unavailable"))
            .child(note(reason.to_owned()))
    }

    fn wizard_version(
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

    fn wizard_loader(
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
                self.wizard_unavailable(loader, loader, "This loader does not have an installer in Block yet")
            }))
    }

    fn wizard_location(
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
                ("Java runtime", "Automatic · managed by Block".to_owned()),
                ("Maximum memory", "2048 MiB · editable after creation".to_owned()),
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
