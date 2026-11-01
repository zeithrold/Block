//! Local modpack inspection, selection and installation inside the shared bottom
//! Drawer.
//!
//! The launcher owns every transition: this presentation derives what the
//! current [`State`] permits, shows what the pack declares, and dispatches
//! [`Action`]s for the host to apply. Rendering never changes the state it
//! reads.
#![expect(clippy::redundant_pub_crate, reason = "Private import presentation.")]

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use block_launcher::{
    CancellationToken,
    catalog::{LoaderKind, validate_name},
    modpack::{CurseFile, PackFile, PackFormat, PackInspection, PackSelection, TargetEvidence},
};
use gpui::{
    AnyElement, App, AppContext as _, ClipboardItem, Context, Entity, InteractiveElement as _,
    IntoElement as _, ParentElement as _, PathPromptOptions, ScrollHandle, Styled as _, Window,
    div, rgb,
};

use super::{
    Block, TextInput, UiAction,
    components::{self, BadgeVariant, ButtonVariant, ControlState, FieldOptions, ProgressState},
    note,
    scroll_area::ScrollAreaExt as _,
    select::ProductChoice,
    shell::{Drawer, Surface},
};
use crate::{
    Language, browser::CollectionLoad, engine::PackInstallRequest, game_wizard::LoaderCatalog,
};

/// One interaction inside the modpack drawer.
#[derive(Clone, Debug)]
pub(super) enum Action {
    /// Start a new import session from an entry point outside the drawer.
    Open,
    Choose,
    /// One optional pack file, named by its archive-relative path.
    ToggleFile(String),
    /// One optional CurseForge reference, named by project and file ID.
    ToggleCurse(u64, u64),
    Confirm,
    Stop,
    Retry,
    /// The official CurseForge page for a project, not a page for one file.
    OpenProject(u64),
    /// The identifier pair as CurseForge prints it: `<project>/<file>`.
    CopyId(String),
    /// One exact Minecraft version for a plain archive's target.
    SelectTargetVersion(String),
    /// One loader for a plain archive's target; `Vanilla` is an explicit choice.
    SelectTargetLoader(LoaderKind),
    /// One build of the chosen loader for the chosen Minecraft version.
    SelectTargetBuild(String),
    /// Ask the engine for the builds of the chosen loader again.
    ReloadTargetBuilds,
    Done,
}

/// What the drawer is showing. The host sets it; rendering only reports it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Phase {
    #[default]
    Choose,
    Inspecting,
    Review,
    Installing,
    Blocked,
    Complete,
}

impl Phase {
    /// The review is on screen, so its optional entries can still be chosen or
    /// dropped. A blocked import keeps them editable: the choice itself may be
    /// what blocks it.
    const fn reviewing(self) -> bool {
        matches!(self, Self::Review | Self::Blocked)
    }
}

/// A plain archive's target while the user is choosing it.
///
/// A plain archive can report evidence without declaring a manifest, so the
/// drawer asks for whichever decision that evidence did not settle. `None` in
/// `loader` means nothing was chosen yet; `Some(LoaderKind::Vanilla)` is the
/// deliberate "no mod loader" choice, which the core records without a loader
/// pin. Loader builds belong to one Minecraft version and one loader, so both
/// choices drop the build that was made for the previous pair.
#[derive(Clone, Debug)]
pub(super) struct TargetDraft {
    pub version: Option<String>,
    pub detected_version: Option<String>,
    pub loader: Option<LoaderKind>,
    pub build: Option<String>,
    pub catalog: Option<LoaderCatalog>,
    /// Identity of the outstanding build request, separate from the session's
    /// inspection/installation request.
    pub request: Option<uuid::Uuid>,
    pub cancel: CancellationToken,
    /// The inspection accepted exactly the choices this draft holds.
    pub applied: bool,
    pub error: Option<crate::i18n::UiText>,
}

impl Default for TargetDraft {
    fn default() -> Self {
        Self {
            version: None,
            detected_version: None,
            loader: None,
            build: None,
            catalog: None,
            request: None,
            cancel: CancellationToken::new(),
            applied: false,
            error: None,
        }
    }
}

impl TargetDraft {
    /// The choices still needed for this plain archive: a target the archive did
    /// not establish is the only reason to ask.
    fn for_inspection(inspection: &PackInspection) -> Option<Self> {
        (inspection.format == PackFormat::Plain && !inspection.target_confirmed()).then(|| Self {
            version: inspection.minecraft_version.clone(),
            detected_version: inspection.minecraft_version.clone(),
            ..Self::default()
        })
    }

    /// The target these choices describe, once nothing is missing. Vanilla is an
    /// explicit target without a loader build; every other loader needs one.
    fn resolved(&self) -> Option<(String, Option<(LoaderKind, String)>)> {
        let version = self.version.clone()?;
        let loader = self.loader.clone()?;
        let loader = match loader {
            LoaderKind::Vanilla => None,
            kind => Some((kind, self.build.clone()?)),
        };
        Some((version, loader))
    }

    /// The loader whose builds are still needed, when the choice needs one.
    fn build_loader(&self) -> Option<LoaderKind> {
        self.loader
            .clone()
            .filter(|kind| *kind != LoaderKind::Vanilla)
    }

    /// Replace the Minecraft version. Loader builds and the accepted target were
    /// made for the previous version, so both are dropped and the outstanding
    /// request is cancelled.
    fn select_version(&mut self, version: String) {
        self.version = Some(version);
        self.invalidate();
    }

    /// Replace the loader. Vanilla is the explicit choice without a build; every
    /// other loader starts from no build again.
    fn select_loader(&mut self, loader: LoaderKind) {
        self.loader = Some(loader);
        self.invalidate();
    }

    /// Replace the build: only a build from the loaded catalog is a choice, so
    /// the caller validates it first.
    fn select_build(&mut self, build: String) {
        self.build = Some(build);
        self.applied = false;
        self.error = None;
    }

    /// Drop everything that described the previous target.
    fn invalidate(&mut self) {
        self.build = None;
        self.catalog = None;
        self.request = None;
        self.cancel.cancel();
        self.cancel = CancellationToken::new();
        self.applied = false;
        self.error = None;
    }

    /// Whether `request` is the build request this draft is waiting for. A
    /// result for any other request is not this drawer's to apply.
    fn awaiting(&self, request: uuid::Uuid) -> bool {
        self.request == Some(request)
    }

    /// Apply a settled build list. Unknown builds are dropped rather than kept
    /// next to a list that no longer offers them.
    fn settle_builds(
        &mut self,
        request: uuid::Uuid,
        result: Result<Vec<block_launcher::install::fabric::LoaderVersion>, String>,
    ) -> bool {
        if !self.awaiting(request) {
            return false;
        }
        self.request = None;
        let Some(catalog) = self.catalog.as_mut() else {
            return true;
        };
        if catalog.request != request {
            return true;
        }
        match result {
            Ok(builds) => {
                catalog.builds = builds;
                catalog.load = CollectionLoad::Ready;
            }
            Err(error) => catalog.load = CollectionLoad::Failed(error.into()),
        }
        if self
            .build
            .as_ref()
            .is_some_and(|build| !catalog.builds.iter().any(|entry| &entry.version == build))
        {
            self.build = None;
            self.applied = false;
        }
        true
    }
}

/// Host-owned drawer state. Every field is set by the launcher, engine events or
/// the action handler; this module only reads them.
#[derive(Clone, Debug)]
pub(super) struct State {
    pub source: Option<PathBuf>,
    pub inspection: Option<PackInspection>,
    /// Present while a plain archive's target still needs explicit choices.
    pub draft: Option<TargetDraft>,
    pub selected_files: BTreeSet<String>,
    pub selected_curse_files: BTreeSet<(u64, u64)>,
    pub phase: Phase,
    pub error: Option<crate::i18n::UiText>,
    pub cancel: CancellationToken,
    pub request: Option<uuid::Uuid>,
    /// Stable ID for this import, including before the catalog commit.
    pub operation_id: Option<String>,
    /// Present only after the Instance has been committed or explicitly resumed.
    pub instance: Option<String>,
    /// The one Instance-name editor of this archive. It is created once per
    /// session, so the typed name, its selection and an ongoing composition
    /// survive every other choice in the drawer.
    pub name: Option<Entity<TextInput>>,
    /// Why the editor's current value cannot name an Instance; `None` while it
    /// can. The same answer gates Confirm and Retry.
    pub name_error: Option<crate::i18n::UiText>,
    /// The catalog's name for an Instance this session already created or is
    /// resuming. Its name is fixed, so the editor is read-only.
    pub instance_name: Option<String>,
    /// Tracker published by this pack request, if it is still the active one.
    pub tracker: Option<std::sync::Arc<std::sync::Mutex<crate::tasks::DownloadTracker>>>,
    pub scroll: ScrollHandle,
}

impl Default for State {
    fn default() -> Self {
        Self {
            source: None,
            inspection: None,
            draft: None,
            selected_files: BTreeSet::new(),
            selected_curse_files: BTreeSet::new(),
            phase: Phase::Choose,
            error: None,
            cancel: CancellationToken::new(),
            request: None,
            operation_id: None,
            instance: None,
            name: None,
            name_error: None,
            instance_name: None,
            tracker: None,
            scroll: ScrollHandle::new(),
        }
    }
}

/// Required files belong to the pack and are never turned off, and a path the
/// pack does not list as optional cannot be toggled at all.
fn optional_path(files: &[PackFile], path: &str) -> bool {
    files.iter().any(|file| !file.required && file.path == path)
}

/// A missing CurseForge entry can be chosen only when the pack marks it
/// optional; a required entry is part of the import already.
fn optional_curse(missing: &[CurseFile], project: u64, file: u64) -> bool {
    missing
        .iter()
        .any(|entry| !entry.required && entry.project_id == project && entry.file_id == file)
}

/// The entries the import would need but cannot verify: the pack requires them,
/// or the user chose them.
fn unresolved_entries<'a>(
    missing: &'a [CurseFile],
    selected: &BTreeSet<(u64, u64)>,
) -> Vec<&'a CurseFile> {
    missing
        .iter()
        .filter(|entry| entry.required || selected.contains(&(entry.project_id, entry.file_id)))
        .collect()
}

fn format_label(pack_format: PackFormat, language: Language) -> String {
    tr!(
        language,
        match pack_format {
            PackFormat::Modrinth => "modpack.format.modrinth",
            PackFormat::CurseForge => "modpack.format.curseforge",
            PackFormat::Prism => "modpack.format.prism",
            PackFormat::Plain => "modpack.format.plain",
        }
    )
}

fn loader_line(inspection: &PackInspection, language: Language) -> String {
    inspection.loader.as_ref().map_or_else(
        || {
            if inspection.format == PackFormat::Plain && inspection.target_confirmed() {
                loader_name(&LoaderKind::Vanilla, language)
            } else {
                tr!(language, "modpack.review.loader_unspecified")
            }
        },
        |(kind, version)| {
            if version.trim().is_empty() {
                tr!(language, "modpack.review.loader_only", loader = kind.name())
            } else {
                tr!(
                    language,
                    "modpack.review.loader",
                    loader = kind.name(),
                    version = version
                )
            }
        },
    )
}

/// Product names for the loaders a plain archive can target. Vanilla is one of
/// them: it is a deliberate choice, not the absence of a choice.
pub(super) fn loader_name(kind: &LoaderKind, language: Language) -> String {
    tr!(
        language,
        match kind {
            LoaderKind::Vanilla => "modpack.loader.vanilla",
            LoaderKind::Fabric => "modpack.loader.fabric",
            LoaderKind::Quilt => "modpack.loader.quilt",
            LoaderKind::Forge => "modpack.loader.forge",
            LoaderKind::NeoForge => "modpack.loader.neoforge",
        }
    )
}

/// The semantic status of a target the archive established without a manifest.
/// A manifest pack states its target in its own format, so it has none here.
fn target_status(evidence: &TargetEvidence, language: Language) -> Option<String> {
    match evidence {
        TargetEvidence::Manifest => None,
        TargetEvidence::Detected { source } => {
            Some(tr!(language, "modpack.target.detected", source = source))
        }
        TargetEvidence::NeedsSelection => Some(tr!(language, "modpack.target.needs_selection")),
        TargetEvidence::UserSelected => Some(tr!(language, "modpack.target.status_selected")),
    }
}

/// Why a value cannot name the new Instance, or `None` when it can.
///
/// The catalog owns that rule, and the editors trim the name they dispatch, so
/// the trimmed value is what the rule sees. Only the empty case has its own
/// copy: the catalog's message describes the accepted range, not the missing
/// step.
fn instance_name_error(value: &str) -> Option<crate::i18n::UiText> {
    let name = value.trim();
    if name.is_empty() {
        return Some(msg!("modpack.instance_name.required"));
    }
    validate_name(name)
        .err()
        .map(|_| msg!("modpack.instance_name.invalid"))
}

impl State {
    /// A step is in flight and still owns results for its request.
    const fn running(&self) -> bool {
        matches!(self.phase, Phase::Inspecting | Phase::Installing)
    }

    /// Whether a reported request is still the session's current one. A result
    /// for a closed, stopped or replaced step is discarded here, so it cannot
    /// overwrite what the drawer shows now.
    pub(super) fn accepts(&self, request: uuid::Uuid) -> bool {
        self.request == Some(request)
    }

    /// The optional entries chosen in this session.
    fn selection(&self) -> PackSelection {
        PackSelection {
            optional_files: self.selected_files.clone(),
            optional_curse_files: self.selected_curse_files.clone(),
        }
    }

    /// Flip one optional file. Required files and paths the pack does not list
    /// are not choices, so they cannot enter the selection.
    fn toggle_file(&mut self, path: &str) {
        if !self.toggleable_file(path) {
            return;
        }
        if !self.selected_files.remove(path) {
            self.selected_files.insert(path.to_owned());
        }
    }

    /// Flip one optional CurseForge entry, under the same rule as a file.
    fn toggle_curse(&mut self, project: u64, file: u64) {
        if !self.toggleable_curse(project, file) {
            return;
        }
        if !self.selected_curse_files.remove(&(project, file)) {
            self.selected_curse_files.insert((project, file));
        }
    }

    fn required_files(&self) -> Vec<&PackFile> {
        self.inspection
            .as_ref()
            .map_or_else(Vec::new, |inspection| {
                inspection
                    .files
                    .iter()
                    .filter(|file| file.required)
                    .collect()
            })
    }

    fn optional_files(&self) -> Vec<&PackFile> {
        self.inspection
            .as_ref()
            .map_or_else(Vec::new, |inspection| {
                inspection
                    .files
                    .iter()
                    .filter(|file| !file.required)
                    .collect()
            })
    }

    /// The entries that make the import impossible as reviewed.
    pub(super) fn unresolved(&self) -> Vec<&CurseFile> {
        self.inspection
            .as_ref()
            .map_or_else(Vec::new, |inspection| {
                unresolved_entries(&inspection.missing_files, &self.selected_curse_files)
            })
    }

    /// The target the installation would receive. A plain archive whose target
    /// still needs choices counts only once its current draft was accepted by
    /// the inspection; every other archive states its own target.
    fn target_ready(&self) -> bool {
        self.draft.as_ref().map_or_else(
            || {
                self.inspection
                    .as_ref()
                    .is_some_and(PackInspection::target_confirmed)
            },
            |draft| draft.applied,
        )
    }

    /// Drop the plain target draft and cancel the build request it owns. Every
    /// step that replaces the archive or ends the session calls this.
    fn reset_target(&mut self) {
        if let Some(draft) = self.draft.as_mut() {
            draft.cancel.cancel();
        }
        self.draft = None;
    }

    /// Begin inspecting this archive. Before an install starts, another archive
    /// has its own default name. Once an operation exists, reselecting its
    /// archive is a retry and retains the draft even if the file moved.
    fn replace_source(&mut self, source: &Path) {
        if self.operation_id.is_none() && self.source.as_deref() != Some(source) {
            self.name = None;
            self.name_error = None;
            self.selected_files.clear();
            self.selected_curse_files.clear();
        }
        self.source = Some(source.to_owned());
        self.inspection = None;
        self.reset_target();
        self.error = None;
        self.phase = Phase::Inspecting;
    }

    pub(super) fn toggleable_file(&self, path: &str) -> bool {
        self.instance.is_none()
            && self.phase.reviewing()
            && self
                .inspection
                .as_ref()
                .is_some_and(|inspection| optional_path(&inspection.files, path))
    }

    pub(super) fn toggleable_curse(&self, project: u64, file: u64) -> bool {
        self.instance.is_none()
            && self.phase.reviewing()
            && self
                .inspection
                .as_ref()
                .is_some_and(|inspection| optional_curse(&inspection.missing_files, project, file))
    }

    /// Install is offered only while reviewing, and only when nothing the import
    /// needs is unverifiable.
    fn can_confirm(&self) -> bool {
        self.phase == Phase::Review && self.can_start()
    }

    /// Whether the inspected archive and the chosen entries can be installed
    /// again: review, a retry after a failed installation and a resumed pack all
    /// start from the same conditions.
    fn installable(&self) -> bool {
        self.inspection.is_some()
            && self.source.is_some()
            && !self.running()
            && self.target_ready()
            && self.unresolved().is_empty()
    }

    /// An installation also needs a name the catalog accepts, so a chosen target
    /// and a resolved file list never dispatch an unusable one.
    fn can_start(&self) -> bool {
        self.installable() && self.name_ready()
    }

    /// Whether this session's editor currently holds a usable name.
    fn name_ready(&self) -> bool {
        self.name.is_some() && self.name_error.is_none()
    }

    /// Stop is offered once, and stays disabled while its cancellation settles.
    fn stoppable(&self) -> bool {
        self.running() && !self.cancel.is_cancelled()
    }

    fn row_count(&self) -> usize {
        self.inspection.as_ref().map_or(0, |inspection| {
            inspection.files.len() + inspection.missing_files.len()
        })
    }

    /// The sheet header copy for the current phase.
    pub(super) fn chrome(&self, language: Language) -> (String, String, f32) {
        let rows = self.row_count() as f32;
        // A plain target adds the version, loader and build controls above the
        // declared files.
        let target = if self.draft.is_some() { 160. } else { 0. };
        // The instance name sits above them, with room for its error.
        let name = if self.name.is_some() { 78. } else { 0. };
        match self.phase {
            Phase::Choose => (
                tr!(language, "modpack.header.choose.title"),
                tr!(language, "modpack.header.choose.description"),
                420.,
            ),
            Phase::Inspecting => (
                tr!(language, "modpack.header.inspecting.title"),
                tr!(language, "modpack.header.inspecting.description"),
                360.,
            ),
            Phase::Review => (
                tr!(language, "modpack.header.review.title"),
                tr!(language, "modpack.header.review.description"),
                (320. + rows * 52. + target + name).min(620.),
            ),
            Phase::Installing => (
                tr!(language, "modpack.header.installing.title"),
                tr!(language, "modpack.header.installing.description"),
                360.,
            ),
            Phase::Blocked => (
                tr!(language, "modpack.header.blocked.title"),
                tr!(language, "modpack.header.blocked.description"),
                (320. + rows * 52. + target + name).min(620.),
            ),
            Phase::Complete => (
                tr!(language, "modpack.header.complete.title"),
                tr!(language, "modpack.header.complete.description"),
                380.,
            ),
        }
    }
}

impl Block {
    /// Open a new local pack import. The drawer owns its session while it is on
    /// screen, so an entry point cannot discard the work it is doing.
    pub(super) fn open_pack_import(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.pack_import_open() {
            return;
        }
        self.replace_open_drawer(window, cx);
        self.shell.pack_import = State::default();
        self.shell
            .surfaces
            .open(Surface::Drawer(Drawer::ImportPack), window, cx);
        cx.notify();
    }

    /// Resume a pack installation that did not complete. The engine matches the
    /// archive against the committed instance and restores the selections it
    /// recorded, so only the source has to be chosen again.
    pub(super) fn resume_pack_import(
        &mut self,
        instance: String,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.pack_import_open() {
            return;
        }
        self.replace_open_drawer(window, cx);
        self.shell.pack_import = State {
            operation_id: Some(instance.clone()),
            instance: Some(instance),
            ..State::default()
        };
        self.shell
            .surfaces
            .open(Surface::Drawer(Drawer::ImportPack), window, cx);
        cx.notify();
    }

    /// The pack drawer is the one open sheet: a drawer already on screen hands
    /// it over instead of staying behind a second drawer that this layer cannot
    /// present.
    fn replace_open_drawer(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        while self
            .shell
            .surfaces
            .top()
            .is_some_and(|entry| matches!(entry.kind, Surface::Drawer(_)))
        {
            self.shell.surfaces.close(window, &self.focus, cx);
        }
    }

    fn pack_import_open(&self) -> bool {
        self.shell
            .surfaces
            .layers()
            .any(|entry| matches!(entry.kind, Surface::Drawer(Drawer::ImportPack)))
    }

    /// The catalog's name for this session's Instance, once it exists. A
    /// generated operation ID that the catalog does not hold yet is not one.
    fn committed_instance_name(&self) -> Option<String> {
        let instance = self.shell.pack_import.instance.as_ref()?;
        self.snapshot
            .instances
            .iter()
            .find(|candidate| &candidate.id == instance)
            .map(|candidate| candidate.name.clone())
    }

    /// Create this archive's Instance-name editor from the name it starts with:
    /// the catalog's own name once the Instance exists, otherwise the pack's
    /// name. A second inspection of the same session keeps the editor it has.
    fn adopt_pack_name(&mut self, archive_name: &str, cx: &mut Context<'_, Self>) {
        if self.shell.pack_import.name.is_some() {
            return;
        }
        let committed = self.committed_instance_name();
        let value = committed.clone().unwrap_or_else(|| archive_name.to_owned());
        // The pack's own name stays available as the placeholder once the field
        // is cleared.
        let field = cx.new(|cx| TextInput::new(&value, archive_name, cx));
        field.update(cx, |input, cx| input.set_read_only(committed.is_some(), cx));
        cx.observe(&field, |view, _, cx| {
            view.refresh_pack_name(cx);
            cx.notify();
        })
        .detach();
        self.shell.pack_import.name = Some(field);
        self.shell.pack_import.instance_name = committed;
        self.refresh_pack_name(cx);
    }

    /// Re-read the editor after an edit. The field and both dispatch buttons use
    /// this one answer, so they cannot disagree about the name.
    fn refresh_pack_name(&mut self, cx: &Context<'_, Self>) {
        let error = self
            .shell
            .pack_import
            .name
            .as_ref()
            .and_then(|field| instance_name_error(&field.read(cx).value()));
        self.shell.pack_import.name_error = error;
    }

    /// The Instance name an installation publishes: the edited value without the
    /// surrounding whitespace the editors ignore.
    fn pack_instance_name(&self, cx: &App) -> Option<String> {
        self.shell
            .pack_import
            .name
            .as_ref()
            .map(|field| field.read(cx).value().trim().to_owned())
    }

    /// Route one interaction inside the drawer. Every transition is applied
    /// here; rendering only reads the resulting state.
    pub(super) fn handle_pack_import(
        &mut self,
        action: Action,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        match action {
            Action::Open => {
                self.open_pack_import(window, cx);
                return;
            }
            Action::Choose => self.choose_pack_source(cx),
            Action::ToggleFile(path) => self.shell.pack_import.toggle_file(&path),
            Action::ToggleCurse(project, file) => {
                self.shell.pack_import.toggle_curse(project, file);
            }
            Action::Confirm => {
                if self.shell.pack_import.can_confirm() {
                    self.begin_pack_install(cx);
                }
            }
            Action::Stop => self.stop_pack_import(),
            Action::Retry => self.begin_pack_install(cx),
            Action::SelectTargetVersion(version) => self.choose_pack_target_version(version, cx),
            Action::SelectTargetLoader(kind) => self.choose_pack_target_loader(kind, cx),
            Action::SelectTargetBuild(version) => self.choose_pack_target_build(version, cx),
            Action::ReloadTargetBuilds => self.reload_pack_target_builds(cx),
            Action::OpenProject(project) => {
                // Only the official project page: no file page and no request.
                cx.open_url(&format!("https://www.curseforge.com/projects/{project}"));
            }
            Action::CopyId(id) => cx.write_to_clipboard(ClipboardItem::new_string(id)),
            Action::Done => {
                self.shell.surfaces.close(window, &self.focus, cx);
                if self.shell.first_run.enabled {
                    self.first_run_overview(window, cx);
                }
            }
        }
        cx.notify();
    }

    /// Choose the archive for this session. The request allocated here also owns
    /// the inspection, so dismissing the drawer while the picker is open cannot
    /// start reading a file afterwards.
    fn choose_pack_source(&mut self, cx: &mut Context<'_, Self>) {
        let state = &self.shell.pack_import;
        if state.request.is_some()
            || !matches!(state.phase, Phase::Choose | Phase::Review | Phase::Blocked)
        {
            return;
        }
        let request = uuid::Uuid::new_v4();
        self.shell.pack_import.request = Some(request);
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(tr!(self.language(), "modpack.choose.chooser").into()),
        });
        cx.spawn(async move |view, cx| {
            let selection = match prompt.await {
                Ok(Ok(paths)) => Ok(paths.and_then(|paths| paths.into_iter().next())),
                _ => Err(()),
            };
            let _ = view.update(cx, |view, cx| {
                view.finish_pack_browse(request, selection, cx)
            });
        })
        .detach();
    }

    fn finish_pack_browse(
        &mut self,
        request: uuid::Uuid,
        selection: Result<Option<PathBuf>, ()>,
        cx: &mut Context<'_, Self>,
    ) {
        if !self.shell.pack_import.accepts(request) {
            return;
        }
        self.shell.pack_import.request = None;
        let source = match selection {
            Ok(Some(source)) => source,
            Ok(None) => {
                cx.notify();
                return;
            }
            Err(()) => {
                self.shell.pack_import.error = Some(msg!("modpack.error.chooser"));
                cx.notify();
                return;
            }
        };
        let Some(engine) = self.engine.clone() else {
            self.shell.pack_import.phase = Phase::Blocked;
            self.shell.pack_import.error = Some(msg!("modpack.error.engine"));
            cx.notify();
            return;
        };
        let state = &mut self.shell.pack_import;
        state.replace_source(&source);
        state.request = Some(request);
        engine.inspect_pack(request, source, state.instance.clone());
        cx.notify();
    }

    /// Install the reviewed archive into this session's instance. The instance
    /// ID is generated once and reused by every retry, so a resumed pack keeps
    /// the record it already published, and the name comes from the drawer's one
    /// editor.
    fn begin_pack_install(&mut self, cx: &mut Context<'_, Self>) {
        if !self.shell.pack_import.can_start() {
            return;
        }
        let (Some(source), Some(inspection)) = (
            self.shell.pack_import.source.clone(),
            self.shell.pack_import.inspection.clone(),
        ) else {
            return;
        };
        // `can_start` accepted this name; retries keep a committed Instance's
        // own name because its editor is read-only.
        let Some(instance_name) = self.pack_instance_name(cx) else {
            return;
        };
        let Some(engine) = self.engine.clone() else {
            self.shell.pack_import.phase = Phase::Blocked;
            self.shell.pack_import.error = Some(msg!("modpack.error.engine"));
            cx.notify();
            return;
        };
        let instance = self
            .shell
            .pack_import
            .operation_id
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let request = uuid::Uuid::new_v4();
        let cancel = CancellationToken::new();
        let selection = self.shell.pack_import.selection();
        let state = &mut self.shell.pack_import;
        state.operation_id = Some(instance.clone());
        state.request = Some(request);
        state.cancel = cancel.clone();
        state.error = None;
        state.phase = Phase::Installing;
        tracing::info!(
            event = "ui.pack_install.started",
            view_id = self.view_id,
            request = %request,
        );
        engine.install_pack(PackInstallRequest {
            request,
            instance_id: instance,
            instance_name,
            source,
            inspection,
            selection,
            defaults: self.saved_preferences.game.clone(),
            cancel,
        });
        cx.notify();
    }

    /// Stop the current step: an inspection result is discarded, while an
    /// installation is cancelled and ends when the engine reports its result.
    fn stop_pack_import(&mut self) {
        let state = &mut self.shell.pack_import;
        match state.phase {
            Phase::Inspecting => {
                state.request = None;
                state.phase = Phase::Choose;
            }
            Phase::Installing => state.cancel.cancel(),
            Phase::Choose | Phase::Review | Phase::Blocked | Phase::Complete => {}
        }
    }

    /// Choose the exact Minecraft version for a plain archive's target. Only a
    /// version the launcher offers is a choice.
    fn choose_pack_target_version(&mut self, version: String, cx: &mut Context<'_, Self>) {
        if self.shell.pack_import.instance.is_some()
            || !self.shell.pack_import.phase.reviewing()
            || !self.versions.contains(&version)
        {
            return;
        }
        let Some(draft) = self.shell.pack_import.draft.as_mut() else {
            return;
        };
        if draft.version.as_ref() == Some(&version) {
            return;
        }
        draft.select_version(version);
        self.reload_pack_target_builds(cx);
        self.apply_pack_target();
        cx.notify();
    }

    /// Choose the loader for a plain archive's target. Vanilla is a choice with
    /// no build; every other loader needs its own build first.
    fn choose_pack_target_loader(&mut self, loader: LoaderKind, cx: &mut Context<'_, Self>) {
        if self.shell.pack_import.instance.is_some() || !self.shell.pack_import.phase.reviewing() {
            return;
        }
        let Some(draft) = self.shell.pack_import.draft.as_mut() else {
            return;
        };
        if draft.loader.as_ref() == Some(&loader) {
            return;
        }
        draft.select_loader(loader);
        self.reload_pack_target_builds(cx);
        self.apply_pack_target();
        cx.notify();
    }

    /// Choose one build of the loader and version currently selected. A build
    /// the loaded catalog does not list is not a choice.
    fn choose_pack_target_build(&mut self, build: String, cx: &mut Context<'_, Self>) {
        if self.shell.pack_import.instance.is_some() || !self.shell.pack_import.phase.reviewing() {
            return;
        }
        let Some(draft) = self.shell.pack_import.draft.as_mut() else {
            return;
        };
        let Some(kind) = draft.build_loader() else {
            return;
        };
        let Some(version) = draft.version.clone() else {
            return;
        };
        let offered = draft.catalog.as_ref().is_some_and(|catalog| {
            catalog.kind == kind
                && catalog.minecraft == version
                && matches!(catalog.load, CollectionLoad::Ready)
                && catalog.builds.iter().any(|entry| entry.version == build)
        });
        if !offered {
            return;
        }
        draft.select_build(build);
        self.apply_pack_target();
        cx.notify();
    }

    /// Start loading the builds of the chosen loader and version. Any earlier
    /// request is cancelled first, so its result cannot arrive for this target.
    fn reload_pack_target_builds(&mut self, cx: &mut Context<'_, Self>) {
        if self.shell.pack_import.instance.is_some() {
            return;
        }
        let Some(draft) = self.shell.pack_import.draft.as_mut() else {
            return;
        };
        draft.cancel.cancel();
        draft.cancel = CancellationToken::new();
        draft.request = None;
        draft.catalog = None;
        let (Some(version), Some(kind)) = (draft.version.clone(), draft.build_loader()) else {
            return;
        };
        let request = uuid::Uuid::new_v4();
        let cancel = CancellationToken::new();
        draft.catalog = Some(LoaderCatalog {
            request,
            minecraft: version.clone(),
            kind: kind.clone(),
            builds: Vec::new(),
            load: CollectionLoad::Loading,
        });
        draft.request = Some(request);
        draft.cancel = cancel.clone();
        if let Some(engine) = self.engine.clone() {
            engine.loader_versions(request, kind, version, cancel);
        } else if !self.visual_only {
            self.receive_pack_loader_versions(
                request,
                Err(tr!(self.language(), "modpack.target.builds_unavailable")),
                cx,
            );
        }
    }

    /// Store the current plain target in the inspection. The core rejects a
    /// version or loader pin it cannot install; the draft then stays incomplete
    /// and confirm stays blocked.
    fn apply_pack_target(&mut self) {
        let state = &mut self.shell.pack_import;
        let Some(draft) = state.draft.as_mut() else {
            return;
        };
        draft.applied = false;
        let Some((version, loader)) = draft.resolved() else {
            return;
        };
        let Some(inspection) = state.inspection.as_mut() else {
            return;
        };
        match inspection.select_target(version, loader) {
            Ok(()) => {
                draft.applied = true;
                draft.error = None;
            }
            Err(error) => {
                draft.error = Some(msg!("modpack.error.target", detail = format!("{error:#}")));
            }
        }
    }

    /// Apply a loader build result to this drawer. Returns whether the request
    /// belonged to the drawer: the host offers it elsewhere only when it did
    /// not, and a result for a cancelled or replaced choice is never applied.
    pub(super) fn receive_pack_loader_versions(
        &mut self,
        request: uuid::Uuid,
        result: Result<Vec<block_launcher::install::fabric::LoaderVersion>, String>,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        let Some(draft) = self.shell.pack_import.draft.as_mut() else {
            return false;
        };
        if !draft.settle_builds(request, result) {
            return false;
        }
        cx.notify();
        true
    }

    pub(super) fn receive_pack_inspected(
        &mut self,
        request: uuid::Uuid,
        source: PathBuf,
        result: Result<(PackInspection, PackSelection), String>,
        cx: &mut Context<'_, Self>,
    ) {
        if !self.shell.pack_import.accepts(request) {
            return;
        }
        let state = &mut self.shell.pack_import;
        state.request = None;
        state.source = Some(source);
        state.reset_target();
        let mut archive_name = None;
        match result {
            Ok((inspection, selection)) => {
                if state.instance.is_some() || state.operation_id.is_none() {
                    state.selected_files = selection.optional_files;
                    state.selected_curse_files = selection.optional_curse_files;
                } else {
                    state
                        .selected_files
                        .retain(|path| optional_path(&inspection.files, path));
                    state.selected_curse_files.retain(|&(project, file)| {
                        optional_curse(&inspection.missing_files, project, file)
                    });
                }
                state.draft = TargetDraft::for_inspection(&inspection);
                archive_name = Some(inspection.name.clone());
                state.inspection = Some(inspection);
                state.error = None;
                state.phase = Phase::Review;
            }
            Err(error) => {
                state.inspection = None;
                state.error = Some(msg!("modpack.error.inspect", detail = error));
                state.phase = Phase::Blocked;
            }
        }
        if let Some(archive_name) = archive_name {
            self.adopt_pack_name(&archive_name, cx);
        }
        cx.notify();
    }

    pub(super) fn receive_pack_created(
        &mut self,
        request: uuid::Uuid,
        instance: String,
        name: String,
        profile: String,
        cx: &mut Context<'_, Self>,
    ) {
        if !self.shell.pack_import.accepts(request) {
            return;
        }
        self.shell.pack_import.operation_id = Some(instance.clone());
        self.shell.pack_import.instance = Some(instance.clone());
        self.shell.pack_import.reset_target();
        // The Instance now carries the name this drawer published, so the field
        // shows exactly that name and a retry cannot rename it.
        let field = self.shell.pack_import.name.clone();
        if let Some(field) = field {
            field.update(cx, |input, cx| {
                input.set_value(&name, cx);
                input.set_read_only(true, cx);
            });
        }
        self.shell.pack_import.instance_name = Some(name);
        self.selected_instance = Some(instance);
        self.selected_profile = Some(profile);
        cx.notify();
    }

    pub(super) fn finish_pack_import(
        &mut self,
        request: uuid::Uuid,
        result: Result<(), String>,
        cx: &mut Context<'_, Self>,
    ) {
        if !self.shell.pack_import.accepts(request) {
            return;
        }
        self.release_pack_tracker();
        let state = &mut self.shell.pack_import;
        state.request = None;
        match result {
            Ok(()) => {
                state.error = None;
                state.phase = Phase::Complete;
                tracing::info!(
                    event = "ui.pack_install.finished",
                    view_id = self.view_id,
                    request = %request,
                    outcome = "succeeded",
                );
            }
            Err(error) => {
                state.error = Some(msg!("modpack.error.install", detail = error));
                state.phase = Phase::Blocked;
                tracing::warn!(
                    event = "ui.pack_install.finished",
                    view_id = self.view_id,
                    request = %request,
                    outcome = "failed",
                );
            }
        }
        cx.notify();
    }

    /// The drawer was dismissed. In-flight work is cancelled and its results no
    /// longer match this session; an instance the engine already committed stays
    /// in the catalog, where its pack can be resumed.
    pub(super) fn pack_import_closed(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.release_pack_tracker();
        let state = &mut self.shell.pack_import;
        state.request = None;
        state.cancel.cancel();
        state.reset_target();
        if self.shell.first_run.enabled
            && self.shell.first_run.stage == super::first_run::Stage::Welcome
        {
            self.first_run_overview(window, cx);
        }
    }

    fn release_pack_tracker(&mut self) {
        let Some(tracker) = self.shell.pack_import.tracker.take() else {
            return;
        };
        if self
            .downloads
            .as_ref()
            .is_some_and(|current| std::sync::Arc::ptr_eq(current, &tracker))
        {
            self.downloads = None;
            self.downloads_revision = 0;
            self.completed = 0;
        }
    }

    pub(super) fn render_modpack_import(
        &self,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let state = &self.shell.pack_import;
        let language = self.language();
        let mut content = div().flex().flex_col().gap_2();
        if let Some(source) = state.source.as_ref() {
            let archive = tr!(language, "modpack.archive", path = source.display());
            content = content.child(super::tooltip::trigger(
                div()
                    .id("modpack-archive")
                    .w_full()
                    .min_w_0()
                    .text_xs()
                    .text_color(rgb(self.palette.muted))
                    .truncate()
                    .child(archive.clone()),
                archive,
                cx,
            ));
        }
        content = content.child(match state.phase {
            Phase::Choose => self.modpack_choose_body(state, language),
            Phase::Inspecting => self.modpack_working_body(
                tr!(language, "modpack.inspecting.progress"),
                tr!(language, "modpack.inspecting.note"),
            ),
            Phase::Review => self.modpack_review_body(state, language, cx),
            Phase::Installing => self.modpack_working_body(
                tr!(language, "modpack.installing.progress"),
                tr!(language, "modpack.installing.note"),
            ),
            Phase::Blocked => self.modpack_blocked_body(state, language, cx),
            Phase::Complete => self.modpack_complete_body(state, language),
        });
        div()
            .id("modpack-import")
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .id("modpack-import-body")
                    .flex_1()
                    .min_h_0()
                    .scroll_area(&state.scroll, "modpack.body", self.palette, false)
                    .child(content),
            )
            .child(self.modpack_footer(state, language, cx))
            .into_any_element()
    }

    fn modpack_choose_body(&self, state: &State, language: Language) -> gpui::Div {
        // A session that already published its instance resumes from that
        // record: the engine restores the selections it saved, so the drawer
        // asks for the original archive instead of a new import.
        let source_note = if state.instance.is_some() && state.source.is_none() {
            "modpack.resume.prompt"
        } else {
            "modpack.choose.prompt"
        };
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(note(tr!(language, source_note)))
            .child(note(tr!(language, "modpack.choose.formats")))
            .children(self.modpack_error(state, language))
    }

    /// A retained failure resolves again in the language the drawer is painted
    /// in; diagnostic text passes through unchanged.
    fn modpack_error(
        &self,
        state: &State,
        language: Language,
    ) -> Option<gpui::Stateful<gpui::Div>> {
        state.error.as_ref().map(|error| note(error.text(language)))
    }

    /// No percentage is invented: the pack reports no measurable count here.
    fn modpack_working_body(&self, progress: String, detail: String) -> gpui::Div {
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(components::progress(
                ProgressState::Indeterminate,
                self.palette,
                progress,
            ))
            .child(note(detail))
    }

    fn modpack_review_body(
        &self,
        state: &State,
        language: Language,
        cx: &Context<'_, Self>,
    ) -> gpui::Div {
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(components::badge(
                tr!(language, "modpack.state.review"),
                self.palette,
                BadgeVariant::Secondary,
            ))
            .child(self.modpack_pack_details(state, language, cx))
            .children(self.modpack_error(state, language))
    }

    /// A blocked import keeps its review on screen, so a choice that blocks it
    /// can still be dropped.
    fn modpack_blocked_body(
        &self,
        state: &State,
        language: Language,
        cx: &Context<'_, Self>,
    ) -> gpui::Div {
        let unexplained = state.error.is_none() && state.unresolved().is_empty();
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(components::badge(
                        tr!(language, "modpack.state.blocked"),
                        self.palette,
                        BadgeVariant::Warning,
                    ))
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(tr!(language, "modpack.blocked.heading")),
                    ),
            )
            .children(self.modpack_error(state, language))
            .children(unexplained.then(|| note(tr!(language, "modpack.blocked.unsupported"))))
            .child(self.modpack_pack_details(state, language, cx))
    }

    fn modpack_complete_body(&self, state: &State, language: Language) -> gpui::Div {
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(components::badge(
                tr!(language, "modpack.state.complete"),
                self.palette,
                BadgeVariant::Success,
            ))
            // The instance carries the name this drawer published, which the
            // pack's own name no longer describes.
            .children(state.instance_name.as_ref().map(|name| {
                div()
                    .text_sm()
                    .child(tr!(language, "modpack.complete.instance", name = name))
            }))
    }

    /// The one name the new Instance will have. Its editor starts from the
    /// pack's own name, and the same catalog rule decides whether Confirm or
    /// Retry may use it.
    fn modpack_instance_name(
        &self,
        state: &State,
        language: Language,
    ) -> Option<gpui::Stateful<gpui::Div>> {
        let field = state.name.as_ref()?;
        // An instance the catalog already holds keeps its saved name, so it is
        // not a required choice any more.
        let locked = state.instance_name.is_some();
        Some(
            components::field_with(
                tr!(language, "games.instance_editor.name"),
                field,
                self.palette,
                FieldOptions {
                    hint: locked.then(|| tr!(language, "modpack.instance_name.locked").into()),
                    error: state
                        .name_error
                        .as_ref()
                        .map(|error| error.text(language).into()),
                    required: !locked,
                },
            )
            .id("modpack-instance-name")
            .debug_selector(|| "modpack-instance-name".into()),
        )
    }

    /// What the pack declares and every optional choice. Review and Blocked share
    /// it so a blocked import still shows what it needs.
    fn modpack_pack_details(
        &self,
        state: &State,
        language: Language,
        cx: &Context<'_, Self>,
    ) -> gpui::Div {
        let Some(inspection) = state.inspection.as_ref() else {
            return div().child(note(tr!(language, "modpack.review.no_inspection")));
        };
        let mut details = div()
            .flex()
            .flex_col()
            .gap_1()
            .children(self.modpack_instance_name(state, language))
            .child(
                div()
                    .id("modpack-pack-name")
                    .debug_selector(|| "modpack-pack-name".into())
                    .text_sm()
                    .child(tr!(
                        language,
                        "modpack.review.pack_name",
                        name = inspection.name.as_str()
                    )),
            )
            .children(
                state
                    .instance
                    .as_ref()
                    .map(|_| note(tr!(language, "modpack.selection.locked"))),
            )
            .child(self.modpack_target(state, inspection, language, cx))
            .child(note(tr!(
                language,
                "modpack.review.format",
                pack_format = format_label(inspection.format, language)
            )));
        if let Some(version) = inspection.version.as_ref() {
            details = details.child(note(tr!(
                language,
                "modpack.review.version",
                version = version
            )));
        }
        let required = state.required_files();
        let optional = state.optional_files();
        if required.is_empty() && optional.is_empty() && inspection.missing_files.is_empty() {
            details = details.child(note(tr!(language, "modpack.review.files_none")));
        }
        if !required.is_empty() {
            let heading = tr!(language, "modpack.review.required", count = required.len());
            details = details.child(self.modpack_section(heading));
            for file in required {
                details = details.child(self.modpack_required_row(file, language));
            }
        }
        if !optional.is_empty() {
            let heading = tr!(language, "modpack.review.optional", count = optional.len());
            details = details.child(self.modpack_section(heading));
            for file in optional {
                details = details.child(self.modpack_optional_row(state, file, cx));
            }
        }
        if !inspection.missing_files.is_empty() {
            details = details.child(self.modpack_unresolved_section(
                state,
                &inspection.missing_files,
                language,
                cx,
            ));
        }
        details
    }

    /// The Minecraft target: what the archive established, or the choices a
    /// plain archive still needs. A manifest pack states its own target.
    fn modpack_target(
        &self,
        state: &State,
        inspection: &PackInspection,
        language: Language,
        cx: &Context<'_, Self>,
    ) -> gpui::Div {
        let Some(draft) = state.draft.as_ref() else {
            let mut declared = div()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().text_sm().child(tr!(
                    language,
                    "modpack.review.minecraft",
                    version = inspection.minecraft_version.as_deref().unwrap_or_default()
                )))
                .child(div().text_sm().child(loader_line(inspection, language)));
            if let Some(status) = target_status(&inspection.target_evidence, language) {
                declared = declared.child(note(status));
            }
            if inspection.format == PackFormat::Plain
                && inspection.has_mods
                && state.instance.is_none()
            {
                declared =
                    declared.child(note(tr!(language, "modpack.target.mods_need_confirmation")));
            }
            return declared;
        };
        let mut section = div().flex().flex_col().gap_2().child(note(tr!(
            language,
            if draft.applied {
                "modpack.target.status_selected"
            } else {
                "modpack.target.needs_selection"
            }
        )));
        if inspection.has_mods {
            section = section.child(note(tr!(language, "modpack.target.mods_need_confirmation")));
        }
        if let Some(known) = &draft.detected_version {
            section = section.child(note(tr!(
                language,
                "modpack.target.version_hint",
                version = known
            )));
        }
        section = section
            .child(self.modpack_target_version(draft, language, cx))
            .child(self.modpack_target_loader(draft, language, cx));
        if let Some(kind) = draft.build_loader() {
            section = section.child(self.modpack_target_build(draft, &kind, language, cx));
        }
        section.children(draft.error.as_ref().map(|error| note(error.text(language))))
    }

    /// The exact Minecraft version. An unreadable version list keeps the known
    /// version on screen and offers the shared reload instead of an empty menu.
    fn modpack_target_version(
        &self,
        draft: &TargetDraft,
        language: Language,
        cx: &Context<'_, Self>,
    ) -> gpui::Div {
        if self.versions.is_empty() {
            let mut status = div()
                .flex()
                .flex_col()
                .gap_2()
                .child(note(self.modpack_version_status(language)));
            if !matches!(self.version_load, CollectionLoad::Loading) {
                status = status.child(components::button(
                    "modpack-reload-versions",
                    tr!(language, "modpack.target.reload_versions"),
                    self.palette,
                    ButtonVariant::Outline,
                    ControlState::default(),
                    cx.listener(|view, (): &(), window, cx| {
                        view.handle(UiAction::ReloadVersions, window, cx);
                    }),
                ));
            }
            return status;
        }
        let value = draft
            .version
            .clone()
            .unwrap_or_else(|| tr!(language, "modpack.target.version_unset"));
        self.product_select_field(
            "modpack-target-version",
            tr!(language, "modpack.target.version_label"),
            &value,
            ProductChoice::PackTargetVersion,
            cx,
        )
    }

    /// The loader, including Vanilla as a deliberate choice. Nothing is
    /// preselected: an unset loader is not a Vanilla target.
    fn modpack_target_loader(
        &self,
        draft: &TargetDraft,
        language: Language,
        cx: &Context<'_, Self>,
    ) -> gpui::Div {
        let value = draft.loader.as_ref().map_or_else(
            || tr!(language, "modpack.target.loader_unset"),
            |kind| loader_name(kind, language),
        );
        self.product_select_field(
            "modpack-target-loader",
            tr!(language, "modpack.target.loader_label"),
            &value,
            ProductChoice::PackTargetLoader,
            cx,
        )
    }

    /// The build of the chosen loader for the chosen version. Loading, failure,
    /// an empty catalog and retry never discard the version or the loader.
    fn modpack_target_build(
        &self,
        draft: &TargetDraft,
        kind: &LoaderKind,
        language: Language,
        cx: &Context<'_, Self>,
    ) -> gpui::Div {
        let section = div().flex().flex_col().gap_2();
        let Some(version) = draft.version.clone() else {
            return section.child(note(tr!(language, "modpack.target.build_needs_version")));
        };
        let loader = loader_name(kind, language);
        let catalog = draft
            .catalog
            .as_ref()
            .filter(|catalog| catalog.kind == *kind && catalog.minecraft == version);
        let Some(catalog) = catalog else {
            return section.child(note(tr!(
                language,
                "modpack.target.builds_loading",
                loader = loader,
                version = version
            )));
        };
        match &catalog.load {
            CollectionLoad::Loading => section.child(note(tr!(
                language,
                "modpack.target.builds_loading",
                loader = loader,
                version = version
            ))),
            CollectionLoad::Failed(error) => section
                .child(note(tr!(
                    language,
                    "modpack.target.builds_failed",
                    loader = loader,
                    detail = error.text(language)
                )))
                .child(self.modpack_retry_builds(language, cx)),
            CollectionLoad::Ready if catalog.builds.is_empty() => section
                .child(note(tr!(
                    language,
                    "modpack.target.builds_empty",
                    loader = loader,
                    version = version
                )))
                .child(self.modpack_retry_builds(language, cx)),
            CollectionLoad::Ready => {
                let value = draft
                    .build
                    .clone()
                    .unwrap_or_else(|| tr!(language, "modpack.target.build_unset"));
                section.child(self.product_select_field(
                    "modpack-target-build",
                    tr!(language, "modpack.target.build_label"),
                    &value,
                    ProductChoice::PackTargetBuild,
                    cx,
                ))
            }
        }
    }

    fn modpack_retry_builds(
        &self,
        language: Language,
        cx: &Context<'_, Self>,
    ) -> components::Control {
        self.modpack_action_button(
            "modpack-retry-builds",
            tr!(language, "modpack.target.retry_builds"),
            Action::ReloadTargetBuilds,
            ButtonVariant::Outline,
            false,
            cx,
        )
    }

    fn modpack_version_status(&self, language: Language) -> String {
        match &self.version_load {
            CollectionLoad::Loading => tr!(language, "modpack.target.version_loading"),
            CollectionLoad::Failed(error) => tr!(
                language,
                "modpack.target.version_failed",
                detail = error.text(language)
            ),
            CollectionLoad::Ready => tr!(language, "modpack.target.version_empty"),
        }
    }

    fn modpack_section(&self, heading: String) -> gpui::Div {
        div()
            .pt_2()
            .text_sm()
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .child(heading)
    }

    /// Required files appear as a stated fact, not as a control the user cannot
    /// operate.
    fn modpack_required_row(
        &self,
        file: &PackFile,
        language: Language,
    ) -> gpui::Stateful<gpui::Div> {
        div()
            .id(gpui::SharedString::from(format!(
                "modpack-required-{}",
                file.path
            )))
            .border_b_1()
            .border_color(rgb(self.palette.border))
            .py_2()
            .flex()
            .flex_col()
            .gap_1()
            .child(div().min_w_0().text_sm().child(file.path.clone()))
            .child(note(tr!(language, "modpack.review.required_included")))
    }

    fn modpack_optional_row(
        &self,
        state: &State,
        file: &PackFile,
        cx: &Context<'_, Self>,
    ) -> gpui::Div {
        let path = file.path.clone();
        let checked = state.selected_files.contains(&path);
        let disabled = !state.toggleable_file(&path);
        let dispatch = path.clone();
        div()
            .border_b_1()
            .border_color(rgb(self.palette.border))
            .py_2()
            .child(components::checkbox(
                format!("modpack-optional-{path}"),
                path,
                checked,
                self.palette,
                ControlState {
                    disabled,
                    ..ControlState::default()
                },
                cx.listener(move |view, (): &(), window, cx| {
                    view.handle(
                        UiAction::PackImport(Action::ToggleFile(dispatch.clone())),
                        window,
                        cx,
                    );
                }),
            ))
    }

    fn modpack_unresolved_section(
        &self,
        state: &State,
        missing: &[CurseFile],
        language: Language,
        cx: &Context<'_, Self>,
    ) -> gpui::Div {
        let heading = tr!(language, "modpack.review.unresolved", count = missing.len());
        let mut section = div()
            .flex()
            .flex_col()
            .gap_2()
            .pt_2()
            .child(self.modpack_section(heading))
            .child(note(tr!(language, "modpack.review.selection_note")));
        for entry in missing {
            section = section.child(self.modpack_curse_row(state, entry, language, cx));
        }
        section.child(note(tr!(language, "modpack.action.project_page_note")))
    }

    /// An identified entry whose file information never resolved. It names both
    /// IDs, offers the official project page, and says plainly what that page
    /// is not.
    fn modpack_curse_row(
        &self,
        state: &State,
        entry: &CurseFile,
        language: Language,
        cx: &Context<'_, Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let (project, file) = (entry.project_id, entry.file_id);
        let chosen = state.selected_curse_files.contains(&(project, file));
        let mut row = div()
            .id(gpui::SharedString::from(format!(
                "modpack-curse-{project}-{file}"
            )))
            .border_b_1()
            .border_color(rgb(self.palette.border))
            .py_2()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().min_w_0().flex_1().text_sm().child(tr!(
                        language,
                        "modpack.review.unresolved_entry",
                        project = project,
                        file = file
                    )))
                    .child(components::badge(
                        tr!(
                            language,
                            if entry.required {
                                "modpack.review.required_tag"
                            } else {
                                "modpack.review.optional_tag"
                            }
                        ),
                        self.palette,
                        if entry.required {
                            BadgeVariant::Secondary
                        } else {
                            BadgeVariant::Outline
                        },
                    )),
            );
        if !entry.required {
            row = row.child(components::checkbox(
                format!("modpack-curse-{project}-{file}"),
                tr!(language, "modpack.review.include_optional"),
                chosen,
                self.palette,
                ControlState {
                    disabled: !state.toggleable_curse(project, file),
                    ..ControlState::default()
                },
                cx.listener(move |view, (): &(), window, cx| {
                    view.handle(
                        UiAction::PackImport(Action::ToggleCurse(project, file)),
                        window,
                        cx,
                    );
                }),
            ));
        }
        if entry.required || chosen {
            row = row.child(note(tr!(language, "modpack.review.unavailable")));
        } else {
            row = row.child(note(tr!(language, "modpack.review.unverified_optional")));
        }
        row.child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(components::button(
                    format!("modpack-copy-id-{project}-{file}"),
                    tr!(language, "modpack.action.copy_id"),
                    self.palette,
                    ButtonVariant::Ghost,
                    ControlState::default(),
                    cx.listener(move |view, (): &(), window, cx| {
                        view.handle(
                            UiAction::PackImport(Action::CopyId(format!("{project}/{file}"))),
                            window,
                            cx,
                        );
                    }),
                ))
                .child(components::button(
                    format!("modpack-open-project-{project}"),
                    tr!(language, "modpack.action.open_project"),
                    self.palette,
                    ButtonVariant::Ghost,
                    ControlState::default(),
                    cx.listener(move |view, (): &(), window, cx| {
                        view.handle(
                            UiAction::PackImport(Action::OpenProject(project)),
                            window,
                            cx,
                        );
                    }),
                )),
        )
    }

    fn modpack_footer(
        &self,
        state: &State,
        language: Language,
        cx: &Context<'_, Self>,
    ) -> gpui::Div {
        let mut footer = div().flex_shrink_0().pt_3().flex().flex_col().gap_2();
        match state.phase {
            Phase::Choose => {
                footer = footer.child(self.modpack_action_button(
                    "modpack-choose",
                    tr!(language, "modpack.action.choose"),
                    Action::Choose,
                    ButtonVariant::Primary,
                    state.request.is_some(),
                    cx,
                ));
            }
            Phase::Inspecting | Phase::Installing => {
                footer = footer
                    .child(self.modpack_action_button(
                        "modpack-stop",
                        tr!(language, "modpack.action.stop"),
                        Action::Stop,
                        ButtonVariant::Outline,
                        !state.stoppable(),
                        cx,
                    ))
                    .children(
                        (state.phase == Phase::Installing)
                            .then(|| note(tr!(language, "modpack.installing.stop_note"))),
                    );
            }
            Phase::Review => {
                footer = footer
                    .child(self.modpack_action_button(
                        "modpack-confirm",
                        tr!(language, "modpack.action.confirm"),
                        Action::Confirm,
                        ButtonVariant::Primary,
                        !state.can_confirm(),
                        cx,
                    ))
                    .child(self.modpack_action_button(
                        "modpack-choose-other",
                        tr!(language, "modpack.action.choose_other"),
                        Action::Choose,
                        ButtonVariant::Ghost,
                        false,
                        cx,
                    ));
            }
            Phase::Blocked => {
                footer = footer
                    .child(self.modpack_action_button(
                        "modpack-retry",
                        tr!(language, "modpack.action.retry"),
                        Action::Retry,
                        ButtonVariant::Primary,
                        !state.can_start(),
                        cx,
                    ))
                    // The note explains the step a retry needs when this
                    // session can no longer start one on its own. An unfinished
                    // target or an unusable name is explained by its own control
                    // instead.
                    .children(
                        (!state.installable() && state.target_ready())
                            .then(|| note(tr!(language, "modpack.action.retry_note"))),
                    )
                    .child(self.modpack_action_button(
                        "modpack-choose-other",
                        tr!(language, "modpack.action.choose_other"),
                        Action::Choose,
                        ButtonVariant::Ghost,
                        false,
                        cx,
                    ));
            }
            Phase::Complete => {
                footer = footer.child(self.modpack_action_button(
                    "modpack-done",
                    tr!(language, "modpack.action.done"),
                    Action::Done,
                    ButtonVariant::Primary,
                    false,
                    cx,
                ));
            }
        }
        footer
    }

    fn modpack_action_button(
        &self,
        id: impl Into<gpui::SharedString>,
        label: String,
        action: Action,
        variant: ButtonVariant,
        disabled: bool,
        cx: &Context<'_, Self>,
    ) -> components::Control {
        components::button(
            id,
            label,
            self.palette,
            variant,
            ControlState {
                disabled,
                ..ControlState::default()
            },
            cx.listener(move |view, (): &(), window, cx| {
                view.handle(UiAction::PackImport(action.clone()), window, cx);
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use block_launcher::install::fabric::LoaderVersion;
    use gpui::EntityInputHandler as _;

    fn pack_file(path: &str, required: bool) -> PackFile {
        PackFile {
            path: path.to_owned(),
            sha1: String::from("synthetic"),
            sha512: String::from("synthetic"),
            size: 0,
            urls: Vec::new(),
            required,
        }
    }

    fn curse_entry(project_id: u64, file_id: u64, required: bool) -> CurseFile {
        CurseFile {
            project_id,
            file_id,
            required,
        }
    }

    #[gpui::test]
    fn cancelling_first_picker_allows_selecting_again(cx: &mut gpui::TestAppContext) {
        let (view, cx) = crate::test_support::window(cx, |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.preferences.reduce_motion = true;
            view
        });
        cx.update(|window, cx| view.update(cx, |view, cx| view.open_pack_import(window, cx)));
        cx.run_until_parked();
        let bounds = crate::test_support::control_bounds(cx, "modpack-choose");
        assert!(bounds.is_some(), "the initial picker button is visible");
        let Some(bounds) = bounds else {
            return;
        };
        cx.simulate_click(bounds.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| view.shell.pack_import.request.is_some()));
        cx.simulate_path_prompt_response(|_| None);
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| view.shell.pack_import.request.is_none()));

        cx.simulate_click(bounds.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        assert!(
            view.read_with(cx, |view, _| view.shell.pack_import.request.is_some()),
            "the rendered button must accept another picker request after cancellation"
        );
        cx.simulate_path_prompt_response(|_| None);
        cx.run_until_parked();
    }

    #[gpui::test]
    fn retry_before_catalog_creation_inspects_the_selected_archive(cx: &mut gpui::TestAppContext) {
        let result = (|| -> anyhow::Result<()> {
            let root = tempfile::tempdir()?;
            let source = root.path().join("pack.mrpack");
            crate::test_support::write_modpack(&source)?;
            let (engine, mut receiver) = crate::Engine::with_root(root.path().to_path_buf())?;
            let request = uuid::Uuid::new_v4();
            let operation = uuid::Uuid::new_v4().to_string();
            let (view, cx) = crate::test_support::window(cx, |window, cx| {
                let mut view = crate::tests::test_view(window, cx);
                view.engine = Some(engine);
                view.shell.pack_import.phase = Phase::Blocked;
                view.shell.pack_import.source = Some(source.clone());
                view.shell.pack_import.operation_id = Some(operation.clone());
                view.shell.pack_import.request = Some(request);
                view.shell
                    .pack_import
                    .selected_files
                    .insert(String::from("mods/optional.jar"));
                view.adopt_pack_name("My selected name", cx);
                view
            });
            view.update(cx, |view, cx| {
                view.finish_pack_browse(request, Ok(Some(source.clone())), cx);
            });
            let runtime = tokio::runtime::Runtime::new()?;
            let inspected = runtime.block_on(async {
                tokio::time::timeout(std::time::Duration::from_secs(5), async {
                    loop {
                        let message = receiver.recv().await.ok_or_else(|| {
                            anyhow::anyhow!("engine closed before pack inspection")
                        })?;
                        if let crate::Message::PackInspected {
                            request: actual,
                            result,
                            ..
                        } = message
                            && actual == request
                        {
                            return Ok::<_, anyhow::Error>(result);
                        }
                    }
                })
                .await?
            })?;
            assert!(
                inspected.is_ok(),
                "an uncommitted operation must inspect without a catalog instance: {inspected:?}"
            );
            view.update(cx, |view, cx| {
                view.receive_pack_inspected(request, source, inspected, cx);
                let state = &view.shell.pack_import;
                assert_eq!(state.operation_id.as_deref(), Some(operation.as_str()));
                assert!(state.instance.is_none());
                assert!(state.toggleable_file("mods/optional.jar"));
                assert!(
                    state.selected_files.contains("mods/optional.jar"),
                    "reinspection must preserve optional choices for this retry"
                );
                assert_eq!(
                    view.pack_instance_name(cx).as_deref(),
                    Some("My selected name")
                );
            });
            Ok(())
        })();
        assert!(result.is_ok(), "{result:?}");
    }

    #[gpui::test]
    fn committed_pack_keeps_its_saved_choices_after_failure(cx: &mut gpui::TestAppContext) {
        let result = (|| -> anyhow::Result<()> {
            let root = tempfile::tempdir()?;
            let source = root.path().join("pack.mrpack");
            crate::test_support::write_modpack(&source)?;
            let inspection = block_launcher::modpack::inspect(&source)?;
            let library = block_launcher::library::Library::new(
                root.path().to_path_buf(),
                std::sync::Arc::new(|_| {}),
                std::sync::Arc::new(|_| {}),
            )?;
            let mut selection = PackSelection::default();
            selection
                .optional_files
                .insert(String::from("mods/optional.jar"));
            let configuration = block_launcher::catalog::ProfileConfiguration {
                name: String::from("Saved"),
                ..Default::default()
            };
            let operation = uuid::Uuid::new_v4().to_string();
            let (instance, profile) = library.create_pack(
                &operation,
                &source,
                &inspection,
                &selection,
                configuration,
                &CancellationToken::new(),
            )?;
            let cancel = CancellationToken::new();
            cancel.cancel();
            let runtime = tokio::runtime::Runtime::new()?;
            let failure =
                match runtime.block_on(library.install_pack(&instance.id, &source, &cancel)) {
                    Ok(()) => anyhow::bail!("the cancelled pack installation must remain pending"),
                    Err(error) => error,
                };
            let request = uuid::Uuid::new_v4();
            let (view, cx) = crate::test_support::window(cx, |window, cx| {
                let mut view = crate::tests::test_view(window, cx);
                view.preferences.reduce_motion = true;
                view.shell.pack_import.inspection = Some(inspection);
                view.shell.pack_import.source = Some(source.clone());
                view.shell.pack_import.phase = Phase::Installing;
                view.shell.pack_import.request = Some(request);
                view.shell
                    .pack_import
                    .selected_files
                    .insert(String::from("mods/optional.jar"));
                view
            });
            view.update(cx, |view, cx| {
                view.shell.pack_import.phase = Phase::Review;
                view.shell.pack_import.operation_id = Some(uuid::Uuid::new_v4().to_string());
                assert!(view.shell.pack_import.toggleable_file("mods/optional.jar"));
                view.shell.pack_import.phase = Phase::Installing;
                view.receive_pack_created(
                    request,
                    instance.id.clone(),
                    instance.name.clone(),
                    profile.id.clone(),
                    cx,
                );
                view.finish_pack_import(request, Err(format!("{failure:#}")), cx);
                assert!(!view.shell.pack_import.toggleable_file("mods/optional.jar"));
                view.shell.pack_import.toggle_file("mods/optional.jar");
                assert!(
                    view.shell
                        .pack_import
                        .selection()
                        .optional_files
                        .contains("mods/optional.jar")
                );
            });
            let chosen = view.read_with(cx, |view, _| view.shell.pack_import.selection());
            let (_, saved) = library.inspect_pack(&source, Some(&instance.id))?;
            assert_eq!(chosen, saved, "the UI presents the persisted choices");
            assert!(
                runtime
                    .block_on(library.install_pack(&instance.id, &source, &cancel))
                    .is_err()
            );
            let (_, after_retry) = library.inspect_pack(&source, Some(&instance.id))?;
            assert_eq!(after_retry, saved, "a retry keeps the persisted choices");
            cx.simulate_resize(gpui::size(gpui::px(1100.), gpui::px(1000.)));
            cx.update(|window, cx| {
                view.update(cx, |view, cx| {
                    view.shell
                        .surfaces
                        .open(Surface::Drawer(Drawer::ImportPack), window, cx);
                    cx.notify();
                });
            });
            cx.run_until_parked();
            let node = cx.update(|window, _| {
                gpui_base::test_support::find(
                    window,
                    &[],
                    &gpui::ElementId::Name("modpack-optional-mods/optional.jar".into()),
                )
            });
            let Some(node) = node else {
                anyhow::bail!("the saved optional choice was not rendered");
            };
            assert_eq!(node.checked(), Some(true), "{node:?}");
            assert_eq!(node.disabled(), Some(true), "{node:?}");
            Ok(())
        })();
        assert!(result.is_ok(), "{result:?}");
    }

    #[gpui::test]
    fn committed_plain_pack_keeps_its_saved_target_after_failure(cx: &mut gpui::TestAppContext) {
        let result = (|| -> anyhow::Result<()> {
            let root = tempfile::tempdir()?;
            let source = root.path().join("plain.zip");
            crate::test_support::write_plain_pack(&source)?;
            let mut inspection = block_launcher::modpack::inspect(&source)?;
            inspection.select_target(String::from("1.20.1"), None)?;
            let mut draft = chosen(Some("1.20.1"), Some(LoaderKind::Vanilla), None);
            draft.applied = true;
            let request = uuid::Uuid::new_v4();
            let (view, cx) = crate::test_support::window(cx, |window, cx| {
                let mut view = crate::tests::test_view(window, cx);
                view.versions = vec![String::from("1.20.1"), String::from("1.20.6")];
                view.shell.pack_import.inspection = Some(inspection);
                view.shell.pack_import.draft = Some(draft);
                view.shell.pack_import.source = Some(source);
                view.shell.pack_import.phase = Phase::Installing;
                view.shell.pack_import.request = Some(request);
                view
            });
            view.update(cx, |view, cx| {
                view.receive_pack_created(
                    request,
                    String::from("instance-a"),
                    String::from("Saved"),
                    String::from("profile-a"),
                    cx,
                );
                view.finish_pack_import(request, Err(String::from("synthetic failure")), cx);
                view.choose_pack_target_version(String::from("1.20.6"), cx);
                assert_eq!(
                    view.shell
                        .pack_import
                        .inspection
                        .as_ref()
                        .and_then(|inspection| inspection.minecraft_version.as_deref()),
                    Some("1.20.1"),
                    "the saved target must not change after the catalog commit"
                );
                assert!(view.shell.pack_import.draft.is_none());
                assert_eq!(
                    view.shell
                        .pack_import
                        .inspection
                        .as_ref()
                        .map(|inspection| loader_line(inspection, Language::English)),
                    Some(String::from("Vanilla (no mod loader)"))
                );
            });
            Ok(())
        })();
        assert!(result.is_ok(), "{result:?}");
    }

    #[gpui::test]
    fn closed_pack_worker_cannot_replace_the_next_operation_tracker(cx: &mut gpui::TestAppContext) {
        let result = (|| -> anyhow::Result<()> {
            let root = tempfile::tempdir()?;
            let source = root.path().join("pack.mrpack");
            crate::test_support::write_modpack(&source)?;
            let inspection = block_launcher::modpack::inspect(&source)?;
            let catalog = block_launcher::catalog::Catalog::open(root.path())?;
            let (saved, _) = catalog.create_instance(
                String::from("Saved game"),
                String::from("1.20.1"),
                None,
                None,
            )?;
            let (engine, mut receiver) = crate::Engine::with_root(root.path().to_path_buf())?;
            let request = uuid::Uuid::new_v4();
            let cancel = CancellationToken::new();
            cancel.cancel();
            engine.install_pack(PackInstallRequest {
                request,
                instance_id: uuid::Uuid::new_v4().to_string(),
                instance_name: String::from("UI pack"),
                source,
                inspection,
                selection: PackSelection::default(),
                defaults: crate::Preferences::default().game,
                cancel,
            });
            let runtime = tokio::runtime::Runtime::new()?;
            let delayed = runtime.block_on(async {
                tokio::time::timeout(std::time::Duration::from_secs(5), async {
                    let mut messages = Vec::new();
                    loop {
                        let message = receiver.recv().await.ok_or_else(|| {
                            anyhow::anyhow!("engine closed before pack operation settled")
                        })?;
                        let finished = matches!(message, crate::Message::PackFinished { .. });
                        messages.push(message);
                        if finished {
                            return Ok::<_, anyhow::Error>(messages);
                        }
                    }
                })
                .await?
            })?;
            let (sender, receiver) = tokio::sync::mpsc::channel(delayed.len() + 2);
            for message in delayed {
                sender.try_send(message)?;
            }
            for message in [
                crate::Message::Status(String::from("old pack stage").into()),
                crate::Message::Log(String::from("old pack log")),
            ] {
                sender.try_send(crate::Message::ForPack {
                    request,
                    message: Box::new(message),
                })?;
            }
            let current = std::sync::Arc::new(std::sync::Mutex::new(
                crate::tasks::DownloadTracker::default(),
            ));
            let (view, cx) = crate::test_support::window(cx, |window, cx| {
                let mut view = crate::tests::test_view(window, cx);
                view.shell.pack_import.phase = Phase::Installing;
                view.shell.pack_import.request = Some(request);
                view.messages = Some(receiver);
                view
            });
            cx.update(|window, cx| {
                view.update(cx, |view, cx| {
                    view.pack_import_closed(window, cx);
                    view.shell.pack_import.phase = Phase::Installing;
                    view.shell.pack_import.request = Some(uuid::Uuid::new_v4());
                    view.downloads = Some(current.clone());
                    view.status = String::from("Current task").into();
                    view.poll(cx);
                    assert!(
                        view.downloads
                            .as_ref()
                            .is_some_and(|tracker| std::sync::Arc::ptr_eq(tracker, &current)),
                        "the old pack tracker replaced the current operation"
                    );
                    assert_eq!(view.status.text(Language::English), "Current task");
                    assert!(!view.logs.iter().any(|line| line.contains("old pack log")));
                    assert!(
                        view.snapshot
                            .instances
                            .iter()
                            .any(|item| item.id == saved.id),
                        "the real catalog refresh still applies"
                    );
                });
            });
            Ok(())
        })();
        assert!(result.is_ok(), "{result:?}");
    }

    #[gpui::test]
    fn closing_pack_detaches_only_its_own_live_tracker(cx: &mut gpui::TestAppContext) {
        let request = uuid::Uuid::new_v4();
        let (sender, receiver) = tokio::sync::mpsc::channel(2);
        let pack = std::sync::Arc::new(std::sync::Mutex::new(
            crate::tasks::DownloadTracker::default(),
        ));
        assert!(
            sender
                .try_send(crate::Message::ForPack {
                    request,
                    message: Box::new(crate::Message::Downloads(pack.clone())),
                })
                .is_ok()
        );
        let (view, cx) = crate::test_support::window(cx, |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.shell.pack_import.phase = Phase::Installing;
            view.shell.pack_import.request = Some(request);
            view.messages = Some(receiver);
            view
        });
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.poll(cx);
                assert!(
                    view.downloads
                        .as_ref()
                        .is_some_and(|tracker| std::sync::Arc::ptr_eq(tracker, &pack))
                );
                view.pack_import_closed(window, cx);
                assert!(view.downloads.is_none(), "the released tracker is detached");
            });
        });

        let next = std::sync::Arc::new(std::sync::Mutex::new(
            crate::tasks::DownloadTracker::default(),
        ));
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.shell.pack_import.tracker = Some(pack.clone());
                view.downloads = Some(next.clone());
                view.pack_import_closed(window, cx);
                assert!(
                    view.downloads
                        .as_ref()
                        .is_some_and(|tracker| std::sync::Arc::ptr_eq(tracker, &next)),
                    "closing the pack must not clear another operation's tracker"
                );
            });
        });
    }

    #[test]
    fn required_files_stay_included_and_unlisted_paths_are_not_toggleable() {
        let files = [
            pack_file("mods/core.jar", true),
            pack_file("mods/extra.jar", false),
        ];
        assert!(optional_path(&files, "mods/extra.jar"));
        assert!(!optional_path(&files, "mods/core.jar"));
        assert!(!optional_path(&files, "mods/absent.jar"));
        assert!(!optional_path(&[], "mods/extra.jar"));
    }

    #[test]
    fn only_required_or_chosen_curseforge_entries_block_completion() {
        let missing = [
            curse_entry(100, 1, true),
            curse_entry(200, 2, false),
            curse_entry(300, 3, false),
        ];
        let blocked = |selected: &BTreeSet<(u64, u64)>| {
            unresolved_entries(&missing, selected)
                .iter()
                .map(|entry| (entry.project_id, entry.file_id))
                .collect::<Vec<_>>()
        };
        let mut selected = BTreeSet::new();
        assert_eq!(blocked(&selected), vec![(100, 1)]);
        selected.insert((200, 2));
        assert_eq!(blocked(&selected), vec![(100, 1), (200, 2)]);
        selected.remove(&(200, 2));
        assert_eq!(blocked(&selected), vec![(100, 1)]);
        assert!(optional_curse(&missing, 200, 2));
        assert!(
            !optional_curse(&missing, 100, 1),
            "a required entry is already part of the import"
        );
        assert!(!optional_curse(&missing, 999, 9));
    }

    #[test]
    fn every_phase_has_catalog_chrome_in_each_language() {
        for phase in [
            Phase::Choose,
            Phase::Inspecting,
            Phase::Review,
            Phase::Installing,
            Phase::Blocked,
            Phase::Complete,
        ] {
            let state = State {
                phase,
                ..State::default()
            };
            for language in Language::ALL {
                let (title, description, height) = state.chrome(language);
                let context = format!("{phase:?}/{language:?}");
                assert!(!title.trim().is_empty(), "{context}");
                assert!(!description.trim().is_empty(), "{context}");
                assert!(!title.contains("modpack."), "{context}: {title}");
                assert!(
                    !description.contains("modpack."),
                    "{context}: {description}"
                );
                assert!(height > 0., "{context}");
            }
        }
    }

    fn reveal_pack_item(
        view: &Entity<Block>,
        cx: &mut gpui::VisualTestContext,
        selector: &'static str,
    ) -> anyhow::Result<gpui::Bounds<gpui::Pixels>> {
        let body = view.read_with(cx, |view, _| view.shell.pack_import.scroll.bounds());
        let mut item = crate::test_support::control_bounds(cx, selector)
            .ok_or_else(|| anyhow::anyhow!("{selector} is not rendered"))?;
        let delta = if item.bottom() > body.bottom() {
            body.bottom() - item.bottom() - gpui::px(8.)
        } else if item.top() < body.top() {
            body.top() - item.top() + gpui::px(8.)
        } else {
            gpui::px(0.)
        };
        if delta != gpui::px(0.) {
            let position = body.center();
            cx.simulate_mouse_move(position, None, gpui::Modifiers::none());
            cx.simulate_event(gpui::ScrollWheelEvent {
                position,
                delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.), delta)),
                ..gpui::ScrollWheelEvent::default()
            });
            cx.run_until_parked();
            cx.update(|window, _| window.refresh());
            cx.run_until_parked();
            item = crate::test_support::control_bounds(cx, selector)
                .ok_or_else(|| anyhow::anyhow!("{selector} disappeared after scrolling"))?;
        }
        anyhow::ensure!(
            item.left() >= body.left() - gpui::px(1.)
                && item.right() <= body.right() + gpui::px(1.)
                && item.top() >= body.top() - gpui::px(1.)
                && item.bottom() <= body.bottom() + gpui::px(1.),
            "{selector} cannot be revealed within the body: item={item:?} body={body:?}"
        );
        Ok(item)
    }

    #[gpui::test]
    fn long_pack_name_keeps_compact_review_usable_in_each_language(cx: &mut gpui::TestAppContext) {
        let result = (|| -> anyhow::Result<()> {
            let root = tempfile::tempdir()?;
            let source = root.path().join("pack.mrpack");
            crate::test_support::write_modpack(&source)?;
            let mut inspection = block_launcher::modpack::inspect(&source)?;
            let original_name = "模".repeat(64);
            inspection.name.clone_from(&original_name);
            let (view, cx) = crate::test_support::window(cx, |window, cx| {
                let mut view = crate::tests::test_view(window, cx);
                view.preferences.reduce_motion = true;
                view.preferences.appearance = crate::Appearance::Dark;
                view.palette = crate::Palette::DARK;
                view
            });
            cx.simulate_resize(gpui::size(gpui::px(1192.), gpui::px(760.)));
            cx.update(|window, cx| {
                view.update(cx, |view, cx| {
                    view.open_pack_import(window, cx);
                    view.shell.pack_import.inspection = Some(inspection);
                    view.shell.pack_import.source = Some(source);
                    view.shell.pack_import.phase = Phase::Review;
                    view.adopt_pack_name(&original_name, cx);
                    cx.notify();
                });
            });
            cx.run_until_parked();
            cx.simulate_resize(gpui::size(gpui::px(360.), gpui::px(540.)));
            let input = view
                .read_with(cx, |view, _| view.shell.pack_import.name.clone())
                .ok_or_else(|| anyhow::anyhow!("instance name input is missing"))?;

            for language in Language::ALL {
                input.update(cx, |input, cx| input.set_value(&original_name, cx));
                view.update(cx, |view, cx| {
                    view.preferences.language = Some(language);
                    view.shell
                        .pack_import
                        .scroll
                        .set_offset(gpui::point(gpui::px(0.), gpui::px(0.)));
                    cx.notify();
                });
                cx.run_until_parked();
                cx.update(|window, _| window.refresh());
                cx.run_until_parked();
                let title = crate::test_support::control_bounds(cx, "drawer-title")
                    .ok_or_else(|| anyhow::anyhow!("drawer title is not visible"))?;
                let footer = crate::test_support::control_bounds(cx, "modpack-confirm")
                    .ok_or_else(|| anyhow::anyhow!("confirm action is not visible"))?;
                let body = view.read_with(cx, |view, _| view.shell.pack_import.scroll.bounds());
                let viewport = cx.update(|window, _| window.viewport_size());
                assert_eq!(
                    view.read_with(cx, |view, _| view.shell.pack_import.chrome(language).0),
                    tr!(language, "modpack.header.review.title")
                );
                assert!(
                    title.size.height <= gpui::px(48.),
                    "{language:?}: the fixed header grows with the pack name: {title:?}"
                );
                assert!(
                    body.size.height >= gpui::px(120.),
                    "{language:?}: the body has no useful space: {body:?}"
                );
                assert!(
                    footer.top() >= body.bottom() && footer.bottom() <= viewport.height,
                    "{language:?}: the footer must remain reachable: {body:?} {footer:?}"
                );
                reveal_pack_item(&view, cx, "modpack-instance-name")?;
                input.update(cx, |input, cx| input.set_value("Native 日本語", cx));
                cx.run_until_parked();
                assert_eq!(
                    view.read_with(cx, |view, _| view
                        .shell
                        .pack_import
                        .inspection
                        .as_ref()
                        .map(|inspection| inspection.name.clone())),
                    Some(original_name.clone()),
                    "{language:?}: editing the instance name must not change the pack name"
                );
                let confirm = cx.update(|window, _| {
                    gpui_base::test_support::find(window, &[], &"modpack-confirm".into())
                });
                assert!(
                    confirm.is_some_and(|button| button.disabled() != Some(true)),
                    "{language:?}: the valid instance name must allow installation"
                );
                let original = reveal_pack_item(&view, cx, "modpack-pack-name")?;
                assert!(
                    original.size.height > gpui::px(18.),
                    "{language:?}: the full pack name must wrap, not be truncated: {original:?}"
                );
                assert!(
                    tr!(
                        language,
                        "modpack.review.pack_name",
                        name = original_name.as_str()
                    )
                    .contains(&original_name)
                );
            }
            Ok(())
        })();
        assert!(result.is_ok(), "{result:?}");
    }

    /// A missing key renders as the localized key itself, which is never product
    /// copy.
    #[test]
    fn blocking_and_recovery_copy_is_localized_in_each_language() {
        for language in Language::ALL {
            let copies = [
                tr!(language, "modpack.review.unavailable"),
                tr!(language, "modpack.review.unverified_optional"),
                tr!(language, "modpack.action.copy_id"),
                tr!(language, "modpack.action.project_page_note"),
                tr!(language, "modpack.action.retry_note"),
                tr!(language, "modpack.action.import_pack"),
                tr!(language, "modpack.resume.prompt"),
            ];
            for copy in copies {
                assert!(!copy.trim().is_empty(), "{language:?}");
                assert!(!copy.contains("modpack."), "{language:?}: {copy}");
            }
        }
    }

    /// A result belongs to the step that asked for it. A closed, stopped or
    /// replaced step no longer matches, so its late result cannot overwrite the
    /// newer one.
    #[test]
    fn a_result_only_applies_to_the_step_that_requested_it() {
        let current = uuid::Uuid::new_v4();
        let stale = uuid::Uuid::new_v4();
        let mut state = State::default();
        assert!(!state.accepts(current), "no step is waiting for a result");
        state.request = Some(current);
        assert!(state.accepts(current));
        assert!(!state.accepts(stale), "a replaced step is not this one");
        state.request = None;
        assert!(!state.accepts(current), "the step was released");
    }

    #[test]
    fn stop_is_offered_once_per_step() {
        let mut state = State {
            phase: Phase::Installing,
            ..State::default()
        };
        assert!(state.stoppable());
        state.cancel.cancel();
        assert!(
            !state.stoppable(),
            "cancellation is settling and has no second press"
        );
        state.phase = Phase::Review;
        assert!(!state.stoppable(), "review has nothing to stop");
    }

    /// Nothing can be installed before an archive was inspected, in either the
    /// review step or a retry that kept its source.
    #[test]
    fn installation_needs_an_inspected_archive_and_a_review_step() {
        let mut state = State {
            source: Some(PathBuf::from("pack.mrpack")),
            phase: Phase::Review,
            ..State::default()
        };
        assert!(!state.can_start(), "no inspection was read");
        assert!(!state.can_confirm());
        state.phase = Phase::Installing;
        assert!(!state.can_start(), "a running step cannot start again");
    }

    /// Entries are only choices once the pack listing exists; the selected sets
    /// are exactly what the engine receives as the installation selection.
    #[test]
    fn choices_are_recorded_as_the_installation_selection() {
        let mut state = State {
            phase: Phase::Review,
            ..State::default()
        };
        state.toggle_file("mods/extra.jar");
        state.toggle_curse(200, 2);
        assert!(
            state.selected_files.is_empty() && state.selected_curse_files.is_empty(),
            "an uninspected pack offers no choices"
        );
        state.selected_files.insert("mods/extra.jar".to_owned());
        state.selected_curse_files.insert((200, 2));
        let selection = state.selection();
        assert!(selection.optional_files.contains("mods/extra.jar"));
        assert!(selection.optional_curse_files.contains(&(200, 2)));
    }

    fn chosen(
        version: Option<&str>,
        loader: Option<LoaderKind>,
        build: Option<&str>,
    ) -> TargetDraft {
        TargetDraft {
            version: version.map(str::to_owned),
            loader,
            build: build.map(str::to_owned),
            ..TargetDraft::default()
        }
    }

    fn loader_build(version: &str) -> LoaderVersion {
        LoaderVersion {
            version: version.to_owned(),
            stable: true,
        }
    }

    fn builds(
        kind: LoaderKind,
        minecraft: &str,
        request: uuid::Uuid,
        offered: Vec<LoaderVersion>,
        load: CollectionLoad,
    ) -> LoaderCatalog {
        LoaderCatalog {
            request,
            minecraft: minecraft.to_owned(),
            kind,
            builds: offered,
            load,
        }
    }

    /// An unset loader is not Vanilla: the core receives a loader pin only after
    /// the user picked a loader, and explicit Vanilla carries none.
    #[test]
    fn an_unset_loader_is_not_vanilla() {
        assert_eq!(chosen(None, None, None).resolved(), None);
        let mut target = chosen(Some("1.20.6"), None, None);
        assert_eq!(target.resolved(), None, "no loader was chosen");
        target.select_loader(LoaderKind::Vanilla);
        assert_eq!(
            target.resolved(),
            Some((String::from("1.20.6"), None)),
            "explicit Vanilla is a target without a loader pin"
        );
    }

    /// A modded loader needs one exact build of that loader for that version.
    #[test]
    fn a_modded_target_requires_an_explicit_build() {
        let mut target = chosen(Some("1.20.6"), None, None);
        target.select_loader(LoaderKind::Fabric);
        assert_eq!(target.resolved(), None, "the build is still missing");
        target.select_build(String::from("0.16.14"));
        assert_eq!(
            target.resolved(),
            Some((
                String::from("1.20.6"),
                Some((LoaderKind::Fabric, String::from("0.16.14")))
            ))
        );
    }

    /// Confirm follows the inspection's acceptance: choices that the core has not
    /// taken yet cannot start an installation. Only `apply_pack_target` sets
    /// `applied`, and only after `select_target` returned `Ok`.
    #[test]
    fn confirm_waits_for_the_inspection_to_accept_the_target() {
        let mut state = State {
            draft: Some(chosen(Some("1.20.6"), Some(LoaderKind::Vanilla), None)),
            ..State::default()
        };
        assert!(!state.target_ready(), "a choice alone is not a target");
        if let Some(draft) = state.draft.as_mut() {
            draft.applied = true;
        }
        assert!(state.target_ready());
    }

    /// Loader builds belong to one Minecraft version and one loader. Changing
    /// either drops the build and cancels the request that was loading it.
    #[test]
    fn a_version_or_loader_change_drops_the_previous_build() {
        let request = uuid::Uuid::new_v4();
        let mut changed_version = chosen(Some("1.20.6"), Some(LoaderKind::Fabric), Some("0.16.14"));
        changed_version.detected_version = Some("1.20.6".into());
        changed_version.applied = true;
        changed_version.request = Some(request);
        changed_version.catalog = Some(builds(
            LoaderKind::Fabric,
            "1.20.6",
            request,
            vec![loader_build("0.16.14")],
            CollectionLoad::Ready,
        ));
        let cancel = changed_version.cancel.clone();
        changed_version.select_version(String::from("1.21.1"));
        assert_eq!(changed_version.detected_version.as_deref(), Some("1.20.6"));
        assert_eq!(changed_version.build, None);
        assert!(changed_version.catalog.is_none());
        assert!(!changed_version.applied);
        assert!(
            cancel.is_cancelled(),
            "the request for the old version is cancelled"
        );

        let mut changed_loader = chosen(Some("1.20.6"), Some(LoaderKind::Fabric), Some("0.16.14"));
        changed_loader.applied = true;
        changed_loader.select_loader(LoaderKind::Quilt);
        assert_eq!(changed_loader.build, None);
        assert!(!changed_loader.applied);
        assert_eq!(
            changed_loader.loader,
            Some(LoaderKind::Quilt),
            "the loader choice itself is kept"
        );
    }

    /// A result settles only the request the drawer is awaiting; a second result
    /// for a settled request and one for a replaced target are both discarded.
    #[test]
    fn a_stale_loader_response_is_ignored() {
        let current = uuid::Uuid::new_v4();
        let stale = uuid::Uuid::new_v4();
        let mut target = chosen(Some("1.20.6"), Some(LoaderKind::Fabric), None);
        target.request = Some(current);
        target.catalog = Some(builds(
            LoaderKind::Fabric,
            "1.20.6",
            current,
            Vec::new(),
            CollectionLoad::Loading,
        ));

        assert!(!target.settle_builds(stale, Ok(vec![loader_build("0.16.14")])));
        assert_eq!(
            target.catalog.as_ref().map(|catalog| catalog.builds.len()),
            Some(0),
            "another request does not fill this catalog"
        );
        assert!(
            target
                .catalog
                .as_ref()
                .is_some_and(|catalog| matches!(catalog.load, CollectionLoad::Loading))
        );

        assert!(target.settle_builds(current, Ok(vec![loader_build("0.16.14")])));
        assert!(target.request.is_none());
        assert!(
            target
                .catalog
                .as_ref()
                .is_some_and(|catalog| matches!(catalog.load, CollectionLoad::Ready))
        );
        assert!(
            !target.settle_builds(current, Ok(vec![loader_build("0.17.0")])),
            "a settled request does not settle twice"
        );

        let replaced = uuid::Uuid::new_v4();
        target.request = Some(replaced);
        target.catalog = Some(builds(
            LoaderKind::Fabric,
            "1.20.6",
            replaced,
            Vec::new(),
            CollectionLoad::Loading,
        ));
        target.select_loader(LoaderKind::Quilt);
        assert!(
            !target.settle_builds(replaced, Ok(vec![loader_build("0.17.0")])),
            "a replaced loader no longer awaits the old request"
        );
    }

    /// A failed build request keeps the version and loader the user chose; the
    /// retry target is still there to try again.
    #[test]
    fn a_failed_build_request_keeps_the_chosen_version_and_loader() {
        let request = uuid::Uuid::new_v4();
        let mut target = chosen(Some("1.20.6"), Some(LoaderKind::Fabric), None);
        target.request = Some(request);
        target.catalog = Some(builds(
            LoaderKind::Fabric,
            "1.20.6",
            request,
            Vec::new(),
            CollectionLoad::Loading,
        ));
        assert!(target.settle_builds(request, Err(String::from("service unavailable"))));
        assert_eq!(target.version.as_deref(), Some("1.20.6"));
        assert_eq!(target.loader, Some(LoaderKind::Fabric));
        assert!(
            target
                .catalog
                .as_ref()
                .is_some_and(|catalog| matches!(catalog.load, CollectionLoad::Failed(_)))
        );
    }

    /// A build that a fresh catalog no longer offers stops being selected.
    #[test]
    fn a_build_missing_from_the_catalog_stops_being_the_target() {
        let request = uuid::Uuid::new_v4();
        let mut target = chosen(Some("1.20.6"), Some(LoaderKind::Fabric), Some("0.16.14"));
        target.applied = true;
        target.request = Some(request);
        target.catalog = Some(builds(
            LoaderKind::Fabric,
            "1.20.6",
            request,
            Vec::new(),
            CollectionLoad::Loading,
        ));
        assert!(target.settle_builds(request, Ok(vec![loader_build("0.17.0")])));
        assert_eq!(target.build, None);
        assert!(!target.applied);
    }

    /// Detection evidence names the path it came from in every language, while a
    /// manifest pack keeps stating its own target.
    #[test]
    fn detected_evidence_names_its_source_in_every_language() {
        let detected = TargetEvidence::Detected {
            source: String::from("overrides/mods"),
        };
        for language in Language::ALL {
            let status = target_status(&detected, language)
                .unwrap_or_else(|| String::from("missing detected status"));
            assert!(status.contains("overrides/mods"), "{language:?}: {status}");
            assert!(!status.contains("modpack."), "{language:?}: {status}");
            for evidence in [TargetEvidence::NeedsSelection, TargetEvidence::UserSelected] {
                let line = target_status(&evidence, language)
                    .unwrap_or_else(|| String::from("missing selection status"));
                assert!(!line.trim().is_empty(), "{language:?}");
                assert!(!line.contains("modpack."), "{language:?}: {line}");
            }
        }
        assert_eq!(
            target_status(&TargetEvidence::Manifest, Language::English),
            None
        );
    }

    /// Every parameterized target message keeps the same placeholders in all
    /// three languages.
    #[test]
    fn target_copy_keeps_its_placeholders_in_every_language() {
        let copies = [
            ("modpack.target.detected", "%{source}"),
            ("modpack.target.version_hint", "%{version}"),
            ("modpack.target.version_failed", "%{detail}"),
            ("modpack.target.builds_loading", "%{loader}"),
            ("modpack.target.builds_loading", "%{version}"),
            ("modpack.target.builds_failed", "%{loader}"),
            ("modpack.target.builds_failed", "%{detail}"),
            ("modpack.target.builds_empty", "%{loader}"),
            ("modpack.target.builds_empty", "%{version}"),
            ("modpack.target.build_preview", "%{version}"),
            ("modpack.error.target", "%{detail}"),
        ];
        for language in Language::ALL {
            for (key, placeholder) in copies {
                let copy = tr!(language, key);
                assert!(copy.contains(placeholder), "{language:?}/{key}: {copy}");
                assert!(!copy.contains("modpack."), "{language:?}/{key}: {copy}");
            }
        }
    }

    /// The headers and labels of the target controls are localized keys, never
    /// the key itself.
    #[test]
    fn target_labels_are_localized_in_each_language() {
        let keys = [
            "modpack.target.status_selected",
            "modpack.target.needs_selection",
            "modpack.target.mods_need_confirmation",
            "modpack.target.version_label",
            "modpack.target.version_unset",
            "modpack.target.version_loading",
            "modpack.target.version_empty",
            "modpack.target.reload_versions",
            "modpack.target.loader_label",
            "modpack.target.loader_unset",
            "modpack.target.build_label",
            "modpack.target.build_unset",
            "modpack.target.build_needs_version",
            "modpack.target.builds_unavailable",
            "modpack.target.retry_builds",
            "modpack.format.plain",
            "modpack.loader.vanilla",
            "modpack.loader.fabric",
            "modpack.loader.quilt",
            "modpack.loader.forge",
            "modpack.loader.neoforge",
        ];
        for language in Language::ALL {
            for key in keys {
                let copy = tr!(language, key);
                assert!(!copy.trim().is_empty(), "{language:?}/{key}");
                assert!(!copy.contains("modpack."), "{language:?}/{key}: {copy}");
            }
        }
    }

    /// The host routes `Message::LoaderVersions` here first: a request this
    /// drawer did not issue is reported as foreign, and only the current request
    /// fills the drawer's catalog.
    #[gpui::test]
    fn a_loader_result_is_only_this_drawers_when_its_request_is_current(
        cx: &mut gpui::TestAppContext,
    ) {
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        let current = uuid::Uuid::new_v4();
        let stale = uuid::Uuid::new_v4();
        view.update(cx, |view, cx| {
            view.shell.pack_import.draft =
                Some(chosen(Some("1.20.6"), Some(LoaderKind::Fabric), None));
            if let Some(draft) = view.shell.pack_import.draft.as_mut() {
                draft.request = Some(current);
                draft.catalog = Some(builds(
                    LoaderKind::Fabric,
                    "1.20.6",
                    current,
                    Vec::new(),
                    CollectionLoad::Loading,
                ));
            }

            assert!(!view.receive_pack_loader_versions(
                stale,
                Ok(vec![loader_build("0.16.14")]),
                cx
            ));
            assert!(view.receive_pack_loader_versions(
                current,
                Ok(vec![loader_build("0.16.14")]),
                cx
            ));
            let draft = view.shell.pack_import.draft.as_ref();
            let settled = draft.map(|draft| {
                (
                    draft.request,
                    draft.catalog.as_ref().map(|catalog| catalog.builds.len()),
                )
            });
            assert_eq!(settled, Some((None, Some(1))));
        });
    }

    /// The catalog is the only name rule, and the editors trim before they
    /// dispatch: a 192-byte CJK pack name is legitimate for the pack and still
    /// not a name an Instance may take.
    #[test]
    fn the_instance_name_follows_the_catalog_rule_after_trimming() {
        let key = |value: &str| instance_name_error(value).and_then(|error| error.key());
        assert_eq!(key(""), Some("modpack.instance_name.required"));
        assert_eq!(key("   "), Some("modpack.instance_name.required"));

        // 64 CJK characters are 192 UTF-8 bytes, over the catalog's 160-byte
        // limit; the byte count, not the character count, is what decides.
        let long = "包".repeat(64);
        assert_eq!(long.len(), 192);
        assert_eq!(key(&long), Some("modpack.instance_name.invalid"));

        let limit = "包".repeat(53) + "a";
        assert_eq!(limit.len(), 160);
        assert_eq!(key(&limit), None, "the limit itself is accepted");
        assert_eq!(
            key(&format!("  {limit}  ")),
            None,
            "surrounding whitespace is not part of the name"
        );
        let over = "包".repeat(53) + "ab";
        assert_eq!(over.len(), 161);
        assert_eq!(key(&over), Some("modpack.instance_name.invalid"));
        assert_eq!(key("Harbor 界"), None);
        assert_eq!(key("Harbor\u{7}"), Some("modpack.instance_name.invalid"));

        // Both answers are product copy in every language, not a key.
        for language in Language::ALL {
            for message in [
                "modpack.instance_name.required",
                "modpack.instance_name.invalid",
            ] {
                let copy = tr!(language, message);
                assert!(!copy.trim().is_empty(), "{language:?}/{message}");
                assert!(!copy.contains("modpack."), "{language:?}/{message}: {copy}");
            }
        }
    }

    /// One archive has one name editor: the pack's own name is shown exactly as
    /// the archive spells it, an edit through the editor re-evaluates it, and
    /// every other choice in the drawer keeps that editor and its draft.
    #[gpui::test]
    fn the_instance_name_editor_is_created_once_and_keeps_its_value(cx: &mut gpui::TestAppContext) {
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        let long = "包".repeat(64);
        view.update(cx, |view, cx| view.adopt_pack_name(&long, cx));
        let field = view.read_with(cx, |view, _| view.shell.pack_import.name.clone());
        assert!(field.is_some(), "an inspected archive owns a name editor");
        let Some(field) = field else {
            return;
        };
        assert_eq!(
            field.read_with(cx, |field, _| field.value()),
            long,
            "the pack's own name is shown unmodified"
        );
        assert_eq!(
            view.read_with(cx, |view, _| {
                let state = &view.shell.pack_import;
                (
                    state.name_error.as_ref().and_then(|error| error.key()),
                    state.name_ready(),
                )
            }),
            (Some("modpack.instance_name.invalid"), false)
        );

        // The user replaces it with a shorter valid name in the same editor.
        field.update(cx, |field, cx| field.set_value("Harbor 界", cx));
        cx.run_until_parked();
        assert!(
            view.read_with(cx, |view, _| view.shell.pack_import.name_ready()),
            "the edit re-evaluates the name and unblocks Confirm and Retry"
        );

        // Target choices and a second inspection of this session adopt no
        // second editor.
        view.update(cx, |view, cx| {
            view.shell.pack_import.phase = Phase::Review;
            view.shell.pack_import.source = Some(PathBuf::from("pack.mrpack"));
            view.shell.pack_import.draft = Some(chosen(None, None, None));
            view.choose_pack_target_version(String::from("1.20.6"), cx);
            view.choose_pack_target_loader(LoaderKind::Vanilla, cx);
            view.adopt_pack_name(&long, cx);
        });
        let kept = view.read_with(cx, |view, cx| {
            view.shell
                .pack_import
                .name
                .as_ref()
                .map(|field| (field.entity_id(), field.read(cx).value()))
        });
        assert_eq!(kept, Some((field.entity_id(), String::from("Harbor 界"))));
    }

    /// A pre-allocated operation ID is not a committed Instance, so the field
    /// stays editable and keeps what was typed; once the catalog holds the
    /// Instance, its own trimmed name is fixed and a retry cannot rename it.
    #[gpui::test]
    fn a_committed_instance_keeps_its_name_and_locks_the_editor(cx: &mut gpui::TestAppContext) {
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        let request = uuid::Uuid::new_v4();
        view.update(cx, |view, cx| {
            view.shell.pack_import.phase = Phase::Installing;
            view.shell.pack_import.request = Some(request);
            view.shell.pack_import.operation_id = Some(uuid::Uuid::new_v4().to_string());
            view.adopt_pack_name("Packed world", cx);
        });
        let field = view.read_with(cx, |view, _| view.shell.pack_import.name.clone());
        assert!(field.is_some(), "an inspected archive owns a name editor");
        let Some(field) = field else {
            return;
        };
        assert_eq!(
            field.read_with(cx, |field, _| field.value()),
            "Packed world"
        );
        assert_eq!(
            view.read_with(cx, |view, _| view.shell.pack_import.instance_name.clone()),
            None,
            "an operation ID the catalog does not hold is not a committed Instance"
        );

        // Nothing is committed yet, so the editor accepts typing.
        cx.update(|window, cx| {
            field.update(cx, |field, cx| {
                field.replace_text_in_range(None, "  ", window, cx);
            });
        });
        assert_eq!(
            field.read_with(cx, |field, _| field.value()),
            "Packed world  "
        );

        view.update(cx, |view, cx| {
            view.receive_pack_created(
                request,
                String::from("instance-x"),
                String::from("Saved catalog name"),
                String::from("profile-x"),
                cx,
            );
        });
        assert_eq!(
            field.read_with(cx, |field, _| field.value()),
            "Saved catalog name",
            "the field shows the name returned by the catalog"
        );
        assert_eq!(
            view.read_with(cx, |view, _| view.shell.pack_import.instance_name.clone()),
            Some(String::from("Saved catalog name"))
        );
        assert_eq!(
            view.read_with(cx, |view, cx| view.pack_instance_name(cx)),
            Some(String::from("Saved catalog name")),
            "a retry publishes the committed name"
        );
        assert_eq!(
            view.read_with(cx, |view, _| view.selected_instance.clone()),
            Some(String::from("instance-x"))
        );

        // Editing the field no longer applies.
        cx.update(|window, cx| {
            field.update(cx, |field, cx| {
                field.replace_text_in_range(None, "Renamed", window, cx);
            });
        });
        assert_eq!(
            field.read_with(cx, |field, _| field.value()),
            "Saved catalog name"
        );
    }

    /// A resumed pack starts from the name the catalog saved for that Instance,
    /// which is read-only, not from the name the archive still carries.
    #[gpui::test]
    fn a_resumed_instance_starts_from_its_saved_catalog_name(cx: &mut gpui::TestAppContext) {
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        view.update(cx, |view, cx| {
            view.snapshot = crate::test_support::ready_snapshot();
            view.shell.pack_import.phase = Phase::Review;
            view.shell.pack_import.instance = Some(String::from("instance-a"));
            view.adopt_pack_name("Packed world", cx);
        });
        let field = view.read_with(cx, |view, _| view.shell.pack_import.name.clone());
        assert!(field.is_some(), "a resumed archive owns a name editor");
        let Some(field) = field else {
            return;
        };
        assert_eq!(field.read_with(cx, |field, _| field.value()), "Test game");
        assert_eq!(
            view.read_with(cx, |view, _| view.shell.pack_import.instance_name.clone()),
            Some(String::from("Test game"))
        );

        cx.update(|window, cx| {
            field.update(cx, |field, cx| {
                field.replace_text_in_range(None, "Renamed", window, cx);
            });
        });
        assert_eq!(
            field.read_with(cx, |field, _| field.value()),
            "Test game",
            "the saved Instance name stays as the catalog recorded it"
        );
    }

    /// The editor belongs to the archive it shows. A cancelled picker and the
    /// same archive again keep the typed name, so a retry after a failed
    /// installation never loses it; another archive is another pack session.
    #[gpui::test]
    fn the_name_editor_belongs_to_the_chosen_archive(cx: &mut gpui::TestAppContext) {
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        let source = PathBuf::from("pack.mrpack");
        let request = uuid::Uuid::new_v4();
        view.update(cx, |view, cx| {
            view.shell.pack_import.replace_source(&source);
            // An inspection of this archive accepted the source above.
            view.shell.pack_import.phase = Phase::Review;
            view.shell.pack_import.request = Some(request);
            view.adopt_pack_name("Packed world", cx);
        });
        let field = view.read_with(cx, |view, _| view.shell.pack_import.name.clone());
        assert!(field.is_some(), "an inspected archive owns a name editor");
        let Some(field) = field else {
            return;
        };
        field.update(cx, |field, cx| field.set_value("My pack", cx));

        // A cancelled picker changes nothing about the session.
        view.update(cx, |view, cx| {
            view.finish_pack_browse(request, Ok(None), cx);
        });
        assert_eq!(
            view.read_with(cx, |view, cx| {
                let state = &view.shell.pack_import;
                (
                    state.phase,
                    state
                        .name
                        .as_ref()
                        .map(|field| (field.entity_id(), field.read(cx).value())),
                )
            }),
            (
                Phase::Review,
                Some((field.entity_id(), String::from("My pack")))
            )
        );

        // Re-selecting the same archive after a failed installation keeps the
        // edited name, because no Instance was committed.
        view.update(cx, |view, _| {
            view.shell.pack_import.replace_source(&source);
        });
        assert_eq!(
            view.read_with(cx, |view, cx| {
                view.shell
                    .pack_import
                    .name
                    .as_ref()
                    .map(|field| (field.entity_id(), field.read(cx).value()))
            }),
            Some((field.entity_id(), String::from("My pack")))
        );

        // Another archive starts a new session from its own pack name.
        view.update(cx, |view, cx| {
            view.shell
                .pack_import
                .replace_source(Path::new("other.mrpack"));
            assert!(view.shell.pack_import.name.is_none());
            view.adopt_pack_name("Other pack", cx);
        });
        let replaced = view.read_with(cx, |view, cx| {
            view.shell
                .pack_import
                .name
                .as_ref()
                .map(|field| (field.entity_id(), field.read(cx).value()))
        });
        let Some((replaced_id, replaced_value)) = replaced else {
            return;
        };
        assert_ne!(
            replaced_id,
            field.entity_id(),
            "another archive owns a new editor"
        );
        assert_eq!(replaced_value, "Other pack");
    }

    #[gpui::test]
    fn retrying_a_moved_archive_keeps_the_name_draft(cx: &mut gpui::TestAppContext) {
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        view.update(cx, |view, cx| {
            view.shell
                .pack_import
                .replace_source(Path::new("pack.mrpack"));
            view.adopt_pack_name("Packed world", cx);
            view.shell.pack_import.operation_id = Some(uuid::Uuid::new_v4().to_string());
        });
        let field = view.read_with(cx, |view, _| view.shell.pack_import.name.clone());
        assert!(field.is_some(), "an inspected archive owns a name editor");
        let Some(field) = field else {
            return;
        };
        field.update(cx, |field, cx| field.set_value("My pack", cx));
        view.update(cx, |view, cx| {
            view.shell
                .pack_import
                .replace_source(Path::new("moved/pack.mrpack"));
            view.adopt_pack_name("Packed world", cx);
        });
        assert_eq!(
            view.read_with(cx, |view, cx| {
                view.shell
                    .pack_import
                    .name
                    .as_ref()
                    .map(|field| (field.entity_id(), field.read(cx).value()))
            }),
            Some((field.entity_id(), String::from("My pack"))),
            "a retry keeps the user's draft even when the archive moved"
        );
    }
}
