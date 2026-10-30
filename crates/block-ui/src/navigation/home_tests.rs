//! Home download recovery exercises the rendered action and the real installer.

use std::{
    io::{Read as _, Write as _},
    net::TcpListener,
    time::{Duration, Instant},
};

use block_launcher::{
    ProxyMode,
    catalog::{Catalog, InstallStatus},
};

#[gpui::test]
fn home_retries_a_failed_download_without_an_account_or_a_second_instance(
    cx: &mut gpui::TestAppContext,
) {
    let result = (|| -> anyhow::Result<()> {
        let proxy = TcpListener::bind("127.0.0.1:0")?;
        proxy.set_nonblocking(true)?;
        let proxy_url = format!("http://{}", proxy.local_addr()?);
        let server = std::thread::spawn(move || -> anyhow::Result<String> {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                match proxy.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_nonblocking(false)?;
                        stream.set_read_timeout(Some(Duration::from_secs(2)))?;
                        let mut bytes = [0; 4096];
                        let length = stream.read(&mut bytes)?;
                        stream.write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")?;
                        return Ok(String::from_utf8_lossy(&bytes[..length]).into_owned());
                    }
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => return Err(error.into()),
                }
            }
        });
        let root = tempfile::tempdir()?;
        let mut preferences = crate::appearance::Preferences::default();
        preferences.downloads.proxy = ProxyMode::Http { url: proxy_url };
        // This fixture rejects one connection; transport retry policy is tested separately.
        preferences.downloads.max_retries = 0;
        preferences.save(root.path())?;
        let catalog = Catalog::open(root.path())?;
        let (instance, profile) =
            catalog.create_instance("Download failed".into(), "1.20.6".into(), None, None)?;
        let mut target = instance.targets[0].clone();
        target.status = InstallStatus::Failed {
            message: "Initial download failure".into(),
        };
        catalog.set_target(&instance.id, &target)?;
        let snapshot = catalog.snapshot()?;
        let (engine, messages) = crate::engine::Engine::with_root(root.path().to_owned())?;
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        cx.simulate_resize(gpui::size(gpui::px(432.), gpui::px(768.)));
        view.update(cx, |view, cx| {
            view.engine = Some(engine);
            view.messages = Some(messages);
            view.snapshot = snapshot;
            view.selected_instance = Some(instance.id.clone());
            view.selected_profile = Some(profile.id.clone());
            view.router.navigate(crate::Tab::Home);
            cx.notify();
        });
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(500));
        cx.run_until_parked();
        let bounds = cx
            .debug_bounds("home-play")
            .ok_or_else(|| anyhow::anyhow!("Home primary action is missing"))?;
        let window_count = cx.windows().len();
        cx.simulate_click(bounds.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        if cx.windows().len() > window_count
            && let Some(window) = cx.windows().last()
        {
            *cx = gpui::VisualTestContext::from_window(*window, &cx.cx);
        }
        crate::test_support::ensure_live_window(cx);
        assert!(view.read_with(cx, |view, _| view.busy && view.accounts.is_empty()));
        let deadline = Instant::now() + Duration::from_secs(5);
        while view.read_with(cx, |view, _| view.busy) && Instant::now() < deadline {
            view.update(cx, crate::Block::poll);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!view.read_with(cx, |view, _| view.busy));
        assert!(
            view.read_with(cx, |view, _| view.router.tab() == crate::Tab::Home
                && view.form.is_none())
        );
        let request = server
            .join()
            .map_err(|_| anyhow::anyhow!("loopback proxy worker failed"))??;
        assert!(request.starts_with("CONNECT piston-meta.mojang.com:443 "));
        let state = catalog.snapshot()?;
        assert_eq!(state.instances.len(), 1);
        assert_eq!(state.profiles.len(), 1);
        let (saved_instance, saved_profile, saved_target) = state.selection(&profile.id)?;
        assert_eq!(saved_instance.id, instance.id);
        assert_eq!(saved_instance.directory, instance.directory);
        assert_eq!(saved_profile.id, profile.id);
        assert!(
            matches!(&saved_target.status, InstallStatus::Failed { message } if message != "Initial download failure")
        );
        assert_eq!(cx.opened_url(), None);
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

fn frame(cx: &mut gpui::VisualTestContext, millis: u64) {
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(millis));
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

fn configured_home(view: &gpui::Entity<crate::Block>, cx: &mut gpui::VisualTestContext) {
    view.update(cx, |view, cx| {
        let account = crate::test_support::test_account();
        assert!(account.is_ok(), "{account:?}");
        if let Ok(account) = account {
            view.selected_account = Some(crate::account_id(&account));
            view.accounts = vec![account];
        }
        view.snapshot = crate::test_support::ready_snapshot();
        view.selected_instance = Some("instance-a".into());
        view.selected_profile = Some("profile-a".into());
        view.router.navigate(crate::Tab::Home);
        view.preferences.reduce_motion = true;
        cx.notify();
    });
    frame(cx, 500);
}

#[gpui::test]
fn queued_launch_keeps_progress_and_primary_action_separate(cx: &mut gpui::TestAppContext) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    for width in [360., 432.] {
        cx.simulate_resize(gpui::size(gpui::px(width), gpui::px(720.)));
        configured_home(&view, cx);
        view.update(cx, |view, cx| {
            view.busy = true;
            view.operation_kind = crate::OperationKind::Install;
            view.status = "Downloading Minecraft resources".into();
            cx.notify();
        });
        frame(cx, 500);
        let progress = cx.debug_bounds("home-task");
        let action = crate::test_support::control_bounds(cx, "home-play");
        assert!(progress.is_some() && action.is_some());
        if let (Some(progress), Some(action)) = (progress, action) {
            assert!(
                action.top() - progress.bottom() >= gpui::px(12.),
                "progress and queued-launch action need a visible gap"
            );
            assert!(action.bottom() <= gpui::px(720.));
        }
    }
}

fn home_action_background(cx: &mut gpui::VisualTestContext) -> gpui::Rgba {
    let bounds = crate::test_support::control_bounds(cx, "home-play");
    assert!(bounds.is_some(), "Home primary action is present");
    let bounds = bounds.unwrap_or_default();
    let color = cx.update(|window, _| {
        let scale = window.scale_factor();
        window.painted_quads().into_iter().find_map(|quad| {
            ((quad.bounds.size.width.0 - f32::from(bounds.size.width) * scale).abs() < 0.5
                && (quad.bounds.size.height.0 - f32::from(bounds.size.height) * scale).abs() < 0.5)
                .then(|| quad.background.as_solid().map(gpui::Hsla::to_rgb))
                .flatten()
        })
    });
    assert!(color.is_some(), "primary action has a painted background");
    color.unwrap_or_default()
}

fn contrast_ratio(a: gpui::Rgba, b: gpui::Rgba) -> f32 {
    let luminance = |color: gpui::Rgba| {
        let linear = |channel: f32| {
            if channel <= 0.04045 {
                channel / 12.92
            } else {
                ((channel + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(color.r) + 0.7152 * linear(color.g) + 0.0722 * linear(color.b)
    };
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

#[gpui::test]
fn home_primary_action_keeps_readable_hover_and_pressed_colors_for_every_action(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    cx.simulate_resize(gpui::size(gpui::px(432.), gpui::px(768.)));
    configured_home(&view, cx);
    for (appearance, palette) in [
        (crate::Appearance::Light, crate::Palette::LIGHT),
        (crate::Appearance::Dark, crate::Palette::DARK),
    ] {
        for (action, status) in [
            ("create", None),
            ("launch", Some(InstallStatus::Ready)),
            (
                "retry",
                Some(InstallStatus::Failed {
                    message: "Interrupted".into(),
                }),
            ),
            ("download", Some(InstallStatus::Pending)),
        ] {
            cx.simulate_mouse_move(
                gpui::point(gpui::px(0.), gpui::px(0.)),
                None,
                gpui::Modifiers::none(),
            );
            view.update(cx, |view, cx| {
                view.preferences.appearance = appearance;
                if let Some(status) = status {
                    view.snapshot = crate::test_support::ready_snapshot();
                    view.snapshot.instances[0].targets[0].status = status;
                    view.selected_profile = Some("profile-a".into());
                    view.selected_instance = Some("instance-a".into());
                } else {
                    view.snapshot = Default::default();
                    view.selected_profile = None;
                    view.selected_instance = None;
                }
                cx.notify();
            });
            cx.run_until_parked();
            let bounds = crate::test_support::control_bounds(cx, "home-play").unwrap_or_default();
            let background = gpui::rgb(palette.background);
            let foreground = gpui::rgb(palette.on_primary);
            let position = gpui::point(bounds.left() + gpui::px(20.), bounds.center().y);
            for state in ["normal", "hover", "pressed"] {
                if state == "hover" {
                    cx.simulate_mouse_move(position, None, gpui::Modifiers::none());
                } else if state == "pressed" {
                    cx.simulate_mouse_down(
                        position,
                        gpui::MouseButton::Left,
                        gpui::Modifiers::none(),
                    );
                }
                cx.run_until_parked();
                let painted = background.blend(home_action_background(cx));
                let contrast = contrast_ratio(foreground, painted);
                assert!(
                    contrast >= 4.5,
                    "{appearance:?} {action} {state}: {contrast:.2}:1 against {painted:?}"
                );
            }
            // Release outside the action to inspect pressed styling without
            // creating a game, retrying downloads or launching the fixture.
            let outside = gpui::point(gpui::px(0.), gpui::px(0.));
            cx.simulate_mouse_move(outside, gpui::MouseButton::Left, gpui::Modifiers::none());
            cx.simulate_mouse_up(outside, gpui::MouseButton::Left, gpui::Modifiers::none());
        }
    }
}

#[gpui::test]
fn task_phase_and_progress_follow_real_events_without_replaying_local_entry(
    cx: &mut gpui::TestAppContext,
) {
    use std::sync::{Arc, Mutex};
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    cx.simulate_resize(gpui::size(gpui::px(360.), gpui::px(720.)));
    configured_home(&view, cx);
    let tracker = Arc::new(Mutex::new(crate::tasks::DownloadTracker::default()));
    tracker
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .apply(block_launcher::DownloadEvent::Started {
            sha1: "test-resource".into(),
            size: 200,
        });
    view.update(cx, |view, cx| {
        view.preferences.reduce_motion = false;
        view.busy = true;
        view.operation_cancellable = true;
        view.operation_kind = crate::OperationKind::Launch;
        view.status = "Preparing · validating profile and account".into();
        cx.notify();
    });
    frame(cx, 70);

    view.update(cx, |view, cx| {
        view.busy = false;
        cx.notify();
    });
    frame(cx, 500);
    view.update(cx, |view, cx| {
        view.busy = true;
        view.downloads = Some(tracker.clone());
        view.status = "Downloading the current resource".into();
        cx.notify();
    });
    frame(cx, 70);

    let first_height = cx
        .debug_bounds("home-task-motion")
        .map(|bounds| bounds.size.height);
    view.update(cx, |view, cx| {
        view.status = "Verifying the current resource".into();
        cx.notify();
    });
    frame(cx, 100);
    let later_height = cx
        .debug_bounds("home-task-motion")
        .map(|bounds| bounds.size.height);
    assert!(
        first_height
            .zip(later_height)
            .is_some_and(|(first, later)| later >= first),
        "status/resource refresh must not restart the panel entry: first={first_height:?}, later={later_height:?}"
    );
    tracker
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .apply(block_launcher::DownloadEvent::Bytes {
            sha1: "test-resource".into(),
            bytes: 75,
        });
    view.update(cx, |_, cx| cx.notify());
    frame(cx, 500);

    observed_running_stops_and_exits_without_interaction(&view, cx);
}

fn observed_running_stops_and_exits_without_interaction(
    view: &gpui::Entity<crate::Block>,
    cx: &mut gpui::VisualTestContext,
) {
    view.update(cx, |view, cx| {
        view.downloads = None;
        view.active_run = Some("observed-run".into());
        view.session_revisions.insert(
            "observed-run".into(),
            (1, block_launcher::session_host::RunState::Running),
        );
        cx.notify();
    });
    frame(cx, 10);

    let stop = cx.debug_bounds("home-stop");
    assert!(stop.is_some());
    if let Some(stop) = stop {
        cx.simulate_click(stop.center(), gpui::Modifiers::none());
    }
    frame(cx, 10);
    assert!(view.read_with(cx, |view, _| view.play_stop.is_cancelled()));

    view.update(cx, |view, cx| {
        view.busy = false;
        view.active_run = None;
        cx.notify();
    });
    frame(cx, 10);

    frame(cx, 500);
    assert_eq!(
        cx.debug_bounds("home-task-motion")
            .map(|bounds| bounds.size.height),
        Some(gpui::px(0.))
    );
}

#[gpui::test]
fn reduced_motion_and_non_cancellable_work_do_not_offer_a_false_stop(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    cx.simulate_resize(gpui::size(gpui::px(360.), gpui::px(720.)));
    configured_home(&view, cx);
    view.update(cx, |view, cx| {
        view.busy = true;
        view.operation_cancellable = false;
        view.operation_kind = crate::OperationKind::General;
        view.status = "Saving metadata".into();
        cx.notify();
    });
    frame(cx, 0);

    assert!(
        cx.debug_bounds("home-task-motion")
            .is_some_and(|bounds| bounds.size.height > gpui::px(80.))
    );

    view.update(cx, |view, cx| {
        view.busy = false;
        cx.notify();
    });
    frame(cx, 0);
    assert_eq!(
        cx.debug_bounds("home-task-motion")
            .map(|bounds| bounds.size.height),
        Some(gpui::px(0.))
    );
}
