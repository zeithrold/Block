use super::*;
impl Block {
    pub(crate) fn auth_provider_detected(
        &mut self,
        provider: Box<block_auth::AuthlibProvider>,
        icon: Option<components::ProviderBadge>,
        cx: &mut Context<'_, Self>,
    ) {
        let revision = self.operation_revision;
        let time = self.provider_motion(cx);
        if let Some(draft) = self.provider_draft_mut()
            && draft.pages.active() == &Page::Address
            && draft
                .attempt
                .as_ref()
                .is_some_and(|(r, c)| *r == revision && !c.is_cancelled())
        {
            let alias = crate::input("", &provider.name, cx);
            draft.fit.invalidate();
            draft.pages.push(
                Page::Confirm(Review {
                    provider,
                    icon,
                    alias,
                }),
                time,
            );
            // Keep the address locked through the outgoing page's slide.
            draft.attempt = None;
            cx.notify();
        }
    }
    pub(super) fn use_auth_provider(
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
            state.error = None;
            state.provider_draft = None;
            state.transition(Phase::Connect);
            state.sync_auth_provider_selection(window, cx);
        }
        self.shell.surfaces.close(window, &self.focus, cx);
        cx.notify();
    }

    pub(crate) fn auth_provider_finished(
        &mut self,
        result: &Result<crate::i18n::UiText, String>,
        cx: &mut Context<'_, Self>,
    ) {
        if let Err(error) = result {
            self.fail_auth_provider(AccountMessage::diagnostic(error.clone()), cx);
        } else if self.cancel.is_cancelled() {
            // A user can stop after the worker sends success but before the UI polls it.
            self.fail_auth_provider(AccountMessage::product("runtime.cancelled"), cx);
        }
    }

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
            {
                let field = match draft.pages.active() {
                    Page::Address => &draft.address,
                    Page::Confirm(review) => &review.alias,
                };
                field.update(cx, |field, cx| field.set_read_only(false, cx));
            }
        }
    }
    pub(crate) fn auth_provider_saved(&mut self, id: &str, cx: &mut Context<'_, Self>) -> bool {
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
        if !matches!(draft.pages.active(), Page::Confirm(review) if review.provider.id == id) {
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
}
