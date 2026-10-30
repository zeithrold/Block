//! Source selection and batch import inside one bottom Drawer.
#![expect(clippy::redundant_pub_crate, reason = "Private import presentation.")]
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    time::{Duration, Instant},
};

use block_launcher::{
    CancellationToken,
    game_import::{DiscoveryReport, DiscoveryRoots, GameCandidate, SourceKind},
};
use gpui::{
    Context, InteractiveElement as _, IntoElement as _, ParentElement as _, ScrollHandle,
    Styled as _, Window, div, relative, rgb,
};
use gpui_base::motion::{Easing, Interpolate as _, Timing};

use super::{
    Block, UiAction,
    components::{self, ButtonVariant, ControlState},
    design::Icon,
    first_run::Stage,
    note,
    scroll_area::ScrollAreaExt as _,
    shell::{Drawer, Surface},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Page {
    #[default]
    Sources,
    List,
    Progress,
    Results,
}

const PAGE_SLIDE_DURATION: Duration = Duration::from_millis(280);

#[derive(Clone, Copy, Debug)]
struct SlideRun {
    from: f32,
    to: f32,
    started: Instant,
}

#[derive(Clone, Debug, Default)]
struct SourceListMotion {
    position: f32,
    run: Option<SlideRun>,
}

impl SourceListMotion {
    fn navigate(&mut self, page: Page, now: Instant, reduced_motion: bool) {
        let target = match page {
            Page::Sources => Some(0.),
            Page::List => Some(1.),
            Page::Progress | Page::Results => None,
        };
        let Some(target) = target else {
            self.run = None;
            return;
        };
        let (position, _) = self.sample(now, reduced_motion);
        self.position = position;
        if reduced_motion || (position - target).abs() < f32::EPSILON {
            self.position = target;
            self.run = None;
        } else {
            self.run = Some(SlideRun {
                from: position,
                to: target,
                started: now,
            });
        }
    }

    fn sample(&mut self, now: Instant, reduced_motion: bool) -> (f32, bool) {
        let Some(run) = self.run else {
            return (self.position, false);
        };
        if reduced_motion {
            self.position = run.to;
            self.run = None;
            return (self.position, false);
        }
        let progress = Timing::new(PAGE_SLIDE_DURATION)
            .ease(Easing::Ease)
            .sample(now.saturating_duration_since(run.started));
        if progress.finished {
            self.position = run.to;
            self.run = None;
        } else {
            self.position = run.from.interpolate(&run.to, progress.directed_progress);
        }
        (self.position, self.run.is_some())
    }

    fn settle(&mut self, page: Page) {
        self.position = if page == Page::List { 1. } else { 0. };
        self.run = None;
    }
}

#[derive(Clone, Debug)]
pub(super) enum Action {
    Source(SourceKind),
    View(SourceKind),
    Sources,
    Toggle(String),
    All,
    Import,
    Stop,
    Retry,
    Done,
    New,
}

#[derive(Clone, Debug)]
pub(super) enum Outcome {
    Saved,
    Failed(String),
    Cancelled,
}

/// What a discovery started from Welcome should do once its report arrives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DeferredDiscovery {
    /// Nothing to present: the report was stale or belonged to an explicit import.
    Ignored,
    /// Show the source drawer because games can be imported.
    OpenDrawer,
    /// Skip the empty drawer and continue to the Account/Game overview.
    ContinueToOverview,
}

#[derive(Clone, Debug)]
pub(super) struct State {
    pub window: Option<gpui::AnyWindowHandle>,
    pub report: Option<DiscoveryReport>,
    pub discovery: Option<uuid::Uuid>,
    pub batch: Option<uuid::Uuid>,
    pub page: Page,
    pub source: SourceKind,
    pub selected: BTreeSet<String>,
    pub submitted: Vec<String>,
    pub outcomes: BTreeMap<String, Outcome>,
    pub cancel: CancellationToken,
    pub running: bool,
    /// The Welcome start button scans first and shows the drawer only when the
    /// report contains games, so an empty scan cannot flash the drawer.
    pub deferred_open: bool,
    pub scroll: ScrollHandle,
    pub source_scroll: ScrollHandle,
    pub list_scroll: ScrollHandle,
    motion: RefCell<SourceListMotion>,
    pub error: Option<String>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            window: None,
            report: None,
            discovery: None,
            batch: None,
            page: Page::Sources,
            source: SourceKind::AdjacentMinecraft,
            selected: BTreeSet::new(),
            submitted: Vec::new(),
            outcomes: BTreeMap::new(),
            cancel: CancellationToken::new(),
            running: false,
            deferred_open: false,
            scroll: ScrollHandle::new(),
            source_scroll: ScrollHandle::new(),
            list_scroll: ScrollHandle::new(),
            motion: RefCell::new(SourceListMotion::default()),
            error: None,
        }
    }
}

impl State {
    fn candidates(&self, source: SourceKind) -> Vec<&GameCandidate> {
        self.report.as_ref().map_or_else(Vec::new, |report| {
            report
                .candidates
                .iter()
                .filter(|candidate| candidate.source == source)
                .collect()
        })
    }
    fn pending(&self) -> Vec<GameCandidate> {
        self.candidates(self.source)
            .into_iter()
            .filter(|candidate| {
                candidate.importable
                    && self.selected.contains(&candidate.id)
                    && !matches!(self.outcomes.get(&candidate.id), Some(Outcome::Saved))
            })
            .cloned()
            .collect()
    }
    fn eligible_ids(&self) -> Vec<String> {
        self.candidates(self.source)
            .into_iter()
            .filter(|candidate| {
                candidate.importable
                    && !matches!(self.outcomes.get(&candidate.id), Some(Outcome::Saved))
            })
            .map(|candidate| candidate.id.clone())
            .collect()
    }
    fn all_selected(&self) -> bool {
        let eligible = self.eligible_ids();
        !eligible.is_empty() && eligible.iter().all(|id| self.selected.contains(id))
    }
    fn toggle_all(&mut self) {
        let eligible = self.eligible_ids();
        let clear = !eligible.is_empty() && eligible.iter().all(|id| self.selected.contains(id));
        for id in eligible {
            if clear {
                self.selected.remove(&id);
            } else {
                self.selected.insert(id);
            }
        }
    }
    fn choose_source(&mut self, source: SourceKind) {
        self.source = source;
        let eligible = self.eligible_ids();
        if let [id] = eligible.as_slice() {
            self.selected.insert(id.clone());
        }
    }
    fn apply_discovery(&mut self, report: DiscoveryReport) {
        let previous_ids = self.report.as_ref().map_or_else(BTreeSet::new, |previous| {
            previous
                .candidates
                .iter()
                .map(|candidate| candidate.id.clone())
                .collect()
        });
        let first_report = self.report.is_none();
        let new_ids = report
            .candidates
            .iter()
            .map(|candidate| candidate.id.clone())
            .collect::<BTreeSet<_>>();
        self.selected.retain(|id| new_ids.contains(id));

        if !report
            .candidates
            .iter()
            .any(|candidate| candidate.source == self.source && candidate.importable)
        {
            self.source = if report.candidates.iter().any(|candidate| {
                candidate.source == SourceKind::AdjacentMinecraft && candidate.importable
            }) {
                SourceKind::AdjacentMinecraft
            } else {
                SourceKind::OfficialLauncher
            };
        }
        for candidate in &report.candidates {
            if matches!(self.outcomes.get(&candidate.id), Some(Outcome::Saved)) {
                self.selected.remove(&candidate.id);
            } else if candidate.importable
                && (first_report || !previous_ids.contains(&candidate.id))
            {
                self.selected.insert(candidate.id.clone());
            }
        }
        self.report = Some(report);
    }
    pub(super) fn chrome(&self) -> (&'static str, &'static str, f32) {
        match self.page {
            Page::Sources => ("Import games", "Choose a source.", 550.),
            Page::List => (
                source_name(self.source),
                "Select games to copy.",
                (260. + self.candidates(self.source).len() as f32 * 74.).min(620.),
            ),
            Page::Progress => (
                "Importing games",
                "Completed games are saved individually.",
                540.,
            ),
            Page::Results => (
                "Import results",
                "Review the games added to Block.",
                (260. + self.submitted.len() as f32 * 74.).min(620.),
            ),
        }
    }
}

fn source_name(source: SourceKind) -> &'static str {
    match source {
        SourceKind::AdjacentMinecraft => "This folder · .minecraft",
        SourceKind::OfficialLauncher => "Minecraft Launcher",
    }
}

fn discovery_roots() -> DiscoveryRoots {
    let adjacent_minecraft = std::env::current_exe()
        .ok()
        .map(|path| super::setup_storage::application_directory(&path).join(".minecraft"));
    let official_launcher = directories::BaseDirs::new().map(|dirs| {
        #[cfg(target_os = "macos")]
        {
            dirs.home_dir()
                .join("Library/Application Support/minecraft")
        }
        #[cfg(target_os = "windows")]
        {
            dirs.data_dir().join(".minecraft")
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            dirs.home_dir().join(".minecraft")
        }
    });
    DiscoveryRoots {
        adjacent_minecraft,
        official_launcher,
    }
}

impl Block {
    pub(super) fn import_action_disabled(&self, action: &UiAction) -> bool {
        let state = &self.shell.import_games;
        match action {
            UiAction::ImportGames(Action::Import | Action::Retry) => {
                state.running || state.discovery.is_some() || state.pending().is_empty()
            }
            UiAction::ImportGames(Action::Toggle(id)) => {
                state.running
                    || state.report.as_ref().is_none_or(|report| {
                        !report
                            .candidates
                            .iter()
                            .any(|candidate| candidate.id == *id && candidate.importable)
                    })
                    || matches!(state.outcomes.get(id), Some(Outcome::Saved))
            }
            UiAction::ImportGames(
                Action::Done
                | Action::New
                | Action::Source(_)
                | Action::Sources
                | Action::View(_)
                | Action::All,
            ) => state.running,
            _ => false,
        }
    }
    pub(super) fn open_import_games(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.begin_import_discovery(window, cx, false);
    }

    /// Start searching for importable games from Welcome without showing the
    /// drawer yet. The resolved report decides whether the drawer is worth a
    /// detour: games open it, an empty scan continues to the overview.
    pub(super) fn start_welcome_import(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.begin_import_discovery(window, cx, true);
    }

    fn begin_import_discovery(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
        deferred: bool,
    ) {
        self.shell.import_games.window = Some(window.window_handle());
        if self.shell.import_games.running {
            return;
        }
        let request = uuid::Uuid::new_v4();
        self.shell.import_games.discovery = Some(request);
        self.shell.import_games.page = Page::Sources;
        self.shell.import_games.deferred_open = false;
        self.shell
            .import_games
            .motion
            .borrow_mut()
            .settle(Page::Sources);
        self.shell.import_games.error = None;
        // Without an engine no report will ever resolve a deferred scan, so
        // keep the drawer immediate there to surface the error.
        if let Some(engine) = &self.engine {
            engine.discover_games(request, discovery_roots());
            if deferred {
                self.shell.import_games.deferred_open = true;
                return;
            }
        } else {
            self.shell.import_games.discovery = None;
            self.shell.import_games.error = Some("Block's data directory is unavailable.".into());
        }
        self.shell
            .surfaces
            .open(Surface::Drawer(Drawer::ImportGames), window, cx);
    }

    /// Resolve a discovery report, deciding whether a deferred Welcome scan
    /// should open the drawer or continue past it. The caller applies the
    /// window work with [`Self::apply_deferred_discovery`].
    pub(super) fn resolve_game_discovery(
        &mut self,
        request: uuid::Uuid,
        report: DiscoveryReport,
    ) -> DeferredDiscovery {
        if self.shell.import_games.discovery != Some(request) {
            return DeferredDiscovery::Ignored;
        }
        let deferred = self.shell.import_games.deferred_open
            && self.shell.first_run.enabled
            && self.shell.first_run.stage == Stage::Welcome;
        let empty = report.candidates.is_empty();
        self.shell.import_games.deferred_open = false;
        self.receive_game_discovery(request, report);
        if !deferred {
            DeferredDiscovery::Ignored
        } else if empty {
            DeferredDiscovery::ContinueToOverview
        } else {
            DeferredDiscovery::OpenDrawer
        }
    }

    pub(super) fn apply_deferred_discovery(
        &mut self,
        decision: DeferredDiscovery,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if !self.shell.first_run.enabled || self.shell.first_run.stage != Stage::Welcome {
            return;
        }
        match decision {
            DeferredDiscovery::Ignored => {}
            DeferredDiscovery::OpenDrawer => {
                self.shell
                    .surfaces
                    .open(Surface::Drawer(Drawer::ImportGames), window, cx);
            }
            DeferredDiscovery::ContinueToOverview => self.first_run_overview(window, cx),
        }
    }

    pub(super) fn receive_game_discovery(&mut self, request: uuid::Uuid, report: DiscoveryReport) {
        let state = &mut self.shell.import_games;
        if state.discovery != Some(request) {
            return;
        }
        state.discovery = None;
        state.apply_discovery(report);
    }

    pub(super) fn receive_game_import(
        &mut self,
        request: uuid::Uuid,
        candidate: String,
        result: Result<(String, String), String>,
    ) {
        if self.shell.import_games.batch != Some(request) {
            return;
        }
        let outcome = match result {
            Ok((instance, profile)) => {
                self.selected_instance = Some(instance);
                self.selected_profile = Some(profile);
                Outcome::Saved
            }
            Err(error) => Outcome::Failed(error),
        };
        self.shell.import_games.outcomes.insert(candidate, outcome);
    }

    pub(super) fn finish_game_import(&mut self, request: uuid::Uuid) {
        let state = &mut self.shell.import_games;
        if state.batch != Some(request) {
            return;
        }
        let was_running = state.running;
        state.running = false;
        for id in &state.submitted {
            state
                .outcomes
                .entry(id.clone())
                .or_insert(Outcome::Cancelled);
        }
        if was_running {
            let saved = state
                .submitted
                .iter()
                .filter(|id| matches!(state.outcomes.get(*id), Some(Outcome::Saved)))
                .count();
            let failed = state
                .submitted
                .iter()
                .filter(|id| matches!(state.outcomes.get(*id), Some(Outcome::Failed(_))))
                .count();
            let cancelled = state.submitted.len().saturating_sub(saved + failed);
            let outcome = if state.cancel.is_cancelled() {
                "cancelled"
            } else if failed != 0 {
                "failed"
            } else {
                "succeeded"
            };
            if outcome == "failed" {
                tracing::error!(
                    event = "ui.import.finished",
                    view_id = self.view_id,
                    batch_id = %request,
                    outcome,
                    saved,
                    failed,
                    cancelled,
                );
            } else {
                tracing::info!(
                    event = "ui.import.finished",
                    view_id = self.view_id,
                    batch_id = %request,
                    outcome,
                    saved,
                    failed,
                    cancelled,
                );
            }
        }
        state.page = Page::Results;
    }

    pub(super) fn import_games_closed(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.shell.import_games.discovery = None;
        if self.shell.import_games.running {
            self.shell.import_games.cancel.cancel();
        }
        if self.shell.first_run.enabled && self.shell.first_run.stage == Stage::Welcome {
            self.first_run_overview(window, cx);
        }
    }

    pub(super) fn handle_import_games(
        &mut self,
        action: Action,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        match action {
            Action::Source(source) => {
                self.shell.import_games.choose_source(source);
            }
            Action::View(source) => {
                let now = cx.background_executor().now();
                let reduced = self.motion_preferences().reduced();
                let state = &mut self.shell.import_games;
                state.source = source;
                state.motion.borrow_mut().navigate(Page::List, now, reduced);
                state.page = Page::List;
            }
            Action::Sources => {
                let now = cx.background_executor().now();
                let reduced = self.motion_preferences().reduced();
                let state = &mut self.shell.import_games;
                state
                    .motion
                    .borrow_mut()
                    .navigate(Page::Sources, now, reduced);
                state.page = Page::Sources;
            }
            Action::Toggle(id) => {
                let state = &mut self.shell.import_games;
                let eligible = state.candidates(state.source).into_iter().any(|candidate| {
                    candidate.id == id
                        && candidate.importable
                        && !matches!(state.outcomes.get(&candidate.id), Some(Outcome::Saved))
                });
                if eligible && !state.running && !state.selected.remove(&id) {
                    state.selected.insert(id);
                }
            }
            Action::All => {
                if !self.shell.import_games.running {
                    self.shell.import_games.toggle_all();
                }
            }
            Action::Import | Action::Retry => {
                if self.shell.import_games.running {
                    return;
                }
                let candidates = self.shell.import_games.pending();
                if candidates.is_empty() {
                    return;
                }
                let request = uuid::Uuid::new_v4();
                let parent = self.proposed_import_parent(&candidates);
                let state = &mut self.shell.import_games;
                state.batch = Some(request);
                state.cancel = CancellationToken::new();
                state.running = true;
                state.page = Page::Progress;
                state.motion.borrow_mut().run = None;
                state.submitted = candidates
                    .iter()
                    .map(|candidate| candidate.id.clone())
                    .collect();
                for id in &state.submitted {
                    state.outcomes.remove(id);
                }
                if let Some(engine) = &self.engine {
                    tracing::info!(
                        event = "ui.import.started",
                        view_id = self.view_id,
                        batch_id = %request,
                        count = candidates.len(),
                    );
                    engine.import_games(
                        request,
                        candidates,
                        parent,
                        self.saved_preferences.game.clone(),
                        self.shell.import_games.cancel.clone(),
                    );
                } else {
                    self.shell.import_games.running = false;
                    self.shell.import_games.error =
                        Some("Block's data directory is unavailable.".into());
                }
            }
            Action::Stop => {
                self.shell.import_games.cancel.cancel();
            }
            Action::Done | Action::New => {
                if self.shell.import_games.running {
                    return;
                }
                self.shell.surfaces.close(window, &self.focus, cx);
                self.first_run_overview(window, cx);
                if matches!(action, Action::New) {
                    self.handle_first_run(super::first_run::Action::Game, window, cx);
                }
            }
        }
        cx.notify();
    }

    fn proposed_import_parent(&self, candidates: &[GameCandidate]) -> PathBuf {
        let preferred = self.first_run_game_parent();
        let fallback = self
            .engine
            .as_ref()
            .map_or_else(|| preferred.clone(), |engine| engine.root.join("instances"));
        candidates.iter().fold(preferred, |parent, candidate| {
            super::setup_storage::import_parent(&parent, &candidate.game_directory, &fallback)
        })
    }

    fn render_sources(
        &self,
        state: &State,
        interactive: bool,
        cx: &Context<'_, Self>,
    ) -> gpui::AnyElement {
        let mut body = div().flex().flex_col().gap_3();
        if state.discovery.is_some() {
            body = body.child(note("Looking for existing games…"));
        }
        for source in [SourceKind::AdjacentMinecraft, SourceKind::OfficialLauncher] {
            let candidates = state.candidates(source);
            let count = candidates.len();
            let summary = match count {
                0 => "No games found.".to_owned(),
                1 => {
                    let candidate = candidates[0];
                    format!(
                        "1 game · {} · {} · {}",
                        candidate.name,
                        candidate
                            .minecraft_version
                            .as_deref()
                            .unwrap_or("Unknown version"),
                        candidate.loader.as_ref().map_or(
                            "Unknown loader".to_owned(),
                            |loader| format!("{:?}", loader.kind)
                        )
                    )
                }
                _ => format!("{count} games found"),
            };
            let source_path = state
                .report
                .as_ref()
                .and_then(|report| report.sources.iter().find(|item| item.kind == source));
            let mut row = div()
                .rounded_lg()
                .border_1()
                .border_color(rgb(if state.source == source {
                    self.palette.foreground
                } else {
                    self.palette.border
                }))
                .p_3()
                .flex()
                .flex_col()
                .gap_2()
                .child(if interactive {
                    self.button_with_variant(
                        format!("import-source-{source:?}"),
                        source_name(source),
                        UiAction::ImportGames(Action::Source(source)),
                        ButtonVariant::Ghost,
                        cx,
                    )
                    .w_full()
                    .justify_start()
                    .child(
                        if state.source == source {
                            Icon::Check
                        } else {
                            Icon::Folder
                        }
                        .render(16., self.palette.foreground),
                    )
                    .into_any_element()
                } else {
                    div()
                        .w_full()
                        .py_2()
                        .text_sm()
                        .child(source_name(source))
                        .into_any_element()
                })
                .child(note(summary));
            if let Some(source_path) = source_path {
                row = row.child(
                    div()
                        .text_xs()
                        .text_color(rgb(self.palette.muted))
                        .child(source_path.root.display().to_string()),
                );
                if let Some(warning) = &source_path.warning {
                    row = row.child(note(warning.clone()));
                }
            }
            if count > 1 {
                row = row.child(if interactive {
                    self.button_with_variant(
                        format!("import-view-{source:?}"),
                        format!("View {count} games →"),
                        UiAction::ImportGames(Action::View(source)),
                        ButtonVariant::Ghost,
                        cx,
                    )
                    .into_any_element()
                } else {
                    div()
                        .text_sm()
                        .text_color(rgb(self.palette.muted))
                        .child(format!("View {count} games"))
                        .into_any_element()
                });
            } else if count == 1
                && let Some(reason) = &candidates[0].unsupported_reason
            {
                row = row.child(note(reason.clone()));
            }
            body = body.child(row);
        }
        if let Some(error) = &state.error {
            body = body.child(note(error.clone()));
        }
        body.into_any_element()
    }

    fn render_list(
        &self,
        state: &State,
        interactive: bool,
        cx: &Context<'_, Self>,
    ) -> gpui::AnyElement {
        let candidates = state.candidates(state.source);
        let eligible = candidates
            .iter()
            .filter(|candidate| {
                candidate.importable
                    && !matches!(state.outcomes.get(&candidate.id), Some(Outcome::Saved))
            })
            .collect::<Vec<_>>();
        let all_selected = state.all_selected();
        let selected_count = eligible
            .iter()
            .filter(|candidate| state.selected.contains(&candidate.id))
            .count();
        let mut body = div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .child(if interactive {
                        self.button_with_variant(
                            "import-sources",
                            "← Sources",
                            UiAction::ImportGames(Action::Sources),
                            ButtonVariant::Ghost,
                            cx,
                        )
                        .into_any_element()
                    } else {
                        div()
                            .text_sm()
                            .text_color(rgb(self.palette.muted))
                            .child("← Sources")
                            .into_any_element()
                    })
                    .child(if interactive {
                        self.button_with_variant(
                            "import-select-all",
                            if all_selected {
                                "Clear selection"
                            } else {
                                "Select all"
                            },
                            UiAction::ImportGames(Action::All),
                            ButtonVariant::Ghost,
                            cx,
                        )
                        .into_any_element()
                    } else {
                        div()
                            .text_sm()
                            .text_color(rgb(self.palette.muted))
                            .child(if all_selected {
                                "Clear selection"
                            } else {
                                "Select all"
                            })
                            .into_any_element()
                    }),
            )
            .child(note(format!("{selected_count} selected")));
        if candidates.is_empty() {
            body = body.child(note("No games were found for this source."));
        }
        for candidate in candidates {
            let saved = matches!(state.outcomes.get(&candidate.id), Some(Outcome::Saved));
            let checked = saved || state.selected.contains(&candidate.id);
            let disabled = !interactive || saved || !candidate.importable;
            let callback_id = candidate.id.clone();
            let checkbox = components::checkbox(
                format!("import-game-{}", candidate.id),
                candidate.name.clone(),
                checked,
                self.palette,
                ControlState {
                    disabled,
                    ..ControlState::default()
                },
                cx.listener(move |view, (): &(), window, cx| {
                    view.handle(
                        UiAction::ImportGames(Action::Toggle(callback_id.clone())),
                        window,
                        cx,
                    )
                }),
            );
            let status = if saved {
                "Imported".to_owned()
            } else if let Some(reason) = &candidate.unsupported_reason {
                reason.clone()
            } else {
                format!(
                    "{} · {}",
                    candidate
                        .minecraft_version
                        .as_deref()
                        .unwrap_or("Unknown version"),
                    candidate
                        .loader
                        .as_ref()
                        .map_or("Vanilla".to_owned(), |loader| format!("{:?}", loader.kind))
                )
            };
            body = body.child(
                div()
                    .border_b_1()
                    .border_color(rgb(self.palette.border))
                    .py_2()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(checkbox)
                    .child(note(status)),
            );
            if let Some(warning) = &candidate.metadata_warning {
                body = body.child(note(warning.clone()));
            }
        }
        if let Some(error) = &state.error {
            body = body.child(note(error.clone()));
        }
        body.into_any_element()
    }

    fn render_progress_results(&self, state: &State) -> gpui::AnyElement {
        let count = state
            .submitted
            .iter()
            .filter(|id| matches!(state.outcomes.get(*id), Some(Outcome::Saved)))
            .count();
        let mut body = div().flex().flex_col().gap_2().child(note(format!(
            "{count} of {} games imported",
            state.submitted.len()
        )));
        for id in &state.submitted {
            let candidate = state.report.as_ref().and_then(|report| {
                report
                    .candidates
                    .iter()
                    .find(|candidate| &candidate.id == id)
            });
            let label = candidate.map_or("Game", |candidate| candidate.name.as_str());
            let status = match state.outcomes.get(id) {
                Some(Outcome::Saved) => "Saved".to_owned(),
                Some(Outcome::Failed(error)) => error.clone(),
                Some(Outcome::Cancelled) => "Not imported".into(),
                None => "Waiting for copy".into(),
            };
            body = body.child(
                div()
                    .border_b_1()
                    .border_color(rgb(self.palette.border))
                    .py_2()
                    .child(label.to_owned())
                    .child(note(status)),
            );
        }
        if let Some(error) = &state.error {
            body = body.child(note(error.clone()));
        }
        body.into_any_element()
    }

    pub(super) fn render_import_games(
        &self,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> gpui::AnyElement {
        let state = &self.shell.import_games;
        let mut running_motion = false;
        let body = match state.page {
            Page::Sources | Page::List => {
                let (position, running) = state.motion.borrow_mut().sample(
                    cx.background_executor().now(),
                    self.motion_preferences().reduced(),
                );
                running_motion = running;
                let source = div()
                    .id("import-sources-content")
                    .size_full()
                    .scroll_area(&state.source_scroll, "Game sources", self.palette, false)
                    .child(self.render_sources(state, state.page == Page::Sources, cx));
                let list = div()
                    .id("import-list-content")
                    .size_full()
                    .scroll_area(&state.list_scroll, "Detected games", self.palette, false)
                    .child(self.render_list(state, state.page == Page::List, cx));
                div()
                    .id("import-games-body")
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_hidden()
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .left(relative(-position))
                            .w(relative(1.))
                            .min_h_0()
                            .overflow_hidden()
                            .child(source),
                    )
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .left(relative(1. - position))
                            .w(relative(1.))
                            .min_h_0()
                            .overflow_hidden()
                            .child(list),
                    )
                    .into_any_element()
            }
            Page::Progress | Page::Results => div()
                .id("import-games-body")
                .flex_1()
                .min_h_0()
                .scroll_area(&state.scroll, "Import results", self.palette, false)
                .child(self.render_progress_results(state))
                .into_any_element(),
        };
        if running_motion {
            window.request_animation_frame();
        }
        let mut footer = div().flex_shrink_0().pt_3().flex().flex_col().gap_2();
        if state.running {
            footer = footer
                .child(self.button(
                    "import-stop",
                    "Stop import",
                    UiAction::ImportGames(Action::Stop),
                    cx,
                ))
                .child(note("Completed games stay saved."));
        } else if state.page == Page::Results {
            if !state.pending().is_empty() {
                footer = footer.child(self.button_with_variant(
                    "import-retry",
                    "Retry remaining games",
                    UiAction::ImportGames(Action::Retry),
                    ButtonVariant::Primary,
                    cx,
                ));
            }
            footer = footer.child(self.button_with_variant(
                "import-done",
                "Continue setup",
                UiAction::ImportGames(Action::Done),
                ButtonVariant::Primary,
                cx,
            ));
        } else {
            let pending = state.pending();
            let n = pending.len();
            if n > 0 {
                let destination = format!(
                    "Copy to: {}",
                    self.proposed_import_parent(&pending).display()
                );
                footer = footer.child(super::tooltip::trigger(
                    div()
                        .id("import-destination")
                        .w_full()
                        .text_xs()
                        .text_color(rgb(self.palette.muted))
                        .truncate()
                        .child(destination.clone()),
                    destination,
                    cx,
                ));
                footer = footer.child(note("Original files stay in place."));
            }
            footer = footer.child(self.button_with_variant(
                "import-copy",
                format!("Import {n} {}", if n == 1 { "game" } else { "games" }),
                UiAction::ImportGames(Action::Import),
                ButtonVariant::Primary,
                cx,
            ));
            if state.page == Page::Sources {
                footer = footer.child(self.button_with_variant(
                    "import-new",
                    "Create a new game",
                    UiAction::ImportGames(Action::New),
                    ButtonVariant::Ghost,
                    cx,
                ));
            }
        }
        div()
            .id("import-games")
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .child(body)
            .child(footer)
            .into_any_element()
    }

    pub(super) fn import_header_back(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.shell.import_games.page == Page::List && !self.shell.import_games.running {
            self.handle_import_games(Action::Sources, window, cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(id: &str, source: SourceKind, importable: bool) -> GameCandidate {
        GameCandidate {
            id: id.into(),
            source,
            source_root: "/synthetic/root".into(),
            name: id.into(),
            game_directory: format!("/synthetic/{id}").into(),
            configured_version: Some("1.21.1".into()),
            minecraft_version: Some("1.21.1".into()),
            loader: None,
            latest_release: None,
            latest_snapshot: None,
            importable,
            unsupported_reason: (!importable).then(|| "unsupported fixture".into()),
            metadata_warning: None,
        }
    }

    #[test]
    fn source_list_motion_reverses_continuously_and_respects_reduced_motion() {
        let start = Instant::now();
        let mut motion = SourceListMotion::default();
        motion.navigate(Page::List, start, false);
        let at_100 = motion.sample(start + Duration::from_millis(100), false).0;
        assert!(at_100 > 0. && at_100 < 1.);
        motion.navigate(Page::Sources, start + Duration::from_millis(100), false);
        assert_eq!(
            motion.sample(start + Duration::from_millis(100), false).0,
            at_100
        );
        let returning = motion.sample(start + Duration::from_millis(140), false).0;
        assert!(returning < at_100);
        motion.navigate(Page::List, start + Duration::from_millis(140), false);
        assert_eq!(
            motion.sample(start + Duration::from_millis(140), false).0,
            returning
        );
        motion.navigate(Page::Sources, start + Duration::from_millis(150), true);
        assert_eq!(
            motion.sample(start + Duration::from_millis(150), true),
            (0., false)
        );
    }

    #[test]
    fn selection_all_clear_ignores_unsupported_and_saved_rows_and_survives_source_changes() {
        let mut state = State::default();
        let adjacent = SourceKind::AdjacentMinecraft;
        let launcher = SourceKind::OfficialLauncher;
        state.report = Some(DiscoveryReport {
            sources: Vec::new(),
            candidates: vec![
                candidate("a", adjacent, true),
                candidate("b", adjacent, true),
                candidate("bad", adjacent, false),
                candidate("saved", adjacent, true),
                candidate("other", launcher, true),
            ],
        });
        state.outcomes.insert("saved".into(), Outcome::Saved);
        state
            .selected
            .extend(["a".into(), "saved".into(), "other".into()]);
        assert!(!state.all_selected());
        state.toggle_all();
        assert!(state.all_selected());
        assert!(!state.selected.contains("bad"));
        assert!(state.selected.contains("other"));
        state.source = launcher;
        assert!(state.selected.contains("a"));
        assert!(state.selected.contains("other"));
        state.source = adjacent;
        state.toggle_all();
        assert!(!state.selected.contains("a") && !state.selected.contains("b"));
        assert!(state.selected.contains("saved"));
    }

    #[test]
    fn refreshed_discovery_preserves_old_choices_selects_new_games_and_drops_vanished_ids() {
        let adjacent = SourceKind::AdjacentMinecraft;
        let mut state = State {
            report: Some(DiscoveryReport {
                sources: Vec::new(),
                candidates: vec![
                    candidate("unchecked", adjacent, true),
                    candidate("checked", adjacent, true),
                    candidate("vanished", adjacent, true),
                    candidate("saved", adjacent, true),
                ],
            }),
            ..State::default()
        };
        state
            .selected
            .extend(["checked".into(), "vanished".into(), "saved".into()]);
        state.outcomes.insert("saved".into(), Outcome::Saved);

        state.apply_discovery(DiscoveryReport {
            sources: Vec::new(),
            candidates: vec![
                candidate("unchecked", adjacent, true),
                candidate("checked", adjacent, true),
                candidate("saved", adjacent, true),
                candidate("new", adjacent, true),
                candidate("unsupported-new", adjacent, false),
            ],
        });

        assert!(!state.selected.contains("unchecked"));
        assert!(state.selected.contains("checked"));
        assert!(!state.selected.contains("vanished"));
        assert!(!state.selected.contains("saved"));
        assert!(state.selected.contains("new"));
        assert!(!state.selected.contains("unsupported-new"));
    }

    #[test]
    fn explicitly_choosing_a_source_selects_its_single_eligible_game() {
        let adjacent = SourceKind::AdjacentMinecraft;
        let launcher = SourceKind::OfficialLauncher;
        let mut state = State {
            report: Some(DiscoveryReport {
                sources: Vec::new(),
                candidates: vec![
                    candidate("old-source", adjacent, true),
                    candidate("single", launcher, true),
                    candidate("unsupported", launcher, false),
                    candidate("saved", launcher, true),
                ],
            }),
            ..State::default()
        };
        state.selected.remove("single");
        state.outcomes.insert("saved".into(), Outcome::Saved);

        state.choose_source(launcher);

        assert_eq!(state.source, launcher);
        assert!(state.selected.contains("single"));
        assert!(!state.selected.contains("saved"));
        assert!(!state.selected.contains("unsupported"));
    }
}
