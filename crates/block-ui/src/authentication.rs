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
    Block, Tab, UiAction, account_id,
    components::{self, AvatarState, BadgeVariant, ButtonVariant, ControlState, ProgressState},
    design::Icon,
    note, patterns,
};

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

impl Phase {
    const fn step(self) -> usize {
        match self {
            Self::Choose => 0,
            Self::Information | Self::Preparing | Self::Waiting | Self::Failed => 1,
            Self::Verifying | Self::Characters => 2,
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

#[derive(Debug)]
pub(crate) struct AuthProviderDraft {
    id: uuid::Uuid,
    pub(crate) name: gpui::Entity<super::TextInput>,
    pub(crate) address: gpui::Entity<super::TextInput>,
    pub(crate) observed_address: String,
    pub(crate) accepted_http: Option<String>,
    pub(crate) error: Option<String>,
    attempt: Option<(u64, block_launcher::CancellationToken)>,
    saved: Option<uuid::Uuid>,
}

impl AuthWizard {
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
    error: Option<String>,
    return_tab: Tab,
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
            "auth-authlib-information"
            | "auth-authlib-http-warning"
            | "auth-authlib-provider-add" => Phase::Information,
            "auth-authlib-characters" => Phase::Characters,
            "auth-authlib-complete" => Phase::Complete,
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
                self.error = Some("Use 1–16 ASCII letters, numbers, or underscores.".into());
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
            return_tab: self.return_tab,
            return_size: self.return_size,
            return_window: self.return_window,
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
        } else if self.provider == AccountProvider::Authlib {
            error.to_owned()
        } else {
            sign_in_error(error)
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
            let address = super::input("", "Server API address or Authlib quick link", cx);
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
                name: super::input("", "Your community server", cx),
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
            if revision == self.operation_revision {
                self.operation_revision = self.operation_revision.wrapping_add(1);
                if let Some(engine) = &mut self.engine {
                    engine.begin_operation(self.operation_revision);
                }
                self.busy = self.active_run.is_some();
                self.operation_cancellable = false;
            }
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
                Err(error) => draft.error = Some(error.to_string()),
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
                    draft.error = Some(error.to_string());
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
                draft.error = Some(
                    if name.is_empty() {
                        "Enter a provider display name."
                    } else {
                        "Confirm the HTTP warning for this address."
                    }
                    .into(),
                );
            }
            cx.notify();
            return;
        }
        self.begin_cancellable("Discovering your authentication provider");
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
            self.auth_provider_finished(
                &Err("Provider setup is unavailable: engine unavailable".into()),
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
        result: &Result<String, String>,
        cx: &mut Context<'_, Self>,
    ) {
        if let Some(draft) = self
            .shell
            .auth
            .as_mut()
            .and_then(|s| s.provider_draft.as_mut())
            && draft
                .attempt
                .as_ref()
                .is_some_and(|(revision, _)| *revision == self.operation_revision)
            && let Err(error) = result
        {
            draft.error = Some(error.clone());
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
        let mut form = div()
            .flex()
            .flex_col()
            .gap_4()
            .child(div().text_xl().child("Add a provider"))
            .child(note(
                "Enter the server details or paste an Authlib quick link.",
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
                        .child(div().text_sm().child("Drop an Authlib quick link here"))
                        .child(note(
                            "You can also paste the link into the server address field.",
                        )),
                ),
                cx.entity().downgrade(),
            ))
            .child(components::field_with(
                "Display name",
                &draft.name,
                self.palette,
                components::FieldOptions {
                    required: true,
                    ..Default::default()
                },
            ))
            .child(components::field_with(
                "Server API address",
                &draft.address,
                self.palette,
                components::FieldOptions {
                    required: true,
                    ..Default::default()
                },
            ))
            .child(self.auth_button(
                "auth-provider-identify",
                "Recognize address",
                UiAction::AuthProviderIdentify,
                ButtonVariant::Outline,
                ControlState {
                    disabled: self.busy,
                    ..Default::default()
                },
                cx,
            ));
        if let Some(provider) = existing {
            form = form.child(note(format!(
                "Already saved as {}. Select it without creating a duplicate.",
                provider.name
            )));
        }
        if let Some(address) = &parsed
            && address.insecure
        {
            form=form.child(note("HTTP sends your password and tokens without encryption. Continue only if you trust this address and network."))
                .child(components::checkbox("auth-provider-http","I accept the HTTP risk for this address",draft.accepted_http.as_deref()==Some(address.url.as_str()),self.palette,ControlState{disabled:self.busy,..Default::default()},cx.listener(|view,(),window,cx|view.handle(UiAction::AuthProviderConsent,window,cx))));
        }
        form.children(draft.error.as_ref().map(|error| note(error.clone())))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(self.auth_button(
                        "auth-provider-cancel",
                        "Cancel",
                        UiAction::AuthProviderCancel,
                        ButtonVariant::Ghost,
                        ControlState::default(),
                        cx,
                    ))
                    .child(self.auth_button(
                        "auth-provider-save",
                        if existing.is_some() {
                            "Use saved provider"
                        } else {
                            "Add and select"
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
                || "Missing provider".into(),
                |provider| provider.name.clone(),
            )
        } else {
            account.provider.label().into()
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
                state.failed("Choose a saved authentication provider first.");
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
                state.failed(
                    "The provider address changed. Choose it again and re-enter your password.",
                );
            }
            cx.notify();
            return;
        }
        if provider.api_url.starts_with("http:")
            && state.accepted_http.as_deref() != Some(provider.api_url.as_str())
        {
            if let Some(state) = &mut self.shell.auth {
                state.failed(
                    "Confirm the HTTP warning for this provider before sending your password.",
                );
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
            self.begin_cancellable("Selecting your provider character");
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
                state.failed(
                    "Enter your provider account and password. The password is never saved.",
                );
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
        self.begin_cancellable("Authenticating with your provider");
        self.operation_kind = super::OperationKind::Account;
        if let Some(state) = &mut self.shell.auth {
            state.characters.clear();
            state.character = None;
            state.prepared_account = None;
            state.error = None;
            state.attempt = Some((self.operation_revision, self.cancel.clone()));
            if !self.busy {
                state.failed("External sign-in could not start: engine unavailable");
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
            && state.return_tab == Tab::Onboarding
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
        let password = cx.new(|cx| {
            gpui_base::input::InputState::new(window, cx)
                .masked(true)
                .placeholder("Your provider password")
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
            username: Some(super::input("", "Your player name", cx)),
            login: Some(super::input("", "Email or username", cx)),
            password: Some(password),
            external_provider: self.shell.providers.records.first().map(|p| p.id),
            external_api: self
                .shell
                .providers
                .records
                .first()
                .map(|p| p.api_url.clone()),
            return_tab: self.tab,
            return_size: window.viewport_size(),
            return_window: Some(window.window_handle()),
            focus: Some(focus.clone()),
            provider: if self.tab == Tab::Onboarding {
                AccountProvider::Microsoft
            } else {
                AccountProvider::default()
            },
            ..AuthWizard::default()
        };
        self.shell.auth = Some(wizard);
        // The Add account wizard is the Accounts page's own page, as in the
        // prototype's /accounts/new; never a separate window or overlay.
        self.shell.instance_editor = None;
        self.shell.providers.visible = false;
        self.shell.accounts.detail_visible = false;
        self.tab = Tab::Accounts;
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

    pub(super) fn auth_finished(&mut self, result: &Result<String, String>) {
        if let Some(state) = &mut self.shell.auth
            && (state.phase.pending() || state.phase == Phase::Saving)
        {
            match result {
                Err(error)
                    if state.provider == AccountProvider::Authlib
                        && !state.characters.is_empty()
                        && state.phase == Phase::Verifying =>
                {
                    state.error = Some(error.clone());
                    state.transition(Phase::Characters);
                }
                Err(error) => state.failed(error),
                Ok(_) => state.failed("Sign-in ended without an identity confirmation"),
            }
        }
    }

    pub(super) fn cancel_auth(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.shell.auth.as_ref().is_some_and(|state| {
            state.return_tab == Tab::Onboarding && state.phase == Phase::Saving
        }) {
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
        if state.return_tab == Tab::Onboarding
            && let Some(account) = &state.prepared_account
        {
            let id = account_id(account);
            self.shell.onboarding.account = Some(id.clone());
            self.shell.onboarding.account_decided = true;
            self.shell.onboarding.error = None;
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
        if self
            .shell
            .auth
            .as_ref()
            .is_some_and(|state| state.return_tab == Tab::Onboarding)
        {
            self.shell.onboarding.account = Some(id.clone());
            self.shell.onboarding.account_decided = true;
            self.shell.onboarding.error = None;
        } else {
            self.shell.accounts.inspected = Some(id.clone());
        }
        self.selected_account = Some(id.clone());
        self.shell.accounts.inspected = Some(id);
        self.shell.accounts.detail_visible = true;
        self.status = format!("{} is saved and ready to use", account.player_name);
        self.close_auth_context(window, cx);
    }

    /// Finish path: first-run preparation returns to its setup page; a normal
    /// Add account stays on Accounts, which now shows the saved account.
    fn close_auth_context(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(state) = self.shell.auth.take() else {
            return;
        };
        if state.return_tab == Tab::Onboarding {
            self.tab = Tab::Onboarding;
            window.resize(state.return_size);
            self.focus.focus(window, cx);
        }
        cx.notify();
    }

    /// Cancel path: abandon the page and return to the page and size that opened
    /// it, preserving any draft there.
    fn abandon_auth_context(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(state) = self.shell.auth.take() else {
            return;
        };
        self.tab = state.return_tab;
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
        let viewport = self.page_viewport(window);
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
                    .children((!narrow).then(|| {
                        self.shell.wizard_entrance.stagger(
                            self.auth_rail(state).into_any_element(),
                            std::time::Duration::ZERO,
                            window,
                            cx,
                        )
                    }))
                    .child(self.frame_detail(div().flex_1().min_h_0().min_w_0().child(
                        self.shell.wizard_entrance.stagger(
                            self.transition_content(
                                super::motion::Scope::Wizard,
                                self.render_auth_transition_body(window, cx),
                                window,
                                cx,
                            ),
                            std::time::Duration::from_millis(40),
                            window,
                            cx,
                        ),
                    ))),
            )
            .child(self.frame_detail(div().w_full().child(self.auth_footer(state, viewport, cx))));
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
            .p(px(if patterns::is_narrow(self.page_viewport(window)) {
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
            .w(px(super::design::SIDEBAR_WIDTH))
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
                } else if state.provider == AccountProvider::Authlib {
                    "Your provider account"
                } else {
                    "Continue with Microsoft"
                },
            ),
            (
                "Verification",
                if state.provider == AccountProvider::Offline {
                    "Validate the local identity"
                } else if state.provider == AccountProvider::Authlib {
                    "Select your provider character"
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
                "Sign in" => matches!(
                    state.phase,
                    Phase::Verifying | Phase::Characters | Phase::Complete
                ),
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
        self.frame_sidebar(
            rail.child(div().flex_1())
                .child(note(format!("Step {} of 4", state.phase.step() + 1))),
        )
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
                } else if state.provider == AccountProvider::Authlib {
                    "Sign in to your provider"
                } else {
                    "Sign in with Microsoft"
                },
                if state.provider == AccountProvider::Offline {
                    "Choose a name for local play without an authentication service."
                } else if state.provider == AccountProvider::Authlib {
                    "Choose your server and enter its account credentials."
                } else {
                    "Choose browser sign-in or enter a device code on another device."
                },
            ),
            Phase::Preparing => (
                "Preparing secure sign-in",
                if state.provider == AccountProvider::Authlib {
                    "Block is authenticating with your selected provider."
                } else {
                    "Block is requesting a real Microsoft authorization session."
                },
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
                if state.provider == AccountProvider::Authlib {
                    "Block is assigning the selected character through your provider."
                } else {
                    "Block is checking Java Edition ownership and resolving your player profile."
                },
            ),
            Phase::Characters => (
                "Choose your character",
                "These characters were returned by your authentication provider.",
            ),
            Phase::Failed if state.provider == AccountProvider::Offline => (
                "Check your player name",
                "Use 1–16 ASCII letters, numbers, or underscores. Your saved accounts are unchanged.",
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
                body.child(choices)
                    .children(state.error.as_ref().map(|error| note(error.clone())))
            }
            Phase::Failed if state.provider == AccountProvider::Offline => body
                .child(self.auth_information(state, cx))
                .children(state.error.clone().map(|error| {
                    div()
                        .text_sm()
                        .text_color(rgb(self.palette.destructive))
                        .child(error)
                })),
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
        if state.return_tab != Tab::Onboarding || state.other_sign_in_options {
            choices = choices
                .child(components::card(self.palette)
                    .child(components::radio("auth-authlib","Authlib-Injector",state.provider==AccountProvider::Authlib,self.palette,ControlState::default(),cx.listener(|view,(),window,cx|view.handle(UiAction::AuthProvider(AccountProvider::Authlib),window,cx))))
                    .child(note("A community authentication provider. Its account and password are separate from Microsoft.")))
                .child(components::card(self.palette)
                    .child(components::radio("auth-offline", "Offline Mode", state.provider == AccountProvider::Offline, self.palette,
                        ControlState::default(), cx.listener(|view, (), window, cx| view.handle(UiAction::AuthProvider(AccountProvider::Offline), window, cx))))
                    .child(note("A local player identity. Does not grant access to authenticated online services.")));
        }
        if state.return_tab == Tab::Onboarding {
            choices = choices.child(self.auth_button(
                "auth-other-options",
                if state.other_sign_in_options {
                    "Hide other sign-in options"
                } else {
                    "Other sign-in options"
                },
                UiAction::AuthOtherOptions,
                ButtonVariant::Outline,
                ControlState::default(),
                cx,
            ));
        }
        choices.child(note("Finish saves the account securely on this device."))
    }

    fn auth_information(&self, state: &AuthWizard, cx: &Context<'_, Self>) -> gpui::Div {
        if state.provider == AccountProvider::Authlib {
            let provider = self
                .shell
                .providers
                .records
                .iter()
                .find(|p| Some(p.id) == state.external_provider);
            let mut card = components::card(self.palette)
                .gap_3()
                .child(
                    div()
                        .flex()
                        .items_end()
                        .gap_2()
                        .child(div().flex_1().min_w_0().child(self.product_select_field(
                            "auth-provider-picker",
                            "Authentication provider",
                            provider.map_or("Choose a server", |p| p.name.as_str()),
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
                card = card.child(note(
                    "No saved providers. Use + to add your server or import its quick link.",
                ));
            }
            if state.provider_draft.is_some() {
                card = card.child(note(
                    "Finish or close the provider dialog before signing in.",
                ));
            }
            if provider.is_some_and(|provider| {
                state.external_api.as_deref() != Some(provider.api_url.as_str())
            }) {
                card = card.child(note(
                    "The provider address changed. Choose it again and re-enter your password.",
                ));
            }
            if let Some(provider) = provider
                && provider.api_url.starts_with("http:")
            {
                card=card.child(div().p_3().rounded_lg().border_1().border_color(rgb(self.palette.destructive))
                    .child(note("This provider uses HTTP. Your password and tokens are sent without encryption. Continue only if you trust this address and network.")))
                    .child(components::checkbox("auth-http-consent","I accept the HTTP risk for this address",state.accepted_http.as_deref()==Some(provider.api_url.as_str()),self.palette,ControlState{disabled:self.busy,..ControlState::default()},cx.listener(|view,(),window,cx|view.handle(UiAction::AuthHttpConsent,window,cx))));
            }
            let mut fields = div().flex().gap_4();
            if let Some(login) = &state.login {
                fields = fields.child(div().flex_1().min_w_0().child(components::field_with(
                    "Account",
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
                            .child("Password *"),
                    )
                    .children(state.password.as_ref().map(|state| ProviderPassword {
                        state: state.clone(),
                        palette: self.palette,
                    })),
            );
            return card.child(fields).child(note("The password is cleared after sending and is never saved. Only Finish stores this provider's tokens securely."));
        }
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
        if state.provider == AccountProvider::Authlib {
            return components::card(self.palette)
                .child(note(if state.phase == Phase::Verifying {
                    "Confirming the selected character with your provider…"
                } else {
                    "Checking your account with the selected provider…"
                }))
                .child(components::progress(
                    ProgressState::Indeterminate,
                    self.palette,
                ));
        }
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
            .child(components::account_avatar(
                &account.player_name,
                self.account_avatars.get(&account.identity_key()),
                self.palette,
                64.,
                AvatarState::Fallback,
            ))
            .child(div().text_xl().child(account.player_name.clone()))
            .child(components::badge(self.account_authority(account), self.palette, BadgeVariant::Secondary))
            .children(self.account_provider(account).map(|provider|note(provider.api_url.clone())))
            .child(components::badge(if state.phase == Phase::Saved { "Account saved" } else { "Ready to save" }, self.palette, BadgeVariant::Success))
            .child(note(if state.phase == Phase::Saved { "The account has been saved and is ready to use." } else { "Finish saves this account securely and selects it. Closing now discards this sign-in." }))
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
                    ControlState {
                        disabled: state.return_tab == Tab::Onboarding
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
                } else if state.provider == AccountProvider::Authlib {
                    "Sign in"
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
            Phase::Characters => ("Continue", UiAction::Authenticate, false),
            Phase::Complete => ("Finish", UiAction::AuthFinish, false),
            Phase::Saving => ("Saving…", UiAction::AuthFinish, true),
            Phase::Saved => ("Done", UiAction::AuthCancel, false),
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
    } else if lower.contains("secure credential store is unavailable or locked") {
        if cfg!(target_os = "linux") {
            "The desktop Secret Service is unavailable or locked. Start or unlock your system keyring, then try again. Credentials are never saved to a plaintext file.".into()
        } else if cfg!(target_os = "windows") {
            "Windows Credential Manager is unavailable. Check access to your Windows user session, then try again. Credentials are never saved to a plaintext file.".into()
        } else {
            "The system keychain is unavailable or locked. Unlock your login keychain, then try again. Credentials are never saved to a plaintext file.".into()
        }
    } else if lower.contains("no supported secure credential store") {
        "This system has no supported secure credential store. Account sign-in is unavailable until a system credential store is configured.".into()
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

    #[test]
    fn secure_store_preflight_errors_explain_system_recovery_without_private_details() {
        let error = block_auth::AuthError::CredentialStoreUnavailable.to_string();
        let message = super::sign_in_error(&format!("{error} PRIVATE_DIAGNOSTIC"));
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
            super::sign_in_error(&block_auth::AuthError::UnsupportedSecureStore.to_string());
        assert!(unsupported.contains("sign-in is unavailable"));
        assert_ne!(
            message,
            super::sign_in_error("Account metadata cannot be written")
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
                        .is_some_and(|draft| draft
                            .error
                            .as_ref()
                            .is_some_and(|error| error.contains("HTTP warning")))
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
                        && state
                            .error
                            .as_ref()
                            .is_some_and(|error| error.contains("provider address changed"))
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
    fn onboarding_account_choices_disclose_other_providers_on_request(cx: &mut TestAppContext) {
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.tab = Tab::Onboarding;
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
                    view.tab = Tab::Onboarding;
                    view.begin_onboarding(cx);
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
                    && view.tab == Tab::Onboarding
                    && view.shell.onboarding.account.as_deref() == Some(account_id.as_str())
                    && view.shell.onboarding.account_decided
                    && view.selected_account.as_deref() == Some(account_id.as_str())
                    && view
                        .accounts
                        .iter()
                        .any(|saved| saved.identity_key() == account.identity_key())
                    && view.shell.onboarding.pending_identity.is_none()
            }));
            assert!(root.path().join("accounts.json").exists());
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
                        view.tab = Tab::Accounts;
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
