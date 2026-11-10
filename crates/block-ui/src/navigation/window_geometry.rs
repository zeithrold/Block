//! Fixed product window sizes.

use gpui::{App, Bounds, Edges, Pixels, Size, Window, px, size};

const PRODUCT_HEIGHT: Pixels = px(720.);

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

/// Fixed outer size, including the actual native frame insets once.
#[expect(
    clippy::redundant_pub_crate,
    reason = "Only the parent UI module uses this helper."
)]
pub(super) fn home_size(window: &Window) -> Size<Pixels> {
    add_insets(preferred_management_size(), content_insets(window))
}

/// Fixed product content dimensions, including the shared 56 px title area.
///
/// The historical export name remains until callers adopt the product name.
pub const fn preferred_management_size() -> Size<Pixels> {
    size(px(360.), PRODUCT_HEIGHT)
}

/// Center the compact content bounds for launcher and standalone crash windows.
/// The live window's decoration insets are added when its outer size is set.
pub fn initial_window_bounds(cx: &App) -> Bounds<Pixels> {
    let center = cx
        .primary_display()
        .map_or_else(Default::default, |display| display.bounds().center());
    Bounds::centered_at(center, preferred_management_size())
}

/// Apply live decoration insets once, after the root publishes its native frame.
pub fn initialize_window_size(window: &mut Window) {
    window.on_next_frame(|window, _| window.resize(home_size(window)));
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

    use super::{add_insets, preferred_management_size, subtract_insets};

    #[test]
    fn product_content_budget_includes_the_shared_chrome() {
        assert_eq!(
            preferred_management_size(),
            gpui::size(gpui::px(360.), gpui::px(720.))
        );
        assert_eq!(
            preferred_management_size().height - gpui::px(crate::design::CHROME_HEIGHT),
            gpui::px(664.)
        );
    }

    #[test]
    fn outer_size_adds_actual_insets_once_and_recovers_the_content_budget() {
        let insets = Edges {
            top: gpui::px(1.),
            right: gpui::px(2.),
            bottom: gpui::px(3.),
            left: gpui::px(4.),
        };
        assert_eq!(
            add_insets(preferred_management_size(), insets),
            gpui::size(gpui::px(366.), gpui::px(724.))
        );
        assert_eq!(
            subtract_insets(gpui::size(gpui::px(366.), gpui::px(724.)), insets),
            gpui::size(gpui::px(360.), gpui::px(720.))
        );
    }

    #[gpui::test]
    fn startup_bounds_center_the_compact_product(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let bounds = super::initial_window_bounds(cx);
            let center = cx
                .primary_display()
                .map_or_else(Default::default, |display| display.bounds().center());
            assert_eq!(bounds.size, gpui::size(gpui::px(360.), gpui::px(720.)));
            assert_eq!(bounds.center(), center);
        });
    }

    #[gpui::test]
    fn startup_size_uses_actual_insets_once_after_root_setup(cx: &mut gpui::TestAppContext) {
        let (_, cx) = crate::test_support::window(cx, crate::tests::test_view);
        cx.update(|window, cx| {
            let home = super::home_size(window);
            assert_eq!(
                subtract_insets(home, super::content_insets(window)),
                gpui::size(gpui::px(360.), gpui::px(720.))
            );
            let initial = gpui::size(gpui::px(500.), gpui::px(800.));
            window.resize(initial);
            super::initialize_window_size(window);
            assert_eq!(
                window.window_bounds().get_bounds().size,
                initial,
                "startup sizing must wait for the root's decoration setup"
            );
            window.simulate_next_frame(cx);
            assert_eq!(window.window_bounds().get_bounds().size, home);
            window.resize(initial);
            window.simulate_next_frame(cx);
            assert_eq!(
                window.window_bounds().get_bounds().size,
                initial,
                "startup sizing must not keep correcting later frames"
            );
        });
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
        for selector in ["home-play", "home-instances", "app-menu"] {
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
