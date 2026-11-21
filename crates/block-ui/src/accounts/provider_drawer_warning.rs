use super::*;
use gpui_base::motion::{Easing, Timing};
use std::{
    cell::Cell,
    rc::Rc,
    time::{Duration, Instant},
};

#[derive(Debug, Default)]
pub(super) struct HttpWarning {
    reserved: Cell<bool>,
    reveal: Cell<Reveal>,
    pub(super) painted_control: Rc<Cell<bool>>,
}

pub(super) enum Destination<'a> {
    Saved(&'a block_auth::AuthlibProvider),
    Https(String),
    Http { url: String, approved: bool },
}

#[derive(Clone, Copy, Debug, Default)]
enum Reveal {
    #[default]
    Hidden,
    Fading(Instant),
    Visible,
}

impl AuthProviderDraft {
    pub(super) fn warning_revealed(&self) -> bool {
        matches!(self.http_warning.reveal.get(), Reveal::Visible)
    }

    pub(super) fn warning_opacity(&self, http: bool, time: MotionTime, window: &Window) -> f32 {
        let warning = &self.http_warning;
        warning.painted_control.set(false);
        let present = http && self.pages.active() == &Page::Address;
        if warning.reserved.replace(present) != present {
            warning.reveal.set(Reveal::Hidden);
            self.fit.invalidate();
        }
        let (reveal, opacity) = match (
            present,
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
    pub(super) fn provider_destination(
        &self,
        draft: &AuthProviderDraft,
        raw: &str,
    ) -> block_auth::Result<Destination<'_>> {
        let address = block_auth::ProviderAddress::parse(raw)?;
        Ok(if let Some(saved) = self.saved_at(&address.url) {
            Destination::Saved(saved)
        } else if address.insecure {
            Destination::Http {
                approved: draft.observed_address == raw
                    && draft.accepted_http.as_deref() == Some(&address.url),
                url: address.url,
            }
        } else {
            Destination::Https(address.url)
        })
    }

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
        draft.http_warning.painted_control.set(false);
        cx.notify();
    }
}
