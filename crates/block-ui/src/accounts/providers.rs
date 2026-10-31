//! Provider management keeps editable fields separate from discovered, saved metadata.
#![expect(
    clippy::redundant_pub_crate,
    reason = "Provider state belongs to the private product shell."
)]

use block_auth::{AuthlibProvider, ProviderAddress};
use gpui::{
    Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, ScrollHandle,
    Styled as _, Window, div, px, rgb,
};

use super::{
    Block, Tab, TextInput, UiAction,
    authentication::AccountMessage,
    components::{self, ButtonVariant, ControlState, FieldOptions},
    input, note, patterns,
    scroll_area::ScrollAreaExt as _,
};

#[derive(Debug, Default)]
pub(super) struct State {
    pub visible: bool,
    pub records: Vec<AuthlibProvider>,
    pub loaded: bool,
    pub load_error: Option<String>,
    query: Option<Entity<TextInput>>,
    form: Option<ProviderForm>,
    list_scroll: ScrollHandle,
    body_scroll: ScrollHandle,
    error: Option<AccountMessage>,
    attempt: Option<(u64, block_launcher::CancellationToken)>,
}

#[derive(Debug)]
struct ProviderForm {
    original: Option<AuthlibProvider>,
    name: Entity<TextInput>,
    address: Entity<TextInput>,
    accepted_address: Option<String>,
    observed_address: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ProviderKey {
    List,
    New,
    Existing(uuid::Uuid),
}

impl State {
    pub(super) fn refresh_placeholders(
        &self,
        language: crate::Language,
        cx: &mut Context<'_, Block>,
    ) {
        if let Some(query) = &self.query {
            let placeholder = tr!(language, "accounts.providers.search.placeholder");
            query.update(cx, |input, cx| input.set_placeholder(&placeholder, cx));
        }
        if let Some(form) = &self.form {
            let placeholder = tr!(language, "accounts.providers.name.placeholder");
            form.name
                .update(cx, |input, cx| input.set_placeholder(&placeholder, cx));
        }
    }

    pub(super) fn motion_key(&self) -> ProviderKey {
        self.form.as_ref().map_or_else(
            || ProviderKey::List,
            |form| {
                form.original.as_ref().map_or(ProviderKey::New, |provider| {
                    ProviderKey::Existing(provider.id)
                })
            },
        )
    }
    pub(super) fn snapshot(&self, cx: &mut Context<'_, Block>) -> Self {
        let list_scroll = ScrollHandle::new();
        list_scroll.set_offset(self.list_scroll.offset());
        let body_scroll = ScrollHandle::new();
        body_scroll.set_offset(self.body_scroll.offset());
        Self {
            visible: self.visible,
            records: self.records.clone(),
            loaded: self.loaded,
            load_error: self.load_error.clone(),
            query: self
                .query
                .as_ref()
                .map(|field| super::frozen_input(field, cx)),
            form: self.form.as_ref().map(|form| ProviderForm {
                original: form.original.clone(),
                name: super::frozen_input(&form.name, cx),
                address: super::frozen_input(&form.address, cx),
                accepted_address: form.accepted_address.clone(),
                observed_address: form.observed_address.clone(),
            }),
            list_scroll,
            body_scroll,
            error: self.error.clone(),
            attempt: None,
        }
    }
}

#[cfg(any(test, debug_assertions))]
pub(super) fn fixture_provider(insecure: bool) -> anyhow::Result<AuthlibProvider> {
    let base = if insecure {
        "http://community.example.test/"
    } else {
        "https://community.example.test/"
    };
    let key = "-----BEGIN PUBLIC KEY-----\nMBkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDAgAA\n-----END PUBLIC KEY-----";
    let metadata = block_auth::ProviderMetadata {
        server_name: Some("Fixture Community".into()),
        icon_url: format!("{base}favicon.ico"),
        skin_domains: vec!["textures.example.test".into()],
        signature_public_key: key.into(),
    };
    let provider = AuthlibProvider { id: uuid::Uuid::from_u128(if insecure {2}else{1}), name:if insecure {"Local community"}else{"Community server"}.into(), api_url: format!("{base}api/"), allow_http:insecure,
        metadata_json:serde_json::json!({"meta":{"serverName":"Fixture Community"},"skinDomains":["textures.example.test"],"signaturePublickey":key}).to_string(), metadata };
    provider.validate()?;
    Ok(provider)
}

impl Block {
    #[cfg(debug_assertions)]
    pub(super) fn provider_scenario(
        &mut self,
        name: &str,
        cx: &mut Context<'_, Self>,
    ) -> anyhow::Result<()> {
        self.router.navigate(Tab::Accounts);
        self.shell.providers.records = vec![fixture_provider(false)?, fixture_provider(true)?];
        self.shell.providers.loaded = true;
        self.open_providers(cx);
        if name == "provider-add" || name == "provider-http-warning" {
            self.edit_provider(None, cx);
            if name == "provider-http-warning"
                && let Some(form) = &self.shell.providers.form
            {
                form.name
                    .update(cx, |field, cx| field.set_value("Local community", cx));
                form.address.update(cx, |field, cx| {
                    field.set_value("http://community.example.test/api/", cx)
                });
            }
        } else {
            self.edit_provider(Some(&uuid::Uuid::from_u128(1).to_string()), cx);
        }
        Ok(())
    }
    pub(super) fn open_providers(&mut self, cx: &mut Context<'_, Self>) {
        if self.router.tab() != Tab::Accounts {
            return;
        }
        let language = self.language();
        let placeholder = tr!(language, "accounts.providers.search.placeholder");
        if self.shell.providers.query.is_none() {
            self.shell.providers.query = Some(input("", &placeholder, cx));
        }
        // A retained search field keeps its draft and IME session; only its
        // placeholder follows the current language.
        if let Some(query) = &self.shell.providers.query {
            query.update(cx, |field, cx| field.set_placeholder(&placeholder, cx));
        }
        self.shell.providers.visible = true;
        cx.notify();
    }

    pub(super) fn close_providers(&mut self, cx: &mut Context<'_, Self>) {
        if let Some((revision, cancel)) = self.shell.providers.attempt.take() {
            cancel.cancel();
            self.abandon_operation(revision);
        }
        self.shell.providers.visible = false;
        self.shell.providers.form = None;
        self.shell.providers.error = None;
        cx.notify();
    }

    pub(super) fn edit_provider(&mut self, id: Option<&str>, cx: &mut Context<'_, Self>) {
        if self.busy {
            return;
        }
        let original = id
            .and_then(|id| {
                self.shell
                    .providers
                    .records
                    .iter()
                    .find(|p| p.id.to_string() == id)
            })
            .cloned();
        if id.is_some() && original.is_none() {
            return;
        }
        let name = input(
            original.as_ref().map_or("", |p| p.name.as_str()),
            &tr!(self.language(), "accounts.providers.name.placeholder"),
            cx,
        );
        let address = input(
            original.as_ref().map_or("", |p| p.api_url.as_str()),
            "https://example.com/api/yggdrasil/",
            cx,
        );
        if original
            .as_ref()
            .is_some_and(|p| self.provider_links(&p.id.to_string()) > 0)
        {
            address.update(cx, |field, cx| field.set_read_only(true, cx));
        }
        cx.observe(&address, |view, field, cx| {
            if let Some(form) = &mut view.shell.providers.form
                && form.address == field
                && form.observed_address != field.read(cx).value()
            {
                form.observed_address = field.read(cx).value();
                form.accepted_address = None;
                view.shell.providers.error = None;
                cx.notify();
            }
        })
        .detach();
        let accepted_address = original
            .as_ref()
            .filter(|p| p.allow_http)
            .map(|p| p.api_url.clone());
        self.shell.providers.form = Some(ProviderForm {
            observed_address: address.read(cx).value(),
            original,
            name,
            address,
            accepted_address,
        });
        self.shell.providers.error = None;
        self.shell
            .providers
            .body_scroll
            .set_offset(gpui::point(px(0.), px(0.)));
        cx.notify();
    }

    pub(super) fn receive_providers(&mut self, result: Result<Vec<AuthlibProvider>, String>) {
        self.shell.providers.loaded = true;
        match result {
            Ok(records) => {
                if self
                    .shell
                    .providers
                    .form
                    .as_ref()
                    .and_then(|f| f.original.as_ref())
                    .is_some_and(|old| !records.iter().any(|p| p.id == old.id))
                {
                    self.shell.providers.form = None;
                }
                self.shell.providers.records = records;
                self.shell.providers.load_error = None;
            }
            Err(error) => self.shell.providers.load_error = Some(error),
        }
    }

    pub(super) fn provider_saved(&mut self, id: &str, cx: &mut Context<'_, Self>) {
        if self.router.tab() == Tab::Accounts
            && self.shell.providers.visible
            && self
                .shell
                .providers
                .attempt
                .as_ref()
                .is_some_and(|(revision, _)| *revision == self.operation_revision)
        {
            // Finish's status arrives next; it must not reopen a different draft.
            let busy = self.busy;
            self.busy = false;
            self.edit_provider(Some(id), cx);
            self.busy = busy;
        }
    }

    pub(super) fn providers_finished(&mut self, result: &Result<crate::i18n::UiText, String>) {
        if self
            .shell
            .providers
            .attempt
            .as_ref()
            .is_some_and(|(revision, _)| *revision == self.operation_revision)
        {
            self.shell.providers.attempt = None;
            self.shell.providers.error = result
                .as_ref()
                .err()
                .map(|error| AccountMessage::diagnostic(error.clone()));
        }
    }

    pub(super) fn identify_provider(&mut self, cx: &mut Context<'_, Self>) {
        if self.busy {
            return;
        }
        let Some(form) = &self.shell.providers.form else {
            return;
        };
        let parsed = ProviderAddress::parse(&form.address.read(cx).value());
        match parsed {
            Ok(parsed) => {
                let existing = self
                    .shell
                    .providers
                    .records
                    .iter()
                    .find(|p| p.api_url == parsed.url)
                    .map(|p| p.id.to_string());
                if let Some(existing) = existing {
                    self.edit_provider(Some(&existing), cx);
                    return;
                }
                if let Some(form) = &mut self.shell.providers.form {
                    if form.name.read(cx).value().trim().is_empty() {
                        form.name
                            .update(cx, |field, cx| field.set_value(&parsed.suggested_name, cx));
                    }
                    form.address
                        .update(cx, |field, cx| field.set_value(&parsed.url, cx));
                    form.accepted_address = None;
                }
                self.shell.providers.error = None;
            }
            Err(error) => {
                self.shell.providers.error = Some(AccountMessage::diagnostic(error.to_string()));
            }
        }
        cx.notify();
    }

    pub(super) fn consent_provider(&mut self, cx: &mut Context<'_, Self>) {
        if let Some(form) = &mut self.shell.providers.form {
            let address = form.address.read(cx).value().trim().to_owned();
            form.accepted_address = if form.accepted_address.as_ref() == Some(&address) {
                None
            } else {
                Some(address)
            };
        }
        cx.notify();
    }

    pub(super) fn save_provider(&mut self, cx: &mut Context<'_, Self>) {
        if self.busy {
            return;
        }
        let Some(form) = &self.shell.providers.form else {
            return;
        };
        let name = form.name.read(cx).value().trim().to_owned();
        let address = form.address.read(cx).value().trim().to_owned();
        let parsed = ProviderAddress::parse(&address);
        let error = if name.is_empty() {
            Some(AccountMessage::product("accounts.providers.error.name"))
        } else if let Err(error) = &parsed {
            Some(AccountMessage::diagnostic(error.to_string()))
        } else if parsed.as_ref().is_ok_and(|p| p.insecure)
            && form.accepted_address.as_ref() != Some(&address)
        {
            Some(AccountMessage::diagnostic(
                block_auth::AuthError::InsecureProvider.to_string(),
            ))
        } else if self.engine.is_none() {
            Some(AccountMessage::product(
                "accounts.providers.error.save_unavailable",
            ))
        } else {
            None
        };
        if error.is_some() {
            self.shell.providers.error = error;
            cx.notify();
            return;
        }
        let original = form.original.clone();
        let accepted = form.accepted_address.as_ref() == Some(&address);
        self.begin_cancellable(
            msg!("accounts.providers.status.discovering"),
            super::ui_logging::Operation::SaveProvider,
        );
        self.operation_kind = super::OperationKind::Account;
        self.shell.providers.attempt = Some((self.operation_revision, self.cancel.clone()));
        self.shell.providers.error = None;
        if let Some(engine) = &self.engine {
            engine.save_provider(original, name, address, accepted, self.cancel.clone());
        }
        cx.notify();
    }

    pub(super) fn provider_links(&self, id: &str) -> usize {
        self.accounts
            .iter()
            .filter(|a| {
                a.authlib
                    .as_ref()
                    .is_some_and(|link| link.provider_id.to_string() == id)
            })
            .count()
    }

    pub(super) fn request_remove_provider(
        &mut self,
        id: String,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.busy {
            return;
        }
        if self.provider_links(&id) > 0 {
            self.shell.providers.error =
                Some(AccountMessage::product("accounts.providers.error.linked"));
        } else if self
            .shell
            .providers
            .records
            .iter()
            .any(|p| p.id.to_string() == id)
        {
            self.shell
                .surfaces
                .open(super::shell::Surface::RemoveProvider(id), window, cx);
        }
        cx.notify();
    }

    pub(super) fn remove_provider(
        &mut self,
        id: String,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.busy {
            return;
        }
        self.shell.surfaces.close(window, &self.focus, cx);
        if self.engine.is_some() {
            self.begin(
                msg!("accounts.providers.status.removing"),
                super::ui_logging::Operation::RemoveProvider,
            );
            self.operation_kind = super::OperationKind::Account;
            self.shell.providers.attempt = Some((self.operation_revision, self.cancel.clone()));
            if let Some(engine) = &self.engine {
                engine.remove_provider(id);
            }
        } else {
            self.shell.providers.error = Some(AccountMessage::product(
                "accounts.providers.error.remove_unavailable",
            ));
        }
        cx.notify();
    }

    fn provider_button(
        &self,
        id: impl Into<gpui::SharedString>,
        title: impl Into<gpui::SharedString>,
        action: UiAction,
        disabled: bool,
        variant: ButtonVariant,
        cx: &Context<'_, Self>,
    ) -> components::Control {
        components::button(
            id,
            title,
            self.palette,
            variant,
            ControlState {
                disabled,
                ..ControlState::default()
            },
            cx.listener(move |view, (), window, cx| view.handle(action.clone(), window, cx)),
        )
    }

    pub(super) fn render_providers(
        &self,
        cx: &Context<'_, Self>,
    ) -> super::page_host::ManagementSlots {
        let language = self.language();
        let state = &self.shell.providers;
        let query = state.query.as_ref().map_or_else(String::new, |field| {
            field.read(cx).value().trim().to_lowercase()
        });
        let mut rows = div().flex().flex_col();
        for provider in &state.records {
            if !format!("{} {}", provider.name, provider.api_url)
                .to_lowercase()
                .contains(&query)
            {
                continue;
            }
            let selected = state
                .form
                .as_ref()
                .and_then(|f| f.original.as_ref())
                .is_some_and(|p| p.id == provider.id);
            rows = rows.child(
                self.provider_button(
                    format!("provider-inspect-{}", provider.id),
                    "",
                    UiAction::ProviderInspect(provider.id.to_string()),
                    self.busy,
                    if selected {
                        ButtonVariant::Secondary
                    } else {
                        ButtonVariant::Ghost
                    },
                    cx,
                )
                .w_full()
                .justify_start()
                .h_auto()
                .py_4()
                .child(components::provider_icon(
                    &provider.name,
                    self.provider_icons.get(&provider.id.to_string()),
                    self.palette,
                    24.,
                ))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .items_start()
                        .gap_2()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(gpui::FontWeight::NORMAL)
                                .truncate()
                                .child(provider.name.clone()),
                        )
                        .child(components::badge(
                            if provider.api_url.starts_with("https:") {
                                "HTTPS"
                            } else {
                                "HTTP"
                            },
                            self.palette,
                            components::BadgeVariant::Secondary,
                        )),
                ),
            );
        }
        if !state.loaded {
            rows = rows.child(note(tr!(language, "accounts.providers.loading")));
        } else if state.load_error.is_some() && state.records.is_empty() {
            rows = rows.child(note(tr!(language, "accounts.providers.load_failed")));
        } else if state.records.is_empty() {
            rows = rows.child(note(tr!(language, "accounts.providers.empty")));
        } else if !state.records.iter().any(|p| {
            format!("{} {}", p.name, p.api_url)
                .to_lowercase()
                .contains(&query)
        }) {
            rows = rows.child(note(tr!(language, "accounts.providers.none_found")));
        }
        if let Some(error) = &state.load_error {
            rows = rows.child(note(error.clone())).child(self.provider_button(
                "providers-reload",
                tr!(language, "accounts.providers.reload"),
                UiAction::ReloadLibrary,
                self.busy,
                ButtonVariant::Outline,
                cx,
            ));
        }
        let rail = div()
            .w_full()
            .h_full()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .p_4()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(note(if !state.loaded {
                        tr!(language, "accounts.providers.rail.title")
                    } else if state.load_error.is_some() {
                        tr!(language, "accounts.providers.rail.unavailable")
                    } else {
                        tr!(
                            language,
                            "accounts.providers.rail.count",
                            count = state.records.len()
                        )
                    }))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .children(
                                state
                                    .query
                                    .as_ref()
                                    .map(|field| div().flex_1().min_w_0().child(field.clone())),
                            )
                            .child(self.provider_button(
                                "provider-add",
                                "+",
                                UiAction::ProviderAdd,
                                self.busy,
                                ButtonVariant::Outline,
                                cx,
                            )),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .id("provider-list-scroll")
                    .scroll_area(
                        &state.list_scroll,
                        "accounts.providers.list",
                        self.palette,
                        false,
                    )
                    .p_3()
                    .child(rows),
            )
            .child(
                patterns::management_footer(self.palette).child(self.provider_button(
                    "providers-back",
                    tr!(language, "accounts.providers.back"),
                    UiAction::ProvidersBack,
                    false,
                    ButtonVariant::Ghost,
                    cx,
                )),
            );
        super::page_host::ManagementSlots {
            sidebar: rail.into_any_element(),
            detail: self.provider_editor(cx).into_any_element(),
        }
    }

    pub(super) fn provider_editor(&self, cx: &Context<'_, Self>) -> gpui::Div {
        let language = self.language();
        let mut content = div().p_8().flex().flex_col().gap_6();
        let mut footer = patterns::management_footer(self.palette).justify_between();
        if let Some(form) = &self.shell.providers.form {
            let address = form.address.read(cx).value();
            let insecure = ProviderAddress::parse(&address).is_ok_and(|p| p.insecure);
            content = self.provider_form_fields(content, form, cx);
            if insecure {
                content = content.child(self.provider_http_consent(form, &address, cx));
            }
            content = self.provider_identify_section(content, cx);
            if let Some(original) = &form.original {
                let links = self.provider_links(&original.id.to_string());
                content = content.children(provider_saved_notes(original, links, language));
                footer = footer.child(self.provider_button(
                    "provider-remove",
                    tr!(language, "accounts.providers.remove"),
                    UiAction::ProviderRemoveRequest(original.id.to_string()),
                    self.busy || links > 0,
                    ButtonVariant::Ghost,
                    cx,
                ));
            } else {
                footer = footer.child(div());
            }
            footer = footer.child(self.provider_button(
                "provider-save",
                if form.original.is_some() {
                    tr!(language, "accounts.providers.save_changes")
                } else {
                    tr!(language, "accounts.providers.add")
                },
                UiAction::ProviderSave,
                self.busy || insecure && form.accepted_address.as_deref() != Some(address.trim()),
                ButtonVariant::Primary,
                cx,
            ));
        } else {
            content = content
                .child(patterns::page_heading(
                    tr!(language, "accounts.providers.empty_title"),
                    tr!(language, "accounts.providers.empty_description"),
                ))
                .child(self.provider_button(
                    "provider-new",
                    tr!(language, "accounts.providers.add"),
                    UiAction::ProviderAdd,
                    self.busy,
                    ButtonVariant::Primary,
                    cx,
                ));
        }
        if let Some(error) = &self.shell.providers.error {
            content = content
                .child(note(error.render(language)).text_color(rgb(self.palette.destructive)));
        }
        if self.shell.providers.attempt.is_some() && self.busy {
            content = content
                .child(components::progress(
                    components::ProgressState::Indeterminate,
                    self.palette,
                    tr!(language, "common.in_progress"),
                ))
                .child(note(self.status.text(self.language())));
        }
        div()
            .size_full()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .id("provider-editor-scroll")
                    .scroll_area(
                        &self.shell.providers.body_scroll,
                        "accounts.providers.details",
                        self.palette,
                        false,
                    )
                    .child(content),
            )
            .child(footer)
    }

    /// The provider's icon and heading above its editable name and address fields.
    fn provider_form_fields(
        &self,
        content: gpui::Div,
        form: &ProviderForm,
        cx: &Context<'_, Self>,
    ) -> gpui::Div {
        let language = self.language();
        let header_icon = form
            .original
            .as_ref()
            .and_then(|provider| self.provider_icons.get(&provider.id.to_string()));
        content
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_4()
                    .child(components::provider_icon(
                        &form.name.read(cx).value(),
                        header_icon,
                        self.palette,
                        36.,
                    ))
                    .child(patterns::page_heading(
                        if form.original.is_some() {
                            tr!(language, "accounts.providers.details.title")
                        } else {
                            tr!(language, "accounts.providers.add.title")
                        },
                        tr!(language, "accounts.providers.subtitle"),
                    )),
            )
            .child(components::field_with(
                tr!(language, "accounts.providers.name.label"),
                &form.name,
                self.palette,
                FieldOptions {
                    required: true,
                    ..FieldOptions::default()
                },
            ))
            .child(components::field_with(
                tr!(language, "accounts.providers.address.label"),
                &form.address,
                self.palette,
                FieldOptions {
                    required: true,
                    ..FieldOptions::default()
                },
            ))
    }

    /// The consent card that an insecure provider address requires before it can be saved.
    fn provider_http_consent(
        &self,
        form: &ProviderForm,
        address: &str,
        cx: &Context<'_, Self>,
    ) -> gpui::Div {
        let language = self.language();
        components::card(self.palette)
            .child(note(tr!(language, "accounts.providers.http_note")))
            .child(components::checkbox(
                "provider-http-consent",
                tr!(language, "accounts.providers.http_consent"),
                form.accepted_address.as_deref() == Some(address.trim()),
                self.palette,
                ControlState {
                    disabled: self.busy,
                    ..ControlState::default()
                },
                cx.listener(|view, (), window, cx| {
                    view.handle(UiAction::ProviderConsent, window, cx)
                }),
            ))
    }

    /// The identify action, which reads the provider an address names.
    fn provider_identify_section(&self, content: gpui::Div, cx: &Context<'_, Self>) -> gpui::Div {
        let language = self.language();
        content
            .child(note(tr!(language, "accounts.providers.identify_note")))
            .child(self.provider_button(
                "provider-identify",
                tr!(language, "accounts.providers.identify"),
                UiAction::ProviderIdentify,
                self.busy,
                ButtonVariant::Outline,
                cx,
            ))
    }
}

/// The notes describing the metadata a saved provider keeps immutable.
fn provider_saved_notes(
    original: &AuthlibProvider,
    links: usize,
    language: crate::Language,
) -> Vec<gpui::AnyElement> {
    let mut notes = vec![
        note(tr!(language, "accounts.providers.links", count = links)).into_any_element(),
        note(tr!(
            language,
            "accounts.providers.server",
            name = original
                .metadata
                .server_name
                .as_deref()
                .unwrap_or(&original.name)
        ))
        .into_any_element(),
        note(tr!(
            language,
            "accounts.providers.skins",
            domains = original.metadata.skin_domains.join(", ")
        ))
        .into_any_element(),
    ];
    if links > 0 {
        notes.push(note(tr!(language, "accounts.providers.address_locked")).into_any_element());
    }
    notes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn provider_drafts_import_existing_links_and_http_consent_tracks_address_edits(
        cx: &mut gpui::TestAppContext,
    ) {
        let result = (|| -> anyhow::Result<()> {
            let existing = fixture_provider(false)?;
            let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
            cx.update(|_,cx|view.update(cx,|view,cx| {
                view.router.navigate(Tab::Accounts);
                view.shell.providers.loaded = true;
                view.shell.providers.records = vec![existing.clone()];
                view.open_providers(cx);
                view.edit_provider(None,cx);
                if let Some(form) = &view.shell.providers.form {
                    form.address.update(cx,|field,cx|field.set_value("authlib-injector:yggdrasil-server:https%3A%2F%2Fcommunity.example.test%2Fapi%2F",cx));
                }
                view.identify_provider(cx);
                assert!(view.shell.providers.form.as_ref().and_then(|f|f.original.as_ref()).is_some_and(|p|p.id==existing.id));
                assert_eq!(view.shell.providers.records,vec![existing.clone()]);
                view.edit_provider(None,cx);
                if let Some(form) = &view.shell.providers.form {
                    form.name.update(cx,|field,cx|field.set_value("Draft community",cx));
                    form.address.update(cx,|field,cx|field.set_value("http://localhost:8080/api/",cx));
                }
            }));
            cx.run_until_parked();
            cx.update(|_, cx| {
                view.update(cx, |view, cx| {
                    view.save_provider(cx);
                    assert!(view.shell.providers.error.as_ref().is_some_and(|error| {
                        error.render(crate::Language::English).contains("HTTP")
                    }));
                    view.consent_provider(cx);
                    assert!(
                        view.shell
                            .providers
                            .form
                            .as_ref()
                            .is_some_and(|f| f.accepted_address.is_some())
                    );
                    if let Some(form) = &view.shell.providers.form {
                        form.address.update(cx, |_, cx| cx.notify());
                    }
                })
            });
            cx.run_until_parked();
            cx.update(|_, cx| {
                view.update(cx, |view, cx| {
                    assert!(
                        view.shell
                            .providers
                            .form
                            .as_ref()
                            .is_some_and(|f| f.accepted_address.is_some())
                    );
                    if let Some(form) = &view.shell.providers.form {
                        form.address.update(cx, |field, cx| {
                            field.set_value("http://localhost:8081/api/", cx)
                        });
                    }
                })
            });
            cx.run_until_parked();
            cx.update(|_, cx| {
                view.update(cx, |view, cx| {
                    assert!(
                        view.shell
                            .providers
                            .form
                            .as_ref()
                            .is_some_and(|f| f.accepted_address.is_none())
                    );
                    view.close_providers(cx);
                    assert!(view.shell.providers.form.is_none() && !view.shell.providers.visible);
                    assert_eq!(view.shell.providers.records, vec![existing.clone()]);
                })
            });
            Ok(())
        })();
        assert!(result.is_ok(), "{result:?}");
    }

    #[gpui::test]
    fn provider_navigation_and_unrelated_removal_results_keep_other_drafts(
        cx: &mut gpui::TestAppContext,
    ) {
        let result = (|| -> anyhow::Result<()> {
            let old = fixture_provider(false)?;
            let other = fixture_provider(true)?;
            let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
            cx.update(|_, cx| {
                view.update(cx, |view, cx| {
                    view.router.navigate(Tab::Accounts);
                    view.shell.providers.records = vec![old.clone(), other.clone()];
                    view.open_providers(cx);
                    view.edit_provider(Some(&other.id.to_string()), cx);
                    if let Some(form) = &view.shell.providers.form {
                        form.name
                            .update(cx, |field, cx| field.set_value("Unsaved name", cx));
                    }
                    view.receive_providers(Ok(vec![other.clone()]));
                    assert!(
                        view.shell
                            .providers
                            .form
                            .as_ref()
                            .is_some_and(|f| f.name.read(cx).value() == "Unsaved name")
                    );
                    view.busy = true;
                    view.cancel = block_launcher::CancellationToken::new();
                    view.close_providers(cx);
                    assert!(view.busy && !view.cancel.is_cancelled());
                })
            });
            Ok(())
        })();
        assert!(result.is_ok(), "{result:?}");
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
    fn provider_management_renders_the_view_language_and_keeps_retained_drafts(
        cx: &mut gpui::TestAppContext,
    ) {
        let result = (|| -> anyhow::Result<()> {
            let provider = fixture_provider(false)?;
            let (view, cx) = crate::test_support::window(cx, |window, cx| {
                let mut view = crate::tests::test_view(window, cx);
                view.preferences.reduce_motion = true;
                view.preferences.language = Some(crate::Language::Japanese);
                view
            });
            cx.update(|_, cx| {
                view.update(cx, |view, cx| {
                    view.router.navigate(Tab::Accounts);
                    view.shell.providers.records = vec![provider.clone()];
                    view.shell.providers.loaded = true;
                    view.open_providers(cx);
                    view.edit_provider(Some(&provider.id.to_string()), cx);
                    if let Some(form) = &view.shell.providers.form {
                        form.name
                            .update(cx, |field, cx| field.set_value("Draft community", cx));
                    }
                });
            });
            assert_eq!(
                control_node(cx, "providers-back").label(),
                Some("アカウントに戻る")
            );
            assert_eq!(
                control_node(cx, "provider-save").label(),
                Some("変更を保存")
            );

            // The retained search editor and the draft stay in place across a
            // language change while their presentation follows the language.
            let query = view.read_with(cx, |view, _| {
                view.shell
                    .providers
                    .query
                    .as_ref()
                    .map(|field| field.entity_id())
            });
            cx.update(|_, cx| {
                view.update(cx, |view, cx| {
                    view.preferences.language = Some(crate::Language::SimplifiedChinese);
                    view.open_providers(cx);
                });
            });
            assert_eq!(
                view.read_with(cx, |view, _| view
                    .shell
                    .providers
                    .query
                    .as_ref()
                    .map(|field| field.entity_id())),
                query
            );
            assert!(view.read_with(cx, |view, cx| {
                view.shell
                    .providers
                    .form
                    .as_ref()
                    .is_some_and(|form| form.name.read(cx).value() == "Draft community")
            }));
            assert_eq!(control_node(cx, "providers-back").label(), Some("返回账户"));
            Ok(())
        })();
        assert!(result.is_ok(), "{result:?}");
    }
}
