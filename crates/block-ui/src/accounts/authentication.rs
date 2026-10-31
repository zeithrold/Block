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
    AppContext as _, Context, FocusHandle, Focusable as _, InteractiveElement as _, IntoElement,
    ParentElement as _, Pixels, ScrollHandle, Size, StatefulInteractiveElement as _, Styled as _,
    Window, div, point, px, rgb,
};

use super::{
    Block, Language, Tab, UiAction, account_id,
    components::{self, AvatarState, BadgeVariant, ButtonVariant, ControlState, ProgressState},
    design::Icon,
    note, patterns,
};

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
    #[default]
    Choose,
    Information,
    Preparing,
    Waiting,
    Verifying,
    Characters,
    Failed,
    Complete,
    Saving,
    Saved,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AuthStep {
    Choose,
    SignIn,
    Verification,
    Complete,
}

impl AuthStep {
    const fn index(self) -> usize {
        match self {
            Self::Choose => 0,
            Self::SignIn => 1,
            Self::Verification => 2,
            Self::Complete => 3,
        }
    }
}

impl Phase {
    const fn step(self) -> AuthStep {
        match self {
            Self::Choose => AuthStep::Choose,
            Self::Information | Self::Preparing | Self::Waiting | Self::Failed => AuthStep::SignIn,
            Self::Verifying | Self::Characters => AuthStep::Verification,
            Self::Complete | Self::Saving | Self::Saved => AuthStep::Complete,
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
    return_size: Size<Pixels>,
    return_window: Option<gpui::AnyWindowHandle>,
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
            "auth-information"
            | "auth-authlib-information"
            | "auth-authlib-http-warning"
            | "auth-authlib-provider-add"
            | "auth-offline-information" => Phase::Information,
            "auth-authlib-characters" => Phase::Characters,
            "auth-authlib-complete" | "auth-offline-complete" | "auth-complete" => Phase::Complete,
            "auth-offline-error" | "auth-error" => Phase::Failed,
            "auth-device-preparing" => Phase::Preparing,
            "auth-device-waiting" | "auth-waiting" => Phase::Waiting,
            "auth-verifying" => Phase::Verifying,
            "auth-saving" => Phase::Saving,
            _ => Phase::Choose,
        };
        if matches!(
            name,
            "auth-complete" | "auth-saving" | "auth-offline-complete"
        ) {
            self.prepared_account = account.clone();
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
        let rail_scroll = ScrollHandle::new();
        rail_scroll.set_offset(self.rail_scroll.offset());
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
            return_size: self.return_size,
            return_window: self.return_window,
            focus: Some(cx.focus_handle()),
            body_scroll,
            rail_scroll,
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
                    || !matches!(s.phase, Phase::Information | Phase::Failed)
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
            state.transition(Phase::Information);
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

    pub(super) fn render_auth_provider_add(&self, cx: &Context<'_, Self>) -> gpui::Div {
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
            .child(patterns::page_heading(
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
        form.children(
            draft
                .error
                .as_ref()
                .map(|error| note(error.render(language))),
        )
        .child(
            div()
                .flex()
                .justify_end()
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
                                        && draft.accepted_http.as_deref() != Some(a.url.as_str())
                                }),
                        ..Default::default()
                    },
                    cx,
                )),
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
            && matches!(state.phase, Phase::Information | Phase::Failed)
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
            state.transition(Phase::Information);
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
            && matches!(state.phase, Phase::Information | Phase::Failed)
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
                || !matches!(state.phase, Phase::Information | Phase::Failed)
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

    pub(super) fn toggle_auth_other_options(&mut self, cx: &mut Context<'_, Self>) {
        if let Some(state) = &mut self.shell.auth
            && state.phase == Phase::Choose
            && !self.busy
        {
            state.other_sign_in_options = !state.other_sign_in_options;
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
            if self.abandon_operation(revision) {
                self.status = msg!("accounts.auth.status.cancelled");
            }
        }
        if let Some(engine) = &self.engine {
            engine.discard_prepared_sign_in();
        }
    }

    pub(super) fn auth_back(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
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
        if self
            .shell
            .auth
            .as_ref()
            .is_some_and(|state| state.provider_draft.is_some())
        {
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
                        || !matches!(state.phase, Phase::Information | Phase::Failed)
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
            return_size: window.viewport_size(),
            return_window: Some(window.window_handle()),
            focus: Some(focus.clone()),
            provider: AccountProvider::Microsoft,
            ..AuthWizard::default()
        };
        self.router.begin_auth();
        self.shell.auth = Some(wizard);
        // The Add account wizard is the Accounts page's own page, as in the
        // prototype's /accounts/new; never a separate window or overlay.
        self.shell.instance_editor = None;
        self.shell.providers.visible = false;
        self.request_frame_resize(window, cx);
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
            if let Some(state) = &mut self.shell.auth {
                match result {
                    Ok(account) => {
                        state.prepared_account = Some(account);
                        state.error = None;
                        state.transition(Phase::Complete);
                    }
                    Err(error) => {
                        state.error = Some(AccountMessage::diagnostic(error.to_string()));
                        state.transition(Phase::Failed);
                    }
                }
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
        if let Some(state) = &mut self.shell.auth
            && (state.phase.pending() || state.phase == Phase::Saving)
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
                Err(error) => state.failed(AccountMessage::diagnostic(error.clone())),
                Ok(_) => state.failed(AccountMessage::product(
                    "accounts.auth.error.ended_without_identity",
                )),
            }
        }
    }

    pub(super) fn cancel_auth(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.router.auth_return() == Some(Tab::Onboarding)
            && self
                .shell
                .auth
                .as_ref()
                .is_some_and(|state| state.phase == Phase::Saving)
        {
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
            .is_some_and(|state| matches!(state.phase, Phase::Saving | Phase::Saved));
        // Finish authorizes a durable save. Closing the page does not cancel
        // that transaction, so only an uncommitted attempt returns to its page.
        if !committing {
            self.abandon_auth_attempt();
        }
        if committing {
            self.close_auth_context(window, cx);
        } else {
            self.abandon_auth_context(window, cx);
        }
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
                state.failed(AccountMessage::product(
                    "accounts.auth.error.save_unavailable",
                ));
            }
            cx.notify();
            return;
        }
        self.begin(
            msg!("accounts.auth.status.saving"),
            super::ui_logging::Operation::SaveAccount,
        );
        self.operation_kind = super::OperationKind::Account;
        if let Some(state) = &mut self.shell.auth {
            state.transition(Phase::Saving);
            state.attempt = Some((self.operation_revision, self.cancel.clone()));
        }
        if let Some(engine) = &self.engine {
            if account.provider == AccountProvider::Offline {
                engine.save_offline_account(account);
            } else if account.provider == AccountProvider::Authlib {
                engine.save_authlib_sign_in(account.identity_key());
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
        } else {
            self.shell.accounts.inspected = Some(id.clone());
        }
        self.selected_account = Some(id.clone());
        self.shell.accounts.inspected = Some(id);
        self.status = msg!("accounts.auth.status.saved", name = account.player_name);
        self.close_auth_context(window, cx);
    }

    /// Finish path: first-run preparation returns to its setup page; a normal
    /// Add account stays on Accounts, which now shows the saved account.
    fn close_auth_context(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(state) = self.shell.auth.take() else {
            return;
        };
        if self.router.end_auth(false) == Some(Tab::Onboarding) {
            window.resize(state.return_size);
            self.focus.focus(window, cx);
        }
        cx.notify();
    }

    /// Cancel path: abandon the page and return to the page and size that opened
    /// it, preserving any draft there.
    fn abandon_auth_context(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(_state) = self.shell.auth.take() else {
            return;
        };
        let _ = self.router.end_auth(true);
        self.request_frame_resize(window, cx);
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
                rail: self.auth_rail(state).into_any_element(),
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
            .p_8()
            .child(self.auth_body(state, cx))
            .into_any_element()
    }

    fn auth_rail(
        &self,
        state: &AuthWizard,
    ) -> super::scroll_area::ScrollArea<gpui::Stateful<gpui::Div>> {
        let language = self.language();
        let mut rail = div()
            .id("auth-rail")
            .w(px(super::design::SIDEBAR_WIDTH))
            .flex_shrink_0()
            .h_full()
            .min_h_0()
            .scroll_area(
                &state.rail_scroll,
                "accounts.auth.rail",
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
            .child(
                div()
                    .text_sm()
                    .child(tr!(language, "accounts.auth.setup.title")),
            );
        let entries = [
            (
                tr!(language, "accounts.auth.step.type.title"),
                tr!(language, "accounts.auth.step.type.description"),
            ),
            (
                tr!(language, "accounts.auth.step.sign_in.title"),
                match state.provider {
                    AccountProvider::Offline => {
                        tr!(language, "accounts.auth.step.sign_in.offline")
                    }
                    AccountProvider::Authlib => {
                        tr!(language, "accounts.auth.step.sign_in.authlib")
                    }
                    AccountProvider::Microsoft => {
                        tr!(language, "accounts.auth.step.sign_in.microsoft")
                    }
                },
            ),
            (
                tr!(language, "accounts.auth.step.verification.title"),
                match state.provider {
                    AccountProvider::Offline => {
                        tr!(language, "accounts.auth.step.verification.offline")
                    }
                    AccountProvider::Authlib => {
                        tr!(language, "accounts.auth.step.verification.authlib")
                    }
                    AccountProvider::Microsoft => {
                        tr!(language, "accounts.auth.step.verification.microsoft")
                    }
                },
            ),
            (
                tr!(language, "accounts.auth.step.complete.title"),
                tr!(language, "accounts.auth.step.complete.description"),
            ),
        ];
        let steps = entries
            .iter()
            .enumerate()
            .map(|(index, (title, description))| components::StepperStep {
                title,
                description,
                complete: match index {
                    0 => state.phase != Phase::Choose,
                    1 => matches!(
                        state.phase,
                        Phase::Verifying | Phase::Characters | Phase::Complete
                    ),
                    2 => matches!(state.phase, Phase::Complete | Phase::Saving | Phase::Saved),
                    3 => state.phase == Phase::Saved,
                    _ => false,
                },
            })
            .collect::<Vec<_>>();
        rail = rail.child(components::stepper(
            "auth-steps",
            "Account setup steps",
            &steps,
            state.phase.step().index(),
            self.palette,
        ));
        rail.child(div().flex_1()).child(note(tr!(
            language,
            "accounts.auth.step.progress",
            step = state.phase.step().index() + 1
        )))
    }

    /// The localized heading for the sign-in step the wizard is on.
    fn auth_heading(&self, state: &AuthWizard) -> (String, String) {
        let language = self.language();
        let (title, description) = auth_heading_keys(state);
        (tr!(language, title), tr!(language, description))
    }

    fn auth_body(&self, state: &AuthWizard, cx: &Context<'_, Self>) -> gpui::Div {
        let language = self.language();
        let (title, description) = self.auth_heading(state);
        let mut body = div().min_w_0().flex().flex_col().gap_6().child(
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(
                    div()
                        .text_2xl()
                        .font_weight(gpui::FontWeight::BOLD)
                        .child(title),
                )
                .child(note(description)),
        );
        body = match state.phase {
            Phase::Choose => body.child(self.auth_choices(state, cx)),
            Phase::Information => body.child(self.auth_information(state, cx)),
            Phase::Preparing | Phase::Waiting if state.provider == AccountProvider::Authlib => {
                body.child(self.auth_waiting(state, cx))
            }
            Phase::Preparing | Phase::Waiting => body
                .child(self.auth_information(state, cx))
                .child(self.auth_waiting(state, cx)),
            Phase::Verifying => body.child(self.auth_waiting(state, cx)),
            Phase::Characters => {
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
                body.child(choices).children(
                    state
                        .error
                        .as_ref()
                        .map(|error| note(error.render(language))),
                )
            }
            Phase::Failed if state.provider == AccountProvider::Offline => body
                .child(self.auth_information(state, cx))
                .children(state.error.as_ref().map(|error| {
                    div()
                        .text_sm()
                        .text_color(rgb(self.palette.destructive))
                        .child(error.render(language))
                })),
            Phase::Failed => body.child(self.auth_information(state, cx)).child(
                components::card(self.palette)
                    .border_color(rgb(self.palette.destructive))
                    .child(Icon::Error.render(24., self.palette.destructive))
                    .child(note(state.error.as_ref().map_or_else(
                        || tr!(language, "accounts.auth.error.fallback"),
                        |error| error.render(language),
                    ))),
            ),
            Phase::Complete | Phase::Saved => body.child(self.auth_complete(state)),
            Phase::Saving => body.child(components::progress(
                ProgressState::Indeterminate,
                self.palette,
                tr!(language, "common.in_progress"),
            )),
        };
        body.children(
            (self.busy
                && !state.phase.pending()
                && !matches!(state.phase, Phase::Saving | Phase::Saved))
            .then(|| note(tr!(language, "accounts.auth.busy_notice"))),
        )
    }

    /// Microsoft is the primary account path on every entry. The provider
    /// alternatives stay behind one disclosure, and the option the user already
    /// selected never disappears when the disclosure is collapsed.
    fn auth_choices(&self, state: &AuthWizard, cx: &Context<'_, Self>) -> gpui::Div {
        let language = self.language();
        let mut choices = div()
            .flex()
            .flex_col()
            .gap_4()
            .child(self.auth_provider_choice(
                "auth-microsoft",
                tr!(language, "accounts.auth.provider.microsoft.label"),
                tr!(language, "accounts.auth.provider.microsoft.description"),
                AccountProvider::Microsoft,
                state.provider,
                cx,
            ));
        // Each alternative follows the disclosure, and the one the user has
        // already selected stays listed so a collapsed section never hides the
        // current choice.
        if state.other_sign_in_options || state.provider == AccountProvider::Authlib {
            choices = choices.child(self.auth_provider_choice(
                "auth-authlib",
                tr!(language, "accounts.auth.provider.authlib.label"),
                tr!(language, "accounts.auth.provider.authlib.description"),
                AccountProvider::Authlib,
                state.provider,
                cx,
            ));
        }
        if state.other_sign_in_options || state.provider == AccountProvider::Offline {
            choices = choices.child(self.auth_provider_choice(
                "auth-offline",
                tr!(language, "accounts.auth.provider.offline.label"),
                tr!(language, "accounts.auth.provider.offline.description"),
                AccountProvider::Offline,
                state.provider,
                cx,
            ));
        }
        choices
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
                // The label states the action; assistive technology also reads
                // the disclosure's current state from this flag.
                .aria_expanded(state.other_sign_in_options),
            )
            .child(note(tr!(language, "accounts.auth.choices.note")))
    }

    fn auth_provider_choice(
        &self,
        id: &'static str,
        label: impl Into<gpui::SharedString>,
        description: impl Into<gpui::SharedString>,
        provider: AccountProvider,
        selected: AccountProvider,
        cx: &Context<'_, Self>,
    ) -> gpui::Div {
        components::card(self.palette)
            .child(components::radio(
                id,
                label,
                provider == selected,
                self.palette,
                ControlState::default(),
                cx.listener(move |view, (), window, cx| {
                    view.handle(UiAction::AuthProvider(provider), window, cx);
                }),
            ))
            .child(note(description))
    }

    fn auth_information(&self, state: &AuthWizard, cx: &Context<'_, Self>) -> gpui::Div {
        let language = self.language();
        if state.provider == AccountProvider::Authlib {
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
                .gap_3()
                .child(
                    div()
                        .flex()
                        .items_end()
                        .gap_2()
                        .child(div().flex_1().min_w_0().child(self.product_select_field(
                            "auth-provider-picker",
                            tr!(language, "select.auth_provider"),
                            &picker_value,
                            super::select::ProductChoice::AuthProvider,
                            cx,
                        )))
                        .child(self.auth_button(
                            "auth-provider-add",
                            "+",
                            UiAction::AuthProviderAdd,
                            ButtonVariant::Outline,
                            ControlState {
                                disabled: self.busy,
                                ..ControlState::default()
                            },
                            cx,
                        )),
                )
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
            let mut fields = div().flex().gap_4();
            if let Some(login) = &state.login {
                fields = fields.child(div().flex_1().min_w_0().child(components::field_with(
                    tr!(language, "accounts.auth.login.label"),
                    login,
                    self.palette,
                    components::FieldOptions {
                        required: true,
                        ..components::FieldOptions::default()
                    },
                )));
            }
            let password_focus = state.password.clone();
            fields = fields.child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
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
                    })),
            );
            return card
                .child(fields)
                .child(note(tr!(language, "accounts.auth.password.note")));
        }
        if state.provider == AccountProvider::Offline {
            let mut card = components::card(self.palette)
                .child(note(tr!(language, "accounts.auth.offline.card_note")));
            if let Some(field) = &state.username {
                card = card.child(components::field_with(
                    tr!(language, "accounts.auth.offline.name.label"),
                    field,
                    self.palette,
                    components::FieldOptions {
                        required: true,
                        ..components::FieldOptions::default()
                    },
                ));
            }
            return card.child(note(tr!(language, "accounts.auth.offline.name.rule")));
        }
        components::card(self.palette)
            .child(note(tr!(language, "accounts.auth.microsoft.card_note")))
            .child(
                div()
                    .flex()
                    .gap_3()
                    .child(components::radio(
                        "auth-method-browser",
                        tr!(language, "accounts.auth.method.browser"),
                        state.method == Method::Browser,
                        self.palette,
                        ControlState::default(),
                        cx.listener(|view, (), window, cx| {
                            view.handle(UiAction::AuthMethod(Method::Browser), window, cx)
                        }),
                    ))
                    .child(components::radio(
                        "auth-method-device",
                        tr!(language, "accounts.auth.method.device"),
                        state.method == Method::Device,
                        self.palette,
                        ControlState::default(),
                        cx.listener(|view, (), window, cx| {
                            view.handle(UiAction::AuthMethod(Method::Device), window, cx)
                        }),
                    )),
            )
    }

    fn auth_waiting(&self, state: &AuthWizard, cx: &Context<'_, Self>) -> gpui::Div {
        let language = self.language();
        if state.provider == AccountProvider::Authlib {
            return self.authlib_waiting(state, language);
        }
        if state.method == Method::Device
            && matches!(state.phase, Phase::Preparing | Phase::Waiting)
        {
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
            .items_center()
            .justify_between()
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
            .items_center()
            .justify_between()
            .gap_3()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_xs()
                    .truncate()
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
            .child(note(tr!(language, "accounts.auth.waiting.moment")));
        if let Some(stage) = state.stage {
            card = card
                .child(note(tr!(language, auth_stage_key(stage))))
                .children(self.stage_boundary_rows(stage, language));
        }
        if state.browser_url.is_some() || self.visual_only && state.phase == Phase::Waiting {
            card = card
                .child(self.auth_button(
                    "auth-reopen",
                    tr!(language, "accounts.auth.waiting.reopen"),
                    UiAction::AuthReopen,
                    ButtonVariant::Outline,
                    ControlState::default(),
                    cx,
                ))
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
            .child(note(if state.phase == Phase::Saved {
                tr!(language, "accounts.auth.complete.note.saved")
            } else {
                tr!(language, "accounts.auth.complete.note.ready")
            }))
    }

    fn auth_footer(&self, state: &AuthWizard, cx: &Context<'_, Self>) -> gpui::Stateful<gpui::Div> {
        let language = self.language();
        let mut footer = patterns::management_footer(self.palette)
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
                        tr!(language, "accounts.auth.footer.cancel_sign_in")
                    } else {
                        tr!(language, "common.close")
                    },
                    UiAction::AuthCancel,
                    ButtonVariant::Ghost,
                    ControlState {
                        disabled: self.router.auth_return() == Some(Tab::Onboarding)
                            && state.phase == Phase::Saving,
                        ..ControlState::default()
                    },
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
                                tr!(language, "accounts.auth.footer.back"),
                                UiAction::AuthBack,
                                ButtonVariant::Ghost,
                                ControlState::default(),
                                cx,
                            )
                        }),
                ),
        );
        let (label, action, busy) = match state.phase {
            Phase::Choose => (
                tr!(language, "accounts.auth.footer.continue"),
                UiAction::AuthNext,
                false,
            ),
            Phase::Information => (
                match state.provider {
                    AccountProvider::Offline => {
                        tr!(language, "accounts.auth.footer.continue")
                    }
                    AccountProvider::Authlib => {
                        tr!(language, "accounts.auth.footer.sign_in")
                    }
                    AccountProvider::Microsoft if state.method == Method::Device => {
                        tr!(language, "accounts.auth.footer.request_code")
                    }
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
                tr!(language, "accounts.auth.footer.try_again"),
                UiAction::AuthRetry,
                false,
            ),
            Phase::Characters => (
                tr!(language, "accounts.auth.footer.continue"),
                UiAction::Authenticate,
                false,
            ),
            Phase::Complete => (
                tr!(language, "accounts.auth.footer.finish"),
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
            && matches!(state.phase, Phase::Information | Phase::Failed)
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
        footer.child(self.auth_button(
            "auth-primary",
            label,
            action,
            ButtonVariant::Primary,
            ControlState {
                busy,
                disabled: state.provider_draft.is_some()
                    || provider_blocked
                    || self.busy
                        && matches!(
                            state.phase,
                            Phase::Choose
                                | Phase::Information
                                | Phase::Failed
                                | Phase::Complete
                                | Phase::Characters
                        ),
                ..ControlState::default()
            },
            cx,
        ))
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
    }
}

/// The semantic keys of the heading and description for the step the wizard is on.
///
/// The keys stay unresolved so the view renders them in its current language,
/// including after a language change.
fn auth_heading_keys(state: &AuthWizard) -> (&'static str, &'static str) {
    match state.phase {
        Phase::Choose => (
            "accounts.auth.choose.title",
            "accounts.auth.choose.description",
        ),
        Phase::Information => match state.provider {
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
            if state.provider == AccountProvider::Offline {
                "accounts.auth.complete.description.offline"
            } else {
                "accounts.auth.complete.description.online"
            },
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
    } else if lower.contains("timeout") || lower.contains("timed out") || lower.contains("expired")
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
                view.auth_next(cx);
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
                    == Phase::Information
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
                assert!(view.shell.auth.as_ref().is_some_and(
                    |state| state.phase == Phase::Choose && state.provider_draft.is_none()
                ));
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
                view.auth_next(cx);
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
                assert!(
                    view.shell
                        .auth
                        .as_ref()
                        .is_some_and(|state| state.phase == Phase::Information)
                );
                view.auth_back(window, cx);
                assert!(
                    view.shell
                        .auth
                        .as_ref()
                        .is_some_and(|state| state.phase == Phase::Choose)
                );
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
                view.auth_next(cx);
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
                view.auth_back(window, cx);
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
    fn account_choices_disclose_other_sign_in_options_from_management(cx: &mut TestAppContext) {
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.preferences.reduce_motion = true;
                view.router.navigate(Tab::Accounts);
                view.open_auth_wizard(window, cx);
            });
        });
        // The managed window must fit all three cards so the pointer reaches them.
        cx.simulate_resize(size(px(1152.), px(720.)));
        cx.run_until_parked();
        // The ordinary Accounts entry starts exactly like onboarding: Microsoft
        // selected, provider alternatives collapsed behind the disclosure.
        assert!(
            view.read_with(cx, |view, _| view
                .shell
                .auth
                .as_ref()
                .is_some_and(|state| state.provider
                    == block_auth::AccountProvider::Microsoft
                    && !state.other_sign_in_options))
        );
        assert_eq!(
            view.read_with(cx, |view, _| view.router.auth_return()),
            Some(Tab::Accounts)
        );
        assert!(crate::test_support::control_bounds(cx, "auth-microsoft").is_some());
        assert!(crate::test_support::control_bounds(cx, "auth-other-options").is_some());
        assert!(crate::test_support::control_bounds(cx, "auth-authlib").is_none());
        assert!(crate::test_support::control_bounds(cx, "auth-offline").is_none());

        // Expanding reveals both alternatives without changing the choice.
        view.update(cx, |view, cx| view.toggle_auth_other_options(cx));
        cx.run_until_parked();
        assert!(crate::test_support::control_bounds(cx, "auth-authlib").is_some());
        assert!(crate::test_support::control_bounds(cx, "auth-offline").is_some());
        assert!(view.read_with(cx, |view, _| {
            view.shell
                .auth
                .as_ref()
                .is_some_and(|state| state.provider == block_auth::AccountProvider::Microsoft)
        }));

        // Select Offline, keep a draft, and walk through Information and back.
        click(cx, "auth-offline");
        assert!(view.read_with(cx, |view, _| {
            view.shell
                .auth
                .as_ref()
                .is_some_and(|state| state.provider == block_auth::AccountProvider::Offline)
        }));
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.auth_next(cx);
                if let Some(field) = view
                    .shell
                    .auth
                    .as_ref()
                    .and_then(|state| state.username.clone())
                {
                    field.update(cx, |field, cx| field.set_value("Local_Draft", cx));
                }
                view.auth_back(window, cx);
            });
        });
        cx.run_until_parked();
        assert!(
            view.read_with(cx, |view, _| view.shell.auth.as_ref().is_some_and(
                |state| state.phase == Phase::Choose
                    && state.provider == block_auth::AccountProvider::Offline
                    && state.other_sign_in_options
            ))
        );

        // Collapsing hides the alternative that is not selected and keeps the
        // selected provider and the typed draft visible and intact.
        view.update(cx, |view, cx| view.toggle_auth_other_options(cx));
        cx.run_until_parked();
        assert!(crate::test_support::control_bounds(cx, "auth-offline").is_some());
        assert!(crate::test_support::control_bounds(cx, "auth-authlib").is_none());
        assert!(
            view.read_with(cx, |view, cx| view.shell.auth.as_ref().is_some_and(
                |state| state.provider == block_auth::AccountProvider::Offline
                    && !state.other_sign_in_options
                    && state
                        .username
                        .as_ref()
                        .is_some_and(|field| field.read(cx).value() == "Local_Draft")
            ))
        );

        // Expanding again brings the remaining alternative back unchanged.
        view.update(cx, |view, cx| view.toggle_auth_other_options(cx));
        cx.run_until_parked();
        assert!(crate::test_support::control_bounds(cx, "auth-authlib").is_some());
        assert!(
            view.read_with(cx, |view, cx| view.shell.auth.as_ref().is_some_and(
                |state| state.provider == block_auth::AccountProvider::Offline
                    && state
                        .username
                        .as_ref()
                        .is_some_and(|field| field.read(cx).value() == "Local_Draft")
            ))
        );
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
            for (width, height) in [(1152., 720.), (960., 600.)] {
                for phase in [Phase::Choose, Phase::Waiting, Phase::Complete] {
                    let (_view, cx) = crate::test_support::window(cx, |window, cx| {
                        let mut view = crate::tests::test_view(window, cx);
                        view.preferences.reduce_motion = true;
                        view.shell.auth = Some(AuthWizard {
                            phase,
                            prepared_account: (phase == Phase::Complete).then(|| account.clone()),
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
        assert_eq!(control_node(cx, "auth-primary").label(), Some("続行"));
        assert_eq!(
            control_node(cx, "auth-microsoft").label(),
            Some("Microsoft アカウント")
        );
        assert_eq!(
            control_node(cx, "auth-other-options").label(),
            Some("その他のサインインオプション")
        );

        // A classified failure is retained as its semantic key, so the same
        // preparation renders the recovery text in a newly selected language.
        cx.update(|_, cx| {
            view.update(cx, |view, cx| {
                view.auth_next(cx);
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
