//! Product window sizes and their display-relative fitting rules.

use gpui::{App, Bounds, Edges, Pixels, Size, Window, px, size};

const MIN_WINDOW_WIDTH: Pixels = px(360.);
const MIN_WINDOW_HEIGHT: Pixels = px(540.);
const MANAGEMENT_HEIGHT: Pixels = px(720.);

/// The same inner bounds used by the component library's window border. Tiled
/// Linux edges have no shadow or border; overlays must not cover resize chrome.
#[expect(
    clippy::redundant_pub_crate,
    reason = "Shared by product overlay hosts."
)]
pub(super) fn content_insets(window: &Window) -> Edges<Pixels> {
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

/// Visible product space after accounting for the component window frame.
#[expect(
    clippy::redundant_pub_crate,
    reason = "Shared by product page layouts."
)]
pub(super) fn content_size(window: &Window) -> Size<Pixels> {
    subtract_insets(window.viewport_size(), content_insets(window))
}

/// Home starts at its final left-pane width while preserving the same content
/// height used by Manage.
#[expect(
    clippy::redundant_pub_crate,
    reason = "Only the parent UI module uses this helper."
)]
pub(super) fn home_size(window: &Window, cx: &App) -> Size<Pixels> {
    fitted_size(
        window.display(cx).map(|display| display.bounds()),
        window.bounds(),
        preferred_home_content_size(),
        content_insets(window),
    )
}

#[expect(
    clippy::redundant_pub_crate,
    reason = "Only the parent UI module uses this helper."
)]
pub(super) fn management_size(window: &Window, cx: &App) -> Size<Pixels> {
    fitted_size(
        window.display(cx).map(|display| display.bounds()),
        window.bounds(),
        preferred_management_size(),
        content_insets(window),
    )
}

/// Preferred Manage content dimensions used to center the application window.
pub fn preferred_management_size() -> Size<Pixels> {
    size(px(1152.), MANAGEMENT_HEIGHT)
}

/// Reserve the expanded frame's position before the compact Home is shown.
/// Centering an oversized preferred frame first can place Home off-screen.
pub fn initial_window_bounds(cx: &App) -> Bounds<Pixels> {
    initial_bounds(cx.primary_display().map(|display| display.bounds()))
}

fn initial_bounds(display: Option<Bounds<Pixels>>) -> Bounds<Pixels> {
    let preferred = preferred_management_size();
    let Some(display) = display else {
        return Bounds::new(gpui::point(px(16.), px(32.)), preferred);
    };
    let available = size(
        (display.size.width - px(32.)).max(MIN_WINDOW_WIDTH),
        (display.size.height - px(96.)).max(MIN_WINDOW_HEIGHT),
    );
    let fitted = size(
        preferred.width.min(available.width),
        preferred.height.min(available.height),
    );
    Bounds::new(
        gpui::point(
            display.origin.x + (display.size.width - fitted.width) / 2.,
            display.origin.y + ((display.size.height - fitted.height) / 2. - px(16.)).max(px(0.)),
        ),
        fitted,
    )
}

/// Width-only navigation targets. The current viewport height is retained so
/// callers can morph Home into a sidebar at a fixed top-left corner.
#[expect(
    clippy::redundant_pub_crate,
    reason = "Only the parent UI module uses this helper."
)]
pub(super) fn navigation_sizes(window: &Window, cx: &App) -> (Size<Pixels>, Size<Pixels>) {
    let insets = content_insets(window);
    navigation_sizes_for(
        window.display(cx).map(|display| display.bounds()),
        window.bounds(),
        window.viewport_size().height,
        insets,
    )
}

const fn preferred_home_content_size() -> Size<Pixels> {
    size(px(super::design::SIDEBAR_WIDTH), MANAGEMENT_HEIGHT)
}

fn fitted_size(
    display: Option<Bounds<Pixels>>,
    window: Bounds<Pixels>,
    preferred_content: Size<Pixels>,
    insets: Edges<Pixels>,
) -> Size<Pixels> {
    let preferred_outer = add_insets(preferred_content, insets);
    let Some(display) = display else {
        return preferred_outer;
    };
    let (available_width, available_height) = available_size(display, window);
    size(
        fit_dimension(preferred_outer.width, available_width, MIN_WINDOW_WIDTH),
        fit_dimension(preferred_outer.height, available_height, MIN_WINDOW_HEIGHT),
    )
}

fn navigation_sizes_for(
    display: Option<Bounds<Pixels>>,
    window: Bounds<Pixels>,
    viewport_height: Pixels,
    insets: Edges<Pixels>,
) -> (Size<Pixels>, Size<Pixels>) {
    let preferred_home = add_insets(preferred_home_content_size(), insets);
    let preferred_management = add_insets(preferred_management_size(), insets);
    let available_width = display.map_or(preferred_management.width, |display| {
        available_size(display, window).0
    });
    let available_width = available_width.max(MIN_WINDOW_WIDTH);
    let home_width = preferred_home
        .width
        .min(available_width)
        .max(MIN_WINDOW_WIDTH);
    let management_width = preferred_management
        .width
        .min(available_width)
        .max(home_width);
    (
        size(home_width, viewport_height),
        size(management_width, viewport_height),
    )
}

fn available_size(display: Bounds<Pixels>, window: Bounds<Pixels>) -> (Pixels, Pixels) {
    let left = window.origin.x.max(display.origin.x + px(16.));
    let top = window.origin.y.max(display.origin.y + px(32.));
    (
        display.right() - left - px(16.),
        display.bottom() - top - px(64.),
    )
}

fn fit_dimension(preferred: Pixels, available: Pixels, minimum: Pixels) -> Pixels {
    available.max(minimum).min(preferred.max(minimum))
}

fn add_insets(size: Size<Pixels>, insets: Edges<Pixels>) -> Size<Pixels> {
    gpui::size(
        size.width + insets.left + insets.right,
        size.height + insets.top + insets.bottom,
    )
}

fn subtract_insets(size: Size<Pixels>, insets: Edges<Pixels>) -> Size<Pixels> {
    gpui::size(
        (size.width - insets.left - insets.right).max(px(0.)),
        (size.height - insets.top - insets.bottom).max(px(0.)),
    )
}

#[cfg(test)]
mod tests {
    use gpui::{Bounds, Edges, point};

    use super::{
        MIN_WINDOW_HEIGHT, MIN_WINDOW_WIDTH, add_insets, fit_dimension, fitted_size,
        navigation_sizes_for, preferred_home_content_size, preferred_management_size,
    };

    fn bounds(x: f32, y: f32, width: f32, height: f32) -> Bounds<gpui::Pixels> {
        Bounds::new(
            point(gpui::px(x), gpui::px(y)),
            gpui::size(gpui::px(width), gpui::px(height)),
        )
    }

    #[test]
    fn fitted_home_and_management_share_visible_height_and_sidebar_width() {
        let display = bounds(0., 0., 1920., 1080.);
        let window = bounds(240., 120., 1152., 720.);
        let insets = Edges {
            top: gpui::px(2.),
            right: gpui::px(3.),
            bottom: gpui::px(4.),
            left: gpui::px(5.),
        };
        let home = fitted_size(Some(display), window, preferred_home_content_size(), insets);
        let management = fitted_size(Some(display), window, preferred_management_size(), insets);
        assert_eq!(home.height, management.height);
        assert_eq!(
            home.width - insets.left - insets.right,
            gpui::px(crate::design::SIDEBAR_WIDTH)
        );
        assert_eq!(
            management.width - insets.left - insets.right,
            gpui::px(1152.)
        );
        assert_eq!(
            preferred_home_content_size().height,
            preferred_management_size().height
        );
    }

    #[test]
    fn width_only_navigation_preserves_height_and_targets_content_widths() {
        let display = bounds(0., 0., 1920., 1080.);
        let window = bounds(200., 100., 360., 611.);
        let insets = Edges {
            top: gpui::px(1.),
            right: gpui::px(2.),
            bottom: gpui::px(3.),
            left: gpui::px(4.),
        };
        let (home, management) =
            navigation_sizes_for(Some(display), window, gpui::px(611.), insets);
        assert_eq!(home.height, gpui::px(611.));
        assert_eq!(management.height, gpui::px(611.));
        assert_eq!(home.width - insets.left - insets.right, gpui::px(360.));
        assert_eq!(
            management.width - insets.left - insets.right,
            gpui::px(1152.)
        );
        assert!(management.width >= home.width);
    }

    #[test]
    fn short_screen_and_current_origin_keep_existing_reserves_and_minimums() {
        let result = fitted_size(
            Some(bounds(0., 0., 1280., 800.)),
            bounds(200., 27., 960., 600.),
            preferred_home_content_size(),
            Edges::default(),
        );
        assert_eq!(result, gpui::size(gpui::px(360.), gpui::px(704.)));
        assert!(gpui::px(32.) + result.height + gpui::px(64.) <= gpui::px(800.));
        assert_eq!(
            fitted_size(
                Some(bounds(0., 0., 320., 480.)),
                bounds(0., 0., 320., 480.),
                preferred_home_content_size(),
                Edges::default(),
            ),
            gpui::size(MIN_WINDOW_WIDTH, MIN_WINDOW_HEIGHT)
        );
        assert_eq!(
            fit_dimension(gpui::px(1152.), gpui::px(900.), MIN_WINDOW_WIDTH),
            gpui::px(900.)
        );
    }

    #[test]
    fn negative_monitor_origins_do_not_change_available_space() {
        let display = bounds(0., 0., 1280., 800.);
        let expected = fitted_size(
            Some(display),
            bounds(900., 100., 360., 600.),
            preferred_home_content_size(),
            Edges::default(),
        );
        assert_eq!(
            fitted_size(
                Some(bounds(-1280., -800., 1280., 800.)),
                bounds(-380., -700., 360., 600.),
                preferred_home_content_size(),
                Edges::default(),
            ),
            expected
        );
        assert_eq!(expected, gpui::size(gpui::px(360.), gpui::px(636.)));
    }

    #[test]
    fn startup_reserves_the_expanded_frame_inside_small_displays() {
        for (width, height) in [(800., 600.), (1280., 720.), (1280., 800.), (1920., 1080.)] {
            let display = bounds(-width, -height, width, height);
            let initial = super::initial_bounds(Some(display));
            assert!(initial.left() >= display.left() && initial.top() >= display.top());
            assert!(initial.right() <= display.right() && initial.bottom() <= display.bottom());
            assert!(initial.size.width <= gpui::px(1152.) && initial.size.height <= gpui::px(720.));
        }
    }

    #[test]
    fn geometry_constants_include_frame_insets_in_outer_size() {
        let insets = Edges {
            top: gpui::px(1.),
            right: gpui::px(2.),
            bottom: gpui::px(3.),
            left: gpui::px(4.),
        };
        assert_eq!(
            add_insets(preferred_home_content_size(), insets),
            gpui::size(gpui::px(366.), gpui::px(724.))
        );
    }

    #[gpui::test]
    fn compact_home_keeps_primary_action_and_profile_switcher_visible(
        cx: &mut gpui::TestAppContext,
    ) {
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        let fitted = fitted_size(
            Some(bounds(0., 0., 1280., 800.)),
            bounds(200., 100., 1152., 720.),
            preferred_home_content_size(),
            Edges::default(),
        );
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
                assert!(
                    action.is_some_and(|action| action.origin.x >= gpui::px(0.)
                        && action.origin.y >= gpui::px(0.)
                        && action.right() <= dimensions.width
                        && action.bottom() <= dimensions.height),
                    "{selector} is outside the fitted Home: {action:?}, {dimensions:?}"
                );
            }
        }
    }
}
