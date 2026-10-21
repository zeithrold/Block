#![expect(
    clippy::redundant_pub_crate,
    reason = "Keep private UI modules scoped to their parent."
)]
//! Native product shell, keeping page, detail and drawer animation lifetimes independent.
use std::{
    sync::{Arc, LazyLock},
    time::Duration,
};

use gpui::{
    Animation, AnimationExt as _, Context, Image, ImageFormat, InteractiveElement as _,
    IntoElement, ParentElement as _, StatefulInteractiveElement as _, Styled as _, Window, div,
    img, px, rgb,
};

use super::{Appearance, Block, Form, Tab, UiAction, account_id, note};

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

#[derive(Debug, Default)]
pub(super) struct ShellState {
    pub drawer: Option<Drawer>,
    pub revision: usize,
    pub drawer_revision: usize,
    pub drawer_focus: Option<gpui::FocusHandle>,
    pub return_focus: Option<gpui::FocusHandle>,
    pub backwards: bool,
    pub confirm_separation: bool,
    pub wizard: super::game_wizard::WizardState,
    pub onboarding_step: usize,
}

pub(super) fn brand(size: f32) -> impl IntoElement {
    img(BRAND.clone()).size(px(size))
}

impl Block {
    pub(super) fn render_shell(
        &self,
        _window: &mut Window,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement + use<> {
        let home = self.tab == Tab::Home && self.form.is_none();
        let content = if home {
            self.home(cx).into_any_element()
        } else if self.tab == Tab::Onboarding {
            self.onboarding(cx).into_any_element()
        } else {
            self.management(cx).into_any_element()
        };
        let content = self.animate(content, "page", false);
        let mut root = div()
            .id("block-shell")
            .debug_selector(|| "block-shell".into())
            .key_context("LauncherForm")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|view, event: &gpui::KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    if view.shell.drawer.is_some() {
                        view.handle(UiAction::Drawer(None), window, cx);
                    } else if view.form.is_some() {
                        view.handle(UiAction::CancelForm, window, cx);
                    }
                }
            }))
            .on_action(cx.listener(|view, _: &super::NextField, window, cx| {
                view.focus_field(false, window, cx);
            }))
            .on_action(cx.listener(|view, _: &super::PreviousField, window, cx| {
                view.focus_field(true, window, cx);
            }))
            .size_full()
            .relative()
            .overflow_hidden()
            .flex()
            .flex_col()
            .bg(rgb(self.palette.background))
            .text_color(rgb(self.palette.foreground))
            .text_size(px(14.))
            .child(self.chrome(home, cx))
            .child(div().flex_1().min_h_0().child(content));
        if let Some(drawer) = self.shell.drawer {
            root = root.child(self.drawer(drawer, cx));
        }
        root
    }

    fn chrome(&self, home: bool, cx: &Context<'_, Self>) -> impl IntoElement + use<> {
        let first_run = self.tab == Tab::Onboarding;
        div()
            .id("window-chrome")
            .h(px(48.))
            .flex_shrink_0()
            .px_4()
            .flex()
            .items_center()
            .gap_2()
            .child(if first_run {
                div().text_sm().child("Block · Welcome").into_any_element()
            } else {
                self.button("app-menu", "☰", UiAction::Drawer(Some(Drawer::Menu)), cx)
                    .into_any_element()
            })
            .child(
                div()
                    .id("window-drag-region")
                    .flex_1()
                    .h_full()
                    .on_mouse_down(gpui::MouseButton::Left, |event, window, _| {
                        if event.click_count == 2 {
                            window.zoom_window();
                        } else {
                            window.start_window_move();
                        }
                    }),
            )
            .children((!first_run).then(|| {
                if home {
                    self.button(
                        "downloads",
                        "Downloads",
                        UiAction::Drawer(Some(Drawer::Downloads)),
                        cx,
                    )
                } else {
                    self.button("back-home", "← Home", UiAction::Tab(Tab::Home), cx)
                }
            }))
            .child(window_controls(cx))
    }

    fn animate(
        &self,
        content: gpui::AnyElement,
        scope: &'static str,
        vertical: bool,
    ) -> gpui::AnyElement {
        let content = div().relative().size_full().child(content);
        if self.preferences.reduce_motion {
            return content.into_any_element();
        }
        let backwards = self.shell.backwards;
        content
            .with_animation(
                (
                    scope,
                    if vertical {
                        self.shell.drawer_revision
                    } else {
                        self.shell.revision
                    },
                ),
                Animation::new(Duration::from_millis(460)).with_easing(|t| 1. - (1. - t).powi(4)),
                move |element, delta| {
                    let distance = (1. - delta)
                        * if vertical {
                            48.
                        } else if backwards {
                            -40.
                        } else {
                            40.
                        };
                    if vertical {
                        element.top(px(distance)).opacity(delta)
                    } else {
                        element.left(px(distance)).opacity(delta)
                    }
                },
            )
            .into_any_element()
    }

    #[expect(
        clippy::too_many_lines,
        reason = "The home composition keeps the player and launch control hierarchy visible together."
    )]
    fn home(&self, cx: &Context<'_, Self>) -> impl IntoElement + use<> {
        let account = self
            .accounts
            .iter()
            .find(|account| Some(account_id(account)) == self.selected_account);
        let name = account.map_or("Welcome to Block", |account| account.player_name.as_str());
        let mut content = div()
            .size_full()
            .flex()
            .flex_col()
            .px_6()
            .pb_6()
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_4()
                    .child(div().text_2xl().child(name.to_owned()))
                    .child(note(if account.is_some() {
                        "Microsoft · Java Edition"
                    } else {
                        "Your next adventure starts here."
                    }))
                    .child(
                        div()
                            .id("account-avatar")
                            .debug_selector(|| "account-avatar".into())
                            .rounded_xl()
                            .p_4()
                            .cursor_pointer()
                            .hover(|style| style.bg(rgb(0x0080_8080)).opacity(0.8))
                            .on_click(cx.listener(|view, _, window, cx| {
                                view.handle(UiAction::Drawer(Some(Drawer::Accounts)), window, cx);
                            }))
                            .child(brand(144.)),
                    )
                    .child(if account.is_none() {
                        self.button(
                            "home-sign-in",
                            "Sign in with Microsoft",
                            UiAction::SignIn,
                            cx,
                        )
                        .into_any_element()
                    } else {
                        note("Switch account").into_any_element()
                    }),
            )
            .child(
                div()
                    .mb_4()
                    .text_xs()
                    .text_color(rgb(self.palette.muted))
                    .child(self.status.clone()),
            );
        if self.busy {
            content = content.child(
                div()
                    .rounded_xl()
                    .p_4()
                    .bg(rgb(self.palette.surface))
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .child("Preparing / running…")
                            .child(note(format!("{} objects verified", self.completed))),
                    )
                    .child(self.button("home-stop", "Stop", UiAction::Stop, cx)),
            );
        } else {
            let profile = self.profile();
            content = content.child(
                div()
                    .flex()
                    .gap_1()
                    .h(px(80.))
                    .child(
                        div()
                            .id("home-play")
                            .debug_selector(|| "home-play".into())
                            .flex_1()
                            .rounded_lg()
                            .px_5()
                            .flex()
                            .flex_col()
                            .justify_center()
                            .bg(rgb(self.palette.primary))
                            .text_color(rgb(self.palette.on_primary))
                            .cursor_pointer()
                            .hover(|style| style.opacity(0.85))
                            .on_click(cx.listener(|view, _, window, cx| {
                                view.handle(
                                    if view.profile().is_some() {
                                        UiAction::Play
                                    } else {
                                        UiAction::CreateInstance
                                    },
                                    window,
                                    cx,
                                );
                            }))
                            .child(div().text_xl().child(if profile.is_some() {
                                "▶  Launch"
                            } else {
                                "+  Create game"
                            }))
                            .child(
                                div().text_xs().child(
                                    profile
                                        .map_or("Start a new adventure", |profile| {
                                            profile.name.as_str()
                                        })
                                        .to_owned(),
                                ),
                            ),
                    )
                    .child(self.button(
                        "home-profiles",
                        "⌄",
                        UiAction::Drawer(Some(Drawer::Profiles)),
                        cx,
                    )),
            );
        }
        content
    }

    fn navigation(&self, cx: &Context<'_, Self>) -> impl IntoElement + use<> {
        let mut sidebar = div()
            .w(px(210.))
            .flex_shrink_0()
            .h_full()
            .p_4()
            .flex()
            .flex_col()
            .gap_2()
            .bg(rgb(self.palette.surface))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .mb_4()
                    .child(brand(40.))
                    .child(div().text_lg().child("Block")),
            );
        for (id, label, tab) in [
            ("nav-games", "Games", Tab::Instances),
            ("nav-profiles", "Profiles", Tab::Profiles),
            ("nav-accounts", "Accounts", Tab::Accounts),
            ("nav-settings", "Settings", Tab::Settings),
            ("nav-about", "About", Tab::About),
        ] {
            sidebar = sidebar.child(self.button(
                id,
                if self.tab == tab {
                    format!("●  {label}")
                } else {
                    label.into()
                },
                UiAction::Tab(tab),
                cx,
            ));
        }
        sidebar = sidebar.child(
            div()
                .flex_1()
                .min_h_0()
                .id("instance-list")
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap_2()
                .pt_4()
                .children(self.snapshot.instances.iter().map(|instance| {
                    self.button(
                        format!("instance-{}", instance.id),
                        instance.name.clone(),
                        UiAction::SelectInstance(instance.id.clone()),
                        cx,
                    )
                })),
        );
        sidebar.child(self.button(
            "create-instance",
            "+ New game",
            UiAction::CreateInstance,
            cx,
        ))
    }

    fn management(&self, cx: &Context<'_, Self>) -> impl IntoElement + use<> {
        let main = match &self.form {
            Some(Form::Instance(form)) => self.instance_form(form, cx).into_any_element(),
            Some(Form::Profile(form)) => self.profile_form(form, cx).into_any_element(),
            Some(Form::Delete(_)) => div().flex().flex_col().gap_4().child(div().text_xl().child("Delete this profile?"))
                .child(note("Only launch settings are removed. Worlds, game data and installation files stay on disk."))
                .child(self.button("confirm-delete", "Delete profile", UiAction::ConfirmDelete, cx))
                .child(self.button("cancel-delete", "Keep profile", UiAction::CancelForm, cx)).into_any_element(),
            None => match self.tab {
                Tab::Accounts => self.accounts_page(cx).into_any_element(),
                Tab::Profiles => self.profiles_page(cx).into_any_element(),
                Tab::Settings => self.settings(cx).into_any_element(),
                Tab::About => self.about(cx).into_any_element(),
                Tab::Home | Tab::Onboarding | Tab::Instances => self.instance_page(cx).into_any_element(),
            },
        };
        let mut detail = div()
            .id("main-content")
            .debug_selector(|| "management-detail".into())
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_y_scroll()
            .p_6()
            .flex()
            .flex_col()
            .gap_4()
            .child(main)
            .child(note(self.status.clone()));
        if self.show_output {
            detail = detail.child(self.output(cx));
        }
        div()
            .size_full()
            .flex()
            .child(self.navigation(cx))
            .child(detail)
    }

    fn settings(&self, cx: &Context<'_, Self>) -> impl IntoElement + use<> {
        div().flex().flex_col().gap_4().child(div().text_2xl().child("Settings"))
            .child(note("Make Block feel like home."))
            .child(div().text_lg().child("Appearance"))
            .child(div().flex().gap_2().children([("system", "System", Appearance::System), ("light", "Light", Appearance::Light), ("dark", "Dark", Appearance::Dark)].map(|(id, label, appearance)| {
                self.button(format!("appearance-{id}"), if self.preferences.appearance == appearance { format!("● {label}") } else { label.into() }, UiAction::SetAppearance(appearance), cx)
            })))
            .child(self.button("reduce-motion", if self.preferences.reduce_motion { "Reduce motion: On" } else { "Reduce motion: Off" }, UiAction::ReduceMotion, cx))
            .child(div().text_lg().child("Profile mode"))
            .child(if self.snapshot.profile_mode == block_launcher::catalog::ProfileMode::Separated {
                note("Separated profiles · several launch profiles can share an installation.").into_any_element()
            } else if self.shell.confirm_separation {
                div().flex().flex_col().gap_2()
                    .child(note("Switch to Separated profiles? Existing games and files stay in place. This conversion cannot be reversed."))
                    .child(self.button("confirm-separate", "Switch to Separated", UiAction::SeparateProfiles, cx))
                    .child(self.button("cancel-separate", "Keep Bundled", UiAction::ConfirmSeparate(false), cx)).into_any_element()
            } else {
                div().flex().flex_col().gap_2().child(note("Bundled profiles · one game, one profile. Recommended for beginners."))
                    .child(self.button("separate-profiles", "Switch to Separated…", UiAction::ConfirmSeparate(true), cx)).into_any_element()
            })
            .child(note("Preview changes immediately. Save to keep them; leaving this page discards unsaved changes."))
            .child(div().flex().gap_2()
                .child(self.button("save-preferences", "Save changes", UiAction::SavePreferences, cx))
                .child(self.button("reset-preferences", "Restore defaults", UiAction::ResetPreferences, cx)))
    }

    fn about(&self, cx: &Context<'_, Self>) -> impl IntoElement + use<> {
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(brand(96.))
            .child(div().text_2xl().child("Block"))
            .child(note("Your next Minecraft adventure starts here."))
            .child(div().child(format!(
                "Version {} · {} / {}",
                env!("CARGO_PKG_VERSION"),
                std::env::consts::OS,
                std::env::consts::ARCH
            )))
            .child(note(
                "Built with Rust and GPUI. Brand artwork from Block-Icon, icon-v4.",
            ))
            .child(
                div()
                    .id("product-website")
                    .px_3()
                    .py_2()
                    .cursor_pointer()
                    .on_click(|_, _, cx| cx.open_url("https://block.ztd.one"))
                    .child("block.ztd.one"),
            )
            .child(self.button(
                "show-output",
                if self.show_output {
                    "Hide output"
                } else {
                    "View output"
                },
                UiAction::ToggleOutput,
                cx,
            ))
            .child(self.button(
                "open-recipe",
                "Open development recipe…",
                UiAction::OpenRecipe,
                cx,
            ))
            .children(self.recipe.as_ref().map(|_| {
                self.button(
                    "launch-recipe",
                    "Launch development recipe",
                    UiAction::LaunchRecipe,
                    cx,
                )
            }))
    }

    fn drawer(&self, drawer: Drawer, cx: &Context<'_, Self>) -> impl IntoElement + use<> {
        let mut content = div()
            .id("selector-content")
            .max_h(px(450.))
            .overflow_y_scroll()
            .p_6()
            .flex()
            .flex_col()
            .gap_3()
            .rounded_t_xl()
            .border_1()
            .border_color(rgb(self.palette.border))
            .bg(rgb(self.palette.background))
            .child(self.button("close-drawer", "Close", UiAction::Drawer(None), cx));
        content = match drawer {
            Drawer::Menu => content.child(div().text_lg().child("Block")).children(
                [
                    ("menu-home", "Home", Tab::Home),
                    ("menu-games", "Games", Tab::Instances),
                    ("menu-accounts", "Accounts", Tab::Accounts),
                    ("menu-settings", "Settings", Tab::Settings),
                    ("menu-about", "About", Tab::About),
                ]
                .map(|(id, label, tab)| self.button(id, label, UiAction::Tab(tab), cx)),
            ),
            Drawer::Accounts => content
                .child(div().text_lg().child("Switch account"))
                .children(self.accounts.iter().map(|account| {
                    self.button(
                        format!("switch-account-{}", account_id(account)),
                        account.player_name.clone(),
                        UiAction::SelectAccount(account_id(account)),
                        cx,
                    )
                }))
                .child(self.button(
                    "manage-accounts",
                    "Manage accounts",
                    UiAction::Tab(Tab::Accounts),
                    cx,
                )),
            Drawer::Profiles => content
                .child(div().text_lg().child("Switch game"))
                .children(self.snapshot.profiles.iter().map(|profile| {
                    self.button(
                        format!("switch-profile-{}", profile.id),
                        profile.name.clone(),
                        UiAction::SelectProfile(profile.id.clone()),
                        cx,
                    )
                }))
                .child(self.button(
                    "manage-games",
                    "Manage games",
                    UiAction::Tab(Tab::Instances),
                    cx,
                )),
            Drawer::Downloads => content
                .child(div().text_lg().child("Downloads"))
                .child(note(self.status.clone()))
                .child(note(format!(
                    "{} resource objects verified",
                    self.completed
                )))
                .child(self.button(
                    "download-stop",
                    "Stop current operation",
                    UiAction::Stop,
                    cx,
                )),
        };
        if let Some(focus) = &self.shell.drawer_focus {
            content = content.track_focus(focus).tab_group();
        }
        let sheet = self.animate(content.into_any_element(), "drawer", true);
        div()
            .absolute()
            .occlude()
            .inset_0()
            .flex()
            .flex_col()
            .justify_end()
            .bg(gpui::rgba(0x0000_0070))
            .child(
                div()
                    .id("drawer-sheet")
                    .debug_selector(|| "drawer-sheet".into())
                    .w_full()
                    .h(px(460.))
                    .child(sheet),
            )
    }
}

fn window_controls<T: 'static>(cx: &Context<'_, T>) -> impl IntoElement + use<T> {
    div()
        .flex()
        .gap_1()
        .child(
            div()
                .id("window-minimize")
                .tab_index(0)
                .px_3()
                .py_2()
                .cursor_pointer()
                .hover(|style| style.bg(rgb(0x0073_7373)))
                .on_click(cx.listener(|_, _, window, _| window.minimize_window()))
                .child("−"),
        )
        .child(
            div()
                .id("window-close")
                .tab_index(0)
                .px_3()
                .py_2()
                .cursor_pointer()
                .hover(|style| style.bg(rgb(0x00dc_2626)))
                .on_click(cx.listener(|_, _, window, _| window.remove_window()))
                .child("×"),
        )
}
