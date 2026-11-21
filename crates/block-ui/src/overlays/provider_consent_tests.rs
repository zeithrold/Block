use super::*;

fn set_provider_address(view: &Entity<Block>, value: &str, cx: &mut VisualTestContext) {
    view.update(cx, |view, cx| {
        if let Some(draft) = view
            .shell
            .auth
            .as_ref()
            .and_then(|state| state.provider_draft.as_ref())
        {
            draft
                .address
                .update(cx, |field, cx| field.set_value(value, cx));
        }
    });
    cx.run_until_parked();
}

#[gpui::test]
fn provider_drawer_reserves_then_reveals_at_the_fitted_height(cx: &mut TestAppContext) {
    let (view, cx, _system_motion) = fixture(cx, false);
    act(&view, UiAction::SignIn, cx);
    view.update(cx, |view, cx| {
        view.choose_auth_provider(block_auth::AccountProvider::Authlib, cx)
    });
    act(&view, UiAction::AuthProviderAdd, cx);
    frame(cx, 900);
    let compact = bounds(cx, "block-drawer");
    set_provider_address(&view, "http://example.test", cx);
    assert_eq!(bounds(cx, "block-drawer"), compact);
    let blocked_title = tr!(
        view.read_with(cx, |v, _| v.language()),
        "accounts.auth.provider_add.confirm_http"
    );
    assert_eq!(
        control_node(cx, "auth-provider-identify").label(),
        Some(blocked_title.as_str())
    );
    act(&view, UiAction::AuthProviderConsent, cx);
    frame(cx, 60);
    let expanding = bounds(cx, "block-drawer");
    assert!(
        expanding.top() < compact.top(),
        "{expanding:?} vs {compact:?}"
    );
    assert_eq!(expanding.bottom(), compact.bottom());
    assert!(crate::test_support::control_bounds(cx, "auth-provider-http").is_none());
    // Even direct dispatch cannot pre-approve a checkbox that has not appeared.
    act(&view, UiAction::AuthProviderConsent, cx);
    frame(cx, 90);
    let halfway = bounds(cx, "block-drawer");
    frame(cx, 90);
    let braking = bounds(cx, "block-drawer");
    frame(cx, 60);
    let expanded = bounds(cx, "block-drawer");
    let total = compact.top() - expanded.top();
    assert!(
        compact.top() - expanding.top() < total * 0.08
            && ((compact.top() - halfway.top()) / total - 0.64).abs() < 0.03
            && braking.top() - expanded.top() < total * 0.06
            && braking.top() > expanded.top(),
        "travel must accelerate into a longer braking phase: {compact:?}, {expanding:?}, {halfway:?}, {braking:?}, {expanded:?}"
    );
    assert!(expanded.top() < expanding.top());
    assert_eq!(expanded.bottom(), compact.bottom());
    assert!(crate::test_support::control_bounds(cx, "auth-provider-warning").is_none());
    animation_frame(cx, 60);
    assert_eq!(bounds(cx, "block-drawer"), expanded);
    assert_provider_consent_waits_for_warning(&view, cx);
    set_provider_address(&view, "https://example.test", cx);
    assert_eq!(bounds(cx, "block-drawer"), expanded);
    assert!(crate::test_support::control_bounds(cx, "auth-provider-warning").is_none());
    assert_ne!(
        control_node(cx, "auth-provider-identify").disabled(),
        Some(true)
    );
    frame(cx, 64);
    let shrinking = bounds(cx, "block-drawer");
    assert!(expanded.top() < shrinking.top() && shrinking.top() < compact.top());
    assert_eq!(shrinking.bottom(), compact.bottom());
    assert!(crate::test_support::control_bounds(cx, "auth-provider-http").is_none());
    frame(cx, 236);
    assert_eq!(bounds(cx, "block-drawer"), compact);
    assert_provider_drawer_refits_after_resize_and_edits(&view, cx, compact, expanded);
}

fn assert_provider_consent_waits_for_warning(view: &Entity<Block>, cx: &mut VisualTestContext) {
    let warning_indicator_alpha = |cx: &mut VisualTestContext| {
        control_quads(cx, "auth-provider-http-indicator")
            .iter()
            .find_map(|quad| quad.background.as_solid())
            .map_or(0., |color| color.a)
    };
    let fading_alpha = warning_indicator_alpha(cx);
    assert!(fading_alpha > 0. && fading_alpha < 1., "{fading_alpha}");
    let fading_checkbox = control_node(cx, "auth-provider-http");
    assert_eq!(fading_checkbox.disabled(), Some(true));
    for _ in 0..4 {
        crate::test_support::press(cx, "tab");
        assert_ne!(control_node(cx, "auth-provider-http").focused(), Some(true));
    }
    click(cx, "auth-provider-http");
    act(view, UiAction::AuthProviderConsent, cx);
    animation_frame(cx, 60);
    assert!(warning_indicator_alpha(cx) > 0.99);
    assert_eq!(
        control_node(cx, "auth-provider-identify").disabled(),
        Some(true)
    );
    click(cx, "auth-provider-http");
    assert_ne!(
        control_node(cx, "auth-provider-identify").disabled(),
        Some(true)
    );
}

fn assert_provider_drawer_refits_after_resize_and_edits(
    view: &Entity<Block>,
    cx: &mut VisualTestContext,
    compact: gpui::Bounds<gpui::Pixels>,
    expanded: gpui::Bounds<gpui::Pixels>,
) {
    cx.simulate_resize(size(px(300.), px(768.)));
    frame(cx, 1200);
    set_provider_address(view, "http://example.test", cx);
    frame(cx, 1200);
    let narrow = bounds(cx, "block-drawer");
    assert!(
        narrow.size.height > expanded.size.height,
        "wrapped copy contributes to the natural height"
    );
    assert_eq!(narrow.bottom(), expanded.bottom());
    cx.simulate_resize(size(px(432.), px(768.)));
    set_provider_address(view, "https://example.test", cx);
    frame(cx, 1200);
    set_provider_address(view, "http://example.test", cx);
    frame(cx, 150);
    let interrupted = bounds(cx, "block-drawer");
    set_provider_address(view, "https://example.test", cx);
    assert_eq!(bounds(cx, "block-drawer"), interrupted);
    frame(cx, 64);
    let returning = bounds(cx, "block-drawer");
    assert!(interrupted.top() < returning.top() && returning.top() < compact.top());
    frame(cx, 236);
    assert_eq!(bounds(cx, "block-drawer"), compact);
    assert!(crate::test_support::control_bounds(cx, "auth-provider-http").is_none());
    set_provider_address(view, "http://example.test", cx);
    frame(cx, 300);
    assert_eq!(bounds(cx, "block-drawer"), expanded);
    animation_frame(cx, 60);
    frame(cx, 60);
    assert!(crate::test_support::control_bounds(cx, "auth-provider-warning").is_some());
    set_provider_address(view, "http://other.test", cx);
    assert!(crate::test_support::control_bounds(cx, "auth-provider-warning").is_some());
    assert_ne!(
        control_node(cx, "auth-provider-http").disabled(),
        Some(true)
    );
    assert_eq!(
        control_node(cx, "auth-provider-identify").disabled(),
        Some(true)
    );
}

#[gpui::test]
fn provider_drawer_tracks_live_edits_without_restarting_visible_warning(cx: &mut TestAppContext) {
    let (view, cx, _system_motion) = fixture(cx, true);
    act(&view, UiAction::SignIn, cx);
    view.update(cx, |view, cx| {
        view.choose_auth_provider(block_auth::AccountProvider::Authlib, cx)
    });
    act(&view, UiAction::AuthProviderAdd, cx);
    frame(cx, 900);
    let compact = bounds(cx, "block-drawer");
    cx.update(|window, cx| {
        if let Some(draft) = view
            .read(cx)
            .shell
            .auth
            .as_ref()
            .and_then(|s| s.provider_draft.as_ref())
        {
            draft.address.read(cx).focus_handle(cx).focus(window, cx);
        }
    });
    cx.simulate_input("http://");
    cx.run_until_parked();
    assert_eq!(bounds(cx, "block-drawer"), compact);
    assert!(crate::test_support::control_bounds(cx, "auth-provider-warning").is_none());
    cx.simulate_input("e");
    cx.run_until_parked();
    let expanded = bounds(cx, "block-drawer");
    assert!(expanded.top() < compact.top());
    let warning = bounds(cx, "auth-provider-warning");
    for part in ["x", "ample.test"] {
        cx.simulate_input(part);
        cx.run_until_parked();
        assert_eq!(bounds(cx, "block-drawer"), expanded);
        assert_eq!(bounds(cx, "auth-provider-warning"), warning);
    }
    act(&view, UiAction::AuthProviderConsent, cx);
    assert_ne!(
        control_node(cx, "auth-provider-identify").disabled(),
        Some(true)
    );
    cx.simulate_input(" ");
    cx.run_until_parked();
    assert_eq!(bounds(cx, "auth-provider-warning"), warning);
    assert_eq!(
        control_node(cx, "auth-provider-identify").disabled(),
        Some(true),
        "even a canonical-equivalent raw edit revokes approval"
    );
    assert_ne!(
        control_node(cx, "auth-provider-http").disabled(),
        Some(true)
    );
    act(&view, UiAction::AuthProviderConsent, cx);
    cx.simulate_input("x");
    cx.run_until_parked();
    assert_eq!(bounds(cx, "block-drawer"), compact);
    assert_eq!(
        control_node(cx, "auth-provider-identify").disabled(),
        Some(true)
    );
    assert!(crate::test_support::control_bounds(cx, "auth-provider-warning").is_none());
    act(&view, UiAction::AuthProviderConsent, cx);
    crate::test_support::press(cx, "backspace");
    cx.run_until_parked();
    assert_eq!(bounds(cx, "auth-provider-warning"), warning);
    assert_eq!(
        control_node(cx, "auth-provider-identify").disabled(),
        Some(true),
        "returning to the exact previously approved text cannot restore consent"
    );
}
