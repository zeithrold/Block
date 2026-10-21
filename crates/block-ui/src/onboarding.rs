//! First launch presents one decision at a time; advanced game options stay optional.
#![expect(
    clippy::redundant_pub_crate,
    reason = "First-run state is shared only with the parent view."
)]
use std::time::Duration;

use gpui::{
    Animation, AnimationExt as _, Context, InteractiveElement as _, IntoElement,
    ParentElement as _, StatefulInteractiveElement as _, Styled as _, Window, div, px, relative,
    rgb,
};

use super::game_wizard::{WizardPage, WizardState};
use super::{Block, Form, InstanceForm, Tab, UiAction, account_id, field, input};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Progress {
    #[default]
    Draft,
    Created,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Step {
    #[default]
    Game,
    Account,
    Review,
}

#[derive(Debug, Default)]
pub(super) struct State {
    pub step: Step,
    pub options: bool,
    pub error: Option<String>,
}

impl Step {
    const fn index(self) -> usize {
        match self {
            Self::Game => 0,
            Self::Account => 1,
            Self::Review => 2,
        }
    }
}

impl Block {
    pub(super) fn begin_onboarding(&mut self, cx: &mut Context<'_, Self>) {
        self.tab = Tab::Onboarding;
        self.onboarding_progress = Progress::Draft;
        self.shell.onboarding.step = Step::Game;
        self.shell.onboarding.options = false;
        self.shell.onboarding.error = None;
        self.shell.wizard = WizardState::default();
        let version = self.versions.first().map_or("1.20.6", String::as_str);
        self.form = Some(Form::Instance(InstanceForm {
            name: input("My Minecraft", "Game name", cx),
            version: input(version, "Minecraft version", cx),
            directory: input("", "Automatic installation location", cx),
            neoforge: false,
        }));
    }

    pub(super) fn onboarding(&self, window: &Window, cx: &Context<'_, Self>) -> gpui::AnyElement {
        let compact = window.viewport_size().width < px(720.);
        let step = self.shell.onboarding.step;
        let mut body = match (&self.form, step) {
            (Some(Form::Instance(form)), Step::Game) => self.onboarding_game(form, cx),
            (_, Step::Account) => self.onboarding_account(cx),
            (Some(Form::Instance(form)), Step::Review) => self.onboarding_review(form, cx),
            _ => self
                .setup_caption("Preparing your game…")
                .into_any_element(),
        };
        if let Some(error) = &self.shell.onboarding.error {
            body = div()
                .flex()
                .flex_col()
                .gap_4()
                .child(body)
                .child(
                    div()
                        .id("onboarding-error")
                        .debug_selector(|| "onboarding-error".into())
                        .rounded_md()
                        .border_1()
                        .border_color(rgb(self.palette.border))
                        .p_3()
                        .text_sm()
                        .child(error.clone()),
                )
                .into_any_element();
        }
        if !self.preferences.reduce_motion {
            let backwards = self.shell.wizard.backwards;
            body = div()
                .relative()
                .w_full()
                .child(body)
                .with_animation(
                    ("first-run-content", self.shell.wizard.revision),
                    Animation::new(Duration::from_millis(320))
                        .with_easing(|t| 1. - (1. - t).powi(3)),
                    move |element, progress| {
                        element
                            .left(px((1. - progress) * if backwards { -24. } else { 24. }))
                            .opacity(progress)
                    },
                )
                .into_any_element();
        }
        div()
            .id("first-run")
            .debug_selector(|| "first-run".into())
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .children((!compact).then(|| self.onboarding_brand()))
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
                                    .id("onboarding-body")
                                    .debug_selector(|| "onboarding-body".into())
                                    .flex_1()
                                    .min_h_0()
                                    .overflow_y_scroll()
                                    .child(
                                        div()
                                            .w_full()
                                            .min_h_full()
                                            .flex()
                                            .flex_col()
                                            .justify_center()
                                            .px(if compact { px(24.) } else { px(48.) })
                                            .py_6()
                                            .child(
                                                div()
                                                    .w_full()
                                                    .max_w(px(480.))
                                                    .mx_auto()
                                                    .child(body),
                                            ),
                                    ),
                            ),
                    ),
            )
            .child(self.onboarding_footer(compact, cx))
            .into_any_element()
    }

    fn onboarding_brand(&self) -> impl IntoElement + use<> {
        let current = self.shell.onboarding.step.index();
        div()
            .w(relative(0.34))
            .flex_shrink_0()
            .h_full()
            .px_8()
            .py_6()
            .bg(rgb(self.palette.surface))
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .justify_center()
                    .gap_5()
                    .child(super::shell::brand(120.))
                    .child(
                        div()
                            .text_size(px(28.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child("Make yourself")
                            .child(div().child("at home.")),
                    )
                    .child(self.setup_caption("A little setup. A whole new world.")),
            )
            .child(
                div().flex().flex_col().gap_3().children(
                    ["Your game", "Your account", "Ready to go"]
                        .into_iter()
                        .enumerate()
                        .map(|(index, label)| {
                            div()
                                .flex()
                                .items_center()
                                .gap_3()
                                .text_sm()
                                .text_color(rgb(if index == current {
                                    self.palette.foreground
                                } else {
                                    self.palette.muted
                                }))
                                .child(
                                    div()
                                        .size(px(22.))
                                        .rounded_full()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .border_1()
                                        .border_color(rgb(if index == current {
                                            self.palette.primary
                                        } else {
                                            self.palette.border
                                        }))
                                        .bg(rgb(if index == current {
                                            self.palette.primary
                                        } else {
                                            self.palette.surface
                                        }))
                                        .text_color(rgb(if index == current {
                                            self.palette.on_primary
                                        } else {
                                            self.palette.muted
                                        }))
                                        .text_xs()
                                        .child(if index < current {
                                            "✓".to_owned()
                                        } else {
                                            (index + 1).to_string()
                                        }),
                                )
                                .child(label)
                        }),
                ),
            )
    }

    fn setup_caption(&self, text: impl Into<gpui::SharedString>) -> gpui::Div {
        div()
            .text_sm()
            .text_color(rgb(self.palette.muted))
            .child(text.into())
    }

    fn setup_heading(&self, eyebrow: &str, title: &str, description: &str) -> gpui::Div {
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(self.palette.muted))
                    .child(eyebrow.to_owned()),
            )
            .child(
                div()
                    .text_size(px(28.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(title.to_owned()),
            )
            .child(self.setup_caption(description.to_owned()))
    }

    fn onboarding_game(&self, form: &InstanceForm, cx: &Context<'_, Self>) -> gpui::AnyElement {
        if let Some(page) = self.shell.wizard.page {
            return match page {
                WizardPage::Version => self.wizard_version(form, cx).into_any_element(),
                WizardPage::Loader => self.wizard_loader(form, cx).into_any_element(),
                WizardPage::Location => self.wizard_location(form, cx).into_any_element(),
            };
        }
        div()
            .flex()
            .flex_col()
            .gap_6()
            .child(self.setup_heading(
                "01 / YOUR GAME",
                "Your first Minecraft.",
                "Start with a name. We’ll take care of the setup.",
            ))
            .child(field("Game name", &form.name))
            .child(self.setup_option(
                "wizard-component-version",
                "Minecraft",
                &form.version.read(cx).value(),
                WizardPage::Version,
                cx,
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        self.button(
                            "onboarding-options",
                            if self.shell.onboarding.options {
                                "Fewer options"
                            } else {
                                "More options"
                            },
                            UiAction::OnboardingOptions,
                            cx,
                        )
                        .bg(gpui::transparent_black())
                        .border_color(gpui::transparent_black())
                        .px_0()
                        .py_0()
                        .text_color(rgb(self.palette.muted)),
                    )
                    .children(self.shell.onboarding.options.then(|| {
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(self.setup_option(
                                "wizard-component-loader",
                                "Mod loader",
                                if form.neoforge {
                                    "NeoForge 20.6.144"
                                } else {
                                    "Vanilla"
                                },
                                WizardPage::Loader,
                                cx,
                            ))
                            .child(self.setup_option(
                                "wizard-component-location",
                                "Game folder",
                                if form.directory.read(cx).value().is_empty() {
                                    "Automatic"
                                } else {
                                    "Custom folder"
                                },
                                WizardPage::Location,
                                cx,
                            ))
                    })),
            )
            .child(self.setup_caption("Java and memory are configured automatically."))
            .into_any_element()
    }

    fn setup_option(
        &self,
        id: &str,
        title: &str,
        value: &str,
        page: WizardPage,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement + use<> {
        self.button(id.to_owned(), "", UiAction::WizardPage(Some(page)), cx)
            .w_full()
            .min_h(px(64.))
            .p_4()
            .bg(gpui::transparent_black())
            .flex()
            .items_center()
            .gap_3()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(self.setup_caption(title.to_owned()))
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .child(value.to_owned()),
                    ),
            )
            .child(div().text_color(rgb(self.palette.muted)).child("›"))
    }

    fn onboarding_account(&self, cx: &Context<'_, Self>) -> gpui::AnyElement {
        let selected = self
            .accounts
            .iter()
            .find(|account| Some(account_id(account)) == self.selected_account);
        let mut body = div().flex().flex_col().gap_6().child(self.setup_heading(
            "02 / YOUR ACCOUNT",
            "Who’s playing?",
            "Use the Microsoft account you use for Minecraft.",
        ));
        if let Some(account) = selected {
            body = body.child(
                div()
                    .rounded_xl()
                    .bg(rgb(self.palette.surface))
                    .p_5()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(self.setup_caption("Account ready"))
                    .child(div().text_xl().child(account.player_name.clone()))
                    .child(self.setup_caption("Microsoft · Minecraft: Java Edition")),
            );
        } else {
            body = body.child(
                div()
                    .rounded_xl()
                    .bg(rgb(self.palette.surface))
                    .p_5()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(microsoft_mark())
                    .child(
                        div()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .child(if self.busy {
                                "Continue in your browser"
                            } else {
                                "Your worlds. Your account."
                            }),
                    )
                    .child(self.setup_caption(if self.busy {
                        "Block will continue when sign-in is complete."
                    } else {
                        "Sign in securely in your browser."
                    }))
                    .child(if self.busy {
                        self.button(
                            "onboarding-cancel-sign-in",
                            "Cancel sign-in",
                            UiAction::Stop,
                            cx,
                        )
                    } else {
                        self.setup_primary(
                            "onboarding-sign-in",
                            "Sign in with Microsoft",
                            UiAction::SignIn,
                            cx,
                        )
                    }),
            );
        }
        if !self.busy {
            body = body.children(
                self.accounts
                    .iter()
                    .filter(|account| Some(account_id(account)) != self.selected_account)
                    .map(|account| {
                        self.button(
                            format!("onboarding-account-{}", account_id(account)),
                            account.player_name.clone(),
                            UiAction::SelectAccount(account_id(account)),
                            cx,
                        )
                    }),
            );
            if selected.is_none() {
                body = body.child(
                    self.button(
                        "onboarding-account-later",
                        "I’ll sign in later",
                        UiAction::OnboardingAccountLater,
                        cx,
                    )
                    .bg(gpui::transparent_black())
                    .border_color(gpui::transparent_black())
                    .px_0()
                    .text_color(rgb(self.palette.muted)),
                );
            }
        }
        body.into_any_element()
    }

    fn onboarding_review(&self, form: &InstanceForm, cx: &Context<'_, Self>) -> gpui::AnyElement {
        let account = self
            .accounts
            .iter()
            .find(|account| Some(account_id(account)) == self.selected_account);
        let rows = [
            ("Game", form.name.read(cx).value()),
            (
                "Minecraft",
                format!(
                    "{} · {}",
                    form.version.read(cx).value(),
                    if form.neoforge { "NeoForge" } else { "Vanilla" }
                ),
            ),
            (
                "Account",
                account.map_or_else(
                    || "Sign in later".to_owned(),
                    |account| account.player_name.clone(),
                ),
            ),
        ];
        div()
            .flex()
            .flex_col()
            .gap_6()
            .child(self.setup_heading(
                "03 / READY TO GO",
                "A world of possibilities.",
                "Your game is ready to be created.",
            ))
            .child(
                div()
                    .rounded_xl()
                    .border_1()
                    .border_color(rgb(self.palette.border))
                    .px_4()
                    .children(rows.into_iter().enumerate().map(|(index, (title, value))| {
                        let mut row = div()
                            .py_4()
                            .flex()
                            .gap_4()
                            .items_center()
                            .child(self.setup_caption(title).flex_shrink_0().w(px(88.)))
                            .child(div().flex_1().min_w_0().text_sm().child(value));
                        if index != 0 {
                            row = row.border_t_1().border_color(rgb(self.palette.border));
                        }
                        row
                    })),
            )
            .child(self.setup_caption(if account.is_some() {
                "We’ll download the game in the background. Your worlds start on the next screen."
            } else {
                "We’ll download the game in the background. Sign in before you play."
            }))
            .into_any_element()
    }

    fn setup_primary(
        &self,
        id: &str,
        label: &str,
        action: UiAction,
        cx: &Context<'_, Self>,
    ) -> gpui::Stateful<gpui::Div> {
        self.button(id.to_owned(), label.to_owned(), action, cx)
            .h(px(38.))
            .px_4()
            .bg(rgb(self.palette.primary))
            .border_color(rgb(self.palette.primary))
            .text_color(rgb(self.palette.on_primary))
            .flex()
            .items_center()
            .justify_center()
    }

    fn onboarding_footer(&self, compact: bool, cx: &Context<'_, Self>) -> gpui::AnyElement {
        let step = self.shell.onboarding.step;
        let subpage = self.shell.wizard.page.is_some();
        let created = self.onboarding_progress == Progress::Created;
        let has_account = self
            .accounts
            .iter()
            .any(|account| Some(account_id(account)) == self.selected_account);
        let blocked = !created && (self.busy || (step == Step::Account && !has_account));
        let label = if created {
            "Finish setup"
        } else if subpage {
            "Done"
        } else if step == Step::Review {
            if compact {
                "Create game"
            } else {
                "Create and download"
            }
        } else {
            "Continue"
        };
        let mut footer = div()
            .id("onboarding-footer")
            .debug_selector(|| "onboarding-footer".into())
            .h(if compact { px(104.) } else { px(72.) })
            .flex_shrink_0()
            .px(if compact { px(12.) } else { px(24.) })
            .border_t_1()
            .border_color(rgb(self.palette.border))
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .child(if self.busy && step == Step::Review && !created {
                div()
                    .id("onboarding-skip-disabled")
                    .px_3()
                    .py_2()
                    .text_color(rgb(self.palette.muted))
                    .opacity(0.4)
                    .child("Skip setup")
                    .into_any_element()
            } else {
                self.button(
                    "onboarding-skip",
                    "Skip setup",
                    UiAction::OnboardingSkip,
                    cx,
                )
                .px(if compact { px(4.) } else { px(12.) })
                .bg(gpui::transparent_black())
                .border_color(gpui::transparent_black())
                .text_color(rgb(self.palette.muted))
                .into_any_element()
            })
            .child(
                div()
                    .flex_1()
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap_2()
                    .children((step != Step::Game || subpage).then(|| {
                        if self.busy || created {
                            div()
                                .px_3()
                                .py_2()
                                .opacity(0.4)
                                .child("Back")
                                .into_any_element()
                        } else {
                            self.button("welcome-back", "Back", UiAction::OnboardingBack, cx)
                                .px_2()
                                .bg(gpui::transparent_black())
                                .border_color(gpui::transparent_black())
                                .into_any_element()
                        }
                    }))
                    .child(self.onboarding_next_action(label, blocked, cx)),
            );
        if compact {
            footer = footer.flex_col().items_start().py_2().gap_1();
        }
        footer.into_any_element()
    }

    fn onboarding_next_action(
        &self,
        label: &str,
        blocked: bool,
        cx: &Context<'_, Self>,
    ) -> gpui::AnyElement {
        if !blocked {
            return self
                .setup_primary("welcome-next", label, UiAction::OnboardingNext, cx)
                .into_any_element();
        }
        div()
            .id("onboarding-next-disabled")
            .debug_selector(|| "onboarding-next-disabled".into())
            .h(px(38.))
            .px_4()
            .rounded_md()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(self.palette.primary))
            .text_color(rgb(self.palette.on_primary))
            .opacity(0.35)
            .child(if self.busy && self.shell.onboarding.step == Step::Review {
                "Creating…".to_owned()
            } else {
                label.to_owned()
            })
            .into_any_element()
    }

    pub(super) fn advance_onboarding(&mut self, cx: &Context<'_, Self>) {
        if self.tab != Tab::Onboarding {
            return;
        }
        if self.onboarding_progress == Progress::Created {
            self.complete_onboarding();
            return;
        }
        if self.busy {
            return;
        }
        self.shell.onboarding.error = None;
        if self.shell.wizard.page.is_some() {
            if self.validate_instance_draft(cx) {
                self.shell.wizard.open_page(None);
            }
            return;
        }
        match self.shell.onboarding.step {
            Step::Game => {
                if !self.validate_instance_draft(cx) {
                    return;
                }
                self.shell.onboarding.step = Step::Account;
            }
            Step::Account => self.shell.onboarding.step = Step::Review,
            Step::Review => {
                self.save_instance(cx);
                return;
            }
        }
        self.shell.wizard.backwards = false;
        self.shell.wizard.revision = self.shell.wizard.revision.wrapping_add(1);
    }

    pub(super) fn back_onboarding(&mut self) {
        if self.busy || self.onboarding_progress == Progress::Created {
            return;
        }
        self.shell.onboarding.error = None;
        if self.shell.wizard.page.is_some() {
            self.shell.wizard.open_page(None);
        } else {
            self.shell.onboarding.step = match self.shell.onboarding.step {
                Step::Review => Step::Account,
                Step::Account | Step::Game => Step::Game,
            };
            self.shell.wizard.backwards = true;
            self.shell.wizard.revision = self.shell.wizard.revision.wrapping_add(1);
        }
    }

    pub(super) fn complete_onboarding(&mut self) -> bool {
        let mut preferences = self.preferences.clone();
        preferences.onboarding_complete = true;
        let Some(engine) = &self.engine else {
            self.report(
                "Setup could not be saved. Check Block's data directory and restart.".into(),
            );
            return false;
        };
        if let Err(error) = preferences.save(&engine.root) {
            self.report(format!("Could not finish setup: {error}"));
            return false;
        }
        self.saved_preferences = preferences.clone();
        self.preferences = preferences;
        self.tab = Tab::Home;
        self.form = None;
        self.pending_window_size = Some(gpui::size(px(432.), px(768.)));
        self.shell.revision = self.shell.revision.wrapping_add(1);
        true
    }
}

fn microsoft_mark() -> impl IntoElement {
    div()
        .size(px(28.))
        .flex_shrink_0()
        .flex()
        .flex_col()
        .gap(px(4.))
        .children(
            [[0x00f2_5022, 0x007f_ba00], [0x0000_a4ef, 0x00ff_b900]].map(|colors| {
                div()
                    .flex()
                    .gap(px(4.))
                    .children(colors.map(|color| div().size(px(12.)).bg(rgb(color))))
            }),
        )
}
