//! Fixed product window sizes.

use gpui::{App, Bounds, Edges, Pixels, Size, Window, px, size};

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
pub(super) fn home_size(window: &Window) -> Size<Pixels> {
    add_insets(preferred_home_content_size(), content_insets(window))
}

#[expect(
    clippy::redundant_pub_crate,
    reason = "Only the parent UI module uses this helper."
)]
pub(super) fn management_size(window: &Window) -> Size<Pixels> {
    add_insets(preferred_management_size(), content_insets(window))
}

/// Fixed Manage content dimensions used to center the application window.
pub fn preferred_management_size() -> Size<Pixels> {
    size(px(1152.), MANAGEMENT_HEIGHT)
}

/// Reserve the expanded frame's position before Home is shown.
pub fn initial_window_bounds(cx: &App) -> Bounds<Pixels> {
    let center = cx
        .primary_display()
        .map_or_else(Default::default, |display| display.bounds().center());
    Bounds::centered_at(center, preferred_management_size())
}

/// Width-only navigation targets. The current viewport height is retained so
/// callers can morph Home into a sidebar at a fixed top-left corner.
#[expect(
    clippy::redundant_pub_crate,
    reason = "Only the parent UI module uses this helper."
)]
pub(super) fn navigation_sizes(window: &Window) -> (Size<Pixels>, Size<Pixels>) {
    navigation_sizes_for(window.viewport_size().height, content_insets(window))
}

const fn preferred_home_content_size() -> Size<Pixels> {
    size(px(super::design::SIDEBAR_WIDTH), MANAGEMENT_HEIGHT)
}

fn navigation_sizes_for(
    viewport_height: Pixels,
    insets: Edges<Pixels>,
) -> (Size<Pixels>, Size<Pixels>) {
    let preferred_home = add_insets(preferred_home_content_size(), insets);
    let preferred_management = add_insets(preferred_management_size(), insets);
    (
        size(preferred_home.width, viewport_height),
        size(preferred_management.width, viewport_height),
    )
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
    use gpui::Edges;

    use super::{
        add_insets, navigation_sizes_for, preferred_home_content_size, preferred_management_size,
    };

    #[test]
    fn home_and_management_share_fixed_content_height_and_sidebar_width() {
        let insets = Edges {
            top: gpui::px(2.),
            right: gpui::px(3.),
            bottom: gpui::px(4.),
            left: gpui::px(5.),
        };
        let home = add_insets(preferred_home_content_size(), insets);
        let management = add_insets(preferred_management_size(), insets);
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
        let insets = Edges {
            top: gpui::px(1.),
            right: gpui::px(2.),
            bottom: gpui::px(3.),
            left: gpui::px(4.),
        };
        let (home, management) = navigation_sizes_for(gpui::px(611.), insets);
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
    fn home_keeps_primary_action_and_profile_switcher_visible(cx: &mut gpui::TestAppContext) {
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        let dimensions = cx.update(|window, _| super::home_size(window));
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
            view.router.navigate(crate::Tab::Home);
            view.preferences.reduce_motion = true;
            cx.notify();
        });
        cx.simulate_resize(dimensions);
        cx.run_until_parked();
        for selector in ["home-play", "home-profiles", "app-menu"] {
            let action = cx.debug_bounds(selector);
            assert!(
                action.is_some_and(|action| action.origin.x >= gpui::px(0.)
                    && action.origin.y >= gpui::px(0.)
                    && action.right() <= dimensions.width
                    && action.bottom() <= dimensions.height),
                "{selector} is outside Home: {action:?}, {dimensions:?}"
            );
        }
    }
}
