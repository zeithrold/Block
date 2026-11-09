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
    StatefulInteractiveElement as _, Styled as _, Window, div, relative, rgb,
};
use gpui_base::{
    TestSupportExt as _,
    motion::{Easing, Interpolate as _, Timing},
};

use super::{
    Block, UiAction,
    components::{self, ButtonVariant, ControlState},
    design::Icon,
    note,
    scroll_area::ScrollAreaExt as _,
    shell::{Drawer, Surface},
};
use crate::Language;

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
    Rescan,
    Done,
    New,
}

#[derive(Clone, Debug)]
pub(super) enum Outcome {
    Saved,
    Failed(String),
    Cancelled,
}

/// A background scan the setup center started only reports a count, so this
/// state never opens the drawer on its own.
#[derive(Clone, Debug)]
pub(super) struct State {
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
    pub scroll: ScrollHandle,
    pub source_scroll: ScrollHandle,
    pub list_scroll: ScrollHandle,
    motion: RefCell<SourceListMotion>,
    pub error: Option<crate::i18n::UiText>,
}

impl Default for State {
    fn default() -> Self {
        Self {
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
            scroll: ScrollHandle::new(),
            source_scroll: ScrollHandle::new(),
            list_scroll: ScrollHandle::new(),
            motion: RefCell::new(SourceListMotion::default()),
            error: None,
        }
    }
}

impl State {
    fn can_import(&self, candidate: &GameCandidate) -> bool {
        candidate.importable && !matches!(self.outcomes.get(&candidate.id), Some(Outcome::Saved))
    }

    pub(super) fn remaining_imports(&self) -> usize {
        self.report.as_ref().map_or(0, |report| {
            report
                .candidates
                .iter()
                .filter(|candidate| self.can_import(candidate))
                .count()
        })
    }

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
            .filter(|candidate| self.can_import(candidate) && self.selected.contains(&candidate.id))
            .cloned()
            .collect()
    }
    fn eligible_ids(&self) -> Vec<String> {
        self.candidates(self.source)
            .into_iter()
            .filter(|candidate| self.can_import(candidate))
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
    pub(super) fn chrome(&self, language: Language) -> (String, String, f32) {
        match self.page {
            Page::Sources => (
                tr!(language, "surface.import_games"),
                tr!(language, "import.header.choose"),
                620.,
            ),
            Page::List => (
                source_label(self.source, language),
                tr!(language, "import.header.select"),
                (260. + self.candidates(self.source).len() as f32 * 74.).min(620.),
            ),
            Page::Progress => (
                tr!(language, "import.header.importing"),
                tr!(language, "import.header.saved_individually"),
                540.,
            ),
            Page::Results => (
                tr!(language, "import.header.results"),
                tr!(language, "import.header.review"),
                (260. + self.submitted.len() as f32 * 74.).min(620.),
            ),
        }
    }
}

fn source_label(source: SourceKind, language: Language) -> String {
    let key = match source {
        SourceKind::AdjacentMinecraft => "import.source.adjacent",
        SourceKind::OfficialLauncher => "import.source.launcher",
    };
    tr!(language, key)
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
            UiAction::ImportGames(Action::Rescan) => state.running || state.discovery.is_some(),
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
        if !self.setup_incomplete()
            || (self.shell.import_games.report.is_none()
                && self.shell.import_games.discovery.is_none())
        {
            self.begin_import_discovery();
        }
        if !self.shell.import_games.running {
            self.shell.import_games.page = Page::Sources;
            self.shell
                .import_games
                .motion
                .borrow_mut()
                .settle(Page::Sources);
        }
        self.shell
            .surfaces
            .open(Surface::Drawer(Drawer::ImportGames), window, cx);
    }

    /// Scan the automatic sources for the setup center. The report only updates
    /// the count the center shows: the drawer stays closed until the user asks
    /// for it, so a discovered game never interrupts setup on its own.
    pub(super) fn scan_games_for_setup(&mut self) {
        self.begin_import_discovery();
    }

    /// Restart the sheet at its source page and issue one discovery request. The
    /// newest request owns the answer; an older report is dropped on arrival.
    fn begin_import_discovery(&mut self) {
        if self.shell.import_games.running {
            return;
        }
        let request = uuid::Uuid::new_v4();
        let state = &mut self.shell.import_games;
        state.discovery = Some(request);
        state.page = Page::Sources;
        state.motion.borrow_mut().settle(Page::Sources);
        state.error = None;
        let Some(engine) = &self.engine else {
            self.shell.import_games.discovery = None;
            self.shell.import_games.error = Some(msg!("import.error.dataDirectory"));
            return;
        };
        engine.discover_games(request, discovery_roots());
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

    /// Closing the sheet is not skipping setup. Only the work owned by the
    /// closed sheet stops; whatever the center already shows stays.
    pub(super) fn import_games_closed(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<'_, Self>,
    ) {
        if !self.setup_incomplete() {
            self.shell.import_games.discovery = None;
        }
        if self.shell.import_games.running {
            self.shell.import_games.cancel.cancel();
        }
    }

    pub(super) fn handle_import_games(
        &mut self,
        action: Action,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        match action {
            Action::Rescan => {
                self.begin_import_discovery();
                // Base disables the invoking button's focus target while busy.
                // Hand it to the stable sheet before the next render.
                if let Some(layer) = self
                    .shell
                    .surfaces
                    .top()
                    .filter(|layer| matches!(layer.kind, Surface::Drawer(Drawer::ImportGames)))
                {
                    layer.focus.focus(window, cx);
                }
            }
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
                let Some(engine) = &self.engine else {
                    self.shell.import_games.error = Some(msg!("import.error.dataDirectory"));
                    return;
                };
                if candidates.iter().any(|candidate| {
                    super::engine::import_overlaps_source(&engine.root, &parent, candidate)
                }) {
                    let error = msg!("setup.library.import_overlap");
                    if let Some(scope) = self.setup_import_library_scope() {
                        self.library_location_error(&scope, error);
                    } else {
                        self.shell.import_games.error = Some(error);
                    }
                    return;
                }
                if let Err(error) = self.saved_preferences.game.validate() {
                    self.shell.import_games.error = Some(msg!(
                        "setup.library.preflight_failed",
                        detail = format!("{error:#}")
                    ));
                    return;
                }
                if let Some(scope) = self.setup_import_library_scope()
                    && !self.commit_setup_library(&scope, cx)
                {
                    return;
                }
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
                    self.shell.import_games.error = Some(msg!("import.error.dataDirectory"));
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
                if self.setup_incomplete() {
                    self.first_run_overview(window, cx);
                    if matches!(action, Action::New) {
                        self.enter_setup_game_choice(cx);
                    }
                } else if matches!(action, Action::New) {
                    self.open_new_instance_setup(window, cx);
                }
            }
        }
        cx.notify();
    }

    /// Continue an ordinary source selection into a new game's Setup.
    ///
    /// Choosing a new game keeps the entry that started the session: an open
    /// draft is resumed as it is, so edits are never replaced, and only a
    /// missing session starts one. Setup is the destination, because nothing
    /// reaches the catalog until the user submits that draft.
    fn open_new_instance_setup(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if !matches!(self.form, Some(super::Form::Instance(_))) {
            self.handle(UiAction::CreateInstance, window, cx);
        }
        self.handle(
            UiAction::WizardKind(super::game_wizard::CreationKind::NewInstance),
            window,
            cx,
        );
    }

    fn proposed_import_parent(&self, candidates: &[GameCandidate]) -> PathBuf {
        let preferred = self.setup_import_library_scope().map_or_else(
            || self.first_run_game_parent(),
            |scope| self.setup_library_parent(&scope),
        );
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
        let language = self.language();
        let summary = match self.first_run_scan(language) {
            Some(super::first_run::Scan::Searching) => tr!(language, "import.searching"),
            Some(super::first_run::Scan::Failed(detail)) => detail,
            Some(super::first_run::Scan::Found(count)) => {
                tr!(language, "setup.import.found", count = count)
            }
            Some(super::first_run::Scan::Empty) => tr!(
                language,
                if state
                    .outcomes
                    .values()
                    .any(|outcome| matches!(outcome, Outcome::Saved))
                {
                    "setup.import.complete"
                } else {
                    "setup.import.none"
                }
            ),
            None => tr!(language, "import.header.choose"),
        };
        let body = div().flex().flex_col().gap_3().child(note(summary));
        let mut rows = div().flex().flex_col().gap_1();
        for source in [SourceKind::AdjacentMinecraft, SourceKind::OfficialLauncher]
            .into_iter()
            .filter(|_| state.report.is_some())
        {
            let candidates = state.candidates(source);
            let count = candidates.len();
            if count == 0 {
                continue;
            }
            let summary_count = if count > 1 {
                candidates
                    .iter()
                    .filter(|candidate| {
                        state.can_import(candidate) && state.selected.contains(&candidate.id)
                    })
                    .count()
            } else {
                count
            };
            let summary = tr!(
                language,
                if summary_count == 1 {
                    "import.source.one"
                } else {
                    "import.source.many"
                },
                count = summary_count
            );
            let path = state
                .report
                .as_ref()
                .and_then(|report| report.sources.iter().find(|path| path.kind == source));
            let selected = state.source == source;
            let label = source_label(source, language);
            let accessible_label = format!("{label}. {summary}");
            let mut content = div()
                .w_full()
                .min_w_0()
                .flex()
                .flex_col()
                .items_start()
                .gap_2();
            content = content.child(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        div()
                            .min_w_0()
                            .debug_selector(move || format!("import-source-title-{source:?}"))
                            .whitespace_normal()
                            .child(label.clone()),
                    )
                    .child(if interactive && count > 1 {
                        self.button_with_variant(
                            format!("import-view-{source:?}"),
                            summary,
                            UiAction::ImportGames(Action::View(source)),
                            ButtonVariant::Info,
                            cx,
                        )
                        .h(gpui::px(24.))
                        .flex_shrink_0()
                        .px_2()
                        .py_0()
                        .rounded_full()
                        .text_xs()
                        .line_height(gpui::px(16.))
                        .gap_1()
                        .child(Icon::ChevronRight.render(12., self.palette.info))
                        .into_any_element()
                    } else {
                        components::badge(
                            summary,
                            self.palette,
                            components::BadgeVariant::Secondary,
                        )
                        .self_start()
                        .into_any_element()
                    }),
            );
            content = content.children(path.map(|path| {
                div()
                    .w_full()
                    .min_w_0()
                    .text_xs()
                    .text_color(rgb(self.palette.muted))
                    .whitespace_normal()
                    .child(path.root.display().to_string())
            }));
            rows = rows.child(
                components::radio_row(
                    format!("import-source-{source:?}"),
                    "",
                    selected,
                    self.palette,
                    ControlState {
                        disabled: !interactive,
                        ..ControlState::default()
                    },
                    cx.listener(move |view, (): &(), window, cx| {
                        view.handle(UiAction::ImportGames(Action::Source(source)), window, cx);
                    }),
                )
                .aria_label(accessible_label)
                .w_full()
                .min_w_0()
                .h_auto()
                .flex_row_reverse()
                .gap_3()
                .child(content.flex_1()),
            );
            if selected {
                rows = rows.children(path.and_then(|path| path.warning.as_ref()).map(|warning| {
                    note(tr!(language, "common.diagnostic_detail", detail = warning))
                }));
                if let [candidate] = candidates.as_slice() {
                    rows = rows.children(candidate.unsupported_reason.as_ref().map(|reason| {
                        note(tr!(
                            language,
                            "common.diagnostic_detail",
                            detail = reason.clone()
                        ))
                    }));
                }
            }
        }
        body.child(rows).into_any_element()
    }

    fn render_list(
        &self,
        state: &State,
        interactive: bool,
        cx: &Context<'_, Self>,
    ) -> gpui::AnyElement {
        let candidates = state.candidates(state.source);
        let eligible = state.eligible_ids();
        let selected_count = eligible
            .iter()
            .filter(|id| state.selected.contains(*id))
            .count();
        let all_selected = state.all_selected();
        let language = self.language();
        let selection_label = if all_selected {
            tr!(language, "import.select.clear")
        } else {
            tr!(language, "import.select.all")
        };
        let mut body = div().flex().flex_col().gap_3().child(
            div()
                .flex()
                .justify_between()
                .items_center()
                .gap_2()
                .child({
                    let summary = tr!(
                        language,
                        "import.selectedCount",
                        count = selected_count,
                        total = eligible.len()
                    );
                    note(summary.clone())
                        .id("import-selection-count")
                        .debug_selector(|| "import-selection-count".into())
                        .aria_label(summary)
                        .min_w_0()
                        .flex_1()
                        .whitespace_normal()
                        .test_support()
                })
                .child(if interactive {
                    self.button_with_variant(
                        "import-select-all",
                        selection_label.clone(),
                        UiAction::ImportGames(Action::All),
                        ButtonVariant::Ghost,
                        cx,
                    )
                    .text_xs()
                    .into_any_element()
                } else {
                    div()
                        .text_xs()
                        .min_h(gpui::px(super::design::CONTROL_HEIGHT))
                        .flex()
                        .items_center()
                        .text_color(rgb(self.palette.muted))
                        .child(selection_label)
                        .into_any_element()
                }),
        );
        if candidates.is_empty() {
            body = body.child(note(tr!(language, "import.list.empty")));
        }
        let mut rows = div().flex().flex_col().gap_1();
        for candidate in candidates {
            let saved = matches!(state.outcomes.get(&candidate.id), Some(Outcome::Saved));
            let checked = saved || state.selected.contains(&candidate.id);
            let disabled = !interactive || saved || !candidate.importable;
            let callback_id = candidate.id.clone();
            let status = if saved {
                tr!(language, "import.status.imported")
            } else if let Some(reason) = &candidate.unsupported_reason {
                tr!(language, "import.cannot_import_detail", detail = reason)
            } else {
                let unknown_version = tr!(language, "import.unknownVersion");
                let version = candidate
                    .minecraft_version
                    .as_deref()
                    .unwrap_or(&unknown_version);
                let loader = candidate.loader.as_ref().map_or_else(
                    || tr!(language, "import.status.vanilla"),
                    |loader| format!("{:?}", loader.kind),
                );
                tr!(
                    language,
                    "import.status.version",
                    version = version,
                    loader = loader
                )
            };
            let mut content = div()
                .min_w_0()
                .flex_1()
                .flex()
                .flex_col()
                .gap_1()
                .whitespace_normal()
                .child(candidate.name.clone())
                .child(note(status));
            if let Some(warning) = &candidate.metadata_warning {
                content = content.child(note(tr!(
                    language,
                    "common.diagnostic_detail",
                    detail = warning
                )));
            }
            rows = rows.child(
                components::checkbox_row(
                    format!("import-game-{}", candidate.id),
                    "",
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
                )
                .aria_label(candidate.name.clone())
                .w_full()
                .min_w_0()
                .flex_row_reverse()
                .gap_3()
                .child(content),
            );
        }
        body = body.child(rows);
        if let Some(error) = &state.error {
            body = body.child(note(error.text(language)));
        }
        body.into_any_element()
    }

    fn render_progress_results(&self, state: &State) -> gpui::AnyElement {
        let language = self.language();
        let count = state
            .submitted
            .iter()
            .filter(|id| matches!(state.outcomes.get(*id), Some(Outcome::Saved)))
            .count();
        let mut body = div().flex().flex_col().gap_2().child(note(tr!(
            language,
            "import.progress.summary",
            count = count,
            total = state.submitted.len()
        )));
        for id in &state.submitted {
            let candidate = state.report.as_ref().and_then(|report| {
                report
                    .candidates
                    .iter()
                    .find(|candidate| &candidate.id == id)
            });
            let fallback = tr!(language, "import.label.game");
            let label = candidate.map_or(fallback.as_str(), |candidate| candidate.name.as_str());
            let status = match state.outcomes.get(id) {
                Some(Outcome::Saved) => tr!(language, "import.status.saved"),
                Some(Outcome::Failed(error)) => {
                    tr!(language, "import.failed_detail", detail = error)
                }
                Some(Outcome::Cancelled) => tr!(language, "import.status.notImported"),
                None => tr!(language, "import.status.waiting"),
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
            body = body.child(note(error.text(language)));
        }
        body.into_any_element()
    }

    pub(super) fn render_import_games(
        &self,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> gpui::AnyElement {
        let state = &self.shell.import_games;
        let language = self.language();
        let pending = state.pending();
        let library_scope = self.setup_import_library_scope();
        let destination_parent =
            (!pending.is_empty()).then(|| self.proposed_import_parent(&pending));
        let destination = destination_parent.as_ref().map(|parent| {
            tr!(
                language,
                "import.footer.destination",
                path = parent.display()
            )
        });
        let list_destination = match (&library_scope, &destination_parent) {
            (Some(scope), Some(parent)) => (super::setup_storage::comparable_path(parent)
                != super::setup_storage::comparable_path(&self.setup_library_parent(scope)))
            .then(|| {
                tr!(
                    language,
                    "setup.library.import_fallback",
                    path = parent.display()
                )
            }),
            _ => destination.clone(),
        };
        // Borrow focus paint room from the surrounding inset. Keep compensation
        // in scroll content so the outer paint remains reachable at its limits.
        let paint_outset = gpui::px(super::design::FOCUS_RING_SPREAD);
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
                    .debug_selector(|| "import-sources-content".into())
                    .size_full()
                    .pr_4()
                    .scroll_area(&state.source_scroll, "import.sources", self.palette, false)
                    .child(
                        div()
                            .w_full()
                            .pl(paint_outset)
                            .py(paint_outset)
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(self.render_sources(state, state.page == Page::Sources, cx))
                            .children(library_scope.clone().map(|scope| {
                                self.setup_library_location(scope, state.page == Page::Sources, cx)
                            }))
                            .children(list_destination.as_ref().map(|destination| {
                                note(destination.clone())
                                    .id("import-destination")
                                    .w_full()
                                    .whitespace_normal()
                            })),
                    );
                let list = div()
                    .id("import-list-content")
                    .debug_selector(|| "import-list-content".into())
                    .size_full()
                    .pr_4()
                    .scroll_area(&state.list_scroll, "import.list", self.palette, false)
                    .child(
                        div()
                            .w_full()
                            .pl(paint_outset)
                            .py(paint_outset)
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(self.render_list(state, state.page == Page::List, cx))
                            .children(library_scope.map(|scope| {
                                self.setup_library_location(scope, state.page == Page::List, cx)
                            }))
                            .children(list_destination.as_ref().map(|destination| {
                                note(destination.clone())
                                    .id("import-list-destination")
                                    .w_full()
                                    .whitespace_normal()
                            })),
                    );
                let page = |offset: f32, content: gpui::AnyElement| {
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left(relative(offset))
                        .w(relative(1.))
                        .min_h_0()
                        .overflow_hidden()
                        .child(content)
                };
                div()
                    .id("import-games-body")
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .ml(-paint_outset)
                    .mt(-paint_outset)
                    .mb(-paint_outset)
                    .mr(gpui::rems(-1.))
                    .overflow_hidden()
                    .child(page(-position, source.into_any_element()))
                    .child(page(1. - position, list.into_any_element()))
                    .into_any_element()
            }
            Page::Progress | Page::Results => div()
                .id("import-games-body")
                .flex_1()
                .min_h_0()
                .mr(gpui::rems(-1.))
                .pr_4()
                .scroll_area(&state.scroll, "import.results", self.palette, false)
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
                    tr!(language, "import.footer.stop"),
                    UiAction::ImportGames(Action::Stop),
                    cx,
                ))
                .child(note(tr!(language, "import.footer.completedStay")));
        } else if state.page == Page::Results {
            // At most one Primary answer: finishing leaves the sheet, retrying
            // only picks up the games that did not arrive.
            if !state.pending().is_empty() {
                footer = footer.child(self.button_with_variant(
                    "import-retry",
                    tr!(language, "import.footer.retry"),
                    UiAction::ImportGames(Action::Retry),
                    ButtonVariant::Outline,
                    cx,
                ));
            }
            footer = footer.child(self.button_with_variant(
                "import-done",
                tr!(language, "import.footer.continue"),
                UiAction::ImportGames(Action::Done),
                ButtonVariant::Primary,
                cx,
            ));
        } else {
            let n = pending.len();
            if n > 0 {
                footer = footer.child(note(tr!(language, "import.footer.originalFiles")));
            }
            if n > 0 || state.page != Page::Sources {
                footer = footer.child(self.button_with_variant(
                    "import-copy",
                    {
                        let import_key = if n == 1 {
                            "import.footer.importOne"
                        } else {
                            "import.footer.importMany"
                        };
                        tr!(language, import_key, count = n)
                    },
                    UiAction::ImportGames(Action::Import),
                    ButtonVariant::Primary,
                    cx,
                ));
            }
            if state.page == Page::Sources {
                footer = footer.child(self.button_with_variant(
                    "import-new",
                    tr!(language, "import.footer.newGame"),
                    UiAction::ImportGames(Action::New),
                    if n == 0 {
                        ButtonVariant::Primary
                    } else {
                        ButtonVariant::Ghost
                    },
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
    fn import_copy_resolves_per_language_and_keeps_arguments() {
        assert_eq!(
            source_label(SourceKind::AdjacentMinecraft, Language::English),
            "Local Minecraft folder"
        );
        assert_eq!(
            source_label(SourceKind::AdjacentMinecraft, Language::SimplifiedChinese),
            "本地 Minecraft 文件夹"
        );
        assert_eq!(
            source_label(SourceKind::OfficialLauncher, Language::Japanese),
            "Minecraft ランチャー"
        );
        assert_eq!(
            tr!(Language::English, "import.footer.importOne", count = 1),
            "Import 1 game"
        );
        assert_eq!(
            tr!(
                Language::SimplifiedChinese,
                "import.footer.importMany",
                count = 3
            ),
            "导入 3 个游戏"
        );
        assert_eq!(
            tr!(Language::Japanese, "import.source.many", count = 3),
            "3 ゲーム"
        );
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
        assert_eq!(state.pending().len(), 1);
        assert!(!state.all_selected());
        state.toggle_all();
        assert_eq!(state.pending().len(), 2);
        assert!(state.all_selected());
        assert!(!state.selected.contains("bad"));
        assert!(state.selected.contains("other"));
        state.source = launcher;
        assert_eq!(state.pending().len(), 1);
        assert!(state.selected.contains("a"));
        assert!(state.selected.contains("other"));
        state.source = adjacent;
        state.toggle_all();
        assert!(state.pending().is_empty());
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
