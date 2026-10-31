//! First-run task overview; completed Wizards own their individual saves.
#![expect(
    clippy::redundant_pub_crate,
    reason = "Private first-run presentation."
)]

use super::scroll_area::ScrollAreaExt as _;
use super::{
    Block, Form, Preferences, Tab, UiAction,
    components::{self, ButtonSize, ButtonVariant, ControlState},
    design::Icon,
    game_wizard::WizardStep,
    note,
    patterns::page_heading,
};
use crate::Language;
use gpui::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, ScrollHandle, Styled as _,
    Window, div, px, rgb,
};
use std::{
    path::{Path, PathBuf},
    time::Instant,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Stage {
    #[default]
    Welcome,
    Overview,
    GameChoice,
    GameWizard,
}

#[derive(Clone, Debug)]
pub(super) enum Action {
    Start,
    Continue,
    Account,
    Game,
    Recommended,
    Custom,
    Back,
    BackToWelcome,
    Import,
    Skip,
    /// A concrete language choice from the first-setup selector.
    Language(Language),
}

/// Product feedback retained by the first-run flow.
///
/// It is translated when the page renders, so an earlier language is never
/// frozen into the notice.
#[derive(Clone, Debug)]
pub(super) enum Notice {
    /// The installation-folder chooser could not be opened.
    InstallChooser,
    LanguageInitialization(String),
    VersionsUnavailable,
    SetupProgress(String),
    LanguageNotSaved(String),
}

impl Notice {
    fn message(&self, language: Language) -> String {
        match self {
            Self::InstallChooser => tr!(language, "setup.notice.install_chooser"),
            Self::LanguageInitialization(error) => tr!(
                language,
                "common.language_initialization_failed",
                error = error
            ),
            Self::VersionsUnavailable => tr!(language, "setup.notice.versions"),
            Self::SetupProgress(detail) => {
                tr!(language, "setup.notice.progress", detail = detail.as_str())
            }
            Self::LanguageNotSaved(detail) => {
                tr!(language, "setup.notice.language", detail = detail.as_str())
            }
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct State {
    /// Older component-only fixtures still exercise the shared draft helpers.
    pub enabled: bool,
    pub stage: Stage,
    pub welcome_button: super::welcome_button::WelcomeButton,
    pub game_parent: Option<PathBuf>,
    pub entered: Option<Instant>,
    pub error: Option<crate::i18n::UiText>,
    pub notice: Option<Notice>,
    pub scroll: ScrollHandle,
}

/// The game library chosen from the running install shape, with the data-root
/// fallback only when the platform app-data directory is unavailable.
fn automatic_game_parent(fallback: &Path) -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|executable| super::setup_storage::automatic_game_parent(&executable))
        .unwrap_or_else(|| fallback.to_owned())
}

impl Block {
    pub(super) fn begin_first_run(&mut self, cx: &mut Context<'_, Self>) {
        self.router.navigate(Tab::Onboarding);
        self.form = None;
        self.shell.first_run.enabled = true;
        let Some(engine) = &self.engine else {
            return;
        };
        self.shell.first_run.stage = if engine.root.join("setup-started").is_file() {
            Stage::Overview
        } else {
            Stage::Welcome
        };
        let fallback = engine.root.join("instances");
        let existing = block_launcher::storage::Locations::load(&engine.root).ok();
        self.shell.first_run.game_parent = Some(
            existing
                .and_then(|value| value.library)
                .unwrap_or_else(|| automatic_game_parent(&fallback)),
        );
        self.shell.onboarding.account = self.selected_account.clone();
        self.motion_dirty = true;
        cx.notify();
    }

    pub(super) fn first_run_account_ready(&self) -> bool {
        self.shell
            .onboarding
            .account
            .as_ref()
            .or(self.selected_account.as_ref())
            .is_some_and(|id| {
                self.accounts
                    .iter()
                    .any(|account| super::account_id(account) == *id)
            })
    }

    fn recommended_setup_version(&self) -> Option<&str> {
        self.version_catalog
            .iter()
            .find(|version| format!("{:?}", version.kind) == "Release")
            .map(|version| version.id.as_str())
            .or_else(|| {
                (self.version_catalog.is_empty() && !self.saved_preferences.game.show_snapshots)
                    .then(|| self.versions.first().map(String::as_str))
                    .flatten()
            })
    }

    pub(super) fn first_run_game_ready(&self) -> bool {
        !self.snapshot.profiles.is_empty()
    }

    pub(super) fn first_run_game_parent(&self) -> PathBuf {
        self.shell.first_run.game_parent.clone().unwrap_or_else(|| {
            self.engine.as_ref().map_or_else(
                || PathBuf::from("."),
                |engine| automatic_game_parent(&engine.root.join("instances")),
            )
        })
    }

    fn mark_first_run_started(&mut self) -> bool {
        let parent = self.first_run_game_parent();
        let result = self
            .engine
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Block's data directory is unavailable"))
            .and_then(|engine| {
                std::fs::create_dir_all(&engine.root)?;
                std::fs::write(engine.root.join("setup-started"), b"1")?;
                super::setup_storage::save_game_parent(&engine.root, &parent)?;
                Ok(())
            });
        if let Err(error) = result {
            self.shell.first_run.notice = Some(Notice::SetupProgress(format!("{error:#}")));
            return false;
        }
        true
    }

    /// Apply a first-setup language choice immediately.
    ///
    /// The preview is usable even when the write fails. Only a successful
    /// language-only write updates the saved state, so a failure is reported
    /// instead of being recorded as saved. Onboarding completion and any open
    /// game draft are deliberately untouched.
    fn choose_first_run_language(
        &mut self,
        language: Language,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.preferences.language = Some(language);
        self.refresh_language(window, cx);
        let result = match &self.engine {
            Some(engine) => Preferences::persist_language(&engine.root, language),
            None => Err(anyhow::anyhow!("Block's data directory is unavailable")),
        };
        match result {
            Ok(()) => self.saved_preferences.language = Some(language),
            Err(error) => {
                self.shell.first_run.notice = Some(Notice::LanguageNotSaved(format!("{error:#}")));
            }
        }
    }

    /// Record a failed installation-folder chooser on the first-run page.
    ///
    /// The retained failure is a semantic identity rather than displayed copy,
    /// so a later success clears exactly this failure. The shared `report` path
    /// still owns the transient toast, status and log; it also fills the generic
    /// first-run error string, which this replaces with the semantic notice.
    pub(super) fn record_install_chooser_failure(&mut self) {
        self.shell.first_run.notice = Some(Notice::InstallChooser);
        self.shell.first_run.error = None;
    }

    /// Clear the retained chooser failure only while it is still current.
    ///
    /// Returns whether the first-run page owned that failure, so a caller can
    /// reset presentation that belonged to the same failed chooser.
    pub(super) fn clear_install_chooser_failure(&mut self) -> bool {
        if matches!(self.shell.first_run.notice, Some(Notice::InstallChooser)) {
            self.shell.first_run.notice = None;
            self.shell.first_run.error = None;
            true
        } else {
            false
        }
    }

    /// The visible failure for the active first-run page.
    pub(super) fn first_run_feedback(&self, language: Language) -> Option<String> {
        self.shell
            .first_run
            .notice
            .as_ref()
            .map(|notice| notice.message(language))
            .or_else(|| {
                self.shell
                    .first_run
                    .error
                    .as_ref()
                    .map(|error| error.text(language))
            })
    }

    pub(super) fn first_run_overview(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.shell.wizard.cancel_loader_request();
        let leaving_welcome = self.shell.first_run.stage == Stage::Welcome;
        // Freeze the leaving Welcome page before the stage changes.
        let outgoing = leaving_welcome.then(|| std::rc::Rc::new(self.visual_snapshot(cx)));
        self.shell.first_run.stage = Stage::Overview;
        self.router.leave_task();
        self.shell.first_run.entered = None;
        self.motion_dirty = true;
        if let Some(outgoing) = outgoing.filter(|_| !self.motion_preferences().reduced()) {
            // Sequence: slide Welcome away, resize the frame, slide Overview in.
            self.start_first_run_switch(
                super::motion::Direction::Forward,
                outgoing,
                cx.background_executor().now(),
            );
            cx.notify();
        } else {
            self.request_frame_resize(window, cx);
        }
    }

    pub(super) fn handle_first_run(
        &mut self,
        action: Action,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if !self.shell.first_run.enabled || self.preferences.onboarding_complete {
            return;
        }
        self.shell.first_run.error = None;
        self.shell.first_run.notice = None;
        match action {
            Action::Language(language) => self.choose_first_run_language(language, window, cx),
            Action::Start => {
                if !self.mark_first_run_started() {
                    return;
                }
                self.start_welcome_import(window, cx);
            }
            Action::Continue => {
                if !self.first_run_account_ready() {
                    self.handle_first_run(Action::Account, window, cx);
                } else if !self.first_run_game_ready() {
                    self.handle_first_run(Action::Game, window, cx);
                } else if self.complete_onboarding() {
                    self.request_frame_resize(window, cx);
                    if self.profile().is_some_and(|profile| {
                        self.snapshot
                            .selection(&profile.id)
                            .is_ok_and(|(_, _, target)| {
                                !matches!(
                                    target.status,
                                    block_launcher::catalog::InstallStatus::Ready
                                )
                            })
                    }) {
                        self.handle(UiAction::Repair, window, cx);
                    }
                }
            }
            Action::Account => {
                self.shell.first_run.stage = Stage::Overview;
                self.router.leave_task();
                self.shell.first_run.entered = Some(cx.background_executor().now());
                self.open_auth_wizard(window, cx);
            }
            Action::Game => {
                self.shell.first_run.stage = Stage::GameChoice;
                self.router.enter_game_choice();
                self.shell.first_run.entered = Some(cx.background_executor().now());
                self.request_frame_resize(window, cx);
            }
            Action::Recommended | Action::Custom => {
                let recommended = matches!(action, Action::Recommended);
                if !recommended && matches!(self.form, Some(Form::Instance(_))) {
                    self.shell.first_run.stage = Stage::GameWizard;
                    self.router.enter_game_wizard();
                    return;
                }
                let version = self.recommended_setup_version().map(str::to_owned);
                if recommended && version.is_none() {
                    self.shell.first_run.notice = Some(Notice::VersionsUnavailable);
                    return;
                }
                self.begin_game_wizard(cx);
                self.router.navigate(Tab::Onboarding);
                self.router.enter_game_wizard();
                self.shell.wizard.separated = false;
                self.shell.wizard.step = if recommended {
                    WizardStep::Review
                } else {
                    WizardStep::Setup
                };
                self.shell.first_run.stage = Stage::GameWizard;
                self.shell.first_run.entered = Some(cx.background_executor().now());
                let directory = self
                    .first_run_game_parent()
                    .join(uuid::Uuid::new_v4().to_string());
                if let Some(Form::Instance(form)) = &self.form {
                    form.directory.update(cx, |field, cx| {
                        field.set_value(&directory.display().to_string(), cx)
                    });
                }
                if recommended && let Some(version) = version {
                    self.set_wizard_version(version, cx);
                }
                if recommended {
                    self.submit_game_wizard(cx);
                }
            }
            Action::Back => {
                self.shell.surfaces.clear(window, &self.focus, cx);
                self.first_run_overview(window, cx);
            }
            Action::BackToWelcome => {
                // Returning to Welcome is navigation, not a reset: `setup-started`
                // and the saved game parent keep the draft resumable.
                let outgoing = std::rc::Rc::new(self.visual_snapshot(cx));
                // The previous control was unmounted while hovered and may
                // never receive mouse-leave. Give the returning control fresh
                // hover/pointer state without changing the frozen snapshot.
                self.shell.first_run.welcome_button = Default::default();
                self.shell.first_run.stage = Stage::Welcome;
                self.router.leave_task();
                self.shell.first_run.entered = None;
                self.motion_dirty = true;
                if self.motion_preferences().reduced() {
                    self.request_frame_resize(window, cx);
                } else {
                    // Sequence: slide Overview away, shrink the frame, slide
                    // Welcome back in from the left.
                    self.start_first_run_switch(
                        super::motion::Direction::Back,
                        outgoing,
                        cx.background_executor().now(),
                    );
                    cx.notify();
                }
            }
            Action::Import => self.open_import_games(window, cx),
            Action::Skip => {
                if self.complete_onboarding() {
                    self.request_frame_resize(window, cx);
                }
            }
        }
        cx.notify();
    }

    pub(super) fn first_run_page(
        &self,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> gpui::AnyElement {
        if self.shell.first_run.stage == Stage::Welcome {
            return self.first_run_welcome(window, cx);
        }
        self.first_run_progress_page(cx)
    }

    /// Welcome is the only full-height page: the invitation control is centred
    /// and Skip setup sits at the bottom edge.
    fn first_run_welcome(&self, window: &Window, cx: &Context<'_, Self>) -> gpui::AnyElement {
        let language = self.language();
        let button_size = px(super::welcome_button::DIAMETER);
        div()
            .id("setup-welcome")
            .debug_selector(|| "setup-welcome".into())
            .relative()
            .size_full()
            .child(
                div()
                    .absolute()
                    .top(px(24.))
                    .left_0()
                    .right_0()
                    .flex()
                    .justify_center()
                    .child(components::button_sized(
                        "not-your-language",
                        "Not your language?",
                        self.palette,
                        ButtonVariant::Ghost,
                        ButtonSize::ExtraSmall,
                        ControlState::default(),
                        cx.listener(|view, (): &(), window, cx| {
                            view.handle(
                                UiAction::Drawer(Some(super::Drawer::Language)),
                                window,
                                cx,
                            );
                        }),
                    )),
            )
            .child(
                div()
                    .absolute()
                    .top(gpui::relative(0.52))
                    .mt(-button_size * 0.5)
                    .left_0()
                    .right_0()
                    .h(button_size)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .absolute()
                            .bottom(gpui::relative(1.))
                            .mb(px(32.))
                            .left_0()
                            .right_0()
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap(px(6.))
                            .child(
                                div()
                                    .text_2xl()
                                    .line_height(px(32.))
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .child(tr!(language, "setup.welcome.title")),
                            )
                            .child(
                                note(tr!(language, "setup.welcome.subtitle")).line_height(px(18.)),
                            ),
                    )
                    .child(super::welcome_button::render(
                        &self.shell.first_run.welcome_button,
                        self.palette,
                        self.motion_preferences(),
                        tr!(language, "setup.welcome.start"),
                        window,
                        cx,
                        cx.listener(|view, _, window, cx| {
                            view.handle(UiAction::FirstRun(Action::Start), window, cx)
                        }),
                    ))
                    .children(self.first_run_feedback(language).map(|message| {
                        div()
                            .absolute()
                            .top(gpui::relative(1.))
                            .mt(px(24.))
                            .left_0()
                            .right_0()
                            .px_6()
                            .child(
                                note(message)
                                    .id("setup-notice")
                                    .debug_selector(|| "setup-notice".into())
                                    .text_color(rgb(self.palette.destructive)),
                            )
                    })),
            )
            .child(
                div()
                    .absolute()
                    .bottom(px(24.))
                    .left_0()
                    .right_0()
                    .flex()
                    .justify_center()
                    .child(
                        self.button_with_variant(
                            "setup-skip",
                            tr!(language, "setup.welcome.skip"),
                            UiAction::FirstRun(Action::Skip),
                            ButtonVariant::Ghost,
                            cx,
                        )
                        .h(px(28.))
                        .py_0()
                        .px_3()
                        .text_xs()
                        .font_weight(gpui::FontWeight::NORMAL)
                        .text_color(rgb(self.palette.muted)),
                    ),
            )
            .into_any_element()
    }

    /// Overview and Game choice share one scrollable body; only Overview owns a
    /// footer, because those are the two steps it tracks.
    fn first_run_progress_page(&self, cx: &Context<'_, Self>) -> gpui::AnyElement {
        let error = self.first_run_feedback(self.language()).map(|message| {
            div()
                .id("setup-notice")
                .debug_selector(|| "setup-notice".into())
                .text_sm()
                .text_color(rgb(self.palette.destructive))
                .child(message)
        });
        let content = if self.shell.first_run.stage == Stage::GameChoice {
            self.first_run_game_choice(cx)
        } else {
            self.first_run_tasks(cx)
        };
        let footer = self.first_run_footer(cx);
        div()
            .id("setup-overview")
            .debug_selector(|| "setup-overview".into())
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .id("setup-scroll")
                    .flex_1()
                    .min_h_0()
                    .scroll_area(
                        &self.shell.first_run.scroll,
                        "setup.overview",
                        self.palette,
                        false,
                    )
                    .child(
                        div()
                            .min_h_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .p_6()
                            .child(
                                div()
                                    .w_full()
                                    .max_w(px(620.))
                                    .flex()
                                    .flex_col()
                                    .gap_4()
                                    .child(content)
                                    .children(error),
                            ),
                    ),
            )
            .children(footer)
            .into_any_element()
    }

    /// Only Overview tracks the two steps, so only it offers Continue.
    fn first_run_footer(&self, cx: &Context<'_, Self>) -> Option<gpui::Div> {
        if self.shell.first_run.stage != Stage::Overview {
            return None;
        }
        let ready = self.first_run_account_ready() && self.first_run_game_ready();
        let language = self.language();
        Some(
            super::patterns::management_footer(self.palette).child(
                div()
                    .w_full()
                    .max_w(px(620.))
                    .mx_auto()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap_3()
                    .child(self.button_with_variant(
                        "setup-continue",
                        if ready {
                            tr!(language, "setup.footer.finish")
                        } else {
                            tr!(language, "setup.footer.continue")
                        },
                        UiAction::FirstRun(Action::Continue),
                        ButtonVariant::Primary,
                        cx,
                    )),
            ),
        )
    }

    /// The two ways to start a game: the recommended current release, or an
    /// explicit version, loader and settings choice.
    fn first_run_game_choice(&self, cx: &Context<'_, Self>) -> gpui::AnyElement {
        let language = self.language();
        let recommended = self.recommended_setup_version().map_or_else(
            || tr!(language, "setup.game_choice.recommended_fallback"),
            |version| tr!(language, "setup.game_choice.recommended", version = version),
        );
        div()
            .flex()
            .flex_col()
            .gap_5()
            .child(page_heading(
                tr!(language, "setup.game_choice.title"),
                tr!(language, "setup.game_choice.description"),
            ))
            .child(self.setup_choice(
                "setup-recommended",
                &tr!(language, "setup.game_choice.recommended_title"),
                &recommended,
                Action::Recommended,
                cx,
            ))
            .child(self.setup_choice(
                "setup-custom",
                &tr!(language, "setup.game_choice.custom_title"),
                &tr!(language, "setup.game_choice.custom_description"),
                Action::Custom,
                cx,
            ))
            .child(self.button_with_variant(
                "setup-import-game",
                tr!(language, "setup.game_choice.import"),
                UiAction::FirstRun(Action::Import),
                ButtonVariant::Ghost,
                cx,
            ))
            .child(self.button_with_variant(
                "setup-reload-versions",
                tr!(language, "setup.game_choice.reload"),
                UiAction::ReloadVersions,
                ButtonVariant::Ghost,
                cx,
            ))
            .into_any_element()
    }

    /// The two setup steps, each naming its own current state.
    fn first_run_tasks(&self, cx: &Context<'_, Self>) -> gpui::AnyElement {
        let language = self.language();
        let account = self.first_run_account_ready();
        let game = self.first_run_game_ready();
        let account_description = self.first_run_account_description(language);
        let game_description = self.first_run_game_description(language);
        div()
            .flex()
            .flex_col()
            .gap_5()
            .child(page_heading(
                tr!(language, "setup.overview.title"),
                tr!(language, "setup.overview.description"),
            ))
            .child(
                div()
                    .w_full()
                    .flex()
                    .items_stretch()
                    .gap_4()
                    .child(self.setup_task_card(
                        "setup-account",
                        &tr!(language, "setup.account.title"),
                        &account_description,
                        Icon::Account,
                        account,
                        Action::Account,
                        cx,
                    ))
                    .child(self.setup_task_card(
                        "setup-game",
                        &tr!(language, "setup.game.title"),
                        &game_description,
                        Icon::Games,
                        game,
                        Action::Game,
                        cx,
                    )),
            )
            .into_any_element()
    }

    /// The selected account's player name, or the step that is still missing.
    fn first_run_account_description(&self, language: Language) -> String {
        self.shell
            .onboarding
            .account
            .as_ref()
            .or(self.selected_account.as_ref())
            .and_then(|id| {
                self.accounts
                    .iter()
                    .find(|account| super::account_id(account) == *id)
            })
            .map_or_else(
                || tr!(language, "setup.account.sign_in"),
                |account| account.player_name.clone(),
            )
    }

    /// How many games are saved, or the step that is still missing.
    fn first_run_game_description(&self, language: Language) -> String {
        let count = self.snapshot.profiles.len();
        if self.first_run_game_ready() {
            if count == 1 {
                tr!(language, "setup.game.saved_one", count = count)
            } else {
                tr!(language, "setup.game.saved_other", count = count)
            }
        } else {
            tr!(language, "setup.game.none")
        }
    }

    fn setup_choice(
        &self,
        id: &'static str,
        title: &str,
        description: &str,
        action: Action,
        cx: &Context<'_, Self>,
    ) -> components::Control {
        self.button_with_variant(
            id,
            "",
            UiAction::FirstRun(action),
            ButtonVariant::Outline,
            cx,
        )
        .w_full()
        .min_h(px(90.))
        .p_4()
        .justify_start()
        .child(
            div()
                .flex()
                .flex_col()
                .items_start()
                .gap_2()
                .child(title.to_owned())
                .child(note(description.to_owned())),
        )
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "A task tile has explicit content and action."
    )]
    fn setup_task_card(
        &self,
        id: &'static str,
        title: &str,
        description: &str,
        icon: Icon,
        complete: bool,
        action: Action,
        cx: &Context<'_, Self>,
    ) -> components::Control {
        self.button_with_variant(
            id,
            "",
            UiAction::FirstRun(action),
            ButtonVariant::Outline,
            cx,
        )
        .flex_1()
        .min_w_0()
        .h_auto()
        .p_6()
        .items_start()
        .justify_start()
        .child(
            div()
                .w_full()
                .min_w_0()
                .whitespace_normal()
                .flex()
                .flex_col()
                .items_start()
                .gap_4()
                .child(icon.render(28., self.palette.foreground))
                .child(
                    div()
                        .w_full()
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .child(
                            div()
                                .w_full()
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap_2()
                                .child(
                                    div()
                                        .text_lg()
                                        .font_weight(gpui::FontWeight::BOLD)
                                        .child(title.to_owned()),
                                )
                                .children(complete.then(|| {
                                    div()
                                        .text_xs()
                                        .child(tr!(self.language(), "setup.game.saved_badge"))
                                })),
                        )
                        .child(note(description.to_owned())),
                ),
        )
    }
}
