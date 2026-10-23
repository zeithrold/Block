//! Adapter lifecycle and physical input regressions for transient feedback.
#![expect(
    clippy::expect_used,
    reason = "Missing adapter state is an explicit fixture failure."
)]
use super::*;

fn fixture(
    cx: &mut gpui::TestAppContext,
) -> (
    gpui::Entity<crate::Block>,
    &mut gpui::VisualTestContext,
    crate::platform::SystemMotionOverride,
) {
    let system_motion = crate::platform::test_system_motion(false);
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    (view, cx, system_motion)
}

#[test]
fn toast_ids_are_stable_across_clones_and_lifetimes_follow_kind() {
    let success = Toast::new("saved".into(), ToastKind::Success);
    let copy = success.clone();
    assert_eq!(success.id, copy.id);
    assert!(success.expires.saturating_duration_since(Instant::now()) <= Duration::from_secs(4));
    let error = Toast::new("failed".into(), ToastKind::Error);
    assert!(error.expires - Instant::now() <= Duration::from_secs(8));
}

#[test]
fn manager_keeps_five_active_items_and_retains_exiting_items() {
    let now = Instant::now();
    let mut manager = ToastManager::new(ToastMotion::sonner());
    for id in 1..=6 {
        manager.push(
            id,
            Rc::new(RefCell::new(Toast::new(format!("{id}"), ToastKind::Info))),
            ToastOptions::default(),
            now,
        );
    }
    trim_to_limit(&mut manager, now);
    let active = manager
        .iter()
        .filter(|(_, _, status)| *status != ToastTransitionStatus::Ending)
        .count();
    assert_eq!(active, 5);
    assert!(
        manager
            .iter()
            .any(|(_, _, status)| status == ToastTransitionStatus::Ending)
    );
}

#[test]
fn duplicate_refreshes_deadline_without_restarting_manager_entry() {
    let now = Instant::now();
    let mut manager = ToastManager::new(ToastMotion::sonner());
    let value = Rc::new(RefCell::new(Toast {
        id: 41,
        message: "saved".into(),
        kind: ToastKind::Success,
        expires: now + Duration::from_secs(4),
    }));
    manager.push(41, value.clone(), ToastOptions::default(), now);
    manager.advance(now + Duration::from_millis(120), false);
    let status = manager.iter().next().expect("one mounted toast").2;

    let duplicate = Toast::new("saved".into(), ToastKind::Success);
    assert_eq!(
        coalesce_existing(&manager, &duplicate, now + Duration::from_secs(1)),
        Some(41)
    );
    assert_eq!(manager.len(), 1);
    assert_eq!(manager.iter().next().expect("one mounted toast").2, status);
    assert!(value.borrow().expires > now + Duration::from_secs(4));
}

#[test]
fn pause_uses_elapsed_duration_and_resume_expires_again() {
    let now = Instant::now();
    let deadline = now + Duration::from_secs(1);
    let mut manager = ToastManager::new(ToastMotion::sonner());
    manager.push(
        9,
        Rc::new(RefCell::new(Toast {
            id: 9,
            message: "working".into(),
            kind: ToastKind::Info,
            expires: deadline,
        })),
        ToastOptions::default(),
        now,
    );
    manager.advance(now + Duration::from_millis(400), false);
    let paused_at = now + Duration::from_millis(1400);
    pause_expiries(&manager, Duration::from_millis(1000));
    manager.advance(paused_at, true);
    assert!(expired_present(&manager, paused_at).is_empty());

    let resumed_at = now + Duration::from_millis(2100);
    manager.advance(resumed_at, false);
    assert_eq!(expired_present(&manager, resumed_at), vec![9]);
}

#[test]
fn swipe_translation_and_motion_respect_reduced_motion() {
    let mut drag = DragState::new(gpui::point(px(10.), px(10.)), gpui::Point::default());
    drag.update(gpui::point(px(90.), px(14.)));
    let offset = drag_visual(Some(drag));
    assert_eq!(offset.x, px(80.));
    assert_eq!(offset.y, px(0.));
    assert_eq!(animation_duration(true), Duration::ZERO);
    assert_eq!(animation_duration(false), Duration::from_millis(400));
}

#[test]
fn swipe_requires_horizontal_or_downward_travel() {
    assert!(swipe_dismisses(px(72.), px(0.)));
    assert!(swipe_dismisses(px(0.), px(48.)));
    assert!(!swipe_dismisses(px(30.), px(-50.)));
    assert!(!swipe_dismisses(px(20.), px(20.)));
}

#[test]
fn diagonal_swipe_keeps_the_axis_selected_after_the_initial_slop() {
    let mut drag = DragState::new(gpui::Point::default(), gpui::Point::default());
    drag.update(gpui::point(px(10.), px(4.)));
    drag.update(gpui::point(px(20.), px(120.)));
    assert_eq!(drag.axis, Some(Axis::Horizontal));
    assert_eq!(drag_visual(Some(drag)), gpui::point(px(20.), px(0.)));
    assert!(!drag.dismisses());
    drag.update(gpui::point(px(80.), px(120.)));
    assert!(drag.dismisses());
}

#[gpui::test]
fn adapter_coalesces_duplicates_without_restarting_the_entry_phase(cx: &mut gpui::TestAppContext) {
    let (_, cx, _system_motion) = fixture(cx);
    let layer = cx.update(|window, app| {
        Root::read(window, app)
            .plugin::<ToastLayer>()
            .expect("product windows install the toast layer")
    });
    cx.update(|window, app| {
        ToastLayer::enqueue(
            vec![Toast::new("Same message".into(), ToastKind::Info)],
            window,
            app,
        );
    });
    let before = layer.read_with(cx, |layer, _| {
        let (id, toast, status) = layer.manager.iter().next().expect("one mounted toast");
        (*id, toast.borrow().expires, status)
    });

    cx.executor().advance_clock(Duration::from_millis(100));
    cx.update(|window, app| {
        ToastLayer::enqueue(
            vec![Toast::new("Same message".into(), ToastKind::Info)],
            window,
            app,
        );
    });
    let after = layer.read_with(cx, |layer, _| {
        let entries = layer.manager.iter().collect::<Vec<_>>();
        assert_eq!(entries.len(), 1);
        (*entries[0].0, entries[0].1.borrow().expires, entries[0].2)
    });
    assert_eq!(before.0, after.0);
    assert_eq!(before.2, ToastTransitionStatus::Starting);
    assert_eq!(after.2, before.2);
    assert!(after.1 > before.1);
}

#[gpui::test]
fn adapter_expires_after_executor_clock_advances(cx: &mut gpui::TestAppContext) {
    let (_, cx, _system_motion) = fixture(cx);
    let layer = cx.update(|window, app| {
        Root::read(window, app)
            .plugin::<ToastLayer>()
            .expect("product windows install the toast layer")
    });
    cx.update(|window, app| {
        ToastLayer::enqueue(
            vec![Toast::new("Expires".into(), ToastKind::Info)],
            window,
            app,
        );
    });
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_secs(5));
    cx.run_until_parked();
    assert!(layer.read_with(cx, |layer, _| {
        layer
            .manager
            .iter()
            .all(|(_, _, status)| status == ToastTransitionStatus::Ending)
    }));
    frame(cx, 250);
    assert!(layer.read_with(cx, |layer, _| layer.manager.is_empty()
        && !layer.timer_running));
}

fn frame(cx: &mut gpui::VisualTestContext, millis: u64) {
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(millis));
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

fn enqueue(cx: &mut gpui::VisualTestContext) -> gpui::Entity<ToastLayer> {
    cx.update(|window, app| {
        ToastLayer::enqueue(
            vec![Toast::new("Settings saved".into(), ToastKind::Success)],
            window,
            app,
        );
        Root::read(window, app)
            .plugin::<ToastLayer>()
            .expect("toast plugin")
    })
}

#[gpui::test]
fn pointer_hover_pauses_expiry_and_leaving_resumes_it(cx: &mut gpui::TestAppContext) {
    let (_, cx, _system_motion) = fixture(cx);
    let layer = enqueue(cx);
    frame(cx, 500);
    let card = cx.debug_bounds("toast").expect("painted toast");
    cx.simulate_mouse_move(card.center(), None, gpui::Modifiers::none());
    frame(cx, 50);
    assert!(layer.read_with(cx, |layer, _| layer.stack_state.is_expanded()));
    frame(cx, 10_000);
    assert!(layer.read_with(cx, |layer, _| {
        layer
            .manager
            .iter()
            .any(|(_, _, phase)| phase == ToastTransitionStatus::Present)
    }));
    cx.simulate_mouse_move(gpui::point(px(0.), px(0.)), None, gpui::Modifiers::none());
    frame(cx, 50);
    assert!(!layer.read_with(cx, |layer, _| layer.stack_state.is_expanded()));
    frame(cx, 4_000);
    frame(cx, 250);
    assert!(layer.read_with(cx, |layer, _| layer.manager.is_empty()
        && !layer.timer_running));
}

#[gpui::test]
fn keyboard_close_uses_native_activation_and_pauses_expiry(cx: &mut gpui::TestAppContext) {
    let (_, cx, _system_motion) = fixture(cx);
    let layer = enqueue(cx);
    frame(cx, 500);
    let focus = layer
        .read_with(cx, |layer, _| layer.close_focus.values().next().cloned())
        .expect("close focus");
    cx.update(|window, app| focus.focus(window, app));
    frame(cx, 50);
    frame(cx, 10_000);
    assert!(layer.read_with(cx, |layer, _| {
        layer
            .manager
            .iter()
            .any(|(_, _, phase)| phase == ToastTransitionStatus::Present)
    }));
    crate::test_support::press(cx, "space");
    frame(cx, 250);
    assert!(layer.read_with(cx, |layer, _| layer.manager.is_empty()));
}

#[gpui::test]
fn swipe_tracks_pointer_outside_card_and_dismisses(cx: &mut gpui::TestAppContext) {
    let (_, cx, _system_motion) = fixture(cx);
    cx.simulate_resize(gpui::size(px(432.), px(768.)));
    let layer = enqueue(cx);
    frame(cx, 500);
    let card = cx.debug_bounds("toast").expect("painted toast");
    let start = card.center();
    let end = gpui::point(card.right() + px(24.), start.y);
    cx.simulate_mouse_down(start, MouseButton::Left, gpui::Modifiers::none());
    cx.simulate_mouse_move(end, MouseButton::Left, gpui::Modifiers::none());
    frame(cx, 20);
    let moved = cx.debug_bounds("toast").expect("dragged toast");
    assert!(moved.origin.x > card.origin.x + px(72.));
    cx.simulate_mouse_up(end, MouseButton::Left, gpui::Modifiers::none());
    frame(cx, 250);
    assert!(layer.read_with(cx, |layer, _| layer.manager.is_empty()
        && layer.gestures.borrow().is_empty()));
}

#[gpui::test]
fn enabling_reduced_motion_settles_an_in_progress_exit(cx: &mut gpui::TestAppContext) {
    let (view, cx, _system_motion) = fixture(cx);
    let layer = enqueue(cx);
    frame(cx, 500);
    let id = layer.read_with(cx, |layer, _| {
        *layer.manager.iter().next().expect("toast").0
    });
    layer.update(cx, |layer, cx| layer.dismiss(id, cx));
    view.update(cx, |view, cx| {
        view.preferences.reduce_motion = true;
        cx.notify();
    });
    cx.run_until_parked();
    assert!(layer.read_with(cx, |layer, _| layer.manager.is_empty()));
    assert!(cx.debug_bounds("toast").is_none());
}

#[gpui::test]
fn system_reduced_motion_settles_exit_with_the_app_preference_off(cx: &mut gpui::TestAppContext) {
    let (view, cx, _normal_policy) = fixture(cx);
    let layer = enqueue(cx);
    frame(cx, 500);
    let id = layer.read_with(cx, |layer, _| {
        *layer.manager.iter().next().expect("toast").0
    });
    layer.update(cx, |layer, cx| layer.dismiss(id, cx));
    let _reduced_policy = crate::platform::test_system_motion(true);
    view.update(cx, |_, cx| cx.notify());
    frame(cx, 0);
    assert!(!view.read_with(cx, |view, _| view.preferences.reduce_motion));
    assert!(layer.read_with(cx, |layer, _| layer.manager.is_empty()));
    assert!(cx.debug_bounds("toast").is_none());
}

fn card(cx: &mut gpui::VisualTestContext, id: u64) -> gpui::Bounds<Pixels> {
    cx.update(|window, _| {
        gpui_base::test_support::find(
            window,
            &[],
            &ElementId::NamedInteger("block-toast-card".into(), id),
        )
    })
    .expect("painted card")
    .bounds()
}

fn first_id(layer: &gpui::Entity<ToastLayer>, cx: &gpui::VisualTestContext) -> u64 {
    layer.read_with(cx, |layer, _| {
        *layer.manager.iter().next().expect("toast").0
    })
}

fn assert_near(actual: Pixels, expected: Pixels) {
    assert!(
        (actual - expected).abs() < px(0.6),
        "{actual:?} != {expected:?}"
    );
}

#[gpui::test]
fn entrance_travels_one_card_height_on_the_css_ease_curve(cx: &mut gpui::TestAppContext) {
    let (_, cx, _system_motion) = fixture(cx);
    let layer = enqueue(cx);
    let id = first_id(&layer, cx);
    frame(cx, 0);
    let start = card(cx, id);
    frame(cx, 100);
    let quarter = card(cx, id);
    frame(cx, 100);
    let half = card(cx, id);
    frame(cx, 200);
    let settled = card(cx, id);
    // Browser CSS ease: cubic-bezier(.25, .1, .25, 1), sampled at .25 and .5.
    assert_near(start.origin.y - settled.origin.y, settled.size.height);
    assert_near(
        quarter.origin.y - settled.origin.y,
        settled.size.height * 0.59149,
    );
    assert_near(
        half.origin.y - settled.origin.y,
        settled.size.height * 0.19760,
    );
}

#[gpui::test]
fn stack_retargets_from_its_painted_position_with_css_timing(cx: &mut gpui::TestAppContext) {
    let (_, cx, _system_motion) = fixture(cx);
    let layer = enqueue(cx);
    let id = first_id(&layer, cx);
    frame(cx, 500);
    let initial = card(cx, id);
    cx.update(|window, app| {
        ToastLayer::enqueue(
            vec![Toast::new("Second message".into(), ToastKind::Info)],
            window,
            app,
        )
    });
    frame(cx, 0);
    assert_near(card(cx, id).bottom(), initial.bottom());
    frame(cx, 100);
    let quarter = card(cx, id);
    assert_near(quarter.bottom(), initial.bottom() - px(14.) * 0.40851);
    assert_near(
        quarter.size.width,
        initial.size.width * (1. - 0.05 * 0.40851),
    );
    cx.update(|window, app| {
        ToastLayer::enqueue(
            vec![Toast::new("Third message".into(), ToastKind::Info)],
            window,
            app,
        )
    });
    frame(cx, 0);
    assert_near(card(cx, id).bottom(), quarter.bottom());
    assert_near(card(cx, id).size.width, quarter.size.width);
    frame(cx, 400);
    assert_near(card(cx, id).bottom(), initial.bottom() - px(28.));
    assert_near(card(cx, id).size.width, initial.size.width * 0.9);
}

#[gpui::test]
fn stack_expand_and_interrupted_collapse_share_css_reversal_timing(cx: &mut gpui::TestAppContext) {
    let (_, cx, _system_motion) = fixture(cx);
    let layer = enqueue(cx);
    let id = first_id(&layer, cx);
    let front = Toast::new("Front message".into(), ToastKind::Info);
    let front_id = front.id;
    cx.update(|window, app| ToastLayer::enqueue(vec![front], window, app));
    frame(cx, 500);
    let collapsed = card(cx, id);
    let front = card(cx, front_id);
    cx.simulate_mouse_move(front.center(), None, gpui::Modifiers::none());
    frame(cx, 0);
    assert_near(card(cx, id).bottom(), collapsed.bottom());
    frame(cx, 100);
    let expanding = card(cx, id);
    assert_near(
        expanding.bottom(),
        collapsed.bottom() - front.size.height * 0.40851,
    );
    cx.simulate_mouse_move(gpui::point(px(0.), px(0.)), None, gpui::Modifiers::none());
    frame(cx, 0);
    assert_near(card(cx, id).bottom(), expanding.bottom());
    frame(cx, 200);
    assert_near(card(cx, id).bottom(), collapsed.bottom());
    assert_near(card(cx, id).size.width, collapsed.size.width);
}

#[gpui::test]
fn collapsed_back_toast_uses_500ms_motion_before_200ms_removal(cx: &mut gpui::TestAppContext) {
    let (_, cx, _system_motion) = fixture(cx);
    let layer = enqueue(cx);
    let id = first_id(&layer, cx);
    cx.update(|window, app| {
        ToastLayer::enqueue(
            vec![Toast::new("Front message".into(), ToastKind::Info)],
            window,
            app,
        )
    });
    frame(cx, 500);
    let initial = card(cx, id);
    layer.update(cx, |layer, cx| layer.dismiss(id, cx));
    frame(cx, 0);
    frame(cx, 100);
    assert_near(
        card(cx, id).origin.y,
        initial.origin.y + initial.size.height * 0.4 * 0.29524,
    );
    frame(cx, 150);
    assert!(layer.read_with(cx, |layer, _| layer.manager.get(&id).is_none()));
}

#[gpui::test]
fn system_reduced_motion_settles_entry_without_replaying_when_disabled(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx, _system_motion) = fixture(cx);
    let layer = enqueue(cx);
    let id = first_id(&layer, cx);
    frame(cx, 0);
    let start = card(cx, id);
    frame(cx, 100);
    let moving = card(cx, id);
    assert!(moving.origin.y < start.origin.y);
    let reduced_policy = crate::platform::test_system_motion(true);
    view.update(cx, |_, cx| cx.notify());
    frame(cx, 0);
    let settled = card(cx, id);
    assert_near(start.origin.y - settled.origin.y, settled.size.height);
    drop(reduced_policy);
    view.update(cx, |_, cx| cx.notify());
    frame(cx, 100);
    assert_near(card(cx, id).origin.y, settled.origin.y);
}

#[gpui::test]
fn rejected_swipe_returns_over_400ms_without_snapping(cx: &mut gpui::TestAppContext) {
    let (_, cx, _system_motion) = fixture(cx);
    let layer = enqueue(cx);
    let id = first_id(&layer, cx);
    frame(cx, 500);
    let initial = card(cx, id);
    let end = initial.center() + gpui::point(px(20.), px(0.));
    cx.simulate_mouse_down(initial.center(), MouseButton::Left, gpui::Modifiers::none());
    cx.simulate_mouse_move(end, MouseButton::Left, gpui::Modifiers::none());
    frame(cx, 0);
    assert_near(card(cx, id).origin.x, initial.origin.x + px(20.));
    cx.simulate_mouse_up(end, MouseButton::Left, gpui::Modifiers::none());
    frame(cx, 0);
    assert_near(card(cx, id).origin.x, initial.origin.x + px(20.));
    frame(cx, 100);
    assert_near(card(cx, id).origin.x, initial.origin.x + px(20.) * 0.59149);
    frame(cx, 300);
    assert_near(card(cx, id).origin.x, initial.origin.x);
    assert!(layer.read_with(cx, |layer, _| layer.swipe_exits.is_empty()));
}

#[gpui::test]
fn grabbing_a_returning_toast_continues_from_its_painted_offset(cx: &mut gpui::TestAppContext) {
    let (_, cx, _system_motion) = fixture(cx);
    let layer = enqueue(cx);
    let id = first_id(&layer, cx);
    frame(cx, 500);
    let initial = card(cx, id);
    let end = initial.center() + gpui::point(px(20.), px(0.));
    cx.simulate_mouse_down(initial.center(), MouseButton::Left, gpui::Modifiers::none());
    cx.simulate_mouse_move(end, MouseButton::Left, gpui::Modifiers::none());
    frame(cx, 0);
    cx.simulate_mouse_up(end, MouseButton::Left, gpui::Modifiers::none());
    frame(cx, 0);
    frame(cx, 100);
    let returning = card(cx, id);
    cx.simulate_mouse_down(
        returning.center(),
        MouseButton::Left,
        gpui::Modifiers::none(),
    );
    frame(cx, 0);
    assert_near(card(cx, id).origin.x, returning.origin.x);
    let end = returning.center() + gpui::point(px(10.), px(0.));
    cx.simulate_mouse_move(end, MouseButton::Left, gpui::Modifiers::none());
    frame(cx, 0);
    assert_near(card(cx, id).origin.x, returning.origin.x + px(10.));
    cx.simulate_mouse_up(end, MouseButton::Left, gpui::Modifiers::none());
    frame(cx, 0);
    frame(cx, 400);
    assert_near(card(cx, id).origin.x, initial.origin.x);
}

#[gpui::test]
fn accepted_swipe_continues_outward_from_release_with_200ms_ease_out(
    cx: &mut gpui::TestAppContext,
) {
    let (_, cx, _system_motion) = fixture(cx);
    let layer = enqueue(cx);
    let id = first_id(&layer, cx);
    frame(cx, 500);
    let initial = card(cx, id);
    let end = initial.center() + gpui::point(px(80.), px(0.));
    cx.simulate_mouse_down(initial.center(), MouseButton::Left, gpui::Modifiers::none());
    cx.simulate_mouse_move(end, MouseButton::Left, gpui::Modifiers::none());
    frame(cx, 0);
    let dragged = card(cx, id);
    cx.simulate_mouse_up(end, MouseButton::Left, gpui::Modifiers::none());
    frame(cx, 0);
    assert_near(card(cx, id).origin.x, dragged.origin.x);
    frame(cx, 100);
    // Browser CSS ease-out: cubic-bezier(0, 0, .58, 1), sampled at .5.
    assert_near(
        card(cx, id).origin.x,
        dragged.origin.x + initial.size.width * 0.68464,
    );
    frame(cx, 150);
    assert!(layer.read_with(cx, |layer, _| layer.manager.is_empty()
        && layer.swipe_exits.is_empty()));
}

#[gpui::test]
fn ordinary_exit_matches_the_prototype_200ms_retention(cx: &mut gpui::TestAppContext) {
    let (_, cx, _system_motion) = fixture(cx);
    let layer = enqueue(cx);
    let id = first_id(&layer, cx);
    frame(cx, 500);
    let initial = card(cx, id);
    layer.update(cx, |layer, cx| layer.dismiss(id, cx));
    frame(cx, 0);
    assert_near(card(cx, id).origin.y, initial.origin.y);
    frame(cx, 100);
    assert_near(
        card(cx, id).origin.y,
        initial.origin.y + initial.size.height * 0.40851,
    );
    frame(cx, 150);
    assert!(cx.debug_bounds("toast").is_none());
    assert!(layer.read_with(cx, |layer, _| layer.manager.is_empty()));
}
