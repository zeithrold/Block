#![expect(
    clippy::redundant_pub_crate,
    reason = "Keep private UI modules scoped to their parent."
)]
//! Native product shell, keeping page, detail and drawer animation lifetimes independent.
use std::{
    cell::RefCell,
    sync::{Arc, LazyLock},
};

use gpui::{
    AnyElement, Context, Image, ImageFormat, InteractiveElement as _, IntoElement,
    ParentElement as _, StatefulInteractiveElement as _, Styled as _, Window, div, img, px, rgb,
};

use super::{Block, Form, Tab, UiAction, account_id, note, scroll_area::ScrollAreaExt as _};

static BRAND: LazyLock<Arc<Image>> = LazyLock::new(|| {
    Arc::new(Image::from_bytes(
        ImageFormat::Png,
        include_bytes!("../assets/block.png").to_vec(),
    ))
});

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Drawer {
    Menu,
    Accounts,
    Profiles,
    Downloads,
}

#[derive(Clone, Debug)]
pub(super) enum Surface {
    Drawer(Drawer),
    DownloadFiles(super::downloads::DownloadGroup),
    DeleteProfile(String),
    RemoveAccount(String),
    RemoveProvider(String),
    SeparateProfiles,
    StorageCleanup(uuid::Uuid),
    GamesHelp,
    OnboardingSection(super::WizardPage),
    WizardSection(super::WizardPage),
    AuthProviderAdd,
    Select(super::select::SelectState<UiAction>),
}

/// The library overlay that presents a surface inside the product window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum OverlayPresentation {
    Dialog,
    Sheet,
}

#[derive(Debug, Default)]
pub(super) struct ShellState {
    pub surfaces: super::overlays::Overlays,
    pub connection: super::connection_test::State,
    pub storage: super::settings::StorageState,
    pub downloads: super::downloads::State,
    pub home_scroll: gpui::ScrollHandle,
    pub home: RefCell<HomeState>,
    pub about_scroll: gpui::ScrollHandle,
    pub help_scroll: gpui::ScrollHandle,
    pub output_scroll: gpui::ScrollHandle,
    pub toast: Option<super::feedback::Toast>,
    pub auth: Option<super::authentication::AuthWizard>,
    pub editor: super::profile_editor::EditorState,
    pub instance_editor: Option<super::instance_editor::InstanceEditor>,
    pub games: super::browser::CollectionState,
    pub profiles: super::browser::CollectionState,
    pub accounts: super::browser::CollectionState,
    pub providers: super::providers::State,
    pub selector: Option<super::search_selector::SearchSelectorState>,
    /// Settings/About preview opened from the Welcome menu, with the size to
    /// restore when the setup draft comes back.
    pub guide_preview: Option<(Tab, gpui::Size<gpui::Pixels>)>,
    pub revision: usize,
    pub backwards: bool,
    pub settings_page: u8,
    pub wizard: super::game_wizard::WizardState,
    pub onboarding: super::onboarding::State,
    pub system_window_chrome: bool,
}

/// Retain only the last truthful panel presentation through its local exit.
/// Changing text or byte counts never changes the disclosure's open identity.
#[derive(Clone, Debug, Default)]
pub(super) struct HomeState {
    disclosure: super::motion::Disclosure,
    panel: Option<HomePanel>,
    pub(super) avatar_hover: super::motion::HoverDecoration,
}

#[derive(Clone, Debug)]
struct HomePanel {
    phase: &'static str,
    status: String,
    resource: Option<(u64, Option<u64>)>,
    verified: usize,
}

impl HomeState {
    fn reconcile(
        &mut self,
        panel: Option<HomePanel>,
        now: std::time::Instant,
        preferences: super::motion::Preferences,
    ) {
        self.disclosure.set_open(panel.is_some(), now, preferences);
        if panel.is_some() {
            self.panel = panel;
        }
    }
}

impl ShellState {
    #[cfg(test)]
    pub(super) fn drawer(&self) -> Option<Drawer> {
        self.surfaces.top().and_then(|layer| match layer.kind {
            Surface::Drawer(drawer) => Some(drawer),
            Surface::Select(ref state) if state.menu == Some(super::select::MenuKind::App) => {
                Some(Drawer::Menu)
            }
            Surface::DownloadFiles(_)
            | Surface::DeleteProfile(_)
            | Surface::RemoveAccount(_)
            | Surface::RemoveProvider(_)
            | Surface::SeparateProfiles
            | Surface::StorageCleanup(_)
            | Surface::GamesHelp
            | Surface::OnboardingSection(_)
            | Surface::WizardSection(_)
            | Surface::AuthProviderAdd
            | Surface::Select(_) => None,
        })
    }
}

pub(super) fn brand(size: f32) -> impl IntoElement {
    img(BRAND.clone()).size(px(size))
}

impl Block {
    pub(super) fn render_shell(
        &self,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement + use<> {
        let home = self.tab == Tab::Home && self.form.is_none();
        let content = self.transition_content(
            super::motion::Scope::Page,
            self.render_page_body(window, cx),
            window,
            cx,
        );
        let mut root = div()
            .id("block-shell")
            .debug_selector(|| "block-shell".into())
            .key_context("LauncherForm")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|view, event: &gpui::KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    // Base Dialog owns Escape while a surface is mounted.
                    if !view.shell.surfaces.is_open() {
                        if view.shell.auth.is_some() {
                            view.handle(UiAction::AuthCancel, window, cx);
                            cx.stop_propagation();
                        } else if view.shell.guide_preview.is_some() {
                            view.leave_guide_preview(window, cx);
                            cx.stop_propagation();
                        } else if view.shell.instance_editor.is_some() {
                            view.handle(UiAction::CancelInstanceSettings, window, cx);
                            cx.stop_propagation();
                        } else if view.form.is_some() {
                            view.handle(UiAction::CancelForm, window, cx);
                            cx.stop_propagation();
                        }
                    }
                }
            }))
            .on_action(cx.listener(|view, _: &super::NextField, window, cx| {
                view.move_focus(false, window, cx);
            }))
            .on_action(cx.listener(|view, _: &super::PreviousField, window, cx| {
                view.move_focus(true, window, cx);
            }))
            .size_full()
            .relative()
            .overflow_hidden()
            .flex()
            .flex_col()
            .bg(rgb(self.palette.background))
            .text_color(rgb(self.palette.foreground))
            .text_size(px(14.))
            .font_family(super::design::FONT_FAMILY)
            .child(div().flex_shrink_0().child(self.chrome(window, home, cx)))
            .child(div().flex_1().min_h_0().child(content));
        root = root.children(self.render_popups(window, cx));
        if let Some(toast) = &self.shell.toast {
            root = root.child(self.render_toast(toast, window, cx));
        }
        root
    }

    fn render_toast(
        &self,
        toast: &super::feedback::Toast,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement + use<> {
        let (icon, color) = match toast.kind {
            super::feedback::ToastKind::Success => {
                (super::design::Icon::Check, self.palette.success)
            }
            super::feedback::ToastKind::Error => {
                (super::design::Icon::Error, self.palette.destructive)
            }
            super::feedback::ToastKind::Info => {
                (super::design::Icon::Info, self.palette.foreground)
            }
        };
        let rail = if self.tab == Tab::Home && self.form.is_none() {
            104.
        } else if super::patterns::is_narrow(window.viewport_size()) {
            super::design::NARROW_FOOTER
        } else {
            super::design::MANAGEMENT_FOOTER
        };
        div()
            .absolute()
            .bottom(px(rail + 12.))
            .left_0()
            .right_0()
            .flex()
            .justify_center()
            .px_4()
            .child(
                div()
                    .id("toast")
                    .debug_selector(|| "toast".into())
                    .occlude()
                    .max_w(px(560.))
                    .p_3()
                    .rounded_xl()
                    .border_1()
                    .border_color(rgb(self.palette.border))
                    .bg(rgb(self.palette.popover))
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(icon.render(18., color))
                    .child(div().flex_1().text_sm().child(toast.message.clone()))
                    .child(
                        self.button("toast-dismiss", "", UiAction::DismissToast, cx)
                            .child(super::design::Icon::Close.render(14., self.palette.muted)),
                    ),
            )
    }

    pub(super) fn navigation_title(&self) -> &'static str {
        if self.shell.auth.is_some() {
            return "Accounts";
        }
        if self.shell.guide_preview.is_some() {
            return match self.tab {
                Tab::Settings => "Settings",
                Tab::About => "About Block",
                _ => "Block",
            };
        }
        if self.tab == Tab::Accounts && self.shell.providers.visible {
            return "Authlib providers";
        }
        if self.shell.instance_editor.is_some() {
            return "Instance settings";
        }
        match &self.form {
            Some(Form::Profile(_)) => match self.shell.editor.page {
                Some(super::profile_editor::EditorPage::Appearance) => "Profile appearance",
                Some(super::profile_editor::EditorPage::Data) => "Data and account",
                Some(super::profile_editor::EditorPage::Launch) | None => "Launch settings",
            },
            Some(Form::Instance(_)) if self.tab != Tab::Onboarding => "Create a game",
            _ => match self.tab {
                Tab::Home => "Block",
                Tab::Onboarding => "Welcome to Block",
                Tab::Instances | Tab::Profiles => "Games",
                Tab::Accounts => "Accounts",
                Tab::Settings => "Settings",
                Tab::About => "About Block",
            },
        }
    }

    fn contextual_back(&self) -> (&'static str, UiAction) {
        if self.shell.auth.is_some() {
            return ("Back", UiAction::AuthCancel);
        }
        if self.shell.guide_preview.is_some() {
            return ("Back to setup", UiAction::Drawer(None));
        }
        if self.tab == Tab::Accounts && self.shell.providers.visible {
            return ("Accounts", UiAction::ProvidersBack);
        }
        if self.shell.instance_editor.is_some() {
            return (
                if self.snapshot.profile_mode == block_launcher::catalog::ProfileMode::Bundled {
                    "Back to game"
                } else {
                    "Back to Instance"
                },
                UiAction::CancelInstanceSettings,
            );
        }
        if matches!(self.form, Some(Form::Profile(_))) {
            return if self.shell.editor.page.is_some() {
                ("Back to launch settings", UiAction::ProfilePageCancel)
            } else {
                (
                    if self.snapshot.profile_mode == block_launcher::catalog::ProfileMode::Bundled {
                        "Back to game"
                    } else {
                        "Back to Profile"
                    },
                    UiAction::CancelForm,
                )
            };
        }
        if matches!(self.form, Some(Form::Instance(_))) {
            return (
                if self.shell.wizard.page.is_some() {
                    "Back to setup"
                } else {
                    "Back to games"
                },
                UiAction::CancelForm,
            );
        }
        ("Home", UiAction::Tab(Tab::Home))
    }

    fn chrome(
        &self,
        window: &Window,
        home: bool,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement + use<> {
        let authenticating = false;
        let first_run = self.tab == Tab::Onboarding;
        let narrow = super::patterns::is_narrow(window.viewport_size());
        let title = if home { "" } else { self.navigation_title() };
        div()
            .id("window-chrome")
            .h(px(if home {
                52.
            } else if narrow {
                40.
            } else {
                56.
            }))
            .border_b_1()
            .border_color(rgb(self.palette.border))
            .flex_shrink_0()
            .px_4()
            .pl(px(
                if cfg!(target_os = "macos") && self.shell.system_window_chrome {
                    80.
                } else {
                    16.
                },
            ))
            .flex()
            .items_center()
            .gap_2()
            .child(if authenticating {
                div().text_sm().child("Block").into_any_element()
            } else {
                self.button("app-menu", "", UiAction::Drawer(Some(Drawer::Menu)), cx)
                    .child(super::design::Icon::Menu.render(18., self.palette.muted))
                    .into_any_element()
            })
            .child(
                div()
                    .id("window-title")
                    .flex_1()
                    .h_full()
                    .flex()
                    .items_center()
                    .min_w_0()
                    .child(div().truncate().text_sm().child(title)),
            )
            .children((!first_run && !authenticating).then(|| {
                if home {
                    div()
                        .children((self.busy && self.downloads.is_some()).then(|| {
                            super::tooltip::trigger(
                                self.button(
                                    "downloads",
                                    "",
                                    UiAction::Drawer(Some(Drawer::Downloads)),
                                    cx,
                                )
                                .child(
                                    super::components::progress_ring(
                                        super::components::ProgressState::Indeterminate,
                                        self.palette,
                                    ),
                                ),
                                "View downloads",
                                cx,
                            )
                        }))
                        .into_any_element()
                } else {
                    let (label, action) = self.contextual_back();
                    self.button("back-home", label, action, cx)
                        .child(super::design::Icon::ArrowLeft.render(16., self.palette.muted))
                        .into_any_element()
                }
            }))
    }

    pub(super) fn render_page_body(&self, window: &Window, cx: &Context<'_, Self>) -> AnyElement {
        if self.tab == Tab::Onboarding {
            return self.onboarding(window, cx);
        }
        if self.tab == Tab::Accounts && self.shell.auth.is_some() {
            return self.render_auth_wizard(window, cx).into_any_element();
        }
        // Welcome's Settings/About preview keeps the setup draft aside and
        // shows the real management page, not the wizard form.
        if let Some((tab, _)) = self.shell.guide_preview {
            return if tab == Tab::Settings {
                self.render_settings(window, cx).into_any_element()
            } else {
                self.render_about(window, cx).into_any_element()
            };
        }
        if let Some(Form::Instance(form)) = &self.form {
            return self.render_game_wizard(form, window, cx).into_any_element();
        }
        if self.tab == Tab::Home {
            self.home(window, cx).into_any_element()
        } else {
            self.management(window, cx).into_any_element()
        }
    }

    fn home(&self, window: &Window, cx: &Context<'_, Self>) -> impl IntoElement + use<> {
        let account = self
            .accounts
            .iter()
            .find(|account| Some(account_id(account)) == self.selected_account);

        let mut content = div()
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .px_6()
            .pb_6()
            .gap_0()
            .child(self.home_identity(account, window, cx))
            .children(
                (!matches!(
                    self.status.as_str(),
                    "Welcome to Block" | "UI scenario fixture"
                ))
                .then(|| {
                    div()
                        .id("home-status")
                        .mb_3()
                        .text_xs()
                        .text_color(rgb(self.palette.muted))
                        .child(self.status.clone())
                }),
            );
        content = content.child(self.home_task(window, cx));
        if self.busy {
            return content.into_any_element();
        }
        let profile = self.profile();
        let (label, action) =
            match profile.and_then(|profile| self.snapshot.selection(&profile.id).ok()) {
                Some((_, _, target)) => match &target.status {
                    block_launcher::catalog::InstallStatus::Ready if account.is_none() => {
                        ("Sign in to play", UiAction::SignIn)
                    }
                    block_launcher::catalog::InstallStatus::Ready => ("Launch", UiAction::Play),
                    block_launcher::catalog::InstallStatus::Failed { .. } => {
                        ("Retry download", UiAction::Repair)
                    }
                    block_launcher::catalog::InstallStatus::Pending
                    | block_launcher::catalog::InstallStatus::Installing => {
                        ("Download", UiAction::Repair)
                    }
                },
                None if profile.is_some() => ("Repair game", UiAction::Repair),
                None => ("Create game", UiAction::CreateInstance),
            };
        if account.is_none() && matches!(action, UiAction::SignIn | UiAction::CreateInstance) {
            return content.into_any_element();
        }
        let description = profile
            .map_or("Start a new adventure", |profile| profile.name.as_str())
            .to_owned();
        let icon = match &action {
            UiAction::Play => super::design::Icon::Play,
            UiAction::SignIn => super::design::Icon::Account,
            UiAction::CreateInstance => super::design::Icon::Plus,
            _ => super::design::Icon::Download,
        };
        let primary = self
            .button("home-play", "", action, cx)
            .w_full()
            .min_w_0()
            .h(px(80.))
            .rounded_xl()
            .px_5()
            .items_center()
            .justify_center()
            .text_xl()
            .bg(rgb(self.palette.primary))
            .text_color(rgb(self.palette.on_primary))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .relative()
                    .left(px(-6.))
                    .child(icon.render(20., self.palette.on_primary))
                    .child(
                        div()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .items_start()
                            .gap_1()
                            .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child(label))
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::NORMAL)
                                    .text_color(rgb(self.palette.on_primary).opacity(0.65))
                                    .truncate()
                                    .child(description.clone()),
                            ),
                    ),
            );
        let tooltip = self.home_version().unwrap_or(description);
        content = content.child(
            div()
                .relative()
                .h(px(80.))
                .flex_shrink_0()
                .child(super::tooltip::trigger(primary, tooltip, cx))
                .child(
                    self.button(
                        "home-profiles",
                        "",
                        UiAction::Drawer(Some(Drawer::Profiles)),
                        cx,
                    )
                    .absolute()
                    .right_0()
                    .top_0()
                    .w(px(56.))
                    .h_full()
                    .rounded_xl()
                    .border_0()
                    .child(
                        div()
                            .absolute()
                            .left_0()
                            .top(px(24.))
                            .bottom(px(24.))
                            .w(px(1.))
                            .bg(rgb(self.palette.on_primary))
                            .opacity(0.2),
                    )
                    .child(super::design::Icon::ChevronDown.render(20., self.palette.on_primary)),
                ),
        );
        content.into_any_element()
    }

    fn home_identity(
        &self,
        account: Option<&block_auth::Account>,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> super::scroll_area::ScrollArea<gpui::Stateful<gpui::Div>> {
        let name = account.map_or("Welcome to Block", |account| account.player_name.as_str());
        let avatar = account.and_then(|account| self.account_avatars.get(&account_id(account)));
        let has_account = account.is_some();
        let mut identity = div()
            .id(if account.is_some() {
                "home-player"
            } else {
                "home-empty"
            })
            .debug_selector(move || {
                if has_account {
                    "home-player".into()
                } else {
                    "home-empty".into()
                }
            })
            .flex_1()
            .min_h_0()
            .scroll_area(&self.shell.home_scroll, "Player", self.palette, false)
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_6()
            .text_center();
        if let Some(account) = account {
            let provider = self.account_authority(account);
            let palette = self.palette;

            identity = identity
                .child(
                    div()
                        .max_w_full()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .text_2xl()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .max_w_full()
                                .truncate()
                                .child(name.to_owned()),
                        )
                        .child(
                            div()
                                .id("home-account-type")
                                .debug_selector(|| "home-account-type".into())
                                .child(
                                    super::components::badge(
                                        provider,
                                        palette,
                                        super::components::BadgeVariant::Secondary,
                                    )
                                    .h(px(24.))
                                    .border_0()
                                    .bg(rgb(
                                        if palette.background == super::Palette::DARK.background {
                                            0x172336
                                        } else {
                                            0xebf2fb
                                        },
                                    ))
                                    .text_color(rgb(
                                        if palette.background == super::Palette::DARK.background {
                                            0x93baf1
                                        } else {
                                            0x526e8e
                                        },
                                    )),
                                ),
                        ),
                )
                .child(self.home_avatar(name, avatar, window, cx));
        } else {
            identity = identity
                .child(super::design::Icon::Account.render(40., self.palette.muted))
                .child(div().text_2xl().child(name.to_owned()))
                .child(note("Add an account to start your next adventure."))
                .child(
                    self.button("home-sign-in", "Add account", UiAction::SignIn, cx)
                        .bg(rgb(self.palette.primary))
                        .text_color(rgb(self.palette.on_primary)),
                );
        }
        identity
    }

    fn home_avatar(
        &self,
        name: &str,
        avatar: Option<&super::components::AccountAvatar>,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement + use<> {
        let palette = self.palette;
        let avatar_size = ((f32::from(window.viewport_size().width) - 48.) * 0.45).clamp(96., 176.);
        let (hover, hover_running) = self
            .shell
            .home
            .borrow()
            .avatar_hover
            .sample(cx.background_executor().now(), self.motion_preferences());
        if hover_running {
            window.request_animation_frame();
        }
        let avatar = self
            .button(
                "account-avatar",
                "",
                UiAction::Drawer(Some(Drawer::Accounts)),
                cx,
            )
            .h_auto()
            .rounded_xl()
            .p_0()
            .border_0()
            .bg(gpui::hsla(0., 0., 0., 0.))
            .child(
                super::components::account_avatar(
                    name,
                    avatar,
                    palette,
                    avatar_size,
                    super::components::AvatarState::Fallback,
                )
                .rounded_2xl()
                .text_size(px(36.)),
            );
        div()
            .id("home-avatar-halo")
            .relative()
            .on_hover(cx.listener(|view, hovered: &bool, _, cx| {
                view.shell.home.borrow_mut().avatar_hover.set_hovered(
                    *hovered,
                    cx.background_executor().now(),
                    view.motion_preferences(),
                );
                cx.notify();
            }))
            .child(
                div()
                    .absolute()
                    .inset(px(-16.))
                    .rounded_2xl()
                    .bg(rgb(
                        if palette.background == super::Palette::DARK.background {
                            0x0026_2626
                        } else {
                            0x00e5_e5e5
                        },
                    ))
                    .opacity(hover),
            )
            .child(super::tooltip::trigger(avatar, "Switch account", cx))
    }

    fn home_version(&self) -> Option<String> {
        let profile = self.profile()?;
        let (instance, _, target) = self.snapshot.selection(&profile.id).ok()?;
        let kind = self
            .version_catalog
            .iter()
            .find(|entry| entry.id == instance.minecraft_version)
            .map(|entry| entry.kind);
        let release = match kind {
            Some(minecraft_manifest::model::VersionType::Release) => "Stable release",
            Some(minecraft_manifest::model::VersionType::Snapshot) => "Snapshot",
            Some(minecraft_manifest::model::VersionType::OldBeta) => "Old beta",
            Some(minecraft_manifest::model::VersionType::OldAlpha) => "Old alpha",
            None => "Type unverified",
        };
        let loader = target.display_name();
        let description = format!(
            "Minecraft {} · {release} · {loader}{}",
            instance.minecraft_version,
            if kind.is_none() {
                ". Official release metadata is not available for this version."
            } else {
                ". Type verified by the official version catalog."
            }
        );

        Some(description)
    }

    fn home_task(&self, window: &Window, cx: &Context<'_, Self>) -> gpui::Stateful<gpui::Div> {
        let mut state = self.shell.home.borrow_mut();
        let observed = self.downloads.as_ref().map(|tracker| {
            tracker
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
        });
        let current = observed
            .as_ref()
            .and_then(|tracker| tracker.transfers().find(|file| !file.complete));
        let stopping = self.can_stop()
            && (self.cancel.is_cancelled()
                || (self.play_stop.is_cancelled()
                    && (self.active_run.is_some()
                        || self.operation_kind == super::OperationKind::Launch)));
        let phase = if stopping {
            "Stopping"
        } else if let Some(run) = &self.active_run {
            match self.session_revisions.get(run).map(|revision| revision.1) {
                Some(
                    block_launcher::session_host::RunState::Starting
                    | block_launcher::session_host::RunState::Preparing,
                ) => "Preparing launch",
                Some(block_launcher::session_host::RunState::MonitoringInterrupted) => {
                    "Monitoring interrupted"
                }
                _ => "Running",
            }
        } else if current.is_some() {
            "Downloading"
        } else if self.operation_kind == super::OperationKind::Launch {
            "Preparing launch"
        } else if self.operation_kind == super::OperationKind::Install {
            "Verifying installation"
        } else {
            "Working"
        };
        state.reconcile(
            self.busy.then(|| HomePanel {
                phase,
                status: self.status.clone(),
                resource: current.map(|file| (file.received, file.size)),
                verified: self.completed,
            }),
            cx.background_executor().now(),
            self.motion_preferences(),
        );
        let panel = state.panel.as_ref().map_or_else(
            || div().into_any_element(),
            |panel| self.home_task_content(panel, cx).into_any_element(),
        );
        super::motion::disclosure(
            "home-task-motion",
            &state.disclosure,
            self.motion_preferences(),
            panel,
            window,
            cx,
        )
        .debug_selector(|| "home-task-motion".into())
        .flex_shrink_0()
    }

    fn home_task_content(
        &self,
        panel: &HomePanel,
        cx: &Context<'_, Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let progress = panel.resource.map_or(
            super::components::ProgressState::Indeterminate,
            |(received, size)| super::components::ProgressState::from_counts(received, size),
        );

        div()
            .id("home-task")
            .debug_selector(|| "home-task".into())
            .min_h(px(80.))
            .w_full()
            .rounded_xl()
            .p_4()
            .bg(rgb(self.palette.surface))
            .flex()
            .items_center()
            .gap_3()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .id("home-task-phase")
                            .debug_selector(|| "home-task-phase".into())
                            .text_sm()
                            .child(panel.phase),
                    )
                    .child(div().text_xs().truncate().child(panel.status.clone()))
                    .children(
                        (panel.phase != "Running" && panel.phase != "Monitoring interrupted")
                            .then(|| super::components::progress(progress, self.palette)),
                    )
                    .child(note(panel.resource.map_or_else(
                        || {
                            format!(
                                "{} resources verified · overall total unknown",
                                panel.verified
                            )
                        },
                        |(received, size)| {
                            format!(
                                "Current resource: {received} / {} bytes",
                                size.map_or_else(|| "unknown".into(), |size| size.to_string())
                            )
                        },
                    ))),
            )
            .children(self.can_stop().then(|| {
                self.button("home-stop", "", UiAction::Stop, cx)
                    .child(super::design::Icon::Stop.render(14., self.palette.foreground))
            }))
    }

    fn management(&self, window: &Window, cx: &Context<'_, Self>) -> impl IntoElement + use<> {
        if self.tab == Tab::Accounts && self.shell.providers.visible {
            return self.render_providers(window, cx).into_any_element();
        }
        if let Some(Form::Instance(form)) = &self.form {
            return self.instance_form(form, window, cx).into_any_element();
        }
        if self.tab == Tab::Settings {
            return self.settings(window, cx).into_any_element();
        }
        if self.tab != Tab::About {
            return self.render_browser_detail(window, cx).into_any_element();
        }
        self.render_about(window, cx).into_any_element()
    }

    fn settings(&self, window: &Window, cx: &Context<'_, Self>) -> impl IntoElement + use<> {
        self.render_settings(window, cx)
    }

    fn render_popups(&self, window: &Window, cx: &Context<'_, Self>) -> Vec<AnyElement> {
        self.shell
            .surfaces
            .layers()
            .filter_map(|layer| {
                let Surface::Select(state) = &layer.kind else {
                    return None;
                };
                Some(
                    gpui_base::Positioner::corner(gpui::Anchor::TopLeft, state.anchor)
                        .margin(px(8.))
                        .child(
                            div()
                                .id(("select-popup", layer.revision))
                                .debug_selector(|| "drawer-sheet".into())
                                .track_focus(&layer.focus)
                                .key_context("LauncherForm")
                                .on_mouse_down_out(cx.listener(|view, _, window, cx| {
                                    view.handle(UiAction::Drawer(None), window, cx);
                                }))
                                .on_key_down(cx.listener(|view, event, window, cx| {
                                    if view.select_key(event, window, cx) {
                                        cx.stop_propagation();
                                    } else if event.keystroke.key == "escape" {
                                        view.handle(UiAction::Drawer(None), window, cx);
                                        cx.stop_propagation();
                                    }
                                }))
                                .w(state
                                    .width
                                    .min((window.viewport_size().width - px(16.)).max(px(1.))))
                                .max_h((window.viewport_size().height - px(24.)).max(px(100.)))
                                .p_1()
                                .rounded_lg()
                                .border_1()
                                .border_color(rgb(self.palette.border))
                                .bg(rgb(self.palette.popover))
                                .child(self.select_options(state, cx)),
                        )
                        .into_any_element(),
                )
            })
            .collect()
    }

    /// One overlay surface's body: notes and its action row. The dialog host
    /// owns the title and chrome; every value comes from current state so a
    /// confirmation that started an operation cannot keep stale buttons.
    pub(super) fn render_surface_dialog_body(
        &self,
        revision: usize,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let kind = self
            .shell
            .surfaces
            .layers()
            .find(|layer| layer.revision == revision)
            .map(|layer| layer.kind.clone());
        let Some(kind) = kind else {
            return div().into_any_element();
        };
        // Wizard and onboarding sections bring their own headings, scrolling
        // and actions; size the surface area like the old section window and
        // keep the dialog footer outside that fixed content height.
        let section_height = (window.viewport_size().height - px(150.)).clamp(px(320.), px(520.));
        let (content, section) = match &kind {
            Surface::OnboardingSection(page) => (
                div()
                    .h(section_height)
                    .min_h_0()
                    .w_full()
                    .child(self.render_onboarding_section(*page, window, cx)),
                true,
            ),
            Surface::WizardSection(_) => (
                div()
                    .min_h_0()
                    .w_full()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .h(section_height)
                            .min_h_0()
                            .w_full()
                            .child(self.render_game_wizard_transition_body(window, cx)),
                    )
                    .child(
                        super::patterns::management_footer(window.viewport_size(), self.palette)
                            .justify_end()
                            .child(
                                self.button("section-cancel", "Cancel", UiAction::Drawer(None), cx)
                                    .w_auto(),
                            )
                            .child(
                                self.button("section-done", "Done", UiAction::WizardPage(None), cx)
                                    .w_auto(),
                            ),
                    ),
                true,
            ),
            Surface::DownloadFiles(group) => (
                div()
                    .px(px(24.))
                    .child(self.render_download_files_body(*group, window)),
                false,
            ),
            Surface::GamesHelp => (div().px(px(24.)).child(self.render_games_help(cx)), false),
            Surface::AuthProviderAdd => (
                div()
                    .px(px(24.))
                    .pb(px(24.))
                    .child(self.render_auth_provider_add(cx)),
                true,
            ),
            _ => (
                div()
                    .px(px(24.))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .children(self.surface_dialog_notes(&kind).into_iter().map(note)),
                false,
            ),
        };
        let content = content.w_full().flex().flex_col();
        if section {
            return content.into_any_element();
        }
        // The separator spans the dialog edge to edge, while the actions keep
        // the same inset as the body.
        content
            .child(
                div()
                    .w_full()
                    .mt_4()
                    .pt_4()
                    .pb_4()
                    .border_t_1()
                    .border_color(rgb(self.palette.border))
                    .flex()
                    .justify_end()
                    .child(
                        div()
                            .px(px(24.))
                            .flex()
                            .justify_end()
                            .gap_2()
                            .children(self.surface_dialog_actions(&kind, cx)),
                    ),
            )
            .into_any_element()
    }

    /// The sheet header copy and height fraction for one home drawer.
    pub(super) fn surface_sheet_chrome(&self, kind: &Surface) -> (&'static str, &'static str, f32) {
        match kind {
            Surface::Drawer(Drawer::Profiles) => {
                ("Switch Profile", "Choose how you want to play.", 0.62)
            }
            Surface::Drawer(Drawer::Accounts) => (
                "Switch Account",
                "Choose the account you want to use.",
                0.62,
            ),
            Surface::Drawer(Drawer::Downloads) => ("Downloads", "Track your download tasks.", 0.56),
            _ => ("", "", 0.5),
        }
    }

    /// The sheet body for a home drawer. The sheet host owns the title,
    /// description, scrim, dismissal and focus; this renders the product list.
    pub(super) fn render_surface_sheet_body(
        &self,
        revision: usize,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let kind = self
            .shell
            .surfaces
            .layers()
            .find(|layer| layer.revision == revision)
            .map(|layer| layer.kind.clone());
        let Some(kind) = kind else {
            return div().into_any_element();
        };
        match &kind {
            Surface::Drawer(drawer @ (Drawer::Accounts | Drawer::Profiles)) => div()
                .w_full()
                .flex()
                .flex_col()
                .child(self.render_selector_body(*drawer, window, cx))
                .into_any_element(),
            Surface::Drawer(Drawer::Downloads) => div()
                .w_full()
                .flex()
                .flex_col()
                .child(self.render_downloads_body(window, cx))
                .into_any_element(),
            _ => div().into_any_element(),
        }
    }

    /// Name the exact record a confirmation removes when the catalog still
    /// holds it, falling back to the surface's neutral title.
    pub(super) fn surface_dialog_title(&self, kind: &Surface) -> String {
        match kind {
            Surface::DeleteProfile(id) => self
                .snapshot
                .profiles
                .iter()
                .find(|profile| &profile.id == id)
                .map_or_else(
                    || "Delete this profile?".to_owned(),
                    |profile| format!("Delete {}?", profile.name),
                ),
            Surface::RemoveAccount(id) => self
                .accounts
                .iter()
                .find(|account| &account_id(account) == id)
                .map_or_else(
                    || "Remove this account?".to_owned(),
                    |account| format!("Remove {}?", account.player_name),
                ),
            Surface::RemoveProvider(_) => "Remove this provider?".to_owned(),
            Surface::SeparateProfiles => "Enable separated profiles?".to_owned(),
            Surface::StorageCleanup(_) => "Remove this retained copy?".to_owned(),
            _ => kind.title().to_owned(),
        }
    }

    fn surface_dialog_notes(&self, kind: &Surface) -> Vec<String> {
        match kind {
            Surface::DeleteProfile(_) => vec![if self.snapshot.profile_mode
                == block_launcher::catalog::ProfileMode::Bundled
            {
                "This game and its launch settings are removed from the library. Worlds, game data and installation files stay on disk.".to_owned()
            } else {
                "Only launch settings are removed. Worlds, game data and installation files stay on disk.".to_owned()
            }],
            Surface::RemoveAccount(_) => vec![
                "This account is removed from the device, including any saved credentials. Your games and worlds stay on disk. Add the account again to use it.".to_owned(),
            ],
            Surface::RemoveProvider(id) => vec![if self.provider_links(id) > 0 {
                "This provider is used by saved accounts. Remove those accounts first.".to_owned()
            } else {
                "The saved provider and cached metadata are removed. Game files stay on disk.".to_owned()
            }],
            Surface::SeparateProfiles => vec![
                "This one-way change lets several launch profiles share an Instance. Existing games and files are preserved. Bundled mode cannot be restored.".to_owned(),
            ],
            Surface::StorageCleanup(id) => self.storage_cleanup_notes(*id),
            _ => Vec::new(),
        }
    }

    fn surface_dialog_actions(
        &self,
        kind: &Surface,
        cx: &Context<'_, Self>,
    ) -> Vec<super::components::Control> {
        if matches!(kind, Surface::GamesHelp) {
            return vec![
                self.surface_dialog_button(
                    "games-help-settings",
                    "Profile mode",
                    Some(UiAction::ProfileModeSettings),
                    SurfaceDialogTone::Outline,
                    cx,
                ),
                self.surface_dialog_button(
                    "close-drawer",
                    "Close",
                    None,
                    SurfaceDialogTone::Outline,
                    cx,
                ),
            ];
        }
        let (cancel_id, cancel_label, cancel_action, primary) = match kind {
            Surface::DeleteProfile(_) => (
                "close-drawer",
                "Cancel",
                None,
                Some((
                    "confirm-delete",
                    "Delete profile",
                    UiAction::ConfirmDelete,
                    SurfaceDialogTone::Destructive,
                )),
            ),
            Surface::RemoveAccount(id) => (
                "close-drawer",
                "Cancel",
                None,
                Some((
                    "confirm-remove-account",
                    "Remove account",
                    UiAction::SignOut(id.clone()),
                    SurfaceDialogTone::Destructive,
                )),
            ),
            Surface::RemoveProvider(id) => {
                let used = self.provider_links(id) > 0;
                (
                    "close-drawer",
                    if used { "Close" } else { "Cancel" },
                    None,
                    (!used).then(|| {
                        (
                            "confirm-remove-provider",
                            "Remove provider",
                            UiAction::ProviderRemove(id.clone()),
                            SurfaceDialogTone::Destructive,
                        )
                    }),
                )
            }
            Surface::SeparateProfiles => (
                "cancel-separate",
                "Keep bundled mode",
                Some(UiAction::ConfirmSeparate(false)),
                Some((
                    "confirm-separate",
                    "Enable separated profiles",
                    UiAction::SeparateProfiles,
                    SurfaceDialogTone::Primary,
                )),
            ),
            Surface::StorageCleanup(id) => (
                "storage-confirm-keep",
                "Keep copy",
                Some(UiAction::StorageCleanup(*id, false)),
                Some((
                    "storage-confirm-remove",
                    "Remove retained copy",
                    UiAction::StorageCleanup(*id, true),
                    SurfaceDialogTone::Destructive,
                )),
            ),
            Surface::DownloadFiles(_) => ("downloads-back", "Back", None, None),
            _ => ("close-drawer", "Close", None, None),
        };
        let mut actions = vec![self.surface_dialog_button(
            cancel_id,
            cancel_label,
            cancel_action,
            SurfaceDialogTone::Outline,
            cx,
        )];
        if let Some((id, label, action, tone)) = primary {
            actions.push(self.surface_dialog_button(id, label, Some(action), tone, cx));
        }
        actions
    }

    fn surface_dialog_button(
        &self,
        id: &'static str,
        label: &'static str,
        action: Option<UiAction>,
        tone: SurfaceDialogTone,
        cx: &Context<'_, Self>,
    ) -> super::components::Control {
        let variant = match tone {
            SurfaceDialogTone::Primary => super::components::ButtonVariant::Primary,
            SurfaceDialogTone::Destructive => super::components::ButtonVariant::Destructive,
            SurfaceDialogTone::Outline => super::components::ButtonVariant::Outline,
        };
        // A confirmation keeps its existing busy rules: only actions that stay
        // available while a run is active may fire, and a plain close always may.
        let available = action.as_ref().is_none_or(Self::available_while_busy);
        let on_activate = cx.listener(move |view, (): &(), window, cx| {
            if view.busy
                && action
                    .as_ref()
                    .is_some_and(|action| !Self::available_while_busy(action))
            {
                return;
            }
            match action.clone() {
                Some(action) => view.handle(action, window, cx),
                None => view.handle(UiAction::Drawer(None), window, cx),
            }
        });
        super::components::button(
            id,
            label,
            self.palette,
            variant,
            super::components::ControlState {
                disabled: self.busy && !available,
                ..Default::default()
            },
            on_activate,
        )
    }
}

#[derive(Clone, Copy)]
enum SurfaceDialogTone {
    Primary,
    Destructive,
    Outline,
}

#[cfg(test)]
#[path = "home_tests.rs"]
mod home_tests;
