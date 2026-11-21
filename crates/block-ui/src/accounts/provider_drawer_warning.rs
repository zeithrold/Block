use super::*;
use gpui_base::motion::{Easing, Timing};
use std::{
    cell::Cell,
    time::{Duration, Instant},
};

const ADDRESS_DEBOUNCE: Duration = Duration::from_millis(300);

#[derive(Debug, Default)]
pub(super) struct HttpWarning {
    pub(super) reserved: bool,
    pending: Option<gpui::Task<()>>,
    reveal: Cell<Reveal>,
}

#[derive(Clone, Copy, Debug, Default)]
enum Reveal {
    #[default]
    Hidden,
    Fading(Instant),
    Visible,
}

impl AuthProviderDraft {
    pub(super) fn http_consent_ready(&self, raw: &str) -> bool {
        self.pages.active() == &Page::Address
            && self.http_warning.pending.is_none()
            && self.http_warning.reserved
            && self.fit.settled()
            && matches!(self.http_warning.reveal.get(), Reveal::Visible)
            && self.observed_address == raw
    }

    pub(super) fn warning_opacity(&self, time: MotionTime, window: &Window) -> f32 {
        let warning = &self.http_warning;
        let (reveal, opacity) = match (
            warning.reserved && self.pages.active() == &Page::Address,
            self.fit.settled(),
            time.reduced,
            warning.reveal.get(),
        ) {
            (false, _, _, _) => (Reveal::Hidden, 0.),
            // Once revealed, a remeasurement must not hide or restart the warning.
            (true, _, _, Reveal::Visible) | (true, true, true, _) => (Reveal::Visible, 1.),
            (true, _, false, Reveal::Fading(started)) => {
                let sample = Timing::new(Duration::from_millis(120))
                    .ease(Easing::EaseOut)
                    .sample(time.now.saturating_duration_since(started));
                if sample.finished {
                    (Reveal::Visible, 1.)
                } else {
                    (Reveal::Fading(started), sample.directed_progress)
                }
            }
            (true, false, _, reveal) => (reveal, 0.),
            (true, true, false, Reveal::Hidden) => (Reveal::Fading(time.now), 0.),
        };
        warning.reveal.set(reveal);
        if matches!(reveal, Reveal::Fading(_)) {
            window.request_animation_frame();
        }
        opacity
    }
}

impl Block {
    pub(super) fn provider_address_edited(
        &mut self,
        field: gpui::Entity<crate::TextInput>,
        cx: &mut Context<'_, Self>,
    ) {
        let value = field.read(cx).value(cx);
        let Some(draft) = self
            .provider_draft_mut()
            .filter(|draft| draft.address == field && draft.observed_address != value)
        else {
            return;
        };
        draft.observed_address = value;
        draft.accepted_http = None;
        draft.error = None;
        let id = draft.id;
        let timer = cx.background_executor().timer(ADDRESS_DEBOUNCE);
        // Replacing the handle cancels the previous edit; dropping the draft cancels it too.
        draft.http_warning.pending = Some(cx.spawn(async move |owner, cx| {
            timer.await;
            // A released owner has no drawer left to update.
            drop(owner.update(cx, |view, cx| {
                let Some(draft) = view.provider_draft().filter(|draft| draft.id == id) else {
                    return;
                };
                let raw = draft.address.read(cx).value(cx);
                if draft.observed_address != raw {
                    return;
                }
                let reserved = block_auth::ProviderAddress::parse(&raw)
                    .is_ok_and(|address| address.insecure && view.saved_at(&address.url).is_none());
                if let Some(draft) = view.provider_draft_mut() {
                    draft.http_warning.pending = None;
                    if draft.http_warning.reserved != reserved {
                        draft.http_warning.reserved = reserved;
                        draft.http_warning.reveal.set(Reveal::Hidden);
                        draft.fit.invalidate();
                    }
                    cx.notify();
                }
            }));
        }));
        cx.notify();
    }
}
