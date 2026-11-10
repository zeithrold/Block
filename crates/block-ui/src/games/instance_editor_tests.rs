//! Compact routing and the root draft guard use the real editor owner.

use crate::{
    Tab, UiAction,
    navigation::{RecordRef, Screen},
    test_support::ready_snapshot,
};
use gpui::{Focusable as _, TestAppContext};

#[gpui::test]
fn untouched_instance_defaults_are_clean_and_raw_edits_are_dirty(cx: &mut TestAppContext) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    let result = cx.update(|window, cx| {
        view.update(cx, |view, cx| -> anyhow::Result<()> {
            view.snapshot = ready_snapshot();
            view.router
                .open_record(Tab::Instances, RecordRef::Instance("instance-a".into()));
            view.handle(UiAction::EditInstance, window, cx);
            let editor = view
                .form
                .as_ref()
                .and_then(|form| match form {
                    crate::Form::EditInstance(form) => Some(form.as_ref()),
                    _ => None,
                })
                .ok_or_else(|| anyhow::anyhow!("missing Instance draft"))?;
            assert!(!view.instance_editor_dirty(cx));
            for field in [&editor.name, &editor.java, &editor.width, &editor.height] {
                let initial = field.read(cx).value();
                field.update(cx, |field, cx| field.set_value(" invalid ", cx));
                assert!(view.instance_editor_dirty(cx));
                field.update(cx, |field, cx| field.set_value(&initial, cx));
                assert!(!view.instance_editor_dirty(cx));
            }
            assert_eq!(
                (
                    view.snapshot.instances[0].width,
                    view.snapshot.instances[0].height
                ),
                (1280, 720)
            );
            Ok(())
        })
    });
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn dirty_editor_prompt_escape_restores_the_actual_text_input_focus(cx: &mut TestAppContext) {
    let result = (|| -> anyhow::Result<()> {
        let (view, cx) = crate::test_support::window(cx, |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.preferences.reduce_motion = true;
            view.snapshot = ready_snapshot();
            view.router
                .open_record(Tab::Instances, RecordRef::Instance("instance-a".into()));
            view
        });
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.handle(UiAction::EditInstance, window, cx);
            })
        });
        cx.run_until_parked();
        let opener = cx.update(|window, cx| {
            view.update(cx, |view, cx| -> anyhow::Result<_> {
                let editor = view
                    .form
                    .as_ref()
                    .and_then(|form| match form {
                        crate::Form::EditInstance(form) => Some(form.as_ref()),
                        _ => None,
                    })
                    .ok_or_else(|| anyhow::anyhow!("missing Instance draft"))?;
                editor
                    .name
                    .update(cx, |field, cx| field.set_value("Native QA draft", cx));
                let opener = editor.name.read(cx).focus_handle(cx);
                opener.focus(window, cx);
                assert!(opener.is_focused(window));
                Ok(opener)
            })
        })?;
        #[derive(Clone, Copy, Debug)]
        enum Origin {
            Back,
            NativeClose,
            ApplicationQuit,
        }
        for origin in [Origin::Back, Origin::NativeClose, Origin::ApplicationQuit] {
            match origin {
                Origin::Back => crate::test_support::press(cx, "alt-left"),
                Origin::NativeClose => {
                    let allowed = cx.update(|window, cx| {
                        view.update(cx, |view, cx| view.allow_native_window_close(window, cx))
                    });
                    assert!(!allowed, "a native close must retain the dirty editor");
                }
                Origin::ApplicationQuit => {
                    let allowed = cx.update(|window, cx| {
                        view.update(cx, |view, cx| view.allow_application_quit(window, cx))
                    });
                    assert!(!allowed, "application quit must retain the dirty editor");
                }
            }
            assert!(
                view.read_with(cx, |view, _| {
                    let Some(crate::shell::Surface::EditorExit(exit)) =
                        view.shell.surfaces.top().map(|layer| &layer.kind)
                    else {
                        return false;
                    };
                    match origin {
                        Origin::Back => matches!(exit.exit.as_ref(), UiAction::NavigationBack),
                        Origin::NativeClose => matches!(exit.exit.as_ref(), UiAction::CloseWindow),
                        Origin::ApplicationQuit => {
                            matches!(exit.exit.as_ref(), UiAction::QuitApplication)
                        }
                    }
                }),
                "{origin:?} must preserve the exact requested exit"
            );
            crate::test_support::press(cx, "escape");
            cx.run_until_parked();
            cx.update(|window, _| {
                assert!(
                    opener.is_focused(window),
                    "{origin:?}: Escape must restore the actual field, not the page fallback"
                )
            });
            assert!(view.read_with(cx, |view, cx| {
                view.form
                    .as_ref()
                    .and_then(|form| match form {
                        crate::Form::EditInstance(form) => Some(form.as_ref()),
                        _ => None,
                    })
                    .is_some_and(|editor| editor.name.read(cx).value() == "Native QA draft")
            }));
        }
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn cancelling_an_instance_editor_returns_to_its_actual_linked_parent(cx: &mut TestAppContext) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.snapshot = ready_snapshot();
            view.router
                .open_record(Tab::Instances, RecordRef::Instance("instance-b".into()));
            view.router
                .open_record(Tab::Instances, RecordRef::Instance("instance-a".into()));
            view.handle(UiAction::EditInstance, window, cx);
            assert_eq!(
                view.router.screen(),
                &Screen::InstanceEditor("instance-a".into())
            );
            view.cancel_instance_editor(window, cx);
            assert_eq!(
                view.router.screen(),
                &Screen::Record(RecordRef::Instance("instance-a".into()))
            );
            view.handle(UiAction::NavigationBack, window, cx);
            assert_eq!(
                view.router.screen(),
                &Screen::Record(RecordRef::Instance("instance-b".into()))
            );
            assert_eq!(view.snapshot.instances[0].name, "Test game");
        });
    });
}
