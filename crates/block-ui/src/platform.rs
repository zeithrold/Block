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

/// Windows reads the "Show animations in Windows" accessibility setting.
#[cfg(target_os = "windows")]
pub(super) fn system_reduce_motion() -> bool {
    block_drop::system_reduce_motion()
}

// This platform retains the application's existing preference. Its system
// integration needs platform-specific implementation and CI validation.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub(super) const fn system_reduce_motion() -> bool {
    false
}
