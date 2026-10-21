//! Explicit account preparation followed by final persistence.
//! Browser authorization URLs are transient and never included in `Debug` output.

#![expect(
    clippy::redundant_pub_crate,
    reason = "Authentication presentation is shared only with the parent view and shell."
)]

use super::scroll_area::ScrollAreaExt as _;
use std::fmt;

use block_auth::{Account, AccountProvider, SignInStage};
use gpui::{
    Context, FocusHandle, InteractiveElement as _, IntoElement, ParentElement as _, Pixels,
    ScrollHandle, Size, Styled as _, Window, div, point, px, relative, rgb,
};

use super::{
    Block, Tab, UiAction, account_id,
    components::{self, AvatarState, BadgeVariant, ButtonVariant, ControlState, ProgressState},
    design::Icon,
    note, patterns,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Phase {
    #[default]
    Choose,
    Information,
    Preparing,
    Waiting,
    Verifying,
    Failed,
    Complete,
    Saving,
    Saved,
}

impl Phase {
    const fn step(self) -> usize {
        match self {
            Self::Choose => 0,
            Self::Information | Self::Preparing | Self::Waiting | Self::Failed => 1,
            Self::Verifying => 2,
            Self::Complete | Self::Saving | Self::Saved => 3,
        }
    }

    const fn pending(self) -> bool {
        matches!(self, Self::Preparing | Self::Waiting | Self::Verifying)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Method {
    #[default]
    Browser,
    Device,
}

#[derive(Clone)]
struct DeviceCode {
    user_code: String,
    verification_uri: String,
    expires_at: std::time::Instant,
}

#[derive(Default)]
pub(super) struct AuthWizard {
    phase: Phase,
    provider: AccountProvider,
    username: Option<gpui::Entity<super::TextInput>>,
    method: Method,
    device_code: Option<DeviceCode>,
    attempt: Option<(u64, block_launcher::CancellationToken)>,
    stage: Option<SignInStage>,
    revision: usize,
    browser_url: Option<String>,
    prepared_account: Option<Account>,
    error: Option<String>,
    return_tab: Tab,
    return_size: Size<Pixels>,
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
            .field("has_prepared_account", &self.prepared_account.is_some())
            .finish_non_exhaustive()
    }
}

impl AuthWizard {
    #[cfg(debug_assertions)]
    pub(super) fn scenario(
        &mut self,
        name: &str,
        account: Option<Account>,
        cx: &mut Context<'_, Block>,
    ) {
        self.phase = match name {
            "auth-information" => Phase::Information,
            "auth-offline-information" => Phase::Information,
            "auth-offline-error" => Phase::Failed,
            "auth-offline-complete" => Phase::Complete,
            "auth-device-preparing" => Phase::Preparing,
            "auth-device-waiting" => Phase::Waiting,
            "auth-waiting" => Phase::Waiting,
            "auth-verifying" => Phase::Verifying,
            "auth-complete" => Phase::Complete,
            "auth-saving" => Phase::Saving,
            "auth-error" => Phase::Failed,
            _ => Phase::Choose,
        };
        if matches!(
            name,
            "auth-complete" | "auth-saving" | "auth-offline-complete"
        ) {
            self.prepared_account = account;
        }
        if name.starts_with("auth-offline-") {
            self.provider = AccountProvider::Offline;
            if let Some(field) = &self.username {
                field.update(cx, |field, cx| {
                    field.set_value(
                        if name == "auth-offline-error" {
                            "bad name"
                        } else {
                            "Pixel"
                        },
                        cx,
                    )
                });
            }
            if name == "auth-offline-error" {
                self.error = Some("Use 1–16 ASCII letters, numbers, or underscores.".into());
            }
        }
        if name.starts_with("auth-device-") {
            self.method = Method::Device;
            self.stage = Some(SignInStage::DeviceAuthorization);
            if name == "auth-device-waiting" {
                self.device_code = Some(DeviceCode {
                    user_code: "TEST-CODE".into(),
                    verification_uri: "https://microsoft.com/devicelogin".into(),
                    expires_at: std::time::Instant::now() + std::time::Duration::from_secs(900),
                });
            }
        }
        if name == "auth-error" {
            self.error = Some("The authorization request expired. Try signing in again.".into());
        }
    }
    pub(super) fn snapshot(&self, cx: &Context<'_, Block>) -> Self {
        let body_scroll = ScrollHandle::new();
        body_scroll.set_offset(self.body_scroll.offset());
        let rail_scroll = ScrollHandle::new();
        rail_scroll.set_offset(self.rail_scroll.offset());
        Self {
            phase: self.phase,
            provider: self.provider,
            username: self.username.clone(),
            method: self.method,
            device_code: None,
            attempt: None,
            stage: self.stage,
            revision: self.revision,
            browser_url: None,
            prepared_account: self.prepared_account.clone(),
            error: self.error.clone(),
            return_tab: self.return_tab,
            return_size: self.return_size,
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
        self.device_code = None;
        self.error = Some(if self.provider == AccountProvider::Offline {
            "The local identity could not be saved. Check the data directory and try adding it again. Previously saved accounts are kept.".into()
        } else {
            sign_in_error(error)
        });
        self.body_scroll.set_offset(point(px(0.), px(0.)));
        self.transition(Phase::Failed);
    }
}

impl Block {
    pub(super) fn choose_auth_provider(
        &mut self,
        provider: AccountProvider,
        cx: &mut Context<'_, Self>,
    ) {
        if let Some(state) = &mut self.shell.auth
            && state.phase == Phase::Choose
            && !self.busy
        {
            state.provider = provider;
            state.error = None;
            state.prepared_account = None;
            cx.notify();
        }
    }

    pub(super) fn auth_next(&mut self, cx: &mut Context<'_, Self>) {
        if let Some(state) = &mut self.shell.auth
            && state.phase == Phase::Choose
        {
            state.transition(Phase::Information);
            cx.notify();
        }
    }

    fn abandon_auth_attempt(&mut self) {
        let attempt = self
            .shell
            .auth
            .as_mut()
            .and_then(|state| state.attempt.take());
        if let Some((revision, cancel)) = attempt {
            cancel.cancel();
            if revision == self.operation_revision {
                self.operation_revision = self.operation_revision.wrapping_add(1);
                if let Some(engine) = &mut self.engine {
                    engine.begin_operation(self.operation_revision);
                }
                self.busy = self.active_run.is_some();
                self.operation_cancellable = false;
                self.status = "Sign-in cancelled. Previously saved accounts are kept.".into();
            }
        }
        if let Some(engine) = &self.engine {
            engine.discard_prepared_sign_in();
        }
    }

    pub(super) fn auth_back(&mut self, cx: &mut Context<'_, Self>) {
        let Some(phase) = self.shell.auth.as_ref().map(|state| state.phase) else {
            return;
        };
        if matches!(phase, Phase::Choose | Phase::Saving | Phase::Saved)
            || phase == Phase::Complete
                && self
                    .shell
                    .auth
                    .as_ref()
                    .is_none_or(|state| state.provider != AccountProvider::Offline)
        {
            return;
        }
        self.abandon_auth_attempt();
        if let Some(state) = &mut self.shell.auth {
            state.browser_url = None;
            state.device_code = None;
            state.stage = None;
            state.prepared_account = None;
            state.error = None;
            state.transition(if phase == Phase::Information {
                Phase::Choose
            } else {
                Phase::Information
            });
        }
        cx.notify();
    }

    pub(super) fn change_auth_method(
        &mut self,
        method: Method,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.shell.auth.as_ref().is_none_or(|state| {
            state.provider != AccountProvider::Microsoft
                || state.method == method
                || matches!(
                    state.phase,
                    Phase::Choose | Phase::Complete | Phase::Saving | Phase::Saved
                )
        }) {
            return;
        }
        self.abandon_auth_attempt();
        if let Some(state) = &mut self.shell.auth {
            state.method = method;
            state.browser_url = None;
            state.device_code = None;
            state.stage = None;
            state.prepared_account = None;
            state.error = None;
            state.transition(Phase::Information);
        }
        if method == Method::Device {
            self.start_auth(window, cx);
        }
        cx.notify();
    }

    pub(super) fn auth_device_code(
        &mut self,
        user_code: String,
        verification_uri: String,
        expires_at: std::time::Instant,
    ) {
        if let Some(state) = &mut self.shell.auth
            && state.phase.pending()
            && state.method == Method::Device
            && state
                .attempt
                .as_ref()
                .is_some_and(|(_, cancel)| !cancel.is_cancelled())
        {
            state.browser_url = Some(verification_uri.clone());
            state.device_code = Some(DeviceCode {
                user_code,
                verification_uri,
                expires_at,
            });
            state.transition(Phase::Waiting);
        }
    }

    pub(super) fn copy_device_code(&self, cx: &mut Context<'_, Self>) {
        if let Some(state) = &self.shell.auth
            && state.phase == Phase::Waiting
            && state
                .attempt
                .as_ref()
                .is_some_and(|(_, cancel)| !cancel.is_cancelled())
            && let Some(code) = &state.device_code
            && code.expires_at > std::time::Instant::now()
        {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(code.user_code.clone()));
        }
    }

    pub(super) fn open_auth_wizard(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.shell.auth.is_some() {
            self.shell
                .surfaces
                .open(super::shell::Surface::Auth, window, cx);
            return;
        }
        if self.busy {
            return;
        }
        let focus = cx.focus_handle();
        let wizard = AuthWizard {
            username: Some(super::input("", "Your player name", cx)),
            return_tab: self.tab,
            return_size: window.viewport_size(),
            focus: Some(focus.clone()),
            ..AuthWizard::default()
        };
        self.shell.auth = Some(wizard);
        self.shell
            .surfaces
            .open(super::shell::Surface::Auth, window, cx);
        cx.notify();
    }

    pub(super) fn start_auth(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.busy
            || self.shell.auth.as_ref().is_none_or(|state| {
                matches!(state.phase, Phase::Complete | Phase::Saving | Phase::Saved)
            })
        {
            return;
        }
        if self
            .shell
            .auth
            .as_ref()
            .is_some_and(|state| state.provider == AccountProvider::Offline)
        {
            let name = self
                .shell
                .auth
                .as_ref()
                .and_then(|state| state.username.as_ref())
                .map_or_else(String::new, |field| {
                    field.read(cx).value().trim().to_owned()
                });
            let result = super::engine::offline_account(&name);
            if let Some(state) = &mut self.shell.auth {
                match result {
                    Ok(account) => {
                        state.prepared_account = Some(account);
                        state.error = None;
                        state.transition(Phase::Complete);
                    }
                    Err(error) => {
                        state.error = Some(error.to_string());
                        state.transition(Phase::Failed);
                    }
                }
            }
            cx.notify();
            return;
        }
        self.abandon_auth_attempt();
        self.begin_cancellable("Preparing secure Microsoft sign-in");
        self.operation_kind = super::OperationKind::Account;
        let Some(state) = &mut self.shell.auth else {
            return;
        };
        state.browser_url = None;
        state.device_code = None;
        state.attempt = Some((self.operation_revision, self.cancel.clone()));
        state.stage = None;
        state.prepared_account = None;
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
            engine.sign_in(self.cancel.clone(), state.method == Method::Device);
        }
        cx.notify();
    }

    pub(super) fn auth_browser(&mut self, url: String, cx: &Context<'_, Self>) {
        if self.shell.auth.as_ref().is_none_or(|state| {
            state.provider != AccountProvider::Microsoft
                || state.method != Method::Browser
                || !state.phase.pending()
                || state
                    .attempt
                    .as_ref()
                    .is_none_or(|(_, cancel)| cancel.is_cancelled())
        }) {
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
        if state.provider != AccountProvider::Microsoft {
            return;
        }
        if state
            .attempt
            .as_ref()
            .is_some_and(|(_, cancel)| cancel.is_cancelled())
            || (!state.phase.pending() && state.phase != Phase::Saving)
            || state.stage.is_some_and(|previous| previous > progress)
        {
            return;
        }
        state.stage = Some(progress);
        if progress >= SignInStage::MicrosoftAuthorization && state.phase != Phase::Saving {
            state.browser_url = None;
            state.device_code = None;
            state.transition(Phase::Verifying);
        }
        self.status = auth_stage_label(progress).into();
    }

    pub(super) fn reopen_auth(&self, cx: &Context<'_, Self>) {
        if let Some(state) = &self.shell.auth
            && state.phase == Phase::Waiting
            && state
                .attempt
                .as_ref()
                .is_some_and(|(_, cancel)| !cancel.is_cancelled())
            && let Some(url) = &state.browser_url
        {
            cx.open_url(url);
        }
    }

    /// Authentication is verified, but nothing is persisted until Finish.
    pub(super) fn auth_prepared(&mut self, account: Account) {
        if let Some(state) = &mut self.shell.auth
            && state.provider == AccountProvider::Microsoft
            && state.phase.pending()
            && state
                .attempt
                .as_ref()
                .is_some_and(|(_, cancel)| !cancel.is_cancelled())
        {
            state.browser_url = None;
            state.device_code = None;
            state.prepared_account = Some(account);
            state.error = None;
            state.transition(Phase::Complete);
        }
    }

    pub(super) fn auth_finished(&mut self, result: &Result<String, String>) {
        if let Some(state) = &mut self.shell.auth
            && (state.phase.pending() || state.phase == Phase::Saving)
        {
            match result {
                Err(error) => state.failed(error),
                Ok(_) => state.failed("Sign-in ended without an identity confirmation"),
            }
        }
    }

    pub(super) fn cancel_auth(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let committing = self
            .shell
            .auth
            .as_ref()
            .is_some_and(|state| matches!(state.phase, Phase::Saving | Phase::Saved));
        // Finish authorizes a durable save. Closing its window does not cancel
        // that transaction or a different operation started in the parent.
        if !committing {
            self.abandon_auth_attempt();
        }
        self.close_auth_context(window, cx);
    }

    pub(super) fn finish_auth(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.busy {
            return;
        }
        let account = self
            .shell
            .auth
            .as_ref()
            .filter(|state| state.phase == Phase::Complete)
            .and_then(|state| state.prepared_account.clone());
        let Some(account) = account else {
            return;
        };
        if self.engine.is_none() {
            if let Some(state) = &mut self.shell.auth {
                state.failed("Secure account saving is unavailable: engine unavailable");
            }
            cx.notify();
            return;
        }
        self.begin("Saving your account securely");
        self.operation_kind = super::OperationKind::Account;
        if let Some(state) = &mut self.shell.auth {
            state.transition(Phase::Saving);
            state.attempt = Some((self.operation_revision, self.cancel.clone()));
        }
        if let Some(engine) = &self.engine {
            if account.provider == AccountProvider::Offline {
                engine.save_offline_account(account);
            } else {
                engine.save_sign_in(account.reference);
            }
        }
        cx.notify();
    }

    pub(super) fn auth_saved(&mut self, account: Account, cx: &mut Context<'_, Self>) {
        let Some(state) = &mut self.shell.auth else {
            return;
        };
        if state.phase != Phase::Saving
            || state
                .prepared_account
                .as_ref()
                .is_none_or(|prepared| prepared.reference != account.reference)
        {
            return;
        }
        state.prepared_account = Some(account);
        state.transition(Phase::Saved);
        let handle = self
            .shell
            .surfaces
            .layers()
            .find(|entry| matches!(entry.kind, super::shell::Surface::Auth))
            .and_then(|entry| entry.window);
        if let Some(handle) = handle {
            let owner = cx.entity();
            cx.defer(move |cx| {
                let _ = handle.update(cx, |_, window, cx| {
                    owner.update(cx, |view, cx| view.finish_saved_auth(window, cx));
                });
            });
        }
    }

    fn finish_saved_auth(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let account = self
            .shell
            .auth
            .as_ref()
            .filter(|state| state.phase == Phase::Saved)
            .and_then(|state| state.prepared_account.clone());
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
        self.shell.surfaces.activate(window);
        self.close_auth_context(window, cx);
    }

    fn close_auth_context(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.shell.auth.take().is_none() {
            return;
        }
        if self
            .shell
            .surfaces
            .top()
            .is_some_and(|entry| matches!(entry.kind, super::shell::Surface::Auth))
        {
            self.shell.surfaces.close(window, &self.focus, cx);
        }
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
                if event.keystroke.key == "escape" && !view.shell.surfaces.is_open() {
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
            (
                "Sign in",
                if state.provider == AccountProvider::Offline {
                    "Choose a local player name"
                } else {
                    "Continue with Microsoft"
                },
            ),
            (
                "Verification",
                if state.provider == AccountProvider::Offline {
                    "Validate the local identity"
                } else {
                    "Ownership and player profile"
                },
            ),
            ("Complete", "Review and save your account"),
        ]
        .map(|(title, description)| components::StepperStep {
            title,
            description,
            complete: match title {
                "Account type" => state.phase != Phase::Choose,
                "Sign in" => matches!(state.phase, Phase::Verifying | Phase::Complete),
                "Verification" => {
                    matches!(state.phase, Phase::Complete | Phase::Saving | Phase::Saved)
                }
                "Complete" => state.phase == Phase::Saved,
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
            Phase::Information => (
                if state.provider == AccountProvider::Offline {
                    "Your local player"
                } else {
                    "Sign in with Microsoft"
                },
                if state.provider == AccountProvider::Offline {
                    "Choose a name for local play without an authentication service."
                } else {
                    "Choose browser sign-in or enter a device code on another device."
                },
            ),
            Phase::Preparing => (
                "Preparing secure sign-in",
                "Block is requesting a real Microsoft authorization session.",
            ),
            Phase::Waiting if state.method == Method::Device => (
                "Approve your device code",
                "Open Microsoft's website on this or another device and enter the code below.",
            ),
            Phase::Waiting => (
                "Continue in your browser",
                "Complete Microsoft sign-in and return here. This window can stay open.",
            ),
            Phase::Verifying => (
                "Verifying your account",
                "Block is checking Java Edition ownership and resolving your player profile.",
            ),
            Phase::Failed => (
                "Sign-in needs your attention",
                "Your existing saved accounts are kept. You can try again.",
            ),
            Phase::Complete => (
                "Your account is ready",
                if state.provider == AccountProvider::Offline {
                    "Your local identity is valid. Finish saves it; closing discards this preparation."
                } else {
                    "Your identity is verified. Finish saves this account securely; closing discards this preparation."
                },
            ),
            Phase::Saving => (
                "Saving your account",
                if state.provider == AccountProvider::Offline {
                    "Block is saving the local identity. This finishes even if you close the window."
                } else {
                    "Block is saving the refresh credential and account metadata. This finishes even if you close the window."
                },
            ),
            Phase::Saved => (
                "Account saved",
                "Your account is saved securely. Returning to the launcher.",
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
            Phase::Choose => body.child(self.auth_choices(state, cx)),
            Phase::Information => body.child(self.auth_information(state, cx)),
            Phase::Preparing | Phase::Waiting => body
                .child(self.auth_information(state, cx))
                .child(self.auth_waiting(state, cx)),
            Phase::Verifying => body.child(self.auth_waiting(state, cx)),
            Phase::Failed => body.child(self.auth_information(state, cx)).child(
                components::card(self.palette)
                    .border_color(rgb(self.palette.destructive))
                    .child(Icon::Error.render(24., self.palette.destructive))
                    .child(note(state.error.clone().unwrap_or_else(|| {
                        "Sign-in could not finish. Try again.".into()
                    }))),
            ),
            Phase::Complete | Phase::Saved => body.child(self.auth_complete(state)),
            Phase::Saving => body.child(components::progress(
                ProgressState::Indeterminate,
                self.palette,
            )),
        };
        body.children(
            (self.busy
                && !state.phase.pending()
                && !matches!(state.phase, Phase::Saving | Phase::Saved))
            .then(|| {
                note("Finish the current launcher task before starting or saving this sign-in.")
            }),
        )
    }

    fn auth_choices(&self, state: &AuthWizard, cx: &Context<'_, Self>) -> gpui::Div {
        let mut choices = div().flex().flex_col().gap_4()
            .child(components::card(self.palette)
                .child(components::radio("auth-microsoft", "Microsoft account", state.provider == AccountProvider::Microsoft, self.palette,
                    ControlState::default(), cx.listener(|view, (), window, cx| view.handle(UiAction::AuthProvider(AccountProvider::Microsoft), window, cx))))
                .child(note("Sign in securely in your browser. Block never asks for your Microsoft password.")));
        choices = choices.child(
            components::card(self.palette)
                .gap_2()
                .child(components::button(
                    "auth-authlib-unavailable",
                    "Authlib-Injector",
                    self.palette,
                    ButtonVariant::Ghost,
                    ControlState {
                        disabled: true,
                        ..ControlState::default()
                    },
                    |(), _, _| {},
                ))
                .child(note(
                    "Provider authentication is not supported in this version.",
                )),
        );
        choices.child(components::card(self.palette)
            .child(components::radio("auth-offline", "Offline Mode", state.provider == AccountProvider::Offline, self.palette,
                ControlState::default(), cx.listener(|view, (), window, cx| view.handle(UiAction::AuthProvider(AccountProvider::Offline), window, cx))))
            .child(note("A local player identity. Does not grant access to authenticated online services.")))
            .child(note("Only Finish saves the account on this device."))
    }

    fn auth_information(&self, state: &AuthWizard, cx: &Context<'_, Self>) -> gpui::Div {
        if state.provider == AccountProvider::Offline {
            let mut card = components::card(self.palette)
                .child(note("An offline identity for local play, without signing in to an authentication service."));
            if let Some(field) = &state.username {
                card = card.child(components::field_with(
                    "Player name",
                    field,
                    self.palette,
                    components::FieldOptions {
                        required: true,
                        ..components::FieldOptions::default()
                    },
                ));
            }
            return card.child(note(
                "Use 1–16 ASCII letters, numbers, or underscores. Names are case-sensitive.",
            ));
        }
        components::card(self.palette)
            .child(note("Block never asks for your Microsoft password. Authorize only on Microsoft's website."))
            .child(div().flex().gap_3()
                .child(components::radio("auth-method-browser", "Browser", state.method == Method::Browser, self.palette,
                    ControlState::default(), cx.listener(|view, (), window, cx| view.handle(UiAction::AuthMethod(Method::Browser), window, cx))))
                .child(components::radio("auth-method-device", "Device Code", state.method == Method::Device, self.palette,
                    ControlState::default(), cx.listener(|view, (), window, cx| view.handle(UiAction::AuthMethod(Method::Device), window, cx)))))
    }

    fn auth_waiting(&self, state: &AuthWizard, cx: &Context<'_, Self>) -> gpui::Div {
        if state.method == Method::Device
            && matches!(state.phase, Phase::Preparing | Phase::Waiting)
        {
            let mut card = components::card(self.palette).gap_3();
            if let Some(code) = &state.device_code {
                card = card.child(note("Enter this short-lived code on Microsoft's website:"))
                    .child(div().flex().items_center().justify_between().gap_3()
                        .child(div().id("auth-device-code").text_2xl().child(code.user_code.clone()))
                        .child(self.auth_button("auth-copy-code", "Copy code", UiAction::CopyDeviceCode, ButtonVariant::Outline,
                            ControlState { disabled: code.expires_at <= std::time::Instant::now(), ..ControlState::default() }, cx)))
                    .child(div().flex().items_center().justify_between().gap_3()
                        .child(div().flex_1().min_w_0().text_xs().truncate().child(code.verification_uri.clone()))
                        .child(self.auth_button("auth-reopen", "Open Microsoft page", UiAction::AuthReopen, ButtonVariant::Outline, ControlState::default(), cx)))
                    .child(note("Keep this window open. Verification begins when Microsoft approves the request."));
            } else {
                card = card.child(note("Requesting your device code…"));
            }
            return card.child(components::progress(
                ProgressState::Indeterminate,
                self.palette,
            ));
        }
        let mut card = components::card(self.palette)
            .child(Icon::ExternalLink.render(32., self.palette.foreground))
            .child(note("After authorization, Block verifies Java Edition ownership and resolves your player profile. You review the identity before saving it."))
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
        let Some(account) = &state.prepared_account else {
            return components::card(self.palette).child(note("Waiting for a verified identity."));
        };
        components::card(self.palette)
            .child(components::avatar_fallback(&account.player_name, self.palette, 64., AvatarState::Fallback))
            .child(div().text_xl().child(account.player_name.clone()))
            .child(components::badge(account.provider.label(), self.palette, BadgeVariant::Secondary))
            .child(components::badge(if state.phase == Phase::Saved { "Account saved" } else { "Ready to save" }, self.palette, BadgeVariant::Success))
            .child(note(if state.phase == Phase::Saved { "The account has been saved and is ready to use." } else { "Finish saves this account and selects it. Closing now discards the preparation and keeps previously saved accounts unchanged." }))
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
        footer = footer.child(
            div()
                .flex()
                .gap_2()
                .child(self.auth_button(
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
                ))
                .children(
                    (!matches!(
                        state.phase,
                        Phase::Choose | Phase::Complete | Phase::Saving | Phase::Saved
                    ) || state.provider == AccountProvider::Offline
                        && state.phase == Phase::Complete)
                        .then(|| {
                            self.auth_button(
                                "auth-back",
                                "Back",
                                UiAction::AuthBack,
                                ButtonVariant::Ghost,
                                ControlState::default(),
                                cx,
                            )
                        }),
                ),
        );
        let (label, action, busy) = match state.phase {
            Phase::Choose => ("Continue", UiAction::AuthNext, false),
            Phase::Information => (
                if state.provider == AccountProvider::Offline {
                    "Continue"
                } else if state.method == Method::Device {
                    "Request a code"
                } else {
                    "Open browser"
                },
                UiAction::Authenticate,
                false,
            ),
            Phase::Preparing | Phase::Waiting | Phase::Verifying => {
                ("Waiting…", UiAction::Authenticate, true)
            }
            Phase::Failed => ("Try again", UiAction::AuthRetry, false),
            Phase::Complete => ("Finish", UiAction::AuthFinish, false),
            Phase::Saving => ("Saving…", UiAction::AuthFinish, true),
            Phase::Saved => ("Done", UiAction::AuthCancel, false),
        };
        footer.child(self.auth_button(
            "auth-primary",
            label,
            action,
            ButtonVariant::Primary,
            ControlState {
                busy,
                disabled: self.busy
                    && matches!(
                        state.phase,
                        Phase::Choose | Phase::Information | Phase::Failed | Phase::Complete
                    ),
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
    } else if lower.contains("ownership")
        || lower.contains("does not own")
        || lower.contains("entitlement")
    {
        "Minecraft: Java Edition ownership could not be verified. Check the Microsoft account and try again.".into()
    } else if lower.contains("no minecraft: java edition player profile") {
        "Complete your Minecraft: Java Edition player profile at minecraft.net, then try again."
            .into()
    } else if lower.contains("create an xbox profile") {
        "Create an Xbox profile for this Microsoft account, then try again.".into()
    } else if lower.contains("family restrictions") || lower.contains("xbox account permissions") {
        "Xbox account permissions or family restrictions prevent this sign-in. Check the account's Xbox settings.".into()
    } else if lower.contains("different microsoft application registration") {
        "This player is saved under another Microsoft application registration. Remove that saved account first, then sign in again.".into()
    } else if lower.contains("application registration") || lower.contains("client id") {
        "The Microsoft application rejected this sign-in method. Check its public-client and personal-account settings; browser sign-in also needs its desktop redirect URI.".into()
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
        SignInStage::DeviceAuthorization => "Requesting a Microsoft device code",
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
        let bounds = crate::test_support::control_bounds(cx, selector);
        assert!(bounds.is_some(), "missing {selector}");
        if let Some(bounds) = bounds {
            let window_count = cx.windows().len();
            cx.simulate_click(bounds.center(), gpui::Modifiers::none());
            cx.run_until_parked();
            if cx.windows().len() > window_count
                && let Some(window) = cx.windows().last()
            {
                *cx = gpui::VisualTestContext::from_window(*window, &cx.cx);
            }
            crate::test_support::ensure_live_window(cx);
            cx.run_until_parked();
            crate::test_support::ensure_live_window(cx);
        }
    }

    fn simulate_started_attempt(view: &mut crate::Block, phase: Phase) {
        view.cancel = block_launcher::CancellationToken::new();
        view.operation_revision = view.operation_revision.wrapping_add(1);
        if let Some(state) = &mut view.shell.auth {
            state.attempt = Some((view.operation_revision, view.cancel.clone()));
            state.transition(phase);
        }
        view.busy = true;
    }

    #[gpui::test]
    fn offline_preparation_validates_edits_and_cancel_preserves_saved_accounts(
        cx: &mut TestAppContext,
    ) {
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.open_auth_wizard(window, cx);
                view.choose_auth_provider(block_auth::AccountProvider::Offline, cx);
                view.auth_next(cx);
                view.start_auth(window, cx);
                assert!(view.shell.auth.as_ref().is_some_and(
                    |state| state.phase == Phase::Failed && state.prepared_account.is_none()
                ));
                let field = view
                    .shell
                    .auth
                    .as_ref()
                    .and_then(|state| state.username.clone());
                if let Some(field) = &field {
                    field.update(cx, |field, cx| field.set_value("  Local_Player  ", cx));
                }
                view.start_auth(window, cx);
                assert!(view.shell.auth.as_ref().is_some_and(|state| {
                    state.phase == Phase::Complete
                        && state
                            .prepared_account
                            .as_ref()
                            .is_some_and(|account| account.player_name == "Local_Player")
                }));
                assert!(view.accounts.is_empty() && !view.busy);
                view.auth_back(cx);
                assert!(
                    view.shell
                        .auth
                        .as_ref()
                        .is_some_and(|state| state.phase == Phase::Information
                            && state.prepared_account.is_none())
                );
                if let Some(field) = &field {
                    field.update(cx, |field, cx| field.set_value("Another_Name", cx));
                }
                view.start_auth(window, cx);
                assert!(
                    view.shell
                        .auth
                        .as_ref()
                        .and_then(|state| state.prepared_account.as_ref())
                        .is_some_and(|account| account.player_name == "Another_Name")
                );
                view.cancel_auth(window, cx);
                assert!(
                    view.shell.auth.is_none()
                        && view.accounts.is_empty()
                        && view.selected_account.is_none()
                );
            });
        });
        assert!(cx.opened_url().is_none());
    }

    #[test]
    fn debug_output_excludes_authorization_urls_and_error_details() {
        let state = AuthWizard {
            browser_url: Some("https://example.invalid/?state=sensitive-auth-state".into()),
            device_code: Some(super::DeviceCode {
                user_code: "sensitive-device-code".into(),
                verification_uri: "https://example.invalid/sensitive".into(),
                expires_at: std::time::Instant::now(),
            }),
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
                    simulate_started_attempt(view, Phase::Waiting);
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
                    view.auth_prepared(account.clone());
                    view.auth_finished(&Ok("late completion".into()));
                });
            });
            assert!(view.read_with(cx, |view, _| {
                view.shell.auth.as_ref().is_some_and(|state| {
                    state.phase == Phase::Choose && state.prepared_account.is_none()
                }) && view.accounts.len() == 1
            }));
            Ok(())
        })();
        assert!(result.is_ok(), "{result:?}");
    }

    #[gpui::test]
    fn a_generic_success_message_cannot_fabricate_a_prepared_account(cx: &mut TestAppContext) {
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
                    && state.prepared_account.is_none()
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
            simulate_started_attempt(view, Phase::Preparing);
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
    fn prepared_identity_waits_for_durable_confirmation_and_keeps_the_game_draft(
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
                    simulate_started_attempt(view, Phase::Waiting);
                    // This is a synthetic unit event, not evidence of real OAuth.
                    view.accounts.clear();
                    view.auth_prepared(account.clone());
                    view.auth_finished(&Ok("Identity prepared".into()));
                    view.busy = false;
                    cx.notify();
                });
            });
            assert!(view.read_with(cx, |view, _| {
                view.shell
                    .auth
                    .as_ref()
                    .is_some_and(|state| state.phase == Phase::Complete)
            }));
            assert!(view.read_with(cx, |view, _| view.accounts.is_empty()
                && view.selected_account.is_none()));
            // Model the authorized save's durable events, not a live OAuth pass.
            view.update(cx, |view, cx| {
                if let Some(state) = &mut view.shell.auth {
                    state.transition(Phase::Saving);
                }
                view.accounts = vec![account.clone()];
                view.auth_saved(account.clone(), cx);
            });
            cx.run_until_parked();
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
    fn closing_a_prepared_account_before_finished_keeps_the_returned_game_draft(
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
                    simulate_started_attempt(view, Phase::Waiting);
                    view.busy = true;
                    view.accounts.clear();
                    view.auth_prepared(account);
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
                && view.accounts.is_empty()
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
                            prepared_account: (phase == Phase::Complete).then(|| account.clone()),
                            ..AuthWizard::default()
                        });
                        view.shell
                            .surfaces
                            .open(crate::shell::Surface::Auth, window, cx);
                        view
                    });
                    crate::test_support::control_bounds(cx, "auth-body");
                    cx.simulate_resize(size(px(width), px(height)));
                    cx.run_until_parked();
                    let body = crate::test_support::control_bounds(cx, "auth-body");
                    let footer = crate::test_support::control_bounds(cx, "auth-footer");
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
                            && state.prepared_account.is_none())
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
                    |state| state.phase == Phase::Failed && state.prepared_account.is_none()
                ));
                view.cancel_auth(window, cx);
                view.auth_stage(block_auth::SignInStage::SaveMetadata);
                assert!(view.shell.auth.is_none());
            });
        });
    }

    #[gpui::test]
    fn closing_a_prepared_identity_never_cancels_an_unrelated_parent_operation(
        cx: &mut TestAppContext,
    ) {
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        let parent_cancel = block_launcher::CancellationToken::new();
        let auth_cancel = block_launcher::CancellationToken::new();
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.open_auth_wizard(window, cx);
                if let Some(state) = &mut view.shell.auth {
                    state.transition(Phase::Complete);
                    state.attempt = Some((1, auth_cancel.clone()));
                }
                view.operation_revision = 2;
                view.cancel = parent_cancel.clone();
                view.busy = true;
                view.cancel_auth(window, cx);
            });
        });
        assert!(auth_cancel.is_cancelled());
        assert!(!parent_cancel.is_cancelled());
        assert!(view.read_with(cx, |view, _| view.busy
            && view.operation_revision == 2
            && view.shell.auth.is_none()));
    }

    #[gpui::test]
    fn switching_device_code_to_browser_discards_codes_and_rejects_late_identity_events(
        cx: &mut TestAppContext,
    ) {
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.open_auth_wizard(window, cx);
                simulate_started_attempt(view, Phase::Preparing);
                if let Some(state) = &mut view.shell.auth {
                    state.method = super::Method::Device;
                }
                view.auth_device_code(
                    "ABCD-EFGH".into(),
                    "https://microsoft.com/devicelogin".into(),
                    std::time::Instant::now() + std::time::Duration::from_secs(60),
                );
                assert!(view.shell.auth.as_ref().is_some_and(
                    |state| state.device_code.is_some() && state.phase == Phase::Waiting
                ));
                let previous = view.cancel.clone();
                view.change_auth_method(super::Method::Browser, window, cx);
                assert!(previous.is_cancelled());
                view.auth_device_code(
                    "LATE-CODE".into(),
                    "https://microsoft.com/devicelogin".into(),
                    std::time::Instant::now() + std::time::Duration::from_secs(60),
                );
                let account = test_account();
                assert!(account.is_ok());
                if let Ok(account) = account {
                    view.auth_prepared(account);
                }
            });
        });
        assert!(
            view.read_with(cx, |view, _| view.shell.auth.as_ref().is_some_and(
                |state| state.phase == Phase::Information
                    && state.method == super::Method::Browser
                    && state.device_code.is_none()
                    && state.prepared_account.is_none()
            ) && view.accounts.is_empty())
        );
        assert_eq!(cx.opened_url(), None);
    }

    #[gpui::test]
    fn final_confirmation_without_a_backend_cannot_claim_to_save_an_identity(
        cx: &mut TestAppContext,
    ) {
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.open_auth_wizard(window, cx);
                let account = test_account();
                assert!(account.is_ok());
                if let Ok(account) = account
                    && let Some(state) = &mut view.shell.auth
                {
                    state.prepared_account = Some(account);
                    state.transition(Phase::Complete);
                }
                view.finish_auth(window, cx);
            });
        });
        assert!(view.read_with(cx, |view, _| {
            view.accounts.is_empty()
                && view.selected_account.is_none()
                && view
                    .shell
                    .auth
                    .as_ref()
                    .is_some_and(|state| state.phase == Phase::Failed)
        }));
    }
    #[gpui::test]
    fn a_save_finishing_after_its_window_closed_preserves_the_parent_game_draft(
        cx: &mut TestAppContext,
    ) {
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        let (sender, receiver) = tokio::sync::mpsc::channel(2);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.begin_game_wizard(cx);
                view.open_auth_wizard(window, cx);
                if let Some(state) = &mut view.shell.auth {
                    state.transition(Phase::Saving);
                }
                view.busy = true;
                view.operation_kind = crate::OperationKind::Account;
                view.messages = Some(receiver);
                view.cancel_auth(window, cx);
            });
        });
        assert!(view.read_with(cx, |view, _| view.busy
            && view.form.is_some()
            && view.shell.auth.is_none()));
        let account = test_account();
        assert!(account.is_ok());
        if let Ok(account) = account {
            assert!(
                sender
                    .try_send(crate::engine::Message::Accounts(Ok(vec![account])))
                    .is_ok()
            );
        }
        assert!(
            sender
                .try_send(crate::engine::Message::Finished(Ok("Account saved".into())))
                .is_ok()
        );
        view.update(cx, crate::Block::poll);
        assert!(view.read_with(cx, |view, _| !view.busy
            && view.form.is_some()
            && view.accounts.len() == 1
            && view.shell.auth.is_none()));
    }
}
