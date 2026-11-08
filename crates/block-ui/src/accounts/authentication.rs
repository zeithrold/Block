//! Account preparation and its single explicit save.
//!
//! Authorization URLs and device codes are transient and never included in
//! `Debug` output.

#![expect(
    clippy::redundant_pub_crate,
    reason = "Authentication presentation is shared only with the parent view and shell."
)]

use super::scroll_area::ScrollAreaExt as _;
use std::fmt;

use block_auth::{Account, AccountProvider, SignInStage};
use gpui::{
    AppContext as _, Context, FocusHandle, Focusable as _, InteractiveElement as _, IntoElement,
    ParentElement as _, ScrollHandle, StatefulInteractiveElement as _, Styled as _, Window, div,
    point, px, rgb,
};

use super::{
    Block, Language, Tab, UiAction, account_id,
    components::{self, AvatarState, BadgeVariant, ButtonVariant, ControlState, ProgressState},
    design::Icon,
    note, patterns,
};

/// The account page's single reading column. Its footer aligns to the same width
/// and horizontal padding, so the page reads as one column at any window size.
const AUTH_CONTENT_WIDTH: f32 = 328.;

/// Microsoft owns most Java Edition accounts and is therefore the page's
/// default; the other types follow it in the alternatives list.
const ACCOUNT_TYPES: [AccountProvider; 3] = [
    AccountProvider::Microsoft,
    AccountProvider::Authlib,
    AccountProvider::Offline,
];

/// A retained account-surface message.
///
/// Product text keeps its semantic key, so every render resolves it in the
/// view's current language, including after a language change. Diagnostics from
/// the authentication backend stay verbatim.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AccountMessage {
    Product(&'static str),
    Detail(&'static str, String),
    Diagnostic(String),
}

impl AccountMessage {
    pub(crate) const fn product(key: &'static str) -> Self {
        Self::Product(key)
    }

    pub(crate) fn diagnostic(text: impl Into<String>) -> Self {
        Self::Diagnostic(text.into())
    }

    pub(crate) fn detail(key: &'static str, text: impl Into<String>) -> Self {
        Self::Detail(key, text.into())
    }

    pub(crate) fn render(&self, language: Language) -> String {
        match self {
            Self::Product(key) => tr!(language, *key),
            Self::Detail(key, detail) => tr!(language, *key, detail = detail),
            Self::Diagnostic(text) => tr!(language, "common.diagnostic_detail", detail = text),
        }
    }
}

#[derive(IntoElement)]
struct ProviderPassword {
    state: gpui::Entity<gpui_base::input::InputState>,
    palette: super::Palette,
}

impl gpui::RenderOnce for ProviderPassword {
    fn render(self, window: &mut Window, cx: &mut gpui::App) -> impl IntoElement {
        let palette = self.palette;
        let focused = self.state.read(cx).focus_handle(cx).is_focused(window);
        self.state.update(cx, |state, _| {
            state.set_editor_style(gpui_base::input::InputEditorStyle {
                foreground: rgb(palette.foreground).into(),
                muted_foreground: rgb(palette.muted).into(),
                background: rgb(palette.background).into(),
                border: rgb(palette.input).into(),
                caret: rgb(palette.foreground).into(),
                selection: gpui::rgba(0x62bd_a855).into(),
                ..Default::default()
            })
        });
        div()
            .id("auth-password")
            .h(px(super::design::CONTROL_HEIGHT))
            .w_full()
            .px_3()
            .py(px((super::design::CONTROL_HEIGHT
                - super::design::BODY_LINE_HEIGHT)
                / 2.))
            .text_size(px(super::design::BODY_FONT_SIZE))
            .line_height(px(super::design::BODY_LINE_HEIGHT))
            .rounded(px(super::design::CONTROL_RADIUS))
            .border_1()
            .border_color(rgb(if focused { palette.ring } else { palette.input }))
            .bg(rgb(palette.background))
            .shadow(if focused {
                super::design::focus_ring(palette.ring)
            } else {
                Vec::new()
            })
            .child(gpui_base::input::Input::new(&self.state))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Phase {
    /// The account page itself: the sign-in form for the selected account type.
    #[default]
    Connect,
    Preparing,
    Waiting,
    Verifying,
    Characters,
    Failed,
    Complete,
    Saving,
    Saved,
}

/// The identity the shared page host animates the account page against.
///
/// It is a motion key only: the page reports progress in place and keeps no
/// numbered step list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AuthStep {
    Choose,
    SignIn,
    Verification,
    Complete,
}

impl Phase {
    /// The page identity motion and content switching use.
    ///
    /// Waiting and verification belong to the page that started the attempt,
    /// so they keep that page's identity instead of becoming steps of their own.
    const fn step(self) -> AuthStep {
        match self {
            Self::Connect => AuthStep::Choose,
            Self::Preparing | Self::Waiting | Self::Failed => AuthStep::SignIn,
            Self::Verifying | Self::Characters => AuthStep::Verification,
            Self::Complete | Self::Saving | Self::Saved => AuthStep::Complete,
        }
    }

    const fn pending(self) -> bool {
        matches!(self, Self::Preparing | Self::Waiting | Self::Verifying)
    }

    /// Whether an exit can still revoke the attempt.
    ///
    /// A pending save is an authorized write: it finishes on its own, so the
    /// page offers no exit that would read as cancelling it.
    const fn committed(self) -> bool {
        matches!(self, Self::Saving | Self::Saved)
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

#[derive(Debug)]
pub(crate) struct AuthProviderDraft {
    id: uuid::Uuid,
    pub(crate) name: gpui::Entity<super::TextInput>,
    pub(crate) address: gpui::Entity<super::TextInput>,
    pub(crate) observed_address: String,
    pub(crate) accepted_http: Option<String>,
    pub(crate) error: Option<AccountMessage>,
    attempt: Option<(u64, block_launcher::CancellationToken)>,
    saved: Option<uuid::Uuid>,
    scroll: ScrollHandle,
}

impl AuthWizard {
    pub(super) fn refresh_placeholders(
        &self,
        language: Language,
        window: &mut Window,
        cx: &mut Context<'_, Block>,
    ) {
        for (input, key) in [
            (
                self.username.as_ref(),
                "accounts.auth.offline.name.placeholder",
            ),
            (self.login.as_ref(), "accounts.auth.login.placeholder"),
        ] {
            if let Some(input) = input {
                let placeholder = tr!(language, key);
                input.update(cx, |input, cx| input.set_placeholder(&placeholder, cx));
            }
        }
        if let Some(password) = &self.password {
            let placeholder = tr!(language, "accounts.auth.password.placeholder");
            password.update(cx, |input, cx| {
                input.set_placeholder(placeholder, window, cx)
            });
        }
        if let Some(draft) = &self.provider_draft {
            for (input, key) in [
                (&draft.name, "accounts.auth.provider_add.name.placeholder"),
                (
                    &draft.address,
                    "accounts.auth.provider_add.address.placeholder",
                ),
            ] {
                let placeholder = tr!(language, key);
                input.update(cx, |input, cx| input.set_placeholder(&placeholder, cx));
            }
        }
    }

    /// Whether the Add-provider page currently owns a draft (drop-zone lifetime).
    pub(super) fn is_provider_drafting(&self) -> bool {
        self.provider_draft.is_some()
    }

    /// Mutable access to the draft for drop-payload handoff.
    pub(super) fn provider_draft_mut(&mut self) -> Option<&mut AuthProviderDraft> {
        self.provider_draft.as_mut()
    }
}

#[derive(Default)]
pub(super) struct AuthWizard {
    phase: Phase,
    provider: AccountProvider,
    pub(super) username: Option<gpui::Entity<super::TextInput>>,
    login: Option<gpui::Entity<super::TextInput>>,
    password: Option<gpui::Entity<gpui_base::input::InputState>>,
    external_provider: Option<uuid::Uuid>,
    external_api: Option<String>,
    accepted_http: Option<String>,
    characters: Vec<block_auth::AuthlibProfile>,
    character: Option<uuid::Uuid>,
    provider_draft: Option<AuthProviderDraft>,
    method: Method,
    device_code: Option<DeviceCode>,
    attempt: Option<(u64, block_launcher::CancellationToken)>,
    stage: Option<SignInStage>,
    revision: usize,
    other_sign_in_options: bool,
    browser_url: Option<String>,
    prepared_account: Option<Account>,
    error: Option<AccountMessage>,
    /// The last attempt failed while persisting an already verified identity.
    /// Its credential is spent, so recovery starts a fresh sign-in.
    save_failed: bool,
    return_window: Option<gpui::AnyWindowHandle>,
    focus: Option<FocusHandle>,
    body_scroll: ScrollHandle,
}

impl fmt::Debug for AuthWizard {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthWizard")
            .field("phase", &self.phase)
            .field("revision", &self.revision)
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
            "auth-authlib-characters" => Phase::Characters,
            "auth-authlib-complete" | "auth-complete" => Phase::Complete,
            "auth-offline-error" | "auth-error" => Phase::Failed,
            "auth-device-preparing" => Phase::Preparing,
            "auth-device-waiting" | "auth-waiting" => Phase::Waiting,
            "auth-verifying" => Phase::Verifying,
            "auth-saving" | "auth-offline-saving" => Phase::Saving,
            _ => Phase::Connect,
        };
        // The account page with its alternatives revealed.
        self.other_sign_in_options = name == "auth-choose";
        if matches!(name, "auth-complete" | "auth-saving") {
            self.prepared_account = account.clone();
        }
        if name.starts_with("auth-offline-") {
            self.provider = AccountProvider::Offline;
            if name == "auth-offline-saving" {
                self.prepared_account = super::engine::offline_account("Pixel").ok();
            }
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
                self.error = Some(AccountMessage::product("accounts.auth.error.offline_name"));
            }
        }
        if name.starts_with("auth-authlib-") {
            self.provider = AccountProvider::Authlib;
            self.external_provider = Some(uuid::Uuid::from_u128(
                if name == "auth-authlib-http-warning" {
                    2
                } else {
                    1
                },
            ));
            self.external_api = Some(
                if name == "auth-authlib-http-warning" {
                    "http://community.example.test/api/"
                } else {
                    "https://community.example.test/api/"
                }
                .into(),
            );
            if let Some(login) = &self.login {
                login.update(cx, |field, cx| field.set_value("player@example.test", cx));
            }
            if name == "auth-authlib-characters" {
                self.characters = vec![
                    block_auth::AuthlibProfile {
                        id: uuid::Uuid::from_u128(11),
                        name: "Explorer".into(),
                    },
                    block_auth::AuthlibProfile {
                        id: uuid::Uuid::from_u128(12),
                        name: "Builder".into(),
                    },
                ];
                self.character = Some(uuid::Uuid::from_u128(12));
            }
            if name == "auth-authlib-complete"
                && let Some(mut account) = account
            {
                account.provider = AccountProvider::Authlib;
                account.authlib = Some(block_auth::AuthlibAccount {
                    provider_id: uuid::Uuid::from_u128(1),
                    login: "player@example.test".into(),
                });
                account.client_id.clear();
                account.credential_ref = account.identity_key();
                self.prepared_account = Some(account);
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
        if name == "auth-waiting" {
            // Mirrors the real browser flow, which keeps the URL so the page can
            // offer "Open browser again" after focus leaves the browser.
            self.browser_url = Some("https://www.microsoft.com/link".into());
        }
        if name == "auth-error" {
            self.error = Some(sign_in_error("The authorization request expired."));
        }
    }
    pub(super) fn snapshot(&self, cx: &Context<'_, Block>) -> Self {
        let body_scroll = ScrollHandle::new();
        body_scroll.set_offset(self.body_scroll.offset());
        Self {
            phase: self.phase,
            provider: self.provider,
            username: self.username.clone(),
            login: self.login.clone(),
            // Outgoing motion layers never retain a password editor or its history.
            password: None,
            external_provider: self.external_provider,
            external_api: self.external_api.clone(),
            accepted_http: self.accepted_http.clone(),
            characters: self.characters.clone(),
            character: self.character,
            provider_draft: None,
            method: self.method,
            device_code: None,
            attempt: None,
            stage: self.stage,
            revision: self.revision,
            other_sign_in_options: self.other_sign_in_options,
            browser_url: None,
            prepared_account: self.prepared_account.clone(),
            error: self.error.clone(),
            save_failed: self.save_failed,
            return_window: self.return_window,
            focus: Some(cx.focus_handle()),
            body_scroll,
        }
    }

    pub(super) const fn motion_key(&self) -> AuthStep {
        self.phase.step()
    }

    fn transition(&mut self, phase: Phase) {
        if self.phase.step() != phase.step() {
            self.revision = self.revision.wrapping_add(1);
            self.body_scroll.set_offset(point(px(0.), px(0.)));
        }
        self.phase = phase;
    }

    fn failed(&mut self, error: AccountMessage) {
        self.browser_url = None;
        self.device_code = None;
        self.save_failed = false;
        self.error = Some(match self.provider {
            AccountProvider::Offline => {
                AccountMessage::product("accounts.auth.error.offline_save_failed")
            }
            AccountProvider::Authlib => error,
            AccountProvider::Microsoft => match error {
                AccountMessage::Diagnostic(raw) => sign_in_error(&raw),
                product => product,
            },
        });
        self.body_scroll.set_offset(point(px(0.), px(0.)));
        self.transition(Phase::Failed);
    }
}

impl Block {
    pub(super) fn auth_save_pending(&self) -> bool {
        self.shell
            .auth
            .as_ref()
            .is_some_and(|auth| auth.phase == Phase::Saving)
    }
    pub(super) fn open_auth_provider_add(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self
            .shell
            .auth
            .as_ref()
            .is_some_and(|state| state.provider_draft.is_some())
        {
            self.shell
                .surfaces
                .open(super::shell::Surface::AuthProviderAdd, window, cx);
            return;
        }
        if self.busy
            || self.shell.auth.as_ref().is_none_or(|s| {
                s.provider != AccountProvider::Authlib
                    || !matches!(s.phase, Phase::Connect | Phase::Failed)
            })
        {
            return;
        }
        if self
            .shell
            .auth
            .as_ref()
            .is_some_and(|s| s.provider_draft.is_none())
        {
            let language = self.language();
            let address = super::input(
                "",
                &tr!(language, "accounts.auth.provider_add.address.placeholder"),
                cx,
            );
            cx.observe(&address, |view, field, cx| {
                let value = field.read(cx).value();
                if let Some(draft) = view
                    .shell
                    .auth
                    .as_mut()
                    .and_then(|s| s.provider_draft.as_mut())
                    && draft.address == field
                    && draft.observed_address != value
                {
                    draft.observed_address = value;
                    draft.accepted_http = None;
                    draft.error = None;
                    cx.notify();
                }
            })
            .detach();
            let draft = AuthProviderDraft {
                id: uuid::Uuid::new_v4(),
                name: super::input(
                    "",
                    &tr!(language, "accounts.auth.provider_add.name.placeholder"),
                    cx,
                ),
                address,
                observed_address: String::new(),
                accepted_http: None,
                error: None,
                attempt: None,
                saved: None,
                scroll: ScrollHandle::new(),
            };
            if let Some(state) = &mut self.shell.auth {
                state.provider_draft = Some(draft);
            }
        }
        self.shell
            .surfaces
            .open(super::shell::Surface::AuthProviderAdd, window, cx);
        cx.notify();
    }

    pub(super) fn cancel_auth_provider(&mut self, cx: &mut Context<'_, Self>) {
        if let Some(draft) = self
            .shell
            .auth
            .as_mut()
            .and_then(|s| s.provider_draft.take())
            && let Some((revision, cancel)) = draft.attempt
        {
            cancel.cancel();
            self.abandon_operation(revision);
        }
        cx.notify();
    }

    pub(super) fn identify_auth_provider(&mut self, cx: &mut Context<'_, Self>) {
        if self.busy {
            return;
        }
        if let Some(draft) = self
            .shell
            .auth
            .as_mut()
            .and_then(|s| s.provider_draft.as_mut())
        {
            match block_auth::ProviderAddress::parse(&draft.address.read(cx).value()) {
                Ok(address) => {
                    if draft.name.read(cx).value().trim().is_empty() {
                        draft
                            .name
                            .update(cx, |field, cx| field.set_value(&address.suggested_name, cx));
                    }
                    draft
                        .address
                        .update(cx, |field, cx| field.set_value(&address.url, cx));
                    draft.observed_address = address.url;
                    draft.accepted_http = None;
                    draft.error = None;
                }
                Err(error) => draft.error = Some(AccountMessage::diagnostic(error.to_string())),
            }
        }
        cx.notify();
    }

    pub(super) fn consent_auth_provider(&mut self, cx: &mut Context<'_, Self>) {
        if self.busy {
            return;
        }
        if let Some(draft) = self
            .shell
            .auth
            .as_mut()
            .and_then(|s| s.provider_draft.as_mut())
            && let Ok(address) = block_auth::ProviderAddress::parse(&draft.address.read(cx).value())
            && address.insecure
        {
            draft.accepted_http = if draft.accepted_http.as_deref() == Some(address.url.as_str()) {
                None
            } else {
                Some(address.url)
            };
            draft.error = None;
        }
        cx.notify();
    }

    pub(super) fn save_auth_provider(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.busy {
            return;
        }
        let Some(draft) = self
            .shell
            .auth
            .as_ref()
            .and_then(|s| s.provider_draft.as_ref())
        else {
            return;
        };
        let address = match block_auth::ProviderAddress::parse(&draft.address.read(cx).value()) {
            Ok(address) => address,
            Err(error) => {
                if let Some(draft) = self
                    .shell
                    .auth
                    .as_mut()
                    .and_then(|s| s.provider_draft.as_mut())
                {
                    draft.error = Some(AccountMessage::diagnostic(error.to_string()));
                }
                cx.notify();
                return;
            }
        };
        if let Some(provider) = self
            .shell
            .providers
            .records
            .iter()
            .find(|p| p.api_url == address.url)
        {
            self.use_auth_provider(provider.id, window, cx);
            return;
        }
        let name = draft.name.read(cx).value().trim().to_owned();
        let accepted = draft.accepted_http.as_deref() == Some(address.url.as_str());
        if name.is_empty() || address.insecure && !accepted {
            if let Some(draft) = self
                .shell
                .auth
                .as_mut()
                .and_then(|s| s.provider_draft.as_mut())
            {
                draft.error = Some(if name.is_empty() {
                    AccountMessage::product("accounts.auth.error.provider_display_name")
                } else {
                    AccountMessage::product("accounts.auth.error.provider_http_confirm")
                });
            }
            cx.notify();
            return;
        }
        self.begin_cancellable(
            msg!("accounts.auth.status.discovering_provider"),
            super::ui_logging::Operation::DiscoverAuthProvider,
        );
        self.operation_kind = super::OperationKind::Account;
        if let Some(draft) = self
            .shell
            .auth
            .as_mut()
            .and_then(|s| s.provider_draft.as_mut())
        {
            draft.error = None;
            draft.saved = None;
            draft.attempt = Some((self.operation_revision, self.cancel.clone()));
            for field in [&draft.name, &draft.address] {
                field.update(cx, |field, cx| field.set_read_only(true, cx));
            }
        }
        if let Some(engine) = &self.engine {
            engine.save_provider(None, name, address.url, accepted, self.cancel.clone());
        } else {
            self.fail_auth_provider(
                AccountMessage::product("accounts.auth.error.provider_setup_unavailable"),
                cx,
            );
        }
        cx.notify();
    }

    fn use_auth_provider(
        &mut self,
        id: uuid::Uuid,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(api) = self
            .shell
            .providers
            .records
            .iter()
            .find(|p| p.id == id)
            .map(|p| p.api_url.clone())
        else {
            return;
        };
        if let Some(state) = &mut self.shell.auth {
            if let Some(field) = &state.password {
                field.update(cx, |field, cx| field.set_value("", window, cx));
            }
            state.external_provider = Some(id);
            state.external_api = Some(api);
            state.accepted_http = None;
            state.error = None;
            state.provider_draft = None;
            state.transition(Phase::Connect);
        }
        self.shell.surfaces.close(window, &self.focus, cx);
        cx.notify();
    }

    pub(super) fn auth_provider_finished(
        &mut self,
        result: &Result<crate::i18n::UiText, String>,
        cx: &mut Context<'_, Self>,
    ) {
        if let Err(error) = result {
            self.fail_auth_provider(AccountMessage::diagnostic(error.clone()), cx);
        }
    }

    /// Release the Add-provider draft after its attempt failed.
    fn fail_auth_provider(&mut self, error: AccountMessage, cx: &mut Context<'_, Self>) {
        let revision = self.operation_revision;
        let Some(draft) = self
            .shell
            .auth
            .as_mut()
            .and_then(|s| s.provider_draft.as_mut())
        else {
            return;
        };
        if draft
            .attempt
            .as_ref()
            .is_some_and(|(attempt, _)| *attempt == revision)
        {
            draft.error = Some(error);
            draft.attempt = None;
            for field in [&draft.name, &draft.address] {
                field.update(cx, |field, cx| field.set_read_only(false, cx));
            }
        }
    }

    pub(super) fn auth_provider_saved(&mut self, id: &str, cx: &mut Context<'_, Self>) -> bool {
        let Ok(id) = id.parse::<uuid::Uuid>() else {
            return false;
        };
        let Some(draft) = self
            .shell
            .auth
            .as_mut()
            .and_then(|s| s.provider_draft.as_mut())
        else {
            return false;
        };
        if draft.attempt.as_ref().is_none_or(|(revision, cancel)| {
            *revision != self.operation_revision || cancel.is_cancelled()
        }) {
            return false;
        }
        draft.saved = Some(id);
        let draft_id = draft.id;
        let handle = self
            .shell
            .auth
            .as_ref()
            .and_then(|state| state.return_window);
        if let Some(handle) = handle {
            let owner = cx.entity();
            cx.defer(move |cx| {
                let _ = handle.update(cx, |_, window, cx| {
                    owner.update(cx, |view, cx| {
                        if view
                            .shell
                            .auth
                            .as_ref()
                            .and_then(|s| s.provider_draft.as_ref())
                            .is_none_or(|draft| draft.id != draft_id || draft.saved != Some(id))
                        {
                            return;
                        }
                        view.use_auth_provider(id, window, cx);
                    })
                });
            });
        }
        true
    }

    pub(super) fn render_auth_provider_add(
        &self,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> gpui::Div {
        let Some(draft) = self
            .shell
            .auth
            .as_ref()
            .and_then(|s| s.provider_draft.as_ref())
        else {
            return div();
        };
        let parsed = block_auth::ProviderAddress::parse(&draft.address.read(cx).value()).ok();
        let existing = parsed.as_ref().and_then(|a| {
            self.shell
                .providers
                .records
                .iter()
                .find(|p| p.api_url == a.url)
        });
        let language = self.language();
        let mut form = div()
            .flex()
            .flex_col()
            .gap_4()
            .child(patterns::wizard_heading(
                tr!(language, "accounts.auth.provider_add.title"),
                tr!(language, "accounts.auth.provider_add.description"),
            ))
            .child(super::attach_provider_drop_bounds(
                div().child(
                    div()
                        .id("provider-drop-zone")
                        .debug_selector(|| "provider-drop-zone".into())
                        .w_full()
                        .p_4()
                        .rounded_lg()
                        .border_1()
                        .border_dashed()
                        .border_color(rgb(self.palette.border))
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap_1()
                        .child(Icon::ExternalLink.render(20., self.palette.muted))
                        .child(
                            div()
                                .text_sm()
                                .child(tr!(language, "accounts.auth.provider_add.drop")),
                        )
                        .child(note(tr!(language, "accounts.auth.provider_add.drop_note"))),
                ),
                cx.entity().downgrade(),
            ))
            .child(components::field_with(
                tr!(language, "accounts.auth.provider_add.name.label"),
                &draft.name,
                self.palette,
                components::FieldOptions {
                    required: true,
                    ..Default::default()
                },
            ))
            .child(components::field_with(
                tr!(language, "accounts.auth.provider_add.address.label"),
                &draft.address,
                self.palette,
                components::FieldOptions {
                    required: true,
                    ..Default::default()
                },
            ))
            .child(self.auth_button(
                "auth-provider-identify",
                tr!(language, "accounts.auth.provider_add.identify"),
                UiAction::AuthProviderIdentify,
                ButtonVariant::Outline,
                ControlState {
                    disabled: self.busy,
                    ..Default::default()
                },
                cx,
            ));
        if let Some(provider) = existing {
            form = form.child(note(tr!(
                language,
                "accounts.auth.provider_add.existing",
                name = provider.name
            )));
        }
        if let Some(address) = &parsed
            && address.insecure
        {
            form = form
                .child(note(tr!(language, "accounts.auth.provider_add.http_note")))
                .child(components::checkbox(
                    "auth-provider-http",
                    tr!(language, "accounts.auth.http_consent"),
                    draft.accepted_http.as_deref() == Some(address.url.as_str()),
                    self.palette,
                    ControlState {
                        disabled: self.busy,
                        ..Default::default()
                    },
                    cx.listener(|view, (), window, cx| {
                        view.handle(UiAction::AuthProviderConsent, window, cx)
                    }),
                ));
        }
        let form = form
            .children(
                draft
                    .error
                    .as_ref()
                    .map(|error| note(error.render(language))),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(self.auth_button(
                        "auth-provider-cancel",
                        tr!(language, "common.cancel"),
                        UiAction::AuthProviderCancel,
                        ButtonVariant::Ghost,
                        ControlState::default(),
                        cx,
                    ))
                    .child(self.auth_button(
                        "auth-provider-save",
                        if existing.is_some() {
                            tr!(language, "accounts.auth.provider_add.use_saved")
                        } else {
                            tr!(language, "accounts.auth.provider_add.add")
                        },
                        UiAction::AuthProviderSave,
                        ButtonVariant::Primary,
                        ControlState {
                            busy: draft.attempt.is_some(),
                            disabled: self.busy
                                || existing.is_none()
                                    && parsed.as_ref().is_some_and(|a| {
                                        a.insecure
                                            && draft.accepted_http.as_deref()
                                                != Some(a.url.as_str())
                                    }),
                            ..Default::default()
                        },
                        cx,
                    )),
            );
        let max_height = (window.viewport_size().height - px(80.)).max(px(0.));
        div().w_full().child(
            div()
                .id("auth-provider-scroll")
                .debug_selector(|| "auth-provider-scroll".into())
                .max_h(max_height)
                .scroll_area(
                    &draft.scroll,
                    "accounts.auth.provider_add",
                    self.palette,
                    false,
                )
                .child(form),
        )
    }

    pub(super) fn account_provider(
        &self,
        account: &Account,
    ) -> Option<&block_auth::AuthlibProvider> {
        let id = account.authlib.as_ref()?.provider_id;
        self.shell
            .providers
            .records
            .iter()
            .find(|provider| provider.id == id)
    }

    pub(super) fn account_authority(&self, account: &Account) -> String {
        if account.provider == AccountProvider::Authlib {
            self.account_provider(account).map_or_else(
                || tr!(self.language(), "accounts.auth.provider.missing"),
                |provider| provider.name.clone(),
            )
        } else {
            provider_label(self.language(), account.provider)
        }
    }
    fn start_external_auth(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(state) = &self.shell.auth else {
            return;
        };
        if state.provider_draft.is_some() {
            return;
        }
        let Some(provider) = self
            .shell
            .providers
            .records
            .iter()
            .find(|p| Some(p.id) == state.external_provider)
            .cloned()
        else {
            if let Some(state) = &mut self.shell.auth {
                state.failed(AccountMessage::product(
                    "accounts.auth.error.choose_provider",
                ));
            }
            cx.notify();
            return;
        };
        if state.external_api.as_deref() != Some(provider.api_url.as_str()) {
            if let Some(state) = &mut self.shell.auth {
                if let Some(field) = &state.password {
                    field.update(cx, |input, cx| input.set_value("", window, cx));
                }
                state.accepted_http = None;
                state.failed(AccountMessage::product(
                    "accounts.auth.provider.address_changed",
                ));
            }
            cx.notify();
            return;
        }
        if provider.api_url.starts_with("http:")
            && state.accepted_http.as_deref() != Some(provider.api_url.as_str())
        {
            if let Some(state) = &mut self.shell.auth {
                state.failed(AccountMessage::product(
                    "accounts.auth.error.provider_http_before_send",
                ));
            }
            cx.notify();
            return;
        }
        if state.phase == Phase::Characters {
            let Some(character) = state
                .character
                .filter(|id| state.characters.iter().any(|p| p.id == *id))
            else {
                return;
            };
            self.begin_cancellable(
                msg!("accounts.auth.status.selecting_character"),
                super::ui_logging::Operation::SelectAuthCharacter,
            );
            self.operation_kind = super::OperationKind::Account;
            if let Some(state) = &mut self.shell.auth {
                state.error = None;
                state.attempt = Some((self.operation_revision, self.cancel.clone()));
                state.transition(Phase::Verifying);
            }
            if let Some(engine) = &self.engine {
                engine.select_authlib_character(provider.id, character, self.cancel.clone());
            }
            cx.notify();
            return;
        }
        let login = state.login.as_ref().map_or_else(String::new, |field| {
            field.read(cx).value().trim().to_owned()
        });
        let password = state
            .password
            .as_ref()
            .map_or_else(String::new, |field| field.read(cx).value().to_string());
        if login.is_empty() || password.is_empty() {
            if let Some(state) = &mut self.shell.auth {
                state.failed(AccountMessage::product(
                    "accounts.auth.error.provider_credentials",
                ));
            }
            cx.notify();
            return;
        }
        let password = block_auth::SecretString::new(password);
        if let Some(field) = self
            .shell
            .auth
            .as_ref()
            .and_then(|state| state.password.as_ref())
        {
            field.update(cx, |input, cx| input.set_value("", window, cx));
        }
        self.abandon_auth_attempt();
        self.begin_cancellable(
            msg!("accounts.auth.status.authenticating"),
            super::ui_logging::Operation::AuthenticateProvider,
        );
        self.operation_kind = super::OperationKind::Account;
        if let Some(state) = &mut self.shell.auth {
            state.characters.clear();
            state.character = None;
            state.prepared_account = None;
            state.error = None;
            state.attempt = Some((self.operation_revision, self.cancel.clone()));
            if !self.busy {
                state.failed(AccountMessage::product(
                    "accounts.auth.error.provider_start",
                ));
            } else {
                state.transition(Phase::Preparing);
            }
            if let Some(focus) = &state.focus {
                focus.focus(window, cx);
            }
        }
        if let Some(engine) = &self.engine {
            engine.sign_in_authlib(
                provider.id,
                provider.api_url,
                login,
                password,
                self.cancel.clone(),
            );
        }
        cx.notify();
    }

    pub(super) fn auth_characters(&mut self, characters: Vec<block_auth::AuthlibProfile>) {
        if let Some(state) = &mut self.shell.auth
            && state.provider == AccountProvider::Authlib
            && state.phase.pending()
            && state
                .attempt
                .as_ref()
                .is_some_and(|(_, cancel)| !cancel.is_cancelled())
            && !characters.is_empty()
        {
            state.character = characters.first().map(|p| p.id);
            state.characters = characters;
            state.error = None;
            state.transition(Phase::Characters);
        }
    }

    pub(super) fn choose_auth_character(&mut self, id: uuid::Uuid, cx: &mut Context<'_, Self>) {
        if let Some(state) = &mut self.shell.auth
            && state.phase == Phase::Characters
            && !self.busy
            && state.characters.iter().any(|profile| profile.id == id)
        {
            state.character = Some(id);
            state.error = None;
            cx.notify();
        }
    }

    pub(super) fn choose_external_provider(
        &mut self,
        id: uuid::Uuid,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.busy {
            return;
        }
        let Some(api) = self
            .shell
            .providers
            .records
            .iter()
            .find(|provider| provider.id == id)
            .map(|provider| provider.api_url.clone())
        else {
            return;
        };
        if let Some(state) = &mut self.shell.auth
            && state.provider == AccountProvider::Authlib
            && matches!(state.phase, Phase::Connect | Phase::Failed)
        {
            if state.external_provider != Some(id)
                || state.external_api.as_deref() != Some(api.as_str())
            {
                state.external_provider = Some(id);
                state.external_api = Some(api);
                state.accepted_http = None;
                state.characters.clear();
                state.character = None;
                if let Some(field) = &state.password {
                    field.update(cx, |input, cx| input.set_value("", window, cx));
                }
            }
            state.error = None;
            state.transition(Phase::Connect);
            if self
                .shell
                .surfaces
                .top()
                .is_some_and(|entry| matches!(entry.kind, super::shell::Surface::Select(_)))
            {
                self.shell.surfaces.close(window, &self.focus, cx);
            }
            cx.notify();
        }
    }

    pub(super) fn consent_external_auth(&mut self, cx: &mut Context<'_, Self>) {
        if self.busy {
            return;
        }
        if let Some(state) = &mut self.shell.auth
            && matches!(state.phase, Phase::Connect | Phase::Failed)
            && let Some(provider) = self
                .shell
                .providers
                .records
                .iter()
                .find(|p| Some(p.id) == state.external_provider)
        {
            state.accepted_http =
                if state.accepted_http.as_deref() == Some(provider.api_url.as_str()) {
                    None
                } else {
                    Some(provider.api_url.clone())
                };
            state.error = None;
            cx.notify();
        }
    }

    pub(super) fn auth_provider_choices(&self) -> Vec<super::select::SelectChoice<UiAction>> {
        if self.shell.auth.as_ref().is_none_or(|state| {
            state.provider != AccountProvider::Authlib
                || !matches!(state.phase, Phase::Connect | Phase::Failed)
                || state.provider_draft.is_some()
        }) {
            return Vec::new();
        }
        self.shell
            .providers
            .records
            .iter()
            .map(|provider| super::select::SelectChoice {
                label: format!("{} · {}", provider.name, provider.api_url),
                selected: self
                    .shell
                    .auth
                    .as_ref()
                    .is_some_and(|state| state.external_provider == Some(provider.id)),
                action: UiAction::AuthExternalProvider(provider.id),
            })
            .collect()
    }

    pub(super) fn choose_auth_provider(
        &mut self,
        provider: AccountProvider,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(state) = &mut self.shell.auth else {
            return;
        };
        if state.phase != Phase::Connect || self.busy || state.provider == provider {
            return;
        }
        state.provider = provider;
        state.error = None;
        state.save_failed = false;
        state.prepared_account = None;
        // The alternatives are found at the bottom of a compact page, so an
        // explicit account type starts the page over at its own form.
        state.body_scroll.set_offset(point(px(0.), px(0.)));
        let window = state.return_window;
        if let Some(window) = window {
            let owner = cx.entity();
            cx.defer(move |cx| {
                let _ = window.update(cx, |_, window, cx| {
                    owner.update(cx, |view, cx| view.focus_auth_form(window, cx));
                });
            });
        }
        cx.notify();
    }

    /// Focus where the account page now starts: the chosen type's first field,
    /// or the page itself for the type whose next step is the footer action.
    ///
    /// Anything the page opened or a phase that moved on in the meantime keeps
    /// its own focus.
    fn focus_auth_form(&self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(state) = &self.shell.auth else {
            return;
        };
        if state.phase != Phase::Connect || self.shell.surfaces.is_open() {
            return;
        }
        match state.provider {
            AccountProvider::Offline => {
                if let Some(field) = &state.username {
                    field.read(cx).focus_handle(cx).focus(window, cx);
                }
            }
            AccountProvider::Authlib => self
                .shell
                .surfaces
                .trigger_focus("auth-provider-picker", cx)
                .focus(window, cx),
            AccountProvider::Microsoft => {
                if let Some(focus) = &state.focus {
                    focus.focus(window, cx);
                }
            }
        }
    }

    pub(super) fn toggle_auth_other_options(&mut self, cx: &mut Context<'_, Self>) {
        if let Some(state) = &mut self.shell.auth
            && state.phase == Phase::Connect
            && !self.busy
        {
            state.other_sign_in_options = !state.other_sign_in_options;
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
            if self.abandon_operation(revision) {
                self.status = msg!("accounts.auth.status.cancelled");
            }
        }
        if let Some(engine) = &self.engine {
            engine.discard_prepared_sign_in();
        }
    }

    pub(super) fn auth_back(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(state) = &self.shell.auth else {
            return;
        };
        let phase = state.phase;
        // A verified identity is either saved or closed, and a save in flight
        // has no earlier page to return to.
        if matches!(phase, Phase::Complete | Phase::Saving | Phase::Saved)
            || phase == Phase::Connect && state.provider == AccountProvider::Microsoft
        {
            return;
        }
        let drafting = state.provider_draft.is_some();
        if drafting {
            self.cancel_auth_provider(cx);
            let handle = self
                .shell
                .auth
                .as_ref()
                .and_then(|state| state.return_window);
            if let Some(handle) = handle {
                let owner = cx.entity();
                cx.defer(move |cx| {
                    let _ = handle.update(cx, |_, window, cx| {
                        owner.update(cx, |view, cx| {
                            if view
                                .shell
                                .auth
                                .as_ref()
                                .is_some_and(|state| state.provider_draft.is_some())
                            {
                                return;
                            }
                            view.shell.surfaces.close(window, &view.focus, cx);
                        })
                    });
                });
            }
        }
        self.abandon_auth_attempt();
        if let Some(state) = &mut self.shell.auth {
            if let Some(password) = &state.password {
                password.update(cx, |input, cx| input.set_value("", window, cx));
            }
            state.characters.clear();
            state.character = None;
            state.browser_url = None;
            state.device_code = None;
            state.stage = None;
            state.method = Method::Browser;
            state.prepared_account = None;
            state.error = None;
            state.save_failed = false;
            if phase == Phase::Connect {
                // The alternatives lead back to the default Microsoft page.
                state.provider = AccountProvider::Microsoft;
            }
            state.transition(Phase::Connect);
        }
        cx.notify();
    }

    /// Switch the Microsoft page between its two real authorization modes.
    ///
    /// The device code is a request of its own, so choosing it starts it. A
    /// browser authorization stays the page's single primary action, so
    /// switching back only returns the page to that state.
    pub(super) fn change_auth_method(
        &mut self,
        method: Method,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.shell.auth.as_ref().is_none_or(|state| {
            state.provider != AccountProvider::Microsoft
                || state.method == method
                || matches!(state.phase, Phase::Complete | Phase::Saving | Phase::Saved)
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
            state.save_failed = false;
            state.transition(Phase::Connect);
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
            return;
        }
        if self.busy {
            return;
        }
        let focus = cx.focus_handle();
        let language = self.language();
        let password = cx.new(|cx| {
            gpui_base::input::InputState::new(window, cx)
                .masked(true)
                .placeholder(tr!(language, "accounts.auth.password.placeholder"))
                .validate(|value, _| value.len() <= 4096)
        });
        cx.observe(&password, |_, _, cx| cx.notify()).detach();
        cx.subscribe(
            &password,
            |view, input, event: &gpui_base::input::InputEvent, cx| {
                if !matches!(
                    event,
                    gpui_base::input::InputEvent::PressEnter {
                        secondary: false,
                        shift: false
                    }
                ) || view.shell.auth.as_ref().is_none_or(|state| {
                    state.provider != AccountProvider::Authlib
                        || state.password.as_ref() != Some(&input)
                        || !matches!(state.phase, Phase::Connect | Phase::Failed)
                }) {
                    return;
                }
                let handle = view
                    .shell
                    .auth
                    .as_ref()
                    .and_then(|state| state.return_window);
                if let Some(handle) = handle {
                    let owner = cx.entity();
                    cx.defer(move |cx| {
                        let _ = handle.update(cx, |_, window, cx| {
                            owner.update(cx, |view, cx| view.start_auth(window, cx))
                        });
                    });
                }
            },
        )
        .detach();
        let wizard = AuthWizard {
            username: Some(super::input(
                "",
                &tr!(language, "accounts.auth.offline.name.placeholder"),
                cx,
            )),
            login: Some(super::input(
                "",
                &tr!(language, "accounts.auth.login.placeholder"),
                cx,
            )),
            password: Some(password),
            external_provider: self.shell.providers.records.first().map(|p| p.id),
            external_api: self
                .shell
                .providers
                .records
                .first()
                .map(|p| p.api_url.clone()),
            return_window: Some(window.window_handle()),
            focus: Some(focus.clone()),
            provider: AccountProvider::Microsoft,
            ..AuthWizard::default()
        };
        self.router.begin_auth();
        self.shell.auth = Some(wizard);
        // The Add account wizard is the Accounts page's own page, as in the
        // prototype's /accounts/new; never a separate window or overlay.
        self.shell.providers.visible = false;
        self.shell.surfaces.clear(window, &focus, cx);
        focus.focus(window, cx);
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
            .is_some_and(|state| state.provider == AccountProvider::Authlib)
        {
            self.start_external_auth(window, cx);
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
            let prepared = match result {
                Ok(account) => Some(account),
                // The only name a local identity can fail on is its own player
                // name, so the page states the rule instead of a raw diagnostic.
                Err(_) => {
                    if let Some(state) = &mut self.shell.auth {
                        state.prepared_account = None;
                        state.save_failed = false;
                        state.error =
                            Some(AccountMessage::product("accounts.auth.error.offline_name"));
                        state.transition(Phase::Failed);
                    }
                    None
                }
            };
            // A local identity has no provider to confirm with, so its name page
            // saves it directly; there is no second confirmation step.
            if let Some(account) = prepared {
                self.begin_save(account, cx);
            }
            cx.notify();
            return;
        }
        self.abandon_auth_attempt();
        self.begin_cancellable(
            msg!("accounts.auth.status.preparing"),
            super::ui_logging::Operation::AuthenticateMicrosoft,
        );
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
        state.save_failed = false;
        if !self.busy {
            state.failed(AccountMessage::product(
                "accounts.auth.error.microsoft_start",
            ));
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
        self.status = msg!(auth_stage_key(progress));
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

    /// Authentication is verified; Finish starts the durable save.
    pub(super) fn auth_prepared(&mut self, account: Account) {
        if let Some(state) = &mut self.shell.auth
            && state.provider == account.provider
            && (account.provider != AccountProvider::Authlib
                || account
                    .authlib
                    .as_ref()
                    .is_some_and(|link| Some(link.provider_id) == state.external_provider))
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

    pub(super) fn auth_finished(&mut self, result: &Result<crate::i18n::UiText, String>) {
        let saving = self
            .shell
            .auth
            .as_ref()
            .is_some_and(|state| state.phase == Phase::Saving);
        if let Some(state) = &mut self.shell.auth
            && (state.phase.pending() || saving)
        {
            match result {
                Err(error)
                    if state.provider == AccountProvider::Authlib
                        && !state.characters.is_empty()
                        && state.phase == Phase::Verifying =>
                {
                    state.error = Some(AccountMessage::diagnostic(error.clone()));
                    state.transition(Phase::Characters);
                }
                Err(error) => {
                    state.failed(AccountMessage::diagnostic(error.clone()));
                    // A failed save has spent the verified credential, so the
                    // only recovery the page can offer is signing in again.
                    if saving {
                        state.save_failed = true;
                        state.prepared_account = None;
                    }
                }
                Ok(_) => state.failed(AccountMessage::product(
                    "accounts.auth.error.ended_without_identity",
                )),
            }
        }
    }

    pub(super) fn cancel_auth(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self
            .shell
            .auth
            .as_ref()
            .is_some_and(|state| state.phase == Phase::Saving)
        {
            // Finish authorized a durable write. Every exit refuses alike while
            // it runs, so none of them can read as revoking it.
            return;
        }
        self.cancel_auth_provider(cx);
        if let Some(password) = self
            .shell
            .auth
            .as_ref()
            .and_then(|state| state.password.as_ref())
        {
            password.update(cx, |input, cx| input.set_value("", window, cx));
        }
        let committing = self
            .shell
            .auth
            .as_ref()
            .is_some_and(|state| state.phase == Phase::Saved);
        if !committing {
            self.abandon_auth_attempt();
        }
        self.leave_auth_context(window, cx);
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
        self.begin_save(account, cx);
    }

    /// Persist one verified identity.
    ///
    /// Online accounts reach this from the review page that confirms them; a
    /// local identity reaches it from its own name page.
    fn begin_save(&mut self, account: Account, cx: &mut Context<'_, Self>) {
        self.begin(
            msg!("accounts.auth.status.saving"),
            super::ui_logging::Operation::SaveAccount,
        );
        self.operation_kind = super::OperationKind::Account;
        let busy = self.busy;
        let attempt = (self.operation_revision, self.cancel.clone());
        let Some(state) = &mut self.shell.auth else {
            return;
        };
        state.error = None;
        state.save_failed = false;
        if !busy {
            state.prepared_account = None;
            state.failed(AccountMessage::product(
                "accounts.auth.error.save_unavailable",
            ));
            cx.notify();
            return;
        }
        state.prepared_account = Some(account.clone());
        state.attempt = Some(attempt);
        state.transition(Phase::Saving);
        if let Some(engine) = &self.engine {
            match account.provider {
                AccountProvider::Offline => engine.save_offline_account(account),
                AccountProvider::Authlib => engine.save_authlib_sign_in(account.identity_key()),
                AccountProvider::Microsoft => engine.save_sign_in(account.reference),
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
                .is_none_or(|prepared| prepared.identity_key() != account.identity_key())
        {
            return;
        }
        state.prepared_account = Some(account);
        state.transition(Phase::Saved);
        if self.router.auth_return() == Some(Tab::Onboarding)
            && let Some(account) = &state.prepared_account
        {
            let id = account_id(account);
            self.shell.onboarding.account = Some(id.clone());
            self.shell.onboarding.account_decided = true;
            self.selected_account = Some(id.clone());
            self.shell.accounts.inspected = Some(id);
        }
        let handle = self
            .shell
            .auth
            .as_ref()
            .and_then(|state| state.return_window);
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
        if self.router.auth_return() == Some(Tab::Onboarding) {
            self.shell.onboarding.account = Some(id.clone());
            self.shell.onboarding.account_decided = true;
        }
        self.selected_account = Some(id.clone());
        self.shell.accounts.inspected = Some(id);
        self.status = msg!("accounts.auth.status.saved", name = account.player_name);
        self.leave_auth_context(window, cx);
    }

    /// Leave the account page for the destination that opened it, keeping that
    /// page's draft — whether the identity was saved or the attempt abandoned.
    fn leave_auth_context(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.shell.auth.take().is_none() {
            return;
        }
        self.router.end_auth();
        self.sync_provider_visibility();
        self.focus.focus(window, cx);
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
        let content = self.host_wizard(
            super::page_host::WizardSlots {
                // The account page is one full-width form. The shared host keeps
                // the slot, and this page has no sidebar to put in it.
                rail: div().into_any_element(),
                body: self.render_auth_transition_body(cx),
                footer: self.auth_footer(state, cx).into_any_element(),
            },
            super::page_host::WizardLayout::Authentication,
            window,
            cx,
        );
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

    /// The page's only scrolling region, laid out as one reading column that the
    /// footer aligns to.
    pub(super) fn render_auth_transition_body(&self, cx: &Context<'_, Self>) -> gpui::AnyElement {
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
                "accounts.auth.body",
                self.palette,
                false,
            )
            .p_4()
            .child(
                div()
                    .w_full()
                    .max_w(px(AUTH_CONTENT_WIDTH))
                    .mx_auto()
                    .child(self.auth_body(state, cx)),
            )
            .into_any_element()
    }

    /// The localized heading for the state the page is in.
    fn auth_heading(&self, state: &AuthWizard) -> (String, String) {
        let language = self.language();
        let (title, description) = auth_heading_keys(state);
        (tr!(language, title), tr!(language, description))
    }

    fn auth_body(&self, state: &AuthWizard, cx: &Context<'_, Self>) -> gpui::Div {
        let language = self.language();
        let (title, description) = self.auth_heading(state);
        let mut body = div()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_4()
            .child(patterns::wizard_heading(title, description));
        body = match state.phase {
            Phase::Connect => body.child(self.auth_connect(state, cx)),
            // Waiting and verification happen on the page the attempt started
            // from, so they replace its form instead of becoming steps.
            Phase::Preparing | Phase::Waiting | Phase::Verifying => {
                body.child(self.auth_waiting(state, cx))
            }
            Phase::Characters => body.child(self.auth_character_choice(state, cx)),
            Phase::Failed => body.child(self.auth_failed(state, cx)),
            Phase::Complete | Phase::Saved => body.child(self.auth_complete(state)),
            Phase::Saving => body.child(components::progress(
                ProgressState::Indeterminate,
                self.palette,
                tr!(language, "common.in_progress"),
            )),
        };
        body.children(
            (self.busy && !state.phase.pending() && !state.phase.committed())
                .then(|| note(tr!(language, "accounts.auth.busy_notice"))),
        )
    }

    /// The account page: one sign-in form, with the other account types one
    /// disclosure away and switched in place.
    fn auth_connect(&self, state: &AuthWizard, cx: &Context<'_, Self>) -> gpui::Div {
        let language = self.language();
        let mut page = div()
            .flex()
            .flex_col()
            .gap_4()
            .child(self.auth_form(state, cx))
            .child(
                self.auth_button(
                    "auth-other-options",
                    if state.other_sign_in_options {
                        tr!(language, "accounts.auth.other.hide")
                    } else {
                        tr!(language, "accounts.auth.other.show")
                    },
                    UiAction::AuthOtherOptions,
                    ButtonVariant::Outline,
                    ControlState::default(),
                    cx,
                )
                // The label states the action; assistive technology reads the
                // disclosure's current state from this flag as well.
                .aria_expanded(state.other_sign_in_options),
            );
        if state.other_sign_in_options {
            page = page.child(
                div().flex().flex_col().gap_3().children(
                    ACCOUNT_TYPES
                        .iter()
                        .copied()
                        .filter(|provider| *provider != state.provider)
                        .map(|provider| self.provider_option(provider, cx)),
                ),
            );
        }
        page
    }

    /// One account type the page can switch its form to.
    fn provider_option(&self, provider: AccountProvider, cx: &Context<'_, Self>) -> gpui::Div {
        let language = self.language();
        components::card(self.palette)
            .gap_2()
            .child(self.auth_button(
                provider_selector(provider),
                provider_label(language, provider),
                UiAction::AuthProvider(provider),
                ButtonVariant::Outline,
                ControlState {
                    disabled: self.busy,
                    ..ControlState::default()
                },
                cx,
            ))
            .child(note(provider_description(language, provider)))
    }

    /// The form the selected account type signs in with.
    fn auth_form(&self, state: &AuthWizard, cx: &Context<'_, Self>) -> gpui::Div {
        match state.provider {
            AccountProvider::Microsoft => self.microsoft_form(cx),
            AccountProvider::Authlib => self.authlib_form(state, cx),
            AccountProvider::Offline => self.offline_form(state),
        }
    }

    /// The default page: one browser authorization, with the device code as an
    /// explicit action instead of a mode chosen by selecting a control.
    fn microsoft_form(&self, cx: &Context<'_, Self>) -> gpui::Div {
        let language = self.language();
        components::card(self.palette)
            .debug_selector(|| "auth-microsoft".into())
            .gap_3()
            .child(note(tr!(language, "accounts.auth.microsoft.card_note")))
            .child(self.auth_button(
                "auth-device-code-action",
                tr!(language, "accounts.auth.microsoft.device_action"),
                UiAction::AuthMethod(Method::Device),
                ButtonVariant::Outline,
                ControlState {
                    disabled: self.busy,
                    ..ControlState::default()
                },
                cx,
            ))
            .child(note(tr!(language, "accounts.auth.microsoft.device_note")))
    }

    /// A community provider signs in with its own account and password, sent to
    /// the address the form states.
    fn authlib_form(&self, state: &AuthWizard, cx: &Context<'_, Self>) -> gpui::Div {
        let language = self.language();
        let provider = self
            .shell
            .providers
            .records
            .iter()
            .find(|p| Some(p.id) == state.external_provider);
        let picker_value = provider.map_or_else(
            || tr!(language, "accounts.auth.provider.picker.empty"),
            |provider| provider.name.clone(),
        );
        let mut card = components::card(self.palette)
            .debug_selector(|| "auth-authlib".into())
            .gap_3()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(div().w_full().min_w_0().child(self.product_select_field(
                        "auth-provider-picker",
                        tr!(language, "select.auth_provider"),
                        &picker_value,
                        super::select::ProductChoice::AuthProvider,
                        cx,
                    )))
                    .child(self.auth_button(
                        "auth-provider-add",
                        tr!(language, "accounts.providers.add"),
                        UiAction::AuthProviderAdd,
                        ButtonVariant::Outline,
                        ControlState {
                            disabled: self.busy,
                            ..ControlState::default()
                        },
                        cx,
                    )),
            )
            // The address that receives the account and password stays visible.
            .children(provider.map(|p| note(p.api_url.clone())));
        if let Some(error) = &self.shell.providers.load_error {
            card = card.child(note(error.clone()));
        }
        if self.shell.providers.loaded && self.shell.providers.records.is_empty() {
            card = card.child(note(tr!(language, "accounts.auth.provider.none")));
        }
        if state.provider_draft.is_some() {
            card = card.child(note(tr!(language, "accounts.auth.provider.dialog_open")));
        }
        if provider.is_some_and(|provider| {
            state.external_api.as_deref() != Some(provider.api_url.as_str())
        }) {
            card = card.child(note(tr!(
                language,
                "accounts.auth.provider.address_changed"
            )));
        }
        if let Some(provider) = provider
            && provider.api_url.starts_with("http:")
        {
            card = card.child(
                div()
                    .p_3()
                    .rounded_lg()
                    .border_1()
                    .border_color(rgb(self.palette.destructive))
                    .child(note(tr!(language, "accounts.auth.provider.http_warning")))
                    .child(components::checkbox(
                        "auth-http-consent",
                        tr!(language, "accounts.auth.http_consent"),
                        state.accepted_http.as_deref() == Some(provider.api_url.as_str()),
                        self.palette,
                        ControlState {
                            disabled: self.busy,
                            ..ControlState::default()
                        },
                        cx.listener(|view, (), window, cx| {
                            view.handle(UiAction::AuthHttpConsent, window, cx);
                        }),
                    )),
            );
        }
        let mut fields = div().flex().flex_col().gap_4();
        if let Some(login) = &state.login {
            fields = fields.child(components::field_with(
                tr!(language, "accounts.auth.login.label"),
                login,
                self.palette,
                components::FieldOptions {
                    required: true,
                    ..components::FieldOptions::default()
                },
            ));
        }
        card.child(fields.child(self.password_field(state)))
            .child(note(tr!(language, "accounts.auth.password.note")))
    }

    /// The masked password editor beside the label that focuses it.
    fn password_field(&self, state: &AuthWizard) -> gpui::Div {
        let language = self.language();
        let password_focus = state.password.clone();
        div()
            .flex()
            .flex_col()
            .min_w_0()
            .gap_2()
            .child(
                div()
                    .id("auth-password-label")
                    .text_size(px(super::design::BODY_FONT_SIZE))
                    .line_height(px(super::design::BODY_LINE_HEIGHT))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .cursor(gpui::CursorStyle::IBeam)
                    .on_click(move |_, window, cx| {
                        if let Some(password) = &password_focus {
                            password.read(cx).focus_handle(cx).focus(window, cx);
                        }
                        cx.stop_propagation();
                    })
                    .child(format!(
                        "{} *",
                        tr!(language, "accounts.auth.password.label")
                    )),
            )
            .children(state.password.as_ref().map(|state| ProviderPassword {
                state: state.clone(),
                palette: self.palette,
            }))
    }

    /// A local identity is added from this form: its name is the whole input, so
    /// the page saves and selects it without another confirmation.
    fn offline_form(&self, state: &AuthWizard) -> gpui::Div {
        let language = self.language();
        let mut card = components::card(self.palette)
            .debug_selector(|| "auth-offline".into())
            .child(note(tr!(language, "accounts.auth.offline.card_note")));
        if let Some(field) = &state.username {
            card = card.child(
                components::field_with(
                    tr!(language, "accounts.auth.offline.name.label"),
                    field,
                    self.palette,
                    components::FieldOptions {
                        required: true,
                        ..components::FieldOptions::default()
                    },
                )
                .id("auth-offline-name")
                .debug_selector(|| "auth-offline-name".into()),
            );
        }
        card.child(note(tr!(language, "accounts.auth.offline.name.rule")))
    }

    /// The failure of the attempt this page started, with the form it started
    /// from still in reach.
    fn auth_failed(&self, state: &AuthWizard, cx: &Context<'_, Self>) -> gpui::Div {
        let language = self.language();
        let message = state.error.as_ref().map_or_else(
            || tr!(language, "accounts.auth.error.fallback"),
            |error| error.render(language),
        );
        if state.provider == AccountProvider::Offline {
            return div()
                .flex()
                .flex_col()
                .gap_3()
                .child(self.offline_form(state))
                .child(
                    div()
                        .flex()
                        .items_start()
                        .gap_2()
                        .text_sm()
                        .text_color(rgb(self.palette.destructive))
                        .child(Icon::Error.render(14., self.palette.destructive))
                        .child(div().min_w_0().flex_1().whitespace_normal().child(message)),
                );
        }
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                components::card(self.palette)
                    .border_color(rgb(self.palette.destructive))
                    .gap_2()
                    .child(Icon::Error.render(24., self.palette.destructive))
                    .child(note(message)),
            )
            .child(self.auth_form(state, cx))
    }

    /// The characters the provider returned; the chosen one becomes the account.
    fn auth_character_choice(&self, state: &AuthWizard, cx: &Context<'_, Self>) -> gpui::Div {
        let language = self.language();
        let mut choices = components::card(self.palette).gap_3();
        for character in &state.characters {
            let id = character.id;
            choices = choices
                .child(components::radio(
                    format!("auth-character-{id}"),
                    character.name.clone(),
                    state.character == Some(id),
                    self.palette,
                    ControlState {
                        disabled: self.busy,
                        ..ControlState::default()
                    },
                    cx.listener(move |view, (), window, cx| {
                        view.handle(UiAction::AuthCharacter(id), window, cx)
                    }),
                ))
                .child(note(character.id.to_string()));
        }
        div().flex().flex_col().gap_3().child(choices).children(
            state
                .error
                .as_ref()
                .map(|error| note(error.render(language))),
        )
    }

    fn auth_waiting(&self, state: &AuthWizard, cx: &Context<'_, Self>) -> gpui::Div {
        let language = self.language();
        if state.provider == AccountProvider::Authlib {
            return self.authlib_waiting(state, language);
        }
        if state.method == Method::Device && state.phase != Phase::Verifying {
            return self.device_waiting(state, language, cx);
        }
        self.browser_waiting(state, language, cx)
    }

    /// The waiting card for an external provider, which signs in without a browser handoff.
    fn authlib_waiting(&self, state: &AuthWizard, language: Language) -> gpui::Div {
        components::card(self.palette)
            .child(note(if state.phase == Phase::Verifying {
                tr!(language, "accounts.auth.waiting.authlib.verifying")
            } else {
                tr!(language, "accounts.auth.waiting.authlib.checking")
            }))
            .child(components::progress(
                ProgressState::Indeterminate,
                self.palette,
                tr!(language, "common.in_progress"),
            ))
    }

    /// The device-code card, which keeps the code and its address visible until sign-in finishes.
    fn device_waiting(
        &self,
        state: &AuthWizard,
        language: Language,
        cx: &Context<'_, Self>,
    ) -> gpui::Div {
        let mut card = components::card(self.palette).gap_3();
        card = if let Some(code) = &state.device_code {
            card.child(note(tr!(
                language,
                "accounts.auth.waiting.device.code_note"
            )))
            .child(self.device_code_row(code, language, cx))
            .child(self.device_open_row(code, language, cx))
            .child(note(tr!(
                language,
                "accounts.auth.waiting.device.keep_open"
            )))
        } else {
            card.child(note(tr!(
                language,
                "accounts.auth.waiting.device.requesting"
            )))
        };
        card.child(components::progress(
            ProgressState::Indeterminate,
            self.palette,
            tr!(language, "common.in_progress"),
        ))
        .child(self.auth_button(
            "auth-use-browser",
            tr!(language, "accounts.auth.device.browser_action"),
            UiAction::AuthMethod(Method::Browser),
            ButtonVariant::Outline,
            ControlState {
                disabled: self.busy,
                ..ControlState::default()
            },
            cx,
        ))
    }

    /// The issued device code beside the control that copies it.
    fn device_code_row(
        &self,
        code: &DeviceCode,
        language: Language,
        cx: &Context<'_, Self>,
    ) -> gpui::Div {
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .id("auth-device-code")
                    .text_2xl()
                    .child(code.user_code.clone()),
            )
            .child(self.auth_button(
                "auth-copy-code",
                tr!(language, "accounts.auth.waiting.device.copy"),
                UiAction::CopyDeviceCode,
                ButtonVariant::Outline,
                ControlState {
                    disabled: code.expires_at <= std::time::Instant::now(),
                    ..ControlState::default()
                },
                cx,
            ))
    }

    /// The verification address beside the control that reopens it.
    fn device_open_row(
        &self,
        code: &DeviceCode,
        language: Language,
        cx: &Context<'_, Self>,
    ) -> gpui::Div {
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .w_full()
                    .min_w_0()
                    .text_sm()
                    .child(code.verification_uri.clone()),
            )
            .child(self.auth_button(
                "auth-reopen",
                tr!(language, "accounts.auth.waiting.device.open"),
                UiAction::AuthReopen,
                ButtonVariant::Outline,
                ControlState::default(),
                cx,
            ))
    }

    /// The browser sign-in card, which reports the observed stage and offers a reopen control.
    fn browser_waiting(
        &self,
        state: &AuthWizard,
        language: Language,
        cx: &Context<'_, Self>,
    ) -> gpui::Div {
        let mut card = components::card(self.palette)
            .child(Icon::ExternalLink.render(32., self.palette.foreground))
            .child(note(tr!(language, "accounts.auth.waiting.ownership_note")))
            .child(components::progress(
                ProgressState::Indeterminate,
                self.palette,
                tr!(language, "common.in_progress"),
            ))
            .child(note(if state.method == Method::Device {
                tr!(language, "accounts.auth.waiting.device.moment")
            } else {
                tr!(language, "accounts.auth.waiting.moment")
            }));
        if let Some(stage) = state.stage {
            card = card
                .child(note(tr!(language, auth_stage_key(stage))))
                .children(self.stage_boundary_rows(stage, language));
        }
        if state.phase == Phase::Waiting && (state.browser_url.is_some() || self.visual_only) {
            card = card
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(self.auth_button(
                            "auth-reopen",
                            tr!(language, "accounts.auth.waiting.reopen"),
                            UiAction::AuthReopen,
                            ButtonVariant::Outline,
                            ControlState::default(),
                            cx,
                        ))
                        .child(self.auth_button(
                            "auth-use-device-code",
                            tr!(language, "accounts.auth.microsoft.device_action"),
                            UiAction::AuthMethod(Method::Device),
                            ButtonVariant::Outline,
                            ControlState {
                                disabled: self.busy,
                                ..ControlState::default()
                            },
                            cx,
                        )),
                )
                .child(note(tr!(language, "accounts.auth.waiting.reopen_note")));
        }
        card
    }

    /// One row per stage boundary, marked once the attempt reaches it.
    fn stage_boundary_rows(&self, stage: SignInStage, language: Language) -> Vec<gpui::Div> {
        [
            (
                SignInStage::Ownership,
                tr!(language, "accounts.auth.waiting.boundary.microsoft_xbox"),
            ),
            (
                SignInStage::PlayerProfile,
                tr!(language, "accounts.auth.waiting.boundary.ownership"),
            ),
            (
                SignInStage::SaveCredential,
                tr!(language, "accounts.auth.waiting.boundary.profile"),
            ),
            (
                SignInStage::SaveMetadata,
                tr!(language, "accounts.auth.waiting.boundary.credential"),
            ),
        ]
        .into_iter()
        .map(|(boundary, label)| self.stage_boundary_row(stage >= boundary, &label, language))
        .collect()
    }

    fn stage_boundary_row(&self, verified: bool, label: &str, language: Language) -> gpui::Div {
        let (icon, color) = if verified {
            (Icon::Check, self.palette.success)
        } else {
            (Icon::Info, self.palette.muted)
        };
        let text = if verified {
            tr!(language, "accounts.auth.waiting.verified", label = label)
        } else {
            tr!(language, "accounts.auth.waiting.pending", label = label)
        };
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(icon.render(14., color))
            .child(note(text))
    }

    fn auth_complete(&self, state: &AuthWizard) -> gpui::Div {
        let language = self.language();
        let Some(account) = &state.prepared_account else {
            return components::card(self.palette)
                .child(note(tr!(language, "accounts.auth.complete.waiting")));
        };
        components::card(self.palette)
            .child(components::account_avatar(
                &account.player_name,
                self.account_avatars.get(&account.identity_key()),
                self.palette,
                64.,
                AvatarState::Fallback,
            ))
            .child(div().text_xl().child(account.player_name.clone()))
            .child(components::badge(
                self.account_authority(account),
                self.palette,
                BadgeVariant::Secondary,
            ))
            .children(
                self.account_provider(account)
                    .map(|provider| note(provider.api_url.clone())),
            )
            .child(components::badge(
                if state.phase == Phase::Saved {
                    tr!(language, "accounts.auth.complete.badge.saved")
                } else {
                    tr!(language, "accounts.auth.complete.badge.ready")
                },
                self.palette,
                BadgeVariant::Success,
            ))
            .children(
                (state.phase == Phase::Saved)
                    .then(|| note(tr!(language, "accounts.auth.complete.note.saved"))),
            )
    }

    /// Whether the page has an earlier state to return to.
    fn can_go_back(state: &AuthWizard) -> bool {
        match state.phase {
            Phase::Connect => state.provider != AccountProvider::Microsoft,
            Phase::Complete | Phase::Saving | Phase::Saved => false,
            _ => true,
        }
    }

    /// Whether acting on this failure can change its outcome in this process.
    ///
    /// A missing engine and a system without any secure credential store are
    /// settled, so the page offers neither as a retry.
    fn can_retry(&self, state: &AuthWizard) -> bool {
        self.engine.is_some()
            && state.error.as_ref()
                != Some(&AccountMessage::product(
                    "accounts.auth.error.store_unsupported",
                ))
    }

    fn auth_footer(&self, state: &AuthWizard, cx: &Context<'_, Self>) -> gpui::Stateful<gpui::Div> {
        let language = self.language();
        let mut exits = div().w_full().flex().flex_col().gap_2();
        // A save in flight already authorizes the write, so no exit presents
        // itself as revoking it.
        if !state.phase.committed() {
            exits = exits
                .child(self.auth_button(
                    "auth-cancel",
                    if state.phase.pending() {
                        tr!(language, "accounts.auth.footer.cancel_sign_in")
                    } else {
                        tr!(language, "common.close")
                    },
                    UiAction::AuthCancel,
                    ButtonVariant::Ghost,
                    ControlState::default(),
                    cx,
                ))
                .children(Self::can_go_back(state).then(|| {
                    self.auth_button(
                        "auth-back",
                        tr!(language, "accounts.auth.footer.back"),
                        UiAction::AuthBack,
                        ButtonVariant::Ghost,
                        ControlState::default(),
                        cx,
                    )
                }));
        }
        let (label, action, busy) = match state.phase {
            Phase::Connect => (
                match state.provider {
                    AccountProvider::Offline => tr!(language, "accounts.auth.footer.add_and_use"),
                    AccountProvider::Authlib => tr!(language, "accounts.auth.footer.sign_in"),
                    AccountProvider::Microsoft => {
                        tr!(language, "accounts.auth.footer.open_browser")
                    }
                },
                UiAction::Authenticate,
                false,
            ),
            Phase::Preparing | Phase::Waiting | Phase::Verifying => (
                tr!(language, "accounts.auth.footer.waiting"),
                UiAction::Authenticate,
                true,
            ),
            Phase::Failed => (
                if state.save_failed && state.provider != AccountProvider::Offline {
                    tr!(language, "accounts.auth.footer.sign_in_again")
                } else {
                    tr!(language, "accounts.auth.footer.try_again")
                },
                UiAction::AuthRetry,
                false,
            ),
            Phase::Characters => (
                tr!(language, "accounts.auth.footer.continue"),
                UiAction::Authenticate,
                false,
            ),
            Phase::Complete => (
                tr!(language, "accounts.auth.footer.save_and_use"),
                UiAction::AuthFinish,
                false,
            ),
            Phase::Saving => (
                tr!(language, "accounts.auth.footer.saving"),
                UiAction::AuthFinish,
                true,
            ),
            Phase::Saved => (
                tr!(language, "accounts.auth.footer.done"),
                UiAction::AuthCancel,
                false,
            ),
        };
        let provider_blocked = state.provider == AccountProvider::Authlib
            && matches!(state.phase, Phase::Connect | Phase::Failed)
            && self
                .shell
                .providers
                .records
                .iter()
                .find(|p| Some(p.id) == state.external_provider)
                .is_none_or(|provider| {
                    state.external_api.as_deref() != Some(provider.api_url.as_str())
                        || provider.api_url.starts_with("http:")
                            && state.accepted_http.as_deref() != Some(provider.api_url.as_str())
                });
        let settled = state.phase == Phase::Failed && !self.can_retry(state);
        patterns::management_footer(self.palette)
            .id("auth-footer")
            .debug_selector(|| "auth-footer".into())
            .h_auto()
            .min_h(px(super::design::MANAGEMENT_FOOTER))
            .px_4()
            .py_3()
            .child(
                div()
                    .w_full()
                    .max_w(px(AUTH_CONTENT_WIDTH))
                    .mx_auto()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(exits)
                    .child(self.auth_button(
                        "auth-primary",
                        label,
                        action,
                        ButtonVariant::Primary,
                        ControlState {
                            busy,
                            disabled: state.provider_draft.is_some()
                                || provider_blocked
                                || settled
                                || self.busy
                                    && matches!(
                                        state.phase,
                                        Phase::Connect
                                            | Phase::Failed
                                            | Phase::Complete
                                            | Phase::Characters
                                    ),
                            ..ControlState::default()
                        },
                        cx,
                    )),
            )
    }

    fn auth_button(
        &self,
        id: &'static str,
        label: impl Into<gpui::SharedString>,
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
        .w_full()
        .min_w_0()
        .h_auto()
        .min_h(px(super::design::CONTROL_HEIGHT))
        .whitespace_normal()
    }
}

/// The semantic keys of the heading and description for the page state.
///
/// The keys stay unresolved so the view renders them in its current language,
/// including after a language change.
fn auth_heading_keys(state: &AuthWizard) -> (&'static str, &'static str) {
    match state.phase {
        Phase::Connect => match state.provider {
            AccountProvider::Offline => (
                "accounts.auth.info.title.offline",
                "accounts.auth.info.description.offline",
            ),
            AccountProvider::Authlib => (
                "accounts.auth.info.title.authlib",
                "accounts.auth.info.description.authlib",
            ),
            AccountProvider::Microsoft => (
                "accounts.auth.info.title.microsoft",
                "accounts.auth.info.description.microsoft",
            ),
        },
        Phase::Preparing => (
            "accounts.auth.preparing.title",
            if state.provider == AccountProvider::Authlib {
                "accounts.auth.preparing.description.authlib"
            } else {
                "accounts.auth.preparing.description.microsoft"
            },
        ),
        Phase::Waiting if state.method == Method::Device => (
            "accounts.auth.waiting.device.title",
            "accounts.auth.waiting.device.description",
        ),
        Phase::Waiting => (
            "accounts.auth.waiting.browser.title",
            "accounts.auth.waiting.browser.description",
        ),
        Phase::Verifying => (
            "accounts.auth.verifying.title",
            if state.provider == AccountProvider::Authlib {
                "accounts.auth.verifying.description.authlib"
            } else {
                "accounts.auth.verifying.description.microsoft"
            },
        ),
        Phase::Characters => (
            "accounts.auth.characters.title",
            "accounts.auth.characters.description",
        ),
        Phase::Failed if state.provider == AccountProvider::Offline => (
            "accounts.auth.failed.offline.title",
            "accounts.auth.failed.offline.description",
        ),
        Phase::Failed => (
            "accounts.auth.failed.title",
            "accounts.auth.failed.description",
        ),
        Phase::Complete => (
            "accounts.auth.complete.title",
            "accounts.auth.complete.description.online",
        ),
        Phase::Saving => (
            "accounts.auth.saving.title",
            if state.provider == AccountProvider::Offline {
                "accounts.auth.saving.description.offline"
            } else {
                "accounts.auth.saving.description.online"
            },
        ),
        Phase::Saved => (
            "accounts.auth.saved.title",
            "accounts.auth.saved.description",
        ),
    }
}

/// Classify a raw sign-in failure into the product recovery text it needs.
///
/// The backend reports failures as English diagnostics; classification reads
/// those diagnostics, while the message the user sees is a semantic key the
/// view renders in its own language.
fn sign_in_error(error: &str) -> AccountMessage {
    let lower = error.to_lowercase();
    if lower.contains("cancel") {
        AccountMessage::product("accounts.auth.error.cancelled")
    } else if lower.contains("timeout")
        || lower.contains("timed out")
        || lower.contains("expired")
        // A prepared identity is single use: once the save took it there is
        // nothing left to persist and only a fresh sign-in can help.
        || lower.contains("no longer available")
        || lower.contains("prepared sign-in is unavailable")
    {
        AccountMessage::product("accounts.auth.error.expired")
    } else if lower.contains("ownership")
        || lower.contains("does not own")
        || lower.contains("entitlement")
    {
        AccountMessage::product("accounts.auth.error.ownership")
    } else if lower.contains("no minecraft: java edition player profile") {
        AccountMessage::product("accounts.auth.error.player_profile")
    } else if lower.contains("create an xbox profile") {
        AccountMessage::product("accounts.auth.error.xbox_profile")
    } else if lower.contains("family restrictions") || lower.contains("xbox account permissions") {
        AccountMessage::product("accounts.auth.error.family")
    } else if lower.contains("different microsoft application registration") {
        AccountMessage::product("accounts.auth.error.registration_other")
    } else if lower.contains("application registration") || lower.contains("client id") {
        AccountMessage::product("accounts.auth.error.registration")
    } else if lower.contains("secure credential store is unavailable or locked") {
        if cfg!(target_os = "linux") {
            AccountMessage::product("accounts.auth.error.store_unavailable.linux")
        } else if cfg!(target_os = "windows") {
            AccountMessage::product("accounts.auth.error.store_unavailable.windows")
        } else {
            AccountMessage::product("accounts.auth.error.store_unavailable.macos")
        }
    } else if lower.contains("no supported secure credential store") {
        AccountMessage::product("accounts.auth.error.store_unsupported")
    } else if lower.contains("metadata")
        || lower.contains("secure storage")
        || lower.contains("credential")
    {
        AccountMessage::product("accounts.auth.error.save_failed")
    } else if lower.contains("engine unavailable") {
        AccountMessage::product("accounts.auth.error.engine_unavailable")
    } else {
        AccountMessage::product("accounts.auth.error.generic")
    }
}

const fn auth_stage_key(stage: SignInStage) -> &'static str {
    match stage {
        SignInStage::SecureStorage => "accounts.auth.stage.secure_storage",
        SignInStage::DeviceAuthorization => "accounts.auth.stage.device_code",
        SignInStage::BrowserAuthorization => "accounts.auth.stage.browser",
        SignInStage::MicrosoftAuthorization => "accounts.auth.stage.microsoft",
        SignInStage::XboxAuthentication => "accounts.auth.stage.xbox_auth",
        SignInStage::XboxAuthorization => "accounts.auth.stage.xbox_authorization",
        SignInStage::MinecraftAuthentication => "accounts.auth.stage.minecraft",
        SignInStage::Ownership => "accounts.auth.stage.ownership",
        SignInStage::PlayerProfile => "accounts.auth.stage.profile",
        SignInStage::SaveCredential => "accounts.auth.stage.credential",
        SignInStage::SaveMetadata => "accounts.auth.stage.metadata",
    }
}

/// The product name of an account's authentication authority.
pub(super) fn provider_label(language: Language, provider: AccountProvider) -> String {
    let key = match provider {
        AccountProvider::Microsoft => "accounts.provider.microsoft",
        AccountProvider::Authlib => "accounts.provider.authlib",
        AccountProvider::Offline => "accounts.provider.offline",
    };
    tr!(language, key)
}

/// What signing in with an account type means, in the reader's language.
fn provider_description(language: Language, provider: AccountProvider) -> String {
    let key = match provider {
        AccountProvider::Microsoft => "accounts.auth.provider.microsoft.description",
        AccountProvider::Authlib => "accounts.auth.provider.authlib.description",
        AccountProvider::Offline => "accounts.auth.provider.offline.description",
    };
    tr!(language, key)
}

/// The stable selector of an account type on this page.
///
/// Exactly one element carries it at a time: the type's form while it is the one
/// in use, or its row in the alternatives list.
const fn provider_selector(provider: AccountProvider) -> &'static str {
    match provider {
        AccountProvider::Microsoft => "auth-microsoft",
        AccountProvider::Authlib => "auth-authlib",
        AccountProvider::Offline => "auth-offline",
    }
}

#[cfg(test)]
mod tests {
    use gpui::{Focusable as _, TestAppContext, px, size};

    use super::{AuthWizard, Phase};
    use crate::{
        Form, Tab,
        test_support::{scroll_into_view, test_account},
    };

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

    #[test]
    fn secure_store_preflight_errors_explain_system_recovery_without_private_details() {
        let error = block_auth::AuthError::CredentialStoreUnavailable.to_string();
        let message = super::sign_in_error(&format!("{error} PRIVATE_DIAGNOSTIC"))
            .render(crate::Language::English);
        assert!(message.contains(if cfg!(target_os = "linux") {
            "Secret Service"
        } else if cfg!(target_os = "windows") {
            "Windows Credential Manager"
        } else {
            "login keychain"
        }));
        assert!(message.contains("try again"));
        assert!(!message.contains("PRIVATE_DIAGNOSTIC"));
        assert!(!message.contains("could not save the account"));
        let unsupported =
            super::sign_in_error(&block_auth::AuthError::UnsupportedSecureStore.to_string())
                .render(crate::Language::English);
        assert!(unsupported.contains("sign-in is unavailable"));
        assert_ne!(
            message,
            super::sign_in_error("Account metadata cannot be written")
                .render(crate::Language::English)
        );
    }

    #[test]
    fn sign_in_failures_keep_the_classified_key_and_render_each_language() {
        use crate::Language::{English, Japanese, SimplifiedChinese};

        let cases = [
            (
                "Sign-in was cancelled by the user",
                "accounts.auth.error.cancelled",
            ),
            ("The sign-in timed out", "accounts.auth.error.expired"),
            (
                "Java Edition ownership could not be found",
                "accounts.auth.error.ownership",
            ),
            (
                "Create an Xbox profile for this account",
                "accounts.auth.error.xbox_profile",
            ),
            (
                "This provider uses a different Microsoft application registration",
                "accounts.auth.error.registration_other",
            ),
            (
                "Engine unavailable",
                "accounts.auth.error.engine_unavailable",
            ),
        ];
        for (raw, expected) in cases {
            let message = super::sign_in_error(raw);
            assert_eq!(message, super::AccountMessage::product(expected), "{raw}");
            assert_ne!(message.render(English), message.render(Japanese), "{raw}");
            assert_ne!(
                message.render(English),
                message.render(SimplifiedChinese),
                "{raw}"
            );
        }
        // Unclassified failures still produce the product recovery text rather
        // than the backend's diagnostic.
        assert_eq!(
            super::sign_in_error("unexpected backend detail"),
            super::AccountMessage::product("accounts.auth.error.generic")
        );
        assert_eq!(
            super::sign_in_error("The sign-in timed out").render(Japanese),
            "認可の有効期限が切れました。サインインをやり直して新しいセッションを要求してください。"
        );
        assert_eq!(
            super::sign_in_error("Sign-in was cancelled by the user").render(SimplifiedChinese),
            "登录已取消。你可以随时重试。"
        );
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
    fn inline_provider_drafts_keep_the_parent_login_and_select_duplicates_without_saving(
        cx: &mut TestAppContext,
    ) {
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                let provider = crate::providers::fixture_provider(false);
                assert!(provider.is_ok());
                let Ok(provider) = provider else {
                    return;
                };
                view.shell.providers.records = vec![provider.clone()];
                view.shell.providers.loaded = true;
                view.open_auth_wizard(window, cx);
                view.choose_auth_provider(block_auth::AccountProvider::Authlib, cx);
                let password = view
                    .shell
                    .auth
                    .as_ref()
                    .and_then(|state| state.password.clone());
                assert!(password.is_some());
                let Some(password) = password else {
                    return;
                };
                password.update(cx, |field, cx| {
                    field.set_value("PARENT_SYNTHETIC_PASSWORD", window, cx)
                });
                view.open_auth_provider_add(window, cx);
                if let Some(draft) = view
                    .shell
                    .auth
                    .as_mut()
                    .and_then(|state| state.provider_draft.as_mut())
                {
                    draft
                        .name
                        .update(cx, |field, cx| field.set_value("New provider", cx));
                    draft.address.update(cx, |field, cx| {
                        field.set_value("http://new.example.test/api/", cx)
                    });
                }
                view.save_auth_provider(window, cx);
                assert!(
                    view.shell
                        .auth
                        .as_ref()
                        .and_then(|state| state.provider_draft.as_ref())
                        .is_some_and(|draft| draft.error.as_ref().is_some_and(|error| error
                            .render(crate::Language::English)
                            .contains("HTTP warning")))
                );
                assert!(!view.auth_provider_saved(&provider.id.to_string(), cx));
                view.cancel_auth_provider(cx);
                assert_eq!(
                    password.read(cx).value().as_ref(),
                    "PARENT_SYNTHETIC_PASSWORD"
                );
                assert!(view.shell.auth.as_ref().is_some_and(|state| state.phase
                    == Phase::Connect
                    && state.provider_draft.is_none()));
                view.open_auth_provider_add(window, cx);
                if let Some(draft) = view
                    .shell
                    .auth
                    .as_mut()
                    .and_then(|state| state.provider_draft.as_mut())
                {
                    draft
                        .address
                        .update(cx, |field, cx| field.set_value(&provider.api_url, cx));
                }
                view.save_auth_provider(window, cx);
                assert_eq!(view.shell.providers.records.len(), 1);
                assert_eq!(password.read(cx).value().as_ref(), "");
                assert!(
                    view.shell
                        .auth
                        .as_ref()
                        .is_some_and(|state| state.provider_draft.is_none()
                            && state.external_provider == Some(provider.id))
                );
                view.open_auth_provider_add(window, cx);
                let pending = block_launcher::CancellationToken::new();
                if let Some(draft) = view
                    .shell
                    .auth
                    .as_mut()
                    .and_then(|state| state.provider_draft.as_mut())
                {
                    draft.attempt = Some((view.operation_revision, pending.clone()));
                }
                view.busy = true;
                view.auth_back(window, cx);
                assert!(pending.is_cancelled());
                assert!(view.shell.auth.as_ref().is_some_and(|state| state.phase
                    == Phase::Connect
                    && state.provider_draft.is_none()));
                assert!(!view.auth_provider_saved(&provider.id.to_string(), cx));
                view.cancel_auth(window, cx);
            })
        });
    }

    #[gpui::test]
    fn external_provider_changes_clear_sensitive_drafts_and_rebind_http_consent(
        cx: &mut TestAppContext,
    ) {
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                let providers = [
                    crate::providers::fixture_provider(false),
                    crate::providers::fixture_provider(true),
                ]
                .into_iter()
                .collect::<anyhow::Result<Vec<_>>>();
                assert!(providers.is_ok());
                let Ok(providers) = providers else {
                    return;
                };
                view.shell.providers.records = providers;
                view.shell.providers.loaded = true;
                view.open_auth_wizard(window, cx);
                view.choose_auth_provider(block_auth::AccountProvider::Authlib, cx);
                let password = view
                    .shell
                    .auth
                    .as_ref()
                    .and_then(|state| state.password.clone());
                assert!(password.is_some());
                let Some(password) = password else {
                    return;
                };
                password.update(cx, |input, cx| {
                    input.set_value("SYNTHETIC_PASSWORD", window, cx)
                });
                let snapshot = view.shell.auth.as_ref().map(|state| state.snapshot(cx));
                assert!(snapshot.is_some_and(|state| state.password.is_none()));
                assert!(!format!("{:?}", view.shell.auth).contains("SYNTHETIC_PASSWORD"));
                view.choose_external_provider(uuid::Uuid::from_u128(2), window, cx);
                assert_eq!(password.read(cx).value().as_ref(), "");
                assert!(
                    view.shell
                        .auth
                        .as_ref()
                        .is_some_and(|state| state.accepted_http.is_none())
                );
                view.consent_external_auth(cx);
                assert!(
                    view.shell
                        .auth
                        .as_ref()
                        .is_some_and(|state| state.accepted_http.as_deref()
                            == Some("http://community.example.test/api/"))
                );
                password.update(cx, |input, cx| {
                    input.set_value("SECOND_SYNTHETIC_PASSWORD", window, cx)
                });
                view.shell.providers.records[1].api_url =
                    "http://community.example.test/changed/".into();
                view.start_auth(window, cx);
                assert_eq!(password.read(cx).value().as_ref(), "");
                assert!(view.shell.auth.as_ref().is_some_and(|state| {
                    state.phase == Phase::Failed
                        && state.error.as_ref().is_some_and(|error| {
                            error
                                .render(crate::Language::English)
                                .contains("provider address changed")
                        })
                }));
                view.auth_back(window, cx);
                assert_eq!(password.read(cx).value().as_ref(), "");
                // Back from a failed attempt returns to the page the attempt
                // started on, which is still the community provider's form.
                assert!(view.shell.auth.as_ref().is_some_and(|state| {
                    state.phase == Phase::Connect
                        && state.provider == block_auth::AccountProvider::Authlib
                }));
                view.auth_back(window, cx);
                // The alternatives lead back to the default Microsoft page.
                assert!(view.shell.auth.as_ref().is_some_and(|state| {
                    state.phase == Phase::Connect
                        && state.provider == block_auth::AccountProvider::Microsoft
                }));
                view.cancel_auth(window, cx);
                assert!(view.shell.auth.is_none());
            })
        });
    }

    #[gpui::test]
    fn external_characters_require_a_live_attempt_and_preparation_matches_the_selected_provider(
        cx: &mut TestAppContext,
    ) {
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.open_auth_wizard(window, cx);
                view.choose_auth_provider(block_auth::AccountProvider::Authlib, cx);
                if let Some(state) = &mut view.shell.auth {
                    state.external_provider = Some(uuid::Uuid::from_u128(1));
                }
                let profiles = vec![
                    block_auth::AuthlibProfile {
                        id: uuid::Uuid::from_u128(1),
                        name: "First".into(),
                    },
                    block_auth::AuthlibProfile {
                        id: uuid::Uuid::from_u128(2),
                        name: "Second".into(),
                    },
                ];
                view.auth_characters(profiles.clone());
                assert!(
                    view.shell
                        .auth
                        .as_ref()
                        .is_some_and(|state| state.characters.is_empty())
                );
                simulate_started_attempt(view, Phase::Preparing);
                view.auth_characters(profiles.clone());
                view.busy = false;
                view.choose_auth_character(uuid::Uuid::from_u128(999), cx);
                assert!(
                    view.shell
                        .auth
                        .as_ref()
                        .is_some_and(|state| state.character == Some(uuid::Uuid::from_u128(1)))
                );
                view.choose_auth_character(uuid::Uuid::from_u128(2), cx);
                assert!(
                    view.shell
                        .auth
                        .as_ref()
                        .is_some_and(|state| state.character == Some(uuid::Uuid::from_u128(2)))
                );
                simulate_started_attempt(view, Phase::Verifying);
                view.auth_finished(&Err("Provider timeout".into()));
                view.busy = false;
                assert!(
                    view.shell
                        .auth
                        .as_ref()
                        .is_some_and(|state| state.phase == Phase::Characters
                            && state.character == Some(uuid::Uuid::from_u128(2)))
                );
                simulate_started_attempt(view, Phase::Verifying);
                let account = test_account();
                assert!(account.is_ok());
                let Ok(mut account) = account else {
                    return;
                };
                account.provider = block_auth::AccountProvider::Authlib;
                account.authlib = Some(block_auth::AuthlibAccount {
                    provider_id: uuid::Uuid::from_u128(99),
                    login: "User".into(),
                });
                account.client_id.clear();
                account.credential_ref = account.identity_key();
                view.auth_prepared(account.clone());
                assert!(
                    view.shell
                        .auth
                        .as_ref()
                        .is_some_and(|state| state.prepared_account.is_none())
                );
                if let Some(link) = &mut account.authlib {
                    link.provider_id = uuid::Uuid::from_u128(1);
                }
                account.credential_ref = account.identity_key();
                view.auth_prepared(account.clone());
                assert!(
                    view.shell
                        .auth
                        .as_ref()
                        .is_some_and(|state| state.phase == Phase::Complete)
                );
                assert!(
                    !view
                        .accounts
                        .iter()
                        .any(|saved| saved.identity_key() == account.identity_key())
                );
                view.cancel_auth(window, cx);
                view.auth_characters(profiles);
                view.auth_prepared(account);
                assert!(view.shell.auth.is_none());
            })
        });
    }

    #[gpui::test]
    fn a_local_identity_is_validated_and_saved_by_its_own_name_page(cx: &mut TestAppContext) {
        let result = (|| -> anyhow::Result<()> {
            let root = tempfile::tempdir()?;
            let (engine, messages) = crate::engine::Engine::with_root(root.path().to_owned())?;
            let (view, cx) = crate::test_support::window(cx, move |window, cx| {
                let mut view = crate::tests::test_view(window, cx);
                view.engine = Some(engine);
                view.messages = Some(messages);
                view
            });
            cx.update(|window, cx| {
                view.update(cx, |view, cx| {
                    view.preferences.reduce_motion = true;
                    view.router.navigate(Tab::Accounts);
                    view.open_auth_wizard(window, cx);
                    view.choose_auth_provider(block_auth::AccountProvider::Offline, cx);
                    view.start_auth(window, cx);
                });
            });
            // An unusable name keeps the page on its own form and states the
            // rule it broke instead of a backend diagnostic.
            assert!(view.read_with(cx, |view, _| {
                view.shell.auth.as_ref().is_some_and(|state| {
                    state.phase == Phase::Failed
                        && state.prepared_account.is_none()
                        && state.error
                            == Some(super::AccountMessage::product(
                                "accounts.auth.error.offline_name",
                            ))
                }) && view.accounts.is_empty()
            }));
            let account = crate::engine::offline_account("Local_Player")?;
            let id = crate::account_id(&account);
            cx.update(|window, cx| {
                view.update(cx, |view, cx| {
                    if let Some(field) = view
                        .shell
                        .auth
                        .as_ref()
                        .and_then(|state| state.username.clone())
                    {
                        field.update(cx, |field, cx| field.set_value("Local_Player", cx));
                    }
                    view.start_auth(window, cx);
                });
            });
            // The name page adds and uses the identity: it saves directly and
            // never asks for a separate confirmation.
            assert!(view.read_with(cx, |view, _| {
                view.shell.auth.as_ref().is_some_and(|state| {
                    state.phase == Phase::Saving
                        && state.prepared_account.as_ref().is_some_and(|prepared| {
                            prepared.identity_key() == account.identity_key()
                        })
                })
            }));
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while view.read_with(cx, |view, _| view.shell.auth.is_some())
                && std::time::Instant::now() < deadline
            {
                cx.update(|_, cx| {
                    view.update(cx, |view, cx| view.poll(cx));
                });
                cx.run_until_parked();
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            assert!(view.read_with(cx, |view, _| {
                view.shell.auth.is_none()
                    && view.selected_account.as_deref() == Some(id.as_str())
                    && view
                        .accounts
                        .iter()
                        .any(|saved| saved.identity_key() == account.identity_key())
            }));
            let saved = crate::global_config::read(root.path())?;
            assert!(saved.is_some_and(|config| {
                config
                    .accounts
                    .iter()
                    .any(|saved| saved.identity_key() == account.identity_key())
            }));
            Ok(())
        })();
        assert!(result.is_ok(), "{result:?}");
    }

    #[gpui::test]
    fn leaving_the_local_name_page_before_adding_saves_nothing(cx: &mut TestAppContext) {
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.router.navigate(Tab::Accounts);
                view.open_auth_wizard(window, cx);
                view.choose_auth_provider(block_auth::AccountProvider::Offline, cx);
                if let Some(field) = view
                    .shell
                    .auth
                    .as_ref()
                    .and_then(|state| state.username.clone())
                {
                    field.update(cx, |field, cx| field.set_value("Typed_Only", cx));
                }
                view.cancel_auth(window, cx);
            });
        });
        // A name nobody added is not an identity: closing the page saves nothing
        // and selects nothing.
        assert!(view.read_with(cx, |view, _| {
            view.shell.auth.is_none()
                && view.accounts.is_empty()
                && view.selected_account.is_none()
                && !view.busy
        }));
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
            error: Some(super::AccountMessage::diagnostic("sensitive-error-detail")),
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
                view.preferences.reduce_motion = true;
                view.begin_first_run(cx);
                view.handle_first_run(crate::first_run::Action::Custom, window, cx);
                if let Some(Form::Instance(form)) = &view.form {
                    form.name
                        .update(cx, |input, cx| input.set_value("世界 draft", cx));
                }
                view.handle_first_run(crate::first_run::Action::Back, window, cx);
                view.open_auth_wizard(window, cx);
            });
        });
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| view.shell.auth.is_some()));
        crate::test_support::press(cx, "escape");
        assert!(view.read_with(cx, |view, cx| {
            view.shell.auth.is_none()
                && view.router.tab() == Tab::Onboarding
                && view.shell.first_run.stage == crate::first_run::Stage::Overview
                && matches!(&view.form, Some(Form::Instance(form)) if form.name.read(cx).value() == "世界 draft")
        }));
        cx.update(|window, cx| {
            view.read_with(cx, |view, _| assert!(view.focus.is_focused(window)));
        });
    }

    #[gpui::test]
    fn onboarding_account_choices_disclose_other_providers_on_request(cx: &mut TestAppContext) {
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.router.navigate(Tab::Onboarding);
                view.open_auth_wizard(window, cx);
            });
        });
        cx.run_until_parked();
        assert!(
            view.read_with(cx, |view, _| view
                .shell
                .auth
                .as_ref()
                .is_some_and(|state| state.provider
                    == block_auth::AccountProvider::Microsoft
                    && !state.other_sign_in_options))
        );
        assert!(crate::test_support::control_bounds(cx, "auth-other-options").is_some());
        assert!(crate::test_support::control_bounds(cx, "auth-authlib").is_none());
        view.update(cx, |view, cx| view.toggle_auth_other_options(cx));
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| {
            view.shell
                .auth
                .as_ref()
                .is_some_and(|state| state.other_sign_in_options)
        }));
        assert!(crate::test_support::control_bounds(cx, "auth-authlib").is_some());
        assert!(crate::test_support::control_bounds(cx, "auth-offline").is_some());
        // Onboarding owns the same two-way disclosure as every other entry.
        view.update(cx, |view, cx| view.toggle_auth_other_options(cx));
        cx.run_until_parked();
        assert!(crate::test_support::control_bounds(cx, "auth-other-options").is_some());
        assert!(crate::test_support::control_bounds(cx, "auth-authlib").is_none());
        assert!(crate::test_support::control_bounds(cx, "auth-offline").is_none());
        assert!(
            view.read_with(cx, |view, _| view
                .shell
                .auth
                .as_ref()
                .is_some_and(|state| state.provider
                    == block_auth::AccountProvider::Microsoft
                    && !state.other_sign_in_options))
        );
    }

    #[gpui::test]
    fn the_account_page_switches_its_form_in_place_behind_one_disclosure(cx: &mut TestAppContext) {
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.preferences.reduce_motion = true;
                view.router.navigate(Tab::Accounts);
                view.open_auth_wizard(window, cx);
            });
        });
        cx.simulate_resize(size(px(360.), px(720.)));
        cx.run_until_parked();
        let body = crate::test_support::control_bounds(cx, "auth-body");
        // The Accounts entry is the Microsoft form itself, with the device code
        // as an action on it and the other account types collapsed.
        assert!(
            view.read_with(cx, |view, _| view.shell.auth.as_ref().is_some_and(
                |state| state.phase == Phase::Connect
                    && state.provider == block_auth::AccountProvider::Microsoft
                    && !state.other_sign_in_options
            ))
        );
        assert_eq!(
            view.read_with(cx, |view, _| view.router.auth_return()),
            Some(Tab::Accounts)
        );
        assert!(crate::test_support::control_bounds(cx, "auth-microsoft").is_some());
        assert!(crate::test_support::control_bounds(cx, "auth-device-code-action").is_some());
        assert!(crate::test_support::control_bounds(cx, "auth-other-options").is_some());
        assert!(crate::test_support::control_bounds(cx, "auth-authlib").is_none());
        assert!(crate::test_support::control_bounds(cx, "auth-offline").is_none());

        // One expansion reveals both alternatives without changing the form.
        assert!(body.is_some_and(|body| scroll_into_view(cx, body, "auth-other-options")));
        click(cx, "auth-other-options");
        assert!(crate::test_support::control_bounds(cx, "auth-authlib").is_some());
        assert!(crate::test_support::control_bounds(cx, "auth-offline").is_some());
        assert!(view.read_with(cx, |view, _| {
            view.shell
                .auth
                .as_ref()
                .is_some_and(|state| state.provider == block_auth::AccountProvider::Microsoft)
        }));

        // Choosing one swaps the form on this page instead of opening a step.
        assert!(body.is_some_and(|body| scroll_into_view(cx, body, "auth-offline")));
        click(cx, "auth-offline");
        assert!(view.read_with(cx, |view, _| {
            view.shell.auth.as_ref().is_some_and(|state| {
                state.phase == Phase::Connect
                    && state.provider == block_auth::AccountProvider::Offline
            })
        }));
        assert!(crate::test_support::control_bounds(cx, "auth-offline").is_some());
        // Microsoft is now an alternative, listed by the same disclosure.
        assert!(crate::test_support::control_bounds(cx, "auth-microsoft").is_some());
        cx.update(|_, cx| {
            view.update(cx, |view, cx| {
                if let Some(field) = view
                    .shell
                    .auth
                    .as_ref()
                    .and_then(|state| state.username.clone())
                {
                    field.update(cx, |field, cx| field.set_value("Local_Draft", cx));
                }
            });
        });

        // Collapsing hides the alternatives and keeps the form in use with its
        // typed draft intact.
        assert!(body.is_some_and(|body| scroll_into_view(cx, body, "auth-other-options")));
        click(cx, "auth-other-options");
        assert!(crate::test_support::control_bounds(cx, "auth-authlib").is_none());
        assert!(crate::test_support::control_bounds(cx, "auth-microsoft").is_none());
        assert!(
            view.read_with(cx, |view, cx| view.shell.auth.as_ref().is_some_and(
                |state| state.provider == block_auth::AccountProvider::Offline
                    && state
                        .username
                        .as_ref()
                        .is_some_and(|field| field.read(cx).value() == "Local_Draft")
            ))
        );

        // Back returns the page to its default Microsoft form and keeps both the
        // disclosure state and the draft for the next visit.
        cx.update(|window, cx| {
            view.update(cx, |view, cx| view.auth_back(window, cx));
        });
        cx.run_until_parked();
        assert!(
            view.read_with(cx, |view, cx| view.shell.auth.as_ref().is_some_and(
                |state| state.phase == Phase::Connect
                    && state.provider == block_auth::AccountProvider::Microsoft
                    && !state.other_sign_in_options
                    && state
                        .username
                        .as_ref()
                        .is_some_and(|field| field.read(cx).value() == "Local_Draft")
            ))
        );
        assert!(crate::test_support::control_bounds(cx, "auth-microsoft").is_some());
    }

    #[gpui::test]
    fn the_page_requests_authorization_only_when_its_own_action_is_activated(
        cx: &mut TestAppContext,
    ) {
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.preferences.reduce_motion = true;
                view.router.navigate(Tab::Accounts);
                view.open_auth_wizard(window, cx);
            });
        });
        cx.run_until_parked();
        // Showing the page requests nothing: no authorization URL and no device
        // code exist until an action asks for one.
        assert!(view.read_with(cx, |view, _| {
            view.shell.auth.as_ref().is_some_and(|state| {
                state.phase == Phase::Connect
                    && state.method == super::Method::Browser
                    && state.browser_url.is_none()
                    && state.device_code.is_none()
            }) && !view.busy
        }));
        // The device code is an action of the page, not a mode selected by a
        // control, so no browser/device selection control exists.
        assert!(crate::test_support::control_bounds(cx, "auth-method-browser").is_none());
        assert!(crate::test_support::control_bounds(cx, "auth-method-device").is_none());
        assert!(crate::test_support::control_bounds(cx, "auth-device-code-action").is_some());
        assert!(crate::test_support::control_bounds(cx, "auth-primary").is_some());
        assert!(
            view.read_with(cx, |view, _| view.shell.auth.as_ref().is_some_and(
                |state| state.method == super::Method::Browser && state.device_code.is_none()
            ))
        );
        // Activating the page asks the backend for a browser authorization; the
        // network-free fixture has no backend, so it reports that instead of
        // falling back to a device code or claiming a session.
        click(cx, "auth-primary");
        assert!(view.read_with(cx, |view, _| {
            view.shell.auth.as_ref().is_some_and(|state| {
                state.phase == Phase::Failed
                    && state.error
                        == Some(super::AccountMessage::product(
                            "accounts.auth.error.microsoft_start",
                        ))
                    && state.device_code.is_none()
            }) && !view.busy
        }));
        assert_eq!(cx.opened_url(), None);
        // The device code is requested by its own explicit action, and only it
        // can produce one.
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.change_auth_method(super::Method::Device, window, cx);
            });
        });
        assert!(view.read_with(cx, |view, _| {
            view.shell.auth.as_ref().is_some_and(|state| {
                state.method == super::Method::Device
                    && state.device_code.is_none()
                    && state.phase == Phase::Failed
            })
        }));
        assert_eq!(cx.opened_url(), None);
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
                    view.preferences.reduce_motion = true;
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
                    state.phase == Phase::Connect && state.prepared_account.is_none()
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
    fn first_run_account_finish_saves_durably_and_cancel_after_save_keeps_it(
        cx: &mut TestAppContext,
    ) {
        let result = (|| -> anyhow::Result<()> {
            let root = tempfile::tempdir()?;
            let account = crate::engine::offline_account("Local_Player")?;
            let account_id = crate::account_id(&account);
            let (engine, messages) = crate::engine::Engine::with_root(root.path().to_owned())?;
            let (view, cx) = crate::test_support::window(cx, move |window, cx| {
                let mut view = crate::tests::test_view(window, cx);
                view.engine = Some(engine);
                view.messages = Some(messages);
                view
            });
            cx.update(|window, cx| {
                view.update(cx, |view, cx| {
                    view.preferences.reduce_motion = true;
                    view.begin_first_run(cx);
                    view.open_auth_wizard(window, cx);
                    if let Some(state) = &mut view.shell.auth {
                        state.provider = block_auth::AccountProvider::Offline;
                        state.prepared_account = Some(account.clone());
                        state.transition(Phase::Complete);
                    }
                    cx.notify();
                });
            });
            cx.update(|window, cx| {
                view.update(cx, |view, cx| view.finish_auth(window, cx));
            });
            assert!(view.read_with(cx, |view, _| {
                view.shell
                    .auth
                    .as_ref()
                    .is_some_and(|state| state.phase == Phase::Saving)
            }));
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while view.read_with(cx, |view, _| view.shell.auth.is_some())
                && std::time::Instant::now() < deadline
            {
                cx.update(|window, cx| {
                    view.update(cx, |view, cx| {
                        view.poll(cx);
                        if view
                            .shell
                            .auth
                            .as_ref()
                            .is_some_and(|state| state.phase == Phase::Saved)
                        {
                            view.cancel_auth(window, cx);
                        }
                    });
                });
                cx.run_until_parked();
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            assert!(view.read_with(cx, |view, _| {
                view.shell.auth.is_none()
                    && view.router.tab() == Tab::Onboarding
                    && view.shell.onboarding.account.as_deref() == Some(account_id.as_str())
                    && view.shell.onboarding.account_decided
                    && view.selected_account.as_deref() == Some(account_id.as_str())
                    && view
                        .accounts
                        .iter()
                        .any(|saved| saved.identity_key() == account.identity_key())
            }));
            let saved = crate::global_config::read(root.path())?;
            assert!(saved.is_some_and(|config| {
                config
                    .accounts
                    .iter()
                    .any(|saved| saved.identity_key() == account.identity_key())
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
                    view.router.navigate(Tab::Instances);
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
            assert!(
                view.read_with(cx, |view, _| view.router.tab() == Tab::Instances
                    && view.form.is_some()
                    && view.accounts.is_empty()
                    && view.shell.auth.is_none()
                    && !view.busy)
            );
            Ok(())
        })();
        assert!(result.is_ok(), "{result:?}");
    }

    #[gpui::test]
    fn account_setup_keeps_actions_reachable_at_supported_window_sizes(cx: &mut TestAppContext) {
        let result = (|| -> anyhow::Result<()> {
            let account = test_account()?;
            for (width, height) in [(360., 720.), (1152., 720.), (960., 600.)] {
                for phase in [
                    Phase::Connect,
                    Phase::Preparing,
                    Phase::Waiting,
                    Phase::Verifying,
                    Phase::Characters,
                    Phase::Failed,
                    Phase::Complete,
                    Phase::Saving,
                    Phase::Saved,
                ] {
                    let (_view, cx) = crate::test_support::window(cx, |window, cx| {
                        let mut view = crate::tests::test_view(window, cx);
                        view.preferences.reduce_motion = true;
                        view.shell.auth = Some(AuthWizard {
                            phase,
                            prepared_account: matches!(phase, Phase::Complete | Phase::Saved)
                                .then(|| account.clone()),
                            ..AuthWizard::default()
                        });
                        view.router.navigate(Tab::Accounts);
                        view.router.begin_auth();
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
                        if selector == "auth-cancel" && phase.committed() {
                            assert!(action.is_none(), "committed save must not offer cancel");
                            continue;
                        }
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
                |state| state.phase == Phase::Connect
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
    fn a_save_in_flight_refuses_every_exit_and_keeps_the_parent_game_draft(
        cx: &mut TestAppContext,
    ) {
        let result = (|| -> anyhow::Result<()> {
            let account = test_account()?;
            let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
            let (sender, receiver) = tokio::sync::mpsc::channel(3);
            cx.update(|window, cx| {
                view.update(cx, |view, cx| {
                    view.begin_game_wizard(cx);
                    view.open_auth_wizard(window, cx);
                    if let Some(state) = &mut view.shell.auth {
                        state.prepared_account = Some(account.clone());
                        state.transition(Phase::Saving);
                    }
                    view.busy = true;
                    view.operation_kind = crate::OperationKind::Account;
                    view.messages = Some(receiver);
                    // Escape and Close both route here. Finish already authorized
                    // a durable write, so no exit can revoke it.
                    view.cancel_auth(window, cx);
                    let tab = view.router.tab();
                    view.handle(crate::UiAction::Tab(Tab::Home), window, cx);
                    assert_eq!(view.router.tab(), tab);
                    view.handle(crate::UiAction::CloseWindow, window, cx);
                });
            });
            assert!(view.read_with(cx, |view, _| {
                view.busy && view.form.is_some() && view.shell.auth.is_some()
            }));
            assert!(!cx.simulate_close(), "native close must wait for the save");
            cx.update(|_, cx| crate::initialize_application_quit(cx));
            crate::test_support::press(cx, "cmd-q");
            assert!(view.read_with(cx, |view, _| {
                view.shell.toast.as_ref().is_some_and(|toast| {
                    toast.message.key() == Some("accounts.auth.saving.wait_exit")
                }) && view.shell.auth.is_some()
            }));
            // The committed page offers no exit that would read as cancelling.
            assert!(crate::test_support::control_bounds(cx, "auth-cancel").is_none());
            assert!(crate::test_support::control_bounds(cx, "auth-back").is_none());
            assert!(
                sender
                    .try_send(crate::engine::Message::Accounts(Ok(vec![account.clone()])))
                    .is_ok()
            );
            assert!(
                sender
                    .try_send(crate::engine::Message::SignedIn(account.clone()))
                    .is_ok()
            );
            assert!(
                sender
                    .try_send(crate::engine::Message::Finished(Ok("Account saved".into())))
                    .is_ok()
            );
            view.update(cx, crate::Block::poll);
            cx.run_until_parked();
            assert!(view.read_with(cx, |view, _| {
                !view.busy
                    && view.form.is_some()
                    && view.accounts.len() == 1
                    && view.shell.auth.is_none()
            }));
            assert!(cx.simulate_close(), "native close resumes after the save");
            assert!(cx.cx.update(crate::request_application_quit));
            Ok(())
        })();
        assert!(result.is_ok(), "{result:?}");
    }

    #[gpui::test]
    fn a_failed_save_offers_signing_in_again_instead_of_another_finish(cx: &mut TestAppContext) {
        let result = (|| -> anyhow::Result<()> {
            let root = tempfile::tempdir()?;
            let account = test_account()?;
            let (engine, messages) = crate::engine::Engine::with_root(root.path().to_owned())?;
            let (view, cx) = crate::test_support::window(cx, move |window, cx| {
                let mut view = crate::tests::test_view(window, cx);
                view.preferences.reduce_motion = true;
                view.engine = Some(engine);
                view.messages = Some(messages);
                view
            });
            cx.update(|window, cx| {
                view.update(cx, |view, cx| {
                    view.router.navigate(Tab::Accounts);
                    view.open_auth_wizard(window, cx);
                    if let Some(state) = &mut view.shell.auth {
                        state.prepared_account = Some(account.clone());
                        state.transition(Phase::Complete);
                    }
                    view.finish_auth(window, cx);
                });
            });
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while view.read_with(cx, |view, _| view.busy) && std::time::Instant::now() < deadline {
                cx.update(|_, cx| {
                    view.update(cx, |view, cx| view.poll(cx));
                });
                cx.run_until_parked();
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            // This engine holds no prepared sign-in for the identity, so the save
            // fails. The verified credential is spent, so the page must not offer
            // the same save again, only a fresh sign-in.
            assert!(view.read_with(cx, |view, _| {
                view.shell.auth.as_ref().is_some_and(|state| {
                    state.phase == Phase::Failed
                        && state.save_failed
                        && state.prepared_account.is_none()
                }) && view.accounts.is_empty()
            }));
            assert_eq!(
                control_node(cx, "auth-primary").label(),
                Some("Sign in again")
            );
            // The recovery is a fresh sign-in, and this process can still offer
            // it: the engine is available and the credential store is not the
            // settled unsupported case.
            assert_ne!(control_node(cx, "auth-primary").disabled(), Some(true));
            Ok(())
        })();
        assert!(result.is_ok(), "{result:?}");
    }

    /// Home is the destination a normal launch starts from: a ready game, no
    /// account yet, so its identity block offers the Add account action.
    fn home_without_an_account(
        view: &gpui::Entity<crate::Block>,
        cx: &mut gpui::VisualTestContext,
    ) {
        view.update(cx, |view, cx| {
            view.preferences.reduce_motion = true;
            view.snapshot = crate::test_support::ready_snapshot();
            view.selected_instance = Some("instance-a".into());
            view.selected_profile = Some("profile-a".into());
            cx.notify();
        });
        view.update_in(cx, |view, window, cx| {
            view.handle(crate::UiAction::Tab(Tab::Home), window, cx);
        });
        cx.run_until_parked();
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(500));
        cx.run_until_parked();
    }

    /// Simulate the engine boundary once the confirmation has authorized the
    /// save: the page is saving, and the engine reports the stored identity, the
    /// account list and the finished operation.
    fn complete_simulated_save(
        view: &gpui::Entity<crate::Block>,
        cx: &mut gpui::VisualTestContext,
        account: &crate::Account,
    ) -> anyhow::Result<()> {
        assert!(view.read_with(cx, |view, _| {
            view.busy
                && view.operation_kind == crate::OperationKind::Account
                && view
                    .shell
                    .auth
                    .as_ref()
                    .is_some_and(|state| state.phase == Phase::Saving)
        }));
        let (sender, receiver) = tokio::sync::mpsc::channel(4);
        sender.try_send(crate::engine::Message::Accounts(Ok(vec![account.clone()])))?;
        sender.try_send(crate::engine::Message::SignedIn(account.clone()))?;
        sender.try_send(crate::engine::Message::Finished(Ok("Account saved".into())))?;
        view.update(cx, |view, cx| {
            view.messages = Some(receiver);
            view.poll(cx);
        });
        cx.run_until_parked();
        Ok(())
    }

    fn assert_no_launch(view: &crate::Block) {
        assert!(!view.busy, "a saved identity must not start an operation");
        assert!(
            !view.pending_play,
            "a saved identity must not queue a launch"
        );
        assert!(view.active_run.is_none(), "a saved identity must not run");
        assert_ne!(view.operation_kind, crate::OperationKind::Launch);
    }

    #[gpui::test]
    fn saving_an_offline_identity_from_home_returns_to_home_without_launching(
        cx: &mut TestAppContext,
    ) {
        let result = (|| -> anyhow::Result<()> {
            let root = tempfile::tempdir()?;
            let (engine, messages) = crate::engine::Engine::with_root(root.path().to_owned())?;
            let (view, cx) = crate::test_support::window(cx, move |window, cx| {
                let mut view = crate::tests::test_view(window, cx);
                view.engine = Some(engine);
                view.messages = Some(messages);
                view
            });
            home_without_an_account(&view, cx);
            // The real Home entry: its identity block adds an account.
            click(cx, "home-sign-in");
            assert!(view.read_with(cx, |view, _| view.shell.auth.is_some()
                && view.router.auth_return() == Some(Tab::Home)));
            let body = crate::test_support::control_bounds(cx, "auth-body")
                .ok_or_else(|| anyhow::anyhow!("missing account page"))?;
            assert!(scroll_into_view(cx, body, "auth-other-options"));
            click(cx, "auth-other-options");
            assert!(scroll_into_view(cx, body, "auth-offline"));
            click(cx, "auth-offline");
            cx.update(|_, cx| {
                view.update(cx, |view, cx| {
                    if let Some(field) = view
                        .shell
                        .auth
                        .as_ref()
                        .and_then(|state| state.username.clone())
                    {
                        field.update(cx, |field, cx| field.set_value("WizardNative", cx));
                    }
                    cx.notify();
                });
            });
            // The local name page's own action adds and uses the identity.
            click(cx, "auth-primary");
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while view.read_with(cx, |view, _| view.shell.auth.is_some())
                && std::time::Instant::now() < deadline
            {
                cx.update(|_, cx| {
                    view.update(cx, |view, cx| view.poll(cx));
                });
                cx.run_until_parked();
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            let account = crate::engine::offline_account("WizardNative")?;
            let id = crate::account_id(&account);
            assert!(view.read_with(cx, |view, _| {
                view.shell.auth.is_none()
                    && view.router.tab() == Tab::Home
                    && view.page_spec().page == crate::navigation::Page::Home
                    && !view.page_spec().task
                    && view.selected_account.as_deref() == Some(id.as_str())
                    && view
                        .accounts
                        .iter()
                        .any(|saved| saved.identity_key() == account.identity_key())
            }));
            view.read_with(cx, |view, _| assert_no_launch(view));
            cx.update(|window, cx| {
                view.read_with(cx, |view, _| assert!(view.focus.is_focused(window)));
            });
            assert_eq!(
                crate::global_config::read(root.path())?.map(|config| config.accounts.len()),
                Some(1)
            );
            Ok(())
        })();
        assert!(result.is_ok(), "{result:?}");
    }

    #[gpui::test]
    fn a_prepared_online_identity_saved_from_home_returns_to_home_without_launching(
        cx: &mut TestAppContext,
    ) {
        let result = (|| -> anyhow::Result<()> {
            let root = tempfile::tempdir()?;
            let (engine, _receiver) = crate::engine::Engine::with_root(root.path().to_owned())?;
            let account = test_account()?;
            let id = crate::account_id(&account);
            let (view, cx) = crate::test_support::window(cx, |window, cx| {
                let mut view = crate::tests::test_view(window, cx);
                view.engine = Some(engine);
                view
            });
            home_without_an_account(&view, cx);
            click(cx, "home-sign-in");
            cx.update(|_, cx| {
                view.update(cx, |view, cx| {
                    // The engine prepared this identity; confirming it saves it.
                    if let Some(state) = &mut view.shell.auth {
                        state.prepared_account = Some(account.clone());
                        state.transition(Phase::Complete);
                    }
                    cx.notify();
                });
            });
            click(cx, "auth-primary");
            complete_simulated_save(&view, cx, &account)?;
            assert!(view.read_with(cx, |view, _| {
                view.shell.auth.is_none()
                    && view.router.tab() == Tab::Home
                    && view.page_spec().page == crate::navigation::Page::Home
                    && view.selected_account.as_deref() == Some(id.as_str())
                    && view
                        .accounts
                        .iter()
                        .any(|saved| saved.identity_key() == account.identity_key())
            }));
            view.read_with(cx, |view, _| assert_no_launch(view));
            cx.update(|window, cx| {
                view.read_with(cx, |view, _| assert!(view.focus.is_focused(window)));
            });
            Ok(())
        })();
        assert!(result.is_ok(), "{result:?}");
    }

    #[gpui::test]
    fn saving_an_identity_from_accounts_keeps_its_account_context(cx: &mut TestAppContext) {
        let result = (|| -> anyhow::Result<()> {
            let root = tempfile::tempdir()?;
            let (engine, _receiver) = crate::engine::Engine::with_root(root.path().to_owned())?;
            let account = test_account()?;
            let id = crate::account_id(&account);
            let (view, cx) = crate::test_support::window(cx, |window, cx| {
                let mut view = crate::tests::test_view(window, cx);
                view.preferences.reduce_motion = true;
                view.engine = Some(engine);
                view.router.navigate(Tab::Accounts);
                view
            });
            cx.run_until_parked();
            // The Accounts page's own Add account action is the entry.
            click(cx, "sign-in");
            assert!(view.read_with(cx, |view, _| view.shell.auth.is_some()
                && view.router.auth_return() == Some(Tab::Accounts)));
            view.update(cx, |view, cx| {
                if let Some(state) = &mut view.shell.auth {
                    state.prepared_account = Some(account.clone());
                    state.transition(Phase::Complete);
                }
                cx.notify();
            });
            click(cx, "auth-primary");
            complete_simulated_save(&view, cx, &account)?;
            assert!(view.read_with(cx, |view, _| {
                view.shell.auth.is_none()
                    && view.router.tab() == Tab::Accounts
                    && view.page_spec().page == crate::navigation::Page::Browser
                    && view.selected_account.as_deref() == Some(id.as_str())
                    && view.shell.accounts.inspected.as_deref() == Some(id.as_str())
            }));
            assert_eq!(
                view.read_with(cx, |view, _| view.page_spec().detail),
                crate::navigation::DetailKey::Browser {
                    tab: Tab::Accounts,
                    screen: crate::navigation::Screen::List,
                    inspected: None,
                    editor_page: None,
                    profile_form: false,
                    instance_editor: None,
                }
            );
            view.read_with(cx, |view, _| assert_no_launch(view));
            cx.update(|window, cx| {
                view.read_with(cx, |view, _| assert!(view.focus.is_focused(window)));
            });
            Ok(())
        })();
        assert!(result.is_ok(), "{result:?}");
    }

    /// Choosing an account type at a compact size starts its form at the top of
    /// the page with its first field focused, and switching back keeps the draft
    /// that was typed there.
    #[gpui::test]
    fn choosing_an_account_type_at_a_compact_size_reveals_its_form_and_focuses_it(
        cx: &mut TestAppContext,
    ) {
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        home_without_an_account(&view, cx);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.handle(crate::UiAction::SignIn, window, cx);
            });
        });
        // A window the manager keeps compact after the page asked for its width.
        cx.simulate_resize(size(px(360.), px(540.)));
        cx.run_until_parked();
        let body = crate::test_support::control_bounds(cx, "auth-body");
        assert!(body.is_some(), "the compact account page must scroll");
        let Some(body) = body else {
            return;
        };
        // The account types sit at the end of the page, behind the disclosure.
        assert!(
            scroll_into_view(cx, body, "auth-other-options"),
            "the disclosure must be reachable"
        );
        click(cx, "auth-other-options");
        assert!(
            scroll_into_view(cx, body, "auth-offline"),
            "the local account type must be reachable"
        );
        assert!(view.read_with(cx, |view, _| {
            view.shell
                .auth
                .as_ref()
                .is_some_and(|state| state.body_scroll.offset().y < px(0.))
        }));
        click(cx, "auth-offline");
        // The chosen type's own form is what the page now shows, from its start.
        let card = crate::test_support::control_bounds(cx, "auth-offline");
        assert!(
            card.is_some_and(|card| card.top() >= body.top() && card.top() < body.bottom()),
            "the local form must start inside the viewport: {card:?} in {body:?}"
        );
        let name = crate::test_support::control_bounds(cx, "auth-offline-name");
        assert!(
            name.is_some_and(|name| name.top() >= body.top() && name.bottom() <= body.bottom()),
            "the local name field must be visible: {name:?} in {body:?}"
        );
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                let state = view.shell.auth.as_ref();
                assert!(
                    state.is_some_and(|state| {
                        state.provider == block_auth::AccountProvider::Offline
                            && state.body_scroll.offset().y == px(0.)
                    }),
                    "the page restarts at the form, not at the alternatives"
                );
                assert!(
                    state
                        .and_then(|state| state.username.as_ref())
                        .is_some_and(|field| field.read(cx).focus_handle(cx).is_focused(window)),
                    "the local name field takes the caret"
                );
                if let Some(field) = state.and_then(|state| state.username.clone()) {
                    field.update(cx, |field, cx| field.set_value("Compact_Draft", cx));
                }
                cx.notify();
            });
        });
        // Switching back and forth keeps the draft and starts each form afresh.
        assert!(
            scroll_into_view(cx, body, "auth-microsoft"),
            "the Microsoft row must be reachable"
        );
        click(cx, "auth-microsoft");
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                let state = view.shell.auth.as_ref();
                assert!(state.is_some_and(|state| {
                    state.provider == block_auth::AccountProvider::Microsoft
                        && state.body_scroll.offset().y == px(0.)
                }));
                assert!(
                    state.is_some_and(|state| state
                        .username
                        .as_ref()
                        .is_some_and(|field| field.read(cx).value() == "Compact_Draft")),
                    "a non-sensitive draft survives the switch"
                );
                assert!(
                    state
                        .and_then(|state| state.focus.as_ref())
                        .is_some_and(|focus| focus.is_focused(window)),
                    "the account page takes focus back"
                );
            });
        });
        assert!(
            scroll_into_view(cx, body, "auth-offline"),
            "the local account type must stay reachable"
        );
        click(cx, "auth-offline");
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                let state = view.shell.auth.as_ref();
                assert!(state.is_some_and(|state| {
                    state.provider == block_auth::AccountProvider::Offline
                        && state.body_scroll.offset().y == px(0.)
                }));
                assert!(
                    state
                        .and_then(|state| state.username.as_ref())
                        .is_some_and(|field| field.read(cx).focus_handle(cx).is_focused(window)),
                    "the returning form focuses its name field"
                );
                assert!(
                    state.is_some_and(|state| state
                        .username
                        .as_ref()
                        .is_some_and(|field| field.read(cx).value() == "Compact_Draft")),
                    "the returning form keeps its draft"
                );
            });
        });
        assert!(
            scroll_into_view(cx, body, "auth-authlib"),
            "the community account type must be reachable"
        );
        click(cx, "auth-authlib");
        let picker = crate::test_support::control_bounds(cx, "auth-provider-picker");
        assert!(
            picker.is_some_and(|picker| {
                picker.top() >= body.top() && picker.bottom() <= body.bottom()
            }),
            "the provider picker must be visible: {picker:?} in {body:?}"
        );
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                assert!(
                    view.shell
                        .surfaces
                        .trigger_focus("auth-provider-picker", cx)
                        .is_focused(window),
                    "the first community-provider control takes focus"
                );
            });
        });
    }

    #[expect(
        clippy::expect_used,
        reason = "The fixture paints every control it inspects."
    )]
    fn control_node(
        cx: &mut gpui::VisualTestContext,
        id: &'static str,
    ) -> gpui_base::test_support::ElementSnapshot {
        cx.run_until_parked();
        cx.update(|window, _| {
            gpui_base::test_support::find(window, &[], &gpui::ElementId::Name(id.into()))
        })
        .expect("painted control")
    }

    #[gpui::test]
    fn account_setup_renders_the_view_language_and_retains_semantic_failures(
        cx: &mut TestAppContext,
    ) {
        let (view, cx) = crate::test_support::window(cx, |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.preferences.reduce_motion = true;
            view.preferences.language = Some(crate::Language::Japanese);
            view
        });
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.router.navigate(Tab::Accounts);
                view.open_auth_wizard(window, cx);
            });
        });
        // The default page starts the browser: its single primary action.
        assert_eq!(
            control_node(cx, "auth-primary").label(),
            Some("ブラウザーを開く")
        );
        assert_eq!(
            control_node(cx, "auth-device-code-action").label(),
            Some("デバイスコードで続行")
        );
        assert_eq!(
            control_node(cx, "auth-other-options").label(),
            Some("その他のサインインオプション")
        );
        cx.update(|_, cx| {
            view.update(cx, |view, cx| view.toggle_auth_other_options(cx));
        });
        assert_eq!(
            control_node(cx, "auth-offline").label(),
            Some("オフラインモード")
        );

        // A classified failure is retained as its semantic key, so the same
        // preparation renders the recovery text in a newly selected language.
        cx.update(|_, cx| {
            view.update(cx, |view, cx| {
                if let Some(state) = view.shell.auth.as_mut() {
                    state.transition(Phase::Preparing);
                }
                view.auth_finished(&Err("The sign-in timed out".into()));
                view.preferences.language = Some(crate::Language::SimplifiedChinese);
                cx.notify();
            });
        });
        assert!(
            view.read_with(cx, |view, _| view.shell.auth.as_ref().is_some_and(
                |state| state.phase == Phase::Failed
                    && state.error
                        == Some(super::AccountMessage::product(
                            "accounts.auth.error.expired"
                        ))
            ))
        );
        assert_eq!(control_node(cx, "auth-primary").label(), Some("重试"));
        assert_eq!(control_node(cx, "auth-cancel").label(), Some("关闭"));
        assert_eq!(
            super::AccountMessage::product("accounts.auth.error.expired")
                .render(crate::Language::SimplifiedChinese),
            "授权已过期。请重新开始登录以请求新的会话。"
        );
    }
}
