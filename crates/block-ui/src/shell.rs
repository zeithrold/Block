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
    Context, Image, ImageFormat, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, Window, div, img, px, rgb,
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
    SeparateProfiles,
    GamesHelp,
    GuideInformation(Tab),
    OnboardingSection(super::WizardPage),
    WizardSection(super::WizardPage),
    Auth,
    Select(super::select::SelectState<UiAction>),
}

#[derive(Debug, Default)]
pub(super) struct ShellState {
    pub surfaces: super::window_surfaces::SurfaceWindows,
    pub connection: super::connection_test::State,
    pub downloads: super::downloads::State,
    pub home_scroll: gpui::ScrollHandle,
    pub home: RefCell<HomeState>,
    pub about_scroll: gpui::ScrollHandle,
    pub output_scroll: gpui::ScrollHandle,
    pub toast: Option<super::feedback::Toast>,
    pub auth: Option<super::authentication::AuthWizard>,
    pub editor: super::profile_editor::EditorState,
    pub instance_editor: Option<super::instance_editor::InstanceEditor>,
    pub games: super::browser::CollectionState,
    pub profiles: super::browser::CollectionState,
    pub accounts: super::browser::CollectionState,
    pub selector: Option<super::search_selector::SearchSelectorState>,
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
            | Surface::SeparateProfiles
            | Surface::GamesHelp
            | Surface::GuideInformation(_)
            | Surface::OnboardingSection(_)
            | Surface::WizardSection(_)
            | Surface::Auth
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
            .capture_any_mouse_down(
                cx.listener(|view, _, window, _| view.shell.surfaces.activate(window)),
            )
            .capture_key_down(
                cx.listener(|view, _, window, _| view.shell.surfaces.activate(window)),
            )
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|view, event: &gpui::KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    // Base Dialog owns Escape while a surface is mounted.
                    if !view.shell.surfaces.is_open() {
                        if view.shell.auth.is_some() {
                            view.handle(UiAction::AuthCancel, window, cx);
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
            .h(px(if narrow { 40. } else { 56. }))
            .border_b_1()
            .border_color(rgb(self.palette.border))
            .flex_shrink_0()
            .px_4()
            .flex()
            .items_center()
            .gap_2()
            .child(if authenticating {
                div().text_sm().child("Block").into_any_element()
            } else {
                self.button("app-menu", "", UiAction::Drawer(Some(Drawer::Menu)), cx)
                    .child(super::design::Icon::More.render(18., self.palette.muted))
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

    pub(super) fn render_page_body(
        &self,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> gpui::AnyElement {
        if self.tab == Tab::Onboarding {
            return self.onboarding(window, cx);
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
            .gap_3()
            .child(self.home_identity(account, window, cx))
            .child(
                div()
                    .id("home-status")
                    .text_xs()
                    .text_color(rgb(self.palette.muted))
                    .child(self.status.clone()),
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
        let primary = self
            .button("home-play", label, action, cx)
            .flex_1()
            .min_w_0()
            .h(px(80.))
            .rounded_lg()
            .px_5()
            .flex_col()
            .items_start()
            .justify_center()
            .text_xl()
            .bg(rgb(self.palette.primary))
            .text_color(rgb(self.palette.on_primary))
            .child(div().text_xs().truncate().child(description));
        content = content.children(self.home_version(cx)).child(
            div()
                .flex()
                .gap_1()
                .h(px(80.))
                .flex_shrink_0()
                .child(primary)
                .child(
                    self.button(
                        "home-profiles",
                        "",
                        UiAction::Drawer(Some(Drawer::Profiles)),
                        cx,
                    )
                    .h_full()
                    .child(super::design::Icon::ChevronDown.render(18., self.palette.muted)),
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
            .gap_4()
            .text_center();
        if let Some(account) = account {
            let provider = match account.provider {
                block_auth::AccountProvider::Microsoft => "Microsoft",
            };
            let palette = self.palette;

            identity = identity
                .child(
                    div()
                        .text_2xl()
                        .max_w_full()
                        .truncate()
                        .child(name.to_owned()),
                )
                .child(
                    div()
                        .id("home-account-type")
                        .debug_selector(|| "home-account-type".into())
                        .child(super::components::badge(
                            provider,
                            palette,
                            super::components::BadgeVariant::Outline,
                        )),
                )
                .child(self.home_avatar(name, window, cx));
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
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement + use<> {
        let palette = self.palette;
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
            .p_4()
            .border_0()
            .bg(gpui::hsla(0., 0., 0., 0.))
            .child(super::components::avatar_fallback(
                name,
                palette,
                144.,
                super::components::AvatarState::Fallback,
            ));
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

    fn home_version(&self, cx: &Context<'_, Self>) -> Option<gpui::Stateful<gpui::Div>> {
        use super::components::{BadgeVariant, badge};
        let profile = self.profile()?;
        let (instance, _, target) = self.snapshot.selection(&profile.id).ok()?;
        let kind = self
            .version_catalog
            .iter()
            .find(|entry| entry.id == instance.minecraft_version)
            .map(|entry| entry.kind);
        let (release, color) = match kind {
            Some(minecraft_manifest::model::VersionType::Release) => {
                ("Stable release", BadgeVariant::Success)
            }
            Some(minecraft_manifest::model::VersionType::Snapshot) => {
                ("Snapshot", BadgeVariant::Warning)
            }
            Some(minecraft_manifest::model::VersionType::OldBeta) => {
                ("Old beta", BadgeVariant::Secondary)
            }
            Some(minecraft_manifest::model::VersionType::OldAlpha) => {
                ("Old alpha", BadgeVariant::Secondary)
            }
            None => ("Type unverified", BadgeVariant::Outline),
        };
        let loader = match target.kind {
            block_launcher::catalog::LoaderKind::Vanilla => "Vanilla".to_owned(),
            block_launcher::catalog::LoaderKind::NeoForge => {
                target.loader_version.as_ref().map_or_else(
                    || "NeoForge".into(),
                    |version| format!("NeoForge {version}"),
                )
            }
        };
        let description = format!(
            "Minecraft {} · {release} · {loader}{}",
            instance.minecraft_version,
            if kind.is_none() {
                ". Official release metadata is not available for this version."
            } else {
                ". Type verified by the official version catalog."
            }
        );

        Some(super::tooltip::trigger(
            div()
                .id("home-version")
                .debug_selector(|| "home-version".into())
                .tab_index(0)
                .flex()
                .flex_wrap()
                .gap_2()
                .child(badge(
                    instance.minecraft_version.clone(),
                    self.palette,
                    color,
                ))
                .child(badge(release, self.palette, color))
                .child(badge(loader, self.palette, BadgeVariant::Secondary)),
            description,
            cx,
        ))
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

    pub(super) fn render_surface_window(
        &self,
        layer: &super::window_surfaces::SurfaceWindow,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement + use<> {
        let content = div()
            .id(("surface-content", layer.revision))
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .gap_3()
            .p_5();
        let mut content = super::scroll_area::viewport(
            content,
            &layer.scroll,
            layer.kind.title(),
            self.palette,
            false,
        );
        content = match &layer.kind {
            Surface::Drawer(Drawer::Menu) => content,
            Surface::Drawer(drawer @ (Drawer::Accounts | Drawer::Profiles)) => content
                .child(self.render_selector(*drawer, window, cx)),
            Surface::Drawer(Drawer::Downloads) => content.child(self.render_downloads(window, cx)),
            Surface::DownloadFiles(group) => content.child(self.render_download_files(*group, window, cx)),
            Surface::DeleteProfile(_) => content
                .child(div().text_xl().child("Delete this profile?"))
                .child(note(if self.snapshot.profile_mode == block_launcher::catalog::ProfileMode::Bundled { "This game and its launch settings are removed from the library. Worlds, game data and installation files stay on disk." } else { "Only launch settings are removed. Worlds, game data and installation files stay on disk." }))
                .child(self.button("confirm-delete", "Delete profile", UiAction::ConfirmDelete, cx)),
            Surface::RemoveAccount(id) => content
                .child(div().text_xl().child("Remove this account?"))
                .child(note("Saved credentials are removed from this device. Your games and worlds stay on disk. Sign in again to use this account."))
                .child(self.button("confirm-remove-account", "Remove account", UiAction::SignOut(id.clone()), cx)),
            Surface::SeparateProfiles => content
                .child(div().text_xl().child("Enable separated profiles?"))
                .child(note("This one-way change lets several launch profiles share an Instance. Existing games and files are preserved. Bundled mode cannot be restored."))
                .child(self.button("confirm-separate", "Enable separated profiles", UiAction::SeparateProfiles, cx))
                .child(self.button("cancel-separate", "Keep bundled mode", UiAction::ConfirmSeparate(false), cx)),
            Surface::GamesHelp => content.child(self.render_games_help(cx)),
            Surface::GuideInformation(tab) => content.p_0().child(div().h_full().w_full().min_h_0()
                .child(if *tab == Tab::Settings { self.render_settings(window, cx).into_any_element() } else { self.render_about(window, cx).into_any_element() })),
            Surface::OnboardingSection(page) => content.p_0().gap_0().overflow_hidden().child(self.render_onboarding_section(*page, window, cx)),
            Surface::Auth => content.p_0().child(self.render_auth_wizard(window, cx)),
            Surface::WizardSection(_) => content.p_0().gap_0()
                .child(div().flex_1().min_h_0().child(self.render_game_wizard_transition_body(window, cx)))
                .child(super::patterns::management_footer(window.viewport_size(), self.palette).justify_end()
                    .child(self.button("section-cancel", "Cancel", UiAction::Drawer(None), cx).w_auto())
                    .child(self.button("section-done", "Done", UiAction::WizardPage(None), cx).w_auto())),
            Surface::Select(state) => content.p_1().gap_0().rounded_lg().child(self.select_options(state, cx)),
        };
        if !matches!(
            layer.kind,
            Surface::OnboardingSection(_)
                | Surface::WizardSection(_)
                | Surface::Auth
                | Surface::Select(_)
        ) {
            content =
                content.child(self.button("close-drawer", "Close", UiAction::Drawer(None), cx));
        }
        div()
            .id(("surface-window", layer.revision))
            .key_context("LauncherForm")
            .track_focus(&layer.focus)
            .capture_any_mouse_down(
                cx.listener(|view, _, window, _| view.shell.surfaces.activate(window)),
            )
            .capture_key_down(
                cx.listener(|view, _, window, _| view.shell.surfaces.activate(window)),
            )
            .on_action(cx.listener(|view, _: &super::NextField, window, cx| {
                view.move_focus(false, window, cx)
            }))
            .on_action(cx.listener(|view, _: &super::PreviousField, window, cx| {
                view.move_focus(true, window, cx)
            }))
            .on_key_down(cx.listener(|view, event: &gpui::KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    view.handle(UiAction::Drawer(None), window, cx);
                    cx.stop_propagation();
                }
            }))
            .size_full()
            .min_h_0()
            .relative()
            .bg(rgb(self.palette.background))
            .text_color(rgb(self.palette.foreground))
            .text_size(px(14.))
            .font_family(super::design::FONT_FAMILY)
            .child(content)
            .children(self.render_popups(window, cx))
    }

    fn render_popups(&self, window: &Window, cx: &Context<'_, Self>) -> Vec<gpui::AnyElement> {
        self.shell
            .surfaces
            .layers()
            .filter_map(|layer| {
                let Surface::Select(state) = &layer.kind else {
                    return None;
                };
                if layer.parent.window_id() != window.window_handle().window_id() {
                    return None;
                }
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
}

#[cfg(test)]
#[path = "home_tests.rs"]
mod home_tests;
