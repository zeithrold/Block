//! Microsoft account setup around the existing PKCE and secure-save lifecycle.
//! Browser authorization URLs are transient and never included in `Debug` output.

#![expect(
    clippy::redundant_pub_crate,
    reason = "Authentication presentation is shared only with the parent view and shell."
)]

use super::scroll_area::ScrollAreaExt as _;
use std::fmt;

use block_auth::{Account, SignInStage};
use gpui::{
    Context, FocusHandle, InteractiveElement as _, IntoElement, ParentElement as _, Pixels,
    ScrollHandle, Size, Styled as _, Window, div, point, px, relative, rgb,
};

use super::{
    Block, Tab, UiAction, account_id,
    components::{self, AvatarState, BadgeVariant, ButtonVariant, ControlState, ProgressState},
    design::Icon,
    note, patterns, window_geometry,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Phase {
    #[default]
    Choose,
    Preparing,
    Waiting,
    Verifying,
    Failed,
    Complete,
}

impl Phase {
    const fn step(self) -> usize {
        match self {
            Self::Choose => 0,
            Self::Preparing | Self::Waiting | Self::Failed => 1,
            Self::Verifying => 2,
            Self::Complete => 3,
        }
    }

    const fn pending(self) -> bool {
        matches!(self, Self::Preparing | Self::Waiting | Self::Verifying)
    }
}

#[derive(Default)]
pub(super) struct AuthWizard {
    phase: Phase,
    stage: Option<SignInStage>,
    revision: usize,
    browser_url: Option<String>,
    saved_account: Option<Account>,
    error: Option<String>,
    return_tab: Tab,
    return_size: Size<Pixels>,
    return_focus: Option<FocusHandle>,
    focus: Option<FocusHandle>,
    body_scroll: ScrollHandle,
    rail_scroll: ScrollHandle,
}

impl fmt::Debug for AuthWizard {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthWizard")
            .field("phase", &self.phase)
            .field("revision", &self.revision)
            .field("return_tab", &self.return_tab)
            .field("has_saved_account", &self.saved_account.is_some())
            .finish_non_exhaustive()
    }
}

impl AuthWizard {
    pub(super) fn snapshot(&self, cx: &Context<'_, Block>) -> Self {
        let body_scroll = ScrollHandle::new();
        body_scroll.set_offset(self.body_scroll.offset());
        let rail_scroll = ScrollHandle::new();
        rail_scroll.set_offset(self.rail_scroll.offset());
        Self {
            phase: self.phase,
            stage: self.stage,
            revision: self.revision,
            browser_url: None,
            saved_account: self.saved_account.clone(),
            error: self.error.clone(),
            return_tab: self.return_tab,
            return_size: self.return_size,
            return_focus: None,
            focus: Some(cx.focus_handle()),
            body_scroll,
            rail_scroll,
        }
    }

    pub(super) const fn motion_key(&self) -> usize {
        self.phase.step()
    }

    fn transition(&mut self, phase: Phase) {
        if self.phase.step() != phase.step() {
            self.revision = self.revision.wrapping_add(1);
            self.body_scroll.set_offset(point(px(0.), px(0.)));
        }
        self.phase = phase;
    }

    fn failed(&mut self, error: &str) {
        self.browser_url = None;
        self.error = Some(sign_in_error(error));
        self.body_scroll.set_offset(point(px(0.), px(0.)));
        self.transition(Phase::Failed);
    }
}

impl Block {
    pub(super) fn open_auth_wizard(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.busy || self.shell.auth.is_some() {
            return;
        }
        let return_focus = if self.shell.overlays.is_open() {
            Some(self.focus.clone())
        } else {
            window.focused(cx)
        };
        let focus = cx.focus_handle();
        let wizard = AuthWizard {
            return_tab: self.tab,
            return_size: window.viewport_size(),
            return_focus,
            focus: Some(focus.clone()),
            ..AuthWizard::default()
        };
        self.shell.overlays.clear(window, &self.focus, cx);
        self.shell.auth = Some(wizard);
        window.resize(window_geometry::management_size(window, cx));
        focus.focus(window, cx);
        cx.notify();
    }

    pub(super) fn start_auth(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.busy
            || self
                .shell
                .auth
                .as_ref()
                .is_none_or(|state| state.phase == Phase::Complete)
        {
            return;
        }
        self.begin_cancellable("Preparing secure Microsoft sign-in");
        let Some(state) = &mut self.shell.auth else {
            return;
        };
        state.browser_url = None;
        state.stage = None;
        state.saved_account = None;
        state.error = None;
        if !self.busy {
            state.failed("Secure sign-in could not start: engine unavailable");
            cx.notify();
            return;
        }
        state.transition(Phase::Preparing);
        if let Some(focus) = &state.focus {
            focus.focus(window, cx);
        }
        if let Some(engine) = &self.engine {
            engine.sign_in(self.cancel.clone());
        }
        cx.notify();
    }

    pub(super) fn auth_browser(&mut self, url: String, cx: &Context<'_, Self>) {
        if self.cancel.is_cancelled() {
            return;
        }
        if self
            .shell
            .auth
            .as_ref()
            .is_some_and(|state| !state.phase.pending())
        {
            return;
        }
        cx.open_url(&url);
        if let Some(state) = &mut self.shell.auth {
            state.browser_url = Some(url);
            state.transition(Phase::Waiting);
        }
    }

    pub(super) fn auth_stage(&mut self, progress: SignInStage) {
        let Some(state) = &mut self.shell.auth else {
            return;
        };
        if self.cancel.is_cancelled()
            || !state.phase.pending()
            || state.stage.is_some_and(|previous| previous > progress)
        {
            return;
        }
        state.stage = Some(progress);
        if progress >= SignInStage::MicrosoftAuthorization {
            state.browser_url = None;
            state.transition(Phase::Verifying);
        }
        self.status = auth_stage_label(progress).into();
    }

    pub(super) fn reopen_auth(&self, cx: &Context<'_, Self>) {
        if !self.cancel.is_cancelled()
            && let Some(state) = &self.shell.auth
            && state.phase == Phase::Waiting
            && let Some(url) = &state.browser_url
        {
            cx.open_url(url);
        }
    }

    /// This event is emitted only after real authentication and durable saving.
    pub(super) fn auth_signed_in(&mut self, account: Account) {
        if let Some(state) = &mut self.shell.auth
            && state.phase.pending()
            && !self.cancel.is_cancelled()
        {
            state.browser_url = None;
            state.saved_account = Some(account);
            state.error = None;
            state.transition(Phase::Complete);
        }
    }

    pub(super) fn auth_finished(&mut self, result: &Result<String, String>) {
        if let Some(state) = &mut self.shell.auth
            && state.phase.pending()
        {
            match result {
                Err(error) => state.failed(error),
                Ok(_) => state.failed("Sign-in ended without a saved account confirmation"),
            }
        }
    }

    pub(super) fn cancel_auth(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let pending = self
            .shell
            .auth
            .as_ref()
            .is_some_and(|state| state.phase.pending());
        let saved_while_finishing = self.busy
            && self
                .shell
                .auth
                .as_ref()
                .is_some_and(|state| state.phase == Phase::Complete);
        if pending || saved_while_finishing {
            if pending {
                self.cancel.cancel();
            }
            // Reject queued results from the abandoned operation. This changes
            // presentation ownership only; it never deletes a saved identity.
            self.operation_revision = self.operation_revision.wrapping_add(1);
            if let Some(engine) = &mut self.engine {
                engine.begin_operation(self.operation_revision);
            }
            self.busy = self.active_run.is_some();
            self.status = if pending {
                "Sign-in cancelled. Previously saved accounts are kept."
            } else {
                "The account is saved. Account setup closed."
            }
            .into();
        }
        self.close_auth_context(window, cx);
    }

    pub(super) fn finish_auth(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.busy {
            return;
        }
        let account = self
            .shell
            .auth
            .as_ref()
            .filter(|state| state.phase == Phase::Complete)
            .and_then(|state| state.saved_account.clone());
        let Some(account) = account else {
            return;
        };
        let id = account_id(&account);
        if self
            .shell
            .auth
            .as_ref()
            .is_some_and(|state| state.return_tab == Tab::Onboarding)
        {
            self.choose_onboarding_account(id.clone());
        } else {
            self.selected_account = Some(id.clone());
        }
        self.shell.accounts.inspected = Some(id);
        self.shell.accounts.detail_visible = true;
        self.status = format!("{} is saved and ready to use", account.player_name);
        self.close_auth_context(window, cx);
    }

    fn close_auth_context(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(state) = self.shell.auth.take() else {
            return;
        };
        self.tab = state.return_tab;
        let dimensions = if self.tab == Tab::Home {
            window_geometry::home_size(window, cx)
        } else if state.return_size.width > px(0.) && state.return_size.height > px(0.) {
            state.return_size
        } else {
            window_geometry::management_size(window, cx)
        };
        window.resize(dimensions);
        state
            .return_focus
            .as_ref()
            .unwrap_or(&self.focus)
            .focus(window, cx);
        cx.notify();
    }

    pub(super) fn render_auth_wizard(
        &self,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement + use<> {
        let Some(state) = &self.shell.auth else {
            return div();
        };
        let viewport = window.viewport_size();
        let narrow = patterns::is_narrow(viewport);
        let content = div()
            .size_full()
            .min_h_0()
            .min_w_0()
            .flex()
            .flex_col()
            .children(narrow.then(|| {
                div()
                    .px_4()
                    .py_3()
                    .flex_shrink_0()
                    .border_b_1()
                    .border_color(rgb(self.palette.border))
                    .child(format!(
                        "Account setup · Step {} of 4",
                        state.phase.step() + 1
                    ))
            }))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .children((!narrow).then(|| self.auth_rail(state)))
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .min_w_0()
                            .child(self.transition_content(
                                super::motion::Scope::Wizard,
                                self.render_auth_transition_body(window, cx),
                                window,
                                cx,
                            )),
                    ),
            )
            .child(self.auth_footer(state, viewport, cx));
        let mut root = div()
            .id("account-wizard")
            .debug_selector(|| "account-wizard".into())
            .size_full()
            .min_h_0()
            .min_w_0()
            .flex()
            .flex_col()
            .tab_group()
            .on_key_down(cx.listener(|view, event: &gpui::KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" && !view.shell.overlays.is_open() {
                    view.cancel_auth(window, cx);
                    cx.stop_propagation();
                }
            }));
        if let Some(focus) = &state.focus {
            root = root.track_focus(focus);
        }
        div().size_full().min_h_0().child(root.child(content))
    }

    pub(super) fn render_auth_transition_body(
        &self,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> gpui::AnyElement {
        let Some(state) = &self.shell.auth else {
            return div().into_any_element();
        };
        div()
            .id("auth-body")
            .debug_selector(|| "auth-body".into())
            .size_full()
            .min_h_0()
            .min_w_0()
            .scroll_area(
                &state.body_scroll,
                "Authentication steps",
                self.palette,
                false,
            )
            .p(px(if patterns::is_narrow(window.viewport_size()) {
                16.
            } else {
                32.
            }))
            .child(self.auth_body(state, cx))
            .into_any_element()
    }

    fn auth_rail(
        &self,
        state: &AuthWizard,
    ) -> super::scroll_area::ScrollArea<gpui::Stateful<gpui::Div>> {
        let mut rail = div()
            .id("auth-rail")
            .w(relative(super::design::BROWSER_FRACTION))
            .flex_shrink_0()
            .h_full()
            .min_h_0()
            .scroll_area(
                &state.rail_scroll,
                "Account setup navigation",
                self.palette,
                false,
            )
            .p_6()
            .flex()
            .flex_col()
            .gap_6()
            .border_r_1()
            .border_color(rgb(self.palette.border))
            .bg(rgb(self.palette.sidebar))
            .child(div().text_sm().child("Account setup"));
        let steps = [
            ("Account type", "Choose how to sign in"),
            ("Sign in", "Continue with Microsoft"),
            ("Verification", "Ownership and secure storage"),
            ("Complete", "Your account is saved"),
        ]
        .map(|(title, description)| components::StepperStep {
            title,
            description,
            complete: match title {
                "Account type" => state.phase != Phase::Choose,
                "Sign in" => matches!(state.phase, Phase::Verifying | Phase::Complete),
                "Verification" | "Complete" => state.phase == Phase::Complete,
                _ => false,
            },
        });
        rail = rail.child(components::stepper(
            "auth-steps",
            "Account setup steps",
            &steps,
            state.phase.step(),
            false,
            self.palette,
        ));
        rail.child(div().flex_1())
            .child(note(format!("Step {} of 4", state.phase.step() + 1)))
    }

    fn auth_body(&self, state: &AuthWizard, cx: &Context<'_, Self>) -> gpui::Div {
        let (title, description) = match state.phase {
            Phase::Choose => (
                "Add an account",
                "Choose an account you own to play Minecraft: Java Edition.",
            ),
            Phase::Preparing => (
                "Preparing secure sign-in",
                "Block is requesting a real Microsoft authorization session.",
            ),
            Phase::Waiting => (
                "Continue in your browser",
                "Complete Microsoft sign-in and return here. This window can stay open.",
            ),
            Phase::Verifying => (
                "Verifying your account",
                "Block is checking ownership, resolving your player profile and saving this account securely.",
            ),
            Phase::Failed => (
                "Sign-in needs your attention",
                "Your existing saved accounts are kept. You can try again.",
            ),
            Phase::Complete => (
                "Your account is ready",
                "Authorization has completed and this account is already saved on your device.",
            ),
        };
        let mut body = div().min_w_0().flex().flex_col().gap_6().child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(div().text_2xl().child(title))
                .child(note(description)),
        );
        body = match state.phase {
            Phase::Choose => body.child(self.auth_choices()),
            Phase::Preparing | Phase::Waiting | Phase::Verifying => {
                body.child(self.auth_waiting(state, cx))
            }
            Phase::Failed => body.child(
                components::card(self.palette)
                    .border_color(rgb(self.palette.destructive))
                    .child(Icon::Error.render(24., self.palette.destructive))
                    .child(note(state.error.clone().unwrap_or_else(|| {
                        "Sign-in could not finish. Try again.".into()
                    }))),
            ),
            Phase::Complete => body.child(self.auth_complete(state)),
        };
        body
    }

    fn auth_choices(&self) -> gpui::Div {
        let mut choices = div().flex().flex_col().gap_4()
            .child(components::card(self.palette).border_color(rgb(self.palette.primary))
                .child(components::radio("auth-microsoft", "Microsoft account", true, self.palette,
                    ControlState::default(), |(), _, _| {}))
                .child(note("Sign in securely in your browser. Block never asks for your Microsoft password.")));
        for (id, label, explanation) in [
            (
                "auth-authlib-unavailable",
                "Authlib-Injector",
                "Provider authentication is not supported in this version.",
            ),
            (
                "auth-device-unavailable",
                "Device Code",
                "Device authorization and polling are not supported in this version.",
            ),
            (
                "auth-offline-unavailable",
                "Offline mode",
                "Local offline identities are not supported in this version.",
            ),
        ] {
            choices = choices.child(
                components::card(self.palette)
                    .gap_2()
                    .child(components::button(
                        id,
                        label,
                        self.palette,
                        ButtonVariant::Ghost,
                        ControlState {
                            disabled: true,
                            ..ControlState::default()
                        },
                        |(), _, _| {},
                    ))
                    .child(note(explanation)),
            );
        }
        choices.child(note("Your account is saved when authorization succeeds. You can remove it later in Accounts."))
    }

    fn auth_waiting(&self, state: &AuthWizard, cx: &Context<'_, Self>) -> gpui::Div {
        let mut card = components::card(self.palette)
            .child(Icon::ExternalLink.render(32., self.palette.foreground))
            .child(note("After Microsoft returns authorization, Block verifies Java Edition ownership, resolves your player profile and saves the account securely."))
            .child(components::progress(ProgressState::Indeterminate, self.palette))
            .child(note("This can take a moment after you finish in the browser."));
        if let Some(stage) = state.stage {
            card = card.child(note(auth_stage_label(stage)));
            for (boundary, label) in [
                (SignInStage::Ownership, "Microsoft and Xbox authorization"),
                (SignInStage::PlayerProfile, "Java Edition ownership"),
                (SignInStage::SaveCredential, "Player profile"),
                (SignInStage::SaveMetadata, "Secure credential storage"),
            ] {
                let verified = stage >= boundary;
                card = card.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(if verified { Icon::Check } else { Icon::Info }.render(
                            14.,
                            if verified {
                                self.palette.success
                            } else {
                                self.palette.muted
                            },
                        ))
                        .child(note(format!(
                            "{label}{}",
                            if verified {
                                " · Verified"
                            } else {
                                " · Pending"
                            }
                        ))),
                );
            }
        }
        if state.browser_url.is_some() || self.visual_only && state.phase == Phase::Waiting {
            card = card.child(self.auth_button("auth-reopen", "Open browser again", UiAction::AuthReopen,
                ButtonVariant::Outline, ControlState::default(), cx))
                .child(note("If the browser did not open, try again. Block cannot tell whether you closed an external browser window."));
        }
        card
    }

    fn auth_complete(&self, state: &AuthWizard) -> gpui::Div {
        let Some(account) = &state.saved_account else {
            return components::card(self.palette)
                .child(note("Waiting for saved account confirmation."));
        };
        components::card(self.palette)
            .child(components::avatar_fallback(&account.player_name, self.palette, 64., AvatarState::Fallback))
            .child(div().text_xl().child(account.player_name.clone()))
            .child(components::badge("Microsoft account", self.palette, BadgeVariant::Secondary))
            .child(components::badge("Account saved", self.palette, BadgeVariant::Success))
            .child(note(if state.return_tab == Tab::Onboarding { "Finish returns to your guide with this account selected in its draft. Completing the guide applies that choice. Closing account setup keeps the saved account." } else { "Finish returns to where you started and selects this account. Closing this setup keeps the saved account." }))
    }

    fn auth_footer(
        &self,
        state: &AuthWizard,
        viewport: Size<Pixels>,
        cx: &Context<'_, Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let mut footer = patterns::management_footer(viewport, self.palette)
            .id("auth-footer")
            .debug_selector(|| "auth-footer".into())
            .justify_between();
        footer = footer.child(self.auth_button(
            "auth-cancel",
            if state.phase.pending() {
                "Cancel sign-in"
            } else {
                "Close"
            },
            UiAction::AuthCancel,
            ButtonVariant::Ghost,
            ControlState::default(),
            cx,
        ));
        let (label, action, busy) = match state.phase {
            Phase::Choose => ("Sign in", UiAction::Authenticate, false),
            Phase::Preparing | Phase::Waiting | Phase::Verifying => {
                ("Waiting…", UiAction::Authenticate, true)
            }
            Phase::Failed => ("Try again", UiAction::AuthRetry, false),
            Phase::Complete => ("Finish", UiAction::AuthFinish, self.busy),
        };
        footer.child(self.auth_button(
            "auth-primary",
            label,
            action,
            ButtonVariant::Primary,
            ControlState {
                busy,
                ..ControlState::default()
            },
            cx,
        ))
    }

    fn auth_button(
        &self,
        id: &'static str,
        label: &'static str,
        action: UiAction,
        variant: ButtonVariant,
        state: ControlState,
        cx: &Context<'_, Self>,
    ) -> components::Control {
        components::button(
            id,
            label,
            self.palette,
            variant,
            state,
            cx.listener(move |view, (), window, cx| {
                view.handle(action.clone(), window, cx);
            }),
        )
    }
}

fn sign_in_error(error: &str) -> String {
    let lower = error.to_lowercase();
    if lower.contains("cancel") {
        "Sign-in was cancelled. You can try again when you are ready.".into()
    } else if lower.contains("timeout") || lower.contains("timed out") || lower.contains("expired")
    {
        "Authorization expired. Start sign-in again to request a fresh session.".into()
    } else if lower.contains("ownership") || lower.contains("does not own") {
        "Minecraft: Java Edition ownership could not be verified. Check the Microsoft account and try again.".into()
    } else if lower.contains("metadata")
        || lower.contains("secure storage")
        || lower.contains("credential")
    {
        "Block could not save the account securely. Check access to the data directory and credential store, then try again.".into()
    } else if lower.contains("engine unavailable") {
        "Secure sign-in is unavailable. Check Block’s data directory and restart the app.".into()
    } else {
        "Microsoft sign-in could not finish. Check your connection and try again. Your saved accounts are kept.".into()
    }
}

const fn auth_stage_label(stage: SignInStage) -> &'static str {
    match stage {
        SignInStage::SecureStorage => "Checking secure credential storage",
        SignInStage::BrowserAuthorization => "Waiting for Microsoft authorization in your browser",
        SignInStage::MicrosoftAuthorization => "Exchanging Microsoft authorization",
        SignInStage::XboxAuthentication => "Authenticating with Xbox Live",
        SignInStage::XboxAuthorization => "Authorizing Minecraft services",
        SignInStage::MinecraftAuthentication => "Obtaining a Minecraft session",
        SignInStage::Ownership => "Verifying Java Edition ownership",
        SignInStage::PlayerProfile => "Resolving your player profile",
        SignInStage::SaveCredential => "Saving your refresh credential securely",
        SignInStage::SaveMetadata => "Saving public account metadata",
    }
}

#[cfg(test)]
mod tests {
    use gpui::{TestAppContext, px, size};

    use super::{AuthWizard, Phase};
    use crate::{Form, Tab, test_support::test_account};

    fn click(cx: &mut gpui::VisualTestContext, selector: &'static str) {
        cx.run_until_parked();
        let bounds = cx.debug_bounds(selector);
        assert!(bounds.is_some(), "missing {selector}");
        if let Some(bounds) = bounds {
            cx.simulate_click(bounds.center(), gpui::Modifiers::none());
        }
    }

    #[test]
    fn debug_output_excludes_authorization_urls_and_error_details() {
        let state = AuthWizard {
            browser_url: Some("https://example.invalid/?state=sensitive-auth-state".into()),
            error: Some("sensitive-error-detail".into()),
            ..AuthWizard::default()
        };
        let debug = format!("{state:?}");
        assert!(!debug.contains("example.invalid"));
        assert!(!debug.contains("sensitive"));
    }

    #[gpui::test]
    fn escape_from_account_setup_restores_the_onboarding_draft_and_focus(cx: &mut TestAppContext) {
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.tab = Tab::Onboarding;
                view.preferences.reduce_motion = true;
                view.begin_onboarding(cx);
                view.shell.onboarding.step = crate::onboarding::Step::Account;
                if let Some(Form::Instance(form)) = &view.form {
                    form.name
                        .update(cx, |input, cx| input.set_value("世界 draft", cx));
                }
                view.open_auth_wizard(window, cx);
            });
        });
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| view.shell.auth.is_some()));
        crate::test_support::press(cx, "escape");
        assert!(view.read_with(cx, |view, cx| {
            view.shell.auth.is_none()
                && view.tab == Tab::Onboarding
                && view.shell.onboarding.step == crate::onboarding::Step::Account
                && matches!(&view.form, Some(Form::Instance(form)) if form.name.read(cx).value() == "世界 draft")
        }));
        cx.update(|window, cx| {
            view.read_with(cx, |view, _| assert!(view.focus.is_focused(window)));
        });
    }

    #[gpui::test]
    fn cancelling_waiting_sign_in_invalidates_late_results_without_removing_accounts(
        cx: &mut TestAppContext,
    ) {
        let result = (|| -> anyhow::Result<()> {
            let account = test_account()?;
            let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
            cx.update(|window, cx| {
                view.update(cx, |view, cx| {
                    view.accounts = vec![account.clone()];
                    view.open_auth_wizard(window, cx);
                    if let Some(state) = &mut view.shell.auth {
                        state.transition(Phase::Waiting);
                    }
                    view.busy = true;
                    cx.notify();
                });
            });
            let operation = view.read_with(cx, |view, _| view.operation_revision);
            click(cx, "auth-cancel");
            assert!(view.read_with(cx, |view, _| {
                view.shell.auth.is_none()
                    && view.cancel.is_cancelled()
                    && view.operation_revision != operation
                    && view.accounts.len() == 1
            }));
            cx.update(|window, cx| {
                view.update(cx, |view, cx| {
                    view.open_auth_wizard(window, cx);
                    view.auth_signed_in(account.clone());
                    view.auth_finished(&Ok("late completion".into()));
                });
            });
            assert!(view.read_with(cx, |view, _| {
                view.shell.auth.as_ref().is_some_and(|state| {
                    state.phase == Phase::Choose && state.saved_account.is_none()
                }) && view.accounts.len() == 1
            }));
            Ok(())
        })();
        assert!(result.is_ok(), "{result:?}");
    }

    #[gpui::test]
    fn a_generic_success_message_cannot_fabricate_a_saved_account(cx: &mut TestAppContext) {
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.open_auth_wizard(window, cx);
                if let Some(state) = &mut view.shell.auth {
                    state.transition(Phase::Preparing);
                }
                view.auth_finished(&Ok("finished without identity".into()));
                view.finish_auth(window, cx);
            });
        });
        assert!(view.read_with(cx, |view, _| {
            view.shell.auth.as_ref().is_some_and(|state| {
                state.phase == Phase::Failed
                    && state.saved_account.is_none()
                    && state.error.is_some()
            }) && view.accounts.is_empty()
        }));
        // The network-free fixture has no Engine, so retry produces a real
        // unavailable error instead of opening a browser or claiming success.
        click(cx, "auth-primary");
        assert!(view.read_with(cx, |view, _| {
            view.shell
                .auth
                .as_ref()
                .is_some_and(|state| state.phase == Phase::Failed)
                && !view.busy
        }));
        assert_eq!(cx.opened_url(), None);
    }

    #[gpui::test]
    fn browser_reopen_uses_the_current_session_and_ignores_cancelled_results(
        cx: &mut TestAppContext,
    ) {
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| view.open_auth_wizard(window, cx));
        });
        let url = "https://example.invalid/synthetic-authorization";
        view.update(cx, |view, cx| {
            if let Some(state) = &mut view.shell.auth {
                state.transition(Phase::Preparing);
            }
            view.auth_browser(url.into(), cx);
            view.reopen_auth(cx);
        });
        assert_eq!(cx.opened_url().as_deref(), Some(url));
        assert!(view.read_with(cx, |view, _| {
            view.shell
                .auth
                .as_ref()
                .is_some_and(|state| state.phase == Phase::Waiting)
        }));
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.cancel_auth(window, cx);
                view.auth_browser("https://example.invalid/late-authorization".into(), cx);
            });
        });
        assert_eq!(cx.opened_url().as_deref(), Some(url));
        assert!(view.read_with(cx, |view, _| view.shell.auth.is_none()));
    }

    #[gpui::test]
    fn a_saved_account_finish_selects_it_and_keeps_the_original_game_draft(
        cx: &mut TestAppContext,
    ) {
        let result = (|| -> anyhow::Result<()> {
            let account = test_account()?;
            let account_id = crate::account_id(&account);
            let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
            cx.update(|window, cx| {
                view.update(cx, |view, cx| {
                    view.preferences.reduce_motion = true;
                    view.tab = Tab::Onboarding;
                    view.begin_onboarding(cx);
                    if let Some(Form::Instance(form)) = &view.form {
                        form.name
                            .update(cx, |input, cx| input.set_value("Keep this game draft", cx));
                    }
                    view.open_auth_wizard(window, cx);
                    if let Some(state) = &mut view.shell.auth {
                        state.transition(Phase::Waiting);
                    }
                    // This is a synthetic unit event, not evidence of real OAuth.
                    view.accounts = vec![account.clone()];
                    view.auth_signed_in(account);
                    view.auth_finished(&Ok("Account saved".into()));
                    cx.notify();
                });
            });
            assert!(view.read_with(cx, |view, _| {
                view.shell
                    .auth
                    .as_ref()
                    .is_some_and(|state| state.phase == Phase::Complete)
            }));
            click(cx, "auth-primary");
            assert!(view.read_with(cx, |view, cx| {
                view.shell.auth.is_none() && view.tab == Tab::Onboarding
                    && view.shell.onboarding.account.as_deref() == Some(account_id.as_str())
                    && view.selected_account.is_none()
                    && matches!(&view.form, Some(Form::Instance(form)) if form.name.read(cx).value() == "Keep this game draft")
            }));
            Ok(())
        })();
        assert!(result.is_ok(), "{result:?}");
    }

    #[gpui::test]
    fn closing_a_saved_account_before_finished_keeps_the_returned_game_draft(
        cx: &mut TestAppContext,
    ) {
        let result = (|| -> anyhow::Result<()> {
            let account = test_account()?;
            let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
            let (sender, receiver) = tokio::sync::mpsc::channel(2);
            cx.update(|window, cx| {
                view.update(cx, |view, cx| {
                    view.preferences.reduce_motion = true;
                    view.tab = Tab::Instances;
                    view.begin_game_wizard(cx);
                    view.open_auth_wizard(window, cx);
                    if let Some(state) = &mut view.shell.auth {
                        state.transition(Phase::Waiting);
                    }
                    view.busy = true;
                    view.accounts = vec![account.clone()];
                    view.auth_signed_in(account);
                    view.messages = Some(receiver);
                });
            });
            let operation = view.read_with(cx, |view, _| view.operation_revision);
            click(cx, "auth-cancel");
            sender.try_send(crate::engine::Message::ForOperation {
                operation,
                message: Box::new(crate::engine::Message::Finished(Ok(
                    "late final event".into()
                ))),
            })?;
            view.update(cx, crate::Block::poll);
            assert!(view.read_with(cx, |view, _| view.tab == Tab::Instances
                && view.form.is_some()
                && view.accounts.len() == 1
                && view.shell.auth.is_none()
                && !view.busy));
            Ok(())
        })();
        assert!(result.is_ok(), "{result:?}");
    }

    #[gpui::test]
    fn account_setup_keeps_actions_reachable_at_default_and_narrow_sizes(cx: &mut TestAppContext) {
        let result = (|| -> anyhow::Result<()> {
            let account = test_account()?;
            for (width, height) in [(960., 600.), (640., 540.), (360., 540.)] {
                for phase in [Phase::Choose, Phase::Waiting, Phase::Complete] {
                    let (_view, cx) = crate::test_support::window(cx, |window, cx| {
                        let mut view = crate::tests::test_view(window, cx);
                        view.preferences.reduce_motion = true;
                        view.shell.auth = Some(AuthWizard {
                            phase,
                            saved_account: (phase == Phase::Complete).then(|| account.clone()),
                            ..AuthWizard::default()
                        });
                        view
                    });
                    cx.simulate_resize(size(px(width), px(height)));
                    cx.run_until_parked();
                    let body = cx.debug_bounds("auth-body");
                    let footer = cx.debug_bounds("auth-footer");
                    assert!(body.is_some() && footer.is_some());
                    let (Some(body), Some(footer)) = (body, footer) else {
                        continue;
                    };
                    assert!(body.size.height > px(0.) && body.bottom() <= footer.top());
                    assert!(footer.bottom() <= px(height));
                    for selector in ["auth-cancel", "auth-primary"] {
                        let action = cx.debug_bounds(selector);
                        assert!(
                            action.is_some_and(|action| {
                                action.left() >= px(0.)
                                    && action.right() <= px(width)
                                    && action.top() >= footer.top()
                                    && action.bottom() <= footer.bottom()
                            }),
                            "{selector} outside {width}×{height}: {action:?}"
                        );
                    }
                }
            }
            Ok(())
        })();
        assert!(result.is_ok(), "{result:?}");
    }
    #[gpui::test]
    fn observed_stages_enter_verification_without_fabricating_or_reviving_success(
        cx: &mut TestAppContext,
    ) {
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.open_auth_wizard(window, cx);
                if let Some(state) = view.shell.auth.as_mut() {
                    state.phase = Phase::Waiting;
                    state.browser_url = Some("https://example.invalid/transient".into());
                }
                view.auth_stage(block_auth::SignInStage::Ownership);
                assert!(
                    view.shell
                        .auth
                        .as_ref()
                        .is_some_and(|state| state.phase == Phase::Verifying
                            && state.browser_url.is_none()
                            && state.saved_account.is_none())
                );
                view.auth_stage(block_auth::SignInStage::MicrosoftAuthorization);
                assert!(
                    view.shell.auth.as_ref().is_some_and(
                        |state| state.stage == Some(block_auth::SignInStage::Ownership)
                    )
                );
                view.auth_finished(&Err("Ownership verification failed".into()));
                view.auth_stage(block_auth::SignInStage::SaveMetadata);
                assert!(view.shell.auth.as_ref().is_some_and(
                    |state| state.phase == Phase::Failed && state.saved_account.is_none()
                ));
                view.cancel_auth(window, cx);
                view.auth_stage(block_auth::SignInStage::SaveMetadata);
                assert!(view.shell.auth.is_none());
            });
        });
    }
}
