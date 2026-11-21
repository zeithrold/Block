use super::{Tab, UiAction, click, row_text, select, settle};
use crate::Language;
use gpui::{px, size};

#[gpui::test]
fn settings_language_select_previews_cancels_and_saves(cx: &mut gpui::TestAppContext) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let (engine, messages) = crate::Engine::with_root(root.path().to_path_buf())?;
        let (view, cx) = crate::test_support::window(cx, |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.preferences.reduce_motion = true;
            view.engine = Some(engine);
            view.messages = Some(messages);
            view
        });
        cx.simulate_resize(size(px(1152.), px(720.)));
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.handle(UiAction::Tab(Tab::Settings), window, cx)
            });
        });
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.handle(UiAction::SettingsPage(0), window, cx)
            });
        });
        click(cx, "settings-language");
        assert_eq!(
            row_text(cx, "settings-language"),
            ["English", "简体中文", "日本語"]
        );
        assert_eq!(select(cx, "settings-language").value(), Some("English"));
        click(cx, "select-choice-2");
        assert!(view.read_with(cx, |view, _| {
            view.preferences.language == Some(Language::Japanese)
                && view.saved_preferences.language.is_none()
        }));
        assert_eq!(crate::Preferences::load(root.path())?.language, None);

        click(cx, "settings-language");
        click(cx, "select-choice-1");
        assert_eq!(
            view.read_with(cx, |view, _| view.preferences.language),
            Some(Language::SimplifiedChinese)
        );
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.handle(UiAction::Tab(Tab::Home), window, cx)
            });
        });
        click(cx, "settings-exit-discard");
        assert_eq!(view.read_with(cx, |view, _| view.router.tab()), Tab::Home);
        assert_eq!(
            view.read_with(cx, |view, _| view.preferences.language),
            None
        );

        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.handle(UiAction::Tab(Tab::Settings), window, cx);
                view.handle(UiAction::SettingsPage(0), window, cx);
            });
        });
        click(cx, "settings-language");
        click(cx, "select-choice-1");
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.handle(UiAction::SavePreferences, window, cx)
            });
        });
        assert_eq!(
            crate::Preferences::load(root.path())?.language,
            Some(Language::SimplifiedChinese)
        );
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn settings_runtime_window_and_proxy_use_real_choices_without_saving_draft_early(
    cx: &mut gpui::TestAppContext,
) {
    // This tests draft semantics; the dialog entrance uses a wall clock that
    // settle's virtual clock cannot advance before the physical discard click.
    let _system_motion = crate::platform::test_system_motion(true);
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.handle(UiAction::Tab(Tab::Settings), window, cx);
            view.handle(UiAction::SettingsPage(1), window, cx);
        });
    });
    click(cx, "settings-default-java");
    assert_eq!(
        row_text(cx, "settings-default-java"),
        ["Automatic Java", "Custom executable"]
    );
    crate::test_support::press(cx, "end enter");
    assert!(
        view.read_with(cx, |view, cx| view.settings_form.as_ref().is_some_and(
            |draft| !draft.java_automatic && draft.is_dirty(&view.saved_preferences, cx)
        ))
    );
    cx.update(|window, cx| view.update(cx, |view, cx| view.save_settings(window, cx)));
    assert!(
        view.read_with(cx, |view, _| view.settings_form.as_ref().is_some_and(
            |draft| draft
                .error
                .as_ref()
                .is_some_and(|error| error.field == Some(crate::settings::SettingsField::Java))
        ))
    );
    view.update(cx, |view, cx| {
        if let Some(draft) = &view.settings_form {
            draft
                .java
                .update(cx, |input, cx| input.set_value("/usr/bin/java", cx));
        }
    });
    click(cx, "settings-default-java");
    crate::test_support::press(cx, "home enter");
    assert!(
        view.read_with(cx, |view, cx| view.settings_form.as_ref().is_some_and(
            |draft| draft.java_automatic
                && draft.java.read(cx).value() == "/usr/bin/java"
                && draft
                    .parse(&view.saved_preferences, cx)
                    .is_ok_and(|preferences| preferences.game.java.is_empty())
        ))
    );
    click(cx, "settings-default-window");
    crate::test_support::press(cx, "home enter");
    assert!(
        view.read_with(cx, |view, cx| view.settings_form.as_ref().is_some_and(
            |draft| draft.width.read(cx).value() == "854"
                && draft.height.read(cx).value() == "480"
                && !draft.window_custom
        ))
    );
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.handle(UiAction::SettingsPage(3), window, cx);
        });
    });
    click(cx, "settings-proxy");
    crate::test_support::press(cx, "down enter");
    assert!(view.read_with(cx, |view, _| {
        view.settings_form
            .as_ref()
            .is_some_and(|draft| draft.proxy_mode == 1)
            && view.saved_preferences == crate::Preferences::default()
    }));
    click(cx, "settings-proxy");
    crate::test_support::press(cx, "end enter");
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            if let Some(draft) = &view.settings_form {
                draft.proxy_url.update(cx, |input, cx| {
                    input.set_value("ftp://127.0.0.1:7890", cx);
                });
            }
            view.save_settings(window, cx);
        });
    });
    assert!(
        view.read_with(cx, |view, cx| view.settings_form.as_ref().is_some_and(
            |draft| draft.proxy_mode == 2
                && draft.proxy_url.read(cx).value() == "ftp://127.0.0.1:7890"
                && draft.error.as_ref().is_some_and(
                    |error| error.field == Some(crate::settings::SettingsField::ProxyUrl)
                )
        ) && view.saved_preferences
            == crate::Preferences::default())
    );
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.handle(UiAction::Tab(Tab::Home), window, cx);
        });
    });
    let discard = click(cx, "settings-exit-discard");
    let state = view.read_with(cx, |view, _| {
        (
            view.router.tab(),
            view.settings_form.is_none(),
            view.preferences == view.saved_preferences,
            view.shell.surfaces.is_open(),
        )
    });
    let viewport = cx.update(|window, _| window.viewport_size());
    assert_eq!(
        state,
        (Tab::Home, true, true, false),
        "discard={discard:?}, viewport={viewport:?}"
    );
}

#[gpui::test]
fn busy_settings_choice_commits_only_its_ui_draft_and_keeps_the_task_busy(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.handle(UiAction::Tab(Tab::Settings), window, cx);
            view.handle(UiAction::SettingsPage(0), window, cx);
            view.busy = true;
            cx.notify();
        });
    });
    click(cx, "settings-appearance");
    crate::test_support::press(cx, "end enter");
    settle(cx);
    assert_eq!(select(cx, "settings-appearance").expanded(), Some(false));
    assert!(view.read_with(cx, |view, _| view.busy
        && view.preferences.appearance == crate::Appearance::Dark
        && view.saved_preferences.appearance == crate::Appearance::System
        && !view.shell.surfaces.is_open()));
}
