//! Home uses display-relative bounds so a portrait window also fits short screens.

use gpui::{App, Bounds, Pixels, Size, Window, px, size};

/// The same inner bounds used by the component library's window border. Tiled
/// Linux edges have no shadow or border; overlays must not cover resize chrome.
#[expect(
    clippy::redundant_pub_crate,
    reason = "Shared by product overlay hosts."
)]
pub(super) fn content_insets(window: &Window) -> gpui::Edges<Pixels> {
    let mut insets = gpui_component::window_paddings(window);
    if let gpui::Decorations::Client { tiling } = window.window_decorations() {
        if !tiling.top {
            insets.top += px(1.);
        }
        if !tiling.bottom {
            insets.bottom += px(1.);
        }
        if !tiling.left {
            insets.left += px(1.);
        }
        if !tiling.right {
            insets.right += px(1.);
        }
    }
    insets
}

/// GPUI exposes full display bounds, not the desktop work area. Reserve logical
/// pixels for panels, a system titlebar and resize borders without changing WM
/// settings. A window deliberately placed outside this area cannot be moved with
/// GPUI's public API; retain the application's minimum usable size in that case.
#[expect(
    clippy::redundant_pub_crate,
    reason = "Only the parent UI module uses this helper."
)]
pub(super) fn home_size(window: &Window, cx: &App) -> Size<Pixels> {
    window.display(cx).map_or_else(preferred_size, |display| {
        fit_home(display.bounds(), window.bounds())
    })
}

#[expect(
    clippy::redundant_pub_crate,
    reason = "Only the parent UI module uses this helper."
)]
pub(super) fn management_size(window: &Window, cx: &App) -> Size<Pixels> {
    let preferred = size(px(960.), px(600.));
    window.display(cx).map_or(preferred, |display| {
        fit(display.bounds(), window.bounds(), preferred)
    })
}

const fn preferred_size() -> Size<Pixels> {
    size(px(432.), px(768.))
}

fn fit_home(display: Bounds<Pixels>, window: Bounds<Pixels>) -> Size<Pixels> {
    fit(display, window, preferred_size())
}

fn fit(display: Bounds<Pixels>, window: Bounds<Pixels>, preferred: Size<Pixels>) -> Size<Pixels> {
    let left = window.origin.x.max(display.origin.x + px(16.));
    let top = window.origin.y.max(display.origin.y + px(32.));
    let available = size(
        display.right() - left - px(16.),
        display.bottom() - top - px(64.),
    );
    size(
        available.width.clamp(px(360.), preferred.width),
        available.height.clamp(px(540.), preferred.height),
    )
}

#[cfg(test)]
mod tests {
    use gpui::{Bounds, point};

    use super::{fit_home, preferred_size};

    fn bounds(x: f32, y: f32, width: f32, height: f32) -> Bounds<gpui::Pixels> {
        Bounds::new(
            point(gpui::px(x), gpui::px(y)),
            gpui::size(gpui::px(width), gpui::px(height)),
        )
    }

    #[test]
    fn xfce_short_screen_leaves_room_for_frame_and_panels() {
        let result = fit_home(bounds(0., 0., 1280., 800.), bounds(200., 27., 960., 600.));
        assert_eq!(result, gpui::size(gpui::px(432.), gpui::px(704.)));
        assert!(gpui::px(27.) + result.height + gpui::px(34.) <= gpui::px(800. - 32.));
    }

    #[test]
    fn large_display_preserves_home_size() {
        assert_eq!(
            fit_home(bounds(0., 0., 1920., 1080.), bounds(480., 240., 960., 600.)),
            preferred_size()
        );
    }

    #[test]
    fn negative_monitor_origins_do_not_change_available_space() {
        let expected = fit_home(bounds(0., 0., 1280., 800.), bounds(900., 100., 360., 600.));
        assert_eq!(
            fit_home(
                bounds(-1280., -800., 1280., 800.),
                bounds(-380., -700., 360., 600.)
            ),
            expected
        );
        assert_eq!(expected, gpui::size(gpui::px(364.), gpui::px(636.)));
    }

    #[test]
    fn tiny_display_or_offscreen_origin_retains_minimum_usable_size() {
        for (display, window) in [
            (bounds(0., 0., 320., 480.), bounds(0., 0., 320., 480.)),
            (bounds(0., 0., 1280., 800.), bounds(1270., 790., 960., 600.)),
        ] {
            assert_eq!(
                fit_home(display, window),
                gpui::size(gpui::px(360.), gpui::px(540.))
            );
        }
    }

    #[gpui::test]
    fn compact_home_keeps_primary_action_and_profile_switcher_visible(
        cx: &mut gpui::TestAppContext,
    ) {
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        let fitted = fit_home(bounds(0., 0., 1280., 800.), bounds(200., 100., 960., 600.));
        view.update(cx, |view, cx| {
            let fixture = crate::test_support::test_account();
            assert!(fixture.is_ok(), "{fixture:?}");
            let Ok(account) = fixture else {
                return;
            };
            view.selected_account = Some(crate::account_id(&account));
            view.accounts = vec![account];
            view.snapshot = crate::test_support::ready_snapshot();
            view.normalize_selection();
            view.tab = crate::Tab::Home;
            view.preferences.reduce_motion = true;
            cx.notify();
        });
        for dimensions in [fitted, gpui::size(gpui::px(360.), gpui::px(540.))] {
            cx.simulate_resize(dimensions);
            cx.run_until_parked();
            for selector in ["home-play", "home-profiles", "app-menu"] {
                let action = cx.debug_bounds(selector);
                assert!(action.is_some(), "missing {selector}");
                if let Some(action) = action {
                    assert!(
                        action.origin.x >= gpui::px(0.)
                            && action.origin.y >= gpui::px(0.)
                            && action.right() <= dimensions.width
                            && action.bottom() <= dimensions.height,
                        "{selector} is outside the fitted Home: {action:?}, {dimensions:?}"
                    );
                }
            }
        }
    }

    #[gpui::test]
    fn navigation_and_completed_onboarding_fit_home_at_current_window_position(
        cx: &mut gpui::TestAppContext,
    ) {
        use gpui::AppContext as _;

        let result = (|| -> anyhow::Result<()> {
            // TestDisplay is 1920×1080; this lower window position leaves only
            // 616 content pixels below it once decoration/panel space is kept.
            let handle = cx.update(|cx| {
                cx.open_window(
                    gpui::WindowOptions {
                        window_bounds: Some(gpui::WindowBounds::Windowed(bounds(
                            200., 400., 960., 600.,
                        ))),
                        ..Default::default()
                    },
                    |window, cx| cx.new(|cx| crate::tests::test_view(window, cx)),
                )
            })?;
            let view = handle.root(cx)?;
            let cx = gpui::VisualTestContext::from_window(*handle, cx).into_mut();
            cx.update(|window, cx| {
                view.update(cx, |view, cx| {
                    view.handle(crate::UiAction::Tab(crate::Tab::Home), window, cx);
                });
                assert_eq!(
                    window.bounds().size,
                    gpui::size(gpui::px(432.), gpui::px(616.))
                );
                window.resize(gpui::size(gpui::px(960.), gpui::px(600.)));
            });
            view.update(cx, |view, cx| {
                view.pending_window_size = Some(preferred_size());
                cx.notify();
            });
            cx.run_until_parked();
            cx.update(|window, _| {
                assert_eq!(
                    window.bounds().size,
                    gpui::size(gpui::px(432.), gpui::px(616.))
                );
            });
            assert!(view.read_with(cx, |view, _| view.pending_window_size.is_none()));
            Ok(())
        })();
        assert!(result.is_ok(), "{result:?}");
    }
}
