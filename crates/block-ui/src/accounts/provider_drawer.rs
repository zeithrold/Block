use super::*;
use crate::drawer_page::{DrawerFit, DrawerPages, MotionTime};
#[path = "provider_drawer_warning.rs"]
mod warning;
use warning::HttpWarning;
#[path = "provider_drawer_result.rs"]
mod result;
#[path = "provider_drawer_view.rs"]
mod view;
#[derive(Debug, PartialEq)]
enum Page {
    Address,
    Confirm(Review),
}

#[derive(Debug)]
struct Review {
    provider: Box<block_auth::AuthlibProvider>,
    icon: Option<components::ProviderBadge>,
    alias: gpui::Entity<crate::TextInput>,
}
impl PartialEq for Review {
    fn eq(&self, other: &Self) -> bool {
        self.provider.id == other.provider.id
    }
}
#[derive(Debug)]
pub(crate) struct AuthProviderDraft {
    id: uuid::Uuid,
    pub(crate) address: gpui::Entity<crate::TextInput>,
    observed_address: String,
    accepted_http: Option<String>,
    http_warning: HttpWarning,
    pub(super) error: Option<AccountMessage>,
    pub(super) attempt: Option<(u64, block_launcher::CancellationToken)>,
    saved: Option<uuid::Uuid>,
    pages: DrawerPages<Page>,
    fit: DrawerFit,
    address_scroll: ScrollHandle,
    review_scroll: ScrollHandle,
}
impl Block {
    fn provider_draft(&self) -> Option<&AuthProviderDraft> {
        self.shell.auth.as_ref()?.provider_draft.as_ref()
    }
    fn provider_draft_mut(&mut self) -> Option<&mut AuthProviderDraft> {
        self.shell.auth.as_mut()?.provider_draft.as_mut()
    }
    fn provider_motion(&self, cx: &Context<'_, Self>) -> MotionTime {
        MotionTime {
            now: cx.background_executor().now(),
            reduced: self.motion_preferences().reduced(),
        }
    }
    fn saved_at(&self, url: &str) -> Option<&block_auth::AuthlibProvider> {
        self.shell
            .providers
            .records
            .iter()
            .find(|p| p.api_url == url)
    }
    pub(crate) fn open_auth_provider_add(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.provider_draft().is_none() {
            if self.busy
                || self.shell.auth.as_ref().is_none_or(|s| {
                    s.provider != AccountProvider::Authlib
                        || !matches!(s.phase, Phase::Connect | Phase::Failed)
                })
            {
                return;
            }
            let address = crate::input(
                "",
                &tr!(
                    self.language(),
                    "accounts.auth.provider_add.address.placeholder"
                ),
                cx,
            );
            cx.observe(&address, Self::provider_address_edited).detach();
            if let Some(state) = &mut self.shell.auth {
                state.provider_draft = Some(AuthProviderDraft {
                    id: uuid::Uuid::new_v4(),
                    address,
                    observed_address: String::new(),
                    accepted_http: None,
                    http_warning: HttpWarning::default(),
                    error: None,
                    attempt: None,
                    saved: None,
                    pages: DrawerPages::new(Page::Address),
                    fit: DrawerFit::default(),
                    address_scroll: ScrollHandle::new(),
                    review_scroll: ScrollHandle::new(),
                });
            }
        }
        self.shell
            .surfaces
            .open(crate::shell::Surface::AuthProviderAdd, window, cx);
        cx.notify();
    }
    pub(crate) fn cancel_auth_provider(&mut self, cx: &mut Context<'_, Self>) {
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
    pub(crate) fn consent_auth_provider(&mut self, cx: &mut Context<'_, Self>) {
        let Some(draft) = self.provider_draft().filter(|_| !self.busy) else {
            return;
        };
        let raw = draft.address.read(cx).value(cx);
        if let Ok(address) = block_auth::ProviderAddress::parse(&raw)
            && address.insecure
            && self.saved_at(&address.url).is_none()
            && draft.http_consent_ready(&raw)
            && let Some(draft) = self.provider_draft_mut()
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
    pub(crate) fn identify_auth_provider(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(draft) = self
            .provider_draft()
            .filter(|d| !self.busy && d.pages.active() == &Page::Address)
        else {
            return;
        };
        let raw = draft.address.read(cx).value(cx);
        let address = match block_auth::ProviderAddress::parse(&raw) {
            Ok(address) => address,
            Err(error) => {
                self.provider_error(AccountMessage::diagnostic(error.to_string()), cx);
                return;
            }
        };
        if let Some(provider) = self.saved_at(&address.url) {
            self.use_auth_provider(provider.id, window, cx);
            return;
        }
        let accepted = address.insecure
            && draft.http_consent_ready(&raw)
            && draft.accepted_http.as_deref() == Some(&address.url);
        if address.insecure && !accepted {
            self.provider_error(
                AccountMessage::product("accounts.auth.error.provider_http_confirm"),
                cx,
            );
            return;
        }
        self.begin_provider_attempt(
            msg!("accounts.auth.status.discovering_provider"),
            crate::ui_logging::Operation::DiscoverAuthProvider,
            cx,
        );
        if let Some(engine) = &self.engine {
            engine.detect_auth_provider(address.url, accepted, self.cancel.clone());
        }
    }
    fn provider_error(&mut self, error: AccountMessage, cx: &mut Context<'_, Self>) {
        if let Some(draft) = self.provider_draft_mut() {
            draft.error = Some(error);
        }
        cx.notify();
    }
    fn begin_provider_attempt(
        &mut self,
        status: crate::i18n::UiText,
        op: crate::ui_logging::Operation,
        cx: &mut Context<'_, Self>,
    ) {
        if self.engine.is_none() {
            self.provider_error(
                AccountMessage::product("accounts.auth.error.provider_setup_unavailable"),
                cx,
            );
            return;
        }
        self.begin_cancellable(status, op);
        self.operation_kind = crate::OperationKind::Account;
        let attempt = (self.operation_revision, self.cancel.clone());
        if let Some(draft) = self.provider_draft_mut() {
            draft.error = None;
            draft.attempt = Some(attempt);
            match draft.pages.active() {
                Page::Address => &draft.address,
                Page::Confirm(review) => &review.alias,
            }
            .update(cx, |input, cx| input.set_read_only(true, cx));
        }
        cx.notify();
    }
    pub(crate) fn save_auth_provider(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(Page::Confirm(review)) = self
            .provider_draft()
            .filter(|_| !self.busy)
            .map(|d| d.pages.active())
        else {
            return;
        };
        if let Some(existing) = self.saved_at(&review.provider.api_url) {
            self.use_auth_provider(existing.id, window, cx);
            return;
        }
        let mut provider = (*review.provider).clone();
        let alias = review.alias.read(cx).value(cx);
        if !alias.trim().is_empty() {
            provider.name = alias.trim().to_owned();
        }
        if let Err(error) = provider.validate() {
            self.provider_error(AccountMessage::diagnostic(error.to_string()), cx);
            return;
        }
        self.begin_provider_attempt(
            msg!("accounts.auth.provider_add.saving"),
            crate::ui_logging::Operation::SaveProvider,
            cx,
        );
        if let Some(engine) = &self.engine {
            engine.save_detected_provider(provider, self.cancel.clone());
        }
    }
}
