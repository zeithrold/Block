//! Small, safe platform queries missing from the pinned GPUI release.
//!
//! Read the system value at each transition/render decision. The pinned GPUI
//! platform API does not expose the `AppKit` accessibility-display notification.
//! This is a live query: active animation responds on its next frame, and an
//! idle window applies the new policy before its next interaction is rendered.
#![expect(
    clippy::redundant_pub_crate,
    reason = "These platform queries are used only by the parent UI."
)]

/// Either preference can request reduced motion; the app cannot override the OS.
pub(super) fn reduce_motion(application_preference: bool) -> bool {
    application_preference || system_reduce_motion()
}

#[cfg(target_os = "macos")]
pub(super) fn system_reduce_motion() -> bool {
    objc2_app_kit::NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
}

// These platforms retain the application's existing preference. Their system
// integration needs platform-specific implementation and CI validation.
#[cfg(not(target_os = "macos"))]
pub(super) const fn system_reduce_motion() -> bool {
    false
}
