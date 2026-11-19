//! The Microsoft browser handoff's status and recovery actions.

use std::{f32::consts::TAU, time::Duration};

use gpui::{
    Animation, AnimationExt as _, Context, InteractiveElement as _, IntoElement,
    ParentElement as _, PathBuilder, RenderOnce, Role, StatefulInteractiveElement as _,
    Styled as _, canvas, div, point, px, rgb,
};

use super::{AuthWizard, Block, ControlState, Method, UiAction, note, patterns};
use crate::{
    Palette,
    action_list::{ActionAppearance, ActionCopy, ActionDestination, ActionListCard, ActionRow},
};

impl Block {
    pub(super) fn browser_wait_heading(&self, state: &AuthWizard) -> gpui::Div {
        let language = self.language();
        let status = tr!(language, "accounts.auth.waiting.browser.status");
        let ring = WaitRing {
            palette: self.palette,
            phase: 0.,
        };
        // Outgoing auth snapshots intentionally do not retain authorization URLs.
        // Only the live browser handoff needs a repeating frame request.
        let ring = if state.browser_url.is_some() && !self.preferences.reduce_motion {
            ring.with_animation(
                "auth-browser-wait-ring",
                Animation::new(Duration::from_millis(1600)).repeat(),
                |ring, phase| WaitRing { phase, ..ring },
            )
            .into_any_element()
        } else {
            ring.into_any_element()
        };
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(patterns::page_title(tr!(
                language,
                "accounts.auth.waiting.browser.title"
            )))
            .child(
                div()
                    .id("auth-browser-wait-status")
                    .role(Role::Status)
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(ring)
                    .child(note(status).min_w_0()),
            )
    }

    pub(super) fn browser_wait_actions(
        &self,
        state: &AuthWizard,
        cx: &Context<'_, Self>,
    ) -> gpui::Div {
        let language = self.language();
        let reopen = ActionRow::new(
            "auth-reopen",
            ActionCopy {
                title: tr!(language, "accounts.auth.waiting.reopen").into(),
                description: tr!(language, "accounts.auth.waiting.reopen_note").into(),
            },
            ActionAppearance {
                palette: self.palette,
                destination: ActionDestination::External,
                state: ControlState {
                    disabled: state.browser_url.is_none() && !self.visual_only,
                    ..ControlState::default()
                },
            },
            cx.listener(|view, (), window, cx| view.handle(UiAction::AuthReopen, window, cx)),
        );
        let device = ActionRow::new(
            "auth-use-device-code",
            ActionCopy {
                title: tr!(language, "accounts.auth.waiting.switch_to_device").into(),
                description: tr!(language, "accounts.auth.waiting.switch_to_device_note").into(),
            },
            ActionAppearance {
                palette: self.palette,
                destination: ActionDestination::Forward,
                state: ControlState {
                    disabled: self.busy,
                    ..ControlState::default()
                },
            },
            cx.listener(|view, (), window, cx| {
                view.handle(UiAction::AuthMethod(Method::Device), window, cx);
            }),
        );
        ActionListCard::render(self.palette, [reopen, device]).mt(px(12.))
    }
}

#[derive(IntoElement)]
struct WaitRing {
    palette: Palette,
    phase: f32,
}

impl RenderOnce for WaitRing {
    fn render(self, _: &mut gpui::Window, _: &mut gpui::App) -> impl IntoElement {
        div()
            .relative()
            .size(px(14.))
            .flex_shrink_0()
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .rounded_full()
                    .border(px(1.6))
                    .border_color(rgb(self.palette.border)),
            )
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, (), window, _| {
                        let (sin, cos) = (self.phase * TAU - TAU * 0.375).sin_cos();
                        let center = bounds.center();
                        let radius = px(6.2);
                        let offset = point(radius * cos, radius * sin);
                        let mut arc = PathBuilder::stroke(px(1.6));
                        arc.move_to(center + offset);
                        arc.arc_to(point(radius, radius), px(0.), false, true, center - offset);
                        if let Ok(path) = arc.build() {
                            window.paint_path(path, rgb(self.palette.foreground));
                        }
                    },
                )
                .size_full(),
            )
    }
}
