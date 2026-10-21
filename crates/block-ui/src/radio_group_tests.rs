//! Radio groups preserve one keyboard entry point and the actual product drafts.
use super::{ControlState, RadioGroup, RadioOption, radio};
use gpui::{
    Context, FocusHandle, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Window, div, px, size,
};
use std::{cell::RefCell, collections::HashMap};

#[derive(Default)]
struct Fixture {
    selected: u8,
    checked: bool,
    checkbox_disabled: bool,
    ancestor_activations: usize,
    focus: RefCell<HashMap<String, FocusHandle>>,
}
impl Render for Fixture {
    fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let group = RadioGroup::new(
            "fixture-radios",
            "Installation choice",
            (0..3)
                .map(|id| RadioOption {
                    id: id.to_string(),
                    checked: self.selected == id,
                    disabled: id == 1,
                    action: id,
                })
                .collect(),
            &self.focus,
            cx,
            cx.listener(|view, selected: &u8, _, cx| {
                view.selected = *selected;
                cx.notify();
            }),
        );
        div()
            .id("radio-fixture")
            .on_click(cx.listener(|view, _, _, _| view.ancestor_activations += 1))
            .child(group.container().children((0..3).map(|id| {
                group.item(
                    &id.to_string(),
                    radio(
                        format!("radio-{id}"),
                        format!("Choice {id}"),
                        self.selected == id,
                        crate::Palette::LIGHT,
                        ControlState {
                            disabled: id == 1,
                            ..Default::default()
                        },
                        cx.listener(move |view, &(): &(), _, cx| {
                            view.selected = id;
                            cx.notify();
                        }),
                    ),
                )
            })))
            .child(super::checkbox(
                "base-checkbox",
                "Enable option",
                self.checked,
                crate::Palette::LIGHT,
                ControlState {
                    disabled: self.checkbox_disabled,
                    ..Default::default()
                },
                cx.listener(|view, (): &(), _, cx| {
                    view.checked = !view.checked;
                    cx.notify();
                }),
            ))
            .child(super::button(
                "radio-after",
                "After group",
                crate::Palette::LIGHT,
                super::ButtonVariant::Outline,
                ControlState::default(),
                |&(), _, _| {},
            ))
    }
}
fn settle(cx: &mut gpui::VisualTestContext) {
    cx.run_until_parked();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(500));
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}
fn click(cx: &mut gpui::VisualTestContext, id: &'static str) {
    settle(cx);
    let bounds = crate::test_support::control_bounds(cx, id);
    assert!(bounds.is_some(), "missing {id}");
    if let Some(bounds) = bounds {
        let window_count = cx.windows().len();
        cx.simulate_click(bounds.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        if cx.windows().len() > window_count
            && let Some(window) = cx.windows().last()
        {
            *cx = gpui::VisualTestContext::from_window(*window, &cx.cx);
        }
        crate::test_support::ensure_live_window(cx);
        cx.run_until_parked();
        crate::test_support::ensure_live_window(cx);
    }
    settle(cx);
}
fn focus_radio(cx: &mut gpui::VisualTestContext, label: &'static str) {
    let id = match label {
        "Choice 0" => "radio-0",
        "Vanilla" => "wizard-loader-vanilla",
        "Use current account" => "default-account-none",
        "vanilla" => "target-vanilla",
        _ => label,
    };
    click(cx, id);
}

#[gpui::test]
fn shared_radio_group_roves_skips_disabled_wraps_and_exposes_one_checked_item(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, |_, _| Fixture {
        selected: 0,
        focus: RefCell::default(),
        ..Default::default()
    });
    settle(cx);
    focus_radio(cx, "Choice 0");
    assert_eq!(view.read_with(cx, |view, _| view.ancestor_activations), 0);
    let keystroke = gpui::Keystroke::parse("space");
    assert!(keystroke.is_ok());
    if let Ok(keystroke) = keystroke {
        cx.simulate_event(gpui::KeyDownEvent {
            keystroke: keystroke.clone(),
            is_held: false,
            prefer_character_input: false,
        });
        cx.simulate_event(gpui::KeyUpEvent { keystroke });
    }
    assert_eq!(view.read_with(cx, |view, _| view.selected), 0);
    assert_eq!(view.read_with(cx, |view, _| view.ancestor_activations), 0);
    cx.simulate_keystrokes("right");
    settle(cx);
    assert_eq!(view.read_with(cx, |view, _| view.selected), 2);
    cx.update(|window, cx| {
        view.read_with(cx, |view, _| {
            assert!(
                view.focus
                    .borrow()
                    .get("fixture-radios:2")
                    .is_some_and(|focus| focus.is_focused(window))
            );
        });
    });
    cx.simulate_keystrokes("right");
    settle(cx);
    assert_eq!(view.read_with(cx, |view, _| view.selected), 0);
    cx.simulate_keystrokes("end");
    settle(cx);
    assert_eq!(view.read_with(cx, |view, _| view.selected), 2);
    cx.simulate_keystrokes("home");
    settle(cx);
    assert_eq!(view.read_with(cx, |view, _| view.selected), 0);
    click(cx, "radio-1");
    assert_eq!(view.read_with(cx, |view, _| view.selected), 0);
    cx.simulate_keystrokes("tab");
    settle(cx);
    assert!(
        cx.update(|window, cx| view.read_with(cx, |view, _| !view
            .focus
            .borrow()
            .values()
            .any(|focus| focus.is_focused(window)))),
        "Tab moves outside the radio group"
    );
}
#[gpui::test]
fn base_checkbox_pointer_and_native_key_up_toggle_the_same_control_and_disabled_blocks_rows(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, |_, _| Fixture::default());
    click(cx, "base-checkbox");
    assert!(view.read_with(cx, |view, _| view.checked));
    for (key, expected) in [("space", false), ("enter", true)] {
        let parsed = gpui::Keystroke::parse(key);
        assert!(parsed.is_ok());
        if let Ok(keystroke) = parsed {
            cx.simulate_event(gpui::KeyDownEvent {
                keystroke: keystroke.clone(),
                is_held: false,
                prefer_character_input: false,
            });
            assert_ne!(
                view.read_with(cx, |view, _| view.checked),
                expected,
                "base activation commits on key up, never a synthetic key-down click"
            );
            cx.simulate_event(gpui::KeyUpEvent { keystroke });
        }
        settle(cx);
        assert_eq!(view.read_with(cx, |view, _| view.checked), expected);
    }
    view.update(cx, |view, cx| {
        view.checkbox_disabled = true;
        cx.notify();
    });
    click(cx, "base-checkbox");
    assert!(view.read_with(cx, |view, _| view.checked));
    assert_eq!(view.read_with(cx, |view, _| view.ancestor_activations), 0);
}

#[gpui::test]
fn real_wizard_loader_group_changes_only_its_checkpoint_and_skips_unsupported_loaders(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    cx.simulate_resize(size(px(960.), px(600.)));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.preferences.reduce_motion = true;
            view.handle(crate::UiAction::CreateInstance, window, cx);
            view.handle(
                crate::UiAction::WizardPage(Some(crate::WizardPage::Loader)),
                window,
                cx,
            );
        });
    });
    settle(cx);
    focus_radio(cx, "Vanilla");
    cx.simulate_keystrokes("right");
    settle(cx);
    assert!(view.read_with(
        cx,
        |view, _| matches!(&view.form, Some(crate::Form::Instance(form)) if form.neoforge)
    ));
    cx.simulate_keystrokes("end");
    settle(cx);
    assert!(view.read_with(
        cx,
        |view, _| matches!(&view.form, Some(crate::Form::Instance(form)) if form.neoforge)
    ));
    assert!(view.read_with(cx, |view, _| view.snapshot.instances.is_empty()));
    click(cx, "wizard-back");
    assert!(view.read_with(
        cx,
        |view, _| matches!(&view.form, Some(crate::Form::Instance(form)) if !form.neoforge)
    ));
}
#[gpui::test]
fn profile_default_account_and_target_groups_remain_drafts_until_the_main_save(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    cx.simulate_resize(size(px(960.), px(600.)));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.preferences.reduce_motion = true;
            view.snapshot = crate::test_support::ready_snapshot();
            let mut target = view.snapshot.instances[0].targets[0].clone();
            target.id = "second-target".into();
            view.snapshot.instances[0].targets.push(target);
            let account = crate::test_support::test_account();
            assert!(account.is_ok(), "{account:?}");
            if let Ok(account) = account {
                view.accounts = vec![account];
            }
            view.initialize_browsers(cx);
            view.handle(crate::UiAction::Tab(crate::Tab::Instances), window, cx);
            view.handle(crate::UiAction::EditProfile, window, cx);
            view.handle(
                crate::UiAction::ProfilePage(crate::profile_editor::EditorPage::Data),
                window,
                cx,
            );
        });
    });
    settle(cx);
    focus_radio(cx, "Use current account");
    cx.simulate_keystrokes("down");
    settle(cx);
    assert!(view.read_with(cx, |view, _| matches!(&view.form, Some(crate::Form::Profile(form)) if form.original.default_account.is_some())));
    assert_eq!(
        view.read_with(cx, |view, _| view.snapshot.profiles[0]
            .default_account
            .clone()),
        None
    );
    click(cx, "cancel-profile-page");
    assert!(view.read_with(cx, |view, _| matches!(&view.form, Some(crate::Form::Profile(form)) if form.original.default_account.is_none())));
    click(cx, "edit-profile-launch");
    settle(cx);
    let body = cx.debug_bounds("profile-field-Memory");
    assert!(body.is_some());
    if let Some(body) = body {
        cx.simulate_event(gpui::ScrollWheelEvent {
            position: body.center(),
            delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-600.))),
            modifiers: gpui::Modifiers::none(),
            touch_phase: gpui::TouchPhase::Moved,
        });
        settle(cx);
    }
    focus_radio(cx, "vanilla");
    cx.simulate_keystrokes("end");
    settle(cx);
    assert!(view.read_with(cx, |view, _| matches!(&view.form, Some(crate::Form::Profile(form)) if form.original.target_id == "second-target")));
    assert_eq!(
        view.read_with(cx, |view, _| view.snapshot.profiles[0].target_id.clone()),
        "vanilla"
    );
}
