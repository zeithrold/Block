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
    ParentElement as _, StatefulInteractiveElement as _, Styled as _, Window, WindowControlArea,
    div, img, px, rgb,
};

use super::{Block, Form, Tab, UiAction, account_id, note, scroll_area::ScrollAreaExt as _};

static BRAND: LazyLock<Arc<Image>> = LazyLock::new(|| {
    Arc::new(Image::from_bytes(
        ImageFormat::Png,
        include_bytes!("../../assets/block.png").to_vec(),
    ))
});

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Drawer {
    ImportGames,
    Language,
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
    GamesHelp,
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
    pub wizard_entrance: super::wizard_entrance::WizardEntrance,
    pub first_run: super::first_run::State,
    pub import_games: super::import_games::State,
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
    pub pending_toasts: Vec<super::feedback::Toast>,
    pub auth: Option<super::authentication::AuthWizard>,
    pub editor: super::profile_editor::EditorState,
    pub instance_editor: Option<super::instance_editor::InstanceEditor>,
    pub games: super::browser::CollectionState,
    pub profiles: super::browser::CollectionState,
    pub accounts: super::browser::CollectionState,
    pub providers: super::providers::State,
    pub selector: Option<super::search_selector::SearchSelectorState>,
    pub revision: usize,
    pub backwards: bool,
    pub settings_page: u8,
    pub wizard: super::game_wizard::WizardState,
    pub onboarding: super::onboarding::State,
    pub chrome_dragging: bool,
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
    phase: HomePhase,
    status: crate::i18n::UiText,
    resource: Option<(u64, Option<u64>)>,
    verified: usize,
}

/// What the retained task panel is doing. Presentation resolves the label, so a
/// retained panel follows the language it is painted in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HomePhase {
    Stopping,
    PreparingLaunch,
    MonitoringInterrupted,
    Running,
    Downloading,
    VerifyingInstallation,
    Working,
}

impl HomePhase {
    const fn key(self) -> &'static str {
        match self {
            Self::Stopping => "home.phase.stopping",
            Self::PreparingLaunch => "home.phase.preparing_launch",
            Self::MonitoringInterrupted => "home.phase.monitoring_interrupted",
            Self::Running => "home.phase.running",
            Self::Downloading => "home.phase.downloading",
            Self::VerifyingInstallation => "home.phase.verifying",
            Self::Working => "home.phase.working",
        }
    }

    /// A running game reports no aggregate progress, so only work with a
    /// verifiable total shows a bar.
    const fn shows_progress(self) -> bool {
        !matches!(self, Self::Running | Self::MonitoringInterrupted)
    }
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
            | Surface::GamesHelp
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
    pub(super) fn wizard_task_active(&self) -> bool {
        self.page_spec().task
    }

    pub(super) fn render_shell(
        &self,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement + use<> {
        let home = self.router.tab() == Tab::Home && self.form.is_none();
        let content = self.render_frame_body(window, cx);
        let mut root = div()
            .id("block-shell")
            .debug_selector(|| "block-shell".into())
            .key_context("LauncherForm")
            .track_focus(&self.focus)
            .on_mouse_up(
                gpui::MouseButton::Left,
                cx.listener(|view, _, _, _| view.shell.chrome_dragging = false),
            )
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
                if !view.motion.frame.active() {
                    view.move_focus(false, window, cx);
                }
            }))
            .on_action(cx.listener(|view, _: &super::PreviousField, window, cx| {
                if !view.motion.frame.active() {
                    view.move_focus(true, window, cx);
                }
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
        root = root.children(self.render_popups(false, cx));
        root
    }

    pub(super) fn navigation_title(&self) -> String {
        let language = self.language();
        let page = self.page_spec().page;
        if page == super::navigation::Page::Authentication {
            return tr!(language, "nav.accounts");
        }
        if page == super::navigation::Page::Providers {
            return tr!(language, "nav.authlib_providers");
        }
        if self.shell.instance_editor.is_some() {
            return tr!(language, "nav.instance_settings");
        }
        match &self.form {
            Some(Form::Profile(_)) => match self.shell.editor.page {
                Some(super::profile_editor::EditorPage::Appearance) => {
                    tr!(language, "nav.profile_appearance")
                }
                Some(super::profile_editor::EditorPage::Data) => {
                    tr!(language, "nav.data_and_account")
                }
                Some(super::profile_editor::EditorPage::Launch) | None => {
                    tr!(language, "nav.launch_settings")
                }
            },
            Some(Form::Instance(_))
                if page == super::navigation::Page::GameWizard
                    && self.router.tab() != Tab::Onboarding =>
            {
                tr!(language, "nav.create_game")
            }
            _ => match self.router.tab() {
                Tab::Home => "Block".to_owned(),
                Tab::Onboarding if self.shell.first_run.enabled => match self.shell.first_run.stage
                {
                    super::first_run::Stage::Welcome | super::first_run::Stage::Overview => {
                        tr!(language, "nav.setup_block")
                    }
                    super::first_run::Stage::GameChoice => tr!(language, "nav.setup_game"),
                    super::first_run::Stage::GameWizard => tr!(language, "nav.create_game"),
                },
                Tab::Onboarding => tr!(language, "nav.setup_block"),
                Tab::Instances | Tab::Profiles => tr!(language, "nav.games"),
                Tab::Accounts => tr!(language, "nav.accounts"),
                Tab::Settings => tr!(language, "nav.settings"),
                Tab::About => tr!(language, "nav.about"),
            },
        }
    }

    /// Window-chrome icon button: the same Ghost geometry as minimize/close so
    /// chrome never mixes an outlined action with the window controls.
    fn chrome_icon_button(
        &self,
        id: &'static str,
        action: UiAction,
        icon: impl IntoElement,
        label: String,
        cx: &Context<'_, Self>,
    ) -> super::components::Control {
        self.button_with_variant(id, "", action, super::components::ButtonVariant::Ghost, cx)
            .aria_label(label)
            .w(px(32.))
            .h(px(32.))
            .px_0()
            .justify_center()
            .child(icon)
    }

    fn chrome(
        &self,
        _window: &Window,
        home: bool,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement + use<> {
        let first_run = self.setup_incomplete() && self.shell.first_run.enabled;
        let task = self.wizard_task_active();
        let language = self.language();
        let title = if home
            || first_run && self.shell.first_run.stage == super::first_run::Stage::Welcome
        {
            String::new()
        } else {
            self.navigation_title()
        };
        div()
            .id("window-chrome")
            .h(px(super::design::CHROME_HEIGHT))
            .relative()
            .flex_shrink_0()
            .px_4()
            .pl(px(16.))
            .flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .absolute()
                    .bottom_0()
                    .left(px(16.))
                    .right(px(16.))
                    .h(px(1.))
                    .bg(rgb(self.palette.border)),
            )
            .child(if task {
                let action = if self.shell.auth.is_some() {
                    UiAction::AuthCancel
                } else if first_run {
                    UiAction::FirstRun(super::first_run::Action::Back)
                } else {
                    UiAction::CancelForm
                };
                let text = tr!(language, "chrome.close_setup");
                super::tooltip::trigger(
                    self.chrome_icon_button(
                        "task-close",
                        action,
                        super::design::Icon::ChevronDown.render(16., self.palette.muted),
                        text.clone(),
                        cx,
                    ),
                    text,
                    cx,
                )
                .into_any_element()
            } else if first_run && self.shell.first_run.stage == super::first_run::Stage::Overview {
                let text = tr!(language, "chrome.back_to_welcome");
                super::tooltip::trigger(
                    self.chrome_icon_button(
                        "setup-back-welcome",
                        UiAction::FirstRun(super::first_run::Action::BackToWelcome),
                        super::design::Icon::ChevronLeft.render(16., self.palette.muted),
                        text.clone(),
                        cx,
                    ),
                    text,
                    cx,
                )
                .into_any_element()
            } else if first_run {
                div().w(px(32.)).into_any_element()
            } else {
                self.button("app-menu", "", UiAction::Drawer(Some(Drawer::Menu)), cx)
                    .child(super::design::Icon::Menu.render(18., self.palette.muted))
                    .into_any_element()
            })
            .child(
                div()
                    .id("window-title")
                    .debug_selector(|| "window-title".into())
                    .flex_1()
                    .h_full()
                    .flex()
                    .items_center()
                    .min_w_0()
                    .window_control_area(WindowControlArea::Drag)
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(|view, _, _, _| view.shell.chrome_dragging = true),
                    )
                    .on_mouse_up(
                        gpui::MouseButton::Left,
                        cx.listener(|view, _, _, _| view.shell.chrome_dragging = false),
                    )
                    .on_mouse_move(
                        cx.listener(|view, event: &gpui::MouseMoveEvent, window, _| {
                            if view.shell.chrome_dragging
                                && event.pressed_button == Some(gpui::MouseButton::Left)
                            {
                                view.shell.chrome_dragging = false;
                                window.start_window_move();
                            } else if event.pressed_button.is_none() {
                                view.shell.chrome_dragging = false;
                            }
                        }),
                    )
                    .child(div().truncate().text_sm().child(title)),
            )
            .children((!first_run).then(|| {
                if home {
                    div()
                        .children((self.busy && self.downloads.is_some()).then(|| {
                            let text = tr!(language, "chrome.view_downloads");
                            super::tooltip::trigger(
                                self.button(
                                    "downloads",
                                    "",
                                    UiAction::Drawer(Some(Drawer::Downloads)),
                                    cx,
                                )
                                .aria_label(text.clone())
                                .child(
                                    super::components::progress_ring(
                                        super::components::ProgressState::Indeterminate,
                                        self.palette,
                                    ),
                                ),
                                text,
                                cx,
                            )
                        }))
                        .into_any_element()
                } else {
                    div().into_any_element()
                }
            }))
            .child(self.chrome_icon_button(
                "window-minimize",
                UiAction::MinimizeWindow,
                super::design::Icon::Minus.render(16., self.palette.muted),
                tr!(language, "chrome.minimize"),
                cx,
            ))
            .child(self.chrome_icon_button(
                "window-close",
                UiAction::CloseWindow,
                super::design::Icon::Close.render(16., self.palette.muted),
                tr!(language, "chrome.close"),
                cx,
            ))
    }

    pub(super) fn home(&self, window: &Window, cx: &Context<'_, Self>) -> impl IntoElement + use<> {
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
            .children((!self.baseline_status()).then(|| {
                div()
                    .id("home-status")
                    .debug_selector(|| "home-status".into())
                    .mb_3()
                    .text_xs()
                    .text_color(rgb(self.palette.muted))
                    .child(self.status.text(self.language()))
            }));
        content = content.child(self.home_task(window, cx));
        // While an install/repair runs the Play action queues instead of
        // disappearing, so the user can ask for a launch without waiting.
        let language = self.language();
        let queue_play = self.play_queue_eligible() && account.is_some();
        if self.busy && !queue_play {
            return content.into_any_element();
        }
        let profile = self.profile();
        let (label, action) = if queue_play {
            (
                if self.pending_play {
                    tr!(language, "home.launch_queued")
                } else {
                    tr!(language, "home.queue_launch")
                },
                UiAction::Play,
            )
        } else {
            match profile.and_then(|profile| self.snapshot.selection(&profile.id).ok()) {
                Some((_, _, target)) => match &target.status {
                    block_launcher::catalog::InstallStatus::Ready if account.is_none() => {
                        (tr!(language, "home.sign_in_to_play"), UiAction::SignIn)
                    }
                    block_launcher::catalog::InstallStatus::Ready => {
                        (tr!(language, "home.launch"), UiAction::Play)
                    }
                    block_launcher::catalog::InstallStatus::Failed { .. } => {
                        (tr!(language, "home.retry_download"), UiAction::Repair)
                    }
                    block_launcher::catalog::InstallStatus::Pending
                    | block_launcher::catalog::InstallStatus::Installing => {
                        (tr!(language, "home.download"), UiAction::Repair)
                    }
                },
                None if profile.is_some() => (tr!(language, "home.repair_game"), UiAction::Repair),
                None => (tr!(language, "home.create_game"), UiAction::CreateInstance),
            }
        };
        if account.is_none() && matches!(action, UiAction::SignIn | UiAction::CreateInstance) {
            return content.into_any_element();
        }
        let description = if queue_play {
            if self.pending_play {
                tr!(language, "home.queue_pending")
            } else {
                tr!(language, "home.queue_waiting")
            }
        } else {
            profile.map_or_else(
                || tr!(language, "home.start_adventure"),
                |profile| profile.name.clone(),
            )
        };
        let icon = match &action {
            UiAction::Play => super::design::Icon::Play,
            UiAction::SignIn => super::design::Icon::Account,
            UiAction::CreateInstance => super::design::Icon::Plus,
            _ => super::design::Icon::Download,
        };
        // The Home action stays primary when its command becomes Create,
        // Repair or Download; changing only the idle fill leaves Outline's
        // hover/pressed colors behind.
        let primary = self
            .button_with_variant(
                "home-play",
                "",
                action,
                super::components::ButtonVariant::Primary,
                cx,
            )
            .w_full()
            .min_w_0()
            .h(px(80.))
            .rounded_xl()
            .px_5()
            .pr(px(76.))
            .items_center()
            .justify_start()
            .text_xl()
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .flex()
                    .items_center()
                    .gap_3()
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
                .mt_3()
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
                    .aria_label(tr!(language, "action.switch_game"))
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

    /// The welcome is already shown by the identity card.
    fn baseline_status(&self) -> bool {
        self.status.key() == Some("home.welcome")
    }

    fn home_identity(
        &self,
        account: Option<&block_auth::Account>,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> super::scroll_area::ScrollArea<gpui::Stateful<gpui::Div>> {
        let language = self.language();
        let name = account.map_or_else(
            || tr!(language, "home.welcome"),
            |account| account.player_name.clone(),
        );
        let avatar = account.and_then(|account| self.account_avatars.get(&account_id(account)));
        let has_account = account.is_some();
        let viewport = div()
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
            .scroll_area(&self.shell.home_scroll, "home.player", self.palette, false);
        // Center within a content-sized child, not the viewport: when content
        // overflows, its start must remain reachable at the scroll origin.
        let mut identity = div()
            .w_full()
            .min_h_full()
            .flex_shrink_0()
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
                                .id("home-player-name")
                                .debug_selector(|| "home-player-name".into())
                                .text_2xl()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .max_w_full()
                                .truncate()
                                .child(name.clone()),
                        )
                        .child(
                            div()
                                .id("home-account-type")
                                .debug_selector(|| "home-account-type".into())
                                .child(
                                    super::components::badge(
                                        provider,
                                        palette,
                                        super::components::BadgeVariant::Info,
                                    )
                                    .h(px(24.))
                                    .border_0(),
                                ),
                        ),
                )
                .child(self.home_avatar(&name, avatar, window, cx));
        } else {
            identity = identity
                .child(super::design::Icon::Account.render(40., self.palette.muted))
                .child(div().text_2xl().child(name))
                .child(note(tr!(language, "home.add_account_hint")))
                .child(
                    self.button(
                        "home-sign-in",
                        tr!(language, "home.add_account"),
                        UiAction::SignIn,
                        cx,
                    )
                    .bg(rgb(self.palette.primary))
                    .text_color(rgb(self.palette.on_primary)),
                );
        }
        viewport.child(identity)
    }

    fn home_avatar(
        &self,
        name: &str,
        avatar: Option<&super::components::AccountAvatar>,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement + use<> {
        let palette = self.palette;
        let avatar_size = (super::design::SIDEBAR_WIDTH - 48.) * 0.45;
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
            .child(super::tooltip::trigger(
                avatar,
                tr!(self.language(), "action.switch_account"),
                cx,
            ))
    }

    fn home_version(&self) -> Option<String> {
        let profile = self.profile()?;
        let (instance, _, target) = self.snapshot.selection(&profile.id).ok()?;
        let language = self.language();
        let kind = self
            .version_catalog
            .iter()
            .find(|entry| entry.id == instance.minecraft_version)
            .map(|entry| entry.kind);
        let release = match kind {
            Some(block_launcher::manifest::model::VersionType::Release) => {
                tr!(language, "home.version.release")
            }
            Some(block_launcher::manifest::model::VersionType::Snapshot) => {
                tr!(language, "home.version.snapshot")
            }
            Some(block_launcher::manifest::model::VersionType::OldBeta) => {
                tr!(language, "home.version.old_beta")
            }
            Some(block_launcher::manifest::model::VersionType::OldAlpha) => {
                tr!(language, "home.version.old_alpha")
            }
            None => tr!(language, "home.version.unverified"),
        };
        let note = if kind.is_none() {
            tr!(language, "home.version.note_unverified")
        } else {
            tr!(language, "home.version.note_verified")
        };
        let loader = target.display_name();

        Some(tr!(
            language,
            "home.version.summary",
            version = instance.minecraft_version,
            release = release,
            loader = loader,
            note = note,
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
            HomePhase::Stopping
        } else if let Some(run) = &self.active_run {
            match self.session_revisions.get(run).map(|revision| revision.1) {
                Some(
                    block_launcher::session_host::RunState::Starting
                    | block_launcher::session_host::RunState::Preparing,
                ) => HomePhase::PreparingLaunch,
                Some(block_launcher::session_host::RunState::MonitoringInterrupted) => {
                    HomePhase::MonitoringInterrupted
                }
                _ => HomePhase::Running,
            }
        } else if current.is_some() {
            HomePhase::Downloading
        } else if self.operation_kind == super::OperationKind::Launch {
            HomePhase::PreparingLaunch
        } else if self.operation_kind == super::OperationKind::Install {
            HomePhase::VerifyingInstallation
        } else {
            HomePhase::Working
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
        let language = self.language();
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
                            .child(tr!(language, panel.phase.key())),
                    )
                    .child(
                        div()
                            .text_xs()
                            .truncate()
                            .child(panel.status.text(language)),
                    )
                    .children(panel.phase.shows_progress().then(|| {
                        super::components::progress(
                            progress,
                            self.palette,
                            tr!(language, "common.in_progress"),
                        )
                    }))
                    .child(note(panel.resource.map_or_else(
                        || tr!(language, "home.verified_resources", count = panel.verified,),
                        |(received, size)| {
                            let total = size.map_or_else(
                                || tr!(language, "home.unknown_total"),
                                |size| size.to_string(),
                            );
                            tr!(
                                language,
                                "home.current_resource",
                                received = received,
                                total = total,
                            )
                        },
                    ))),
            )
            .children(self.can_stop().then(|| {
                self.button("home-stop", "", UiAction::Stop, cx)
                    .aria_label(tr!(language, "home.stop"))
                    .child(super::design::Icon::Stop.render(14., self.palette.foreground))
            }))
    }

    /// A select nested directly in a sheet must paint above that sheet's scrim.
    /// Other selects retain their existing shell host, including dialog-owned
    /// selects stacked over a sheet.
    pub(super) fn render_popups(&self, sheet: bool, cx: &Context<'_, Self>) -> Vec<AnyElement> {
        let mut sheet_parent = false;
        self.shell
            .surfaces
            .layers()
            .filter_map(|layer| {
                let state = match &layer.kind {
                    Surface::Select(state) if sheet_parent == sheet => state,
                    Surface::Select(_) => return None,
                    kind => {
                        sheet_parent = kind.overlay() == Some(OverlayPresentation::Sheet);
                        return None;
                    }
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
                                .w(state.width)
                                .max_h(px(696.))
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
        // Wizard sections bring their own headings, scrolling
        // and actions; size the surface area like the old section window and
        // keep the dialog footer outside that fixed content height.
        let language = self.language();
        let section_height = px(520.);
        let (content, section) = match &kind {
            Surface::WizardSection(page) => (
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
                            .child(self.render_game_wizard_section_body(*page, window, cx)),
                    )
                    .child(
                        super::patterns::management_footer(self.palette)
                            .justify_end()
                            .child(
                                self.button(
                                    "section-cancel",
                                    tr!(language, "common.cancel"),
                                    UiAction::Drawer(None),
                                    cx,
                                )
                                .w_auto(),
                            )
                            .child(
                                self.button(
                                    "section-done",
                                    tr!(language, "action.done"),
                                    UiAction::WizardPage(None),
                                    cx,
                                )
                                .w_auto(),
                            ),
                    ),
                true,
            ),
            Surface::DownloadFiles(group) => (
                div()
                    .px(px(24.))
                    .child(self.render_download_files_body(*group)),
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

    /// Fixed content height for each sheet.
    pub(super) fn surface_sheet_chrome(&self, kind: &Surface) -> (String, String, f32) {
        let language = self.language();
        match kind {
            Surface::Drawer(Drawer::ImportGames) => self.shell.import_games.chrome(language),
            Surface::Drawer(Drawer::Language) => (
                tr!(language, "setup.language.label"),
                tr!(language, "setup.language.description"),
                260.,
            ),
            Surface::Drawer(Drawer::Profiles) => (
                tr!(language, "sheet.profiles_title"),
                tr!(language, "sheet.profiles_description"),
                540.,
            ),
            Surface::Drawer(Drawer::Accounts) => (
                tr!(language, "sheet.accounts_title"),
                tr!(language, "sheet.accounts_description"),
                540.,
            ),
            Surface::Drawer(Drawer::Downloads) => (
                tr!(language, "surface.downloads"),
                tr!(language, "sheet.downloads_description"),
                440.,
            ),
            _ => (String::new(), String::new(), 440.),
        }
    }

    /// The sheet body for a home drawer. The sheet host owns the title,
    /// description, scrim, dismissal and focus; this renders the product list.
    pub(super) fn render_surface_sheet_body(
        &self,
        kind: &Surface,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        match kind {
            Surface::Drawer(Drawer::ImportGames) => self.render_import_games(window, cx),
            Surface::Drawer(Drawer::Language) => {
                div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(self.product_select_field(
                        "setup-language",
                        tr!(self.language(), "setup.language.label"),
                        self.language().self_name(),
                        super::select::ProductChoice::Language,
                        cx,
                    ))
                    .children(self.first_run_feedback(self.language()).map(|message| {
                        note(message).debug_selector(|| "setup-language-notice".into())
                    }))
                    .into_any_element()
            }
            Surface::Drawer(drawer @ (Drawer::Accounts | Drawer::Profiles)) => div()
                .size_full()
                .min_h_0()
                .flex()
                .flex_col()
                .child(self.render_selector_body(*drawer, window, cx))
                .into_any_element(),
            Surface::Drawer(Drawer::Downloads) => div()
                .size_full()
                .min_h_0()
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
        let language = self.language();
        match kind {
            Surface::DeleteProfile(id) => self
                .snapshot
                .profiles
                .iter()
                .find(|profile| &profile.id == id)
                .map_or_else(
                    || tr!(language, "dialog.delete_profile.title"),
                    |profile| {
                        tr!(
                            language,
                            "dialog.delete_profile.title_named",
                            name = profile.name,
                        )
                    },
                ),
            Surface::RemoveAccount(id) => self
                .accounts
                .iter()
                .find(|account| &account_id(account) == id)
                .map_or_else(
                    || tr!(language, "dialog.remove_account.title"),
                    |account| {
                        tr!(
                            language,
                            "dialog.remove_account.title_named",
                            name = account.player_name,
                        )
                    },
                ),
            Surface::RemoveProvider(_) => tr!(language, "dialog.remove_provider.title"),
            Surface::SeparateProfiles => tr!(language, "dialog.separate_profiles.title"),
            _ => kind.title(language),
        }
    }

    fn surface_dialog_notes(&self, kind: &Surface) -> Vec<String> {
        let language = self.language();
        match kind {
            Surface::DeleteProfile(_) => vec![if self.snapshot.profile_mode
                == block_launcher::catalog::ProfileMode::Bundled
            {
                tr!(language, "dialog.delete_profile.bundled")
            } else {
                tr!(language, "dialog.delete_profile.separated")
            }],
            Surface::RemoveAccount(_) => vec![tr!(language, "dialog.remove_account.note")],
            Surface::RemoveProvider(id) => vec![if self.provider_links(id) > 0 {
                tr!(language, "dialog.remove_provider.used")
            } else {
                tr!(language, "dialog.remove_provider.note")
            }],
            Surface::SeparateProfiles => vec![tr!(language, "dialog.separate_profiles.note")],
            _ => Vec::new(),
        }
    }

    fn surface_dialog_actions(
        &self,
        kind: &Surface,
        cx: &Context<'_, Self>,
    ) -> Vec<super::components::Control> {
        let language = self.language();
        if matches!(kind, Surface::GamesHelp) {
            return vec![
                self.surface_dialog_button(
                    "games-help-settings",
                    tr!(language, "action.profile_mode"),
                    Some(UiAction::ProfileModeSettings),
                    SurfaceDialogTone::Outline,
                    cx,
                ),
                self.surface_dialog_button(
                    "close-drawer",
                    tr!(language, "common.close"),
                    None,
                    SurfaceDialogTone::Outline,
                    cx,
                ),
            ];
        }
        let (cancel_id, cancel_label, cancel_action, primary) = match kind {
            Surface::DeleteProfile(_) => (
                "close-drawer",
                tr!(language, "common.cancel"),
                None,
                Some((
                    "confirm-delete",
                    tr!(language, "action.delete_profile"),
                    UiAction::ConfirmDelete,
                    SurfaceDialogTone::Destructive,
                )),
            ),
            Surface::RemoveAccount(id) => (
                "close-drawer",
                tr!(language, "common.cancel"),
                None,
                Some((
                    "confirm-remove-account",
                    tr!(language, "action.remove_account"),
                    UiAction::SignOut(id.clone()),
                    SurfaceDialogTone::Destructive,
                )),
            ),
            Surface::RemoveProvider(id) => {
                let used = self.provider_links(id) > 0;
                (
                    "close-drawer",
                    if used {
                        tr!(language, "common.close")
                    } else {
                        tr!(language, "common.cancel")
                    },
                    None,
                    (!used).then(|| {
                        (
                            "confirm-remove-provider",
                            tr!(language, "action.remove_provider"),
                            UiAction::ProviderRemove(id.clone()),
                            SurfaceDialogTone::Destructive,
                        )
                    }),
                )
            }
            Surface::SeparateProfiles => (
                "cancel-separate",
                tr!(language, "action.keep_bundled"),
                Some(UiAction::ConfirmSeparate(false)),
                Some((
                    "confirm-separate",
                    tr!(language, "action.enable_separated"),
                    UiAction::SeparateProfiles,
                    SurfaceDialogTone::Primary,
                )),
            ),
            Surface::DownloadFiles(_) => {
                ("downloads-back", tr!(language, "action.back"), None, None)
            }
            _ => ("close-drawer", tr!(language, "common.close"), None, None),
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
        label: String,
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

#[cfg(test)]
mod phase_tests {
    use super::HomePhase;
    use crate::Language;

    /// The task panel keeps its identity as a phase, not as the label it last
    /// painted: the label resolves per language and the progress bar follows the
    /// phase itself.
    #[test]
    fn every_phase_keeps_its_own_label_and_progress_rule() {
        for (phase, key, shows_progress) in [
            (HomePhase::Stopping, "home.phase.stopping", true),
            (
                HomePhase::PreparingLaunch,
                "home.phase.preparing_launch",
                true,
            ),
            (
                HomePhase::MonitoringInterrupted,
                "home.phase.monitoring_interrupted",
                false,
            ),
            (HomePhase::Running, "home.phase.running", false),
            (HomePhase::Downloading, "home.phase.downloading", true),
            (
                HomePhase::VerifyingInstallation,
                "home.phase.verifying",
                true,
            ),
            (HomePhase::Working, "home.phase.working", true),
        ] {
            assert_eq!(phase.key(), key);
            assert_eq!(phase.shows_progress(), shows_progress, "{key}");
            for language in Language::ALL {
                let label = tr!(language, key);
                assert!(!label.is_empty(), "{key} resolved to nothing");
                assert_ne!(label, key, "{key} has no {} text", language.code());
            }
        }
    }
}
