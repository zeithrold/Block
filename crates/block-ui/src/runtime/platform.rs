//! Small, safe platform queries missing from the pinned GPUI release.
//!
//! macOS and Windows query their system APIs directly. Linux refreshes desktop
//! settings off the render thread at most once per second; active animations
//! apply a refreshed value on a subsequent frame, while an idle window applies
//! it on its next interaction. GPUI does not expose AppKit's preference-change
//! notification.
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

/// Linux desktop settings are read off the render thread and cached.
#[cfg(target_os = "linux")]
pub(super) fn system_reduce_motion() -> bool {
    linux_motion_preference::cached()
}

#[cfg(any(target_os = "linux", test))]
mod linux_motion_preference {
    #[cfg(target_os = "linux")]
    use std::{
        process::{Command, Stdio},
        sync::OnceLock,
        thread,
    };
    use std::{
        sync::Mutex,
        time::{Duration, Instant},
    };

    const REFRESH_INTERVAL: Duration = Duration::from_secs(1);
    const FAILURE_RETRY: Duration = Duration::from_secs(5);
    #[cfg(target_os = "linux")]
    const COMMAND_TIMEOUT: Duration = Duration::from_millis(750);

    #[derive(Default)]
    struct State {
        reduced: bool,
        refreshing: bool,
        last_refresh: Option<(Instant, Duration)>,
    }

    #[derive(Default)]
    struct Cache(Mutex<State>);

    impl Cache {
        fn claim_refresh(&self, now: Instant) -> bool {
            let mut state = self
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let due = state
                .last_refresh
                .is_none_or(|(last, interval)| now.saturating_duration_since(last) >= interval);
            if state.refreshing || !due {
                return false;
            }
            state.refreshing = true;
            true
        }

        #[cfg(target_os = "linux")]
        fn read(&self, now: Instant) -> bool {
            let refresh = self.claim_refresh(now);
            if refresh {
                let cache = CACHE.get_or_init(Cache::default);
                let spawned = thread::Builder::new()
                    .name("block-motion-preference".into())
                    .spawn(move || {
                        let result = query_system_preference();
                        cache.finish(result, Instant::now());
                    });
                if spawned.is_err() {
                    self.finish(Err(()), Instant::now());
                }
            }
            self.0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .reduced
        }

        fn finish(&self, result: Result<bool, ()>, now: Instant) {
            let mut state = self
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let interval = match result {
                Ok(reduced) => {
                    state.reduced = reduced;
                    REFRESH_INTERVAL
                }
                Err(()) => FAILURE_RETRY,
            };
            state.last_refresh = Some((now, interval));
            state.refreshing = false;
        }

        #[cfg(test)]
        fn value(&self) -> bool {
            self.0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .reduced
        }
    }

    #[cfg(target_os = "linux")]
    static CACHE: OnceLock<Cache> = OnceLock::new();

    #[cfg(target_os = "linux")]
    pub(super) fn cached() -> bool {
        CACHE.get_or_init(Cache::default).read(Instant::now())
    }

    #[cfg(target_os = "linux")]
    fn query_system_preference() -> Result<bool, ()> {
        // The standardized portal setting works across desktop environments.
        if let Some(value) = run_query(
            "gdbus",
            &[
                "call",
                "--session",
                "--dest",
                "org.freedesktop.portal.Desktop",
                "--object-path",
                "/org/freedesktop/portal/desktop",
                "--method",
                "org.freedesktop.portal.Settings.ReadOne",
                "org.freedesktop.appearance",
                "reduced-motion",
            ],
            portal_motion_preference,
        ) {
            return Ok(value);
        }

        if current_desktop_is_kde() {
            for program in ["kreadconfig6", "kreadconfig5"] {
                if let Some(value) = run_query(
                    program,
                    &[
                        "--file",
                        "kdeglobals",
                        "--group",
                        "KDE",
                        "--key",
                        "AnimationDurationFactor",
                        "--default",
                        "1.0",
                    ],
                    kde_animation_disabled,
                ) {
                    return Ok(value);
                }
            }
        }

        if let Some(value) = run_query(
            "gsettings",
            &["get", "org.gnome.desktop.a11y.interface", "reduced-motion"],
            gnome_reduced_motion,
        ) {
            return Ok(value);
        }

        run_query(
            "gsettings",
            &["get", "org.gnome.desktop.interface", "enable-animations"],
            gnome_animations_disabled,
        )
        .ok_or(())
    }

    #[cfg(target_os = "linux")]
    fn run_query(
        program: &str,
        args: &[&str],
        parse: impl FnOnce(&str) -> Option<bool>,
    ) -> Option<bool> {
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let started = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    if !status.success() {
                        return None;
                    }
                    let mut output = Vec::new();
                    use std::io::Read as _;
                    child
                        .stdout
                        .take()?
                        .take(4096)
                        .read_to_end(&mut output)
                        .ok()?;
                    return parse(std::str::from_utf8(&output).ok()?);
                }
                Ok(None) if started.elapsed() < COMMAND_TIMEOUT => {
                    thread::sleep(Duration::from_millis(10));
                }
                Ok(None) | Err(_) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
            }
        }
    }

    fn portal_motion_preference(output: &str) -> Option<bool> {
        let value = output.split_once("uint32")?.1.trim_start();
        let number = value
            .split(|character: char| !character.is_ascii_digit())
            .next()?
            .parse::<u32>()
            .ok()?;
        match number {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        }
    }

    #[cfg(target_os = "linux")]
    fn current_desktop_is_kde() -> bool {
        std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|desktops| {
            desktops
                .split(':')
                .any(|desktop| desktop.eq_ignore_ascii_case("KDE"))
        })
    }

    fn kde_animation_disabled(value: &str) -> Option<bool> {
        let factor = value.trim().parse::<f64>().ok()?;
        factor.is_finite().then_some(factor == 0.)
    }

    fn gnome_animations_disabled(value: &str) -> Option<bool> {
        match value.trim() {
            "false" => Some(true),
            "true" => Some(false),
            _ => None,
        }
    }

    fn gnome_reduced_motion(value: &str) -> Option<bool> {
        match value.trim().trim_matches('\'') {
            "reduce" => Some(true),
            "no-preference" => Some(false),
            _ => None,
        }
    }

    #[cfg(test)]
    mod tests {
        use super::{
            Cache, gnome_animations_disabled, gnome_reduced_motion, kde_animation_disabled,
            portal_motion_preference,
        };
        use std::{sync::Arc, thread, time::Instant};

        #[test]
        fn concurrent_reads_claim_only_one_refresh() {
            let cache = Arc::new(Cache::default());
            let now = Instant::now();
            let readers = (0..16)
                .map(|_| {
                    let cache = cache.clone();
                    thread::spawn(move || cache.claim_refresh(now))
                })
                .collect::<Vec<_>>();
            let claims = readers
                .into_iter()
                .map(|reader| reader.join().ok())
                .collect::<Vec<_>>();
            assert!(claims.iter().all(Option::is_some));
            assert_eq!(
                claims.into_iter().flatten().filter(|claim| *claim).count(),
                1
            );
        }

        #[test]
        fn refresh_updates_value_and_failure_keeps_last_known_value() {
            let cache = Cache::default();
            let now = Instant::now();
            cache.finish(Ok(true), now);
            assert!(cache.value());
            assert!(!cache.claim_refresh(now + std::time::Duration::from_millis(999)));
            assert!(cache.claim_refresh(now + std::time::Duration::from_secs(1)));
            let failed_at = now + std::time::Duration::from_secs(1);
            cache.finish(Err(()), failed_at);
            assert!(cache.value());
            assert!(!cache.claim_refresh(failed_at + std::time::Duration::from_secs(4)));
            let retry_at = failed_at + std::time::Duration::from_secs(5);
            assert!(cache.claim_refresh(retry_at));
            cache.finish(Ok(false), retry_at);
            assert!(!cache.value());
        }

        #[test]
        fn portal_and_gnome_values_are_parsed_strictly() {
            assert_eq!(portal_motion_preference("(<uint32 1>,)"), Some(true));
            assert_eq!(portal_motion_preference("(<uint32 0>,)"), Some(false));
            assert_eq!(portal_motion_preference("(<uint32 10>,)"), None);
            assert_eq!(portal_motion_preference("(<'1'>,)"), None);
            assert_eq!(gnome_animations_disabled("false\n"), Some(true));
            assert_eq!(gnome_animations_disabled("true\n"), Some(false));
            assert_eq!(gnome_animations_disabled("unexpected"), None);
            assert_eq!(gnome_reduced_motion("'reduce'\n"), Some(true));
            assert_eq!(gnome_reduced_motion("'no-preference'\n"), Some(false));
            assert_eq!(gnome_reduced_motion("unexpected"), None);
            assert_eq!(gnome_reduced_motion("'full'\n"), None);
            assert_eq!(kde_animation_disabled("0\n"), Some(true));
            assert_eq!(kde_animation_disabled("1.0\n"), Some(false));
            assert_eq!(kde_animation_disabled("invalid"), None);
        }
    }
}
#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub(super) const fn system_reduce_motion() -> bool {
    false
}
