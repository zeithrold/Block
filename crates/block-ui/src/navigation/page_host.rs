//! Shared page layouts and their independent frame, pane and wizard motion.
use std::time::Duration;

use gpui::{
    AnyElement, Context, Div, InteractiveElement as _, IntoElement as _, ParentElement as _,
    Styled as _, Window, div, px, rgb,
};
use gpui_base::ElementExt as _;

use super::{
    Block, Form,
    motion::Scope,
    navigation::{LayoutKind, Page},
};

pub(super) struct ContentSlots {
    pub body: AnyElement,
    pub footer: AnyElement,
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
    pub(super) fn action_footer<E: gpui::ParentElement>(&self, footer: E) -> E {
        if self.visual_only {
            footer
        } else {
            footer.on_prepaint(|bounds, window, cx| {
                super::feedback::ToastLayer::reserve_actions(
                    bounds.size.height + px(12.),
                    window,
                    cx,
                );
            })
        }
    }

    pub(super) fn render_page_body(&self, window: &Window, cx: &Context<'_, Self>) -> AnyElement {
        let spec = self.page_spec();
        let content = match spec.layout {
            LayoutKind::Plain => match spec.page {
                Page::Home => self.home(window, cx).into_any_element(),
                Page::FirstRun => self.first_run_page(window, cx),
                _ => unreachable!("plain layout requires a plain page"),
            },
            LayoutKind::SinglePane => match spec.page {
                Page::Settings => self.render_settings(cx).into_any_element(),
                _ => unreachable!("single-pane layout requires a single-pane page"),
            },
            LayoutKind::Management => {
                let (slots, transition_detail, id) = match spec.page {
                    Page::Browser => (self.render_browser_detail(cx), true, "browser-page"),
                    Page::Providers => (self.render_providers(cx), true, "providers-page"),
                    Page::About => (self.render_about(cx), false, "about-page"),
                    _ => unreachable!("management layout requires a management page"),
                };
                self.host_content(slots, transition_detail, window, cx)
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

    pub(super) fn host_content(
        &self,
        slots: ContentSlots,
        transition_detail: bool,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> Div {
        let body = if transition_detail {
            self.transition_content(Scope::Detail, slots.body, window, cx)
        } else {
            slots.body
        };
        div()
            .size_full()
            .min_h_0()
            .min_w_0()
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(
                self.frame_detail(
                    div()
                        .id("page-content")
                        .debug_selector(|| "page-content".into())
                        .flex_1()
                        .min_h_0()
                        .min_w_0()
                        .w_full()
                        .flex()
                        .flex_col()
                        .overflow_hidden()
                        .child(body),
                ),
            )
            .child(self.action_footer(div().w_full().flex_shrink_0().child(slots.footer)))
    }

    pub(super) fn host_wizard(
        &self,
        slots: WizardSlots,
        layout: WizardLayout,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let rail = self
            .frame_sidebar(div().w_full().min_h_0().flex_shrink_0().child(slots.rail))
            .into_any_element();
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
                .flex_col()
                .child(rail)
                .child(
                    self.frame_detail(
                        div()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .flex()
                            .flex_col()
                            .child(div().flex_1().min_h_0().flex().flex_col().child(body))
                            .child(
                                self.action_footer(
                                    super::patterns::management_footer(self.palette)
                                        .id("wizard-footer")
                                        .debug_selector(|| "wizard-footer".into())
                                        .h_auto()
                                        .min_h(px(super::design::MANAGEMENT_FOOTER))
                                        .py_3()
                                        .child(
                                            div()
                                                .w_full()
                                                .max_w(px(544.))
                                                .mx_auto()
                                                .flex()
                                                .flex_col()
                                                .child(slots.footer),
                                        ),
                                ),
                            ),
                    ),
                )
                .into_any_element(),
            WizardLayout::Authentication => div()
                .size_full()
                .min_h_0()
                .min_w_0()
                .flex()
                .flex_col()
                .child(
                    self.frame_detail(
                        div()
                            .flex_1()
                            .min_h_0()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(body),
                    ),
                )
                .child(self.action_footer(div().w_full().flex_shrink_0().child(slots.footer)))
                .into_any_element(),
        }
    }
}
