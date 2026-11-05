//! First-run setup center.
//!
//! Welcome records that setup started and then hands over to the center: games
//! first, accounts second, each step saving on its own. Discovery runs in the
//! background, so the center can only report how many games are importable, and
//! finishing setup rejects a report that is still in flight.
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
use block_launcher::catalog::{InstallStatus, PackStatus};
use gpui::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, ScrollHandle, Styled as _,
    Window, div, prelude::FluentBuilder as _, px, rgb,
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
    /// Start the one step the saved game still needs, chosen from its state.
    Prepare,
    /// Scan the automatic sources again without touching the rest of setup.
    Rescan,
    Skip,
    /// A concrete language choice from the first-setup selector.
    Language(Language),
}

/// What the selected game still needs, which decides the center's single repair
/// action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GameState {
    /// No saved game is selected, so there is nothing to repair.
    None,
    /// The saved game is ready to play as it is.
    Ready,
    /// The instance's modpack installation never completed.
    PendingPack,
    /// The launch target still needs its game files prepared.
    Unprepared,
}

/// What the center knows about the background scan. One state at a time, so a
/// loading scan, a failure, a found count and an empty result never read as
/// each other.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Scan {
    /// A request is in flight.
    Searching,
    /// The scan could not run; its detail is shown as the failure text.
    Failed(String),
    /// This many games can be imported.
    Found(usize),
    /// The scan finished and nothing importable was found.
    Empty,
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
        if self.shell.first_run.stage == Stage::Overview {
            self.ensure_setup_scan();
        }
        self.motion_dirty = true;
        cx.notify();
    }

    /// Ensure the center has a discovery result. One scan covers the session: a
    /// report or an in-flight request is reused.
    fn ensure_setup_scan(&mut self) {
        let state = &self.shell.import_games;
        if state.report.is_some() || state.discovery.is_some() || state.running {
            return;
        }
        self.scan_games_for_setup();
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
            .find(|version| version.kind == block_launcher::manifest::model::VersionType::Release)
            .map(|version| version.id.as_str())
    }

    pub(super) fn first_run_game_ready(&self) -> bool {
        !self.snapshot.instances.is_empty()
    }

    /// What the center knows about the background scan, or nothing before the
    /// first request.
    pub(super) fn first_run_scan(&self, language: Language) -> Option<Scan> {
        let state = &self.shell.import_games;
        if state.discovery.is_some() {
            return Some(Scan::Searching);
        }
        if let Some(error) = &state.error {
            return Some(Scan::Failed(error.text(language)));
        }
        let report = state.report.as_ref()?;
        let found = report
            .candidates
            .iter()
            .filter(|candidate| candidate.importable)
            .count();
        Some(if found == 0 {
            Scan::Empty
        } else {
            Scan::Found(found)
        })
    }

    /// What the selected game still needs. It reads the selected profile, which
    /// is exactly what the center's repair action acts on, so the center never
    /// offers a step that could not run.
    fn first_run_game_state(&self) -> GameState {
        let Some(profile) = self.profile() else {
            return GameState::None;
        };
        let Some(instance) = self
            .snapshot
            .instances
            .iter()
            .find(|instance| instance.id == profile.instance_id)
        else {
            return GameState::None;
        };
        if instance
            .pack
            .as_ref()
            .is_some_and(|pack| pack.status == PackStatus::Pending)
        {
            GameState::PendingPack
        } else if instance
            .targets
            .iter()
            .any(|target| target.id == profile.target_id && target.status == InstallStatus::Ready)
        {
            GameState::Ready
        } else {
            GameState::Unprepared
        }
    }

    pub(super) fn first_run_continue_exits(&self) -> bool {
        self.first_run_game_state() != GameState::PendingPack
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

    pub(super) fn first_run_overview(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) {
        // The center exists only while setup is incomplete; a late action from
        // inside the flow cannot revive it after Finish.
        if !self.shell.first_run.enabled || self.preferences.onboarding_complete {
            return;
        }
        self.shell.wizard.cancel_loader_request();
        self.shell.first_run.stage = Stage::Overview;
        self.router.leave_task();
        self.shell.first_run.entered = None;
        // Entering the center is what asks the automatic sources for a count.
        self.ensure_setup_scan();
        self.motion_dirty = true;
        cx.notify();
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
                // The marker is the only thing that remembers setup started. A
                // failed write stays on Welcome with its notice rather than
                // pretending the center can be resumed later.
                if !self.mark_first_run_started() {
                    return;
                }
                self.first_run_overview(window, cx);
            }
            Action::Continue => {
                let state = self.first_run_game_state();
                if state == GameState::PendingPack {
                    self.handle_first_run(Action::Prepare, window, cx);
                } else if self.complete_onboarding() && state == GameState::Unprepared {
                    self.handle(UiAction::Repair, window, cx);
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
            }
            Action::Recommended | Action::Custom => {
                // Any game entry resumes the draft the user already has instead
                // of replacing it with a fresh one.
                if matches!(self.form, Some(Form::Instance(_))) {
                    self.shell.first_run.stage = Stage::GameWizard;
                    self.router.enter_game_wizard();
                    self.focus_field(false, window, cx);
                    cx.notify();
                    return;
                }
                let recommended = matches!(action, Action::Recommended);
                let version = self.recommended_setup_version().map(str::to_owned);
                if recommended && version.is_none() {
                    self.shell.first_run.notice = Some(Notice::VersionsUnavailable);
                    return;
                }
                self.begin_game_wizard(cx);
                self.shell.wizard.step = WizardStep::Setup;
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
                // Recommended only pre-fills the same Setup page; the draft is
                // saved when the user asks for it.
                if recommended && let Some(version) = version {
                    self.set_wizard_version(version, cx);
                }
                self.mark_game_wizard_clean(cx);
                self.focus_field(false, window, cx);
            }
            Action::Back => {
                self.shell.surfaces.clear(window, &self.focus, cx);
                self.first_run_overview(window, cx);
            }
            Action::BackToWelcome => {
                // Returning to Welcome is navigation, not a reset: `setup-started`
                // and the saved game parent keep the draft resumable.
                // The previous control was unmounted while hovered and may
                // never receive mouse-leave. Give the returning control fresh
                // hover/pointer state when the page returns.
                self.shell.first_run.welcome_button = Default::default();
                self.shell.first_run.stage = Stage::Welcome;
                self.router.leave_task();
                self.shell.first_run.entered = None;
                self.motion_dirty = true;
            }
            Action::Import => self.open_import_games(window, cx),
            Action::Prepare => match self.first_run_game_state() {
                GameState::PendingPack => {
                    if let Some(instance) =
                        self.profile().map(|profile| profile.instance_id.clone())
                    {
                        self.resume_pack_import(instance, window, cx);
                    }
                }
                GameState::Unprepared => self.handle(UiAction::Repair, window, cx),
                GameState::None | GameState::Ready => {}
            },
            Action::Rescan => self.scan_games_for_setup(),
            Action::Skip => {
                self.complete_onboarding();
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
        self.first_run_progress_page(window, cx)
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
    fn first_run_progress_page(
        &self,
        _window: &Window,
        cx: &Context<'_, Self>,
    ) -> gpui::AnyElement {
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
                    .debug_selector(|| "setup-scroll".into())
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
                            .p_4()
                            .child(
                                div()
                                    .w_full()
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

    /// The primary action names what happens to the selected game and where
    /// setup goes next. Leaving without preparing remains an explicit choice.
    fn first_run_footer(&self, cx: &Context<'_, Self>) -> Option<gpui::Stateful<gpui::Div>> {
        if self.shell.first_run.stage != Stage::Overview {
            return None;
        }
        let state = self.first_run_game_state();
        let language = self.language();
        let later = matches!(state, GameState::Unprepared | GameState::PendingPack).then(|| {
            self.button_with_variant(
                "setup-later",
                tr!(language, "setup.footer.home_later"),
                UiAction::FirstRun(Action::Skip),
                ButtonVariant::Ghost,
                cx,
            )
            .w_full()
            .h_auto()
            .min_h(px(36.))
            .whitespace_normal()
        });
        Some(
            super::patterns::management_footer(self.palette)
                .id("setup-footer")
                .debug_selector(|| "setup-footer".into())
                .h_auto()
                .min_h(px(super::design::MANAGEMENT_FOOTER))
                .py_3()
                .child(
                    div()
                        .w_full()
                        .mx_auto()
                        .flex()
                        .flex_col()
                        .items_stretch()
                        .justify_end()
                        .gap_3()
                        .children(later)
                        .child(
                            self.button_with_variant(
                                "setup-continue",
                                tr!(
                                    language,
                                    match state {
                                        GameState::Unprepared => "setup.footer.prepare_home",
                                        GameState::PendingPack => "setup.game.continue_import",
                                        GameState::None | GameState::Ready => "setup.footer.home",
                                    }
                                ),
                                UiAction::FirstRun(Action::Continue),
                                ButtonVariant::Primary,
                                cx,
                            )
                            .w_full()
                            .h_auto()
                            .min_h(px(36.))
                            .whitespace_normal(),
                        ),
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
                "setup-import-pack",
                tr!(language, "modpack.action.import_pack"),
                UiAction::PackImport(super::modpack_import::Action::Open),
                ButtonVariant::Outline,
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

    /// Games first, accounts second, then the facts that decide what the
    /// center still offers. Every row states one thing and owns at most one
    /// action.
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
                    .flex_col()
                    .items_stretch()
                    .gap_4()
                    .child(self.setup_task_card(
                        "setup-game",
                        &tr!(language, "setup.game.title"),
                        &game_description,
                        Icon::Games,
                        game,
                        Action::Game,
                        cx,
                    ))
                    .child(self.setup_task_card(
                        "setup-account",
                        &tr!(language, "setup.account.title"),
                        &account_description,
                        Icon::Account,
                        account,
                        Action::Account,
                        cx,
                    )),
            )
            .children(
                (matches!(
                    self.shell.games.load,
                    super::browser::CollectionLoad::Failed(_)
                ) || matches!(
                    self.shell.accounts.load,
                    super::browser::CollectionLoad::Failed(_)
                ))
                .then(|| {
                    self.button_with_variant(
                        "setup-retry-library",
                        tr!(language, "setup.library.retry"),
                        UiAction::ReloadLibrary,
                        ButtonVariant::Outline,
                        cx,
                    )
                }),
            )
            .child(self.first_run_notes(cx))
            .into_any_element()
    }

    /// The center's own state: where games are saved, what the background scan
    /// found, the draft it can resume, the one step the saved game still needs
    /// and the language entry a returning user needs.
    fn first_run_notes(&self, cx: &Context<'_, Self>) -> gpui::Div {
        let language = self.language();
        div()
            .w_full()
            .flex()
            .flex_col()
            .gap_3()
            .rounded_lg()
            .border_1()
            .border_color(rgb(self.palette.border))
            .p_4()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(note(tr!(language, "setup.library.label")))
                    .child(
                        div()
                            .text_sm()
                            .min_w_0()
                            .whitespace_normal()
                            .child(self.first_run_game_parent().display().to_string()),
                    ),
            )
            .children(self.first_run_import_note(cx))
            .children(self.first_run_draft_note(cx))
            .children(self.first_run_prepare_note())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap_3()
                    .child(note(tr!(language, "setup.language.description")))
                    .child(self.button_with_variant(
                        "setup-language-entry",
                        tr!(language, "setup.language.label"),
                        UiAction::Drawer(Some(super::Drawer::Language)),
                        ButtonVariant::Ghost,
                        cx,
                    )),
            )
    }

    /// The background scan, which can only report a number: searching, a
    /// failure with its retry, how many games are importable, or an explicit
    /// empty result. The drawer opens only when the user asks for it.
    fn first_run_import_note(&self, cx: &Context<'_, Self>) -> Option<gpui::AnyElement> {
        let language = self.language();
        match self.first_run_scan(language)? {
            Scan::Searching => {
                Some(note(tr!(language, "setup.import.scanning")).into_any_element())
            }
            Scan::Failed(detail) => Some(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(note(detail))
                    .child(self.button_with_variant(
                        "setup-rescan",
                        tr!(language, "setup.import.retry"),
                        UiAction::FirstRun(Action::Rescan),
                        ButtonVariant::Ghost,
                        cx,
                    ))
                    .into_any_element(),
            ),
            Scan::Empty => Some(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(note(tr!(language, "setup.import.none")))
                    .child(self.button_with_variant(
                        "setup-rescan",
                        tr!(language, "setup.import.retry"),
                        UiAction::FirstRun(Action::Rescan),
                        ButtonVariant::Ghost,
                        cx,
                    ))
                    .into_any_element(),
            ),
            Scan::Found(count) => Some(
                div()
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap_3()
                    .child(note(tr!(language, "setup.import.found", count = count)))
                    .child(self.button_with_variant(
                        "setup-import",
                        tr!(language, "setup.import.open"),
                        UiAction::FirstRun(Action::Import),
                        ButtonVariant::Ghost,
                        cx,
                    ))
                    .into_any_element(),
            ),
        }
    }

    /// A game draft that was left in the wizard stays resumable here; nothing
    /// is written until its own save runs.
    fn first_run_draft_note(&self, cx: &Context<'_, Self>) -> Option<gpui::AnyElement> {
        matches!(self.form, Some(Form::Instance(_))).then(|| {
            let language = self.language();
            div()
                .flex()
                .flex_col()
                .items_start()
                .gap_3()
                .child(note(tr!(language, "setup.draft.found")))
                .child(self.button_with_variant(
                    "setup-draft-continue",
                    tr!(language, "setup.draft.continue"),
                    UiAction::FirstRun(Action::Custom),
                    ButtonVariant::Ghost,
                    cx,
                ))
                .into_any_element()
        })
    }

    /// The saved game's one remaining step, separate from leaving for Home.
    fn first_run_prepare_note(&self) -> Option<gpui::AnyElement> {
        let language = self.language();
        let description = match self.first_run_game_state() {
            GameState::PendingPack => tr!(language, "setup.game.pack_pending"),
            GameState::Unprepared => tr!(language, "setup.game.unprepared"),
            GameState::None | GameState::Ready => return None,
        };
        Some(note(description).into_any_element())
    }

    /// The selected account's player name, or the step that is still missing.
    fn first_run_account_description(&self, language: Language) -> String {
        match &self.shell.accounts.load {
            super::browser::CollectionLoad::Loading => {
                return tr!(language, "games.browser.loading_records");
            }
            super::browser::CollectionLoad::Failed(error) => {
                return tr!(
                    language,
                    "setup.library.failed",
                    detail = error.text(language)
                );
            }
            super::browser::CollectionLoad::Ready => {}
        }
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

    /// How many games are saved, or the step that is still missing. The count is
    /// of Instances: a game is one Instance, however many Profiles launch it.
    pub(super) fn first_run_game_description(&self, language: Language) -> String {
        match &self.shell.games.load {
            super::browser::CollectionLoad::Loading => {
                return tr!(language, "games.browser.loading_records");
            }
            super::browser::CollectionLoad::Failed(error) => {
                return tr!(
                    language,
                    "setup.library.failed",
                    detail = error.text(language)
                );
            }
            super::browser::CollectionLoad::Ready => {}
        }
        let count = self.snapshot.instances.len();
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
        .h_auto()
        .min_h(px(90.))
        .p_4()
        .justify_start()
        .child(
            div()
                .min_w_0()
                .w_full()
                .flex()
                .flex_col()
                .items_start()
                .gap_2()
                .child(div().w_full().whitespace_normal().child(title.to_owned()))
                .child(note(description.to_owned()).w_full().whitespace_normal()),
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
        let game = matches!(action, Action::Game);
        self.button_with_variant(
            id,
            "",
            UiAction::FirstRun(action),
            ButtonVariant::Outline,
            cx,
        )
        .w_full()
        .flex_shrink_0()
        .min_w_0()
        .h_auto()
        .p_4()
        .when(game, |card| card.p_6())
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
                .child(icon.render(if game { 28. } else { 20. }, self.palette.foreground))
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
