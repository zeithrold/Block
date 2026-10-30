//! Debug-only, network-free UI states. These fixtures never authenticate or install games.

use std::sync::{Arc, Mutex};

use block_auth::{Account, AccountProvider, AccountRef};
use block_launcher::{
    DownloadEvent,
    catalog::{
        DataPolicy, InstallStatus, Instance, LaunchTarget, LoaderKind, Profile, ProfileColor,
        ProfileIcon, ProfileMode, Snapshot,
    },
    game_import::{DetectedLoader, DiscoveryReport, GameCandidate, ImportSourceReport, SourceKind},
    recipe::{Compatibility, RuntimeSelection},
};
use gpui::{Context, Window};

use crate::{
    Block, Tab, UiAction, account_id,
    appearance::Appearance,
    browser::CollectionLoad,
    first_run::{self, Stage as FirstRunStage},
    game_wizard::{WizardPage, WizardStep},
    import_games, onboarding,
    shell::{Drawer, Surface},
    tasks::DownloadTracker,
};

pub const SCENARIOS: &[&str] = &[
    "home-empty",
    "home-ready",
    "home-downloading",
    "home-error",
    "games-bundled",
    "games-separated",
    "games-detail",
    "games-instance-settings",
    "wizard-setup",
    "wizard-review",
    "wizard-optifine",
    "wizard-optifine-selected",
    "accounts-list",
    "accounts-detail",
    "accounts-offline-detail",
    "accounts-providers",
    "provider-add",
    "provider-http-warning",
    "remove-provider-confirmation",
    "auth-choose",
    "auth-information",
    "auth-authlib-information",
    "auth-authlib-http-warning",
    "auth-authlib-characters",
    "auth-authlib-complete",
    "auth-authlib-provider-add",
    "auth-offline-information",
    "auth-offline-error",
    "auth-offline-complete",
    "auth-device-preparing",
    "auth-device-waiting",
    "auth-waiting",
    "auth-verifying",
    "auth-complete",
    "auth-saving",
    "auth-error",
    "settings-general",
    "settings-game",
    "settings-profiles",
    "settings-downloads",
    "settings-socks5",
    "settings-storage",
    "about",
    "setup-welcome",
    "setup-overview",
    "setup-overview-ready",
    "setup-game-choice",
    "setup-import-sources",
    "setup-import-list",
    "setup-account",
    "setup-game",
    "home-profile-drawer",
    "home-account-drawer",
    "home-downloads-drawer",
    "toast-motion",
    "toast-motion-reduced",
    "download-files",
    "wizard-version-picker",
    "wizard-loader-picker",
    "wizard-fabric-picker",
    "wizard-quilt-picker",
    "wizard-neoforge-picker",
    "wizard-forge-picker",
    "wizard-loader-loading",
    "wizard-loader-error",
    "wizard-loader-empty",
    "wizard-loader-search-empty",
    "wizard-configuration",
    "remove-confirmation",
    "remove-account-confirmation",
    "separate-confirmation",
    "games-help",
];

/// Wizard pages whose loader catalog is synthetic, so no fetch runs.
const LOADER_FIXTURES: &[&str] = &[
    "wizard-fabric-picker",
    "wizard-quilt-picker",
    "wizard-neoforge-picker",
    "wizard-forge-picker",
    "wizard-loader-loading",
    "wizard-loader-error",
    "wizard-loader-empty",
    "wizard-loader-search-empty",
];

/// Available only in debug builds, paired with a disposable CLI data directory.
#[derive(Debug)]
pub struct ScenarioOptions {
    pub name: String,
    pub dark: bool,
}

impl Block {
    pub(super) fn apply_scenario(
        &mut self,
        options: &ScenarioOptions,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            SCENARIOS.contains(&options.name.as_str()),
            "unknown UI scenario"
        );
        let name = options.name.as_str();
        let toast_scenario = matches!(name, "toast-motion" | "toast-motion-reduced");
        let toast_trace = toast_scenario && crate::feedback::toast_trace::enabled();
        self.prepare_scenario(options, toast_trace, window, cx)?;
        resize_scenario_window(name, toast_scenario, window);
        self.home_scenario(name)?;
        self.games_scenario(name, window, cx);
        self.wizard_scenario(name, window, cx);
        self.accounts_scenario(name, window, cx)?;
        self.providers_scenario(name, window, cx)?;
        self.settings_scenario(name, window, cx);
        if name == "about" {
            self.router.navigate(Tab::About);
        }
        self.setup_scenario(name, window, cx)?;
        self.initialize_browsers(cx);
        self.open_scenario_surface(name, window, cx);
        self.start_scenario_traces(toast_trace, window, cx);
        cx.notify();
        Ok(())
    }

    /// Resets the state every fixture shares: palette, library, accounts, version
    /// catalog and collection load states.
    fn prepare_scenario(
        &mut self,
        options: &ScenarioOptions,
        toast_trace: bool,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) -> anyhow::Result<()> {
        self.form = None;
        self.preferences.appearance = if options.dark {
            Appearance::Dark
        } else {
            Appearance::Light
        };
        self.preferences.reduce_motion = if toast_trace {
            options.name == "toast-motion-reduced"
        } else {
            !(crate::drawer::motion_trace_enabled() || crate::frame_motion::trace::enabled())
        };
        if toast_trace {
            self.motion.apply_policy(self.preferences.reduce_motion, cx);
        }
        self.preferences.onboarding_complete = true;
        self.saved_preferences = self.preferences.clone();
        self.palette = self.preferences.palette(window.appearance());
        self.snapshot = library();
        self.accounts = scenario_accounts()?;
        self.selected_profile = Some("vanilla".into());
        self.selected_instance = Some("vanilla".into());
        self.selected_account = self.accounts.first().map(account_id);
        self.versions = ["1.21.1", "1.20.6", "1.20.1", "1.16.5", "1.12.2", "24w14a"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        self.version_catalog = self
            .versions
            .iter()
            .map(|id| block_launcher::manifest::model::VersionEntry {
                id: id.clone(),
                kind: if id == "24w14a" {
                    block_launcher::manifest::model::VersionType::Snapshot
                } else {
                    block_launcher::manifest::model::VersionType::Release
                },
                url: "https://example.invalid/ui-scenario".into(),
                // Synthetic timestamps must preserve the native-support
                // boundary; empty timestamps classify every fixture as old.
                time: "2024-01-01T00:00:00+00:00".into(),
                release_time: if id == "1.12.2" {
                    "2017-09-18T11:33:40+00:00"
                } else {
                    "2024-01-01T00:00:00+00:00"
                }
                .into(),
                sha1: "0".repeat(40),
                compliance_level: 1,
            })
            .collect();
        self.version_load = CollectionLoad::Ready;
        self.shell.games.load = CollectionLoad::Ready;
        self.shell.profiles.load = CollectionLoad::Ready;
        self.shell.accounts.load = CollectionLoad::Ready;
        self.router.navigate(Tab::Home);
        Ok(())
    }

    /// Home fixtures: an empty library, an active download set, or a failed
    /// install for the selected Profile.
    fn home_scenario(&mut self, name: &str) -> anyhow::Result<()> {
        if name == "home-empty" {
            self.snapshot = Snapshot::default();
            self.accounts.clear();
            self.selected_profile = None;
            self.selected_instance = None;
            self.selected_account = None;
        }
        if matches!(
            name,
            "home-downloading" | "home-downloads-drawer" | "download-files"
        ) {
            let mut tracker = DownloadTracker::default();
            for index in 0..8 {
                let sha1 = format!("{index:040x}");
                tracker.apply(DownloadEvent::Started {
                    sha1: sha1.clone(),
                    size: 4_194_304,
                });
                tracker.apply(DownloadEvent::Bytes {
                    sha1: sha1.clone(),
                    bytes: 1_048_576,
                });
                if index < 2 {
                    tracker.apply(DownloadEvent::Complete {
                        sha1,
                        cached: index == 0,
                    });
                }
            }
            self.downloads = Some(Arc::new(Mutex::new(tracker)));
            self.busy = true;
            self.operation_kind = crate::OperationKind::Install;
            self.operation_cancellable = true;
            self.status = "Downloading Minecraft resources".into();
        }
        if name == "home-error" {
            self.status = "The download was interrupted. Check your connection and retry.".into();
            fail_selected_target(
                &mut self.snapshot,
                "vanilla",
                "The download was interrupted. Check your connection and retry.",
            )?;
        }
        Ok(())
    }

    /// Games fixtures: the library list, a detail page, and the confirmations
    /// that open over them.
    fn games_scenario(&mut self, name: &str, window: &mut Window, cx: &mut Context<'_, Self>) {
        if name.starts_with("games-")
            || matches!(
                name,
                "remove-confirmation" | "separate-confirmation" | "games-help"
            )
        {
            self.router.navigate(Tab::Instances);
            if name == "games-separated" {
                self.snapshot.profile_mode = ProfileMode::Separated;
            }
            if matches!(name, "games-detail" | "games-instance-settings") {
                self.handle(UiAction::Inspect("vanilla".into()), window, cx);
                if name == "games-instance-settings" {
                    self.handle(UiAction::EditInstance, window, cx);
                }
            }
        }
    }

    /// Wizard fixtures: the real draft, opened on Review or on one component
    /// page, with a synthetic catalog wherever the page needs one.
    fn wizard_scenario(&mut self, name: &str, window: &mut Window, cx: &mut Context<'_, Self>) {
        if !name.starts_with("wizard-") {
            return;
        }
        self.begin_game_wizard(cx);
        if name == "wizard-review" {
            self.shell.wizard.step = WizardStep::Review;
        }
        if name.starts_with("wizard-optifine") {
            self.set_wizard_version("1.21.1".into(), cx);
        }
        if let Some(page) = wizard_page(name) {
            self.handle(UiAction::WizardPage(Some(page)), window, cx);
        }
        if name == "wizard-optifine-selected"
            && let Some(super::Form::Instance(form)) = &mut self.form
        {
            form.optifine = Some(Box::new(block_launcher::install::optifine::OptiFineJar {
                minecraft: form.version.read(cx).value(),
                version: format!("{}_HD_U_J1", form.version.read(cx).value()),
                launchwrapper: "2.3".into(),
                forge: Some("52.0.16".into()),
                resource: block_launcher::Resource {
                    sha1: "a".repeat(40),
                    size: 1,
                    url: "block-import:scenario-only".into(),
                },
            }));
        }
        if LOADER_FIXTURES.contains(&name) {
            self.loader_scenario(name, cx);
        }
    }

    /// A synthetic loader catalog: real-looking builds, the catalog's load state
    /// and the build the page starts on.
    fn loader_scenario(&mut self, name: &str, cx: &mut Context<'_, Self>) {
        let (kind, version) = scenario_loader(name);
        if let Some(super::Form::Instance(form)) = &mut self.form {
            form.loader = kind.clone();
            let (builds, load, search_query) = loader_fixture_data(name, version);
            form.loader_version = (!matches!(
                name,
                "wizard-loader-loading" | "wizard-loader-error" | "wizard-loader-empty"
            ))
            .then(|| version.into());
            self.shell.wizard.loader_catalog = Some(super::game_wizard::LoaderCatalog {
                request: uuid::Uuid::new_v4(),
                minecraft: form.version.read(cx).value(),
                kind,
                builds,
                load,
            });
            if let Some(query) = search_query
                && let Some(search) = &self.shell.wizard.loader_search
            {
                search.update(cx, |field, cx| field.set_value(query, cx));
            }
        }
    }

    /// Account fixtures: the list, a detail page, and every step of the sign-in
    /// wizard reached from Accounts.
    fn accounts_scenario(
        &mut self,
        name: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> anyhow::Result<()> {
        if !(name.starts_with("accounts-")
            || name.starts_with("auth-")
            || name == "remove-account-confirmation")
        {
            return Ok(());
        }
        self.router.navigate(Tab::Accounts);
        if name == "accounts-detail"
            && let Some(id) = self.selected_account.clone()
        {
            self.handle(UiAction::Inspect(id), window, cx);
        }
        if name == "accounts-offline-detail" {
            self.handle(UiAction::Inspect(account_id(&self.accounts[2])), window, cx);
        }
        if name.starts_with("auth-") {
            if name.starts_with("auth-authlib-") {
                self.shell.providers.records = vec![
                    super::providers::fixture_provider(false)?,
                    super::providers::fixture_provider(true)?,
                ];
                self.shell.providers.loaded = true;
            }
            self.open_auth_wizard(window, cx);
            if let Some(auth) = &mut self.shell.auth {
                auth.scenario(
                    name,
                    if name.starts_with("auth-offline-") {
                        self.accounts.get(2).cloned()
                    } else {
                        self.accounts.first().cloned()
                    },
                    cx,
                );
            }
            if name == "auth-authlib-provider-add" {
                self.open_auth_provider_add(window, cx);
            }
        }
        Ok(())
    }

    /// Provider fixtures: the record list, the add/edit form, and the removal
    /// confirmation.
    fn providers_scenario(
        &mut self,
        name: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> anyhow::Result<()> {
        if !(name == "accounts-providers"
            || name.starts_with("provider-")
            || name == "remove-provider-confirmation")
        {
            return Ok(());
        }
        self.provider_scenario(name, cx)?;
        if name == "remove-provider-confirmation" {
            self.request_remove_provider(uuid::Uuid::from_u128(1).to_string(), window, cx);
        }
        Ok(())
    }

    /// Settings fixtures: one page per tab, plus the proxy and storage states.
    fn settings_scenario(&mut self, name: &str, window: &mut Window, cx: &mut Context<'_, Self>) {
        if let Some(section) = ["general", "game", "profiles", "downloads", "storage"]
            .iter()
            .position(|section| name == format!("settings-{section}"))
        {
            self.router.navigate(Tab::Settings);
            self.open_settings(cx);
            self.select_settings_page(section as u8, window, cx);
        }
        if name == "settings-storage" {
            self.shell.storage.paths = Ok(crate::settings::StoragePaths {
                library: std::path::PathBuf::from(
                    "/scenario/Application Support/one.ztd.block/.minecraft",
                ),
                cache: std::path::PathBuf::from(
                    "/scenario/Application Support/one.ztd.block/cache",
                ),
            });
        }
        if name == "settings-socks5" {
            self.saved_preferences.downloads.proxy = block_launcher::ProxyMode::Socks5 {
                url: "socks5h://127.0.0.1:1080".into(),
            };
            self.preferences.clone_from(&self.saved_preferences);
            self.router.navigate(Tab::Settings);
            self.open_settings(cx);
            self.select_settings_page(3, window, cx);
        }
    }

    /// First-run fixtures reset setup state before selecting their page.
    fn setup_scenario(
        &mut self,
        name: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> anyhow::Result<()> {
        if !name.starts_with("setup-") {
            return Ok(());
        }
        self.preferences.onboarding_complete = false;
        self.saved_preferences.onboarding_complete = false;
        self.router.navigate(Tab::Onboarding);
        self.form = None;
        self.shell.first_run = first_run::State::default();
        self.shell.first_run.enabled = true;
        self.shell.first_run.stage = match name {
            "setup-welcome" | "setup-import-sources" | "setup-import-list" => {
                FirstRunStage::Welcome
            }
            "setup-game-choice" => FirstRunStage::GameChoice,
            "setup-game" => FirstRunStage::GameWizard,
            _ => FirstRunStage::Overview,
        };
        if name == "setup-game-choice" {
            self.router.enter_game_choice();
        }
        self.shell.onboarding = onboarding::State::default();
        self.shell.import_games = import_games::State::default();
        self.shell.onboarding.account = None;
        self.selected_account = None;
        self.selected_profile = None;
        self.selected_instance = None;
        self.accounts.clear();
        self.snapshot = Snapshot::default();
        self.shell.first_run.game_parent =
            Some(std::path::PathBuf::from("/scenario/Block/instances"));

        match name {
            "setup-overview-ready" => {
                self.accounts = scenario_accounts()?;
                self.selected_account = self.accounts.first().map(account_id);
                self.shell.onboarding.account = self.selected_account.clone();
                self.snapshot = library();
                self.selected_profile = Some("vanilla".into());
                self.selected_instance = Some("vanilla".into());
            }
            "setup-import-sources" | "setup-import-list" => {
                let report = setup_import_report();
                let state = &mut self.shell.import_games;
                state.report = Some(report);
                state.selected = state
                    .report
                    .as_ref()
                    .into_iter()
                    .flat_map(|report| report.candidates.iter())
                    .filter(|candidate| candidate.importable)
                    .map(|candidate| candidate.id.clone())
                    .collect();
                if name == "setup-import-list" {
                    self.handle(
                        UiAction::ImportGames(import_games::Action::View(
                            SourceKind::OfficialLauncher,
                        )),
                        window,
                        cx,
                    );
                }
            }
            "setup-account" => {
                self.open_auth_wizard(window, cx);
                if let Some(auth) = &mut self.shell.auth {
                    auth.scenario("auth-choose", None, cx);
                }
            }
            "setup-game" => {
                self.handle(UiAction::FirstRun(first_run::Action::Custom), window, cx);
            }
            _ => {}
        }
        Ok(())
    }

    /// The overlay or drawer that opens on top of the family's page.
    fn open_scenario_surface(
        &mut self,
        name: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let surface = match name {
            "home-profile-drawer" => Some(Surface::Drawer(Drawer::Profiles)),
            "home-account-drawer" => Some(Surface::Drawer(Drawer::Accounts)),
            "home-downloads-drawer" => Some(Surface::Drawer(Drawer::Downloads)),
            "download-files" => Some(Surface::DownloadFiles(
                crate::downloads::DownloadGroup::Active,
            )),
            "remove-confirmation" => Some(Surface::DeleteProfile("vanilla".into())),
            "remove-account-confirmation" => {
                self.selected_account.clone().map(Surface::RemoveAccount)
            }
            "separate-confirmation" => Some(Surface::SeparateProfiles),
            "games-help" => Some(Surface::GamesHelp),
            "setup-import-sources" | "setup-import-list" => {
                Some(Surface::Drawer(Drawer::ImportGames))
            }
            _ => None,
        };
        let Some(surface) = surface else {
            return;
        };
        if let Surface::Drawer(drawer @ (Drawer::Profiles | Drawer::Accounts)) = surface {
            self.handle(UiAction::Drawer(Some(drawer)), window, cx);
        } else {
            self.shell.surfaces.open(surface, window, cx);
        }
    }

    /// Starts the opt-in motion traces; each records into its own file.
    fn start_scenario_traces(
        &mut self,
        toast_trace: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.motion_dirty = true;
        if crate::drawer::motion_trace_enabled() {
            crate::drawer::start_motion_trace(cx.weak_entity(), window, cx);
        }
        if crate::frame_motion::trace::enabled() {
            crate::frame_motion::trace::start(cx.weak_entity(), window, cx);
        }
        if toast_trace {
            crate::feedback::toast_trace::start(
                window,
                cx,
                if self.preferences.reduce_motion {
                    "toast-motion-reduced"
                } else {
                    "toast-motion"
                },
                self.preferences.reduce_motion,
            );
        }
    }
}

/// Home-sized fixtures keep the window a Home visit uses; every other page opens
/// at the management size.
fn resize_scenario_window(name: &str, toast_scenario: bool, window: &mut Window) {
    let home = name.starts_with("home-")
        || name == "download-files"
        || toast_scenario
        || matches!(
            name,
            "setup-welcome" | "setup-import-sources" | "setup-import-list"
        );
    window.resize(if home {
        crate::window_geometry::home_size(window)
    } else {
        crate::window_geometry::management_size(window)
    });
}

fn fail_selected_target(
    snapshot: &mut Snapshot,
    profile_id: &str,
    message: &str,
) -> anyhow::Result<()> {
    let profile = snapshot
        .profiles
        .iter()
        .find(|profile| profile.id == profile_id)
        .ok_or_else(|| anyhow::anyhow!("scenario profile is missing: {profile_id}"))?;
    let (instance_id, target_id) = (profile.instance_id.clone(), profile.target_id.clone());
    let target = snapshot
        .instances
        .iter_mut()
        .find(|instance| instance.id == instance_id)
        .and_then(|instance| {
            instance
                .targets
                .iter_mut()
                .find(|target| target.id == target_id)
        })
        .ok_or_else(|| anyhow::anyhow!("scenario launch target is missing: {profile_id}"))?;
    target.status = InstallStatus::Failed {
        message: message.into(),
    };
    Ok(())
}

fn loader_fixture_data(
    name: &str,
    version: &str,
) -> (
    Vec<block_launcher::install::fabric::LoaderVersion>,
    CollectionLoad,
    Option<&'static str>,
) {
    let no_builds = matches!(
        name,
        "wizard-loader-loading" | "wizard-loader-error" | "wizard-loader-empty"
    );
    let builds = if no_builds {
        Vec::new()
    } else {
        vec![
            block_launcher::install::fabric::LoaderVersion {
                version: version.into(),
                stable: true,
            },
            block_launcher::install::fabric::LoaderVersion {
                version: if name == "wizard-forge-picker" {
                    "1.21.1-52.1.2".into()
                } else if name == "wizard-neoforge-picker" {
                    "21.1.1-beta".into()
                } else {
                    "0.16.0-beta.1".into()
                },
                stable: name == "wizard-forge-picker",
            },
        ]
    };
    let load = match name {
        "wizard-loader-loading" => CollectionLoad::Loading,
        "wizard-loader-error" => CollectionLoad::Failed(
            "The official catalog could not be reached. Check your connection and retry.".into(),
        ),
        _ => CollectionLoad::Ready,
    };
    let search_query = (name == "wizard-loader-search-empty").then_some("no-matching-loader-build");
    (builds, load, search_query)
}

/// The loader a picker fixture shows, with the build its catalog is pinned to.
fn scenario_loader(name: &str) -> (LoaderKind, &'static str) {
    match name {
        "wizard-forge-picker" => (LoaderKind::Forge, "1.21.1-52.1.3"),
        "wizard-quilt-picker" => (LoaderKind::Quilt, "0.29.2"),
        "wizard-neoforge-picker" => (LoaderKind::NeoForge, "21.1.219"),
        _ => (LoaderKind::Fabric, "0.16.14"),
    }
}

/// The component page a wizard fixture opens.
fn wizard_page(name: &str) -> Option<WizardPage> {
    match name {
        "wizard-optifine" | "wizard-optifine-selected" => Some(WizardPage::OptiFine),
        "wizard-version-picker" => Some(WizardPage::Version),
        "wizard-loader-picker"
        | "wizard-fabric-picker"
        | "wizard-quilt-picker"
        | "wizard-neoforge-picker"
        | "wizard-forge-picker"
        | "wizard-loader-loading"
        | "wizard-loader-error"
        | "wizard-loader-empty"
        | "wizard-loader-search-empty" => Some(WizardPage::Loader),
        "wizard-configuration" => Some(WizardPage::Location),
        _ => None,
    }
}

fn library() -> Snapshot {
    let mut snapshot = Snapshot {
        profile_mode: ProfileMode::Bundled,
        ..Snapshot::default()
    };
    for (index, (id, name, version, kind)) in [
        ("vanilla", "Vanilla", "1.20.1", LoaderKind::Vanilla),
        (
            "neoforge",
            "NeoForge Worlds",
            "1.21.1",
            LoaderKind::NeoForge,
        ),
        ("snapshot", "Snapshot Lab", "24w14a", LoaderKind::Vanilla),
    ]
    .into_iter()
    .enumerate()
    {
        snapshot.instances.push(Instance {
            id: id.into(),
            name: name.into(),
            minecraft_version: version.into(),
            directory: std::env::temp_dir().join("block-ui-scenario-only").join(id),
            targets: vec![LaunchTarget {
                id: "default".into(),
                loader_version: (kind == LoaderKind::NeoForge).then(|| "21.1.80".into()),
                kind,
                manifest: None,
                status: InstallStatus::Ready,
                optifine: None,
                optifine_mod: None,
            }],
            settings: None,
        });
        snapshot.profiles.push(Profile {
            id: id.into(),
            name: name.into(),
            instance_id: id.into(),
            target_id: "default".into(),
            runtime: RuntimeSelection::Managed,
            memory_mib: 2048,
            jvm_args: Vec::new(),
            game_args: Vec::new(),
            data_policy: DataPolicy::Isolated,
            default_account: None,
            compatibility: Compatibility::Official,
            appearance: block_launcher::catalog::ProfileAppearance {
                icon: [
                    ProfileIcon::Blocks,
                    ProfileIcon::Sword,
                    ProfileIcon::FlaskConical,
                ][index],
                color: [
                    ProfileColor::Gray,
                    ProfileColor::Purple,
                    ProfileColor::Orange,
                ][index],
            },
            inheritance: Default::default(),
            server: Default::default(),
        });
    }
    snapshot
}

fn setup_import_report() -> DiscoveryReport {
    let adjacent_root = std::path::PathBuf::from("/scenario/.minecraft");
    let launcher_root = std::path::PathBuf::from("/scenario/Minecraft Launcher");
    let mut candidates = vec![setup_candidate(
        "adjacent-scenario",
        SourceKind::AdjacentMinecraft,
        &adjacent_root,
        "Adjacent Vanilla",
        "1.20.6",
        block_launcher::game_import::LoaderKind::Vanilla,
    )];
    for (id, name, version, loader) in [
        (
            "launcher-vanilla",
            "Launcher Vanilla",
            "1.21.1",
            block_launcher::game_import::LoaderKind::Vanilla,
        ),
        (
            "launcher-fabric",
            "Fabric Builder",
            "1.21.1",
            block_launcher::game_import::LoaderKind::Fabric,
        ),
        (
            "launcher-forge",
            "Forge Workshop",
            "1.20.1",
            block_launcher::game_import::LoaderKind::Forge,
        ),
        (
            "launcher-quilt",
            "Quilt Garden",
            "1.19.4",
            block_launcher::game_import::LoaderKind::Quilt,
        ),
    ] {
        candidates.push(setup_candidate(
            id,
            SourceKind::OfficialLauncher,
            &launcher_root,
            name,
            version,
            loader,
        ));
    }
    DiscoveryReport {
        sources: vec![
            ImportSourceReport {
                kind: SourceKind::AdjacentMinecraft,
                root: adjacent_root,
                candidates_found: 1,
                warning: None,
            },
            ImportSourceReport {
                kind: SourceKind::OfficialLauncher,
                root: launcher_root,
                candidates_found: 4,
                warning: None,
            },
        ],
        candidates,
    }
}

fn setup_candidate(
    id: &str,
    source: SourceKind,
    source_root: &std::path::Path,
    name: &str,
    version: &str,
    loader: block_launcher::game_import::LoaderKind,
) -> GameCandidate {
    GameCandidate {
        id: id.into(),
        source,
        source_root: source_root.to_path_buf(),
        name: name.into(),
        game_directory: source_root.join("instances").join(id),
        configured_version: Some(version.into()),
        minecraft_version: Some(version.into()),
        loader: Some(DetectedLoader {
            kind: loader,
            version: (loader != block_launcher::game_import::LoaderKind::Vanilla)
                .then(|| "0.16.14".into()),
        }),
        latest_release: Some("1.21.1".into()),
        latest_snapshot: Some("24w14a".into()),
        importable: true,
        unsupported_reason: None,
        metadata_warning: None,
    }
}

fn scenario_accounts() -> anyhow::Result<Vec<Account>> {
    let mut accounts = ["Zeithrold", "Nova", "Pixel"]
        .into_iter()
        .enumerate()
        .map(|(index, name)| {
            Ok(Account {
                reference: AccountRef {
                    player_id: format!("00000000-0000-4000-8000-{:012}", index + 1).parse()?,
                },
                provider: AccountProvider::Microsoft,
                player_name: name.into(),
                client_id: "ui-scenario-public-client".into(),
                credential_ref: "ui-scenario-no-credentials".into(),
                authlib: None,
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    accounts[2] = crate::engine::offline_account("Pixel")?;
    Ok(accounts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_error_fixture_fails_the_selected_profile_target_only() -> anyhow::Result<()> {
        let mut snapshot = library();

        fail_selected_target(
            &mut snapshot,
            "vanilla",
            "The download was interrupted. Check your connection and retry.",
        )?;

        let (_, _, target) = snapshot.selection("vanilla")?;
        assert!(matches!(
            &target.status,
            InstallStatus::Failed { message } if message.contains("download was interrupted")
        ));
        let (_, _, untouched) = snapshot.selection("neoforge")?;
        assert_eq!(untouched.status, InstallStatus::Ready);
        Ok(())
    }

    #[test]
    fn loader_fixtures_keep_empty_catalog_separate_from_search_empty() -> anyhow::Result<()> {
        let (empty_builds, empty_load, empty_search) =
            loader_fixture_data("wizard-loader-empty", "0.16.14");
        let (search_builds, search_load, search_query) =
            loader_fixture_data("wizard-loader-search-empty", "0.16.14");
        let (loading_builds, loading_load, _) =
            loader_fixture_data("wizard-loader-loading", "0.16.14");
        let (failed_builds, failed_load, _) = loader_fixture_data("wizard-loader-error", "0.16.14");

        assert!(empty_builds.is_empty());
        assert_eq!(empty_load, CollectionLoad::Ready);
        assert_eq!(empty_search, None);
        assert_eq!(search_builds.len(), 2);
        assert_eq!(search_load, CollectionLoad::Ready);
        assert_eq!(search_query, Some("no-matching-loader-build"));
        let search_query = search_query.ok_or_else(|| anyhow::anyhow!("missing search query"))?;
        assert!(
            search_builds
                .iter()
                .all(|build| { !build.version.to_lowercase().contains(search_query) })
        );
        assert!(loading_builds.is_empty());
        assert_eq!(loading_load, CollectionLoad::Loading);
        assert!(failed_builds.is_empty());
        assert!(matches!(failed_load, CollectionLoad::Failed(_)));
        Ok(())
    }
}
