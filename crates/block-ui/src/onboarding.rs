//! Three honest setup steps share component drafts with the game wizard.
#![expect(
    clippy::redundant_pub_crate,
    reason = "First-run state is shared only with the parent view."
)]
use super::scroll_area::ScrollAreaExt as _;
use std::{cell::Cell, rc::Rc};

use gpui::{
    Context, FocusHandle, Focusable as _, InteractiveElement as _, IntoElement, ParentElement as _,
    ScrollHandle, Styled as _, Window, div, point, px, relative, rgb,
};

use super::game_wizard::{ProfileDraft, WizardField, WizardPage, WizardState};
use super::{
    Block, Form, InstanceForm, Tab, UiAction, account_id,
    components::{
        self, AvatarState, BadgeVariant, ButtonSize, ButtonVariant, ControlState, FieldOptions,
        ProgressState,
    },
    design::BROWSER_FRACTION,
    input, patterns,
    shell::Surface,
};

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

#[derive(Clone, Debug, Default)]
pub(super) struct State {
    pub step: Step,
    pub error: Option<String>,
    pub account: Option<String>,
    /// A verified identity kept in memory until the final setup commit saves it
    /// together with the game. Nothing reaches secure storage before that.
    pub pending_identity: Option<block_auth::Account>,
    pub account_decided: bool,
    scroll: ScrollHandle,
    heading_focus: Option<FocusHandle>,
    error_field: Option<WizardField>,
    reveal: Rc<Cell<Option<WizardField>>>,
}

impl State {
    pub(super) fn snapshot(&self, cx: &Context<'_, Block>) -> Self {
        let scroll = ScrollHandle::new();
        scroll.set_offset(self.scroll.offset());
        Self {
            step: self.step,
            error: self.error.clone(),
            account: self.account.clone(),
            pending_identity: self.pending_identity.clone(),
            account_decided: self.account_decided,
            scroll,
            heading_focus: Some(cx.focus_handle()),
            error_field: self.error_field,
            reveal: Rc::new(Cell::new(None)),
        }
    }
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
        self.shell.onboarding = State {
            account: self.selected_account.clone(),
            heading_focus: Some(cx.focus_handle()),
            ..State::default()
        };
        self.shell.wizard = WizardState::new(cx);
        self.shell.wizard.profile = Some(ProfileDraft::new(&self.saved_preferences.game, cx));
        self.shell.wizard.include_snapshots = self.saved_preferences.game.show_snapshots;
        let version = self.versions.first().map_or("1.20.6", String::as_str);
        self.form = Some(Form::Instance(InstanceForm {
            name: input("My Minecraft", "Game name", cx),
            version: input(version, "Minecraft version", cx),
            directory: input("", "Automatic installation location", cx),
            loader: block_launcher::catalog::LoaderKind::Vanilla,
            loader_version: None,
            optifine: None,
        }));
    }

    pub(super) fn onboarding(&self, window: &Window, cx: &Context<'_, Self>) -> gpui::AnyElement {
        let compact = patterns::is_narrow(window.viewport_size());
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
                    .children((!compact).then(|| self.onboarding_brand(cx)))
                    .child(div().flex_1().min_w_0().min_h_0().h_full().child(
                        self.transition_content(
                            super::motion::Scope::Wizard,
                            self.render_onboarding_transition_body(window, cx),
                            window,
                            cx,
                        ),
                    )),
            )
            .child(self.onboarding_footer(window, cx))
            .into_any_element()
    }

    pub(super) fn render_onboarding_transition_body(
        &self,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> gpui::AnyElement {
        let compact = patterns::is_narrow(window.viewport_size());
        let step = self.shell.onboarding.step;
        let mut body = match (&self.form, step) {
            (Some(Form::Instance(form)), Step::Game) => self.onboarding_game(form, compact, cx),
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
                        .border_color(rgb(self.palette.destructive))
                        .text_color(rgb(self.palette.destructive))
                        .p_3()
                        .text_sm()
                        .child(error.clone()),
                )
                .into_any_element();
        }
        div()
            .id("onboarding-body")
            .debug_selector(|| "onboarding-body".into())
            .size_full()
            .min_h_0()
            .scroll_area(
                &self.shell.onboarding.scroll,
                "Setup steps",
                self.palette,
                false,
            )
            .child(
                div()
                    .id("onboarding-content")
                    .debug_selector(|| "onboarding-content".into())
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
                            .max_w(px(620.))
                            .mx_auto()
                            .children(compact.then(|| {
                                div()
                                    .mb_5()
                                    .flex()
                                    .items_center()
                                    .gap_3()
                                    .child(super::shell::brand(56.))
                                    .child(
                                        div()
                                            .min_w_0()
                                            .child(self.setup_caption("WELCOME TO BLOCK"))
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .child("A little setup. A whole new world."),
                                            ),
                                    )
                            }))
                            .child(body),
                    ),
            )
            .into_any_element()
    }

    fn onboarding_brand(&self, cx: &gpui::App) -> impl IntoElement + use<> {
        let ready = [
            self.onboarding_game_ready(cx),
            self.shell.onboarding.account_decided || self.shell.onboarding.account.is_some(),
            self.onboarding_progress == Progress::Created,
        ];
        let steps = [
            components::StepperStep {
                title: "Your game",
                description: "Choose your components",
                complete: ready[0],
            },
            components::StepperStep {
                title: "Your account",
                description: "Choose who is playing",
                complete: ready[1],
            },
            components::StepperStep {
                title: "Ready to go",
                description: "Review and create",
                complete: ready[2],
            },
        ];
        div()
            .w(relative(BROWSER_FRACTION))
            .flex_shrink_0()
            .h_full()
            .min_h_0()
            .px_8()
            .py_6()
            .bg(rgb(self.palette.sidebar))
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
            .child(components::stepper(
                "onboarding-steps",
                "Welcome setup",
                &steps,
                self.shell.onboarding.step.index(),
                false,
                self.palette,
            ))
    }

    fn setup_caption(&self, text: impl Into<gpui::SharedString>) -> gpui::Div {
        div()
            .text_sm()
            .text_color(rgb(self.palette.muted))
            .child(text.into())
    }

    fn setup_heading(
        &self,
        eyebrow: &str,
        title: &str,
        description: &str,
    ) -> gpui::Stateful<gpui::Div> {
        let mut heading = div()
            .id("onboarding-step-heading")
            .debug_selector(|| "onboarding-step-heading".into());
        if let Some(focus) = &self.shell.onboarding.heading_focus {
            heading = heading.track_focus(focus);
        }
        heading
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

    fn onboarding_game(
        &self,
        form: &InstanceForm,
        compact: bool,
        cx: &Context<'_, Self>,
    ) -> gpui::AnyElement {
        let version = form.version.read(cx).value();
        let version_kind = self
            .version_catalog
            .iter()
            .find(|entry| entry.id == version.trim())
            .map_or("Exact version ID", |entry| {
                Self::wizard_version_kind(entry.kind)
            });
        let configuration = self.wizard_configuration_summary(cx);
        let name = patterns::reveal_field(
            components::field_with(
                "Game name",
                &form.name,
                self.palette,
                FieldOptions {
                    required: true,
                    error: self
                        .shell
                        .onboarding
                        .error
                        .as_ref()
                        .filter(|_| {
                            self.shell.onboarding.error_field == Some(WizardField::GameName)
                        })
                        .cloned()
                        .map(Into::into),
                    ..FieldOptions::default()
                },
            ),
            self.shell.onboarding.scroll.clone(),
            self.shell.onboarding.reveal.clone(),
            WizardField::GameName,
        )
        .id("onboarding-game-name")
        .debug_selector(|| "onboarding-game-name".into());
        div().flex().flex_col().gap_6()
            .child(self.setup_heading("01 / YOUR GAME", "Your first Minecraft.", "Start with a name. Choose components now or keep the defaults."))
            .children(self.onboarding_game_ready(cx).then(|| components::badge("Game configuration ready", self.palette, BadgeVariant::Success)))
            .child(name)
            .child(div().id("onboarding-components").debug_selector(|| "onboarding-components".into()).grid().grid_cols(if compact { 2 } else { 4 }).gap_3()
                .child(self.wizard_component("version", "Minecraft", (&version, version_kind), WizardPage::Version, cx))
                .child(self.wizard_component("loader", "Mod loader", (form.loader.name(), form.loader_version.as_deref().unwrap_or("Original game")), WizardPage::Loader, cx))
                .child(self.wizard_optifine_component(form, cx))
                .child(self.wizard_component("location", "Configuration", (&configuration.0, &configuration.1), WizardPage::Location, cx)))
            .child(self.setup_caption("Your choices remain a draft until you create the game. Java and required files are downloaded from verified sources.").id("onboarding-draft-note").debug_selector(|| "onboarding-draft-note".into()))
            .into_any_element()
    }

    fn onboarding_account(&self, cx: &Context<'_, Self>) -> gpui::AnyElement {
        let selected = self.onboarding_identity();
        let pending = self.shell.onboarding.pending_identity.is_some();
        let mut body = div().flex().flex_col().gap_6().child(self.setup_heading(
            "02 / YOUR ACCOUNT",
            "Who’s playing?",
            "Choose the account you want to play with.",
        ));
        if let Some(account) = selected {
            body = body.child(
                components::card(self.palette)
                    .child(div().flex().items_center().gap_3()
                        .child(components::avatar_fallback(&account.player_name, self.palette, 40., AvatarState::Unavailable))
                        .child(div().flex_1().min_w_0().child(div().text_lg().child(account.player_name.clone()))
                            .child(self.setup_caption(self.account_authority(account)))))
                    .children((!pending).then(|| components::badge("Account saved", self.palette, BadgeVariant::Success)))
                    .children(pending.then(|| components::badge("Ready to save", self.palette, BadgeVariant::Primary)))
                    .child(self.setup_caption(if pending {
                        "Verified and kept in memory. Finishing setup saves this account together with your game; leaving before then saves nothing."
                    } else {
                        "This account is saved on your device. Leaving setup keeps the saved account."
                    })),
            );
        } else {
            body = body.child(
                components::card(self.palette)
                    .child(microsoft_mark())
                    .children(self.busy.then(|| {
                        components::avatar_fallback("", self.palette, 32., AvatarState::Loading)
                    }))
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
                        "Complete account setup in its window. Setup saves the account when you create your game."
                    } else {
                        "Sign in with Microsoft or add a local offline identity."
                    }))
                    .children(self.busy.then(|| self.setup_caption(self.status.clone())))
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
            body = body.child(self.onboarding_account_actions(selected.is_some(), cx));
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

    fn onboarding_account_actions(
        &self,
        has_selection: bool,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement + use<> {
        div()
            .flex()
            .flex_wrap()
            .gap_2()
            .children((!self.accounts.is_empty()).then(|| {
                components::button(
                    "onboarding-account-different",
                    "Use a different account",
                    self.palette,
                    ButtonVariant::Outline,
                    ControlState::default(),
                    cx.listener(|view, &(): &(), window, cx| {
                        view.handle(UiAction::OnboardingChooseAccount, window, cx);
                    }),
                )
            }))
            .children(has_selection.then(|| {
                components::button(
                    "onboarding-account-add",
                    "Add an account",
                    self.palette,
                    ButtonVariant::Ghost,
                    ControlState::default(),
                    cx.listener(|view, &(): &(), window, cx| {
                        view.handle(UiAction::SignIn, window, cx);
                    }),
                )
            }))
    }

    fn onboarding_review(&self, form: &InstanceForm, cx: &Context<'_, Self>) -> gpui::AnyElement {
        let created = self.onboarding_progress == Progress::Created;
        let pending = self.shell.onboarding.pending_identity.is_some();
        let account = self.onboarding_identity();
        let rows = self.onboarding_review_rows(form, cx);
        div().flex().flex_col().gap_6()
            .child(self.setup_heading("03 / READY TO GO", "A world of possibilities.", if created { "Your game has been saved. Finish to return to Home." } else { "Review your choices. Each Change button returns to its draft." }))
            .child(components::card(self.palette).py_0().px_4().child(
                div().flex().flex_wrap().gap_3().children(rows.into_iter().enumerate().map(|(index, (title, value, action))| {
                    // Ten review rows do not fit the fixed footer in one column;
                    // a wrapped grid keeps every value visible without scrolling.
                    div().w(px(248.)).min_w_0().py_2().flex().flex_col().gap_1()
                        .child(div().flex().items_center().justify_between().gap_2()
                            .child(self.setup_caption(title).min_w_0())
                            .children((!created).then(|| components::button_sized(
                                format!("onboarding-review-change-{index}"), "Change", self.palette, ButtonVariant::Ghost, ButtonSize::Small,
                                ControlState { disabled: self.busy, ..ControlState::default() },
                                cx.listener(move |view, &(): &(), window, cx| view.handle(action.clone(), window, cx)),
                            ))))
                        .child(div().text_sm().whitespace_normal().child(value))
                })),
            ))
            .children(created.then(|| components::badge("Game saved", self.palette, BadgeVariant::Primary)))
            .children((self.busy && created).then(|| components::progress(ProgressState::Indeterminate, self.palette)))
            .children(self.busy.then(|| self.setup_caption(self.status.clone())))
            .child(self.setup_caption(if pending {
                "Creating saves this new account and the game together. If the game cannot be saved, the account is removed again. Downloads can be retried from Home."
            } else if account.is_some() {
                "Your game catalog and setup preference are saved in separate steps. Downloads can be retried from Home if they fail."
            } else {
                "Your game can download without an account. Sign in before you play; download errors can be retried from Home."
            })).into_any_element()
    }

    fn onboarding_review_rows(
        &self,
        form: &InstanceForm,
        cx: &Context<'_, Self>,
    ) -> [(&'static str, String, UiAction); 10] {
        let account = self.onboarding_identity();
        let profile = self.shell.wizard.profile.as_ref();
        let directory = form.directory.read(cx).value();
        let runtime = profile.map_or_else(
            || "Managed Java".into(),
            |profile| {
                if profile.managed {
                    "Managed Java".into()
                } else {
                    profile.java.read(cx).value()
                }
            },
        );
        let memory = profile.map_or_else(
            || "Default".into(),
            |profile| {
                profile.memory.read(cx).value().parse::<u32>().map_or_else(
                    |_| format!("{} MiB", profile.memory.read(cx).value()),
                    components::memory_label,
                )
            },
        );
        let window = profile.map_or_else(
            || "Default".into(),
            |profile| {
                format!(
                    "{} × {}{}",
                    profile.width.read(cx).value(),
                    profile.height.read(cx).value(),
                    if profile.fullscreen {
                        " · Fullscreen"
                    } else {
                        " · Windowed"
                    }
                )
            },
        );
        [
            (
                "Game",
                form.name.read(cx).value(),
                UiAction::OnboardingChange(Step::Game),
            ),
            (
                "Initial Profile",
                form.name.read(cx).value(),
                UiAction::OnboardingChange(Step::Game),
            ),
            (
                "Minecraft",
                form.version.read(cx).value(),
                UiAction::OnboardingSection(WizardPage::Version),
            ),
            (
                "Mod loader",
                form.loader_label(),
                UiAction::OnboardingSection(WizardPage::Loader),
            ),
            (
                "OptiFine",
                form.optifine.as_ref().map_or_else(
                    || "Not selected".into(),
                    |selected| selected.version.clone(),
                ),
                UiAction::OnboardingSection(WizardPage::OptiFine),
            ),
            (
                "Game folder",
                if directory.is_empty() {
                    "Automatic".into()
                } else {
                    directory
                },
                UiAction::OnboardingSection(WizardPage::Location),
            ),
            (
                "Java",
                runtime,
                UiAction::OnboardingSection(WizardPage::Location),
            ),
            (
                "Memory",
                memory,
                UiAction::OnboardingSection(WizardPage::Location),
            ),
            (
                "Window",
                window,
                UiAction::OnboardingSection(WizardPage::Location),
            ),
            (
                "Account",
                account.map_or_else(
                    || "Sign in later".into(),
                    |account| account.player_name.clone(),
                ),
                UiAction::OnboardingChange(Step::Account),
            ),
        ]
    }

    fn setup_primary(
        &self,
        id: &str,
        label: &str,
        action: UiAction,
        cx: &Context<'_, Self>,
    ) -> components::Control {
        components::button_sized(
            id.to_owned(),
            label.to_owned(),
            self.palette,
            ButtonVariant::Primary,
            ButtonSize::Large,
            ControlState {
                disabled: self.busy,
                ..ControlState::default()
            },
            cx.listener(move |view, &(): &(), window, cx| view.handle(action.clone(), window, cx)),
        )
    }

    fn onboarding_footer(&self, window: &Window, cx: &Context<'_, Self>) -> gpui::AnyElement {
        let step = self.shell.onboarding.step;
        let compact = patterns::is_narrow(window.viewport_size());
        let created = self.onboarding_progress == Progress::Created;
        // A verified identity kept in memory counts as the step-2 choice; it is
        // persisted only by the final setup commit.
        let has_account = self.onboarding_identity().is_some()
            || self
                .accounts
                .iter()
                .any(|account| Some(account_id(account)) == self.shell.onboarding.account);
        let blocked = !created && (self.busy || (step == Step::Account && !has_account));
        let creating = self.busy && step == Step::Review && !created;
        let label = if created {
            "Finish setup"
        } else if creating {
            "Creating…"
        } else if step == Step::Review {
            "Create game"
        } else {
            "Continue"
        };
        let skip_disabled = creating;
        let reason = self.onboarding_footer_reason(has_account, cx);

        patterns::management_footer(window.viewport_size(), self.palette)
            .id("onboarding-footer")
            .debug_selector(|| "onboarding-footer".into())
            .min_w_0()
            .flex_col()
            .justify_center()
            .items_start()
            .gap_1()
            .child(
                div()
                    .w_full()
                    .id("onboarding-footer-reason")
                    .debug_selector(|| "onboarding-footer-reason".into())
                    .text_xs()
                    .text_color(rgb(if self.shell.onboarding.error.is_some() {
                        self.palette.destructive
                    } else {
                        self.palette.muted
                    }))
                    .truncate()
                    .child(reason),
            )
            .child(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(if compact { 4. } else { 12. }))
                    .child(components::button_sized(
                        if skip_disabled {
                            "onboarding-skip-disabled"
                        } else {
                            "onboarding-skip"
                        },
                        if compact {
                            "Set up later"
                        } else {
                            "Skip setup"
                        },
                        self.palette,
                        ButtonVariant::Ghost,
                        ButtonSize::Small,
                        ControlState {
                            disabled: skip_disabled,
                            ..ControlState::default()
                        },
                        cx.listener(|view, &(): &(), window, cx| {
                            view.handle(UiAction::OnboardingSkip, window, cx);
                        }),
                    ))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(if compact { 4. } else { 8. }))
                            .children((step != Step::Game).then(|| {
                                components::button_sized(
                                    "welcome-back",
                                    "Back",
                                    self.palette,
                                    ButtonVariant::Ghost,
                                    ButtonSize::Small,
                                    ControlState {
                                        disabled: self.busy || created,
                                        ..ControlState::default()
                                    },
                                    cx.listener(|view, &(): &(), window, cx| {
                                        view.handle(UiAction::OnboardingBack, window, cx);
                                    }),
                                )
                            }))
                            .child(self.onboarding_advance_button(blocked, creating, label, cx)),
                    ),
            )
            .into_any_element()
    }

    fn onboarding_advance_button(
        &self,
        blocked: bool,
        creating: bool,
        label: &'static str,
        cx: &Context<'_, Self>,
    ) -> components::Control {
        components::button(
            if blocked {
                "onboarding-next-disabled"
            } else {
                "welcome-next"
            },
            label,
            self.palette,
            ButtonVariant::Primary,
            ControlState {
                disabled: blocked,
                busy: creating,
                invalid: self.shell.onboarding.error.is_some(),
            },
            cx.listener(|view, &(): &(), window, cx| {
                view.handle(UiAction::OnboardingNext, window, cx);
            }),
        )
    }

    fn onboarding_footer_reason(&self, has_account: bool, cx: &gpui::App) -> String {
        if self.onboarding_progress == Progress::Created {
            "Game saved · Finish setup when ready".into()
        } else if self.busy {
            self.status.clone()
        } else if let Some(error) = &self.shell.onboarding.error {
            error.clone()
        } else if self.shell.onboarding.step == Step::Account && !has_account {
            "Choose an account or select “I’ll sign in later”".into()
        } else if !self.onboarding_game_ready(cx) {
            "Review your game configuration before continuing".into()
        } else if self.shell.onboarding.step == Step::Review {
            "Ready to create · Downloads may continue after setup".into()
        } else {
            "Your choices stay in this draft".into()
        }
    }

    fn onboarding_game_ready(&self, cx: &gpui::App) -> bool {
        let Some(Form::Instance(form)) = &self.form else {
            return false;
        };
        let name = form.name.read(cx).value();
        let version = form.version.read(cx).value();
        let directory = form.directory.read(cx).value();
        !name.trim().is_empty()
            && name.len() <= 160
            && !name.chars().any(char::is_control)
            && !version.trim().is_empty()
            && version.len() <= 80
            && !version.chars().any(char::is_control)
            && self.wizard_loader_issue(cx).is_none()
            && (directory.trim().is_empty() || std::path::Path::new(directory.trim()).is_absolute())
            && self
                .shell
                .wizard
                .profile
                .as_ref()
                .is_some_and(|profile| profile.values(cx).is_ok())
    }

    pub(super) fn advance_onboarding(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
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
        self.shell.onboarding.error_field = None;
        match self.shell.onboarding.step {
            Step::Game => {
                if !self.validate_onboarding_game(window, cx) {
                    return;
                }
                self.change_onboarding_step(Step::Account, window, cx);
            }
            Step::Account => {
                self.shell.onboarding.account_decided = true;
                self.change_onboarding_step(Step::Review, window, cx);
            }
            Step::Review => self.submit_onboarding(window, cx),
        }
    }

    pub(super) fn change_onboarding_step(
        &mut self,
        step: Step,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.busy || self.onboarding_progress == Progress::Created {
            return;
        }
        self.shell.wizard.backwards = step.index() < self.shell.onboarding.step.index();
        self.motion.direction = if self.shell.wizard.backwards {
            super::motion::Direction::Back
        } else {
            super::motion::Direction::Forward
        };
        self.motion_dirty = true;
        self.shell.onboarding.step = step;
        self.shell.onboarding.error = None;
        self.shell.onboarding.error_field = None;
        self.shell
            .onboarding
            .scroll
            .set_offset(point(px(0.), px(0.)));
        self.shell.wizard.revision = self.shell.wizard.revision.wrapping_add(1);
        if let Some(focus) = &self.shell.onboarding.heading_focus {
            focus.focus(window, cx);
        }
        cx.notify();
    }

    pub(super) fn back_onboarding(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.shell.wizard.page.is_some() {
            self.finish_onboarding_section(true, window, cx);
            return;
        }
        let step = match self.shell.onboarding.step {
            Step::Review => Step::Account,
            Step::Account | Step::Game => Step::Game,
        };
        self.change_onboarding_step(step, window, cx);
    }

    pub(super) fn choose_onboarding_account(&mut self, id: String) {
        let pending = self
            .shell
            .onboarding
            .pending_identity
            .as_ref()
            .is_some_and(|account| account_id(account) == id);
        if pending
            || self
                .accounts
                .iter()
                .any(|account| account_id(account) == id)
        {
            if !pending && self.shell.onboarding.pending_identity.take().is_some() {
                // Choosing an already saved identity abandons the in-memory
                // preparation; its credentials were never persisted.
                if let Some(engine) = &self.engine {
                    engine.discard_prepared_sign_in();
                }
            }
            self.shell.onboarding.account = Some(id);
            self.shell.onboarding.account_decided = true;
            self.shell.onboarding.error = None;
        }
    }

    /// The identity this setup would use: a verified in-memory preparation
    /// first, otherwise an already saved account.
    pub(super) fn onboarding_identity(&self) -> Option<&block_auth::Account> {
        self.shell.onboarding.pending_identity.as_ref().or_else(|| {
            self.accounts
                .iter()
                .find(|account| Some(account_id(account)) == self.shell.onboarding.account)
        })
    }

    pub(super) fn open_onboarding_section(
        &mut self,
        page: WizardPage,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.busy || self.onboarding_progress == Progress::Created || self.tab != Tab::Onboarding
        {
            return;
        }
        if self
            .shell
            .surfaces
            .layers()
            .any(|entry| matches!(entry.kind, Surface::OnboardingSection(_)))
        {
            self.shell
                .surfaces
                .open(Surface::OnboardingSection(page), window, cx);
            return;
        }
        self.shell.onboarding.error = None;
        self.open_game_wizard_page(Some(page), cx);
        self.motion_dirty = true;
        self.shell
            .surfaces
            .open(Surface::OnboardingSection(page), window, cx);
        let input = match page {
            WizardPage::Version => self.shell.wizard.version_search.as_ref(),
            WizardPage::Location => match &self.form {
                Some(Form::Instance(form)) => Some(&form.directory),
                _ => None,
            },
            WizardPage::Loader | WizardPage::OptiFine => None,
        };
        if let Some(input) = input {
            input.read(cx).focus_handle(cx).focus(window, cx);
        }
    }

    pub(super) fn finish_onboarding_section(
        &mut self,
        cancel: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(page) = self
            .shell
            .surfaces
            .top()
            .and_then(|layer| match layer.kind {
                Surface::OnboardingSection(page) => Some(page),
                _ => None,
            })
        else {
            return;
        };
        if self.busy
            || (!cancel
                && self
                    .form_inputs()
                    .iter()
                    .any(|input| input.read(cx).is_composing()))
        {
            return;
        }
        if cancel {
            self.cancel_component_draft(cx);
        } else {
            if !self.validate_wizard_section(page, cx) {
                self.focus_wizard_error(window, cx);
                return;
            }
            self.open_game_wizard_page(None, cx);
        }
        self.shell.surfaces.close(window, &self.focus, cx);
        self.motion.direction = super::motion::Direction::Back;
        self.motion_dirty = true;
        cx.notify();
    }

    pub(super) fn render_onboarding_section(
        &self,
        page: WizardPage,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> gpui::AnyElement {
        let Some(Form::Instance(form)) = &self.form else {
            return div().into_any_element();
        };

        let content = match page {
            WizardPage::Version => self.wizard_version(form, cx).into_any_element(),
            WizardPage::Loader => self.wizard_loader(form, cx).into_any_element(),
            WizardPage::OptiFine => self.wizard_optifine(form, cx).into_any_element(),
            WizardPage::Location => self.wizard_configuration(form, cx).into_any_element(),
        };
        div()
            .id("onboarding-section")
            .debug_selector(|| "onboarding-section".into())
            .w_full()
            .min_h_0()
            .h_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .id("onboarding-section-scroll")
                    .debug_selector(|| "onboarding-section-scroll".into())
                    .min_h_0()
                    .flex_1()
                    .scroll_area(
                        &self.shell.wizard.body_scroll,
                        "Game configuration",
                        self.palette,
                        false,
                    )
                    .p_5()
                    .child(content)
                    .children(self.shell.wizard.error.as_ref().map(|error| {
                        div()
                            .id("onboarding-section-error")
                            .debug_selector(|| "onboarding-section-error".into())
                            .mt_3()
                            .text_sm()
                            .text_color(rgb(self.palette.destructive))
                            .child(error.clone())
                    })),
            )
            .child(
                patterns::management_footer(window.viewport_size(), self.palette)
                    .id("onboarding-section-footer")
                    .debug_selector(|| "onboarding-section-footer".into())
                    .justify_end()
                    .child(components::button(
                        "onboarding-section-cancel",
                        "Cancel",
                        self.palette,
                        ButtonVariant::Ghost,
                        ControlState::default(),
                        cx.listener(|view, &(): &(), window, cx| {
                            view.handle(UiAction::OnboardingSectionCancel, window, cx);
                        }),
                    ))
                    .child(components::button(
                        "onboarding-section-done",
                        "Done",
                        self.palette,
                        ButtonVariant::Primary,
                        ControlState::default(),
                        cx.listener(|view, &(): &(), window, cx| {
                            view.handle(UiAction::OnboardingSectionDone, window, cx);
                        }),
                    )),
            )
            .into_any_element()
    }

    fn validate_onboarding_game(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        if self.validate_instance_draft(cx) {
            return true;
        }
        let field = match &self.form {
            Some(Form::Instance(form))
                if form.name.read(cx).value().trim().is_empty()
                    || form.name.read(cx).value().len() > 160
                    || form.name.read(cx).value().chars().any(char::is_control) =>
            {
                WizardField::GameName
            }
            Some(Form::Instance(form)) if form.version.read(cx).value().trim().is_empty() => {
                WizardField::Version
            }
            _ if self.wizard_loader_issue(cx).is_some() => WizardField::Loader,
            _ => WizardField::Directory,
        };
        self.shell.onboarding.error = Some(self.status.clone());
        self.shell.onboarding.error_field = Some(field);
        if field == WizardField::GameName {
            self.shell.onboarding.step = Step::Game;
            self.shell.onboarding.reveal.set(Some(field));
            if let Some(Form::Instance(form)) = &self.form {
                form.name.read(cx).focus_handle(cx).focus(window, cx);
            }
        } else {
            let page = if field == WizardField::Version {
                WizardPage::Version
            } else if field == WizardField::Loader {
                WizardPage::Loader
            } else {
                WizardPage::Location
            };
            self.open_onboarding_section(page, window, cx);
            self.set_wizard_error(self.status.clone(), cx);
            self.focus_wizard_error(window, cx);
        }
        false
    }

    fn submit_onboarding(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if !self.validate_onboarding_game(window, cx) {
            return;
        }
        let Some(Form::Instance(form)) = &self.form else {
            return;
        };
        let name = form.name.read(cx).value();
        let version = form.version.read(cx).value();
        let directory = form.directory.read(cx).value();
        let loader = form.selected_loader();
        let optifine = form.optifine.as_deref().cloned();
        let Some(profile) = &self.shell.wizard.profile else {
            return;
        };
        let mut values = match profile.values(cx) {
            Ok(values) => values,
            Err(error) => {
                let error = error.to_string();
                self.open_onboarding_section(WizardPage::Location, window, cx);
                self.set_wizard_error(error, cx);
                self.focus_wizard_error(window, cx);
                return;
            }
        };
        values.name.clone_from(&name);
        values.target_id = None;
        let defaults = values.defaults.clone();
        // A verified but unsaved identity is committed here, in the same task
        // as the catalog entry, so a failure leaves neither half behind.
        let pending_account = self.shell.onboarding.pending_identity.clone();
        self.begin_cancellable("Creating your game and preparing verified downloads");
        self.mark_creation();
        if let Some(engine) = &self.engine {
            engine.create_instance_with_profile(
                pending_account,
                name,
                version,
                (!directory.trim().is_empty()).then(|| std::path::PathBuf::from(directory.trim())),
                loader,
                optifine,
                defaults,
                Some(values),
                self.cancel.clone(),
            );
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
        // Setup is complete: no in-memory identity remains, and any prepared
        // sign-in that was not committed is discarded.
        self.shell.onboarding.pending_identity = None;
        if let Some(engine) = &self.engine {
            engine.discard_prepared_sign_in();
        }
        self.selected_account = self.shell.onboarding.account.clone().filter(|id| {
            self.accounts
                .iter()
                .any(|account| account_id(account) == *id)
        });
        self.saved_preferences = preferences.clone();
        self.preferences = preferences;
        self.tab = Tab::Home;
        self.form = None;
        self.pending_window_size = Some(gpui::size(px(432.), px(768.)));
        self.shell.revision = self.shell.revision.wrapping_add(1);
        self.push_toast(
            "Setup saved · Continue on Home".into(),
            super::feedback::ToastKind::Success,
        );
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

#[cfg(test)]
#[path = "onboarding_tests.rs"]
mod tests;
