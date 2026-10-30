//! Shared page layouts and their independent frame, pane and wizard motion.
use std::time::Duration;

use gpui::{
    AnyElement, Context, Div, InteractiveElement as _, IntoElement as _, ParentElement as _,
    Styled as _, Window, div, px, rgb,
};

use super::{
    Block, Form, Palette,
    motion::Scope,
    navigation::{LayoutKind, Page},
};

pub(super) struct ManagementSlots {
    pub sidebar: AnyElement,
    /// The detail's scrolling body and fixed footer remain together.
    pub detail: AnyElement,
}

pub(super) struct WizardSlots {
    pub rail: AnyElement,
    pub body: AnyElement,
    pub footer: AnyElement,
}

pub(super) enum WizardLayout {
    Game,
    Authentication,
}

impl Block {
    pub(super) fn render_page_body(&self, window: &Window, cx: &Context<'_, Self>) -> AnyElement {
        let spec = self.page_spec();
        let content = match spec.layout {
            LayoutKind::Plain => match spec.page {
                Page::Home => self.home(window, cx).into_any_element(),
                Page::FirstRun => self.first_run_page(window, cx),
                _ => unreachable!("plain layout requires a plain page"),
            },
            LayoutKind::Management => {
                let (slots, palette, transition_detail, id) = match spec.page {
                    Page::Browser => (
                        self.render_browser_detail(cx),
                        self.palette,
                        true,
                        "browser-page",
                    ),
                    Page::Providers => (
                        self.render_providers(cx),
                        Palette {
                            sidebar: self.palette.background,
                            ..self.palette
                        },
                        true,
                        "providers-page",
                    ),
                    Page::Settings => (
                        self.render_settings(cx),
                        Palette {
                            sidebar: self.palette.background,
                            ..self.palette
                        },
                        true,
                        "settings-page",
                    ),
                    Page::About => (self.render_about(cx), self.palette, false, "about-page"),
                    _ => unreachable!("management layout requires a management page"),
                };
                self.host_management(slots, palette, transition_detail, window, cx)
                    .id(id)
                    .into_any_element()
            }
            LayoutKind::Wizard => match spec.page {
                Page::Authentication => self.render_auth_wizard(window, cx).into_any_element(),
                Page::GameWizard => match &self.form {
                    Some(Form::Instance(form)) => {
                        self.render_game_wizard(form, window, cx).into_any_element()
                    }
                    _ => div().into_any_element(),
                },
                _ => unreachable!("wizard layout requires a wizard page"),
            },
        };
        if spec.task {
            self.shell.wizard_entrance.apply(
                div()
                    .size_full()
                    .bg(rgb(self.palette.background))
                    .child(content)
                    .into_any_element(),
                super::window_geometry::content_size(window).height
                    - px(super::design::CHROME_HEIGHT),
                window,
                cx,
            )
        } else {
            content
        }
    }

    pub(super) fn host_management(
        &self,
        slots: ManagementSlots,
        palette: Palette,
        transition_detail: bool,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> Div {
        let detail = if transition_detail {
            self.transition_content(Scope::Detail, slots.detail, window, cx)
        } else {
            slots.detail
        };
        div()
            .size_full()
            .min_h_0()
            .min_w_0()
            .flex()
            .overflow_hidden()
            .child(
                self.frame_sidebar(
                    div()
                        .id("management-sidebar")
                        .debug_selector(|| "management-sidebar".into())
                        .h_full()
                        .min_h_0()
                        .min_w_0()
                        .w(px(super::design::SIDEBAR_WIDTH))
                        .flex_shrink_0()
                        .bg(rgb(palette.sidebar))
                        .border_r_1()
                        .border_color(rgb(palette.border))
                        .child(slots.sidebar),
                ),
            )
            .child(
                self.frame_detail(
                    div()
                        .id("management-content")
                        .debug_selector(|| "management-content".into())
                        .h_full()
                        .min_h_0()
                        .min_w_0()
                        .flex_1()
                        .child(detail),
                ),
            )
    }

    pub(super) fn host_wizard(
        &self,
        slots: WizardSlots,
        layout: WizardLayout,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let rail = self.shell.wizard_entrance.stagger(
            self.frame_sidebar(div().h_full().min_h_0().flex_shrink_0().child(slots.rail))
                .into_any_element(),
            Duration::ZERO,
            window,
            cx,
        );
        let body = self.transition_content(Scope::Wizard, slots.body, window, cx);
        let body = self
            .shell
            .wizard_entrance
            .stagger(body, Duration::from_millis(40), window, cx);
        match layout {
            WizardLayout::Game => div()
                .id("wizard-frame")
                .size_full()
                .min_h_0()
                .flex()
                .child(rail)
                .child(
                    self.frame_detail(
                        div()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .h_full()
                            .flex()
                            .flex_col()
                            .child(div().flex_1().min_h_0().child(body))
                            .child(
                                super::patterns::management_footer(self.palette)
                                    .id("wizard-footer")
                                    .debug_selector(|| "wizard-footer".into())
                                    .child(
                                        div()
                                            .w_full()
                                            .max_w(px(608.))
                                            .mx_auto()
                                            .flex()
                                            .flex_col()
                                            .child(slots.footer),
                                    ),
                            ),
                    ),
                )
                .into_any_element(),
            WizardLayout::Authentication => {
                div()
                    .size_full()
                    .min_h_0()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div().flex_1().min_h_0().flex().child(rail).child(
                            self.frame_detail(div().flex_1().min_h_0().min_w_0().child(body)),
                        ),
                    )
                    .child(self.frame_detail(div().w_full().child(slots.footer)))
                    .into_any_element()
            }
        }
    }
}
