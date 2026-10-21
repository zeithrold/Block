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
    Select(super::select::SelectState<UiAction>),
}

#[derive(Debug, Default)]
pub(super) struct ShellState {
    pub overlays: super::overlay::OverlayHost<Surface>,
    pub drawer_swipe: super::overlay_drawer::State,
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
        self.overlays.top().and_then(|layer| match layer.kind {
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
        let home = self.tab == Tab::Home && self.form.is_none() && self.shell.auth.is_none();
        let content = self.transition_content(
            super::motion::Scope::Page,
            self.render_page_body(window, cx),
            window,
            cx,
        );
        let modal = self.motion.captures_input();
        let mut root = div()
            .id("block-shell")
            .debug_selector(|| "block-shell".into())
            .key_context("LauncherForm")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|view, event: &gpui::KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    if view.dismiss_tooltip(window, cx) {
                        cx.stop_propagation();
                        return;
                    }
                    if view.motion.captures_input() && !view.shell.overlays.is_open() {
                        cx.stop_propagation();
                        return;
                    }
                    // Base Dialog owns Escape while a surface is mounted.
                    if !view.shell.overlays.is_open() {
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
            .child(
                div()
                    .flex_shrink_0()
                    .inert(modal)
                    .child(self.chrome(window, home, cx)),
            )
            .child(div().flex_1().min_h_0().inert(modal).child(content));
        root = root.children(self.render_overlay_motion(window, cx));
        root = root.child(self.render_tooltip(window, cx));
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
        let mut node = gpui::accesskit::Node::new(gpui::accesskit::Role::Status);
        node.set_label(toast.message.clone());
        node.set_live(gpui::accesskit::Live::Polite);
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
        let rail = if self.tab == Tab::Home && self.form.is_none() && self.shell.auth.is_none() {
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
                    .accessibility(node, false, |_, _, _| {})
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
            return "Add account";
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
        let authenticating = self.shell.auth.is_some();
        let first_run = self.tab == Tab::Onboarding;
        let narrow = super::patterns::is_narrow(window.viewport_size());
        let title = if home { "" } else { self.navigation_title() };
        div()
            .id("window-chrome")
            .h(px(if narrow { 40. } else { 56. }))
            .border_b_1().border_color(rgb(self.palette.border))
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
                    .id("window-drag-region")
                    .flex_1()
                    .h_full().flex().items_center().min_w_0()
                    .child(div().truncate().text_sm().child(title))
                    .on_mouse_down(gpui::MouseButton::Left, |event, window, _| {
                        if event.click_count == 2 {
                            window.zoom_window();
                        } else {
                            window.start_window_move();
                        }
                    }),
            )
            .children((!first_run && !authenticating).then(|| {
                if home {
                    div().children((self.busy && self.downloads.is_some()).then(|| super::tooltip::trigger(
                        self.button("downloads", "", UiAction::Drawer(Some(Drawer::Downloads)), cx)
                            .accessibility_label("View downloads")
                            .accessibility_expanded(self.shell.overlays.top().is_some_and(|layer| matches!(layer.kind, Surface::Drawer(Drawer::Downloads) | Surface::DownloadFiles(_))))
                            .accessibility_description("Real download activity. The overall resource total is not known.")
                            .child(super::components::progress_ring(super::components::ProgressState::Indeterminate, self.palette)), "View downloads", cx)))
                        .into_any_element()
                } else {
                    let (label, action) = self.contextual_back();
                    self.button("back-home", label, action, cx)
                        .child(super::design::Icon::ArrowLeft.render(16., self.palette.muted)).into_any_element()
                }
            }))
            .children((!self.shell.system_window_chrome).then(|| window_controls(self.palette, cx)))
    }

    pub(super) fn render_page_body(
        &self,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> gpui::AnyElement {
        if self.shell.auth.is_some() {
            return self.render_auth_wizard(window, cx).into_any_element();
        }
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
        let mut status = gpui::accesskit::Node::new(gpui::accesskit::Role::Status);
        status.set_label(self.status.clone());
        status.set_live(gpui::accesskit::Live::Polite);
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
                    .accessibility(status, false, |_, _, _| {})
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
            .accessibility_description(description.clone())
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
                    .accessibility_label("Switch game")
                    .accessibility_expanded(self.shell.overlays.top().is_some_and(|layer| {
                        matches!(layer.kind, Surface::Drawer(Drawer::Profiles))
                    }))
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
            let mut badge = gpui::accesskit::Node::new(gpui::accesskit::Role::Label);
            badge.set_label(format!("Account type: {provider} · Java Edition"));
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
                        .accessibility(badge, false, |_, _, _| {})
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
            .accessibility_label("Switch account")
            .accessibility_expanded(
                self.shell
                    .overlays
                    .top()
                    .is_some_and(|layer| matches!(layer.kind, Surface::Drawer(Drawer::Accounts))),
            )
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
                    .opacity(hover)
                    .paint_scale(0.25_f32.mul_add(hover, 0.75), gpui::point(0.5, 0.5))
                    .inert(true),
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
        let mut node = gpui::accesskit::Node::new(gpui::accesskit::Role::Group);
        node.set_label("Selected game version");
        node.set_description(description.clone());
        Some(super::tooltip::trigger(
            div()
                .id("home-version")
                .debug_selector(|| "home-version".into())
                .accessibility(node, true, |_, _, _| {})
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
        let mut semantic = gpui::accesskit::Node::new(gpui::accesskit::Role::Status);
        semantic.set_label(format!("{}: {}", panel.phase, panel.status));
        semantic.set_live(gpui::accesskit::Live::Polite);
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
                            .accessibility(semantic, false, |_, _, _| {})
                            .text_sm()
                            .child(panel.phase),
                    )
                    .child(div().text_xs().truncate().child(panel.status.clone()))
                    .children(
                        (panel.phase != "Running" && panel.phase != "Monitoring interrupted").then(
                            || {
                                super::components::progress(progress, self.palette)
                                    .accessibility_label("Task progress")
                            },
                        ),
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
                    .accessibility_label("Stop current operation")
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

    fn render_overlay_motion(
        &self,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> Vec<gpui::AnyElement> {
        let mut result = Vec::new();
        let now = cx.background_executor().now();
        for (index, scene) in self.motion.overlays.iter().enumerate() {
            let frame = scene.sample(now, self.motion_preferences());
            if frame.captures_input {
                result.push(
                    div()
                        .id(("overlay-shield", index))
                        .absolute()
                        .inset_0()
                        .occlude()
                        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .into_any_element(),
                );
            }
            if let (Some(pose), Some(old)) = (frame.outgoing, scene.outgoing())
                && let Some(layer) = old.snapshot.shell.overlays.layers().nth(index)
            {
                result.push(
                    old.snapshot
                        .overlay_presentation(
                            layer,
                            OverlayPresentation {
                                pose,
                                backdrop_opacity: frame.backdrop_opacity,
                                inert: true,
                            },
                            window,
                            cx,
                        )
                        .into_any_element(),
                );
            }
            if let Some(pose) = frame.current
                && let Some(layer) = self.shell.overlays.layers().nth(index)
            {
                result.push(
                    self.overlay_presentation(
                        layer,
                        OverlayPresentation {
                            pose,
                            backdrop_opacity: frame.backdrop_opacity,
                            inert: index + 1 < self.shell.overlays.layers().count(),
                        },
                        window,
                        cx,
                    )
                    .into_any_element(),
                );
            }
        }
        result
    }

    #[expect(
        clippy::too_many_lines,
        reason = "All overlay surfaces share one positioning, focus and dismissal implementation."
    )]
    fn overlay_presentation(
        &self,
        layer: &super::overlay::Layer<Surface>,
        presentation: OverlayPresentation,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement + use<> {
        let menu = matches!(layer.kind, Surface::Drawer(Drawer::Menu));
        let popup = menu || matches!(layer.kind, Surface::Select(_));
        let dialog = !matches!(
            layer.kind,
            Surface::Drawer(_) | Surface::Select(_) | Surface::DownloadFiles(_)
        );
        let viewport = window.viewport_size();
        let (role, title) = match &layer.kind {
            Surface::Drawer(Drawer::Menu) => (gpui::accesskit::Role::Menu, "Block menu"),
            Surface::Drawer(Drawer::Accounts) => (gpui::accesskit::Role::Dialog, "Switch account"),
            Surface::Drawer(Drawer::Profiles) => (gpui::accesskit::Role::Dialog, "Switch game"),
            Surface::Drawer(Drawer::Downloads) => (gpui::accesskit::Role::Dialog, "Downloads"),
            Surface::DownloadFiles(group) => (gpui::accesskit::Role::Dialog, group.label()),
            Surface::DeleteProfile(_) => (gpui::accesskit::Role::Dialog, "Delete profile"),
            Surface::RemoveAccount(_) => (gpui::accesskit::Role::Dialog, "Remove account"),
            Surface::SeparateProfiles => {
                (gpui::accesskit::Role::Dialog, "Enable separated profiles")
            }
            Surface::GamesHelp => (gpui::accesskit::Role::Dialog, "Instances and Profiles"),
            Surface::GuideInformation(tab) => (
                gpui::accesskit::Role::Dialog,
                if *tab == Tab::Settings {
                    "Settings"
                } else {
                    "About Block"
                },
            ),
            Surface::OnboardingSection(_) => (gpui::accesskit::Role::Dialog, "Game configuration"),
            Surface::Select(state) => (
                if state.menu.is_some() {
                    gpui::accesskit::Role::Menu
                } else {
                    gpui::accesskit::Role::ListBox
                },
                state.title.as_str(),
            ),
        };
        let content_role = if role == gpui::accesskit::Role::Dialog {
            gpui::accesskit::Role::Group
        } else {
            role
        };
        let content_title = if content_role == gpui::accesskit::Role::Group {
            ""
        } else {
            title
        };
        let mut semantic = gpui::accesskit::Node::new(content_role);
        semantic.set_label(content_title);
        let content = div()
            .id(("overlay-content", layer.revision))
            .debug_selector(|| "drawer-sheet".into())
            .accessibility(semantic, false, |_, _, _| {})
            .tab_group()
            .occlude()
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_key_down(
                cx.listener(move |view, event: &gpui::KeyDownEvent, window, cx| {
                    if view.select_key(event, window, cx) {
                        cx.stop_propagation();
                        return;
                    }
                    if view.shell.overlays.top().is_some_and(|layer| {
                        matches!(
                            layer.kind,
                            Surface::Drawer(Drawer::Accounts | Drawer::Profiles)
                                | Surface::OnboardingSection(_)
                        )
                    }) {
                        return;
                    }
                    if matches!(event.keystroke.key.as_str(), "down" | "up" | "home" | "end") {
                        let backwards = matches!(event.keystroke.key.as_str(), "up" | "end");
                        if matches!(event.keystroke.key.as_str(), "home" | "end")
                            && let Some(layer) = view.shell.overlays.top()
                        {
                            layer.focus.focus(window, cx);
                        }
                        view.move_focus(backwards, window, cx);
                        cx.stop_propagation();
                    }
                }),
            )
            .max_h((viewport.height - px(64.)).max(px(160.)))
            .p_5()
            .flex()
            .flex_col()
            .gap_3()
            .rounded_xl()
            .border_1()
            .border_color(rgb(self.palette.border))
            .bg(rgb(self.palette.popover));
        let mut content = super::scroll_area::viewport_with_role(
            content,
            &layer.scroll,
            content_title,
            self.palette,
            false,
            content_role,
        )
        .tab_index(-1)
        .tab_stop(false);
        let draggable = matches!(
            layer.kind,
            Surface::Drawer(Drawer::Accounts | Drawer::Profiles | Drawer::Downloads)
        );
        if draggable {
            let revision = layer.revision;
            content = content
                .child(self.drawer_swipe_handle(revision, cx))
                .on_prepaint_bounds({
                    let view = cx.weak_entity();
                    move |_, bounds, _, cx| {
                        let _ = view.update(cx, |view, _| {
                            view.shell
                                .drawer_swipe
                                .record_height(revision, bounds.size.height);
                        });
                    }
                });
        }
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
            Surface::GuideInformation(tab) => content.p_0().child(div().h((viewport.height - px(160.)).max(px(220.))).w_full().min_h_0()
                .child(if *tab == Tab::Settings { self.render_settings(window, cx).into_any_element() } else { self.render_about(window, cx).into_any_element() })),
            Surface::OnboardingSection(page) => content.p_0().gap_0().overflow_hidden().child(self.render_onboarding_section(*page, window, cx)),
            Surface::Select(state) => content.p_1().gap_0().rounded_lg().child(self.select_options(state, cx)),
        };
        if !popup && !matches!(layer.kind, Surface::OnboardingSection(_)) {
            content = content.child(self.button(
                "close-drawer",
                if dialog { "Cancel" } else { "Close" },
                UiAction::Drawer(None),
                cx,
            ));
        }
        let mut motion_viewport = viewport;
        if !popup && !dialog {
            motion_viewport.height = (viewport.height - px(48.)).min(px(460.));
        }
        let mut offset = presentation.pose.offset(motion_viewport);
        let swipe = self.shell.drawer_swipe.frame(
            layer.revision,
            motion_viewport.height,
            cx.background_executor().now(),
            self.motion_preferences(),
        );
        if draggable {
            offset.y += swipe.offset;
            if swipe.running {
                window.request_animation_frame();
            }
        }
        content = content
            .relative()
            .left(offset.x)
            .top(offset.y)
            .opacity(presentation.pose.opacity);
        let mut backdrop = div()
            .id(("overlay-backdrop", layer.revision))
            .inert(presentation.inert)
            .absolute()
            .inset_0();
        if presentation.pose.scale < 1. && !presentation.inert {
            // Scaling suppresses descendant input geometry. Keep the current
            // focus identity on the unscaled host until the first keyboard
            // event finishes the entrance and is routed to the real control.
            // Otherwise even Escape has no dispatch path during the 100 ms.
            let revision = layer.revision;
            backdrop = backdrop
                .track_focus(&window.focused(cx).unwrap_or_else(|| layer.focus.clone()))
                .on_key_down(
                    cx.listener(move |view, event: &gpui::KeyDownEvent, window, cx| {
                        if view
                            .shell
                            .overlays
                            .top()
                            .is_none_or(|top| top.revision != revision)
                        {
                            return;
                        }
                        if let Some(scene) = view.motion.overlays.last_mut() {
                            scene.settle();
                        }
                        let event = event.clone();
                        cx.notify();
                        if !popup && event.keystroke.key_char.is_some() {
                            // Install the real input handler before AppKit returns
                            // to its native IME. Do not synthesize printable text.
                            window.defer(cx, |window, cx| {
                                window.refresh();
                                window.draw(cx).clear(cx);
                            });
                            return;
                        }
                        window.defer(cx, move |window, cx| {
                            window.refresh();
                            if !event.is_held {
                                window.dispatch_keystroke(event.keystroke, cx);
                            }
                        });
                        cx.stop_propagation();
                    }),
                );
        }
        if popup {
            let anchor = if let Surface::Select(state) = &layer.kind {
                state.anchor
            } else {
                gpui::point(px(12.), px(52.))
            };
            // The shared Select records its trigger's left/bottom plus a 6px
            // gap. Use that actual anchor even when the popup snaps sideways
            // to fit the window, rather than scaling about a guessed corner.
            let scale_origin = gpui::point(
                anchor.x + offset.x,
                anchor.y
                    - px(if matches!(layer.kind, Surface::Select(_)) {
                        6.
                    } else {
                        8.
                    })
                    + offset.y,
            );
            backdrop = backdrop.child(
                gpui_base::Positioner::corner(gpui::Anchor::TopLeft, anchor)
                    .margin(px(8.))
                    .occlude()
                    .child(
                        content
                            .paint_scale_at(presentation.pose.scale, scale_origin)
                            .w(if let Surface::Select(state) = &layer.kind {
                                state.width.min((viewport.width - px(16.)).max(px(1.)))
                            } else {
                                px(224.).min((viewport.width - px(16.)).max(px(1.)))
                            }),
                    ),
            );
        } else {
            backdrop = backdrop
                .bg(gpui::hsla(
                    0.,
                    0.,
                    0.,
                    (112. / 255.)
                        * presentation.backdrop_opacity
                        * if draggable { swipe.backdrop } else { 1. },
                ))
                .flex();
            if dialog {
                backdrop = backdrop.items_center().justify_center().p_4().child(
                    content
                        .paint_scale(presentation.pose.scale, gpui::point(0.5, 0.5))
                        .w_full()
                        .max_w(px(if matches!(layer.kind, Surface::GuideInformation(_)) {
                            900.
                        } else {
                            440.
                        })),
                );
            } else {
                backdrop = backdrop.flex_col().justify_end().child(
                    content
                        .w_full()
                        .max_h((viewport.height - px(48.)).min(px(460.))),
                );
            }
        }
        super::base_overlay::Modal {
            view: cx.weak_entity(),
            focus: layer.focus.clone(),
            revision: layer.revision,
            label: title.to_owned().into(),
            depth: self
                .shell
                .overlays
                .layers()
                .position(|entry| entry.revision == layer.revision)
                .unwrap_or(0),
            inert: presentation.inert,
            content: backdrop.into_any_element(),
        }
    }
}

#[cfg(test)]
#[path = "home_tests.rs"]
mod home_tests;

#[derive(Clone, Copy)]
struct OverlayPresentation {
    pose: super::motion::Pose,
    backdrop_opacity: f32,
    inert: bool,
}

#[derive(Clone, Copy)]
enum WindowControl {
    Minimize,
    Close,
}

fn apply_window_control(control: WindowControl, window: &mut Window) {
    match control {
        WindowControl::Minimize => window.minimize_window(),
        WindowControl::Close => window.remove_window(),
    }
}

fn window_controls(palette: super::Palette, cx: &Context<'_, Block>) -> impl IntoElement + use<> {
    div().flex().gap_1().children(
        [
            (
                "window-minimize",
                super::design::Icon::Minus,
                WindowControl::Minimize,
            ),
            (
                "window-close",
                super::design::Icon::Close,
                WindowControl::Close,
            ),
        ]
        .map(|(id, icon, control)| {
            let control = super::components::button_sized(
                id,
                "",
                palette,
                super::components::ButtonVariant::Ghost,
                super::components::ButtonSize::Icon,
                super::components::ControlState::default(),
                cx.listener(move |_, (): &(), window, _| apply_window_control(control, window)),
            )
            .child(icon.render(16., palette.muted))
            .accessibility_label(if matches!(control, WindowControl::Close) {
                "Close Block"
            } else {
                "Minimize Block"
            });
            super::tooltip::trigger(
                control,
                if id == "window-close" {
                    "Close Block"
                } else {
                    "Minimize Block"
                },
                cx,
            )
        }),
    )
}
