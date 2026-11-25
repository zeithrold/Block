//! The Microsoft browser handoff's status and recovery actions.

use gpui::{
    Context, InteractiveElement as _, ParentElement as _, Role, StatefulInteractiveElement as _,
    Styled as _, div, px,
};

use super::{
    AuthWizard, Block, ControlState, Method, UiAction, note, patterns, wait_ring::WaitRing,
};
use crate::action_list::{
    ActionAppearance, ActionCopy, ActionDestination, ActionMenuList, ActionRow,
};
use crate::design::Icon;

impl Block {
    pub(super) fn browser_wait_heading(&self, state: &AuthWizard) -> gpui::Div {
        let language = self.language();
        let status = tr!(language, "accounts.auth.waiting.browser.status");
        // Outgoing auth snapshots intentionally do not retain authorization URLs.
        // Only the live browser handoff needs a repeating frame request.
        let ring = WaitRing::element(
            self.palette,
            "auth-browser-wait-ring",
            state.browser_url.is_some() && !self.preferences.reduce_motion,
        );
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
                icon: Icon::Browser,
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
                icon: Icon::DeviceCode,
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
        ActionMenuList::render(self.palette, [reopen, device]).mt(px(12.))
    }
}
