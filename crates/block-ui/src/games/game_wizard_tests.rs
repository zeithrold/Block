//! Product outcomes for drafts, catalog choices, and wizard boundaries.

use block_launcher::catalog::{ProfileColor, ProfileIcon, ProfileMode};
use gpui::{EntityInputHandler as _, Focusable as _, Modifiers, TestAppContext, point, px, size};

use super::{CreationKind, WizardFailure, WizardField, WizardPage, WizardState, WizardStep};
use crate::{Form, Language, test_support::ready_snapshot};

/// The wizard error painted in English. The wizard keeps product causes and text
/// a shared owner rendered apart, so tests read the resolved message.
fn error_text(view: &crate::Block) -> Option<String> {
    view.shell.wizard.error_text(Language::English)
}

fn compatible_optifine() -> block_launcher::install::optifine::OptiFineJar {
    block_launcher::install::optifine::OptiFineJar {
        minecraft: "1.21.1".into(),
        version: "1.21.1_HD_U_J1".into(),
        launchwrapper: "2.3".into(),
        forge: Some("52.0.16".into()),
        resource: block_launcher::Resource {
            sha1: "a".repeat(40),
            size: 123,
            url: "block-import:synthetic".into(),
        },
    }
}

#[gpui::test]
fn optifine_import_is_draft_bound_revalidates_late_results_and_cancels_without_saving(
    cx: &mut TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    view.update(cx, |view, cx| {
        let selected = compatible_optifine();
        let count = view.snapshot.instances.len();
        view.begin_game_wizard(cx);
        view.set_wizard_version("1.21.1".into(), cx);
        view.open_game_wizard_page(Some(WizardPage::OptiFine), cx);
        let field = match &view.form {
            Some(Form::Instance(form)) => form.version.entity_id(),
            _ => return,
        };
        let old = uuid::Uuid::new_v4();
        view.shell.wizard.optifine_request = Some(old);
        view.cancel_component_draft(cx);
        view.finish_optifine_import(old, field, Ok(selected.clone()), cx);
        assert!(matches!(&view.form, Some(Form::Instance(form)) if form.optifine.is_none()));
        view.open_game_wizard_page(Some(WizardPage::OptiFine), cx);
        let current = uuid::Uuid::new_v4();
        view.shell.wizard.optifine_request = Some(current);
        view.finish_optifine_import(old, field, Ok(selected.clone()), cx);
        assert_eq!(view.shell.wizard.optifine_request, Some(current));
        view.finish_optifine_import(current, field, Ok(selected.clone()), cx);
        assert!(matches!(&view.form, Some(Form::Instance(form)) if form.optifine.is_some()));
        assert_eq!(view.snapshot.instances.len(), count);
        view.cancel_component_draft(cx);
        assert!(matches!(&view.form, Some(Form::Instance(form)) if form.optifine.is_none()));
        view.open_game_wizard_page(Some(WizardPage::OptiFine), cx);
        let changed = uuid::Uuid::new_v4();
        view.shell.wizard.optifine_request = Some(changed);
        view.set_wizard_version("1.20.6".into(), cx);
        view.finish_optifine_import(changed, field, Ok(selected), cx);
        assert!(matches!(&view.form, Some(Form::Instance(form)) if form.optifine.is_none()));
        assert!(
            error_text(view)
                .as_deref()
                .is_some_and(|error| error.contains("1.21.1"))
        );
        assert_eq!(view.snapshot.instances.len(), count);
    });
}

fn click(cx: &mut gpui::VisualTestContext, selector: &'static str) {
    cx.run_until_parked();
    let bounds = crate::test_support::control_bounds(cx, selector);
    assert!(bounds.is_some(), "required control is missing: {selector}");
    if let Some(bounds) = bounds {
        let window_count = cx.windows().len();
        cx.simulate_click(bounds.center(), Modifiers::none());
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
}

#[gpui::test]
fn optifine_chooser_failure_preserves_the_wizard_and_ignores_stale_requests(
    cx: &mut TestAppContext,
) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let (engine, messages) = crate::engine::Engine::with_root(root.path().to_path_buf())?;
        let (view, cx) = crate::test_support::window(cx, |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.preferences.reduce_motion = true;
            view.engine = Some(engine);
            view.messages = Some(messages);
            view.begin_game_wizard(cx);
            view
        });
        cx.simulate_resize(size(px(1100.), px(1300.)));
        click(cx, "wizard-component-optifine");
        click(cx, "optifine-select-jar");
        assert!(cx.did_prompt_for_paths());
        let (request, field) = view.read_with(cx, |view, _| {
            let request = view.shell.wizard.optifine_request;
            let field = match &view.form {
                Some(Form::Instance(form)) => Some(form.version.entity_id()),
                _ => None,
            };
            (request, field)
        });
        let (Some(request), Some(field)) = (request, field) else {
            anyhow::bail!("missing pending chooser");
        };
        view.update(cx, |view, cx| {
            assert!(
                view.finish_optifine_browse(request, field, Err(()), cx)
                    .is_none()
            );
        });
        cx.simulate_path_prompt_response(|_| None);
        cx.run_until_parked();
        assert!(crate::test_support::control_bounds(cx, "wizard-optifine-error").is_some());
        assert!(view.read_with(cx, |view, _| {
            error_text(view)
                .as_deref()
                .is_some_and(|error| error.contains("file chooser"))
                && view.shell.wizard.optifine_request.is_none()
                && matches!(&view.form, Some(Form::Instance(form)) if form.optifine.is_none())
        }));

        click(cx, "optifine-select-jar");
        let old = view.read_with(cx, |view, _| view.shell.wizard.optifine_request);
        assert!(old.is_some());
        click(cx, "section-cancel");
        click(cx, "wizard-component-optifine");
        click(cx, "optifine-select-jar");
        let current = view.read_with(cx, |view, _| view.shell.wizard.optifine_request);
        assert!(current.is_some() && current != old);
        view.update(cx, |view, cx| {
            if let Some(old) = old {
                assert!(
                    view.finish_optifine_browse(old, field, Err(()), cx)
                        .is_none()
                );
            }
            assert_eq!(view.shell.wizard.optifine_request, current);
            assert!(error_text(view).is_none());
        });
        cx.simulate_path_prompt_response(|_| None);
        cx.run_until_parked();
        assert_eq!(
            view.read_with(cx, |view, _| view.shell.wizard.optifine_request),
            current
        );
        cx.simulate_path_prompt_response(|_| None);
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| {
            view.shell.wizard.optifine_request.is_none() && error_text(view).is_none()
        }));
        click(cx, "optifine-select-jar");
        let request = view.read_with(cx, |view, _| view.shell.wizard.optifine_request);
        let Some(request) = request else {
            anyhow::bail!("missing pending chooser");
        };
        let path = root.path().join("selected.jar");
        view.update(cx, |view, cx| {
            assert_eq!(
                view.finish_optifine_browse(request, field, Ok(Some(path.clone())), cx),
                Some(path)
            );
            view.finish_optifine_import(request, field, Err(anyhow::anyhow!("invalid JAR")), cx);
        });
        cx.simulate_path_prompt_response(|_| None);
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| {
            view.shell.wizard.optifine_request.is_none()
                && error_text(view)
                    .as_deref()
                    .is_some_and(|error| error.contains("Could not import OptiFine"))
                && matches!(&view.form, Some(Form::Instance(form)) if form.optifine.is_none())
        }));
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

fn entry(
    id: &str,
    kind: block_launcher::manifest::model::VersionType,
) -> block_launcher::manifest::model::VersionEntry {
    // Synthetic entries carry plausible release times: the ARM64 version policy
    // classifies LWJGL 2-era versions by the official catalog times.
    let release_time = match id {
        "1.12.2" => "2017-09-18T11:33:40+00:00",
        "b1.7.3" | "a1.2.6" => "2011-06-30T00:00:00+00:00",
        _ => "2024-01-01T00:00:00+00:00",
    };
    block_launcher::manifest::model::VersionEntry {
        id: id.into(),
        kind,
        url: "https://example.invalid/manifest".into(),
        time: release_time.into(),
        release_time: release_time.into(),
        sha1: "0".repeat(40),
        compliance_level: 1,
    }
}

#[test]
fn bundled_and_separated_steps_keep_their_own_back_stack() {
    let mut bundled = WizardState::default();
    bundled.back();
    assert_eq!(bundled.step, WizardStep::Setup);
    bundled.next();
    assert_eq!(bundled.step, WizardStep::Review);
    bundled.back();
    assert_eq!(bundled.step, WizardStep::Setup);

    let mut separated = WizardState {
        separated: true,
        step: WizardStep::GetStarted,
        ..WizardState::default()
    };
    for expected in [WizardStep::Setup, WizardStep::Profile, WizardStep::Review] {
        separated.next();
        assert_eq!(separated.step, expected);
    }
    for expected in [
        WizardStep::Profile,
        WizardStep::Setup,
        WizardStep::GetStarted,
    ] {
        separated.back();
        assert_eq!(separated.step, expected);
    }
    separated.back();
    assert_eq!(separated.step, WizardStep::GetStarted);
}

#[gpui::test]
fn wizard_component_cards_keep_two_columns_and_grow_with_their_content(cx: &mut TestAppContext) {
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.preferences.reduce_motion = true;
        view.versions = vec!["1.21.1".into(), "1.20.6".into()];
        view.begin_game_wizard(cx);
        view
    });
    // One value with no ASCII space and full-width glyphs: the longest real
    // value still has to wrap instead of collapsing into one clipped line.
    let long = "1.21.1-neoforge-21.1.172-构建设置与超长版本标识符用于校验换行".to_owned();
    // Two columns are the layout at every window the product opens, so the
    // cards have to stay content-sized at the standard and fitted widths.
    for width in [960., 1152.] {
        view.update(cx, |view, cx| {
            if let Some(Form::Instance(form)) = &mut view.form {
                form.version
                    .update(cx, |field, cx| field.set_value("1.21.1", cx));
            }
            cx.notify();
        });
        cx.simulate_resize(size(px(width), px(900.)));
        cx.run_until_parked();
        let grid = crate::test_support::control_bounds(cx, "wizard-components");
        let version = crate::test_support::control_bounds(cx, "wizard-component-version");
        let loader = crate::test_support::control_bounds(cx, "wizard-component-loader");
        let optifine = crate::test_support::control_bounds(cx, "wizard-component-optifine");
        let location = crate::test_support::control_bounds(cx, "wizard-component-location");
        let value = crate::test_support::control_bounds(cx, "wizard-component-version-value");
        let detail = crate::test_support::control_bounds(cx, "wizard-component-version-detail");
        assert!(
            grid.is_some()
                && version.is_some()
                && loader.is_some()
                && optifine.is_some()
                && location.is_some()
                && value.is_some()
                && detail.is_some(),
            "live wizard cards must be rendered at {width}"
        );
        let (
            Some(grid),
            Some(version),
            Some(loader),
            Some(optifine),
            Some(location),
            Some(value),
            Some(detail),
        ) = (grid, version, loader, optifine, location, value, detail)
        else {
            return;
        };
        // Version -> Loader / OptiFine -> Configuration stay in two equal columns.
        assert!(
            (version.left() - optifine.left()).abs() <= px(1.)
                && (loader.left() - location.left()).abs() <= px(1.),
            "both rows must share the same two columns at {width}"
        );
        assert!(
            (version.size.width - loader.size.width).abs() <= px(1.)
                && (loader.size.width - optifine.size.width).abs() <= px(1.),
            "cards must split the grid width equally at {width}"
        );
        let first_row = version.size.height.max(loader.size.height);
        let second_row = optifine.size.height.max(location.size.height);
        assert!(
            (loader.left() - version.right()).abs() <= px(13.)
                && (optifine.top() - (version.top() + first_row)).abs() <= px(13.),
            "cards must keep the 12 px grid gap at {width}"
        );
        assert!(
            version.top() < optifine.top() && loader.top() < location.top(),
            "Version and Loader must lead the first row at {width}"
        );
        for card in [version, loader, optifine, location] {
            assert!(
                card.size.height >= px(96.),
                "cards must keep their 96 px minimum height at {width}: {card:?}"
            );
        }
        assert!(
            (grid.size.height - (first_row + second_row + px(12.))).abs() <= px(1.),
            "the grid must reserve only its two content rows at {width}: grid={grid:?}, version={version:?}, optifine={optifine:?}"
        );
        // The value and its detail sit inside the card instead of being clipped.
        assert!(
            value.top() >= version.top()
                && value.bottom() <= detail.top() + px(1.)
                && detail.bottom() <= version.bottom() + px(1.)
                && value.left() >= version.left()
                && value.right() <= version.right() + px(1.),
            "a summary value must stay readable inside its card at {width}: value={value:?}, detail={detail:?}, card={version:?}"
        );
        // A longer value wraps into more lines and grows its row. A square or
        // fixed-height card cannot do that, and the unchanged second row proves
        // the height comes from content rather than from the column width.
        view.update(cx, |view, cx| {
            if let Some(Form::Instance(form)) = &mut view.form {
                form.version
                    .update(cx, |field, cx| field.set_value(&long, cx));
            }
            cx.notify();
        });
        cx.run_until_parked();
        let tall_version = crate::test_support::control_bounds(cx, "wizard-component-version");
        let tall_value = crate::test_support::control_bounds(cx, "wizard-component-version-value");
        let tall_detail =
            crate::test_support::control_bounds(cx, "wizard-component-version-detail");
        let tall_optifine = crate::test_support::control_bounds(cx, "wizard-component-optifine");
        let tall_location = crate::test_support::control_bounds(cx, "wizard-component-location");
        assert!(
            tall_version.is_some()
                && tall_value.is_some()
                && tall_detail.is_some()
                && tall_optifine.is_some()
                && tall_location.is_some(),
            "a long value must keep rendering at {width}"
        );
        let (
            Some(tall_version),
            Some(tall_value),
            Some(tall_detail),
            Some(tall_optifine),
            Some(tall_location),
        ) = (
            tall_version,
            tall_value,
            tall_detail,
            tall_optifine,
            tall_location,
        )
        else {
            return;
        };
        assert!(
            tall_value.size.height >= value.size.height + px(20.),
            "a long value must wrap instead of staying one clipped line at {width}: before={:?}, after={:?}",
            value.size.height,
            tall_value.size.height
        );
        assert!(
            tall_version.size.height >= version.size.height + px(20.),
            "the card must grow with the wrapped value at {width}: before={:?}, after={:?}",
            version.size.height,
            tall_version.size.height
        );
        assert!(
            (tall_optifine.size.height.max(tall_location.size.height) - second_row).abs() <= px(1.),
            "only the row holding the long value may grow at {width}"
        );
        assert!(
            tall_value.right() <= tall_version.right() + px(1.)
                && tall_detail.bottom() <= tall_version.bottom() + px(1.),
            "a wrapped long value must stay inside its card at {width}: value={tall_value:?}, card={tall_version:?}"
        );
        assert!(view.read_with(cx, |view, _| view.shell.wizard.page.is_none()));
    }
}

#[gpui::test]
fn review_leads_with_identity_and_keeps_unchanged_defaults_folded(cx: &mut TestAppContext) {
    let (view, cx) = review_page(cx);
    // Identity, then what Create does, then the runtime summary, then the fold.
    assert!(
        !view.read_with(cx, |view, _| view.shell.wizard.advanced.borrow().is_open()),
        "review details must start collapsed"
    );
    let identity = crate::test_support::control_bounds(cx, "wizard-review-identity");
    let consequence = crate::test_support::control_bounds(cx, "wizard-review-consequence");
    let summary = crate::test_support::control_bounds(cx, "wizard-review-summary");
    let options = crate::test_support::control_bounds(cx, "wizard-review-options");
    let short_target = crate::test_support::control_bounds(cx, "wizard-review-launch-target");
    assert!(
        identity.is_some()
            && consequence.is_some()
            && summary.is_some()
            && options.is_some()
            && short_target.is_some(),
        "the review must render its identity, consequence, summary and fold"
    );
    assert!(
        short_target.is_some_and(|target| target.size.height == px(20.)),
        "the ordinary short target must keep its compact badge: {short_target:?}"
    );
    let (Some(identity), Some(consequence), Some(summary), Some(options)) =
        (identity, consequence, summary, options)
    else {
        return;
    };
    assert!(
        identity.bottom() <= consequence.top()
            && consequence.bottom() <= summary.top()
            && summary.bottom() <= options.top(),
        "review order must be identity, consequence, runtime summary, then details: \
         identity={identity:?}, consequence={consequence:?}, summary={summary:?}, options={options:?}"
    );
    // Java and memory lead the visible summary while the fold is still closed.
    let java_fact = crate::test_support::control_bounds(cx, "wizard-review-java");
    let memory_fact = crate::test_support::control_bounds(cx, "wizard-review-memory");
    assert!(
        java_fact.is_some() && memory_fact.is_some(),
        "the runtime summary must always show Java and memory"
    );
    for fact in [java_fact, memory_fact].into_iter().flatten() {
        assert!(
            fact.bottom() <= options.top(),
            "a runtime fact must precede the folded details: {fact:?}"
        );
    }
    // Values that decide nothing stay inside the collapsed fold: they lay out
    // below its trigger and are only revealed by opening it.
    for selector in [
        "wizard-review-server",
        "wizard-review-automatic-connection",
        "wizard-review-jvm-arguments",
        "wizard-review-game-window",
        "wizard-review-game-files",
    ] {
        let fact = crate::test_support::control_bounds(cx, selector);
        assert!(
            fact.is_some(),
            "the fold must still describe the unchanged default: {selector}"
        );
        if let Some(fact) = fact {
            assert!(
                fact.top() >= options.bottom(),
                "an unchanged default must stay inside the fold: {selector}={fact:?}"
            );
        }
    }
}

#[gpui::test]
fn review_shows_changed_settings_outside_the_fold_and_keeps_its_draft_across_steps(
    cx: &mut TestAppContext,
) {
    let directory = std::env::temp_dir().join("block-review-game");
    let directory = directory.to_string_lossy().into_owned();
    let java = std::env::temp_dir().join(format!("java{}", std::env::consts::EXE_SUFFIX));
    let java = java.to_string_lossy().into_owned();
    let (view, cx) = review_page(cx);
    // A deliberate override is visible without opening the fold.
    view.update(cx, |view, cx| {
        if let Some(Form::Instance(form)) = &mut view.form {
            form.directory
                .update(cx, |field, cx| field.set_value(&directory, cx));
        }
        if let Some(profile) = &mut view.shell.wizard.profile {
            profile.managed = false;
            profile
                .java
                .update(cx, |field, cx| field.set_value(&java, cx));
            profile
                .jvm
                .update(cx, |field, cx| field.set_value("-XX:+UseG1GC", cx));
            profile.server.update(cx, |field, cx| {
                field.set_value("play.example.com:25565", cx)
            });
            profile.fullscreen = true;
        }
        cx.notify();
    });
    cx.run_until_parked();
    assert!(
        !view.read_with(cx, |view, _| view.shell.wizard.advanced.borrow().is_open()),
        "a visible summary must not depend on the fold being open"
    );
    let collapsed = crate::test_support::control_bounds(cx, "wizard-review-options");
    assert!(collapsed.is_some(), "the review must keep its fold trigger");
    if let Some(collapsed) = collapsed {
        for selector in [
            "wizard-review-server",
            "wizard-review-jvm-arguments",
            "wizard-review-game-window",
            "wizard-review-game-files",
        ] {
            let fact = crate::test_support::control_bounds(cx, selector);
            assert!(
                fact.is_some(),
                "a changed setting must stay visible in the review summary: {selector}"
            );
            if let Some(fact) = fact {
                assert!(
                    fact.bottom() <= collapsed.top(),
                    "a changed setting must not move into the fold: {selector}={fact:?}"
                );
            }
        }
    }
    // The fold reuses one draft's expansion state rather than a new one.
    click(cx, "wizard-review-options");
    assert!(
        view.read_with(cx, |view, _| view.shell.wizard.advanced.borrow().is_open()),
        "opening the disclosure must reveal the folded defaults"
    );
    assert!(
        view.read_with(cx, |view, _| {
            view.shell
                .wizard
                .profile
                .as_ref()
                .is_some_and(|profile| profile.options)
        }),
        "the disclosure must keep using the existing profile options state"
    );
    // Returning to setup and advancing again must keep the same draft values.
    click(cx, "wizard-back");
    assert_eq!(
        view.read_with(cx, |view, _| view.shell.wizard.step),
        WizardStep::Setup
    );
    assert!(
        view.read_with(cx, |view, _| view.shell.wizard.advanced.borrow().is_open()),
        "one draft keeps its expansion across Setup and Review"
    );
    assert!(view.read_with(cx, |view, cx| {
        view.shell.wizard.profile.as_ref().is_some_and(|profile| {
            profile.jvm.read(cx).value() == "-XX:+UseG1GC"
                && !profile.managed
                && profile.fullscreen
        }) && matches!(&view.form, Some(Form::Instance(form)) if form.directory.read(cx).value() == directory)
    }));
    click(cx, "wizard-next");
    assert_eq!(
        view.read_with(cx, |view, _| view.shell.wizard.step),
        WizardStep::Review
    );
    assert!(
        view.read_with(cx, |view, _| !view.shell.wizard.advanced.borrow().is_open()),
        "arriving at the review must fold the defaults again"
    );
    assert!(view.read_with(cx, |view, _| error_text(view).is_none()));
    assert!(crate::test_support::control_bounds(cx, "wizard-review-server").is_some());
    assert_eq!(
        view.read_with(cx, |view, _| view.snapshot.instances.len()),
        1
    );
}

#[gpui::test]
fn review_keeps_a_long_launch_target_inside_the_identity_card(cx: &mut TestAppContext) {
    let (view, cx) = review_page(cx);
    // Review a real, compatible combination whose complete identity is wider
    // than one review fact column.
    view.update(cx, |view, cx| {
        let selected = compatible_optifine();
        let loader = block_launcher::catalog::LoaderKind::Forge;
        assert!(
            matches!(&view.form, Some(Form::Instance(_))),
            "the Review must retain the instance draft"
        );
        if let Some(Form::Instance(form)) = &mut view.form {
            let minecraft = form.version.read(cx).value();
            assert_eq!(minecraft, "1.21.1");
            assert!(
                selected
                    .validate_for(&minecraft, &loader, Some("52.0.16"))
                    .is_ok()
            );
            form.loader = loader;
            form.loader_version = Some("52.0.16".into());
            form.optifine = Some(Box::new(selected));
            assert_eq!(
                form.loader_label(),
                "Forge 52.0.16 + OptiFine 1.21.1_HD_U_J1"
            );
        }
        cx.notify();
    });
    for width in [960., 1152.] {
        cx.simulate_resize(size(px(width), px(1400.)));
        cx.run_until_parked();
        let identity = crate::test_support::control_bounds(cx, "wizard-review-identity");
        let target = crate::test_support::control_bounds(cx, "wizard-review-launch-target");
        assert!(identity.is_some() && target.is_some());
        let (Some(identity), Some(target)) = (identity, target) else {
            return;
        };
        assert!(
            target.left() >= identity.left()
                && target.right() <= identity.right()
                && target.top() >= identity.top()
                && target.bottom() <= identity.bottom(),
            "the complete launch target must stay inside the identity card at {width}: \
             target={target:?}, identity={identity:?}"
        );
    }
}

/// The Review page of a fresh game draft at the wide diagnostic window.
fn review_page(
    cx: &mut TestAppContext,
) -> (gpui::Entity<crate::Block>, &mut gpui::VisualTestContext) {
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.preferences.reduce_motion = true;
        view.versions = vec!["1.21.1".into()];
        view.snapshot = ready_snapshot();
        view.begin_game_wizard(cx);
        view
    });
    cx.simulate_resize(size(px(960.), px(1400.)));
    view.update(cx, |view, cx| {
        if let Some(Form::Instance(form)) = &mut view.form {
            form.name.update(cx, |field, cx| {
                field.set_value("Steam \u{4e16}\u{754c}", cx)
            });
        }
        view.shell.wizard.step = WizardStep::Review;
        view.shell.wizard.page = None;
        cx.notify();
    });
    cx.run_until_parked();
    (view, cx)
}

#[gpui::test]
fn entering_the_review_folds_details_the_setup_disclosure_left_open(cx: &mut TestAppContext) {
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.preferences.reduce_motion = true;
        view.versions = vec!["1.21.1".into()];
        view.snapshot = ready_snapshot();
        view.begin_game_wizard(cx);
        view
    });
    cx.simulate_resize(size(px(960.), px(1400.)));
    cx.run_until_parked();
    // Setup and the Review share the wizard's one expansion state, so a Setup
    // disclosure the user opened would otherwise arrive open in the Review.
    click(cx, "wizard-profile-options");
    assert!(
        view.read_with(cx, |view, _| view.shell.wizard.advanced.borrow().is_open()),
        "the setup disclosure must open before the review could inherit it"
    );
    click(cx, "wizard-next");
    assert_eq!(
        view.read_with(cx, |view, _| view.shell.wizard.step),
        WizardStep::Review
    );
    assert!(
        view.read_with(cx, |view, _| !view.shell.wizard.advanced.borrow().is_open()),
        "the review must lead with the decisions instead of a setup expansion"
    );
    let trigger = crate::test_support::control_bounds(cx, "wizard-review-options");
    let folded = crate::test_support::control_bounds(cx, "wizard-review-game-window");
    assert!(
        trigger.is_some() && folded.is_some(),
        "the folded review must still describe its defaults"
    );
    let (Some(trigger), Some(folded)) = (trigger, folded) else {
        return;
    };
    assert!(
        folded.top() >= trigger.bottom(),
        "the review must keep its unchanged defaults folded: folded={folded:?} \
         trigger={trigger:?}"
    );
}

#[gpui::test]
fn outer_profile_validation_retains_its_language_across_continue_and_submit(
    cx: &mut TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.preferences.reduce_motion = true;
        view.preferences.language = Some(Language::SimplifiedChinese);
        view.versions = vec!["1.21.1".into()];
        view.snapshot = ready_snapshot();
        view.begin_game_wizard(cx);
        view
    });
    cx.simulate_resize(size(px(960.), px(1400.)));
    click(cx, "wizard-profile-options");
    view.update(cx, |view, cx| {
        if let Some(profile) = &view.shell.wizard.profile {
            profile
                .jvm
                .update(cx, |input, cx| input.set_value("\"", cx));
        }
    });
    click(cx, "wizard-next");
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            let error = view.shell.wizard.error.clone();
            assert_eq!(
                error.as_ref().and_then(|error| error.key()),
                Some("common.arguments_unclosed_quote")
            );
            assert_eq!(
                view.shell.wizard.error_text(view.language()).as_deref(),
                Some("请闭合附加参数中的双引号")
            );
            view.handle(crate::UiAction::SetLanguage(Language::Japanese), window, cx);
            assert_eq!(view.shell.wizard.error, error);
            assert_eq!(
                view.shell.wizard.error_text(view.language()).as_deref(),
                Some("追加引数の二重引用符を閉じてください")
            );
            view.wizard_kind(false, cx);
            view.shell.wizard.separated = true;
            view.shell.wizard.step = WizardStep::Profile;
            if let Some(profile) = &view.shell.wizard.profile {
                profile.jvm.update(cx, |input, cx| input.set_value("", cx));
                profile
                    .width
                    .update(cx, |input, cx| input.set_value("100", cx));
            }
            cx.notify();
        });
    });
    click(cx, "wizard-next");
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            let error = view.shell.wizard.error.clone();
            assert_eq!(
                error.as_ref().and_then(|error| error.key()),
                Some("wizard.error.widthRange")
            );
            assert_eq!(
                view.shell.wizard.error_text(view.language()).as_deref(),
                Some("ウィンドウ幅は 320 から 16384 までの整数で入力してください。")
            );
            view.handle(
                crate::UiAction::SetLanguage(Language::SimplifiedChinese),
                window,
                cx,
            );
            assert_eq!(view.shell.wizard.error, error);
            assert_eq!(
                view.shell.wizard.error_text(view.language()).as_deref(),
                Some("窗口宽度必须是 320 到 16384 之间的整数。")
            );
            view.shell.wizard.step = WizardStep::Review;
            view.submit_game_wizard(cx);
            assert_eq!(
                view.shell
                    .wizard
                    .error
                    .as_ref()
                    .and_then(|error| error.key()),
                Some("wizard.error.widthRange")
            );
        });
    });
}

#[gpui::test]
fn review_shows_arguments_and_java_inherited_from_saved_defaults(cx: &mut TestAppContext) {
    let java = std::env::temp_dir().join(format!("java{}", std::env::consts::EXE_SUFFIX));
    let java = java.to_string_lossy().into_owned();
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.preferences.reduce_motion = true;
        view.versions = vec!["1.21.1".into()];
        view.snapshot = ready_snapshot();
        // The saved defaults a new draft inherits: applying them is not a user
        // edit, but Create still submits both values.
        view.saved_preferences.game.jvm_arguments = "-XX:+UseG1GC".into();
        view.saved_preferences.game.java = java;
        view.begin_game_wizard(cx);
        view
    });
    cx.simulate_resize(size(px(960.), px(1400.)));
    view.update(cx, |view, cx| {
        view.shell.wizard.step = WizardStep::Review;
        view.shell.wizard.page = None;
        cx.notify();
    });
    cx.run_until_parked();
    assert!(
        !view.read_with(cx, |view, _| view.shell.wizard.advanced.borrow().is_open()),
        "review details must start collapsed"
    );
    let options = crate::test_support::control_bounds(cx, "wizard-review-options");
    let jvm = crate::test_support::control_bounds(cx, "wizard-review-jvm-arguments");
    let java_fact = crate::test_support::control_bounds(cx, "wizard-review-java");
    assert!(
        options.is_some() && jvm.is_some() && java_fact.is_some(),
        "the review must show Java and its inherited arguments"
    );
    let (Some(options), Some(jvm), Some(java_fact)) = (options, jvm, java_fact) else {
        return;
    };
    assert!(
        jvm.bottom() <= options.top(),
        "inherited arguments are submitted by Create, so they belong to the visible summary: {jvm:?}"
    );
    assert!(
        java_fact.bottom() <= options.top(),
        "the inherited Java executable must stay visible: {java_fact:?}"
    );
}

#[gpui::test]
fn review_shows_a_window_choice_inherited_from_saved_defaults(cx: &mut TestAppContext) {
    // Fullscreen and a non-default size are submitted consequences even when
    // they equal the saved preference, including another built-in size preset.
    for (fullscreen, width, height) in [(true, 1280, 720), (false, 1600, 900), (false, 1920, 1080)]
    {
        let (view, cx) = crate::test_support::window(cx, |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.preferences.reduce_motion = true;
            view.versions = vec!["1.21.1".into()];
            view.snapshot = ready_snapshot();
            view.saved_preferences.game.fullscreen = fullscreen;
            view.saved_preferences.game.width = width;
            view.saved_preferences.game.height = height;
            view.begin_game_wizard(cx);
            view
        });
        cx.simulate_resize(size(px(960.), px(1400.)));
        view.update(cx, |view, cx| {
            view.shell.wizard.step = WizardStep::Review;
            view.shell.wizard.page = None;
            cx.notify();
        });
        cx.run_until_parked();
        assert!(
            !view.read_with(cx, |view, _| view.shell.wizard.advanced.borrow().is_open()),
            "the inherited window choice must be readable without opening the fold"
        );
        let options = crate::test_support::control_bounds(cx, "wizard-review-options");
        let window_fact = crate::test_support::control_bounds(cx, "wizard-review-game-window");
        assert!(
            options.is_some() && window_fact.is_some(),
            "an inherited fullscreen or custom window must stay in the review summary"
        );
        let (Some(options), Some(window_fact)) = (options, window_fact) else {
            return;
        };
        assert!(
            window_fact.bottom() <= options.top(),
            "an inherited window consequence must not move into the fold: window={window_fact:?}, fold={options:?}"
        );
    }
}

fn review_bounds(
    cx: &mut gpui::VisualTestContext,
    selector: &'static str,
    entry: &str,
) -> gpui::Bounds<gpui::Pixels> {
    let bounds = crate::test_support::control_bounds(cx, selector);
    assert!(bounds.is_some(), "{entry}: {selector} must be rendered");
    bounds.unwrap_or_default()
}

/// Give the real `wizard-body` viewport pointer focus by clicking its blank
/// margin, the way `scroll_area_tests` reaches a viewport's keyboard path. The
/// viewport itself owns that focus target, so End and Home scroll the page.
fn focus_wizard_body(cx: &mut gpui::VisualTestContext, entry: &str) {
    let body = review_bounds(cx, "wizard-body", entry);
    let blank = point(body.left() + px(8.), body.center().y);
    cx.simulate_click(blank, Modifiers::none());
    settle(cx);
    assert!(
        cx.update(|window, app| window.focused(app).is_some()),
        "{entry}: clicking the body leaves the viewport's focus target in place"
    );
}

fn settle(cx: &mut gpui::VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

/// The Review tail must reach the edge of the fixed footer through the one real
/// body viewport, with the values it describes.
fn assert_review_tail_scrolls_above_the_footer(
    view: &gpui::Entity<crate::Block>,
    cx: &mut gpui::VisualTestContext,
    entry: &str,
) {
    let viewport = cx.update(|window, _| window.viewport_size());
    let body = review_bounds(cx, "wizard-body", entry);
    let footer = review_bounds(cx, "wizard-footer", entry);
    assert!(
        body.bottom() <= footer.top(),
        "{entry}: the body must end above the fixed footer: body={body:?} footer={footer:?}"
    );
    assert!(
        footer.bottom() <= viewport.height + px(1.),
        "{entry}: the fixed footer stays inside {viewport:?}: footer={footer:?}"
    );
    let (max, tracked) = view.read_with(cx, |view, _| {
        let scroll = &view.shell.wizard.body_scroll;
        (scroll.max_offset(), scroll.bounds())
    });
    // A nested Review viewport claims the shared handle while sizing itself to
    // its own content, so the outer page keeps the space and reports no range.
    assert!(
        max.y > px(0.),
        "{entry}: these values must need the body scroll: max={max:?} tracked={tracked:?}"
    );
    focus_wizard_body(cx, entry);
    crate::test_support::press(cx, "end");
    settle(cx);
    let offset = view.read_with(cx, |view, _| view.shell.wizard.body_scroll.offset().y);
    assert!(
        (offset + max.y).abs() <= px(1.),
        "{entry}: End must reach the true maximum: offset={offset:?} max={max:?}"
    );
    crate::test_support::press(cx, "home");
    settle(cx);
    assert_eq!(
        view.read_with(cx, |view, _| view.shell.wizard.body_scroll.offset().y),
        px(0.),
        "{entry}: Home must return the body to its top"
    );
    crate::test_support::press(cx, "end");
    settle(cx);
    let body = review_bounds(cx, "wizard-body", entry);
    let trigger = review_bounds(cx, "wizard-review-options", entry);
    for selector in ["wizard-review-game-window", "wizard-review-game-data"] {
        let fact = review_bounds(cx, selector, entry);
        assert!(
            fact.top() >= trigger.bottom(),
            "{entry}: {selector} must stay a folded default of the expanded details: \
             fact={fact:?} trigger={trigger:?}"
        );
        assert!(
            fact.size.height > px(0.)
                && fact.top() >= body.top() - px(1.)
                && fact.bottom() <= body.bottom() + px(1.),
            "{entry}: {selector} and its value must sit inside the body viewport at its \
             maximum: fact={fact:?} body={body:?} max={max:?}"
        );
    }
    let content = review_bounds(cx, "wizard-review-content", entry);
    assert!(
        content.size.height > body.size.height && content.bottom() <= body.bottom() + px(1.),
        "{entry}: the whole expanded Review must be reachable: content={content:?} body={body:?}"
    );
    assert_eq!(
        review_bounds(cx, "wizard-footer", entry),
        footer,
        "{entry}: scrolling the body must not move the fixed footer"
    );
}

#[gpui::test]
fn expanded_review_scrolls_all_of_its_facts_above_the_fixed_footer(cx: &mut TestAppContext) {
    // The diagnostic window: 1152×664 with the Review's Java, JVM, server,
    // automatic connection and directory values long, and the fold really open.
    // The game window keeps the default 1280×720, so Game window and Game data
    // stay the folded defaults at the bottom of the expanded Review.
    let directory = std::env::temp_dir().join("block review long folder");
    let directory = directory.to_string_lossy().into_owned();
    let java = std::env::temp_dir().join("java runtimes/bin/java");
    let java = java.to_string_lossy().into_owned();
    let arguments = "-XX:+UseG1GC -XX:MaxGCPauseMillis=200 -Dfml.readTimeout=180";
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.preferences.reduce_motion = true;
        view.versions = vec!["1.21.1".into()];
        let release = block_launcher::manifest::model::VersionType::Release;
        view.version_catalog = vec![entry("1.21.1", release)];
        view.snapshot = ready_snapshot();
        view.begin_game_wizard(cx);
        view
    });
    // A tall window reaches the fold trigger, which a real click opens; reduced
    // motion settles the disclosure at its natural height.
    cx.simulate_resize(size(px(960.), px(1400.)));
    view.update(cx, |view, cx| {
        if let Some(Form::Instance(form)) = &mut view.form {
            form.directory
                .update(cx, |field, cx| field.set_value(&directory, cx));
        }
        if let Some(profile) = &mut view.shell.wizard.profile {
            profile.managed = false;
            profile
                .java
                .update(cx, |field, cx| field.set_value(&java, cx));
            profile
                .jvm
                .update(cx, |field, cx| field.set_value(arguments, cx));
            profile.server.update(cx, |field, cx| {
                field.set_value("play.example.test:25565", cx)
            });
            profile.connect_on_launch = true;
        }
        view.shell.wizard.step = WizardStep::Review;
        view.shell.wizard.page = None;
        cx.notify();
    });
    cx.run_until_parked();
    click(cx, "wizard-review-options");
    assert!(
        view.read_with(cx, |view, _| view.shell.wizard.advanced.borrow().is_open()),
        "the fold must really open"
    );
    cx.simulate_resize(size(px(1152.), px(664.)));
    cx.run_until_parked();
    assert_review_tail_scrolls_above_the_footer(&view, cx, "Review at 1152x664");
    // Back then Continue keeps this draft, but the returned Review folds its
    // defaults again; opening them there still scrolls the same tail into view.
    click(cx, "wizard-back");
    assert_eq!(
        view.read_with(cx, |view, _| view.shell.wizard.step),
        WizardStep::Setup
    );
    click(cx, "wizard-next");
    assert_eq!(
        view.read_with(cx, |view, _| view.shell.wizard.step),
        WizardStep::Review
    );
    assert!(
        view.read_with(cx, |view, _| !view.shell.wizard.advanced.borrow().is_open()),
        "the returned review must fold its defaults again"
    );
    // A tall window reaches the fold trigger, which a real click opens.
    cx.simulate_resize(size(px(960.), px(1400.)));
    cx.run_until_parked();
    click(cx, "wizard-review-options");
    cx.simulate_resize(size(px(1152.), px(664.)));
    cx.run_until_parked();
    assert_review_tail_scrolls_above_the_footer(&view, cx, "Review after Back");
    // First-run setup renders the same frame, so the same body viewport owns it.
    view.update(cx, |view, cx| {
        view.router.navigate(crate::Tab::Onboarding);
        view.router.enter_game_wizard();
        view.shell.first_run.enabled = true;
        view.shell.first_run.stage = crate::first_run::Stage::GameWizard;
        cx.notify();
    });
    cx.run_until_parked();
    assert_review_tail_scrolls_above_the_footer(&view, cx, "first-run Review at 1152x664");
}

#[gpui::test]
fn first_run_create_status_names_the_save_and_starts_no_preparation(cx: &mut TestAppContext) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let (engine, mut messages) = crate::Engine::with_root(root.path().into())?;
        let (view, cx) = crate::test_support::window(cx, move |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.preferences.reduce_motion = true;
            view.versions = vec!["1.21.1".into()];
            view.engine = Some(engine);
            view.begin_game_wizard(cx);
            view
        });
        view.update(cx, |view, cx| {
            view.router.navigate(crate::Tab::Onboarding);
            view.router.enter_game_wizard();
            view.shell.first_run.enabled = true;
            view.shell.wizard.step = WizardStep::Review;
            view.submit_game_wizard(cx);
            assert!(view.busy, "first-run Create must start its save");
            assert_eq!(
                view.status.text(Language::English),
                "Saving your game setup",
                "the busy status must name the save first-run Create really starts"
            );
        });
        // The real submit path saves the setup and starts no preparation: only
        // Finish begins installing game files.
        let runtime = tokio::runtime::Runtime::new()?;
        let finished = runtime.block_on(async {
            tokio::time::timeout(std::time::Duration::from_secs(20), async {
                loop {
                    let message = messages.recv().await.ok_or_else(|| {
                        anyhow::anyhow!("the engine channel closed before the save finished")
                    })?;
                    let message = match message {
                        crate::Message::ForOperation { message, .. } => *message,
                        message => message,
                    };
                    match message {
                        crate::Message::Downloads(_) => {
                            return Err(anyhow::anyhow!(
                                "first-run Create must not prepare game files"
                            ));
                        }
                        crate::Message::Finished(result) => return Ok(result),
                        _ => {}
                    }
                }
            })
            .await
            .map_err(|_| anyhow::anyhow!("the first-run save did not finish"))?
        })?;
        assert_eq!(
            finished
                .ok()
                .map(|status| status.text(Language::English))
                .as_deref(),
            Some("Game saved"),
            "first-run Create must end after saving"
        );
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn ordinary_create_status_still_announces_preparation(cx: &mut TestAppContext) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        // A saved file cannot be an installation directory, so the catalog
        // refuses this creation before any download begins. The status is fixed
        // before the engine sees the request, so it is still the real one.
        let blocked = root.path().join("not-a-directory");
        std::fs::write(&blocked, b"1")?;
        let (engine, mut messages) = crate::Engine::with_root(root.path().into())?;
        let (view, cx) = crate::test_support::window(cx, move |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.preferences.reduce_motion = true;
            view.versions = vec!["1.21.1".into()];
            view.engine = Some(engine);
            view.begin_game_wizard(cx);
            view
        });
        view.update(cx, |view, cx| {
            if let Some(Form::Instance(form)) = &mut view.form {
                let blocked = blocked.to_string_lossy().into_owned();
                form.directory
                    .update(cx, |field, cx| field.set_value(&blocked, cx));
            }
            view.shell.wizard.step = WizardStep::Review;
            view.submit_game_wizard(cx);
            assert!(view.busy, "ordinary Create must start its installation");
            assert_eq!(
                view.status.text(Language::English),
                "Creating your game and preparing verified downloads",
                "ordinary Create still prepares game files"
            );
        });
        // The refused creation must fail locally instead of downloading.
        let runtime = tokio::runtime::Runtime::new()?;
        let finished = runtime.block_on(async {
            tokio::time::timeout(std::time::Duration::from_secs(20), async {
                loop {
                    let message = messages.recv().await.ok_or_else(|| {
                        anyhow::anyhow!("the engine channel closed before the creation ended")
                    })?;
                    let message = match message {
                        crate::Message::ForOperation { message, .. } => *message,
                        message => message,
                    };
                    match message {
                        crate::Message::Downloads(_) => {
                            return Err(anyhow::anyhow!("a file path cannot host an installation"));
                        }
                        crate::Message::Finished(result) => return Ok(result),
                        _ => {}
                    }
                }
            })
            .await
            .map_err(|_| anyhow::anyhow!("the refused creation did not finish"))?
        })?;
        assert!(
            finished.is_err(),
            "a file path cannot host an installation: {finished:?}"
        );
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn retained_wizard_snapshot_freezes_editors_and_scroll_without_sharing_mutation(
    cx: &mut TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.preferences.reduce_motion = true;
        view.snapshot = ready_snapshot();
        view.snapshot.profile_mode = ProfileMode::Separated;
        view.begin_game_wizard(cx);
        view
    });
    view.update(cx, |view, cx| {
        view.shell
            .wizard
            .body_scroll
            .set_offset(point(px(0.), px(-112.)));
        view.shell
            .wizard
            .section_scroll
            .set_offset(point(px(0.), px(-48.)));
        view.shell
            .wizard
            .version_scroll
            .set_offset(point(px(0.), px(-64.)));
        if let Some(profile) = &view.shell.wizard.profile {
            profile
                .name
                .update(cx, |input, cx| input.set_value("中文 Profile", cx));
        }
        let frozen = view.shell.wizard.snapshot(cx);
        view.shell
            .wizard
            .body_scroll
            .set_offset(point(px(0.), px(-8.)));
        view.shell
            .wizard
            .section_scroll
            .set_offset(point(px(0.), px(-4.)));
        view.shell
            .wizard
            .version_scroll
            .set_offset(point(px(0.), px(-2.)));
        if let Some(profile) = &view.shell.wizard.profile {
            profile
                .name
                .update(cx, |input, cx| input.set_value("New live name", cx));
        }
        assert_eq!(frozen.body_scroll.offset(), point(px(0.), px(-112.)));
        assert_eq!(frozen.section_scroll.offset(), point(px(0.), px(-48.)));
        assert_eq!(frozen.version_scroll.offset(), point(px(0.), px(-64.)));
        assert!(
            frozen.reveal.get().is_none(),
            "a retained exit must not reveal or focus fields"
        );
        let profile = frozen.profile.as_ref();
        assert!(profile.is_some());
        if let Some(profile) = profile {
            assert_eq!(profile.name.read(cx).value(), "中文 Profile");
        }
    });
}

#[gpui::test]
fn component_cancel_restores_values_and_done_only_commits_the_local_draft(cx: &mut TestAppContext) {
    let directory = std::env::temp_dir().join("block-new-game");
    let directory = directory.to_string_lossy().into_owned();
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.preferences.reduce_motion = true;
        view.versions = vec!["1.21.1".into(), "1.20.6".into()];
        view.snapshot = ready_snapshot();
        view.begin_game_wizard(cx);
        view
    });
    view.update(cx, |view, cx| {
        view.open_game_wizard_page(Some(WizardPage::Loader), cx);
        if let Some(Form::Instance(form)) = &mut view.form {
            form.version
                .update(cx, |field, cx| field.set_value("1.20.6", cx));
            form.directory
                .update(cx, |field, cx| field.set_value(&directory, cx));
            form.loader = block_launcher::catalog::LoaderKind::NeoForge;
            form.loader_version = Some("20.6.144".into());
        }
        view.back_game_wizard(cx);
        if let Some(Form::Instance(form)) = &view.form {
            assert_eq!(form.version.read(cx).value(), "1.21.1");
            assert_eq!(form.directory.read(cx).value(), "");
            assert!(!form.is_neoforge());
        }
        assert_eq!(view.snapshot.instances.len(), 1);
        view.open_game_wizard_page(Some(WizardPage::Location), cx);
        if let Some(Form::Instance(form)) = &view.form {
            form.directory
                .update(cx, |field, cx| field.set_value("relative-path", cx));
        }
        view.advance_game_wizard(cx);
        assert_eq!(view.shell.wizard.page, Some(WizardPage::Location));
        assert!(error_text(view).is_some());
        if let Some(Form::Instance(form)) = &view.form {
            form.directory
                .update(cx, |field, cx| field.set_value(&directory, cx));
        }
        view.advance_game_wizard(cx);
        assert_eq!(view.shell.wizard.page, None);
        assert!(error_text(view).is_none());
        if let Some(Form::Instance(form)) = &view.form {
            assert_eq!(form.directory.read(cx).value(), directory);
        }
        assert_eq!(view.snapshot.instances.len(), 1);
        assert_eq!(view.snapshot.profiles.len(), 1);
    });
}

#[gpui::test]
fn loader_picker_search_preserves_the_selected_real_build_and_cancel_rolls_back(
    cx: &mut TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.preferences.reduce_motion = true;
        view.versions = vec!["1.21.1".into(), "1.20.6".into()];
        view.begin_game_wizard(cx);
        view
    });
    cx.simulate_resize(size(px(960.), px(600.)));

    click(cx, "wizard-component-loader");
    assert!(crate::test_support::control_bounds(cx, "wizard-loader-picker").is_some());
    click(cx, "wizard-loader-neoforge");
    view.update(cx, |view, cx| {
        let request = view.shell.wizard.loader_catalog.as_ref().map(|catalog| catalog.request);
        if let Some(request) = request { view.receive_loader_versions(request, Ok(vec![loader_build("21.1.219", true)]), cx); }
        assert!(matches!(&view.form, Some(Form::Instance(form)) if form.is_neoforge() && form.version.read(cx).value() == "1.21.1"));
        cx.notify();
    });
    view.update(cx, |view, cx| {
        if let Some(search) = &view.shell.wizard.loader_search {
            search.update(cx, |field, cx| field.set_value("does-not-exist", cx));
        }
        cx.notify();
    });
    cx.run_until_parked();
    assert!(crate::test_support::control_bounds(cx, "wizard-loader-build-21.1.219").is_none());
    view.update(cx, |view, cx| {
        if let Some(search) = &view.shell.wizard.loader_search {
            search.update(cx, |field, cx| field.set_value("21.1", cx));
        }
        cx.notify();
    });
    cx.run_until_parked();
    assert!(crate::test_support::control_bounds(cx, "wizard-loader-build-21.1.219").is_some());
    click(cx, "wizard-loader-fabric");
    assert!(view.read_with(
        cx,
        |view, _| matches!(&view.form, Some(Form::Instance(form)) if form.loader == block_launcher::catalog::LoaderKind::Fabric && form.loader_version.is_none())
    ));
    click(cx, "wizard-back");
    view.read_with(cx, |view, cx| {
        assert!(matches!(&view.form, Some(Form::Instance(form)) if !form.is_neoforge() && form.version.read(cx).value() == "1.21.1"));
    });
}

#[gpui::test]
fn separated_profile_review_preserves_home_selection_until_submission(cx: &mut TestAppContext) {
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.preferences.reduce_motion = true;
        view.snapshot = ready_snapshot();
        view.snapshot.profile_mode = ProfileMode::Separated;
        view.selected_instance = Some("instance-a".into());
        view.selected_profile = Some("profile-a".into());
        view.begin_game_wizard(cx);
        view
    });
    cx.simulate_resize(size(px(960.), px(600.)));
    click(cx, "wizard-kind-profile");
    assert_eq!(
        view.read_with(cx, |view, _| view.shell.wizard.kind),
        CreationKind::ExistingInstance
    );
    click(cx, "wizard-next");
    click(cx, "wizard-instance-instance-a");
    click(cx, "wizard-next");
    assert_eq!(
        view.read_with(cx, |view, _| view.shell.wizard.step),
        WizardStep::Profile
    );
    view.update(cx, |view, cx| {
        let profile = view.shell.wizard.profile.as_ref();
        assert!(profile.is_some());
        if let Some(profile) = profile {
            profile
                .name
                .update(cx, |field, cx| field.set_value("Creative 中文", cx));
            profile
                .memory
                .update(cx, |field, cx| field.set_value("4", cx));
        }
        view.wizard_profile_icon(ProfileIcon::Leaf);
        view.wizard_profile_color(ProfileColor::Green);
        view.advance_game_wizard(cx);
        assert_eq!(view.shell.wizard.step, WizardStep::Review);
        if let Some(profile) = &view.shell.wizard.profile {
            let values = profile.values(cx);
            assert!(values.is_ok());
            if let Ok(values) = values {
                assert_eq!(values.name, "Creative 中文");
                assert_eq!(values.defaults.memory_mib, 4096);
                assert_eq!(values.appearance.icon, ProfileIcon::Leaf);
                assert_eq!(values.target_id.as_deref(), Some("vanilla"));
            }
        }
        assert_eq!(view.selected_profile.as_deref(), Some("profile-a"));
        assert_eq!(view.snapshot.profiles.len(), 1);
        cx.notify();
    });
    click(cx, "cancel-instance");
    assert!(view.read_with(cx, |view, _| view.form.is_none()));
    assert_eq!(
        view.read_with(cx, |view, _| view.selected_profile.clone()),
        Some("profile-a".into())
    );
    assert_eq!(
        view.read_with(cx, |view, _| view.snapshot.profiles.len()),
        1
    );
}

#[gpui::test]
fn version_search_filters_real_metadata_and_does_not_steal_ime_navigation(cx: &mut TestAppContext) {
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.preferences.reduce_motion = true;
        view.versions = vec!["1.20.6".into(), "1.21.1".into()];
        view.version_catalog = vec![
            entry(
                "1.20.6",
                block_launcher::manifest::model::VersionType::Release,
            ),
            entry(
                "1.21.1",
                block_launcher::manifest::model::VersionType::Release,
            ),
            entry(
                "24w44a",
                block_launcher::manifest::model::VersionType::Snapshot,
            ),
        ];
        view.begin_game_wizard(cx);
        view.open_game_wizard_page(Some(WizardPage::Version), cx);
        view
    });
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            if let Some(search) = &view.shell.wizard.version_search {
                search.read(cx).focus_handle(cx).focus(window, cx);
            }
        });
    });
    crate::test_support::press(cx, "down");
    view.read_with(cx, |view, cx| {
        if let Some(Form::Instance(form)) = &view.form {
            assert_eq!(form.version.read(cx).value(), "1.21.1");
            assert_eq!(view.matching_wizard_versions(form, cx).len(), 2);
        }
    });
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            if let Some(search) = &view.shell.wizard.version_search {
                search.update(cx, |search, cx| {
                    search.replace_and_mark_text_in_range(None, "中", None, window, cx);
                });
            }
        });
    });
    crate::test_support::press(cx, "up");
    view.read_with(cx, |view, cx| {
        if let Some(Form::Instance(form)) = &view.form {
            assert_eq!(form.version.read(cx).value(), "1.21.1");
        }
    });
    view.update(cx, |view, cx| {
        if let Some(search) = &view.shell.wizard.version_search {
            search.update(cx, |search, cx| search.set_value("24w", cx));
        }
        view.shell.wizard.include_snapshots = true;
        if let Some(Form::Instance(form)) = &view.form {
            assert_eq!(
                view.matching_wizard_versions(form, cx)
                    .iter()
                    .map(|(id, _)| id.as_str())
                    .collect::<Vec<_>>(),
                ["24w44a"]
            );
        }
        cx.notify();
    });
    crate::test_support::press(cx, "down");
    view.read_with(cx, |view, cx| {
        if let Some(Form::Instance(form)) = &view.form {
            assert_eq!(form.version.read(cx).value(), "24w44a");
        }
    });
}

#[gpui::test]
fn long_version_list_end_reveals_the_last_row_and_has_one_roving_tab_stop(cx: &mut TestAppContext) {
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.preferences.reduce_motion = true;
        view.versions = (0..40).map(|index| format!("1.20.{index}")).collect();
        view.begin_game_wizard(cx);
        view.open_game_wizard_page(Some(WizardPage::Version), cx);
        if let Some(search) = &view.shell.wizard.version_search {
            search.read(cx).focus_handle(cx).focus(window, cx);
        }
        view
    });
    cx.simulate_resize(size(px(960.), px(600.)));
    crate::test_support::press(cx, "tab end");
    cx.run_until_parked();
    view.read_with(cx, |view, cx| { assert!(matches!(&view.form, Some(Form::Instance(form)) if form.version.read(cx).value() == "1.20.39")); });
    let last = crate::test_support::control_bounds(cx, "wizard-version-39");
    let viewport = crate::test_support::control_bounds(cx, "wizard-version-list");
    assert!(
        last.zip(viewport)
            .is_some_and(|(last, viewport)| last.top() >= viewport.top() - px(1.)
                && last.bottom() <= viewport.bottom() + px(1.)),
        "last={last:?}, viewport={viewport:?}"
    );

    crate::test_support::press(cx, "tab");
    cx.update(|window, cx| { assert!(matches!(&view.read(cx).form, Some(Form::Instance(form)) if form.version.read(cx).focus_handle(cx).is_focused(window))); });
}

#[gpui::test]
fn version_loading_failure_retry_and_recovery_preserve_the_draft(cx: &mut TestAppContext) {
    let (sender, receiver) = tokio::sync::mpsc::channel(4);
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.preferences.reduce_motion = true;
        view.versions = vec!["1.20.6".into()];
        view.version_load = crate::browser::CollectionLoad::Loading;
        view.messages = Some(receiver);
        view.begin_game_wizard(cx);
        view.open_game_wizard_page(Some(WizardPage::Version), cx);
        if let Some(Form::Instance(form)) = &view.form {
            form.name
                .update(cx, |field, cx| field.set_value("保留 draft", cx));
        }
        view
    });
    cx.simulate_resize(size(px(960.), px(600.)));
    cx.run_until_parked();

    assert!(crate::test_support::control_bounds(cx, "wizard-retry-versions").is_none());
    assert!(
        sender
            .try_send(crate::Message::Versions(Err(
                "Fixture network unavailable".into()
            )))
            .is_ok()
    );
    view.update(cx, crate::Block::poll);
    cx.run_until_parked();

    click(cx, "wizard-retry-versions");
    // No test Engine: retry must report the actual missing engine, never Ready.
    assert!(view.read_with(cx, |view, _| matches!(&view.version_load, crate::browser::CollectionLoad::Failed(message) if message.text(Language::English).contains("Engine unavailable"))));
    assert!(
        sender
            .try_send(crate::Message::Versions(Ok(vec![entry(
                "1.20.6",
                block_launcher::manifest::model::VersionType::Release
            )])))
            .is_ok()
    );
    view.update(cx, crate::Block::poll);
    cx.run_until_parked();

    view.read_with(cx, |view, cx| {
        assert!(matches!(view.version_load, crate::browser::CollectionLoad::Ready));
        assert!(matches!(&view.form, Some(Form::Instance(form)) if form.name.read(cx).value() == "保留 draft" && form.version.read(cx).value() == "1.20.6"));
        assert_eq!(view.version_catalog.len(), 1);
    });
}

#[gpui::test]
fn exact_version_metadata_and_filtered_result_count_never_guess_a_release_type(
    cx: &mut TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.preferences.reduce_motion = true;
        view.versions = vec!["1.20.6".into()];
        view.version_catalog = vec![
            entry(
                "1.20.6",
                block_launcher::manifest::model::VersionType::Release,
            ),
            entry(
                "24w44a",
                block_launcher::manifest::model::VersionType::Snapshot,
            ),
            entry(
                "b1.7.3",
                block_launcher::manifest::model::VersionType::OldBeta,
            ),
        ];
        view.begin_game_wizard(cx);
        view.open_game_wizard_page(Some(WizardPage::Version), cx);
        view.shell.wizard.include_snapshots = true;
        view
    });
    cx.simulate_resize(size(px(960.), px(600.)));
    cx.run_until_parked();

    for (id, _source) in [
        ("24w44a", "Official catalog · Snapshot"),
        ("b1.7.3", "Official catalog · Old beta"),
        ("1.20.6", "Official catalog · Stable release"),
        (
            "snapshot-looking-unknown",
            "Exact version ID · not found in the loaded official catalog; installation will verify it",
        ),
    ] {
        view.update(cx, |view, cx| {
            if let Some(Form::Instance(form)) = &view.form {
                form.version.update(cx, |field, cx| field.set_value(id, cx));
            }
        });
        cx.run_until_parked();
    }
    view.update(cx, |view, cx| {
        if let Some(search) = &view.shell.wizard.version_search {
            search.update(cx, |field, cx| field.set_value("24w", cx));
        }
    });
    cx.run_until_parked();

    view.update(cx, |view, cx| {
        if let Some(search) = &view.shell.wizard.version_search {
            search.update(cx, |field, cx| field.set_value("no-match", cx));
        }
    });
    cx.run_until_parked();
}

#[gpui::test]
fn minecraft_change_clears_the_build_and_cancel_restores_the_loader(cx: &mut TestAppContext) {
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.preferences.reduce_motion = true;
        view.snapshot = ready_snapshot();
        view.versions = vec!["1.20.6".into(), "1.21.1".into()];
        view.begin_game_wizard(cx);
        if let Some(Form::Instance(form)) = &mut view.form {
            form.loader = block_launcher::catalog::LoaderKind::NeoForge;
            form.loader_version = Some("20.6.144".into());
        }
        view.open_game_wizard_page(Some(WizardPage::Version), cx);
        view.set_wizard_version(" 1.21.1 ".into(), cx);
        view
    });
    cx.simulate_resize(size(px(960.), px(600.)));
    cx.run_until_parked();

    view.read_with(cx, |view, cx| { assert!(matches!(&view.form, Some(Form::Instance(form)) if form.is_neoforge() && form.loader_version.is_none() && form.version.read(cx).value() == "1.21.1")); });
    click(cx, "wizard-back");
    view.update(cx, |view, cx| {
        assert!(matches!(&view.form, Some(Form::Instance(form)) if form.is_neoforge() && form.version.read(cx).value() == "1.20.6"));
        assert!(view.shell.wizard.version_notice.is_none());
        view.open_game_wizard_page(Some(WizardPage::Version), cx);
        view.set_wizard_version("1.20.6".into(), cx);
        assert!(view.shell.wizard.version_notice.is_none(), "the unchanged compatible version cannot announce a removal");
        view.set_wizard_version("1.21.1".into(), cx);
        let request = view.shell.wizard.loader_catalog.as_ref().map(|catalog| catalog.request);
        if let Some(request) = request { view.receive_loader_versions(request, Ok(vec![loader_build("21.1.219", true)]), cx); }
        view.advance_game_wizard(cx);
        assert!(view.shell.wizard.version_notice.is_some());
        assert!(view.shell.wizard.page.is_none());
        assert_eq!(view.snapshot.instances.len(), 1);
        assert_eq!(view.snapshot.profiles.len(), 1);
    });
}

#[gpui::test]
fn wizard_keeps_step_rail_and_actions_within_the_client(cx: &mut TestAppContext) {
    for (width, height) in [(1152., 720.), (960., 540.)] {
        let (_view, cx) = crate::test_support::window(cx, |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.preferences.reduce_motion = true;
            view.begin_game_wizard(cx);
            view
        });
        cx.simulate_resize(size(px(width), px(height)));
        cx.run_until_parked();
        for selector in ["wizard-rail", "wizard-footer", "wizard-next"] {
            let bounds = crate::test_support::control_bounds(cx, selector);
            assert!(
                bounds.is_some_and(|bounds| bounds.left() >= px(0.)
                    && bounds.right() <= px(width)
                    && bounds.bottom() <= px(height)),
                "{selector} outside {width}×{height}: {bounds:?}"
            );
        }
        let footer = crate::test_support::control_bounds(cx, "wizard-footer");
        assert_eq!(footer.map(|bounds| bounds.size.height), Some(px(72.)));
        let rail = crate::test_support::control_bounds(cx, "wizard-rail");
        assert_eq!(rail.map(|bounds| bounds.size.width), Some(px(360.)));
    }
}

#[gpui::test]
fn submitting_an_existing_instance_saves_one_fully_configured_profile(cx: &mut TestAppContext) {
    let result = (|| -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let catalog = block_launcher::catalog::Catalog::open(root.path())?;
        let (instance, original) = catalog.create_instance(
            "Shared".into(),
            "1.20.6".into(),
            None,
            Some("20.6.144".into()),
        )?;
        catalog.separate_profiles()?;
        let snapshot = catalog.snapshot()?;
        let (engine, mut receiver) = crate::Engine::with_root(root.path().into())?;
        let instance_id = instance.id;
        let original_id = original.id.clone();
        let (view, cx) = crate::test_support::window(cx, move |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.preferences.reduce_motion = true;
            view.snapshot = snapshot;
            view.engine = Some(engine);
            view.selected_instance = Some(instance_id);
            view.selected_profile = Some(original_id);
            view.begin_game_wizard(cx);
            view
        });
        view.update(cx, |view, cx| {
            view.wizard_kind(false, cx);
            view.wizard_profile_target("neoforge-20.6.144".into());
            if let Some(profile) = &view.shell.wizard.profile {
                profile
                    .name
                    .update(cx, |field, cx| field.set_value("Creative 中文", cx));
                profile
                    .memory
                    .update(cx, |field, cx| field.set_value("4", cx));
                profile.server.update(cx, |field, cx| {
                    field.set_value("play.example.test:25565", cx);
                });
                profile
                    .width
                    .update(cx, |field, cx| field.set_value("1600", cx));
                profile
                    .height
                    .update(cx, |field, cx| field.set_value("900", cx));
            }
            view.wizard_profile_color(ProfileColor::Green);
            view.wizard_profile_icon(ProfileIcon::Leaf);
            view.shell.wizard.step = WizardStep::Review;
            view.submit_game_wizard(cx);
            assert!(view.busy);
            view.submit_game_wizard(cx);
        });
        let runtime = tokio::runtime::Runtime::new()?;
        let messages = runtime.block_on(async {
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                let mut messages = Vec::new();
                loop {
                    let message = receiver.recv().await.ok_or_else(|| {
                        anyhow::anyhow!("engine channel closed before save completed")
                    })?;
                    let inner = match &message {
                        crate::Message::ForOperation { message, .. } => message.as_ref(),
                        message => message,
                    };
                    let finished = matches!(inner, crate::Message::Finished(_));
                    if matches!(inner, crate::Message::Created { .. }) {
                        let snapshot = catalog.snapshot()?;
                        assert_eq!(snapshot.profiles.len(), 2);
                        let created = snapshot
                            .profiles
                            .iter()
                            .find(|profile| profile.id != original.id);
                        assert!(created.is_some_and(|profile| {
                            profile.name == "Creative 中文"
                                && profile.memory_mib == 4096
                                && profile.target_id == "neoforge-20.6.144"
                                && profile.appearance.icon == ProfileIcon::Leaf
                                && profile.server.address == "play.example.test:25565"
                                && !profile.server.connect_on_launch
                                && profile
                                    .game_args
                                    .windows(2)
                                    .any(|pair| pair[0] == "--width" && pair[1] == "1600")
                                && profile
                                    .game_args
                                    .windows(2)
                                    .any(|pair| pair[0] == "--height" && pair[1] == "900")
                        }));
                    }
                    messages.push(message);
                    if finished {
                        return Ok::<_, anyhow::Error>(messages);
                    }
                }
            })
            .await?
        })?;
        let (sender, saved_receiver) = tokio::sync::mpsc::channel(messages.len().max(1));
        for message in messages {
            sender.try_send(message)?;
        }
        view.update(cx, |view, cx| {
            view.messages = Some(saved_receiver);
            view.poll(cx);
        });
        let snapshot = catalog.snapshot()?;
        assert_eq!(snapshot.instances.len(), 1);
        assert_eq!(snapshot.profiles.len(), 2);
        assert!(
            snapshot
                .profiles
                .iter()
                .any(|profile| profile.id == original.id && profile.name == original.name)
        );
        assert!(!view.read_with(cx, |view, _| view.busy));
        assert!(view.read_with(cx, |view, _| view.form.is_none()));
        assert_ne!(
            view.read_with(cx, |view, _| view.selected_profile.clone()),
            Some(original.id)
        );
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

#[gpui::test]
fn profile_error_expands_advanced_settings_and_reveals_the_invalid_input(cx: &mut TestAppContext) {
    for width in [960., 1152.] {
        let (view, cx) = crate::test_support::window(cx, |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.preferences.reduce_motion = true;
            view.snapshot = ready_snapshot();
            view.snapshot.profile_mode = ProfileMode::Separated;
            view.begin_game_wizard(cx);
            view.shell.wizard.step = WizardStep::Profile;
            if let Some(profile) = &view.shell.wizard.profile {
                profile
                    .height
                    .update(cx, |field, cx| field.set_value("not-a-number", cx));
            }
            view
        });
        cx.simulate_resize(size(px(width), px(540.)));
        click(cx, "wizard-next");
        cx.run_until_parked();
        assert_eq!(
            view.read_with(cx, |view, _| view.shell.wizard.step),
            WizardStep::Profile
        );
        view.read_with(cx, |view, _| {
            assert!(!view.shell.surfaces.is_open());
            assert_eq!(view.shell.wizard.page, Some(WizardPage::Location));
            assert_eq!(
                view.page_spec().wizard,
                Some(crate::navigation::WizardKey::Game(
                    WizardStep::Profile,
                    Some(WizardPage::Location)
                ))
            );
        });
        cx.update(|window, _| {
            let next = gpui_base::test_support::find_all(window, &[], &"wizard-next".into());
            assert_eq!(next.len(), 1);
            assert_eq!(next[0].label(), Some("Done"));
        });
        assert!(view.read_with(cx, |view, _| {
            view.shell
                .wizard
                .profile
                .as_ref()
                .is_some_and(|profile| profile.options)
        }));
        let input = crate::test_support::control_bounds(cx, "wizard-field-Height");
        let body = crate::test_support::control_bounds(cx, "wizard-body");
        assert!(
            input.zip(body).is_some_and(
                |(input, body)| input.top() >= body.top() && input.bottom() <= body.bottom()
            ),
            "invalid input must be visible at {width}: {input:?}, {body:?}"
        );
        cx.update(|window, cx| {
            let profile = view.read(cx).shell.wizard.profile.as_ref();
            assert!(profile.is_some_and(|profile| {
                profile.height.read(cx).focus_handle(cx).is_focused(window)
                    && profile.height.read(cx).value() == "not-a-number"
            }));
        });
        assert_eq!(
            view.read_with(cx, |view, _| view.snapshot.profiles.len()),
            1
        );
    }
}

#[gpui::test]
fn configuration_done_validates_its_own_fields_and_cancel_restores_editable_entities(
    cx: &mut TestAppContext,
) {
    let java = std::env::temp_dir().join(format!("java{}", std::env::consts::EXE_SUFFIX));
    let java = java.to_string_lossy().into_owned();
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.preferences.reduce_motion = true;
        view.snapshot = ready_snapshot();
        view.begin_game_wizard(cx);
        view.open_game_wizard_page(Some(WizardPage::Location), cx);
        if let Some(Form::Instance(form)) = &view.form {
            form.name.update(cx, |field, cx| field.set_value("", cx));
        }
        if let Some(profile) = &mut view.shell.wizard.profile {
            profile.managed = false;
            profile
                .java
                .update(cx, |field, cx| field.set_value(&java, cx));
            profile
                .width
                .update(cx, |field, cx| field.set_value("invalid", cx));
        }
        view
    });
    cx.simulate_resize(size(px(960.), px(600.)));
    click(cx, "wizard-next");
    assert_eq!(
        view.read_with(cx, |view, _| view.shell.wizard.page),
        Some(WizardPage::Location)
    );
    assert_eq!(
        view.read_with(cx, |view, _| view.shell.wizard.error_field),
        Some(WizardField::Width)
    );
    view.update(cx, |view, cx| {
        if let Some(profile) = &mut view.shell.wizard.profile {
            profile
                .width
                .update(cx, |field, cx| field.set_value("1600", cx));
            profile
                .height
                .update(cx, |field, cx| field.set_value("900", cx));
            profile.fullscreen = true;
        }
    });
    click(cx, "wizard-next");
    view.update(cx, |view, cx| {
        assert_eq!(
            view.shell.wizard.page, None,
            "section Done must not validate the external blank game name"
        );
        assert_eq!(
            view.wizard_configuration_summary(cx),
            ("Custom Java".to_owned(), "Fullscreen".to_owned())
        );
        let original = view.shell.wizard.profile.as_ref().map(|profile| {
            (
                profile.width.entity_id(),
                profile.width.read(cx).value(),
                profile.java.read(cx).value(),
            )
        });
        view.open_game_wizard_page(Some(WizardPage::Location), cx);
        if let Some(profile) = &mut view.shell.wizard.profile {
            profile
                .width
                .update(cx, |field, cx| field.set_value("1920", cx));
            profile
                .java
                .update(cx, |field, cx| field.set_value("/different/java", cx));
            profile.fullscreen = false;
        }
        view.cancel_component_draft(cx);
        assert_eq!(
            view.shell.wizard.profile.as_ref().map(|profile| (
                profile.width.entity_id(),
                profile.width.read(cx).value(),
                profile.java.read(cx).value()
            )),
            original
        );
        assert!(
            view.shell
                .wizard
                .profile
                .as_ref()
                .is_some_and(|profile| profile.fullscreen)
        );
        assert_eq!(view.snapshot.instances.len(), 1);
    });
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            if let Some(profile) = &view.shell.wizard.profile {
                profile.width.update(cx, |field, cx| {
                    field.replace_text_in_range(Some(0..4), "1280", window, cx);
                });
                assert_eq!(
                    profile.width.read(cx).value(),
                    "1280",
                    "cancel must restore the editable original, never a frozen outgoing entity"
                );
            }
        });
    });
}

/// Open the real Configuration section over the live setup page.
fn open_configuration_section(view: &gpui::Entity<crate::Block>, cx: &mut gpui::VisualTestContext) {
    click(cx, "wizard-component-location");
    view.read_with(cx, |view, _| {
        assert!(
            view.shell.surfaces.top().is_some_and(|layer| matches!(
                &layer.kind,
                crate::shell::Surface::WizardSection(WizardPage::Location)
            )),
            "the Configuration card must open its own section surface"
        );
        assert_eq!(view.shell.wizard.page, Some(WizardPage::Location));
    });
}

/// Enter a relative installation folder and let the section's own Done reject it.
fn reject_relative_installation_folder(
    view: &gpui::Entity<crate::Block>,
    cx: &mut gpui::VisualTestContext,
) {
    open_configuration_section(view, cx);
    view.update(cx, |view, cx| {
        if let Some(Form::Instance(form)) = &view.form {
            form.directory
                .update(cx, |field, cx| field.set_value("relative-fixture", cx));
        }
        cx.notify();
    });
    click(cx, "wizard-next");
    view.read_with(cx, |view, _| {
        assert_eq!(view.shell.wizard.page, Some(WizardPage::Location));
        assert_eq!(view.shell.wizard.error_field, Some(WizardField::Directory));
        assert!(
            error_text(view)
                .as_deref()
                .is_some_and(|error| error.contains("absolute installation folder")),
            "the section Done must reject the relative folder"
        );
    });
    assert!(
        crate::test_support::control_bounds(cx, "wizard-error").is_some()
            && crate::test_support::control_bounds(cx, "wizard-location-error").is_some(),
        "the rejection must be painted in the section"
    );
}

/// A cancelled section must leave the outer page with the restored draft and no
/// trace of the validation that belonged to the discarded edits.
fn assert_section_cancelled_without_stale_validation(
    view: &gpui::Entity<crate::Block>,
    cx: &mut gpui::VisualTestContext,
) {
    view.read_with(cx, |view, cx| {
        assert!(
            !view.shell.surfaces.is_open(),
            "the section surface must close"
        );
        assert_eq!(view.shell.wizard.page, None, "the outer page is restored");
        assert!(view.shell.wizard.section_snapshot.is_none());
        assert!(
            error_text(view).is_none(),
            "a cancelled draft must not leave its validation on the outer page"
        );
        assert!(view.shell.wizard.error_field.is_none());
        assert!(
            view.shell.wizard.reveal.get().is_none(),
            "no pending reveal may survive the cancellation"
        );
        assert!(
            matches!(&view.form, Some(Form::Instance(form))
            if form.directory.read(cx).value().is_empty()
                && form.name.read(cx).value() == "Retained game"),
            "section cancellation restores the checkpoint, not the rejected edits"
        );
    });
    assert!(
        crate::test_support::control_bounds(cx, "wizard-error").is_none(),
        "the outer page must not paint the cancelled draft's error"
    );
}

#[gpui::test]
fn configuration_section_pointer_cancel_discards_the_rejected_validation(cx: &mut TestAppContext) {
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.preferences.reduce_motion = true;
        view.snapshot = ready_snapshot();
        view.snapshot.profile_mode = ProfileMode::Separated;
        view.versions = vec!["1.21.1".into()];
        view.begin_game_wizard(cx);
        if let Some(Form::Instance(form)) = &view.form {
            form.name
                .update(cx, |field, cx| field.set_value("Retained game", cx));
        }
        view
    });
    cx.simulate_resize(size(px(960.), px(900.)));
    click(cx, "wizard-next");
    reject_relative_installation_folder(&view, cx);
    click(cx, "section-cancel");
    assert_section_cancelled_without_stale_validation(&view, cx);
    // A valid Configuration opened after the cancellation starts clean and commits.
    click(cx, "wizard-component-location");
    view.read_with(cx, |view, _| {
        assert_eq!(view.shell.wizard.page, Some(WizardPage::Location));
        assert!(error_text(view).is_none());
        assert!(view.shell.wizard.reveal.get().is_none());
    });
    assert!(
        crate::test_support::control_bounds(cx, "wizard-location-error").is_none(),
        "a fresh section must not show the discarded draft's error"
    );
    click(cx, "wizard-next");
    view.read_with(cx, |view, _| {
        assert!(!view.shell.surfaces.is_open());
        assert_eq!(view.shell.wizard.page, None);
        assert!(error_text(view).is_none());
    });
}

#[gpui::test]
fn configuration_section_escape_discards_the_rejected_validation(cx: &mut TestAppContext) {
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.preferences.reduce_motion = true;
        view.snapshot = ready_snapshot();
        view.snapshot.profile_mode = ProfileMode::Separated;
        view.versions = vec!["1.21.1".into()];
        view.begin_game_wizard(cx);
        if let Some(Form::Instance(form)) = &view.form {
            form.name
                .update(cx, |field, cx| field.set_value("Retained game", cx));
        }
        view
    });
    cx.simulate_resize(size(px(960.), px(900.)));
    click(cx, "wizard-next");
    reject_relative_installation_folder(&view, cx);
    crate::test_support::press(cx, "escape");
    cx.run_until_parked();
    assert_section_cancelled_without_stale_validation(&view, cx);
}

/// Direct wizard state, no surface: validation schedules the reveal and
/// cancellation must discard it together with the message and field owner.
#[gpui::test]
fn wizard_state_section_cancel_discards_validation_and_pending_reveal(cx: &mut TestAppContext) {
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.preferences.reduce_motion = true;
        view.snapshot = ready_snapshot();
        view.versions = vec!["1.21.1".into()];
        view.begin_game_wizard(cx);
        view
    });
    view.update(cx, |view, cx| {
        view.open_game_wizard_page(Some(WizardPage::Location), cx);
        if let Some(Form::Instance(form)) = &view.form {
            form.directory
                .update(cx, |field, cx| field.set_value("relative-fixture", cx));
        }
        assert!(!view.validate_wizard_section(WizardPage::Location, cx));
        assert_eq!(view.shell.wizard.error_field, Some(WizardField::Directory));
        assert_eq!(
            view.shell.wizard.reveal.get(),
            Some(WizardField::Directory),
            "validation schedules the reveal for the section"
        );
        view.cancel_component_draft(cx);
        assert_eq!(view.shell.wizard.page, None);
        assert!(error_text(view).is_none());
        assert!(view.shell.wizard.error_field.is_none());
        assert_eq!(view.shell.wizard.reveal.get(), None);
        assert!(matches!(&view.form, Some(Form::Instance(form))
            if form.directory.read(cx).value().is_empty()));
    });
}

#[gpui::test]
fn server_address_can_be_saved_while_unknown_quick_play_cannot_be_enabled(cx: &mut TestAppContext) {
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.preferences.reduce_motion = true;
        view.snapshot = ready_snapshot();
        view.snapshot.profile_mode = ProfileMode::Separated;
        view.begin_game_wizard(cx);
        view.wizard_kind(false, cx);
        view.shell.wizard.step = WizardStep::Profile;
        if let Some(profile) = &view.shell.wizard.profile {
            profile.server.update(cx, |field, cx| {
                field.set_value("play.example.test:25565", cx);
            });
        }
        view
    });
    cx.simulate_resize(size(px(960.), px(600.)));
    click(cx, "wizard-profile-connect");
    view.read_with(cx, |view, cx| {
        assert!(
            !view
                .shell
                .wizard
                .profile
                .as_ref()
                .is_some_and(|profile| profile.connect_on_launch)
        );
        let values = view
            .shell
            .wizard
            .profile
            .as_ref()
            .and_then(|profile| profile.values(cx).ok());
        assert!(
            values.is_some_and(|values| values.server.address == "play.example.test:25565"
                && !values.server.connect_on_launch)
        );
    });
    view.update(cx, |view, cx| {
        if let Some(profile) = &mut view.shell.wizard.profile {
            profile.connect_on_launch = true;
        }
        cx.notify();
    });
    click(cx, "wizard-next");
    assert_eq!(
        view.read_with(cx, |view, _| view.shell.wizard.step),
        WizardStep::Profile
    );
    assert_eq!(
        view.read_with(cx, |view, _| view.shell.wizard.error_field),
        Some(WizardField::Server)
    );
    assert_eq!(
        view.read_with(cx, |view, _| view.snapshot.profiles.len()),
        1
    );
}

#[gpui::test]
fn automatic_connection_prefers_installed_manifests_and_the_official_catalog_family(
    cx: &mut TestAppContext,
) {
    let result = (|| -> anyhow::Result<()> {
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../block-launcher/tests/fixtures/manifests");
        let root = tempfile::tempdir()?;
        let legacy_path = root.path().join("1.12.2.json");
        std::fs::write(&legacy_path, std::fs::read(fixtures.join("1.12.2.json"))?)?;
        let index: block_launcher::manifest::model::VersionIndex =
            block_launcher::manifest::decode(&std::fs::read(
                fixtures.join("official/version_manifest_v2.json"),
            )?)?;
        let modern = index
            .versions
            .iter()
            .find(|entry| entry.id == "1.20.6")
            .ok_or_else(|| anyhow::anyhow!("official fixture missing"))?;
        let modern_path = root.path().join("1.20.6.json");
        std::fs::write(
            &modern_path,
            std::fs::read(
                fixtures
                    .join("official/versions")
                    .join(format!("{}.json", modern.sha1)),
            )?,
        )?;
        view.update(cx, |view, _| {
            view.version_catalog = vec![
                entry(
                    "1.20.6",
                    block_launcher::manifest::model::VersionType::Release,
                ),
                entry(
                    "1.12.2",
                    block_launcher::manifest::model::VersionType::Release,
                ),
                entry(
                    "24w14a",
                    block_launcher::manifest::model::VersionType::Snapshot,
                ),
                entry(
                    "a1.2.6",
                    block_launcher::manifest::model::VersionType::OldAlpha,
                ),
            ];
            use block_launcher::catalog::AutomaticConnection;
            // Catalog-only versions fall back to the official family.
            assert_eq!(
                view.automatic_connection("1.20.6"),
                Some(AutomaticConnection::QuickPlay)
            );
            assert_eq!(
                view.automatic_connection("1.12.2"),
                Some(AutomaticConnection::Legacy)
            );
            assert_eq!(
                view.automatic_connection("a1.2.6"),
                Some(AutomaticConnection::Unsupported)
            );
            assert_eq!(view.automatic_connection("unknown-version"), None);
            // Installed official manifests are authoritative.
            let mut snapshot = ready_snapshot();
            snapshot.instances[0].minecraft_version = "1.12.2".into();
            snapshot.instances[0].targets[0].manifest = Some(legacy_path.clone());
            view.snapshot = snapshot;
            assert_eq!(
                view.automatic_connection("1.12.2"),
                Some(AutomaticConnection::Legacy)
            );
            view.snapshot.instances[0].minecraft_version = "1.20.6".into();
            view.snapshot.instances[0].targets[0].manifest = Some(modern_path.clone());
            assert_eq!(
                view.automatic_connection("1.20.6"),
                Some(AutomaticConnection::QuickPlay)
            );
        });
        Ok(())
    })();
    assert!(result.is_ok(), "{result:?}");
}

fn loader_build(version: &str, stable: bool) -> block_launcher::install::fabric::LoaderVersion {
    block_launcher::install::fabric::LoaderVersion {
        version: version.into(),
        stable,
    }
}

#[gpui::test]
fn loader_selection_is_a_local_draft_and_cancel_rejects_the_cancelled_catalog_result(
    cx: &mut TestAppContext,
) {
    use block_launcher::catalog::LoaderKind;
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.begin_game_wizard(cx);
        view.visual_only = true;
        view
    });
    view.update(cx, |view, cx| {
        view.open_game_wizard_page(Some(WizardPage::Loader), cx);
        view.choose_wizard_loader(LoaderKind::Fabric, cx);
        let request = view.shell.wizard.loader_catalog.as_ref().map(|catalog| catalog.request);
        assert!(request.is_some());
        if let Some(request) = request {
            view.receive_loader_versions(request, Ok(vec![loader_build("0.9.0", true), loader_build("0.20.0-beta.1", false), loader_build("0.19.5", true)]), cx);
        }
        assert!(view.wizard_loader_issue(cx).is_none());
        assert!(matches!(&view.form, Some(Form::Instance(form)) if form.loader_version.as_deref() == Some("0.19.5")));
        view.choose_wizard_loader_build("0.9.0".into(), cx);
        view.advance_game_wizard(cx);
        assert!(view.shell.wizard.page.is_none());
        assert!(view.snapshot.instances.is_empty());
        view.open_game_wizard_page(Some(WizardPage::Loader), cx);
        view.choose_wizard_loader(LoaderKind::Quilt, cx);
        let cancelled = view.shell.wizard.loader_catalog.as_ref().map(|catalog| catalog.request);
        let cancel = view.shell.wizard.loader_request.as_ref().map(|request| request.0.clone());
        view.cancel_component_draft(cx);
        assert!(cancel.is_some_and(|cancel| cancel.is_cancelled()));
        if let Some(request) = cancelled { view.receive_loader_versions(request, Ok(vec![loader_build("0.29.2", true)]), cx); }
        assert!(matches!(&view.form, Some(Form::Instance(form)) if form.loader == LoaderKind::Fabric && form.loader_version.as_deref() == Some("0.9.0")));
        assert!(view.wizard_loader_issue(cx).is_none());
        assert!(view.snapshot.instances.is_empty());
    });
}

#[gpui::test]
fn changing_minecraft_keeps_the_loader_but_cancels_old_mappings_and_rejects_late_results(
    cx: &mut TestAppContext,
) {
    use block_launcher::catalog::LoaderKind;
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.begin_game_wizard(cx);
        view.visual_only = true;
        view
    });
    view.update(cx, |view, cx| {
        view.set_wizard_version("1.20.6".into(), cx);
        view.choose_wizard_loader(LoaderKind::Fabric, cx);
        let old = view.shell.wizard.loader_catalog.as_ref().map(|catalog| catalog.request);
        let cancel = view.shell.wizard.loader_request.as_ref().map(|request| request.0.clone());
        view.set_wizard_version("1.21.1".into(), cx);
        assert!(cancel.is_some_and(|cancel| cancel.is_cancelled()));
        assert!(matches!(&view.form, Some(Form::Instance(form)) if form.loader == LoaderKind::Fabric && form.loader_version.is_none()));
        if let Some(request) = old { view.receive_loader_versions(request, Ok(vec![loader_build("0.15.0", true)]), cx); }
        assert!(matches!(&view.form, Some(Form::Instance(form)) if form.loader_version.is_none()));
        let current = view.shell.wizard.loader_catalog.as_ref().map(|catalog| catalog.request);
        assert_ne!(old, current);
        if let Some(request) = current { view.receive_loader_versions(request, Ok(vec![loader_build("0.16.14", true)]), cx); }
        assert!(view.wizard_loader_issue(cx).is_none());
        assert!(matches!(&view.form, Some(Form::Instance(form)) if form.loader_version.as_deref() == Some("0.16.14") && form.version.read(cx).value() == "1.21.1"));
        assert!(view.shell.wizard.version_notice.is_some());
        view.choose_wizard_loader(LoaderKind::Quilt, cx);
        assert!(view.shell.wizard.version_notice.is_none());
        assert!(view.snapshot.instances.is_empty());
    });
}

#[gpui::test]
fn loader_empty_error_retry_and_delisted_build_states_cannot_be_committed_as_vanilla(
    cx: &mut TestAppContext,
) {
    use block_launcher::catalog::LoaderKind;
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.begin_game_wizard(cx);
        view.visual_only = true;
        view.open_game_wizard_page(Some(WizardPage::Loader), cx);
        view.choose_wizard_loader(LoaderKind::Quilt, cx);
        view
    });
    view.update(cx, |view, cx| {
        assert!(view.wizard_loader_issue(cx).is_some());
        let request = view.shell.wizard.loader_catalog.as_ref().map(|catalog| catalog.request);
        if let Some(request) = request { view.receive_loader_versions(request, Err("Fixture throttle".into()), cx); }
        assert!(!view.validate_instance_draft(cx));
        view.advance_game_wizard(cx);
        assert_eq!(view.shell.wizard.page, Some(WizardPage::Loader));
        view.load_wizard_loader_versions(true, cx);
        let next = view.shell.wizard.loader_catalog.as_ref().map(|catalog| catalog.request);
        if let Some(request) = request { view.receive_loader_versions(request, Ok(vec![loader_build("0.1.0", true)]), cx); }
        if let Some(request) = next { view.receive_loader_versions(request, Ok(Vec::new()), cx); }
        assert!(!view.validate_instance_draft(cx));
        view.choose_wizard_loader_build("not-listed".into(), cx);
        assert!(matches!(&view.form, Some(Form::Instance(form)) if form.loader_version.is_none()));
        view.load_wizard_loader_versions(true, cx);
        let loaded = view.shell.wizard.loader_catalog.as_ref().map(|catalog| catalog.request);
        if let Some(request) = loaded { view.receive_loader_versions(request, Ok(vec![loader_build("0.29.2", true)]), cx); }
        assert!(view.validate_instance_draft(cx));
        view.load_wizard_loader_versions(true, cx);
        let refresh = view.shell.wizard.loader_catalog.as_ref().map(|catalog| catalog.request);
        if let Some(request) = refresh { view.receive_loader_versions(request, Ok(vec![loader_build("0.30.0", true)]), cx); }
        assert!(!view.validate_instance_draft(cx));
        assert!(matches!(&view.form, Some(Form::Instance(form)) if form.loader == LoaderKind::Quilt && form.loader_version.is_none()));
        view.choose_wizard_loader_build("0.30.0".into(), cx);
        assert!(view.validate_instance_draft(cx));
        assert!(view.snapshot.instances.is_empty());
    });
}

/// A real Loader section whose official catalog has no build for the chosen
/// loader and game. The rejected Done must not outlive the cancelled draft.
#[gpui::test]
fn loader_section_cancel_discards_the_unavailable_catalog_validation(cx: &mut TestAppContext) {
    use block_launcher::catalog::LoaderKind;

    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.preferences.reduce_motion = true;
        view.snapshot = ready_snapshot();
        view.snapshot.profile_mode = ProfileMode::Separated;
        view.versions = vec!["1.21.1".into()];
        view.begin_game_wizard(cx);
        view
    });
    cx.simulate_resize(size(px(960.), px(900.)));
    click(cx, "wizard-next");
    click(cx, "wizard-component-loader");
    view.read_with(cx, |view, _| {
        assert!(
            view.shell.surfaces.top().is_some_and(|layer| matches!(
                &layer.kind,
                crate::shell::Surface::WizardSection(WizardPage::Loader)
            )),
            "the Mod loader card must open its own section surface"
        );
        assert_eq!(view.shell.wizard.page, Some(WizardPage::Loader));
        assert!(matches!(&view.form, Some(Form::Instance(form))
            if form.loader == LoaderKind::Vanilla));
    });
    // Choose a loader whose official catalog resolves to zero builds through the
    // existing request boundary; no network or product hook is involved.
    click(cx, "wizard-loader-fabric");
    view.update(cx, |view, cx| {
        let request = view
            .shell
            .wizard
            .loader_catalog
            .as_ref()
            .map(|catalog| catalog.request);
        assert!(
            request.is_some(),
            "choosing Fabric must request its catalog"
        );
        if let Some(request) = request {
            view.receive_loader_versions(request, Ok(Vec::new()), cx);
        }
        assert!(view.wizard_loader_issue(cx).is_some());
        assert!(matches!(&view.form, Some(Form::Instance(form))
            if form.loader == LoaderKind::Fabric && form.loader_version.is_none()));
    });
    click(cx, "wizard-next");
    view.read_with(cx, |view, _| {
        assert_eq!(view.shell.wizard.page, Some(WizardPage::Loader));
        assert_eq!(view.shell.wizard.error_field, Some(WizardField::Loader));
        assert!(
            error_text(view)
                .as_deref()
                .is_some_and(|error| error.contains("no official build")),
            "the section Done must reject the unavailable official catalog"
        );
    });
    assert!(
        crate::test_support::control_bounds(cx, "wizard-error").is_some(),
        "the rejection must be painted in the section"
    );
    click(cx, "section-cancel");
    view.read_with(cx, |view, _| {
        assert!(
            !view.shell.surfaces.is_open(),
            "the section surface must close"
        );
        assert_eq!(view.shell.wizard.page, None, "the outer page is restored");
        assert!(
            error_text(view).is_none(),
            "the cancelled loader draft's catalog error must not survive"
        );
        assert!(view.shell.wizard.error_field.is_none());
        assert!(
            view.shell.wizard.reveal.get().is_none(),
            "the rejected Loader Done leaves a pending reveal that cancellation must discard"
        );
        assert!(matches!(&view.form, Some(Form::Instance(form))
            if form.loader == LoaderKind::Vanilla && form.loader_version.is_none()));
        assert!(view.shell.wizard.loader_catalog.is_none());
        assert!(view.shell.wizard.loader_request.is_none());
    });
    assert!(
        crate::test_support::control_bounds(cx, "wizard-error").is_none(),
        "the outer page must not paint the cancelled loader draft's error"
    );
    // Reopening starts from the restored Vanilla checkpoint with no old error.
    click(cx, "wizard-component-loader");
    view.read_with(cx, |view, cx| {
        assert_eq!(view.shell.wizard.page, Some(WizardPage::Loader));
        assert!(error_text(view).is_none());
        assert!(view.shell.wizard.reveal.get().is_none());
        assert!(view.wizard_loader_issue(cx).is_none());
    });
    assert!(crate::test_support::control_bounds(cx, "wizard-error").is_none());
    click(cx, "wizard-next");
    view.read_with(cx, |view, _| {
        assert!(!view.shell.surfaces.is_open());
        assert_eq!(view.shell.wizard.page, None);
        assert!(error_text(view).is_none());
    });
}

fn picker_catalog_entry(
    id: &str,
    kind: block_launcher::manifest::model::VersionType,
    release_time: &str,
) -> block_launcher::manifest::model::VersionEntry {
    block_launcher::manifest::model::VersionEntry {
        id: id.into(),
        kind,
        url: format!("https://example.invalid/{id}.json"),
        time: release_time.into(),
        release_time: release_time.into(),
        sha1: "a".repeat(40),
        compliance_level: 1,
    }
}

fn picker_catalog() -> Vec<block_launcher::manifest::model::VersionEntry> {
    use block_launcher::manifest::model::VersionType;
    vec![
        picker_catalog_entry("1.21.1", VersionType::Release, "2024-08-08T12:21:54+00:00"),
        picker_catalog_entry("1.20.6", VersionType::Release, "2024-04-29T12:47:07+00:00"),
        picker_catalog_entry("1.18.2", VersionType::Release, "2022-02-28T11:22:16+00:00"),
        picker_catalog_entry("1.16.5", VersionType::Release, "2021-01-14T16:05:29+00:00"),
        picker_catalog_entry("1.13", VersionType::Release, "2018-07-18T12:57:53+00:00"),
        picker_catalog_entry("1.12.2", VersionType::Release, "2017-09-18T11:33:40+00:00"),
        picker_catalog_entry("1.8.9", VersionType::Release, "2015-12-09T09:19:57+00:00"),
    ]
}

#[gpui::test]
fn arm64_version_picker_disables_lwjgl2_versions_with_the_reason(cx: &mut TestAppContext) {
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.preferences.reduce_motion = true;
        view.host_arch_override = Some(block_launcher::platform::Architecture::Aarch64);
        view.version_catalog = picker_catalog();
        view.begin_game_wizard(cx);
        view
    });
    view.update(cx, |view, cx| {
        use block_launcher::support::LWJGL2_ARM64_REASON;
        assert_eq!(
            view.wizard_version_restriction("1.12.2"),
            Some(LWJGL2_ARM64_REASON)
        );
        assert_eq!(
            view.wizard_version_restriction("1.8.9"),
            Some(LWJGL2_ARM64_REASON)
        );
        assert_eq!(view.wizard_version_restriction("1.16.5"), None);
        assert_eq!(view.wizard_version_restriction("1.18.2"), None);

        // Mouse/keyboard rows and the version setter cannot choose the version.
        view.set_wizard_version("1.20.6".into(), cx);
        view.set_wizard_version("1.12.2".into(), cx);
        assert!(
            matches!(&view.form, Some(Form::Instance(form)) if form.version.read(cx).value() == "1.20.6")
        );
        assert!(view.shell.wizard.version_notice.as_ref().is_some_and(|notice| {
            notice.text(Language::English).contains(LWJGL2_ARM64_REASON)
        }));
        assert!(view.shell.wizard.version_notice.as_ref().is_some_and(|notice| {
            notice.text(Language::SimplifiedChinese).starts_with("Minecraft 1.12.2 无法在 ARM64 上原生运行：")
                && notice.text(Language::Japanese).starts_with("Minecraft 1.12.2 は ARM64 でネイティブ実行できません:")
                && notice.text(Language::Japanese).contains(LWJGL2_ARM64_REASON)
        }));

        // An exact ID typed into the field is rejected by validation with the same reason.
        view.open_game_wizard_page(Some(WizardPage::Version), cx);
        if let Some(Form::Instance(form)) = &view.form {
            form.version.update(cx, |field, cx| field.set_value("1.12.2", cx));
        }
        assert!(!view.validate_wizard_section(WizardPage::Version, cx));
        assert!(
            error_text(view).is_some_and(|error| error.contains(LWJGL2_ARM64_REASON)),
            "{:?}",
            error_text(view)
        );
    });
    cx.simulate_resize(size(px(960.), px(600.)));
    cx.run_until_parked();
}

#[gpui::test]
fn x64_version_picker_keeps_lwjgl2_versions_selectable(cx: &mut TestAppContext) {
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.preferences.reduce_motion = true;
        view.host_arch_override = Some(block_launcher::platform::Architecture::X86_64);
        view.version_catalog = picker_catalog();
        view.begin_game_wizard(cx);
        view
    });
    view.update(cx, |view, cx| {
        assert_eq!(view.wizard_version_restriction("1.12.2"), None);
        view.set_wizard_version("1.12.2".into(), cx);
        assert!(
            matches!(&view.form, Some(Form::Instance(form)) if form.version.read(cx).value() == "1.12.2")
        );
        view.open_game_wizard_page(Some(WizardPage::Version), cx);
        assert!(view.validate_wizard_section(WizardPage::Version, cx));
    });
    cx.simulate_resize(size(px(960.), px(600.)));
    cx.run_until_parked();
}

#[gpui::test]
fn profile_memory_validation_and_values_use_gb_with_one_to_thirty_two_gb_bounds(
    cx: &mut TestAppContext,
) {
    let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
    view.update(cx, |view, cx| {
        view.preferences.reduce_motion = true;
        view.begin_game_wizard(cx);
        let Some(profile) = view.shell.wizard.profile.as_mut() else {
            return;
        };
        profile
            .memory
            .update(cx, |field, cx| field.set_value("0.5", cx));
        assert_eq!(profile.invalid_field(cx), Some(WizardField::Memory));
        profile
            .memory
            .update(cx, |field, cx| field.set_value("33", cx));
        assert_eq!(profile.invalid_field(cx), Some(WizardField::Memory));
        profile
            .memory
            .update(cx, |field, cx| field.set_value("1", cx));
        assert_eq!(profile.invalid_field(cx), None);
        let one = profile.values(cx);
        assert!(one.is_ok());
        if let Ok(one) = one {
            assert_eq!(one.defaults.memory_mib, 1024);
        }
        profile
            .memory
            .update(cx, |field, cx| field.set_value("32", cx));
        let max = profile.values(cx);
        assert!(max.is_ok());
        if let Ok(max) = max {
            assert_eq!(max.defaults.memory_mib, 32768);
        }
        profile
            .memory
            .update(cx, |field, cx| field.set_value("1.5", cx));
        let finer = profile.values(cx);
        assert!(finer.is_ok());
        if let Ok(finer) = finer {
            assert_eq!(finer.defaults.memory_mib, 1536);
        }
    });
}

/// Footer labels of a fresh wizard rendered for `language`, read from the
/// painted controls.
fn wizard_footer_labels(
    cx: &mut TestAppContext,
    language: Language,
) -> (Option<String>, Option<String>) {
    let (_view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.preferences.reduce_motion = true;
        view.preferences.language = Some(language);
        view.begin_game_wizard(cx);
        view
    });
    cx.simulate_resize(size(px(960.), px(600.)));
    cx.run_until_parked();
    cx.update(|window, _| {
        let label = |selector: &'static str| {
            gpui_base::test_support::find_all(window, &[], &selector.into())
                .first()
                .and_then(|control| control.label())
                .map(str::to_owned)
        };
        (label("wizard-next"), label("cancel-instance"))
    })
}

#[gpui::test]
fn wizard_footer_copy_is_chinese_for_a_chinese_view(cx: &mut TestAppContext) {
    assert_eq!(
        wizard_footer_labels(cx, Language::SimplifiedChinese),
        (Some("继续".to_owned()), Some("取消".to_owned()))
    );
}

#[gpui::test]
fn wizard_footer_copy_is_japanese_for_a_japanese_view(cx: &mut TestAppContext) {
    assert_eq!(
        wizard_footer_labels(cx, Language::Japanese),
        (Some("次へ".to_owned()), Some("キャンセル".to_owned()))
    );
}

#[gpui::test]
fn retained_notice_and_error_keep_their_cause_and_repaint_per_language(cx: &mut TestAppContext) {
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.preferences.reduce_motion = true;
        view.begin_game_wizard(cx);
        view
    });
    view.update(cx, |view, cx| {
        if let Some(Form::Instance(form)) = &mut view.form {
            form.loader = block_launcher::catalog::LoaderKind::Forge;
            form.loader_version = Some("52.0.16".into());
            form.optifine = Some(Box::new(compatible_optifine()));
        }
        view.choose_wizard_loader(block_launcher::catalog::LoaderKind::Fabric, cx);
    });
    let notice = view.read_with(cx, |view, _| view.shell.wizard.version_notice.clone());
    assert!(notice.is_some(), "removing OptiFine must retain a notice");
    if let Some(notice) = notice {
        assert_eq!(
            notice.text(Language::English),
            "OptiFine was removed because it cannot be combined directly with Fabric."
        );
        assert_eq!(
            notice.text(Language::SimplifiedChinese),
            "由于无法与 Fabric 直接组合使用，OptiFine 已被移除。"
        );
        assert_eq!(
            notice.text(Language::Japanese),
            "Fabric と直接組み合わせられないため、OptiFine を削除しました。"
        );
    }
    view.update(cx, |view, cx| {
        view.open_game_wizard_page(Some(WizardPage::Version), cx);
        if let Some(Form::Instance(form)) = &mut view.form {
            form.version.update(cx, |field, cx| field.set_value("", cx));
        }
        assert!(!view.validate_wizard_section(WizardPage::Version, cx));
    });
    view.read_with(cx, |view, _| {
        assert_eq!(
            view.shell.wizard.error_text(Language::English).as_deref(),
            Some("Choose a Minecraft version from the list.")
        );
        assert_eq!(
            view.shell
                .wizard
                .error_text(Language::SimplifiedChinese)
                .as_deref(),
            Some("请从列表中选择 Minecraft 版本。")
        );
        assert_eq!(
            view.shell.wizard.error_text(Language::Japanese).as_deref(),
            Some("リストから Minecraft バージョンを選択してください。")
        );
    });
}

#[test]
fn feature_locale_covers_every_key_in_every_language() -> anyhow::Result<()> {
    let document: serde_json::Value =
        serde_json::from_str(include_str!("../../locales/wizard.json"))?;
    let entries = document
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("the locale document must be an object"))?;
    assert_eq!(
        entries.get("_version").and_then(serde_json::Value::as_u64),
        Some(2)
    );
    for (key, translations) in entries.iter().filter(|(key, _)| key.as_str() != "_version") {
        let translations = translations
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("{key} must carry one copy per language"))?;
        for language in ["en", "zh-Hans", "ja"] {
            let copy = translations
                .get(language)
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            assert!(!copy.trim().is_empty(), "{key} has no {language} copy");
        }
    }
    Ok(())
}

#[test]
fn whole_messages_are_translated_with_their_arguments() {
    assert_eq!(tr!(Language::English, "wizard.footer.continue"), "Continue");
    assert_eq!(
        tr!(Language::SimplifiedChinese, "wizard.footer.continue"),
        "继续"
    );
    assert_eq!(tr!(Language::Japanese, "wizard.footer.continue"), "次へ");
    assert_eq!(
        tr!(
            Language::SimplifiedChinese,
            "wizard.rail.stepOf",
            current = 2,
            total = 4
        ),
        "第 2 步，共 4 步"
    );
    assert_eq!(
        tr!(
            Language::Japanese,
            "wizard.rail.stepOf",
            current = 2,
            total = 4
        ),
        "4 ステップ中 2 ステップ目"
    );
    assert_eq!(
        tr!(
            Language::Japanese,
            "wizard.version.source.catalog",
            kind = "安定版"
        ),
        "公式カタログ · 安定版"
    );
}

/// The installation-folder chooser is raised and cleared through its identity,
/// so a shared owner never has to compare painted text.
#[gpui::test]
fn install_chooser_failure_is_raised_and_cleared_by_identity(cx: &mut TestAppContext) {
    let (view, cx) = crate::test_support::window(cx, |window, cx| {
        let mut view = crate::tests::test_view(window, cx);
        view.preferences.reduce_motion = true;
        view.begin_game_wizard(cx);
        view
    });
    view.update(cx, |view, _| {
        let wizard = &mut view.shell.wizard;
        assert_eq!(wizard.failure(), None);
        wizard.set_failure(WizardFailure::ChooserFailed);
        assert_eq!(wizard.failure(), Some(WizardFailure::ChooserFailed));
        assert_eq!(
            wizard.error_text(Language::English).as_deref(),
            Some("Could not open the installation folder chooser. Check your system file picker and try again.")
        );
        assert_eq!(
            wizard.error_text(Language::SimplifiedChinese).as_deref(),
            Some("无法打开安装文件夹选择器。请检查系统文件选择器后重试。")
        );
        assert_eq!(
            wizard.error_text(Language::Japanese).as_deref(),
            Some("インストールフォルダーの選択画面を開けませんでした。システムのファイル選択画面を確認して再試行してください。")
        );
        // A newer report replaces the cause instead of being hidden by it.
        wizard.error = Some("Engine unavailable".into());
        assert_eq!(wizard.failure(), None);
        assert_eq!(wizard.error_text(Language::English).as_deref(), Some("Engine unavailable"));
        // Clearing another identity leaves the painted message alone.
        wizard.clear_failure(WizardFailure::ChooserFailed);
        assert_eq!(wizard.error_text(Language::English).as_deref(), Some("Engine unavailable"));
        // Raising it again and clearing it removes it.
        wizard.set_failure(WizardFailure::ChooserFailed);
        wizard.clear_failure(WizardFailure::ChooserFailed);
        assert_eq!(wizard.failure(), None);
        assert_eq!(wizard.error_text(Language::English), None);
        assert!(!wizard.has_error());
    });
}
