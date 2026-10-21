//! Shared transient feedback. Important form errors also remain on their form.
#![expect(
    clippy::redundant_pub_crate,
    reason = "Feedback is private to the UI implementation."
)]
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug)]
pub(super) enum ToastKind {
    Success,
    Error,
    Info,
}

#[derive(Clone, Debug)]
pub(super) struct Toast {
    pub message: String,
    pub kind: ToastKind,
    pub expires: Instant,
}

impl Toast {
    pub(super) fn new(message: String, kind: ToastKind) -> Self {
        Self {
            message,
            kind,
            expires: Instant::now()
                + Duration::from_secs(if matches!(kind, ToastKind::Error) {
                    8
                } else {
                    4
                }),
        }
    }
}
