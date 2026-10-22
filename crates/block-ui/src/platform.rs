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

/// GTK/GNOME exposes the system animation preference through gsettings.
#[cfg(target_os = "linux")]
pub(super) fn system_reduce_motion() -> bool {
    let output = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "enable-animations"])
        .output();
    output
        .ok()
        .filter(|output| output.status.success())
        .is_some_and(|output| animations_disabled(&String::from_utf8_lossy(&output.stdout)))
}

#[cfg(target_os = "linux")]
fn animations_disabled(value: &str) -> bool {
    value.trim().eq_ignore_ascii_case("false")
}

/// Windows system motion preference still needs a platform query; the in-app
/// preference remains authoritative until that lands.
#[cfg(target_os = "windows")]
pub(super) const fn system_reduce_motion() -> bool {
    false
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "linux")]
    #[test]
    fn gsettings_animation_values_are_interpreted_strictly() {
        assert!(super::animations_disabled("false\n"));
        assert!(!super::animations_disabled("true\n"));
        assert!(!super::animations_disabled(""));
    }
}
