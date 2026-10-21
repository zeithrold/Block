//! Debug-only, network-free UI states. These fixtures never authenticate or install games.

use std::sync::{Arc, Mutex};

use block_auth::{Account, AccountProvider, AccountRef};
use block_launcher::{
    DownloadEvent,
    catalog::{
        DataPolicy, InstallStatus, Instance, LaunchTarget, LoaderKind, Profile, ProfileColor,
        ProfileIcon, ProfileMode, Snapshot,
    },
    recipe::{Compatibility, RuntimeSelection},
};
use gpui::{Context, Window};

use crate::{
    Block, Tab, UiAction, account_id,
    appearance::Appearance,
    browser::CollectionLoad,
    game_wizard::{WizardPage, WizardStep},
    onboarding,
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
    "settings-storage",
    "about",
    "welcome-1",
    "welcome-2",
    "welcome-3",
    "home-profile-drawer",
    "home-account-drawer",
    "home-downloads-drawer",
    "download-files",
    "wizard-version-picker",
    "wizard-loader-picker",
    "wizard-configuration",
    "welcome-version-picker",
    "remove-confirmation",
    "remove-account-confirmation",
    "separate-confirmation",
    "games-help",
    "welcome-settings",
    "welcome-about",
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
        self.form = None;
        self.preferences.appearance = if options.dark {
            Appearance::Dark
        } else {
            Appearance::Light
        };
        self.preferences.reduce_motion = true;
        self.preferences.onboarding_complete = true;
        self.saved_preferences = self.preferences.clone();
        self.palette = self.preferences.palette(window.appearance());
        self.snapshot = library();
        self.accounts = ["Zeithrold", "Nova", "Pixel"]
            .into_iter()
            .enumerate()
            .map(|(i, name)| {
                Ok(Account {
                    reference: AccountRef {
                        player_id: format!("00000000-0000-4000-8000-{:012}", i + 1).parse()?,
                    },
                    provider: AccountProvider::Microsoft,
                    player_name: name.into(),
                    client_id: "ui-scenario-public-client".into(),
                    credential_ref: "ui-scenario-no-credentials".into(),
                    authlib: None,
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        self.accounts[2] = crate::engine::offline_account("Pixel")?;
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
            .map(|id| minecraft_manifest::model::VersionEntry {
                id: id.clone(),
                kind: if id == "24w14a" {
                    minecraft_manifest::model::VersionType::Snapshot
                } else {
                    minecraft_manifest::model::VersionType::Release
                },
                url: "https://example.invalid/ui-scenario".into(),
                time: String::new(),
                release_time: String::new(),
                sha1: "0".repeat(40),
                compliance_level: 1,
            })
            .collect();
        self.version_load = CollectionLoad::Ready;
        self.shell.games.load = CollectionLoad::Ready;
        self.shell.profiles.load = CollectionLoad::Ready;
        self.shell.accounts.load = CollectionLoad::Ready;
        self.tab = Tab::Home;
        let name = options.name.as_str();
        let home = name.starts_with("home-") || name == "download-files";
        window.resize(if home {
            crate::window_geometry::home_size(window, cx)
        } else {
            crate::window_geometry::management_size(window, cx)
        });
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
        }
        if name.starts_with("games-")
            || matches!(
                name,
                "remove-confirmation" | "separate-confirmation" | "games-help"
            )
        {
            self.tab = Tab::Instances;
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
        if name.starts_with("wizard-") {
            self.begin_game_wizard(cx);
            if name == "wizard-review" {
                self.shell.wizard.step = WizardStep::Review;
            }
            let page = match name {
                "wizard-version-picker" => Some(WizardPage::Version),
                "wizard-loader-picker" => Some(WizardPage::Loader),
                "wizard-configuration" => Some(WizardPage::Location),
                _ => None,
            };
            if let Some(page) = page {
                self.handle(UiAction::WizardPage(Some(page)), window, cx);
            }
        }
        if name.starts_with("accounts-")
            || name.starts_with("auth-")
            || name == "remove-account-confirmation"
        {
            self.tab = Tab::Accounts;
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
                    let owner = cx.entity();
                    cx.defer(move |cx| {
                        let handle = owner
                            .read(cx)
                            .shell
                            .surfaces
                            .layers()
                            .find(|entry| matches!(entry.kind, Surface::Auth))
                            .and_then(|entry| entry.window);
                        if let Some(handle) = handle {
                            let _ = handle.update(cx, |_, window, cx| {
                                owner.update(cx, |view, cx| view.open_auth_provider_add(window, cx))
                            });
                        }
                    });
                }
            }
        }
        if name == "accounts-providers"
            || name.starts_with("provider-")
            || name == "remove-provider-confirmation"
        {
            self.provider_scenario(name, cx)?;
            if name == "remove-provider-confirmation" {
                self.request_remove_provider(uuid::Uuid::from_u128(1).to_string(), window, cx);
            }
        }
        if let Some(section) = ["general", "game", "profiles", "downloads", "storage"]
            .iter()
            .position(|section| name == format!("settings-{section}"))
        {
            self.tab = Tab::Settings;
            self.open_settings(cx);
            self.select_settings_page(section as u8, window, cx);
        }
        if name == "about" {
            self.tab = Tab::About;
        }
        if name.starts_with("welcome-") {
            self.begin_onboarding(cx);
            self.shell.onboarding.step = match name {
                "welcome-2" => onboarding::Step::Account,
                "welcome-3" => onboarding::Step::Review,
                _ => onboarding::Step::Game,
            };
            self.shell.onboarding.account = self.selected_account.clone();
            match name {
                "welcome-version-picker" => {
                    self.handle(UiAction::OnboardingSection(WizardPage::Version), window, cx)
                }
                "welcome-settings" => self.handle(UiAction::Tab(Tab::Settings), window, cx),
                "welcome-about" => self.handle(UiAction::Tab(Tab::About), window, cx),
                _ => (),
            }
        }
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
            _ => None,
        };
        if let Some(surface) = surface {
            if let Surface::Drawer(drawer @ (Drawer::Profiles | Drawer::Accounts)) = surface {
                self.handle(UiAction::Drawer(Some(drawer)), window, cx);
            } else {
                self.shell.surfaces.open(surface, window, cx);
            }
        }
        self.motion_dirty = true;
        cx.notify();
        Ok(())
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
