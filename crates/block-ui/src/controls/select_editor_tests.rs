use super::{Block, Surface, Tab, UiAction, click, row_text, select, settle};
use crate::Form;
use block_launcher::catalog::DataPolicy;
use gpui::{Focusable as _, px, size};

pub(super) fn open_editor(
    cx: &mut gpui::TestAppContext,
    action: UiAction,
) -> (gpui::Entity<Block>, &mut gpui::VisualTestContext) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    cx.simulate_resize(size(px(960.), px(700.)));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.preferences.reduce_motion = true;
            view.router.navigate(Tab::Instances);
            view.snapshot = crate::test_support::ready_snapshot();
            view.handle(UiAction::Inspect("instance-a".into()), window, cx);
            view.handle(action, window, cx);
        });
    });
    settle(cx);
    (view, cx)
}

#[gpui::test]
fn profile_selects_commit_only_the_page_draft_and_cancel_restores_its_checkpoint(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = open_editor(cx, UiAction::EditInstance);
    click(cx, "edit-instance-data");
    click(cx, "data-policy");
    assert_eq!(row_text(cx, "data-policy").len(), 2);
    assert_eq!(select(cx, "data-policy").expanded(), Some(true));
    crate::test_support::press(cx, "end enter");
    assert!(view.read_with(cx, |view, _| matches!(&view.form, Some(Form::EditInstance(form)) if form.original.data_policy == DataPolicy::Shared) && view.snapshot.instances[0].data_policy == DataPolicy::Isolated));
    click(cx, "cancel-instance-page");
    assert!(view.read_with(cx, |view, _| matches!(&view.form, Some(Form::EditInstance(form)) if form.original.data_policy == DataPolicy::Isolated)));
    click(cx, "edit-instance-launch");
    click(cx, "instance-runtime");
    crate::test_support::press(cx, "end enter");
    assert!(view.read_with(
        cx,
        |view, _| matches!(&view.form, Some(Form::EditInstance(form)) if !form.managed)
    ));
    cx.update(|window, cx| {
        view.read_with(cx, |view, cx| {
            assert!(matches!(&view.form, Some(Form::EditInstance(form)) if form.java.read(cx).focus_handle(cx).is_focused(window)));
        });
    });
    view.update(cx, |view, cx| {
        if let Some(Form::EditInstance(form)) = &view.form {
            form.java
                .update(cx, |input, cx| input.set_value("/usr/bin/java", cx));
        }
    });
    click(cx, "instance-runtime");
    crate::test_support::press(cx, "home enter");
    assert!(view.read_with(cx, |view, cx| matches!(&view.form, Some(Form::EditInstance(form)) if form.managed && form.java.read(cx).value(cx) == "/usr/bin/java")));
    click(cx, "cancel-instance-page");
    assert!(view.read_with(cx, |view, cx| matches!(&view.form, Some(Form::EditInstance(form)) if form.managed && form.java.read(cx).value(cx).is_empty())));
}

#[gpui::test]
fn instance_selects_preserve_custom_input_validate_real_values_and_discard_on_cancel(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = open_editor(cx, UiAction::EditInstance);
    click(cx, "edit-instance-launch");
    click(cx, "instance-runtime");
    crate::test_support::press(cx, "end enter");
    assert_eq!(
        view.read_with(cx, |view, _| view.instance_runtime_automatic()),
        Some(false)
    );
    cx.update(|window, cx| {
        view.update(cx, |view, cx| view.save_instance_draft(window, cx));
    });

    // Selecting automatic uses the managed runtime; no unavailable version is advertised.
    click(cx, "instance-runtime");
    assert_eq!(row_text(cx, "instance-runtime").len(), 2);
    crate::test_support::press(cx, "home enter");
    click(cx, "instance-window");
    crate::test_support::press(cx, "home enter");
    assert_eq!(
        view.read_with(cx, Block::instance_window_values),
        Some(("854".into(), "480".into(), false))
    );
    click(cx, "instance-window");
    crate::test_support::press(cx, "end enter");
    assert_eq!(
        view.read_with(cx, Block::instance_window_values),
        Some(("854".into(), "480".into(), true))
    );
    settle(cx);

    click(cx, "done-instance-page");
    click(cx, "cancel-instance");
    assert!(view.read_with(cx, |view, _| {
        matches!(view.form, Some(Form::EditInstance(_)))
            && matches!(
                view.shell.surfaces.top().map(|layer| &layer.kind),
                Some(Surface::EditorExit(_))
            )
            && view.snapshot.instances[0].width == 1280
            && view.snapshot.instances[0].height == 720
    }));
    click(cx, "editor-exit-discard");
    assert!(view.read_with(cx, |view, _| view.form.is_none()
        && view.snapshot.instances[0].width == 1280
        && view.snapshot.instances[0].height == 720));
}
