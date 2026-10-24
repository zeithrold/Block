//! First-run task overview; completed Wizards own their individual saves.
#![expect(
    clippy::redundant_pub_crate,
    reason = "Private first-run presentation."
)]

use super::scroll_area::ScrollAreaExt as _;
use super::{
    Block, Form, Tab, UiAction,
    components::{self, ButtonVariant},
    design::Icon,
    game_wizard::WizardStep,
    note,
    patterns::page_heading,
};
use gpui::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, ScrollHandle,
    StatefulInteractiveElement as _, Styled as _, Window, div, px, rgb,
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
    Storage,
}

/// Mutually exclusive storage cards on the Game storage page. The Global card
/// is the fixed app-data library; the Local card is the executable-adjacent
/// library, optionally overridden with the native folder picker.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum StorageChoice {
    Global,
    #[default]
    Local,
}

impl StorageChoice {
    /// Infer the initial card from the saved game parent. A parent that already
    /// points at the app-data library keeps the Global card selected; every
    /// other saved parent becomes the Local card's path.
    pub(super) fn infer(current: &Path, global: Option<&Path>) -> Self {
        if global.is_some_and(|global| global == current) {
            Self::Global
        } else {
            Self::Local
        }
    }
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
    MoreOptions,
    SelectStorage(StorageChoice),
    SaveStorage,
    BrowseStorage,
    Import,
    Skip,
}

#[derive(Clone, Debug, Default)]
pub(super) struct State {
    /// Older component-only fixtures still exercise the shared draft helpers.
    pub enabled: bool,
    pub stage: Stage,
    pub welcome_button: super::welcome_button::WelcomeButton,
    pub game_parent: Option<PathBuf>,
    /// Selected storage card; the two cards are always mutually exclusive.
    pub storage_choice: StorageChoice,
    /// Fixed app-data library shown by the Global card, cached on entry.
    pub storage_global: Option<PathBuf>,
    /// Executable-adjacent or picker-chosen library shown by the Local card.
    pub storage_local: Option<PathBuf>,
    pub entered: Option<Instant>,
    pub error: Option<String>,
    pub scroll: ScrollHandle,
}

impl Block {
    pub(super) fn begin_first_run(&mut self, cx: &mut Context<'_, Self>) {
        self.tab = Tab::Onboarding;
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
        self.shell.first_run.game_parent =
            Some(existing.and_then(|value| value.library).unwrap_or_else(|| {
                std::env::current_exe().map_or_else(
                    |_| fallback.clone(),
                    |executable| super::setup_storage::default_game_parent(&executable, &fallback),
                )
            }));
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
                |engine| engine.root.join("instances"),
            )
        })
    }

    /// Default path for the Local storage card: the game library beside the
    /// running executable (never inside a macOS `.app` bundle).
    fn adjacent_game_parent(&self) -> PathBuf {
        std::env::current_exe()
            .map(|executable| super::setup_storage::adjacent_game_parent(&executable))
            .unwrap_or_else(|_| {
                self.engine.as_ref().map_or_else(
                    || PathBuf::from(".minecraft"),
                    |engine| engine.root.join(".minecraft"),
                )
            })
    }

    fn save_storage_parent(
        &mut self,
        parent: PathBuf,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let result = self
            .engine
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Block's data directory is unavailable"))
            .and_then(|engine| super::setup_storage::save_game_parent(&engine.root, &parent));
        match result {
            Ok(()) => {
                self.shell.first_run.game_parent = Some(parent);
                self.first_run_overview(window, cx);
            }
            Err(error) => self.shell.first_run.error = Some(error.to_string()),
        }
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
            self.shell.first_run.error = Some(format!("Could not save setup progress: {error}"));
            return false;
        }
        true
    }

    pub(super) fn first_run_overview(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.shell.wizard.cancel_loader_request();
        let leaving_welcome = self.shell.first_run.stage == Stage::Welcome;
        // Freeze the leaving Welcome page before the stage changes.
        let outgoing = leaving_welcome.then(|| std::rc::Rc::new(self.visual_snapshot(cx)));
        self.shell.first_run.stage = Stage::Overview;
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
        match action {
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
                self.shell.first_run.entered = Some(cx.background_executor().now());
                self.open_auth_wizard(window, cx);
            }
            Action::Game => {
                self.shell.first_run.stage = Stage::GameChoice;
                self.shell.first_run.entered = Some(cx.background_executor().now());
                self.request_frame_resize(window, cx);
            }
            Action::Recommended | Action::Custom => {
                let recommended = matches!(action, Action::Recommended);
                if !recommended && matches!(self.form, Some(Form::Instance(_))) {
                    self.shell.first_run.stage = Stage::GameWizard;
                    return;
                }
                let version = self.recommended_setup_version().map(str::to_owned);
                if recommended && version.is_none() {
                    self.shell.first_run.error = Some("Load the Minecraft version list before using recommended setup. You can retry or use custom setup.".into());
                    return;
                }
                self.begin_game_wizard(cx);
                self.tab = Tab::Onboarding;
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
                self.shell.first_run.stage = Stage::Welcome;
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
            Action::MoreOptions => {
                let current = self.first_run_game_parent();
                let global = super::setup_storage::global_game_parent();
                let choice = StorageChoice::infer(&current, global.as_deref());
                let local = match choice {
                    StorageChoice::Local => current,
                    StorageChoice::Global => self.adjacent_game_parent(),
                };
                self.shell.first_run.storage_choice = choice;
                self.shell.first_run.storage_global = global;
                self.shell.first_run.storage_local = Some(local);
                self.shell.first_run.stage = Stage::Storage;
                self.shell.first_run.entered = Some(cx.background_executor().now());
                self.request_frame_resize(window, cx);
            }
            Action::SelectStorage(choice) => {
                self.shell.first_run.storage_choice = choice;
            }
            Action::BrowseStorage => {
                // Only the Local card browses: choosing a folder replaces that
                // card's path and selects it. Global stays read-only.
                let response = cx.prompt_for_paths(gpui::PathPromptOptions {
                    files: false,
                    directories: true,
                    multiple: false,
                    prompt: Some("Choose Folder".into()),
                });
                cx.spawn(async move |view, cx| {
                    if let Ok(Ok(Some(paths))) = response.await
                        && let Some(path) = paths.into_iter().next()
                    {
                        let _ = view.update(cx, |view, cx| {
                            view.shell.first_run.storage_local = Some(path);
                            view.shell.first_run.storage_choice = StorageChoice::Local;
                            cx.notify();
                        });
                    }
                })
                .detach();
            }
            Action::SaveStorage => match self.shell.first_run.storage_choice {
                StorageChoice::Global => {
                    let global = self
                        .shell
                        .first_run
                        .storage_global
                        .clone()
                        .or_else(super::setup_storage::global_game_parent);
                    match global {
                        Some(parent) => self.save_storage_parent(parent, window, cx),
                        None => {
                            self.shell.first_run.error = Some(
                                "Global storage is unavailable on this system. Choose local storage instead."
                                    .into(),
                            );
                        }
                    }
                }
                StorageChoice::Local => {
                    let parent = self
                        .shell
                        .first_run
                        .storage_local
                        .clone()
                        .unwrap_or_else(|| self.first_run_game_parent());
                    self.save_storage_parent(parent, window, cx);
                }
            },
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
        if self.shell.auth.is_some() {
            return self.render_auth_wizard(window, cx).into_any_element();
        }
        if self.shell.first_run.stage == Stage::GameWizard
            && let Some(Form::Instance(form)) = &self.form
        {
            return self.render_game_wizard(form, window, cx).into_any_element();
        }
        if self.shell.first_run.stage == Stage::Welcome {
            let button_size = px(super::welcome_button::DIAMETER);
            return div()
                .id("setup-welcome")
                .debug_selector(|| "setup-welcome".into())
                .relative()
                .size_full()
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
                                        .child("Welcome Back"),
                                )
                                .child(
                                    note("Add an account and set up a game.").line_height(px(18.)),
                                ),
                        )
                        .child(super::welcome_button::render(
                            &self.shell.first_run.welcome_button,
                            self.palette,
                            self.motion_preferences(),
                            window,
                            cx,
                            cx.listener(|view, _, window, cx| {
                                view.handle(UiAction::FirstRun(Action::Start), window, cx)
                            }),
                        ))
                        .children(self.shell.first_run.error.as_ref().map(|error| {
                            div()
                                .absolute()
                                .top(gpui::relative(1.))
                                .mt(px(24.))
                                .left_0()
                                .right_0()
                                .px_6()
                                .child(note(error.clone()))
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
                                "Skip setup",
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
                .into_any_element();
        }
        let error = self.shell.first_run.error.as_ref().map(|message| {
            div()
                .text_sm()
                .text_color(rgb(self.palette.destructive))
                .child(message.clone())
        });
        let content = match self.shell.first_run.stage {
            Stage::Storage => {
                let global = self
                    .shell
                    .first_run
                    .storage_global
                    .clone()
                    .or_else(super::setup_storage::global_game_parent);
                let local = self
                    .shell
                    .first_run
                    .storage_local
                    .clone()
                    .unwrap_or_else(|| self.first_run_game_parent());
                div()
                    .flex()
                    .flex_col()
                    .gap_5()
                    .child(page_heading(
                        "Game storage",
                        "Choose where new games are saved. Existing games stay in their current folders.",
                    ))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_4()
                            .child(self.storage_card(
                                "setup-storage-global",
                                StorageChoice::Global,
                                "Global storage",
                                "Use Block's app-data folder. Shared by every Block installation for this user.",
                                global.as_deref(),
                                false,
                                cx,
                            ))
                            .child(self.storage_card(
                                "setup-storage-local",
                                StorageChoice::Local,
                                "Local storage",
                                "Keep new games beside Block so the app folder can move with them.",
                                Some(local.as_path()),
                                true,
                                cx,
                            )),
                    )
                    .into_any_element()
            }
            Stage::GameChoice => div()
                .flex()
                .flex_col()
                .gap_5()
                .child(page_heading(
                    "Set up a game",
                    "Use the current release of Minecraft, or choose its version and components.",
                ))
                .child(self.setup_choice(
                    "setup-recommended",
                    "Recommended setup",
                    &self.recommended_setup_version().map_or_else(
                        || "Stable Vanilla Minecraft · automatic Java".to_owned(),
                        |version| format!("Minecraft {version} · Vanilla · automatic Java"),
                    ),
                    Action::Recommended,
                    cx,
                ))
                .child(self.setup_choice(
                    "setup-custom",
                    "Custom setup",
                    "Choose the Minecraft version, loader and game settings.",
                    Action::Custom,
                    cx,
                ))
                .child(self.button_with_variant(
                    "setup-import-game",
                    "Import existing games",
                    UiAction::FirstRun(Action::Import),
                    ButtonVariant::Ghost,
                    cx,
                ))
                .child(self.button_with_variant(
                    "setup-reload-versions",
                    "Reload versions",
                    UiAction::ReloadVersions,
                    ButtonVariant::Ghost,
                    cx,
                ))
                .into_any_element(),
            _ => {
                let account = self.first_run_account_ready();
                let game = self.first_run_game_ready();
                let account_description = self
                    .shell
                    .onboarding
                    .account
                    .as_ref()
                    .or(self.selected_account.as_ref())
                    .and_then(|id| {
                        self.accounts
                            .iter()
                            .find(|account| super::account_id(account) == *id)
                    })
                    .map_or("Sign in to your Minecraft account.", |account| {
                        account.player_name.as_str()
                    });
                let game_description = if game {
                    format!(
                        "{} {} saved",
                        self.snapshot.profiles.len(),
                        if self.snapshot.profiles.len() == 1 {
                            "game"
                        } else {
                            "games"
                        }
                    )
                } else {
                    "Create a game or import an existing one.".into()
                };
                div()
                    .flex()
                    .flex_col()
                    .gap_5()
                    .child(page_heading(
                        "Two steps and you're ready",
                        "Add an account and a game to finish setup.",
                    ))
                    .child(
                        div()
                            .w_full()
                            .flex()
                            .items_stretch()
                            .gap_4()
                            .child(self.setup_task_card(
                                "setup-account",
                                "Account",
                                account_description,
                                Icon::Account,
                                account,
                                Action::Account,
                                cx,
                            ))
                            .child(self.setup_task_card(
                                "setup-game",
                                "Game",
                                &game_description,
                                Icon::Games,
                                game,
                                Action::Game,
                                cx,
                            )),
                    )
                    .into_any_element()
            }
        };
        let footer = match self.shell.first_run.stage {
            Stage::Overview => {
                let ready = self.first_run_account_ready() && self.first_run_game_ready();
                Some(
                    super::patterns::management_footer(self.page_viewport(window), self.palette)
                        .child(
                            div()
                                .w_full()
                                .max_w(px(620.))
                                .mx_auto()
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap_3()
                                .child(self.button_with_variant(
                                    "setup-more-options",
                                    "More options",
                                    UiAction::FirstRun(Action::MoreOptions),
                                    ButtonVariant::Ghost,
                                    cx,
                                ))
                                .child(self.button_with_variant(
                                    "setup-continue",
                                    if ready { "Finish" } else { "Continue" },
                                    UiAction::FirstRun(Action::Continue),
                                    ButtonVariant::Primary,
                                    cx,
                                )),
                        ),
                )
            }
            Stage::Storage => Some(
                super::patterns::management_footer(self.page_viewport(window), self.palette).child(
                    div()
                        .w_full()
                        .max_w(px(620.))
                        .mx_auto()
                        .flex()
                        .items_center()
                        .justify_end()
                        .child(self.button_with_variant(
                            "setup-save-storage",
                            "Save",
                            UiAction::FirstRun(Action::SaveStorage),
                            ButtonVariant::Primary,
                            cx,
                        )),
                ),
            ),
            _ => None,
        };
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
                    .scroll_area(&self.shell.first_run.scroll, "Setup", self.palette, false)
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

    /// One storage option card. Both cards display their path read-only; the
    /// whole card selects the option and only the Local card can browse.
    #[expect(
        clippy::too_many_arguments,
        reason = "A storage card has explicit content and behavior."
    )]
    fn storage_card(
        &self,
        id: &'static str,
        choice: StorageChoice,
        title: &'static str,
        description: &'static str,
        path: Option<&Path>,
        browsable: bool,
        cx: &Context<'_, Self>,
    ) -> gpui::AnyElement {
        let selected = self.shell.first_run.storage_choice == choice;
        let palette = self.palette;
        let path_selector = match choice {
            StorageChoice::Global => "setup-storage-global-path",
            StorageChoice::Local => "setup-storage-local-path",
        };
        let card = components::card(palette)
            .id(format!("{id}-card"))
            .debug_selector(move || format!("{id}-card"))
            .gap_3()
            .border_color(rgb(if selected {
                palette.primary
            } else {
                palette.border
            }))
            .bg(rgb(if selected {
                palette.secondary
            } else {
                palette.card
            }))
            .on_click(cx.listener(move |view, _, window, cx| {
                view.handle_first_run(Action::SelectStorage(choice), window, cx);
            }))
            .child(
                components::radio(
                    id,
                    "",
                    selected,
                    palette,
                    components::ControlState::default(),
                    cx.listener(move |view, &(): &(), window, cx| {
                        view.handle_first_run(Action::SelectStorage(choice), window, cx);
                    }),
                )
                .w_full()
                .min_w_0()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .gap(px(6.))
                        .min_w_0()
                        .child(
                            div()
                                .text_lg()
                                .font_weight(gpui::FontWeight::BOLD)
                                .child(title),
                        )
                        .child(note(description)),
                ),
            )
            .child(self.storage_path_row(path_selector, path, browsable, cx));
        if selected {
            card.into_any_element()
        } else {
            card.cursor_pointer()
                .hover(move |style| style.border_color(rgb(palette.ring)))
                .into_any_element()
        }
    }

    /// Read-only path presentation: plain small text, never an input or a
    /// framed container. The Local card adds its folder picker on the right;
    /// the Global card never browses.
    fn storage_path_row(
        &self,
        path_selector: &'static str,
        path: Option<&Path>,
        browsable: bool,
        cx: &Context<'_, Self>,
    ) -> gpui::Div {
        let path_text = path.map_or_else(
            || "Not available on this system".to_owned(),
            |path| path.display().to_string(),
        );
        let mut row = div().w_full().flex().items_center().gap_3().child(
            div()
                .id(path_selector)
                .debug_selector(move || path_selector.to_owned())
                .flex_1()
                .min_w_0()
                .text_xs()
                .text_color(rgb(self.palette.muted))
                .truncate()
                .child(path_text),
        );
        if browsable {
            row = row.child(self.button_with_variant(
                "setup-storage-choose-folder",
                "Choose Folder…",
                UiAction::FirstRun(Action::BrowseStorage),
                ButtonVariant::Outline,
                cx,
            ));
        }
        row
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
                                .children(complete.then(|| div().text_xs().child("Saved"))),
                        )
                        .child(note(description.to_owned())),
                ),
        )
    }
}
