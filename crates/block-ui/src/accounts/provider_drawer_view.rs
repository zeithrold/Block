//! Page bodies scroll independently; header and primary action stay pinned.
use super::*;
use crate::drawer_page::{DrawerHeight, DrawerPage, HeaderAction, HeaderSlots, ScrollPage};
impl Block {
    pub(crate) fn back_auth_provider(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let time = self.provider_motion(cx);
        if self.busy {
            return;
        }
        if let Some(draft) = self.provider_draft_mut()
            && draft.pages.can_back()
        {
            if let Page::Confirm(review) = draft.pages.active() {
                review
                    .alias
                    .update(cx, |input, cx| input.set_read_only(true, cx));
            }
            draft.pages.back(time);
            draft.fit.invalidate();
            draft.error = None;
            draft
                .address
                .update(cx, |input, cx| input.set_read_only(false, cx));
            draft.address.read(cx).focus_handle(cx).focus(window, cx);
        }
        cx.notify();
    }
    pub(crate) fn provider_header(&self) -> HeaderSlots {
        HeaderSlots {
            leading: self
                .provider_draft()
                .filter(|d| d.pages.can_back())
                .map(|_| HeaderAction {
                    id: "auth-provider-back",
                    label: tr!(self.language(), "action.back"),
                    icon: Icon::ChevronLeft,
                    action: UiAction::AuthProviderBack,
                    disabled: self.busy,
                }),
            trailing: None,
        }
    }
    pub(crate) fn provider_chrome(&self) -> (String, String, DrawerHeight) {
        let lang = self.language();
        let height = self.provider_draft().map_or(DrawerHeight::Retain, |draft| {
            DrawerHeight::Content(draft.fit.clone())
        });
        if self.provider_draft().is_some_and(|d| d.pages.can_back()) {
            (
                tr!(lang, "accounts.auth.provider_add.confirm"),
                tr!(lang, "accounts.auth.provider_add.review_hint"),
                height,
            )
        } else {
            (
                tr!(lang, "accounts.auth.provider_add.title"),
                tr!(lang, "accounts.auth.provider_add.description"),
                height,
            )
        }
    }
    pub(crate) fn render_auth_provider_add(
        &self,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> gpui::AnyElement {
        let Some(draft) = self.provider_draft() else {
            return div().into_any_element();
        };
        let lang = self.language();
        let raw = draft.address.read(cx).value(cx);
        let destination = self.provider_destination(draft, &raw).ok();
        let http = matches!(destination, Some(Destination::Http { .. }));
        let consent = matches!(destination, Some(Destination::Http { approved: true, .. }));
        let time = self.provider_motion(cx);
        let opacity = draft.warning_opacity(http, time, window);
        let body = draft.pages.render(time, window, |page, active| {
            let error = draft
                .error
                .as_ref()
                .filter(|_| active)
                .map(|error| error.render(lang).into());
            let (id, scroll, content) = match page {
                Page::Address => {
                    let mut body = div()
                        .flex()
                        .flex_col()
                        .gap_4()
                        .child(components::field_with(
                            tr!(lang, "accounts.auth.provider_add.address.label"),
                            &draft.address,
                            self.palette,
                            components::FieldOptions {
                                required: true,
                                error,
                                ..Default::default()
                            },
                        ));
                    if let Some(Destination::Saved(provider)) = &destination {
                        body = body.child(note(tr!(
                            lang,
                            "accounts.auth.provider_add.existing",
                            name = provider.name
                        )));
                    } else if http {
                        let interactive = active && !self.busy && draft.warning_revealed();
                        let shown = raw.clone();
                        let control = components::checkbox(
                            "auth-provider-http",
                            tr!(lang, "accounts.auth.http_consent"),
                            consent,
                            self.palette,
                            ControlState {
                                disabled: !interactive,
                                ..Default::default()
                            },
                            cx.listener(move |view, (), window, cx| {
                                if view
                                    .provider_draft()
                                    .is_some_and(|draft| draft.address.read(cx).value(cx) == shown)
                                {
                                    view.handle(UiAction::AuthProviderConsent, window, cx);
                                }
                            }),
                        )
                        .tab_stop(interactive);
                        let warning = div()
                            .flex()
                            .flex_col()
                            .gap_4()
                            .child(note(tr!(lang, "accounts.auth.provider_add.http_note")))
                            .child(checkbox::ConsentControl {
                                content: div().child(control),
                                enabled: interactive,
                                painted: draft.http_warning.painted_control.clone(),
                            });
                        body = body.child(if active && opacity > 0. {
                            warning
                                .opacity(opacity)
                                .debug_selector(|| "auth-provider-warning".into())
                                .into_any_element()
                        } else {
                            crate::drawer_page::reserve(warning)
                        });
                    }
                    ("authlib-add-body", &draft.address_scroll, body)
                }
                Page::Confirm(review) => {
                    let p = &review.provider;
                    let mut body = div().w_full().min_w_0().flex().flex_col().gap_4().child(
                        components::card(self.palette)
                            .debug_selector(|| "auth-provider-preview".into())
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_3()
                            .min_w_0()
                            .child(components::provider_icon(
                                &p.name,
                                review.icon.as_ref(),
                                self.palette,
                                40.,
                            ))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(
                                        div()
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .child(p.name.clone()),
                                    )
                                    .child(note(p.api_url.clone())),
                            ),
                    );
                    if let Some(provider) = self.saved_at(&p.api_url) {
                        body = body.child(note(tr!(
                            lang,
                            "accounts.auth.provider_add.existing",
                            name = provider.name
                        )));
                    } else {
                        body = body.child(components::field_with(
                            tr!(lang, "accounts.auth.provider_add.alias"),
                            &review.alias,
                            self.palette,
                            components::FieldOptions {
                                hint: Some(
                                    tr!(lang, "accounts.auth.provider_add.alias_hint").into(),
                                ),
                                error,
                                required: false,
                            },
                        ));
                    }
                    ("authlib-confirm-body", &draft.review_scroll, body)
                }
            };
            ScrollPage { id, scroll, active }.body_with(
                self.palette,
                content.into_any_element(),
                Some(&draft.fit),
            )
        });
        let (id, title, action, blocked) = match draft.pages.active() {
            Page::Address => (
                "auth-provider-identify",
                if matches!(destination, Some(Destination::Saved(_))) {
                    tr!(lang, "accounts.auth.provider_add.use_saved")
                } else if http && !consent {
                    tr!(lang, "accounts.auth.provider_add.confirm_http")
                } else {
                    tr!(lang, "accounts.auth.provider_add.identify")
                },
                UiAction::AuthProviderIdentify,
                destination.is_none() || http && !consent,
            ),
            Page::Confirm(review) => (
                "auth-provider-save",
                if self.saved_at(&review.provider.api_url).is_some() {
                    tr!(lang, "accounts.auth.provider_add.use_saved")
                } else {
                    tr!(lang, "accounts.auth.provider_add.add")
                },
                UiAction::AuthProviderSave,
                false,
            ),
        };
        let footer = self.auth_button(
            id,
            title,
            action,
            ButtonVariant::Primary,
            ControlState {
                busy: draft.attempt.is_some(),
                disabled: self.busy || blocked,
                ..Default::default()
            },
            cx,
        );
        DrawerPage::new("authlib-add-page", body, footer.into_any_element())
            .render_with(Some(&draft.fit))
    }
}
