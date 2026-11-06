//! Local modpack inspection, selection and installation in the game wizard.
//!
//! The launcher owns every transition: this presentation derives what the
//! current [`State`] permits, shows what the pack declares, and dispatches
//! [`Action`]s for the host to apply. Rendering never changes the state it
//! reads.
#![expect(clippy::redundant_pub_crate, reason = "Private import presentation.")]

use std::{collections::BTreeSet, path::PathBuf};

use block_launcher::{
    CancellationToken,
    catalog::{LoaderKind, validate_name},
    modpack::{CurseFile, PackFile, PackFormat, PackInspection, PackSelection, TargetEvidence},
};
use gpui::{
    AnyElement, App, AppContext as _, ClipboardItem, Context, Entity, InteractiveElement as _,
    IntoElement as _, ParentElement as _, PathPromptOptions, ScrollHandle,
    StatefulInteractiveElement as _, Styled as _, Window, div, rgb,
};

use super::{
    Block, TextInput, UiAction,
    components::{self, BadgeVariant, ButtonVariant, ControlState, FieldOptions, ProgressState},
    game_wizard::{CreationKind, WizardStep},
    note, patterns,
    select::ProductChoice,
    shell::Surface,
};
use crate::{
    Language, browser::CollectionLoad, engine::PackInstallRequest, game_wizard::LoaderCatalog,
};

/// One interaction in the modpack wizard.
#[derive(Clone, Debug)]
pub(super) enum Action {
    /// Start a new import session from the directory-import shortcut.
    Open,
    Choose,
    /// One optional pack file, named by its archive-relative path.
    ToggleFile(String),
    /// One optional CurseForge reference, named by project and file ID.
    ToggleCurse(u64, u64),
    ToggleRequired,
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

/// What the wizard is showing. The host sets it; rendering only reports it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Phase {
    #[default]
    Choose,
    Inspecting,
    Review,
    Installing,
    Blocked,
    Stopped,
    Complete,
}

impl Phase {
    /// The review is on screen, so its optional entries can still be chosen or
    /// dropped. A blocked import keeps them editable: the choice itself may be
    /// what blocks it.
    const fn reviewing(self) -> bool {
        matches!(self, Self::Review | Self::Blocked | Self::Stopped)
    }
}

/// A plain archive's target while the user is choosing it.
///
/// A plain archive can report evidence without declaring a manifest, so the
/// wizard asks for whichever decision that evidence did not settle. `None` in
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
    /// result for any other request is not this session's to apply.
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

/// The page and first-run draft to restore when leaving an import shortcut.
#[derive(Debug)]
struct ReturnContext {
    first_run: Option<super::first_run::Stage>,
    directory_import: bool,
    draft: Option<FirstRunDraft>,
}

#[derive(Debug)]
struct FirstRunDraft {
    form: super::Form,
    wizard: super::game_wizard::WizardState,
}

/// Host-owned state for one import session.
#[derive(Debug)]
pub(super) struct State {
    /// Most recently selected source, including an unreadable archive.
    pub source: Option<PathBuf>,
    /// Source that owns the currently retained name and choices.
    accepted_source: Option<PathBuf>,
    pub inspection: Option<PackInspection>,
    /// Present while a plain archive's target still needs explicit choices.
    pub draft: Option<TargetDraft>,
    pub selected_files: BTreeSet<String>,
    pub selected_curse_files: BTreeSet<(u64, u64)>,
    pub phase: Phase,
    pub error: Option<crate::i18n::UiText>,
    pub cancel: CancellationToken,
    pub request: Option<uuid::Uuid>,
    completed_request: Option<uuid::Uuid>,
    /// Stable ID for this import, including before the catalog commit.
    pub operation_id: Option<String>,
    /// Present only after the Instance has been committed or explicitly resumed.
    pub instance: Option<String>,
    /// Profile created for this import, selected only when the user finishes.
    pub profile: Option<String>,
    /// The one Instance-name editor of this archive. It is created once per
    /// session, so the typed name, its selection and an ongoing composition
    /// survive every other choice in the wizard.
    pub name: Option<Entity<TextInput>>,
    initial_name: Option<String>,
    /// Why the editor's current value cannot name an Instance; `None` while it
    /// can. The same answer gates Confirm and Retry.
    pub name_error: Option<crate::i18n::UiText>,
    /// The catalog's name for an Instance this session already created or is
    /// resuming. Its name is fixed, so the editor is read-only.
    pub instance_name: Option<String>,
    /// Tracker published by this pack request, if it is still the active one.
    pub tracker: Option<std::sync::Arc<std::sync::Mutex<crate::tasks::DownloadTracker>>>,
    pub scroll: ScrollHandle,
    required: super::motion::Disclosure,
    return_to: Option<ReturnContext>,
    /// A copy drawer opened over the creation source page keeps that session.
    source_drawer_kind: Option<CreationKind>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            source: None,
            accepted_source: None,
            inspection: None,
            draft: None,
            selected_files: BTreeSet::new(),
            selected_curse_files: BTreeSet::new(),
            phase: Phase::Choose,
            error: None,
            cancel: CancellationToken::new(),
            request: None,
            completed_request: None,
            operation_id: None,
            instance: None,
            profile: None,
            name: None,
            initial_name: None,
            name_error: None,
            instance_name: None,
            tracker: None,
            scroll: ScrollHandle::new(),
            required: super::motion::Disclosure::default(),
            return_to: None,
            source_drawer_kind: None,
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
    /// Back has already returned to the source page, so Cancel no longer needs
    /// to restore the drawer that opened the pack.
    pub(super) fn clear_source_drawer_return(&mut self) {
        self.source_drawer_kind = None;
    }

    /// A visual snapshot paints independent controls and never owns the live
    /// first-run return context or an operation's tracker.
    pub(super) fn snapshot(&self, cx: &mut Context<'_, Block>) -> Self {
        let scroll = ScrollHandle::new();
        scroll.set_offset(self.scroll.offset());
        let cancel = CancellationToken::new();
        if self.cancel.is_cancelled() {
            cancel.cancel();
        }
        Self {
            source: self.source.clone(),
            accepted_source: self.accepted_source.clone(),
            inspection: self.inspection.clone(),
            draft: self.draft.clone(),
            selected_files: self.selected_files.clone(),
            selected_curse_files: self.selected_curse_files.clone(),
            phase: self.phase,
            error: self.error.clone(),
            cancel,
            request: self.request,
            completed_request: self.completed_request,
            operation_id: self.operation_id.clone(),
            instance: self.instance.clone(),
            profile: self.profile.clone(),
            name: self
                .name
                .as_ref()
                .map(|field| cx.new(|cx| field.read(cx).frozen(cx))),
            initial_name: self.initial_name.clone(),
            name_error: self.name_error.clone(),
            instance_name: self.instance_name.clone(),
            tracker: None,
            scroll,
            required: self.required.clone(),
            return_to: None,
            source_drawer_kind: None,
        }
    }

    /// A step is in flight and still owns results for its request.
    const fn running(&self) -> bool {
        matches!(self.phase, Phase::Inspecting | Phase::Installing)
    }

    /// Whether a reported request is still the session's current one. A result
    /// for a closed, stopped or replaced step is discarded here, so it cannot
    /// overwrite the current wizard state.
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

    /// Inspect a candidate without discarding the last accepted archive's name.
    /// A failed inspection must not destroy the user's draft.
    fn begin_inspection(&mut self) {
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

    /// The wizard heading copy for the current phase.
    pub(super) fn chrome(&self, language: Language) -> (String, String) {
        match self.phase {
            Phase::Choose => (
                tr!(language, "modpack.header.choose.title"),
                tr!(language, "modpack.header.choose.description"),
            ),
            Phase::Inspecting => (
                tr!(language, "modpack.header.inspecting.title"),
                tr!(language, "modpack.header.inspecting.description"),
            ),
            Phase::Review => (
                tr!(language, "modpack.header.review.title"),
                tr!(language, "modpack.header.review.description"),
            ),
            Phase::Installing if self.cancel.is_cancelled() => (
                tr!(language, "modpack.header.stopping.title"),
                tr!(language, "modpack.header.stopping.description"),
            ),
            Phase::Installing => (
                tr!(language, "modpack.header.installing.title"),
                tr!(language, "modpack.header.installing.description"),
            ),
            Phase::Blocked => (
                tr!(language, "modpack.header.blocked.title"),
                tr!(language, "modpack.header.blocked.description"),
            ),
            Phase::Stopped => (
                tr!(language, "modpack.header.stopped.title"),
                tr!(language, "modpack.header.stopped.description"),
            ),
            Phase::Complete => (
                tr!(language, "modpack.header.complete.title"),
                tr!(language, "modpack.header.complete.description"),
            ),
        }
    }
}

impl Block {
    /// Synthetic native-capture states use the real archive parser and result
    /// handlers. No picker, worker, download, or installation is started.
    #[cfg(debug_assertions)]
    pub(super) fn modpack_scenario(
        &mut self,
        name: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> anyhow::Result<()> {
        self.open_pack_import(window, cx);
        if name == "modpack-choose" {
            return Ok(());
        }
        let source = PathBuf::from("/scenario/Compact workshop.mrpack");
        let request = uuid::Uuid::from_u128(0x360720);
        {
            let state = &mut self.shell.pack_import;
            state.source = Some(source.clone());
            state.begin_inspection();
            state.request = Some(request);
        }
        if name == "modpack-inspecting" {
            return Ok(());
        }
        // Reuse the synthetic archive used by this owner's native tests.
        let directory = tempfile::tempdir()?;
        let archive = directory.path().join("fixture.mrpack");
        write_fabric_pack(&archive)?;
        let mut inspection = block_launcher::modpack::inspect(&archive)?;
        inspection.name = "Compact workshop".into();
        inspection.version = Some("1.2.3".into());
        inspection.files.extend(
            [
                ("mods/required-world-tools.jar", true),
                ("mods/optional-map-overlay.jar", false),
                ("resourcepacks/optional-landscape-textures.zip", false),
            ]
            .into_iter()
            .map(|(path, required)| PackFile {
                path: path.into(),
                sha1: "0".repeat(40),
                sha512: "0".repeat(128),
                size: 0,
                urls: Vec::new(),
                required,
            }),
        );
        self.receive_pack_inspected(
            request,
            source,
            Ok((inspection, PackSelection::default())),
            cx,
        );
        if name == "modpack-review" {
            return Ok(());
        }
        self.shell.pack_import.phase = Phase::Installing;
        self.shell.pack_import.request = Some(request);
        if name == "modpack-blocked" {
            self.finish_pack_import(
                request,
                Err(
                    "Synthetic archive verification failed before any files were installed.".into(),
                ),
                cx,
            );
            return Ok(());
        }
        // The fixture library already owns these records. Simulate the engine's
        // commit message against those exact IDs so completion has a real result
        // to open and a stopped request keeps a pending catalog target.
        let instance = self
            .snapshot
            .instances
            .iter_mut()
            .find(|instance| instance.id == "vanilla")
            .ok_or_else(|| anyhow::anyhow!("modpack scenario needs its fixture Instance"))?;
        instance.name = "Compact workshop".into();
        instance.minecraft_version = self
            .shell
            .pack_import
            .inspection
            .as_ref()
            .and_then(|inspection| inspection.minecraft_version.clone())
            .ok_or_else(|| anyhow::anyhow!("synthetic pack must declare Minecraft"))?;
        let target = instance
            .targets
            .first_mut()
            .ok_or_else(|| anyhow::anyhow!("modpack scenario needs its fixture target"))?;
        target.kind = LoaderKind::Fabric;
        target.loader_version = self
            .shell
            .pack_import
            .inspection
            .as_ref()
            .and_then(|inspection| inspection.loader.as_ref().map(|(_, build)| build.clone()));
        target.status = if name == "modpack-complete" {
            block_launcher::catalog::InstallStatus::Ready
        } else {
            block_launcher::catalog::InstallStatus::Pending
        };
        instance.pack = Some(block_launcher::catalog::PackAttachment {
            target_id: target.id.clone(),
            status: if name == "modpack-complete" {
                block_launcher::catalog::PackStatus::Ready
            } else {
                block_launcher::catalog::PackStatus::Pending
            },
        });
        let profile = self
            .snapshot
            .profiles
            .iter_mut()
            .find(|profile| profile.id == "vanilla")
            .ok_or_else(|| anyhow::anyhow!("modpack scenario needs its fixture Profile"))?;
        profile.name = "Compact workshop".into();
        self.receive_pack_created(
            request,
            "vanilla".into(),
            "Compact workshop".into(),
            "vanilla".into(),
            cx,
        );
        match name {
            "modpack-stopped" => {
                self.stop_pack_import();
                self.finish_pack_import(request, Err("Synthetic cancellation".into()), cx);
            }
            "modpack-complete" => self.finish_pack_import(request, Ok(()), cx),
            "modpack-installing" => {}
            _ => anyhow::bail!("unknown modpack scenario"),
        }
        Ok(())
    }

    /// Open the same wizard branch from the existing directory-import shortcut.
    pub(super) fn open_pack_import(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.router.game_wizard() {
            if self.shell.wizard.step != WizardStep::GetStarted
                || !self.shell.surfaces.top().is_some_and(|entry| {
                    matches!(
                        entry.kind,
                        Surface::Drawer(super::shell::Drawer::ImportGames)
                    )
                })
            {
                return;
            }
            self.replace_open_drawer(window, cx);
            self.import_games_closed(window, cx);
            self.shell.pack_import.source_drawer_kind = Some(self.shell.wizard.kind);
            self.wizard_kind(CreationKind::ImportPack, cx);
            self.shell.wizard.next();
            cx.notify();
            return;
        }
        let mut return_to = self.pack_return_context();
        self.hold_first_run_draft(&mut return_to);
        let directory_import = self.replace_open_drawer(window, cx);
        return_to.directory_import = directory_import;
        self.start_pack_wizard(cx);
        self.shell.pack_import.return_to = Some(return_to);
        if directory_import {
            self.import_games_closed(window, cx);
        }
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
        if self.router.game_wizard() {
            return;
        }
        let mut return_to = self.pack_return_context();
        self.hold_first_run_draft(&mut return_to);
        let directory_import = self.replace_open_drawer(window, cx);
        return_to.directory_import = directory_import;
        self.start_pack_wizard(cx);
        if directory_import {
            self.import_games_closed(window, cx);
        }
        let profile = self
            .pack_profile(&instance)
            .map(|profile| profile.id.clone());
        self.shell.pack_import = State {
            operation_id: Some(instance.clone()),
            profile,
            instance: Some(instance),
            return_to: Some(return_to),
            ..State::default()
        };
        cx.notify();
    }

    fn start_pack_wizard(&mut self, cx: &mut Context<'_, Self>) {
        let first_run = self.setup_incomplete() && self.shell.first_run.enabled;
        self.begin_game_wizard(cx);
        if first_run {
            self.shell.first_run.stage = super::first_run::Stage::GameWizard;
        }
        self.shell.wizard.kind = CreationKind::ImportPack;
        self.shell.wizard.step = WizardStep::Setup;
    }

    fn pack_return_context(&self) -> ReturnContext {
        ReturnContext {
            first_run: (self.router.tab() == super::Tab::Onboarding
                && self.shell.first_run.enabled)
                .then_some(self.shell.first_run.stage),
            directory_import: false,
            draft: None,
        }
    }

    fn hold_first_run_draft(&mut self, return_to: &mut ReturnContext) {
        if return_to.first_run.is_none() || !matches!(self.form, Some(super::Form::Instance(_))) {
            return;
        }
        let Some(form) = self.form.take() else {
            return;
        };
        let mut wizard = std::mem::take(&mut self.shell.wizard);
        wizard.cancel_loader_request();
        wizard.suspend_optifine_request();
        return_to.draft = Some(FirstRunDraft { form, wizard });
    }

    fn restore_first_run_draft(&mut self, draft: FirstRunDraft) {
        self.form = Some(draft.form);
        self.shell.wizard = draft.wizard;
    }

    pub(super) fn held_pack_parent_draft_id(&self, cx: &App) -> Option<gpui::EntityId> {
        self.shell
            .pack_import
            .return_to
            .as_ref()
            .and_then(|context| context.draft.as_ref())
            .and_then(|draft| match &draft.form {
                super::Form::Instance(form) if draft.wizard.is_dirty(form, cx) => {
                    Some(form.name.entity_id())
                }
                super::Form::Instance(_) | super::Form::Profile(_) => None,
            })
    }

    pub(super) fn changed_pack_draft_id(&self, cx: &App) -> Option<gpui::EntityId> {
        let state = &self.shell.pack_import;
        if state.instance.is_none()
            && state.phase != Phase::Complete
            && let Some(name) = &state.name
        {
            let name_changed = state
                .initial_name
                .as_ref()
                .is_some_and(|initial| name.read(cx).value() != *initial);
            let target_changed = state.draft.as_ref().is_some_and(|draft| {
                draft.version != draft.detected_version
                    || draft.loader.is_some()
                    || draft.build.is_some()
            });
            if name_changed
                || target_changed
                || !state.selected_files.is_empty()
                || !state.selected_curse_files.is_empty()
            {
                return Some(name.entity_id());
            }
        }
        None
    }

    /// Cancel a shortcut-entered session back to its originating page.
    pub(super) fn return_from_pack_shortcut(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        if let Some(kind) = self.shell.pack_import.source_drawer_kind.take() {
            self.pack_import_closed();
            let return_to = self.shell.pack_import.return_to.take();
            self.shell.pack_import = State {
                return_to,
                ..State::default()
            };
            self.shell.wizard.kind = kind;
            self.shell.wizard.step = WizardStep::GetStarted;
            self.open_import_games(window, cx);
            return true;
        }
        let Some(mut return_to) = self.shell.pack_import.return_to.take() else {
            return false;
        };
        self.pack_import_closed();
        self.form = None;
        if let Some(draft) = return_to.draft.take() {
            self.restore_first_run_draft(draft);
        }
        if let Some(stage) = return_to.first_run {
            self.shell.first_run.stage = stage;
            if stage == super::first_run::Stage::GameChoice {
                self.router.enter_game_choice();
            } else {
                self.router.leave_task();
            }
        } else {
            self.router.leave_task();
        }
        if return_to.directory_import {
            self.open_import_games(window, cx);
        }
        true
    }

    /// Leave the directory-import sheet before entering the page task.
    fn replace_open_drawer(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> bool {
        let mut directory_import = false;
        while self
            .shell
            .surfaces
            .top()
            .is_some_and(|entry| matches!(entry.kind, Surface::Drawer(_)))
        {
            directory_import |= self.shell.surfaces.top().is_some_and(|entry| {
                matches!(
                    entry.kind,
                    Surface::Drawer(super::shell::Drawer::ImportGames)
                )
            });
            self.shell.surfaces.close(window, &self.focus, cx);
        }
        directory_import
    }

    pub(super) fn pack_wizard_active(&self) -> bool {
        self.router.game_wizard()
            && self.shell.wizard.kind == CreationKind::ImportPack
            && self.shell.wizard.step == WizardStep::Setup
    }

    pub(super) fn pack_shortcut_preserves_current_draft(&self) -> bool {
        self.pack_wizard_active() && self.shell.pack_import.source_drawer_kind.is_some()
    }

    /// Back preserves this wizard session's draft, but not requests for a page
    /// that is no longer visible. An installation must settle after Stop before
    /// its committed choices can safely be reviewed or changed.
    pub(super) fn suspend_pack_import(&mut self) -> bool {
        if matches!(
            self.shell.pack_import.phase,
            Phase::Installing | Phase::Complete
        ) {
            return false;
        }
        let state = &mut self.shell.pack_import;
        state.request = None;
        if state.phase == Phase::Inspecting {
            state.phase = Phase::Choose;
        }
        if let Some(draft) = state.draft.as_mut()
            && draft.request.take().is_some()
        {
            draft.cancel.cancel();
            if let Some(catalog) = draft.catalog.as_mut() {
                catalog.load = CollectionLoad::Failed(msg!("wizard.loader.cancelled"));
            }
        }
        true
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
        self.shell.pack_import.initial_name = Some(value);
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

    /// Route one interaction in the wizard. Every transition is applied
    /// here; rendering only reads the resulting state.
    pub(super) fn handle_pack_import(
        &mut self,
        action: Action,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if !matches!(action, Action::Open) && !self.pack_wizard_active() {
            return;
        }
        match action {
            Action::Open => {
                if self.pack_wizard_active() {
                    return;
                }
                self.open_pack_import(window, cx);
                if self.pack_wizard_active() {
                    self.choose_pack_source(cx);
                }
                return;
            }
            Action::Choose => self.choose_pack_source(cx),
            Action::ToggleFile(path) => self.shell.pack_import.toggle_file(&path),
            Action::ToggleCurse(project, file) => {
                self.shell.pack_import.toggle_curse(project, file);
            }
            Action::ToggleRequired => {
                let preferences = self.motion_preferences();
                let state = &mut self.shell.pack_import.required;
                state.set_open(
                    !state.is_open(),
                    cx.background_executor().now(),
                    preferences,
                );
            }
            Action::Confirm => {
                if self.shell.pack_import.can_confirm() {
                    self.begin_pack_install(cx);
                }
            }
            Action::Stop => self.stop_pack_import(),
            Action::Retry => {
                if self.shell.pack_import.inspection.is_none() {
                    if let Some(source) = self.shell.pack_import.source.clone()
                        && self.shell.pack_import.request.is_none()
                    {
                        let request = uuid::Uuid::new_v4();
                        self.shell.pack_import.request = Some(request);
                        self.finish_pack_browse(request, Ok(Some(source)), cx);
                    }
                } else {
                    self.begin_pack_install(cx);
                }
            }
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
                self.finish_completed_pack(window, cx);
            }
        }
        cx.notify();
    }

    /// Select the committed result only for an explicit exit from completion.
    pub(super) fn select_completed_pack(&mut self) -> bool {
        if !self.pack_wizard_active() || self.shell.pack_import.phase != Phase::Complete {
            return false;
        }
        let (Some(instance), Some(profile)) = (
            self.shell.pack_import.instance.clone(),
            self.shell.pack_import.profile.clone(),
        ) else {
            return false;
        };
        self.selected_instance = Some(instance.clone());
        self.selected_profile = Some(profile.clone());
        self.shell.games.inspected = Some(
            if self.snapshot.profile_mode == block_launcher::catalog::ProfileMode::Bundled {
                profile
            } else {
                instance
            },
        );
        self.apply_profile_account();
        self.pack_import_closed();
        true
    }

    pub(super) fn finish_completed_pack(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        if !self.select_completed_pack() {
            return false;
        }
        if self.router.tab() == super::Tab::Onboarding && self.shell.first_run.enabled {
            let draft = self
                .shell
                .pack_import
                .return_to
                .as_mut()
                .and_then(|return_to| return_to.draft.take());
            if let Some(draft) = draft {
                self.restore_first_run_draft(draft);
            } else {
                self.form = None;
                self.shell.wizard = super::game_wizard::WizardState::default();
            }
            self.first_run_overview(window, cx);
        } else {
            self.initialize_browsers(cx);
            self.router.leave_task();
            self.form = None;
            if let Some(profile) = self.shell.pack_import.profile.clone() {
                let tab = if self.snapshot.profile_mode
                    == block_launcher::catalog::ProfileMode::Bundled
                {
                    super::Tab::Instances
                } else {
                    super::Tab::Profiles
                };
                self.router
                    .open_record(tab, super::navigation::RecordRef::Profile(profile));
            }
        }
        true
    }

    /// Choose the archive for this session. The request allocated here also owns
    /// the inspection, so leaving the wizard while the picker is open cannot
    /// start reading a file afterwards.
    fn choose_pack_source(&mut self, cx: &mut Context<'_, Self>) {
        let state = &self.shell.pack_import;
        if state.request.is_some()
            || !matches!(
                state.phase,
                Phase::Choose | Phase::Review | Phase::Blocked | Phase::Stopped
            )
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
        let state = &mut self.shell.pack_import;
        state.source = Some(source.clone());
        state.begin_inspection();
        let Some(engine) = self.engine.clone() else {
            self.shell.pack_import.phase = Phase::Blocked;
            self.shell.pack_import.error = Some(msg!("modpack.error.engine"));
            cx.notify();
            return;
        };
        let state = &mut self.shell.pack_import;
        state.request = Some(request);
        self.motion_dirty = true;
        engine.inspect_pack(request, source, state.instance.clone());
        cx.notify();
    }

    /// Install the reviewed archive into this session's instance. The instance
    /// ID is generated once and reused by every retry, so a resumed pack keeps
    /// the record it already published, and the name comes from the session's one
    /// editor.
    fn begin_pack_install(&mut self, cx: &mut Context<'_, Self>) {
        if self.shell.pack_import.request.is_some() || !self.shell.pack_import.can_start() {
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
        state.completed_request = None;
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
            Phase::Choose | Phase::Review | Phase::Blocked | Phase::Stopped | Phase::Complete => {}
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

    /// Apply a loader build result to this session. Returns whether the request
    /// belonged to the session: the host offers it elsewhere only when it did
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
        state.reset_target();
        state.source = Some(source.clone());
        let mut archive_name = None;
        match result {
            Ok((inspection, selection)) => {
                if state.operation_id.is_none() && state.accepted_source.as_deref() != Some(&source)
                {
                    state.name = None;
                    state.initial_name = None;
                    state.name_error = None;
                    state.selected_files.clear();
                    state.selected_curse_files.clear();
                    state.required = super::motion::Disclosure::default();
                }
                state.accepted_source = Some(source);
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
        self.shell.pack_import.profile = Some(profile);
        self.shell.pack_import.reset_target();
        // The Instance now carries the name this session published, so the field
        // shows exactly that name and a retry cannot rename it.
        let field = self.shell.pack_import.name.clone();
        if let Some(field) = field {
            field.update(cx, |input, cx| {
                input.set_value(&name, cx);
                input.set_read_only(true, cx);
            });
        }
        self.shell.pack_import.instance_name = Some(name);
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
        state.completed_request = None;
        match result {
            Ok(()) => {
                state.error = None;
                state.phase = Phase::Complete;
                state.completed_request = Some(request);
                tracing::info!(
                    event = "ui.pack_install.finished",
                    view_id = self.view_id,
                    request = %request,
                    outcome = "succeeded",
                );
            }
            Err(_) if state.cancel.is_cancelled() => {
                state.error = None;
                state.phase = Phase::Stopped;
                tracing::info!(
                    event = "ui.pack_install.finished",
                    view_id = self.view_id,
                    request = %request,
                    outcome = "stopped",
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
        self.dismiss_finished_pack_confirmation(request, cx);
        self.motion_dirty = true;
        cx.notify();
    }

    fn dismiss_finished_pack_confirmation(&self, request: uuid::Uuid, cx: &mut Context<'_, Self>) {
        let confirmation = self
            .shell
            .surfaces
            .top()
            .and_then(|entry| match &entry.kind {
                Surface::StopPackImport {
                    request: pending,
                    window,
                    ..
                } if *pending == request => Some(*window),
                _ => None,
            });
        if let Some(window) = confirmation {
            let owner = cx.entity();
            cx.defer(move |cx| {
                let _ = window.update(cx, |_, window, cx| {
                    owner.update(cx, |view, cx| {
                        if view.shell.surfaces.top().is_some_and(|entry| {
                            matches!(
                                entry.kind,
                                Surface::StopPackImport { request: pending, .. }
                                    if pending == request
                            )
                        }) {
                            view.shell.surfaces.close(window, &view.focus, cx);
                            if view.shell.pack_import.phase == Phase::Complete
                                && view.shell.pack_import.completed_request == Some(request)
                            {
                                view.focus_completed_pack(request, window, cx);
                            }
                            cx.notify();
                        }
                    });
                });
            });
        }
    }

    fn focus_completed_pack(
        &self,
        request: uuid::Uuid,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let focus = self.shell.surfaces.trigger_focus("modpack-done", cx);
        let owner = cx.weak_entity();
        window.on_next_frame(move |window, _| {
            window.on_next_frame(move |window, cx| {
                let _ = owner.update(cx, |view, cx| {
                    if view.pack_wizard_active()
                        && view.shell.pack_import.phase == Phase::Complete
                        && view.shell.pack_import.completed_request == Some(request)
                        && !view.shell.surfaces.is_open()
                    {
                        focus.focus(window, cx);
                    }
                });
            });
        });
    }

    /// End this wizard session. Catalog updates remain independent of its
    /// request, so an already committed instance can still be resumed.
    pub(super) fn pack_import_closed(&mut self) {
        self.release_pack_tracker();
        let state = &mut self.shell.pack_import;
        state.request = None;
        state.cancel.cancel();
        state.reset_target();
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
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let state = &self.shell.pack_import;
        let language = self.language();
        let (title, description) = state.chrome(language);
        let mut content = div()
            .id("modpack-import")
            .debug_selector(|| "modpack-import".into())
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                patterns::page_heading(title, description)
                    .id("modpack-heading")
                    .debug_selector(|| "modpack-heading".into()),
            );
        content = content.child(match state.phase {
            Phase::Choose => self.modpack_choose_body(state, language),
            Phase::Inspecting => self.modpack_working_body(
                tr!(language, "modpack.inspecting.progress"),
                tr!(language, "modpack.inspecting.note"),
            ),
            Phase::Review => self.modpack_review_body(state, language, window, cx),
            Phase::Installing => self.modpack_working_body(
                tr!(
                    language,
                    if state.cancel.is_cancelled() {
                        "modpack.stopping.progress"
                    } else {
                        "modpack.installing.progress"
                    }
                ),
                tr!(
                    language,
                    if state.cancel.is_cancelled() {
                        "modpack.stopping.note"
                    } else if state.instance.is_some() {
                        "modpack.installing.resume_note"
                    } else {
                        "modpack.installing.note"
                    }
                ),
            ),
            Phase::Blocked | Phase::Stopped => {
                self.modpack_blocked_body(state, language, window, cx)
            }
            Phase::Complete => self.modpack_complete_body(state, language),
        });
        if matches!(state.phase, Phase::Blocked | Phase::Stopped) && state.inspection.is_none() {
            content = content
                .children(
                    (!state.installable() && state.target_ready())
                        .then(|| note(tr!(language, "modpack.action.retry_note"))),
                )
                .child(self.modpack_action_button(
                    "modpack-choose-other",
                    self.pack_choose_again_label(state, language),
                    Action::Choose,
                    ButtonVariant::Ghost,
                    state.request.is_some(),
                    cx,
                ));
        }
        if state.phase == Phase::Installing {
            content = content.child(note(tr!(language, "modpack.installing.stop_note")));
        }
        content.into_any_element()
    }

    fn modpack_choose_body(&self, state: &State, language: Language) -> gpui::Div {
        // A session that already published its instance resumes from that
        // record: the engine restores the selections it saved, so the wizard
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

    /// A retained failure resolves again in the language the wizard is painted
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
        window: &Window,
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
            .child(self.modpack_pack_details(state, language, window, cx))
    }

    /// A blocked import keeps its review on screen, so a choice that blocks it
    /// can still be dropped.
    fn modpack_blocked_body(
        &self,
        state: &State,
        language: Language,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> gpui::Div {
        if state.phase == Phase::Stopped {
            return div()
                .flex()
                .flex_col()
                .gap_2()
                .child(note(tr!(language, "modpack.stopped.note")))
                .child(self.modpack_pack_details(state, language, window, cx));
        }
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
                            .min_w_0()
                            .flex_1()
                            .whitespace_normal()
                            .child(tr!(language, "modpack.blocked.heading")),
                    ),
            )
            .children(unexplained.then(|| note(tr!(language, "modpack.blocked.unsupported"))))
            .child(self.modpack_pack_details(state, language, window, cx))
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
            // The instance carries the name this session published, which the
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
                tr!(
                    language,
                    if self.shell.wizard.separated {
                        "games.instance_editor.name"
                    } else {
                        "wizard.setup.gameName"
                    }
                ),
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
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> gpui::Div {
        let Some(inspection) = state.inspection.as_ref() else {
            return div()
                .flex()
                .flex_col()
                .gap_2()
                .children(self.modpack_instance_name(state, language))
                .children(state.source.as_ref().map(|source| {
                    note(tr!(language, "modpack.archive", path = source.display()))
                        .id("modpack-archive")
                        .debug_selector(|| "modpack-archive".into())
                }))
                .children(self.modpack_error(state, language))
                .child(note(tr!(language, "modpack.review.no_inspection")));
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
            .children(inspection.version.as_ref().map(|version| {
                note(tr!(language, "modpack.review.version", version = version))
                    .id("modpack-pack-version")
                    .debug_selector(|| "modpack-pack-version".into())
            }))
            .child(note(tr!(
                language,
                "modpack.review.format",
                pack_format = format_label(inspection.format, language)
            )))
            .children(state.source.as_ref().map(|source| {
                note(tr!(language, "modpack.archive", path = source.display()))
                    .id("modpack-archive")
                    .debug_selector(|| "modpack-archive".into())
            }))
            .child(self.modpack_action_button(
                "modpack-choose-other",
                self.pack_choose_again_label(state, language),
                Action::Choose,
                ButtonVariant::Ghost,
                state.request.is_some(),
                cx,
            ))
            .children(
                state
                    .instance
                    .as_ref()
                    .map(|_| note(tr!(language, "modpack.selection.locked"))),
            )
            .child(
                div()
                    .id("modpack-target")
                    .debug_selector(|| "modpack-target".into())
                    .child(self.modpack_target(state, inspection, language, cx)),
            )
            .children(self.modpack_error(state, language))
            .children(
                (!state.unresolved().is_empty())
                    .then(|| note(tr!(language, "modpack.review.blocked_summary"))),
            );
        let required = state.required_files();
        let optional = state.optional_files();
        if required.is_empty() && optional.is_empty() && inspection.missing_files.is_empty() {
            details = details.child(note(tr!(language, "modpack.review.files_none")));
        }
        if !inspection.missing_files.is_empty() {
            details = details.child(self.modpack_unresolved_section(
                state,
                &inspection.missing_files,
                language,
                cx,
            ));
        }
        if !optional.is_empty() {
            let heading = tr!(language, "modpack.review.optional", count = optional.len());
            details = details.child(self.modpack_section(heading));
            for file in optional {
                details = details.child(self.modpack_optional_row(state, file, cx));
            }
        }
        if !required.is_empty() {
            let heading = tr!(language, "modpack.review.required", count = required.len());
            let trigger = super::motion::disclosure_trigger(
                "modpack-required-toggle",
                heading,
                &state.required,
                self.palette,
                self.motion_preferences(),
                cx,
                cx.listener(|view, (): &(), window, cx| {
                    view.handle(UiAction::PackImport(Action::ToggleRequired), window, cx);
                }),
            )
            .aria_expanded(state.required.is_open());
            let rows = div().flex().flex_col().children(
                required
                    .into_iter()
                    .map(|file| self.modpack_required_row(file, language)),
            );
            details = details.child(trigger).child(super::motion::disclosure(
                "modpack-required-list",
                &state.required,
                self.motion_preferences(),
                rows.into_any_element(),
                window,
                cx,
            ));
        }
        if state.instance.is_some() {
            let summary = self
                .pack_resume_launch_summary(state, language)
                .unwrap_or_else(|| tr!(language, "modpack.review.resume_settings_unavailable"));
            return details
                .child(
                    note(summary)
                        .id("modpack-resume-settings")
                        .debug_selector(|| "modpack-resume-settings".into()),
                )
                .child(
                    note(tr!(language, "modpack.review.resume_effect"))
                        .id("modpack-resume-effect")
                        .debug_selector(|| "modpack-resume-effect".into()),
                );
        }
        let defaults = &self.saved_preferences.game;
        let java = if defaults.java.trim().is_empty() {
            tr!(language, "wizard.profile.automaticJava")
        } else {
            defaults.java.clone()
        };
        let window = if defaults.fullscreen {
            tr!(language, "wizard.config.fullscreenValue")
        } else {
            tr!(
                language,
                "wizard.profile.windowSizeValue",
                width = defaults.width,
                height = defaults.height
            )
        };
        details
            .child(note(tr!(language, "modpack.review.profile_defaults")))
            .child(note(tr!(
                language,
                "modpack.review.effective_defaults",
                java = java,
                memory = components::memory_label(defaults.memory_mib),
                window = window
            )))
            .child(note(tr!(language, "modpack.review.install_effect")))
    }

    /// The installer uses the first Profile for the Instance, regardless of
    /// which target that Profile currently selects.
    fn pack_profile(&self, instance: &str) -> Option<&block_launcher::catalog::Profile> {
        self.snapshot
            .profiles
            .iter()
            .find(|profile| profile.instance_id == instance)
    }

    /// Report the installer's Profile settings, not defaults used for creation.
    fn pack_resume_launch_summary(&self, state: &State, language: Language) -> Option<String> {
        let instance = self
            .snapshot
            .instances
            .iter()
            .find(|instance| Some(&instance.id) == state.instance.as_ref())?;
        let profile = self.pack_profile(&instance.id)?;
        let mut effective = profile.clone();
        effective.runtime = profile.effective_runtime(instance);
        effective.game_args = profile.effective_game_args(instance);
        let settings = block_launcher::catalog::InstanceSettings::from_profile(&effective).ok()?;
        let java = match &effective.runtime {
            block_launcher::recipe::RuntimeSelection::Managed => {
                tr!(language, "wizard.profile.automaticJava")
            }
            block_launcher::recipe::RuntimeSelection::Local { java } => java.display().to_string(),
        };
        let has_width = effective
            .game_args
            .iter()
            .any(|arg| arg == "--width" || arg.starts_with("--width="));
        let has_height = effective
            .game_args
            .iter()
            .any(|arg| arg == "--height" || arg.starts_with("--height="));
        let window = if settings.fullscreen {
            tr!(language, "wizard.config.fullscreenValue")
        } else if has_width && has_height {
            tr!(
                language,
                "wizard.profile.windowSizeValue",
                width = settings.width,
                height = settings.height
            )
        } else {
            tr!(language, "modpack.review.saved_window")
        };
        Some(tr!(
            language,
            "modpack.review.effective_profile",
            java = java,
            memory = components::memory_label(profile.memory_mib),
            window = window
        ))
    }

    fn pack_choose_again_label(&self, state: &State, language: Language) -> String {
        tr!(
            language,
            if state.instance.is_some() {
                "modpack.action.choose_original"
            } else {
                "modpack.action.choose_other"
            }
        )
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
        let id = format!("modpack-required-{}", file.path);
        div()
            .id(gpui::SharedString::from(id.clone()))
            .debug_selector(move || id.clone())
            .border_b_1()
            .border_color(rgb(self.palette.border))
            .py_2()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .min_w_0()
                    .w_full()
                    .text_sm()
                    .whitespace_normal()
                    .child(file.path.clone()),
            )
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
                .flex_wrap()
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

    pub(super) fn modpack_wizard_footer(
        &self,
        language: Language,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let state = &self.shell.pack_import;
        let (id, label, action, variant, disabled) = match state.phase {
            Phase::Choose => (
                "modpack-choose",
                tr!(language, "modpack.action.choose"),
                Action::Choose,
                ButtonVariant::Primary,
                state.request.is_some(),
            ),
            Phase::Inspecting | Phase::Installing => (
                "modpack-stop",
                tr!(
                    language,
                    if state.phase == Phase::Installing && state.cancel.is_cancelled() {
                        "modpack.action.stopping"
                    } else {
                        "modpack.action.stop"
                    }
                ),
                Action::Stop,
                ButtonVariant::Outline,
                !state.stoppable(),
            ),
            Phase::Review | Phase::Blocked | Phase::Stopped if !state.unresolved().is_empty() => (
                "modpack-choose",
                self.pack_choose_again_label(state, language),
                Action::Choose,
                ButtonVariant::Primary,
                state.request.is_some(),
            ),
            Phase::Review => (
                "modpack-confirm",
                tr!(language, "modpack.action.confirm"),
                Action::Confirm,
                ButtonVariant::Primary,
                !state.can_confirm() || state.request.is_some(),
            ),
            Phase::Blocked | Phase::Stopped
                if state.inspection.is_none() && state.source.is_none() =>
            {
                (
                    "modpack-choose",
                    tr!(language, "modpack.action.choose"),
                    Action::Choose,
                    ButtonVariant::Primary,
                    state.request.is_some(),
                )
            }
            Phase::Blocked | Phase::Stopped if state.inspection.is_none() => (
                "modpack-recheck",
                tr!(language, "modpack.action.recheck"),
                Action::Retry,
                ButtonVariant::Primary,
                state.request.is_some(),
            ),
            Phase::Blocked | Phase::Stopped
                if state.draft.as_ref().is_some_and(|draft| {
                    draft.catalog.as_ref().is_some_and(|catalog| {
                        matches!(catalog.load, CollectionLoad::Failed(_))
                            || matches!(catalog.load, CollectionLoad::Ready)
                                && catalog.builds.is_empty()
                    })
                }) =>
            {
                (
                    "modpack-reload-builds-primary",
                    tr!(language, "modpack.target.retry_builds"),
                    Action::ReloadTargetBuilds,
                    ButtonVariant::Primary,
                    state.request.is_some(),
                )
            }
            Phase::Blocked | Phase::Stopped if !state.target_ready() => (
                "modpack-choose",
                self.pack_choose_again_label(state, language),
                Action::Choose,
                ButtonVariant::Primary,
                state.request.is_some(),
            ),
            Phase::Blocked | Phase::Stopped => (
                "modpack-retry",
                tr!(language, "modpack.action.retry"),
                Action::Retry,
                ButtonVariant::Primary,
                !state.can_start() || state.request.is_some(),
            ),
            Phase::Complete => (
                "modpack-done",
                tr!(language, "modpack.action.done"),
                Action::Done,
                ButtonVariant::Primary,
                false,
            ),
        };
        let cancel = (state.phase != Phase::Complete).then(|| {
            self.button(
                "cancel-instance",
                tr!(language, "common.cancel"),
                UiAction::CancelForm,
                cx,
            )
        });
        let back =
            (state.phase != Phase::Installing && state.phase != Phase::Complete).then(|| {
                self.button(
                    "wizard-back",
                    tr!(language, "wizard.footer.back"),
                    UiAction::WizardBack,
                    cx,
                )
            });
        let primary = self.modpack_action_button(id, label, action, variant, disabled, cx);
        let primary = if state.phase == Phase::Complete {
            primary.track_focus(&self.shell.surfaces.trigger_focus("modpack-done", cx))
        } else {
            primary
        };
        if state.phase == Phase::Complete {
            return primary.w_full().into_any_element();
        }
        div()
            .w_full()
            .flex()
            .flex_col()
            .gap_2()
            .children(cancel.map(|button| button.w_full()))
            .children(back.map(|button| button.w_full()))
            .child(primary.w_full())
            .into_any_element()
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
        .h_auto()
        .min_h(gpui::px(36.))
        .whitespace_normal()
    }
}

/// A synthetic manifest archive, shared by debug captures and native tests.
#[cfg(any(test, debug_assertions))]
fn write_fabric_pack(path: &std::path::Path) -> anyhow::Result<()> {
    const ZIP_HEX: &str = concat!(
        "504b03041400000008000dae615db11a6a457600000096000000130000006d6f6472696e74682e69",
        "6e6465782e6a736f6e4d8c310e83300c45af527986a8a9c4920354e2022c550737712aabc44106b1",
        "20ee5e53968effbdffff06b96ac165209db90a04dfc01b0b4180c24251312fd0809ce88e2fe57899",
        "307e0caee7a64f66bce544134922894c3384edefc0bcbb5ddd51cabf8b76ac98484d18ed9cf7b09b",
        "e2f1183e9efb17504b010214031400000008000dae615db11a6a4576000000960000001300000000",
        "000000000000008001000000006d6f6472696e74682e696e6465782e6a736f6e504b050600000000",
        "0100010041000000a70000000000",
    );
    let bytes = ZIP_HEX
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|digits| Ok(u8::from_str_radix(std::str::from_utf8(digits)?, 16)?))
        .collect::<anyhow::Result<Vec<_>>>()?;
    std::fs::write(path, bytes)?;
    Ok(())
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

    fn write_curse_pack(path: &std::path::Path) -> anyhow::Result<()> {
        const ZIP_HEX: &str = concat!(
            "504b03041400000008009989615d287ca172a5000000190100000d0000006d616e69666573742e6a",
            "736f6e658ec10ac2300c865f45722ea3db6dbd2a82a037f1223b943585ead6ceac1365f4dd6d95e9",
            "d45b92efff928cd04a6b34f67e7fef1004b4c6624d52fb9d539daccfc0de8903526f9c059133b0b2",
            "4de9e57aa1cdcd0f8431779d38e4c99a168118e7282b78f6e44e6d9d54118038568181360da67a84",
            "8edc096bbf5981284bce79fe829f3e0e082f832154203c0d18d8bf55fc58c597a565d363a818b8f8",
            "1b19954ecfeaf000504b010214031400000008009989615d287ca172a5000000190100000d000000",
            "00000000000000008001000000006d616e69666573742e6a736f6e504b050600000000010001003b",
            "000000d00000000000",
        );
        let bytes = ZIP_HEX
            .as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|digits| Ok(u8::from_str_radix(std::str::from_utf8(digits)?, 16)?))
            .collect::<anyhow::Result<Vec<_>>>()?;
        std::fs::write(path, bytes)?;
        Ok(())
    }

    fn write_optional_curse_pack(path: &std::path::Path) -> anyhow::Result<()> {
        const ZIP_HEX: &str = concat!(
            "504b030414000000080058aa615d2076217a99000000e40000000d0000006d616e69666573742e6a",
            "736f6e4d8e3d0bc2301086ff8adc1c4adaad591541d04d5ca4c3d15c20dae6d2340a52fadf4d946a",
            "b7f7ee79ee63821e9d3534c6f3cb1328e8ada336a08927d61edb3b889f71a1305a76a04a010efb6c",
            "6ff71bf63135b14be27311a0cc63cb2650d31a15952c3e9cf591512700eadacc028ced28e7097ce0",
            "1bb5f1b00355d752caea0bff75fa20d0f0b081342883dd48732380d39560755eb2caf31b504b0102",
            "140314000000080058aa615d2076217a99000000e40000000d000000000000000000000080010000",
            "00006d616e69666573742e6a736f6e504b050600000000010001003b000000c40000000000",
        );
        let bytes = ZIP_HEX
            .as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|digits| Ok(u8::from_str_radix(std::str::from_utf8(digits)?, 16)?))
            .collect::<anyhow::Result<Vec<_>>>()?;
        std::fs::write(path, bytes)?;
        Ok(())
    }

    fn click_pack(cx: &mut gpui::VisualTestContext, selector: &'static str) -> anyhow::Result<()> {
        cx.run_until_parked();
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(500));
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        let bounds = crate::test_support::control_bounds(cx, selector)
            .ok_or_else(|| anyhow::anyhow!("missing control {selector}"))?;
        cx.simulate_click(bounds.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        Ok(())
    }

    fn receive_inspection(
        view: &Entity<Block>,
        cx: &mut gpui::VisualTestContext,
        receiver: &mut tokio::sync::mpsc::Receiver<crate::Message>,
    ) -> anyhow::Result<()> {
        let runtime = tokio::runtime::Runtime::new()?;
        let message = runtime.block_on(async {
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    let message = receiver
                        .recv()
                        .await
                        .ok_or_else(|| anyhow::anyhow!("engine closed before inspection"))?;
                    if matches!(message, crate::Message::PackInspected { .. }) {
                        return Ok::<_, anyhow::Error>(message);
                    }
                }
            })
            .await?
        })?;
        let (sender, inbox) = tokio::sync::mpsc::channel(1);
        sender.try_send(message)?;
        view.update(cx, |view, cx| {
            view.messages = Some(inbox);
            view.poll(cx);
        });
        cx.run_until_parked();
        Ok(())
    }

    #[gpui::test]
    fn copy_drawer_pack_switch_returns_to_the_same_creation_session(cx: &mut gpui::TestAppContext) {
        let result = (|| -> anyhow::Result<()> {
            let (view, cx) = crate::test_support::window(cx, |window, cx| {
                let mut view = crate::tests::test_view(window, cx);
                view.preferences.reduce_motion = true;
                view
            });
            cx.simulate_resize(gpui::size(gpui::px(1152.), gpui::px(900.)));
            click_pack(cx, "create-instance")?;
            let original = view
                .read_with(cx, |view, _| match &view.form {
                    Some(super::super::Form::Instance(form)) => Some(form.name.clone()),
                    _ => None,
                })
                .ok_or_else(|| anyhow::anyhow!("missing creation draft"))?;
            click_pack(cx, "wizard-kind-instance")?;
            original.update(cx, |field, cx| field.set_value("Keep this game", cx));
            click_pack(cx, "wizard-back")?;
            click_pack(cx, "wizard-copy-games")?;
            view.update(cx, |view, cx| {
                let scroll = &view.shell.import_games.source_scroll;
                scroll.set_offset(gpui::point(gpui::px(0.), -scroll.max_offset().y));
                cx.notify();
            });
            cx.run_until_parked();
            click_pack(cx, "import-pack")?;
            assert!(view.read_with(cx, |view, _| view.pack_wizard_active()));
            assert!(
                cx.did_prompt_for_paths(),
                "the import action must open the picker"
            );
            cx.simulate_path_prompt_response(|_| None);
            cx.run_until_parked();
            click_pack(cx, "cancel-instance")?;
            assert!(view.read_with(cx, |view, _| {
                matches!(
                    view.shell.surfaces.top().map(|layer| &layer.kind),
                    Some(Surface::Drawer(super::super::shell::Drawer::ImportGames))
                ) && view.router.game_wizard()
                    && view.shell.wizard.step == WizardStep::GetStarted
                    && matches!(&view.form, Some(super::super::Form::Instance(form))
                        if form.name.entity_id() == original.entity_id())
            }));
            assert_eq!(
                original.read_with(cx, |field, _| field.value()),
                "Keep this game"
            );
            Ok(())
        })();
        assert!(result.is_ok(), "{result:?}");
    }

    #[gpui::test]
    fn back_to_sources_keeps_pack_draft_protected_on_quit(cx: &mut gpui::TestAppContext) {
        let result = (|| -> anyhow::Result<()> {
            let root = tempfile::tempdir()?;
            let source = root.path().join("pack.mrpack");
            crate::test_support::write_modpack(&source)?;
            let (engine, mut receiver) = crate::Engine::with_root(root.path().to_path_buf())?;
            let (view, cx) = crate::test_support::window(cx, |window, cx| {
                let mut view = crate::tests::test_view(window, cx);
                view.preferences.reduce_motion = true;
                view.engine = Some(engine);
                view
            });
            cx.simulate_resize(gpui::size(gpui::px(1152.), gpui::px(900.)));
            cx.update(|_, cx| crate::initialize_application_quit(cx));
            click_pack(cx, "create-instance")?;
            click_pack(cx, "wizard-kind-pack")?;
            let chosen = source;
            cx.simulate_path_prompt_response(move |_| Some(vec![chosen]));
            cx.run_until_parked();
            receive_inspection(&view, cx, &mut receiver)?;
            let name = view
                .read_with(cx, |view, _| view.shell.pack_import.name.clone())
                .ok_or_else(|| anyhow::anyhow!("missing pack name"))?;
            name.update(cx, |field, cx| field.set_value("Edited pack", cx));
            cx.run_until_parked();
            click_pack(cx, "wizard-back")?;
            assert_eq!(
                view.read_with(cx, |view, _| view.shell.wizard.step),
                WizardStep::GetStarted
            );
            crate::test_support::press(cx, "cmd-q");
            cx.run_until_parked();
            assert!(view.read_with(cx, |view, _| {
                view.shell
                    .surfaces
                    .top()
                    .is_some_and(|layer| matches!(layer.kind, Surface::DiscardGameDraft { .. }))
            }));
            click_pack(cx, "discard-game-stay")?;
            assert_eq!(name.read_with(cx, |field, _| field.value()), "Edited pack");
            Ok(())
        })();
        assert!(result.is_ok(), "{result:?}");
    }

    #[gpui::test]
    fn back_to_sources_keeps_the_first_run_parent_protected_on_quit(cx: &mut gpui::TestAppContext) {
        let result = (|| -> anyhow::Result<()> {
            let root = tempfile::tempdir()?;
            let (engine, _receiver) = crate::Engine::with_root(root.path().to_path_buf())?;
            let (view, cx) = crate::test_support::window(cx, |window, cx| {
                let mut view = crate::tests::test_view(window, cx);
                view.preferences.reduce_motion = true;
                view.preferences.onboarding_complete = false;
                view.saved_preferences.onboarding_complete = false;
                view.engine = Some(engine);
                view.begin_first_run(cx);
                view
            });
            cx.simulate_resize(gpui::size(gpui::px(1152.), gpui::px(900.)));
            cx.update(|_, cx| crate::initialize_application_quit(cx));
            click_pack(cx, "setup-game")?;
            click_pack(cx, "setup-custom")?;
            let name = view
                .read_with(cx, |view, _| match &view.form {
                    Some(super::super::Form::Instance(form)) => Some(form.name.clone()),
                    _ => None,
                })
                .ok_or_else(|| anyhow::anyhow!("missing first-run draft"))?;
            name.update(cx, |field, cx| field.set_value("Edited parent", cx));
            click_pack(cx, "cancel-instance")?;
            click_pack(cx, "setup-game")?;
            click_pack(cx, "setup-import-pack")?;
            assert!(cx.did_prompt_for_paths());
            cx.simulate_path_prompt_response(|_| None);
            cx.run_until_parked();
            assert!(view.read_with(cx, |view, cx| view.held_pack_parent_draft_id(cx).is_some()));
            click_pack(cx, "wizard-back")?;
            assert_eq!(
                view.read_with(cx, |view, _| view.shell.wizard.step),
                WizardStep::GetStarted
            );
            crate::test_support::press(cx, "cmd-q");
            cx.run_until_parked();
            assert!(view.read_with(cx, |view, _| {
                view.shell
                    .surfaces
                    .top()
                    .is_some_and(|layer| matches!(layer.kind, Surface::DiscardGameDraft { .. }))
            }));
            click_pack(cx, "discard-game-stay")?;
            assert_eq!(
                name.read_with(cx, |field, _| field.value()),
                "Edited parent"
            );
            Ok(())
        })();
        assert!(result.is_ok(), "{result:?}");
    }

    #[gpui::test]
    fn unresolved_curse_entries_offer_a_working_replacement_action(cx: &mut gpui::TestAppContext) {
        let result = (|| -> anyhow::Result<()> {
            let root = tempfile::tempdir()?;
            let required = root.path().join("required.zip");
            let optional = root.path().join("optional.zip");
            write_curse_pack(&required)?;
            write_optional_curse_pack(&optional)?;
            let (engine, mut receiver) = crate::Engine::with_root(root.path().to_path_buf())?;
            let (view, cx) = crate::test_support::window(cx, |window, cx| {
                let mut view = crate::tests::test_view(window, cx);
                view.preferences.reduce_motion = true;
                view.engine = Some(engine);
                view
            });
            cx.simulate_resize(gpui::size(gpui::px(1152.), gpui::px(900.)));
            click_pack(cx, "create-instance")?;
            click_pack(cx, "wizard-kind-pack")?;
            cx.simulate_path_prompt_response(move |_| Some(vec![required]));
            cx.run_until_parked();
            receive_inspection(&view, cx, &mut receiver)?;
            assert_eq!(
                view.read_with(cx, |view, _| view.shell.pack_import.phase),
                Phase::Review
            );
            click_pack(cx, "modpack-choose")?;
            assert!(cx.did_prompt_for_paths(), "replacement must open a picker");
            cx.simulate_path_prompt_response(move |_| Some(vec![optional]));
            cx.run_until_parked();
            receive_inspection(&view, cx, &mut receiver)?;
            assert!(crate::test_support::control_bounds(cx, "modpack-confirm").is_some());
            reveal_pack_item(&view, cx, "modpack-curse-990002-9900021")?;
            click_pack(cx, "modpack-curse-990002-9900021")?;
            assert!(view.read_with(cx, |view, _| {
                !view.shell.pack_import.unresolved().is_empty()
            }));
            click_pack(cx, "modpack-choose")?;
            assert!(
                cx.did_prompt_for_paths(),
                "selected optional entry must offer replacement"
            );
            cx.simulate_path_prompt_response(|_| None);
            cx.run_until_parked();
            click_pack(cx, "modpack-curse-990002-9900021")?;
            assert!(view.read_with(cx, |view, _| view.shell.pack_import.unresolved().is_empty()));
            assert!(view.read_with(cx, |view, _| view.shell.pack_import.can_confirm()));
            let button = cx
                .update(|window, _| {
                    gpui_base::test_support::find(
                        window,
                        &[],
                        &gpui::ElementId::Name("modpack-confirm".into()),
                    )
                })
                .ok_or_else(|| anyhow::anyhow!("install button is not accessible"))?;
            assert_ne!(
                button.disabled(),
                Some(true),
                "Install must become actionable again"
            );
            Ok(())
        })();
        assert!(result.is_ok(), "{result:?}");
    }

    #[gpui::test]
    fn add_game_picker_review_and_done_use_the_same_import_session(cx: &mut gpui::TestAppContext) {
        let result = (|| -> anyhow::Result<()> {
            let root = tempfile::tempdir()?;
            let source = root.path().join("pack.mrpack");
            crate::test_support::write_modpack(&source)?;
            let catalog = block_launcher::catalog::Catalog::open(root.path())?;
            let (old_instance, old_profile) =
                catalog.create_instance("Existing".into(), "1.20.1".into(), None, None)?;
            let snapshot = catalog.snapshot()?;
            let (engine, mut receiver) = crate::Engine::with_root(root.path().to_path_buf())?;
            let (view, cx) = crate::test_support::window(cx, |window, cx| {
                let mut view = crate::tests::test_view(window, cx);
                view.preferences.reduce_motion = true;
                view.snapshot = snapshot;
                view.selected_instance = Some(old_instance.id.clone());
                view.selected_profile = Some(old_profile.id.clone());
                view.engine = Some(engine);
                view
            });
            cx.simulate_resize(gpui::size(gpui::px(1152.), gpui::px(900.)));
            assert!(
                crate::test_support::control_bounds(cx, "import-pack").is_none(),
                "the browser has no separate pack button"
            );
            click_pack(cx, "create-instance")?;
            assert_eq!(
                view.read_with(cx, |view, _| view.shell.wizard.step),
                WizardStep::GetStarted
            );
            click_pack(cx, "wizard-kind-pack")?;
            assert!(view.read_with(cx, |view, _| {
                view.pack_wizard_active()
                    && view.page_spec().layout == crate::navigation::LayoutKind::Wizard
                    && !view.shell.surfaces.is_open()
            }));
            assert!(cx.did_prompt_for_paths());
            cx.simulate_path_prompt_response(|_| None);
            cx.run_until_parked();
            assert!(view.read_with(cx, |view, _| view.shell.pack_import.request.is_none()));
            click_pack(cx, "modpack-choose")?;
            let chosen = source.clone();
            cx.simulate_path_prompt_response(move |_| Some(vec![chosen]));
            cx.run_until_parked();

            let runtime = tokio::runtime::Runtime::new()?;
            let message = runtime.block_on(async {
                tokio::time::timeout(std::time::Duration::from_secs(5), async {
                    loop {
                        let message = receiver
                            .recv()
                            .await
                            .ok_or_else(|| anyhow::anyhow!("engine closed before inspection"))?;
                        if matches!(message, crate::Message::PackInspected { .. }) {
                            return Ok::<_, anyhow::Error>(message);
                        }
                    }
                })
                .await?
            })?;
            let (sender, inbox) = tokio::sync::mpsc::channel(1);
            sender.try_send(message)?;
            view.update(cx, |view, cx| {
                view.messages = Some(inbox);
                view.poll(cx);
            });
            cx.run_until_parked();
            let (field, original) = view.read_with(cx, |view, _| {
                (
                    view.shell.pack_import.name.clone(),
                    view.shell
                        .pack_import
                        .inspection
                        .as_ref()
                        .map(|pack| pack.name.clone()),
                )
            });
            let field = field.ok_or_else(|| anyhow::anyhow!("the name editor is missing"))?;
            let original = original.ok_or_else(|| anyhow::anyhow!("inspection is missing"))?;
            field.update(cx, |field, cx| field.set_value("My imported game", cx));
            cx.run_until_parked();
            reveal_pack_item(&view, cx, "modpack-optional-mods/optional.jar")?;
            click_pack(cx, "modpack-optional-mods/optional.jar")?;
            assert!(view.read_with(cx, |view, _| {
                view.shell
                    .pack_import
                    .selected_files
                    .contains("mods/optional.jar")
                    && view
                        .shell
                        .pack_import
                        .inspection
                        .as_ref()
                        .is_some_and(|pack| pack.name == original)
            }));

            click_pack(cx, "wizard-back")?;
            click_pack(cx, "wizard-kind-instance")?;
            click_pack(cx, "wizard-back")?;
            click_pack(cx, "wizard-kind-pack")?;
            assert!(cx.did_prompt_for_paths());
            cx.simulate_path_prompt_response(|_| None);
            cx.run_until_parked();
            assert!(view.read_with(cx, |view, cx| {
                view.shell.pack_import.name.as_ref().is_some_and(|name| {
                    name.entity_id() == field.entity_id()
                        && name.read(cx).value() == "My imported game"
                }) && view
                    .shell
                    .pack_import
                    .selected_files
                    .contains("mods/optional.jar")
            }));

            let library = block_launcher::library::Library::new(
                root.path().to_path_buf(),
                std::sync::Arc::new(|_| {}),
                std::sync::Arc::new(|_| {}),
            )?;
            let inspection = view
                .read_with(cx, |view, _| view.shell.pack_import.inspection.clone())
                .ok_or_else(|| anyhow::anyhow!("the inspection was lost"))?;
            let selection = view.read_with(cx, |view, _| view.shell.pack_import.selection());
            let configuration = block_launcher::catalog::ProfileConfiguration {
                name: String::from("My imported game"),
                ..Default::default()
            };
            let (instance, profile) = library.create_pack(
                &uuid::Uuid::new_v4().to_string(),
                &source,
                &inspection,
                &selection,
                configuration,
                &CancellationToken::new(),
            )?;
            let request = uuid::Uuid::new_v4();
            let updated = catalog.snapshot()?;
            view.update(cx, |view, cx| {
                view.snapshot = updated;
                view.shell.pack_import.phase = Phase::Installing;
                view.shell.pack_import.request = Some(request);
                view.receive_pack_created(
                    request,
                    instance.id.clone(),
                    instance.name.clone(),
                    profile.id.clone(),
                    cx,
                );
                assert_eq!(
                    view.selected_instance.as_deref(),
                    Some(old_instance.id.as_str())
                );
                assert_eq!(
                    view.selected_profile.as_deref(),
                    Some(old_profile.id.as_str())
                );
                view.finish_pack_import(request, Ok(()), cx);
                assert_eq!(view.shell.pack_import.phase, Phase::Complete);
                assert!(view.pack_wizard_active());
            });
            cx.run_until_parked();
            click_pack(cx, "modpack-done")?;
            assert!(view.read_with(cx, |view, _| {
                !view.pack_wizard_active()
                    && view.selected_instance.as_ref() == Some(&instance.id)
                    && view.selected_profile.as_ref() == Some(&profile.id)
                    && view.shell.games.inspected.as_ref() == Some(&profile.id)
            }));
            Ok(())
        })();
        assert!(result.is_ok(), "{result:?}");
    }

    #[gpui::test]
    fn review_puts_decisions_before_a_collapsible_required_file_list(
        cx: &mut gpui::TestAppContext,
    ) {
        let result = (|| -> anyhow::Result<()> {
            let root = tempfile::tempdir()?;
            let source = root.path().join("pack.mrpack");
            crate::test_support::write_modpack(&source)?;
            let mut inspection = block_launcher::modpack::inspect(&source)?;
            inspection.version = Some(String::from("1.2.3"));
            for index in 0..30 {
                inspection
                    .files
                    .push(pack_file(&format!("mods/required-{index}.jar"), true));
            }
            let name = inspection.name.clone();
            let (view, cx) = crate::test_support::window(cx, |window, cx| {
                let mut view = crate::tests::test_view(window, cx);
                view.preferences.reduce_motion = true;
                view.start_pack_wizard(cx);
                view.shell.pack_import.source = Some(source);
                view.shell.pack_import.inspection = Some(inspection);
                view.shell.pack_import.phase = Phase::Review;
                view.adopt_pack_name(&name, cx);
                view
            });
            cx.simulate_resize(gpui::size(gpui::px(1152.), gpui::px(1300.)));
            cx.run_until_parked();
            let controls = [
                "modpack-instance-name",
                "modpack-pack-name",
                "modpack-pack-version",
                "modpack-archive",
                "modpack-choose-other",
                "modpack-target",
                "modpack-optional-mods/optional.jar",
                "modpack-required-toggle",
            ];
            let mut previous = gpui::px(0.);
            for id in controls {
                let bounds = crate::test_support::control_bounds(cx, id)
                    .ok_or_else(|| anyhow::anyhow!("missing {id}"))?;
                assert!(bounds.top() >= previous, "{id} is out of order: {bounds:?}");
                previous = bounds.top();
            }
            assert!(!view.read_with(cx, |view, _| view.shell.pack_import.required.is_open()));
            click_pack(cx, "modpack-required-toggle")?;
            assert!(view.read_with(cx, |view, _| view.shell.pack_import.required.is_open()));
            assert!(
                crate::test_support::control_bounds(cx, "modpack-required-mods/required-29.jar")
                    .is_some()
            );
            click_pack(cx, "modpack-required-toggle")?;
            assert!(!view.read_with(cx, |view, _| view.shell.pack_import.required.is_open()));
            Ok(())
        })();
        assert!(result.is_ok(), "{result:?}");
    }

    #[gpui::test]
    fn stop_retains_the_draft_and_success_wins_the_cancellation_race(
        cx: &mut gpui::TestAppContext,
    ) {
        let result = (|| -> anyhow::Result<()> {
            let root = tempfile::tempdir()?;
            let source = root.path().join("pack.mrpack");
            crate::test_support::write_modpack(&source)?;
            let inspection = block_launcher::modpack::inspect(&source)?;
            let request = uuid::Uuid::new_v4();
            let (view, cx) = crate::test_support::window(cx, |window, cx| {
                let mut view = crate::tests::test_view(window, cx);
                view.preferences.reduce_motion = true;
                view.start_pack_wizard(cx);
                view.shell.pack_import.source = Some(source);
                view.shell.pack_import.inspection = Some(inspection);
                view.shell.pack_import.phase = Phase::Installing;
                view.shell.pack_import.request = Some(request);
                view.shell.pack_import.operation_id = Some(String::from("retry-operation"));
                view.adopt_pack_name("Retained name", cx);
                view
            });
            cx.simulate_resize(gpui::size(gpui::px(1152.), gpui::px(720.)));
            click_pack(cx, "modpack-stop")?;
            assert!(view.read_with(cx, |view, _| view.shell.pack_import.cancel.is_cancelled()));
            assert!(cx.update(|window, _| {
                gpui_base::test_support::find(window, &[], &"modpack-stop".into())
                    .is_some_and(|button| button.disabled() == Some(true))
            }));
            view.update(cx, |view, cx| {
                view.finish_pack_import(request, Err(String::from("cancelled")), cx);
            });
            assert!(view.read_with(cx, |view, cx| {
                view.shell.pack_import.phase == Phase::Stopped
                    && view.shell.pack_import.error.is_none()
                    && view.pack_instance_name(cx).as_deref() == Some("Retained name")
                    && view.shell.pack_import.operation_id.as_deref() == Some("retry-operation")
            }));
            cx.run_until_parked();
            click_pack(cx, "modpack-retry")?;
            assert!(
                view.read_with(cx, |view, cx| {
                    view.shell.pack_import.phase == Phase::Blocked
                        && view.pack_instance_name(cx).as_deref() == Some("Retained name")
                        && view.shell.pack_import.operation_id.as_deref() == Some("retry-operation")
                }),
                "without an engine Retry fails explicitly without losing the draft"
            );

            let racing = uuid::Uuid::new_v4();
            view.update(cx, |view, cx| {
                view.shell.pack_import.phase = Phase::Installing;
                view.shell.pack_import.cancel = CancellationToken::new();
                view.shell.pack_import.request = Some(racing);
                cx.notify();
            });
            click_pack(cx, "modpack-stop")?;
            view.update(cx, |view, cx| view.finish_pack_import(racing, Ok(()), cx));
            assert_eq!(
                view.read_with(cx, |view, _| view.shell.pack_import.phase),
                Phase::Complete
            );
            Ok(())
        })();
        assert!(result.is_ok(), "{result:?}");
    }

    #[gpui::test]
    fn leave_confirmation_preserves_or_cancels_the_live_request_through_the_dialog(
        cx: &mut gpui::TestAppContext,
    ) {
        let result = (|| -> anyhow::Result<()> {
            let request = uuid::Uuid::new_v4();
            let (view, cx) = crate::test_support::window(cx, |window, cx| {
                let mut view = crate::tests::test_view(window, cx);
                view.preferences.reduce_motion = true;
                view.start_pack_wizard(cx);
                view.shell.pack_import.phase = Phase::Installing;
                view.shell.pack_import.request = Some(request);
                view
            });
            cx.simulate_resize(gpui::size(gpui::px(1152.), gpui::px(720.)));
            let cancel = view.read_with(cx, |view, _| view.shell.pack_import.cancel.clone());
            cx.update(|window, cx| {
                view.update(cx, |view, cx| {
                    view.handle(UiAction::CreateInstance, window, cx);
                    view.handle(UiAction::WizardKind(CreationKind::NewInstance), window, cx);
                    view.handle(UiAction::WizardBack, window, cx);
                    view.open_pack_import(window, cx);
                    view.resume_pack_import(String::from("another-instance"), window, cx);
                });
            });
            assert!(view.read_with(cx, |view, _| {
                view.pack_wizard_active() && view.shell.pack_import.request == Some(request)
            }));
            assert!(
                !cancel.is_cancelled(),
                "repeated entry cannot reset a live task"
            );
            click_pack(cx, "cancel-instance")?;
            assert!(view.read_with(cx, |view, _| {
                matches!(view.shell.surfaces.top().map(|layer| &layer.kind),
                    Some(Surface::StopPackImport { request: current, .. }) if *current == request)
            }));
            assert!(!cancel.is_cancelled());
            crate::test_support::press(cx, "escape");
            cx.run_until_parked();
            assert!(view.read_with(cx, |view, _| {
                view.pack_wizard_active()
                    && view.shell.pack_import.request == Some(request)
                    && !view.shell.surfaces.is_open()
            }));
            let restored = cx.update(|window, cx| window.focused(cx));
            assert!(restored.is_some(), "the trigger regains focus");
            click_pack(cx, "cancel-instance")?;
            click_pack(cx, "pack-leave-cancel")?;
            assert_eq!(cx.update(|window, cx| window.focused(cx)), restored);
            assert!(!cancel.is_cancelled());

            cx.update(|window, cx| {
                view.update(cx, |view, cx| {
                    view.handle(UiAction::Tab(super::super::Tab::Settings), window, cx);
                    view.handle(UiAction::CancelForm, window, cx);
                });
            });
            cx.run_until_parked();
            assert_eq!(
                view.read_with(cx, |view, _| view.shell.surfaces.layers().count()),
                1,
                "a repeated exit does not replace or stack confirmations"
            );
            click_pack(cx, "pack-leave-confirm")?;
            assert!(cancel.is_cancelled());
            assert!(view.read_with(cx, |view, _| {
                view.router.tab() == super::super::Tab::Settings
                    && view.shell.pack_import.request.is_none()
                    && !view.pack_wizard_active()
            }));
            view.update(cx, |view, cx| {
                view.finish_pack_import(request, Ok(()), cx);
                assert_ne!(view.shell.pack_import.phase, Phase::Complete);
            });
            Ok(())
        })();
        assert!(result.is_ok(), "{result:?}");
    }

    #[gpui::test]
    fn completion_closes_the_matching_leave_confirmation_without_leaving(
        cx: &mut gpui::TestAppContext,
    ) {
        let result = (|| -> anyhow::Result<()> {
            let request = uuid::Uuid::new_v4();
            let (view, cx) = crate::test_support::window(cx, |window, cx| {
                let mut view = crate::tests::test_view(window, cx);
                view.preferences.reduce_motion = true;
                view.start_pack_wizard(cx);
                view.shell.pack_import.phase = Phase::Installing;
                view.shell.pack_import.request = Some(request);
                view
            });
            cx.simulate_resize(gpui::size(gpui::px(1152.), gpui::px(720.)));
            let cancel = view.read_with(cx, |view, _| view.shell.pack_import.cancel.clone());
            for _ in 0..12 {
                let focused = cx.update(|window, _| {
                    gpui_base::test_support::find(window, &[], &"cancel-instance".into())
                        .and_then(|node| node.focused())
                        == Some(true)
                });
                if focused {
                    break;
                }
                crate::test_support::press(cx, "tab");
                cx.run_until_parked();
            }
            assert!(
                cx.update(|window, _| {
                    gpui_base::test_support::find(window, &[], &"cancel-instance".into())
                        .and_then(|node| node.focused())
                        == Some(true)
                }),
                "the installation Cancel control has focus"
            );
            click_pack(cx, "cancel-instance")?;
            view.update(cx, |view, cx| view.finish_pack_import(request, Ok(()), cx));
            cx.run_until_parked();
            for _ in 0..3 {
                cx.update(|window, cx| window.simulate_next_frame(cx));
                cx.run_until_parked();
            }
            assert!(view.read_with(cx, |view, _| {
                view.pack_wizard_active()
                    && view.shell.pack_import.phase == Phase::Complete
                    && !view.shell.surfaces.is_open()
            }));
            assert!(!cancel.is_cancelled());
            assert!(crate::test_support::control_bounds(cx, "modpack-done").is_some());
            assert!(
                cx.update(|window, _| {
                    gpui_base::test_support::find(window, &[], &"modpack-done".into())
                        .and_then(|node| node.focused())
                        == Some(true)
                }),
                "completion must move focus to the live Done control"
            );
            Ok(())
        })();
        assert!(result.is_ok(), "{result:?}");
    }

    #[gpui::test]
    fn delayed_completion_focus_does_not_steal_a_replacement_dialog(cx: &mut gpui::TestAppContext) {
        let result = (|| -> anyhow::Result<()> {
            let old = uuid::Uuid::new_v4();
            let replacement = uuid::Uuid::new_v4();
            let (view, cx) = crate::test_support::window(cx, |window, cx| {
                let mut view = crate::tests::test_view(window, cx);
                view.preferences.reduce_motion = true;
                view.start_pack_wizard(cx);
                view.shell.pack_import.phase = Phase::Installing;
                view.shell.pack_import.request = Some(old);
                view
            });
            cx.simulate_resize(gpui::size(gpui::px(1152.), gpui::px(720.)));
            click_pack(cx, "cancel-instance")?;
            view.update(cx, |view, cx| view.finish_pack_import(old, Ok(()), cx));
            cx.run_until_parked();
            view.update(cx, |view, cx| {
                view.shell.pack_import.phase = Phase::Installing;
                view.shell.pack_import.request = Some(replacement);
                view.shell.pack_import.completed_request = None;
                cx.notify();
            });
            click_pack(cx, "cancel-instance")?;
            for _ in 0..3 {
                cx.update(|window, cx| window.simulate_next_frame(cx));
                cx.run_until_parked();
            }
            assert!(view.read_with(cx, |view, _| {
                matches!(
                    view.shell.surfaces.top().map(|layer| &layer.kind),
                    Some(Surface::StopPackImport { request, .. }) if *request == replacement
                )
            }));
            assert!(
                cx.update(|window, cx| {
                    gpui_base::active_focus_trap(window, cx)
                        .is_some_and(|trap| trap.contains_focused(window, cx))
                }),
                "the old completion must not steal focus from the new dialog"
            );
            Ok(())
        })();
        assert!(result.is_ok(), "{result:?}");
    }

    #[gpui::test]
    fn application_quit_confirms_only_changed_uncommitted_pack_drafts(
        cx: &mut gpui::TestAppContext,
    ) {
        let (view, cx) = crate::test_support::window(cx, |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.preferences.reduce_motion = true;
            view.start_pack_wizard(cx);
            view.shell.pack_import.phase = Phase::Review;
            view.adopt_pack_name("Pack", cx);
            view
        });
        cx.simulate_resize(gpui::size(gpui::px(1152.), gpui::px(720.)));
        cx.update(|_, cx| crate::initialize_application_quit(cx));
        crate::test_support::press(cx, "cmd-q");
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| !view.shell.surfaces.is_open()));
        view.update(cx, |view, cx| {
            if let Some(name) = &view.shell.pack_import.name {
                name.update(cx, |field, cx| field.set_value("Edited pack", cx));
            }
            cx.notify();
        });
        crate::test_support::press(cx, "cmd-q");
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| {
            view.shell
                .surfaces
                .top()
                .is_some_and(|layer| matches!(layer.kind, Surface::DiscardGameDraft { .. }))
        }));
        let result = click_pack(cx, "discard-game-stay");
        assert!(result.is_ok(), "{result:?}");
        assert!(view.read_with(cx, |view, cx| {
            view.shell
                .pack_import
                .name
                .as_ref()
                .is_some_and(|name| name.read(cx).value() == "Edited pack")
        }));
    }

    #[gpui::test]
    fn completed_pack_closes_a_quit_confirmation_without_quitting(cx: &mut gpui::TestAppContext) {
        let request = uuid::Uuid::new_v4();
        let (view, cx) = crate::test_support::window(cx, |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.preferences.reduce_motion = true;
            view.start_pack_wizard(cx);
            view.shell.pack_import.phase = Phase::Installing;
            view.shell.pack_import.request = Some(request);
            view
        });
        cx.simulate_resize(gpui::size(gpui::px(1152.), gpui::px(720.)));
        cx.update(|_, cx| crate::initialize_application_quit(cx));
        crate::test_support::press(cx, "cmd-q");
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| {
            matches!(
                view.shell.surfaces.top().map(|layer| &layer.kind),
                Some(Surface::StopPackImport { request: current, .. }) if *current == request
            )
        }));
        view.update(cx, |view, cx| view.finish_pack_import(request, Ok(()), cx));
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| {
            view.pack_wizard_active()
                && view.shell.pack_import.phase == Phase::Complete
                && !view.shell.surfaces.is_open()
        }));
        assert!(crate::test_support::control_bounds(cx, "modpack-done").is_some());
    }

    #[gpui::test]
    fn application_quit_keeps_the_pack_request_when_cancelled_or_replaced(
        cx: &mut gpui::TestAppContext,
    ) {
        let old = uuid::Uuid::new_v4();
        let replacement = uuid::Uuid::new_v4();
        let cancel = CancellationToken::new();
        let (view, cx) = crate::test_support::window(cx, |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.preferences.reduce_motion = true;
            view.start_pack_wizard(cx);
            view.shell.pack_import.phase = Phase::Installing;
            view.shell.pack_import.request = Some(old);
            view.shell.pack_import.cancel = cancel.clone();
            view
        });
        cx.simulate_resize(gpui::size(gpui::px(1152.), gpui::px(720.)));
        cx.update(|_, cx| crate::initialize_application_quit(cx));
        crate::test_support::press(cx, "cmd-q");
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| {
            matches!(
                view.shell.surfaces.top().map(|layer| &layer.kind),
                Some(Surface::StopPackImport { request, exit, .. })
                    if *request == old && matches!(exit.as_ref(), UiAction::QuitApplication)
            )
        }));
        let result = click_pack(cx, "pack-leave-cancel");
        assert!(result.is_ok(), "{result:?}");
        assert!(!cancel.is_cancelled());
        crate::test_support::press(cx, "cmd-q");
        cx.run_until_parked();
        let new_cancel = CancellationToken::new();
        view.update(cx, |view, cx| {
            view.shell.pack_import.request = Some(replacement);
            view.shell.pack_import.cancel = new_cancel.clone();
            cx.notify();
        });
        let result = click_pack(cx, "pack-leave-confirm");
        assert!(result.is_ok(), "{result:?}");
        assert!(!new_cancel.is_cancelled());
        assert!(view.read_with(cx, |view, _| {
            view.pack_wizard_active()
                && view.shell.pack_import.request == Some(replacement)
                && !view.shell.surfaces.is_open()
        }));
        crate::test_support::press(cx, "cmd-q");
        cx.run_until_parked();
        let result = click_pack(cx, "pack-leave-confirm");
        assert!(result.is_ok(), "{result:?}");
        assert!(new_cancel.is_cancelled());
        assert!(view.read_with(cx, |view, _| !view.shell.surfaces.is_open()));
    }

    #[gpui::test]
    fn stale_leave_confirmation_cannot_cancel_a_replacement_request(cx: &mut gpui::TestAppContext) {
        let result = (|| -> anyhow::Result<()> {
            let old = uuid::Uuid::new_v4();
            let replacement = uuid::Uuid::new_v4();
            let (view, cx) = crate::test_support::window(cx, |window, cx| {
                let mut view = crate::tests::test_view(window, cx);
                view.preferences.reduce_motion = true;
                view.start_pack_wizard(cx);
                view.shell.pack_import.phase = Phase::Installing;
                view.shell.pack_import.request = Some(old);
                view
            });
            cx.simulate_resize(gpui::size(gpui::px(1152.), gpui::px(720.)));
            click_pack(cx, "cancel-instance")?;
            let cancel = CancellationToken::new();
            view.update(cx, |view, cx| {
                view.shell.pack_import.request = Some(replacement);
                view.shell.pack_import.cancel = cancel.clone();
                cx.notify();
            });
            click_pack(cx, "pack-leave-confirm")?;
            assert!(!cancel.is_cancelled());
            assert!(view.read_with(cx, |view, _| {
                view.pack_wizard_active()
                    && view.shell.pack_import.request == Some(replacement)
                    && !view.shell.surfaces.is_open()
            }));
            Ok(())
        })();
        assert!(result.is_ok(), "{result:?}");
    }

    #[gpui::test]
    fn first_catalog_refresh_keeps_the_import_wizard_until_done(cx: &mut gpui::TestAppContext) {
        let result = (|| -> anyhow::Result<()> {
            let request = uuid::Uuid::new_v4();
            let (view, cx) = crate::test_support::window(cx, |window, cx| {
                let mut view = crate::tests::test_view(window, cx);
                view.preferences.reduce_motion = true;
                view.selected_instance = None;
                view.selected_profile = None;
                view.start_pack_wizard(cx);
                view.shell.pack_import.phase = Phase::Installing;
                view.shell.pack_import.request = Some(request);
                view
            });
            cx.simulate_resize(gpui::size(gpui::px(1152.), gpui::px(720.)));
            view.update(cx, |view, cx| {
                view.receive_pack_created(
                    request,
                    String::from("instance-a"),
                    String::from("Test game"),
                    String::from("profile-a"),
                    cx,
                );
                view.apply_catalog(crate::test_support::ready_snapshot(), cx);
                assert_eq!(view.snapshot.instances.len(), 1);
                assert!(view.pack_wizard_active());
                assert_eq!(view.shell.pack_import.phase, Phase::Installing);
                view.finish_pack_import(request, Ok(()), cx);
                assert!(view.pack_wizard_active());
                assert_eq!(view.shell.pack_import.phase, Phase::Complete);
            });
            cx.run_until_parked();
            click_pack(cx, "modpack-done")?;
            assert!(view.read_with(cx, |view, _| {
                view.selected_instance.as_deref() == Some("instance-a")
                    && view.selected_profile.as_deref() == Some("profile-a")
            }));
            Ok(())
        })();
        assert!(result.is_ok(), "{result:?}");
    }

    #[gpui::test]
    fn completed_first_run_shortcut_closes_to_overview_with_the_import_selected(
        cx: &mut gpui::TestAppContext,
    ) {
        use crate::{Tab, first_run::Stage};

        let result = (|| -> anyhow::Result<()> {
            let (view, cx) = crate::test_support::window(cx, |window, cx| {
                let mut view = crate::tests::test_view(window, cx);
                view.preferences.reduce_motion = true;
                view.preferences.onboarding_complete = false;
                view.saved_preferences.onboarding_complete = false;
                view.shell.first_run.enabled = true;
                view.shell.first_run.stage = Stage::Overview;
                view.router.navigate(Tab::Onboarding);
                view.snapshot = crate::test_support::ready_snapshot();
                view.open_pack_import(window, cx);
                view.shell.pack_import.phase = Phase::Complete;
                view.shell.pack_import.instance = Some(String::from("instance-a"));
                view.shell.pack_import.profile = Some(String::from("profile-a"));
                view
            });
            cx.simulate_resize(gpui::size(gpui::px(1152.), gpui::px(720.)));
            click_pack(cx, "task-close")?;
            assert!(view.read_with(cx, |view, _| {
                view.router.tab() == Tab::Onboarding
                    && view.shell.first_run.stage == Stage::Overview
                    && !view.router.game_wizard()
                    && view.selected_instance.as_deref() == Some("instance-a")
                    && view.selected_profile.as_deref() == Some("profile-a")
            }));
            Ok(())
        })();
        assert!(result.is_ok(), "{result:?}");
    }

    #[gpui::test]
    fn completed_first_run_pack_without_custom_draft_opens_a_fresh_custom_setup(
        cx: &mut gpui::TestAppContext,
    ) {
        use crate::{Tab, first_run::Stage};

        for exit in ["done", "close", "escape"] {
            let result = (|| -> anyhow::Result<()> {
                let (view, cx) = crate::test_support::window(cx, |window, cx| {
                    let mut view = crate::tests::test_view(window, cx);
                    view.preferences.reduce_motion = true;
                    view.preferences.onboarding_complete = false;
                    view.saved_preferences.onboarding_complete = false;
                    view.shell.first_run.enabled = true;
                    view.shell.first_run.stage = Stage::Overview;
                    view.router.navigate(Tab::Onboarding);
                    view.snapshot = crate::test_support::ready_snapshot();
                    view.open_import_games(window, cx);
                    view
                });
                cx.simulate_resize(gpui::size(gpui::px(1152.), gpui::px(1000.)));
                cx.run_until_parked();
                view.update(cx, |view, cx| {
                    let scroll = &view.shell.import_games.source_scroll;
                    scroll.set_offset(gpui::point(gpui::px(0.), -scroll.max_offset().y));
                    cx.notify();
                });
                click_pack(cx, "import-pack")?;
                view.update(cx, |view, cx| {
                    view.shell.pack_import.phase = Phase::Complete;
                    view.shell.pack_import.instance = Some(String::from("instance-a"));
                    view.shell.pack_import.profile = Some(String::from("profile-a"));
                    cx.notify();
                });
                match exit {
                    "done" => click_pack(cx, "modpack-done")?,
                    "close" => click_pack(cx, "task-close")?,
                    _ => {
                        cx.update(|window, cx| {
                            view.update(cx, |view, cx| view.focus.focus(window, cx));
                        });
                        crate::test_support::press(cx, "escape");
                        cx.run_until_parked();
                    }
                }
                click_pack(cx, "setup-game")?;
                click_pack(cx, "setup-custom")?;
                assert!(view.read_with(cx, |view, _| {
                    view.shell.first_run.stage == Stage::GameWizard
                        && view.shell.wizard.kind == CreationKind::NewInstance
                        && view.shell.wizard.step == WizardStep::Setup
                        && !view.pack_wizard_active()
                }));
                assert!(
                    crate::test_support::control_bounds(cx, "wizard-component-version").is_some()
                );
                Ok(())
            })();
            assert!(result.is_ok(), "exit={exit}: {result:?}");
        }
    }

    #[gpui::test]
    fn completed_pack_explicit_close_escape_and_navigation_select_the_result(
        cx: &mut gpui::TestAppContext,
    ) {
        use crate::Tab;

        for exit in ["close", "escape", "settings"] {
            let result = (|| -> anyhow::Result<()> {
                let (view, cx) = crate::test_support::window(cx, |window, cx| {
                    let mut view = crate::tests::test_view(window, cx);
                    view.preferences.reduce_motion = true;
                    view.snapshot = crate::test_support::ready_snapshot();
                    view.selected_instance = Some(String::from("previous-instance"));
                    view.selected_profile = Some(String::from("previous-profile"));
                    view.start_pack_wizard(cx);
                    view.shell.pack_import.phase = Phase::Complete;
                    view.shell.pack_import.instance = Some(String::from("instance-a"));
                    view.shell.pack_import.profile = Some(String::from("profile-a"));
                    view
                });
                cx.simulate_resize(gpui::size(gpui::px(1152.), gpui::px(720.)));
                cx.run_until_parked();
                assert_eq!(
                    view.read_with(cx, |view, _| view.selected_instance.clone()),
                    Some(String::from("previous-instance")),
                    "completion itself must not change the active game"
                );
                match exit {
                    "close" => click_pack(cx, "task-close")?,
                    "escape" => {
                        cx.update(|window, cx| {
                            view.update(cx, |view, cx| view.focus.focus(window, cx));
                        });
                        crate::test_support::press(cx, "escape");
                        cx.run_until_parked();
                    }
                    _ => cx.update(|window, cx| {
                        view.update(cx, |view, cx| {
                            view.handle(UiAction::Tab(Tab::Settings), window, cx);
                        });
                    }),
                }
                assert!(view.read_with(cx, |view, _| {
                    !view.pack_wizard_active()
                        && view.selected_instance.as_deref() == Some("instance-a")
                        && view.selected_profile.as_deref() == Some("profile-a")
                        && view.router.tab()
                            == if exit == "settings" {
                                Tab::Settings
                            } else {
                                Tab::Instances
                            }
                        && (exit == "settings"
                            || matches!(view.router.screen(),
                            crate::navigation::Screen::Record(
                                crate::navigation::RecordRef::Profile(id)) if id == "profile-a"))
                }));
                Ok(())
            })();
            assert!(result.is_ok(), "{exit}: {result:?}");
        }
    }

    #[gpui::test]
    fn a_late_picker_result_cannot_replace_a_new_wizard_request(cx: &mut gpui::TestAppContext) {
        let result = (|| -> anyhow::Result<()> {
            let (view, cx) = crate::test_support::window(cx, |window, cx| {
                let mut view = crate::tests::test_view(window, cx);
                view.preferences.reduce_motion = true;
                view
            });
            cx.simulate_resize(gpui::size(gpui::px(1152.), gpui::px(720.)));
            click_pack(cx, "create-instance")?;
            click_pack(cx, "wizard-kind-pack")?;
            let old = view.read_with(cx, |view, _| view.shell.pack_import.request);
            assert!(old.is_some());
            cx.update(|window, cx| {
                view.update(cx, |view, cx| view.handle(UiAction::WizardBack, window, cx));
            });
            click_pack(cx, "wizard-kind-pack")?;
            let current = view.read_with(cx, |view, _| view.shell.pack_import.request);
            assert!(current.is_some() && current != old);
            cx.simulate_path_prompt_response(|_| Some(vec![PathBuf::from("/stale/pack.zip")]));
            cx.run_until_parked();
            assert!(view.read_with(cx, |view, _| {
                view.shell.pack_import.request == current
                    && view.shell.pack_import.source.is_none()
                    && view.shell.pack_import.error.is_none()
            }));
            cx.simulate_path_prompt_response(|_| None);
            cx.run_until_parked();
            assert!(view.read_with(cx, |view, _| view.shell.pack_import.request.is_none()));
            Ok(())
        })();
        assert!(result.is_ok(), "{result:?}");
    }

    #[gpui::test]
    fn pending_pack_repair_reopens_the_wizard_and_restores_saved_choices(
        cx: &mut gpui::TestAppContext,
    ) {
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
            let config = block_launcher::catalog::ProfileConfiguration {
                name: String::from("Saved pack"),
                memory_mib: 6144,
                game_args: vec![
                    "--width".into(),
                    "1600".into(),
                    "--height".into(),
                    "900".into(),
                ],
                ..Default::default()
            };
            let (instance, profile) = library.create_pack(
                &uuid::Uuid::new_v4().to_string(),
                &source,
                &inspection,
                &selection,
                config,
                &CancellationToken::new(),
            )?;
            library.catalog().save_instance(
                &instance.id,
                block_launcher::catalog::InstanceConfiguration {
                    name: instance.name.clone(),
                    settings: block_launcher::catalog::InstanceSettings {
                        runtime: block_launcher::recipe::RuntimeSelection::Local {
                            java: std::env::temp_dir().join("saved-pack-java"),
                        },
                        width: 1920,
                        height: 1080,
                        fullscreen: false,
                    },
                },
            )?;
            library.catalog().separate_profiles()?;
            let snapshot = library.catalog().snapshot()?;
            let (engine, mut receiver) = crate::Engine::with_root(root.path().to_path_buf())?;
            let (view, cx) = crate::test_support::window(cx, |window, cx| {
                let mut view = crate::tests::test_view(window, cx);
                view.preferences.reduce_motion = true;
                view.snapshot = snapshot;
                view.selected_instance = Some(instance.id.clone());
                view.selected_profile = Some(profile.id.clone());
                view.saved_preferences.game.memory_mib = 4096;
                view.saved_preferences.game.width = 854;
                view.saved_preferences.game.height = 480;
                view.engine = Some(engine);
                view
            });
            cx.simulate_resize(gpui::size(gpui::px(1152.), gpui::px(900.)));
            cx.update(|window, cx| {
                view.update(cx, |view, cx| view.handle(UiAction::Repair, window, cx));
            });
            cx.run_until_parked();
            assert!(view.read_with(cx, |view, _| {
                view.pack_wizard_active()
                    && view.shell.wizard.separated
                    && view.shell.pack_import.instance.as_ref() == Some(&instance.id)
                    && view.shell.pack_import.phase == Phase::Choose
                    && !view.shell.surfaces.is_open()
            }));
            click_pack(cx, "modpack-choose")?;
            let chosen = source.clone();
            cx.simulate_path_prompt_response(move |_| Some(vec![chosen]));
            cx.run_until_parked();
            let runtime = tokio::runtime::Runtime::new()?;
            let message = runtime.block_on(async {
                tokio::time::timeout(std::time::Duration::from_secs(5), async {
                    loop {
                        let message = receiver
                            .recv()
                            .await
                            .ok_or_else(|| anyhow::anyhow!("engine closed before inspection"))?;
                        if matches!(message, crate::Message::PackInspected { .. }) {
                            return Ok::<_, anyhow::Error>(message);
                        }
                    }
                })
                .await?
            })?;
            let (sender, inbox) = tokio::sync::mpsc::channel(1);
            sender.try_send(message)?;
            view.update(cx, |view, cx| {
                view.messages = Some(inbox);
                view.poll(cx);
            });
            cx.run_until_parked();
            assert!(view.read_with(cx, |view, cx| {
                view.shell.pack_import.phase == Phase::Review
                    && view.shell.pack_import.selected_files == selection.optional_files
                    && !view.shell.pack_import.toggleable_file("mods/optional.jar")
                    && view.pack_instance_name(cx).as_deref() == Some("Saved pack")
                    && view.pack_choose_again_label(&view.shell.pack_import, Language::English)
                        == "Select original archive"
            }));
            assert!(crate::test_support::control_bounds(cx, "modpack-resume-settings").is_some());
            assert!(crate::test_support::control_bounds(cx, "modpack-resume-effect").is_some());
            assert_eq!(
                view.read_with(cx, |view, _| {
                    view.pack_resume_launch_summary(&view.shell.pack_import, Language::English)
                }),
                Some(format!(
                    "Saved launch settings: {} · 6 GB · 1920 × 1080",
                    std::env::temp_dir().join("saved-pack-java").display()
                ))
            );
            let field = view
                .read_with(cx, |view, _| view.shell.pack_import.name.clone())
                .ok_or_else(|| anyhow::anyhow!("saved name is not shown"))?;
            cx.update(|window, cx| {
                field.update(cx, |field, cx| {
                    field.replace_text_in_range(None, "altered", window, cx);
                });
            });
            assert_eq!(field.read_with(cx, |field, _| field.value()), "Saved pack");
            let request = uuid::Uuid::new_v4();
            view.update(cx, |view, cx| {
                view.shell.pack_import.phase = Phase::Installing;
                view.shell.pack_import.request = Some(request);
                view.finish_pack_import(request, Ok(()), cx);
            });
            cx.run_until_parked();
            click_pack(cx, "modpack-done")?;
            assert!(view.read_with(cx, |view, _| {
                view.shell.games.inspected.as_ref() == Some(&instance.id)
                    && view.selected_instance.as_ref() == Some(&instance.id)
                    && view.selected_profile.as_ref() == Some(&profile.id)
            }));
            Ok(())
        })();
        assert!(result.is_ok(), "{result:?}");
    }

    #[gpui::test]
    fn pending_modded_pack_uses_the_first_instance_profile_for_summary_and_completion(
        cx: &mut gpui::TestAppContext,
    ) {
        let result =
            (|| -> anyhow::Result<()> {
                use block_launcher::catalog::{ProfileConfiguration, ProfileInheritance};
                use block_launcher::recipe::RuntimeSelection;

                let root = tempfile::tempdir()?;
                let source = root.path().join("fabric.mrpack");
                write_fabric_pack(&source)?;
                let inspection = block_launcher::modpack::inspect(&source)?;
                let library = block_launcher::library::Library::new(
                    root.path().to_path_buf(),
                    std::sync::Arc::new(|_| {}),
                    std::sync::Arc::new(|_| {}),
                )?;
                let (instance, mut first) = library.create_pack(
                    &uuid::Uuid::new_v4().to_string(),
                    &source,
                    &inspection,
                    &PackSelection::default(),
                    ProfileConfiguration {
                        name: "Fabric pack".into(),
                        ..Default::default()
                    },
                    &CancellationToken::new(),
                )?;
                library.catalog().separate_profiles()?;
                first.target_id = "vanilla".into();
                first.memory_mib = 3072;
                first.runtime = RuntimeSelection::Local {
                    java: std::env::temp_dir().join("first-profile-java"),
                };
                first.inheritance = ProfileInheritance::default();
                first.game_args = vec![
                    "--width".into(),
                    "1024".into(),
                    "--height".into(),
                    "768".into(),
                ];
                library.catalog().save_profile(first.clone())?;
                let target = instance
                    .pack
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("pack target missing"))?
                    .target_id
                    .clone();
                let second = library.catalog().create_profile_configured(
                    &instance.id,
                    ProfileConfiguration {
                        name: "Fabric profile".into(),
                        target_id: target.clone(),
                        memory_mib: 6144,
                        runtime: RuntimeSelection::Local {
                            java: std::env::temp_dir().join("second-profile-java"),
                        },
                        game_args: vec![
                            "--width".into(),
                            "1600".into(),
                            "--height".into(),
                            "900".into(),
                        ],
                        ..Default::default()
                    },
                )?;
                let snapshot = library.catalog().snapshot()?;
                assert_eq!(snapshot.profiles[0].id, first.id);
                assert_eq!(snapshot.profiles[0].target_id, "vanilla");
                assert_eq!(snapshot.profiles[1].target_id, target);

                let (engine, mut receiver) = crate::Engine::with_root(root.path().to_path_buf())?;
                let (view, cx) = crate::test_support::window(cx, |window, cx| {
                    let mut view = crate::tests::test_view(window, cx);
                    view.preferences.reduce_motion = true;
                    view.snapshot = snapshot;
                    view.selected_instance = Some(instance.id.clone());
                    view.selected_profile = Some(second.id.clone());
                    view.engine = Some(engine);
                    view
                });
                cx.simulate_resize(gpui::size(gpui::px(1152.), gpui::px(900.)));
                cx.update(|window, cx| {
                    view.update(cx, |view, cx| view.handle(UiAction::Repair, window, cx));
                });
                cx.run_until_parked();
                assert!(view.read_with(cx, |view, _| {
                    view.pack_wizard_active()
                        && view.shell.pack_import.profile.as_ref() == Some(&first.id)
                }));
                click_pack(cx, "modpack-choose")?;
                let chosen = source.clone();
                cx.simulate_path_prompt_response(move |_| Some(vec![chosen]));
                cx.run_until_parked();
                receive_inspection(&view, cx, &mut receiver)?;
                assert_eq!(
                    view.read_with(cx, |view, _| {
                        view.pack_resume_launch_summary(&view.shell.pack_import, Language::English)
                    }),
                    Some(format!(
                        "Saved launch settings: {} · 3 GB · 1024 × 768",
                        std::env::temp_dir().join("first-profile-java").display()
                    ))
                );

                // Keep the test offline: the real engine reports its chosen Profile
                // before trying to read this now-unavailable archive.
                std::fs::write(&source, b"unavailable")?;
                click_pack(cx, "modpack-confirm")?;
                let runtime = tokio::runtime::Runtime::new()?;
                let created = runtime.block_on(async {
                    tokio::time::timeout(std::time::Duration::from_secs(5), async {
                        loop {
                            let message = receiver.recv().await.ok_or_else(|| {
                                anyhow::anyhow!("engine closed before pack creation")
                            })?;
                            if matches!(message, crate::Message::PackCreated { .. }) {
                                return Ok::<_, anyhow::Error>(message);
                            }
                        }
                    })
                    .await?
                })?;
                let request = match &created {
                    crate::Message::PackCreated {
                        request, profile, ..
                    } => {
                        assert_eq!(profile, &first.id);
                        *request
                    }
                    _ => unreachable!("the receiver returned a pack creation"),
                };
                let finished = runtime.block_on(async {
                    tokio::time::timeout(std::time::Duration::from_secs(5), async {
                        loop {
                            let message = receiver.recv().await.ok_or_else(|| {
                                anyhow::anyhow!("engine closed before pack finished")
                            })?;
                            if matches!(message, crate::Message::PackFinished { .. }) {
                                return Ok::<_, anyhow::Error>(message);
                            }
                        }
                    })
                    .await?
                })?;
                assert!(matches!(
                    finished,
                    crate::Message::PackFinished {
                        request: finished_request,
                        result: Err(_),
                    } if finished_request == request
                ));
                let (sender, inbox) = tokio::sync::mpsc::channel(2);
                sender.try_send(created)?;
                // Completion is simulated at the engine boundary; no install or
                // network request is needed to verify the selected saved Profile.
                sender.try_send(crate::Message::PackFinished {
                    request,
                    result: Ok(()),
                })?;
                view.update(cx, |view, cx| {
                    view.messages = Some(inbox);
                    view.poll(cx);
                });
                cx.run_until_parked();
                click_pack(cx, "modpack-done")?;
                assert!(view.read_with(cx, |view, _| {
                    view.selected_instance.as_ref() == Some(&instance.id)
                        && view.selected_profile.as_ref() == Some(&first.id)
                }));
                Ok(())
            })();
        assert!(result.is_ok(), "{result:?}");
    }

    #[gpui::test]
    fn first_run_pack_shortcut_restores_the_same_custom_draft_after_cancel_and_done(
        cx: &mut gpui::TestAppContext,
    ) {
        use crate::first_run::{Action as FirstRunAction, Stage};

        for origin in [Stage::Overview, Stage::GameChoice] {
            let result = (|| -> anyhow::Result<()> {
                let (view, cx) = crate::test_support::window(cx, |window, cx| {
                    let mut view = crate::tests::test_view(window, cx);
                    view.preferences.reduce_motion = true;
                    view.preferences.onboarding_complete = false;
                    view.saved_preferences.onboarding_complete = false;
                    view.snapshot = crate::test_support::ready_snapshot();
                    view.begin_first_run(cx);
                    view
                });
                cx.simulate_resize(gpui::size(gpui::px(1152.), gpui::px(1000.)));
                cx.update(|window, cx| {
                    view.update(cx, |view, cx| {
                        view.handle(UiAction::FirstRun(FirstRunAction::Game), window, cx);
                        view.handle(UiAction::FirstRun(FirstRunAction::Custom), window, cx);
                    });
                });
                let (name, profile_name) = view.read_with(cx, |view, _| {
                    let name = match &view.form {
                        Some(crate::Form::Instance(form)) => Some(form.name.clone()),
                        _ => None,
                    };
                    let profile = view
                        .shell
                        .wizard
                        .profile
                        .as_ref()
                        .map(|draft| draft.name.clone());
                    (name, profile)
                });
                let name = name.ok_or_else(|| anyhow::anyhow!("custom name is missing"))?;
                let profile_name =
                    profile_name.ok_or_else(|| anyhow::anyhow!("custom profile is missing"))?;
                cx.update(|window, cx| {
                    name.update(cx, |input, cx| {
                        input.set_value("Original draft", cx);
                        input.replace_and_mark_text_in_range(None, "草", None, window, cx);
                    });
                    profile_name.update(cx, |input, cx| input.set_value("Custom profile", cx));
                    view.update(cx, |view, cx| {
                        if let Some(crate::Form::Instance(form)) = &mut view.form {
                            form.loader = LoaderKind::Forge;
                            form.loader_version = Some(String::from("47.2.20"));
                        }
                        view.handle(UiAction::FirstRun(FirstRunAction::Back), window, cx);
                        if origin == Stage::GameChoice {
                            view.handle(UiAction::FirstRun(FirstRunAction::Game), window, cx);
                        }
                        view.open_import_games(window, cx);
                    });
                });
                cx.run_until_parked();
                let value = name.read_with(cx, |input, _| input.value());
                assert!(name.read_with(cx, |input, _| input.is_composing()));
                for _ in 0..2 {
                    view.update(cx, |view, cx| {
                        let scroll = &view.shell.import_games.source_scroll;
                        scroll.set_offset(gpui::point(gpui::px(0.), -scroll.max_offset().y));
                        cx.notify();
                    });
                    cx.run_until_parked();
                    click_pack(cx, "import-pack")?;
                    view.update(cx, |view, cx| {
                        assert!(
                            view.shell
                                .pack_import
                                .return_to
                                .as_ref()
                                .is_some_and(|context| context.draft.is_some())
                        );
                        let frozen = view.visual_snapshot(cx);
                        assert!(frozen.shell.pack_import.return_to.is_none());
                        assert!(
                            view.shell
                                .pack_import
                                .return_to
                                .as_ref()
                                .is_some_and(|context| context.draft.is_some()),
                            "a visual snapshot must not take the live draft"
                        );
                    });
                    click_pack(cx, "cancel-instance")?;
                    assert!(
                        view.read_with(cx, |view, cx| {
                            view.shell.first_run.stage == origin
                                && matches!(&view.form, Some(crate::Form::Instance(form))
                                if form.name.entity_id() == name.entity_id()
                                    && form.name.read(cx).value() == value
                                    && form.name.read(cx).is_composing()
                                    && form.loader == LoaderKind::Forge
                                    && form.loader_version.as_deref() == Some("47.2.20"))
                                && view.shell.wizard.profile.as_ref().is_some_and(|profile| {
                                    profile.name.entity_id() == profile_name.entity_id()
                                        && profile.name.read(cx).value() == "Custom profile"
                                })
                        }),
                        "origin={origin:?}: Cancel must restore the same live entities"
                    );
                }
                view.update(cx, |view, cx| {
                    let scroll = &view.shell.import_games.source_scroll;
                    scroll.set_offset(gpui::point(gpui::px(0.), -scroll.max_offset().y));
                    cx.notify();
                });
                cx.run_until_parked();
                click_pack(cx, "import-pack")?;
                view.update(cx, |view, cx| {
                    view.shell.pack_import.phase = Phase::Complete;
                    view.shell.pack_import.instance = Some(String::from("instance-a"));
                    view.shell.pack_import.profile = Some(String::from("profile-a"));
                    cx.notify();
                });
                click_pack(cx, "modpack-done")?;
                assert!(
                    view.read_with(cx, |view, cx| {
                        view.shell.first_run.stage == Stage::Overview
                            && matches!(&view.form, Some(crate::Form::Instance(form))
                            if form.name.entity_id() == name.entity_id()
                                && form.name.read(cx).value() == value
                                && form.name.read(cx).is_composing()
                                && form.loader_version.as_deref() == Some("47.2.20"))
                            && view.shell.wizard.profile.as_ref().is_some_and(|profile| {
                                profile.name.entity_id() == profile_name.entity_id()
                                    && profile.name.read(cx).value() == "Custom profile"
                            })
                    }),
                    "origin={origin:?}: Done must restore the same live draft"
                );
                cx.update(|window, cx| {
                    view.update(cx, |view, cx| {
                        view.handle(UiAction::FirstRun(FirstRunAction::Game), window, cx);
                        view.handle(UiAction::FirstRun(FirstRunAction::Custom), window, cx);
                    });
                });
                assert!(
                    view.read_with(cx, |view, _| {
                        view.shell.first_run.stage == Stage::GameWizard
                            && matches!(&view.form, Some(crate::Form::Instance(form))
                            if form.name.entity_id() == name.entity_id())
                    }),
                    "Custom must reopen the retained draft"
                );
                Ok(())
            })();
            assert!(result.is_ok(), "origin={origin:?}: {result:?}");
        }
    }

    #[gpui::test]
    fn closing_a_pack_shortcut_confirms_loss_of_its_changed_parent_draft(
        cx: &mut gpui::TestAppContext,
    ) {
        let (view, cx) = crate::test_support::window(cx, |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.preferences.reduce_motion = true;
            view.begin_game_wizard(cx);
            view.router.navigate(crate::Tab::Onboarding);
            view.router.enter_game_wizard();
            view.shell.first_run.enabled = true;
            view.shell.first_run.stage = crate::first_run::Stage::GameWizard;
            if let Some(super::super::Form::Instance(form)) = &view.form {
                form.name
                    .update(cx, |input, cx| input.set_value("Parent draft", cx));
            }
            view
        });
        cx.simulate_resize(gpui::size(gpui::px(1152.), gpui::px(900.)));
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.handle(
                    UiAction::FirstRun(crate::first_run::Action::Back),
                    window,
                    cx,
                );
                view.handle(UiAction::PackImport(Action::Open), window, cx);
            });
        });
        cx.simulate_path_prompt_response(|_| None);
        cx.run_until_parked();
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                assert!(view.held_pack_parent_draft_id(cx).is_some());
                view.handle(UiAction::CloseWindow, window, cx);
                assert!(view.shell.surfaces.top().is_some_and(|layer| {
                    matches!(layer.kind, Surface::DiscardGameDraft { .. })
                }));
            });
        });
        let result = click_pack(cx, "discard-game-stay");
        assert!(result.is_ok(), "{result:?}");
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.handle(UiAction::CancelForm, window, cx);
                assert!(
                    matches!(&view.form, Some(super::super::Form::Instance(form))
                    if form.name.read(cx).value() == "Parent draft")
                );
            });
        });
    }

    #[gpui::test]
    fn directory_shortcut_cancel_returns_to_its_original_drawer_and_page(
        cx: &mut gpui::TestAppContext,
    ) {
        use crate::{Tab, first_run::Stage, shell::Drawer};

        for origin in [None, Some(Stage::Overview), Some(Stage::GameChoice)] {
            let result = (|| -> anyhow::Result<()> {
                let (view, cx) = crate::test_support::window(cx, |window, cx| {
                    let mut view = crate::tests::test_view(window, cx);
                    view.preferences.reduce_motion = true;
                    if let Some(stage) = origin {
                        view.preferences.onboarding_complete = false;
                        view.saved_preferences.onboarding_complete = false;
                        view.shell.first_run.enabled = true;
                        view.shell.first_run.stage = stage;
                        view.router.navigate(Tab::Onboarding);
                        if stage == Stage::GameChoice {
                            view.router.enter_game_choice();
                        }
                    } else {
                        view.router.navigate(Tab::Home);
                    }
                    view.open_import_games(window, cx);
                    view
                });
                let parent = view.read_with(cx, |view, _| view.router.entry_id());
                cx.simulate_resize(gpui::size(gpui::px(1152.), gpui::px(1000.)));
                cx.run_until_parked();
                view.update(cx, |view, cx| {
                    let scroll = &view.shell.import_games.source_scroll;
                    scroll.set_offset(gpui::point(gpui::px(0.), -scroll.max_offset().y));
                    cx.notify();
                });
                cx.run_until_parked();
                click_pack(cx, "import-pack")?;
                assert!(view.read_with(cx, |view, _| view.pack_wizard_active()));
                assert_eq!(
                    view.read_with(cx, |view, _| view.router.tab()),
                    if origin.is_some() {
                        Tab::Onboarding
                    } else {
                        Tab::Home
                    }
                );
                click_pack(cx, "cancel-instance")?;
                assert!(view.read_with(cx, |view, _| {
                    matches!(
                        view.shell.surfaces.top().map(|layer| &layer.kind),
                        Some(Surface::Drawer(Drawer::ImportGames))
                    ) && view.router.tab()
                        == if origin.is_some() {
                            Tab::Onboarding
                        } else {
                            Tab::Home
                        }
                        && origin.is_none_or(|stage| view.shell.first_run.stage == stage)
                        && (origin != Some(Stage::GameChoice) || view.wizard_task_active())
                        && view.router.entry_id() == parent
                }));
                Ok(())
            })();
            assert!(result.is_ok(), "origin={origin:?}: {result:?}");
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
            let operation = uuid::Uuid::new_v4().to_string();
            let (view, cx) = crate::test_support::window(cx, |window, cx| {
                let mut view = crate::tests::test_view(window, cx);
                view.preferences.reduce_motion = true;
                view.engine = Some(engine);
                view.start_pack_wizard(cx);
                view.shell.pack_import.phase = Phase::Blocked;
                view.shell.pack_import.source = Some(source.clone());
                view.shell.pack_import.operation_id = Some(operation.clone());
                view.shell
                    .pack_import
                    .selected_files
                    .insert(String::from("mods/optional.jar"));
                view.adopt_pack_name("My selected name", cx);
                view
            });
            cx.simulate_resize(gpui::size(gpui::px(1152.), gpui::px(900.)));
            click_pack(cx, "modpack-recheck")?;
            let request = view
                .read_with(cx, |view, _| view.shell.pack_import.request)
                .ok_or_else(|| anyhow::anyhow!("recheck did not start an inspection"))?;
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
                view.start_pack_wizard(cx);
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
            cx.update(|_, cx| {
                view.update(cx, |view, cx| {
                    view.pack_import_closed();
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
        cx.update(|_, cx| {
            view.update(cx, |view, cx| {
                view.poll(cx);
                assert!(
                    view.downloads
                        .as_ref()
                        .is_some_and(|tracker| std::sync::Arc::ptr_eq(tracker, &pack))
                );
                view.pack_import_closed();
                assert!(view.downloads.is_none(), "the released tracker is detached");
            });
        });

        let next = std::sync::Arc::new(std::sync::Mutex::new(
            crate::tasks::DownloadTracker::default(),
        ));
        cx.update(|_, cx| {
            view.update(cx, |view, _| {
                view.shell.pack_import.tracker = Some(pack.clone());
                view.downloads = Some(next.clone());
                view.pack_import_closed();
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
            Phase::Stopped,
            Phase::Complete,
        ] {
            let state = State {
                phase,
                ..State::default()
            };
            for language in Language::ALL {
                let (title, description) = state.chrome(language);
                let context = format!("{phase:?}/{language:?}");
                assert!(!title.trim().is_empty(), "{context}");
                assert!(!description.trim().is_empty(), "{context}");
                assert!(!title.contains("modpack."), "{context}: {title}");
                assert!(
                    !description.contains("modpack."),
                    "{context}: {description}"
                );
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
    fn compact_curseforge_actions_are_reachable_and_operable_in_each_language(
        cx: &mut gpui::TestAppContext,
    ) {
        let result = (|| -> anyhow::Result<()> {
            let root = tempfile::tempdir()?;
            let source = root.path().join("curseforge.zip");
            write_curse_pack(&source)?;
            let inspection = block_launcher::modpack::inspect(&source)?;
            assert_eq!(inspection.format, PackFormat::CurseForge);
            assert_eq!(inspection.missing_files.len(), 2);
            let name = inspection.name.clone();
            let (view, cx) = crate::test_support::window(cx, |window, cx| {
                let mut view = crate::tests::test_view(window, cx);
                view.preferences.reduce_motion = true;
                view.start_pack_wizard(cx);
                view.shell.pack_import.inspection = Some(inspection);
                view.shell.pack_import.source = Some(source);
                view.shell.pack_import.phase = Phase::Review;
                view.adopt_pack_name(&name, cx);
                view
            });
            cx.simulate_resize(gpui::size(gpui::px(360.), gpui::px(540.)));
            for language in Language::ALL {
                view.update(cx, |view, cx| {
                    view.preferences.language = Some(language);
                    view.shell
                        .pack_import
                        .scroll
                        .set_offset(gpui::point(gpui::px(0.), gpui::px(0.)));
                    cx.notify();
                });
                cx.run_until_parked();
                for selector in [
                    "modpack-copy-id-990001-9900011",
                    "modpack-open-project-990001",
                ] {
                    let bounds = reveal_pack_item(&view, cx, selector)?;
                    let body = view.read_with(cx, |view, _| view.shell.pack_import.scroll.bounds());
                    assert!(
                        bounds.left() >= body.left() + gpui::px(16.)
                            && bounds.right() <= body.right() - gpui::px(16.),
                        "{language:?}/{selector}: {bounds:?} outside padded body {body:?}"
                    );
                }
                cx.update(|_, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string("sentinel".into()))
                });
                reveal_pack_item(&view, cx, "modpack-copy-id-990001-9900011")?;
                click_pack(cx, "modpack-copy-id-990001-9900011")?;
                assert_eq!(
                    cx.update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text())),
                    Some(String::from("990001/9900011"))
                );
                cx.update(|_, cx| cx.open_url("block:test-sentinel"));
                reveal_pack_item(&view, cx, "modpack-open-project-990001")?;
                click_pack(cx, "modpack-open-project-990001")?;
                assert_eq!(
                    cx.cx.opened_url().as_deref(),
                    Some("https://www.curseforge.com/projects/990001")
                );
            }
            Ok(())
        })();
        assert!(result.is_ok(), "{result:?}");
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
                let title = crate::test_support::control_bounds(cx, "modpack-heading")
                    .ok_or_else(|| anyhow::anyhow!("wizard heading is not visible"))?;
                let footer = crate::test_support::control_bounds(cx, "modpack-confirm")
                    .ok_or_else(|| anyhow::anyhow!("confirm action is not visible"))?;
                let body = view.read_with(cx, |view, _| view.shell.pack_import.scroll.bounds());
                let viewport = cx.update(|window, _| window.viewport_size());
                assert_eq!(
                    view.read_with(cx, |view, _| view.shell.pack_import.chrome(language).0),
                    tr!(language, "modpack.header.review.title")
                );
                assert!(
                    title.top() >= body.top() && title.bottom() <= body.bottom(),
                    "{language:?}: the heading is inside the scrollable body: {title:?}"
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

    /// A result settles only the request the session is awaiting; a second result
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
    /// session did not issue is reported as foreign, and only the current request
    /// fills the session's catalog.
    #[gpui::test]
    fn a_loader_result_is_only_this_sessions_when_its_request_is_current(
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
    /// every other choice in the wizard keeps that editor and its draft.
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
            None,
            "a committed import is selected only after Done"
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

    #[gpui::test]
    fn first_unreadable_archive_displays_the_attempted_path(cx: &mut gpui::TestAppContext) {
        let bad = PathBuf::from("/local/first-bad-pack.zip");
        let request = uuid::Uuid::new_v4();
        let (view, cx) = crate::test_support::window(cx, |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.preferences.reduce_motion = true;
            view.start_pack_wizard(cx);
            view.shell.pack_import.phase = Phase::Inspecting;
            view.shell.pack_import.request = Some(request);
            view
        });
        cx.simulate_resize(gpui::size(gpui::px(1152.), gpui::px(720.)));
        view.update(cx, |view, cx| {
            view.receive_pack_inspected(request, bad.clone(), Err("unreadable".into()), cx);
        });
        cx.run_until_parked();
        assert_eq!(
            view.read_with(cx, |view, _| view.shell.pack_import.source.clone()),
            Some(bad)
        );
        assert!(crate::test_support::control_bounds(cx, "modpack-archive").is_some());
    }

    #[gpui::test]
    fn unreadable_replacement_displays_attempted_path_and_keeps_editor_ownership(
        cx: &mut gpui::TestAppContext,
    ) {
        let result = (|| -> anyhow::Result<()> {
            let root = tempfile::tempdir()?;
            let original = root.path().join("original.mrpack");
            crate::test_support::write_modpack(&original)?;
            let inspection = block_launcher::modpack::inspect(&original)?;
            let initial = uuid::Uuid::new_v4();
            let (view, cx) = crate::test_support::window(cx, |window, cx| {
                let mut view = crate::tests::test_view(window, cx);
                view.preferences.reduce_motion = true;
                view.start_pack_wizard(cx);
                view.shell.pack_import.request = Some(initial);
                view
            });
            cx.simulate_resize(gpui::size(gpui::px(1152.), gpui::px(900.)));
            view.update(cx, |view, cx| {
                view.receive_pack_inspected(
                    initial,
                    original.clone(),
                    Ok((inspection.clone(), PackSelection::default())),
                    cx,
                );
            });
            let field = view
                .read_with(cx, |view, _| view.shell.pack_import.name.clone())
                .ok_or_else(|| anyhow::anyhow!("the original name editor is missing"))?;
            field.update(cx, |field, cx| field.set_value("Kept draft", cx));

            let bad = root.path().join("bad.zip");
            let failed = uuid::Uuid::new_v4();
            view.update(cx, |view, cx| {
                view.shell.pack_import.begin_inspection();
                view.shell.pack_import.request = Some(failed);
                view.receive_pack_inspected(failed, bad.clone(), Err("unreadable".into()), cx);
            });
            cx.run_until_parked();
            assert_eq!(
                view.read_with(cx, |view, _| view.shell.pack_import.source.clone()),
                Some(bad)
            );
            assert!(crate::test_support::control_bounds(cx, "modpack-archive").is_some());
            assert!(view.read_with(cx, |view, cx| {
                view.shell.pack_import.name.as_ref().is_some_and(|name| {
                    name.entity_id() == field.entity_id() && name.read(cx).value() == "Kept draft"
                })
            }));

            let same = uuid::Uuid::new_v4();
            view.update(cx, |view, cx| {
                view.shell.pack_import.begin_inspection();
                view.shell.pack_import.request = Some(same);
                view.receive_pack_inspected(
                    same,
                    original.clone(),
                    Ok((inspection.clone(), PackSelection::default())),
                    cx,
                );
            });
            assert!(
                view.read_with(cx, |view, cx| {
                    view.shell.pack_import.name.as_ref().is_some_and(|name| {
                        name.entity_id() == field.entity_id()
                            && name.read(cx).value() == "Kept draft"
                    })
                }),
                "re-inspecting the accepted archive retains its editor"
            );

            let other = uuid::Uuid::new_v4();
            let mut other_inspection = inspection;
            other_inspection.name = String::from("Other archive");
            view.update(cx, |view, cx| {
                view.shell.pack_import.begin_inspection();
                view.shell.pack_import.request = Some(other);
                view.receive_pack_inspected(
                    other,
                    root.path().join("other.mrpack"),
                    Ok((other_inspection, PackSelection::default())),
                    cx,
                );
            });
            assert!(
                view.read_with(cx, |view, cx| {
                    view.shell.pack_import.name.as_ref().is_some_and(|name| {
                        name.entity_id() != field.entity_id()
                            && name.read(cx).value() == "Other archive"
                    })
                }),
                "a different accepted archive starts with its own name"
            );
            Ok(())
        })();
        assert!(result.is_ok(), "{result:?}");
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
            view.shell.pack_import.source = Some(source.clone());
            view.shell.pack_import.accepted_source = Some(source.clone());
            view.shell.pack_import.begin_inspection();
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
            view.shell.pack_import.begin_inspection();
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

        // An unreadable replacement displays the attempted source and retains the editor.
        let failed = uuid::Uuid::new_v4();
        view.update(cx, |view, cx| {
            view.shell.pack_import.request = Some(failed);
            view.receive_pack_inspected(
                failed,
                PathBuf::from("bad.mrpack"),
                Err(String::from("unreadable archive")),
                cx,
            );
            assert_eq!(
                view.shell.pack_import.source.as_deref(),
                Some(std::path::Path::new("bad.mrpack"))
            );
            assert_eq!(
                view.shell.pack_import.name.as_ref().map(Entity::entity_id),
                Some(field.entity_id())
            );
        });

        let inspection = (|| -> anyhow::Result<_> {
            let root = tempfile::tempdir()?;
            let path = root.path().join("other.mrpack");
            crate::test_support::write_modpack(&path)?;
            let mut inspection = block_launcher::modpack::inspect(&path)?;
            inspection.name = String::from("Other pack");
            Ok(inspection)
        })();
        assert!(inspection.is_ok(), "{inspection:?}");
        let Ok(inspection) = inspection else {
            return;
        };
        let next = uuid::Uuid::new_v4();
        view.update(cx, |view, cx| {
            view.shell.pack_import.begin_inspection();
            view.shell.pack_import.request = Some(next);
            view.receive_pack_inspected(
                next,
                PathBuf::from("other.mrpack"),
                Ok((inspection, PackSelection::default())),
                cx,
            );
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
            view.shell.pack_import.source = Some(PathBuf::from("pack.mrpack"));
            view.shell.pack_import.begin_inspection();
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
            view.shell.pack_import.begin_inspection();
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
