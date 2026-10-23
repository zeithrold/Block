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
    #[cfg(test)]
    if let Some(system_preference) = TEST_SYSTEM_MOTION.get() {
        return application_preference || system_preference;
    }
    application_preference || system_reduce_motion()
}

// Motion tests must exercise both policies even on a runner whose OS already
// requests reduced motion. Scope the input to the test thread, without changing
// the desktop preference or leaking a policy to other tests.
#[cfg(test)]
thread_local! {
    static TEST_SYSTEM_MOTION: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
pub(super) struct SystemMotionOverride(Option<bool>);

#[cfg(test)]
impl Drop for SystemMotionOverride {
    fn drop(&mut self) {
        TEST_SYSTEM_MOTION.set(self.0);
    }
}

#[cfg(test)]
#[must_use]
pub(super) fn test_system_motion(reduced: bool) -> SystemMotionOverride {
    SystemMotionOverride(TEST_SYSTEM_MOTION.replace(Some(reduced)))
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
#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
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
