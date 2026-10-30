//! Product information from the running binary, with no preview build metadata.

use super::scroll_area::ScrollAreaExt as _;
use gpui::{
    ClipboardItem, Context, InteractiveElement as _, IntoElement, ParentElement as _, Styled as _,
    div, px, relative, rgb,
};

use super::{
    Block, UiAction,
    components::{self, BadgeVariant, ButtonVariant, ControlState},
    design::Icon,
    feedback::ToastKind,
    note, patterns,
};

const REPOSITORY: &str = "https://github.com/zeithrold/Block";
const FEEDBACK: &str = "https://github.com/zeithrold/Block/issues";
const WEBSITE: &str = "https://block.ztd.one";

/// Immutable identity supplied by the executable that opened the launcher.
/// Missing source stamps remain explicit for manual local builds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BuildInfo {
    /// The executable's Cargo package version.
    pub version: &'static str,
    /// The executable's target operating system.
    pub os: &'static str,
    /// The executable's target architecture.
    pub arch: &'static str,
    /// Source manifest identity stamped by the native verification tools.
    pub source_sha256: Option<&'static str>,
    /// Commit identity, when the build was stamped with one.
    pub source_commit: Option<&'static str>,
}

impl Default for BuildInfo {
    fn default() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION"),
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
            source_sha256: None,
            source_commit: None,
        }
    }
}

impl BuildInfo {
    fn diagnostic_text(self) -> String {
        serde_json::json!({
            "product_id": "one.ztd.block",
            "version": self.version,
            "os": self.os,
            "arch": self.arch,
            "source_sha256": self.source_sha256,
            "source_commit": self.source_commit,
        })
        .to_string()
    }
}

impl Block {
    pub(super) fn render_about(&self, cx: &Context<'_, Self>) -> super::page_host::ManagementSlots {
        let brand = self.about_brand();
        let body = self.about_body(cx);
        let footer = patterns::management_footer(self.palette)
            .id("about-footer")
            .debug_selector(|| "about-footer".into())
            .justify_between()
            .child(self.about_link("product-website", "Website", WEBSITE, ButtonVariant::Ghost))
            .child(self.button(
                "show-output",
                if self.show_output {
                    "Hide output"
                } else {
                    "View output"
                },
                UiAction::ToggleOutput,
                cx,
            ));
        super::page_host::ManagementSlots {
            sidebar: brand.into_any_element(),
            detail: div()
                .size_full()
                .min_w_0()
                .min_h_0()
                .flex()
                .flex_col()
                .child(body)
                .child(footer)
                .into_any_element(),
        }
    }

    fn about_brand(&self) -> gpui::Stateful<gpui::Div> {
        let info = self.build_info;
        div()
            .id("about-brand")
            .debug_selector(|| "about-brand".into())
            .w_full()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_5()
            .px_5()
            .child(super::shell::brand(144.))
            .child(
                div()
                    .text_size(px(30.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child("Block"),
            )
            .child(components::badge(
                format!("v{}", info.version),
                self.palette,
                BadgeVariant::Secondary,
            ))
            .child(note("One block at a time."))
    }

    fn about_body(&self, cx: &Context<'_, Self>) -> impl IntoElement + use<> {
        let mut body = div()
            .id("about-body")
            .debug_selector(|| "about-body".into())
            .flex_1()
            .min_w_0()
            .min_h_0()
            .scroll_area(&self.shell.about_scroll, "About Block", self.palette, false)
            .child(
                div()
                    .debug_selector(|| "about-scroll-content".into())
                    .min_h_full()
                    .flex()
                    .flex_col()
                    .gap_7()
                    .p_8()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(note("MINECRAFT: JAVA EDITION LAUNCHER"))
                            .child(
                                div()
                                    .text_size(px(24.))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child("Made for your next world."),
                            )
                            .child(note("A quiet home for your accounts, games, and the way you play. Built around persistent Instances and the Profiles that make them yours.")),
                    )
                    .child(self.about_build_information(cx))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_2()
                            .child(self.about_link("about-source", "View source", REPOSITORY, ButtonVariant::Outline))
                            .child(self.about_link("about-feedback", "Feedback", FEEDBACK, ButtonVariant::Ghost))
                            .child(self.button("about-open-logs", "Open logs", UiAction::OpenLogs, cx))
                            .child(self.button(
                                "about-export-logs",
                                if self.log_export_cancel.is_some() {
                                    "Waiting for save dialog…"
                                } else if self.exporting_logs {
                                    "Exporting…"
                                } else {
                                    "Export logs…"
                                },
                                UiAction::ExportLogs,
                                cx,
                            ))
                            .children(self.log_export_cancel.as_ref().map(|_| self.button(
                                "about-cancel-log-export", "Cancel export", UiAction::CancelLogExport, cx,
                            ))),
                    )
                    .child(note("Launcher logs only. Review before sharing."))
                    .child(note("Built with Rust and GPUI. Game errors open Crash help with recovery actions and diagnostic export. You can inspect the current game output below."))
                    .child(note("Brand artwork: Block-Icon, icon-v4. Interface icons: Lucide. Font: Inter.")),
            );
        if self.show_output {
            body = body.child(
                div()
                    .id("about-diagnostics")
                    .p_8()
                    .pt_0()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(self.output(cx))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_2()
                            .child(self.button(
                                "open-recipe",
                                "Open development recipe…",
                                UiAction::OpenRecipe,
                                cx,
                            ))
                            .children(self.recipe.as_ref().map(|_| {
                                self.button(
                                    "launch-recipe",
                                    "Launch development recipe",
                                    UiAction::LaunchRecipe,
                                    cx,
                                )
                            })),
                    ),
            );
        }
        body
    }

    fn about_build_information(&self, cx: &Context<'_, Self>) -> impl IntoElement + use<> {
        let info = self.build_info;
        let rows = [
            ("Version", info.version.to_owned()),
            ("Platform", format!("{} / {}", info.os, info.arch)),
            (
                "Source commit",
                info.source_commit
                    .map_or_else(|| "Not recorded in this build".into(), abbreviated_identity),
            ),
            (
                "Source fingerprint",
                info.source_sha256
                    .map_or_else(|| "Not recorded in this build".into(), abbreviated_identity),
            ),
        ];
        div()
            .border_t_1()
            .border_color(rgb(self.palette.border))
            .pt_5()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div().flex().items_center().justify_between().gap_3()
                    .child(div().text_sm().font_weight(gpui::FontWeight::MEDIUM).child("Build information"))
                    .child(super::tooltip::trigger(components::button(
                        "about-copy-build", "Copy", self.palette,
                        ButtonVariant::Ghost, ControlState::default(),
                        cx.listener(|view, (), _, cx| view.copy_build_information(cx)),
                    )
                        .child(Icon::Copy.render(14., self.palette.muted)), "Copy build information", cx)),
            )
            .child(
                div().flex().flex_wrap().gap_y_4().children(rows.into_iter().map(|(label, value)| {
                    div().w(relative(0.5))
                        .min_w_0().pr_3().flex().flex_col().gap_1()
                        .child(div().text_xs().text_color(rgb(self.palette.muted)).child(label))
                        .child(div().text_sm().child(value))
                })),
            )
            .child(note(if info.source_sha256.is_some() {
                "Copy build information to include the complete source identity in a report."
            } else {
                "This local build has no recorded source fingerprint. Copying build information preserves that fact."
            }))
    }

    fn about_link(
        &self,
        id: &'static str,
        label: &'static str,
        url: &'static str,
        variant: ButtonVariant,
    ) -> impl IntoElement + use<> {
        components::button_role(
            id,
            label,
            self.palette,
            variant,
            ControlState::default(),
            gpui::accesskit::Role::Link,
            move |(), _, cx| {
                cx.open_url(url);
            },
        )
        .child(Icon::ExternalLink.render(12., self.palette.muted))
    }

    fn copy_build_information(&mut self, cx: &mut Context<'_, Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(self.build_info.diagnostic_text()));
        self.push_toast("Build information copied".into(), ToastKind::Success);
        cx.notify();
    }

    pub(super) fn open_launcher_logs(&mut self, cx: &Context<'_, Self>) {
        match block_logging::log_directory() {
            Ok(directory) => {
                tracing::info!(event = "ui.logs.open_requested", view_id = self.view_id);
                cx.open_with_system(&directory);
            }
            Err(_) => {
                tracing::error!(event = "ui.logs.open_unavailable", view_id = self.view_id);
                self.push_toast("Could not locate launcher logs.".into(), ToastKind::Error);
            }
        }
    }

    pub(super) fn export_launcher_logs(&mut self, cx: &Context<'_, Self>) {
        if self.exporting_logs {
            return;
        }
        let directory = match block_logging::log_directory() {
            Ok(directory) => directory,
            Err(_) => {
                tracing::error!(event = "ui.logs.export_unavailable", view_id = self.view_id);
                self.push_toast("Could not locate launcher logs.".into(), ToastKind::Error);
                return;
            }
        };
        let selection = cx.prompt_for_new_path(&directory, Some("block-launcher-logs.zip"));
        let cancel = super::CancellationToken::new();
        self.exporting_logs = true;
        self.log_export_cancel = Some(cancel.clone());
        let view_id = self.view_id;
        tracing::info!(event = "ui.logs.export_started", view_id);
        cx.spawn(async move |view, cx| {
            // A picker may wait for the user indefinitely. Race explicit
            // cancellation, not a deadline that could expire a healthy dialog.
            let selection = tokio::select! {
                biased;
                () = cancel.cancelled() => None,
                selection = selection => Some(selection),
            };
            let (outcome, message, kind) = match selection {
                Some(Ok(Ok(Some(path)))) => {
                    // Once writing starts, cancellation is no longer offered.
                    // A released view must not start an unobserved export.
                    if view
                        .update(cx, |view, cx| {
                            view.log_export_cancel = None;
                            cx.notify();
                        })
                        .is_err()
                    {
                        return;
                    }
                    let result = cx
                        .background_executor()
                        .spawn(async move { block_logging::export_launcher_logs(&path) })
                        .await;
                    if result.is_ok() {
                        (
                            "succeeded",
                            "Launcher logs exported. Review before sharing.",
                            ToastKind::Success,
                        )
                    } else {
                        (
                            "failed",
                            "Could not export launcher logs. Try a new file name or folder.",
                            ToastKind::Error,
                        )
                    }
                }
                None | Some(Ok(Ok(None))) => ("cancelled", "Log export canceled.", ToastKind::Info),
                Some(Ok(Err(_)) | Err(_)) => (
                    "failed",
                    "The log export save dialog could not open.",
                    ToastKind::Error,
                ),
            };
            if outcome == "failed" {
                tracing::error!(event = "ui.logs.export_finished", view_id, outcome);
            } else {
                tracing::info!(event = "ui.logs.export_finished", view_id, outcome);
            }
            let _ = view.update(cx, |view, cx| {
                view.exporting_logs = false;
                view.log_export_cancel = None;
                view.push_toast(message.into(), kind);
                cx.notify();
            });
        })
        .detach();
    }
}

fn abbreviated_identity(identity: &str) -> String {
    let short: String = identity.chars().take(12).collect();
    if identity.chars().count() > 12 {
        format!("{short}…")
    } else {
        short
    }
}

#[cfg(test)]
mod tests {
    use super::BuildInfo;
    use crate::Tab;
    use gpui::{TestAppContext, VisualTestContext, point, px, size};

    fn click(cx: &mut VisualTestContext, selector: &'static str) {
        cx.run_until_parked();
        let bounds = crate::test_support::control_bounds(cx, selector);
        assert!(bounds.is_some(), "missing {selector}");
        if let Some(bounds) = bounds {
            let window_count = cx.windows().len();
            cx.simulate_click(bounds.center(), gpui::Modifiers::none());
            cx.run_until_parked();
            if cx.windows().len() > window_count
                && let Some(window) = cx.windows().last()
            {
                *cx = VisualTestContext::from_window(*window, &cx.cx);
            }
            crate::test_support::ensure_live_window(cx);
            cx.run_until_parked();
            crate::test_support::ensure_live_window(cx);
        }
    }

    #[test]
    fn manual_build_identity_keeps_absent_stamps_explicit() {
        let info = BuildInfo::default();
        let parsed: Result<serde_json::Value, _> = serde_json::from_str(&info.diagnostic_text());
        assert!(parsed.is_ok());
        if let Ok(parsed) = parsed {
            assert!(parsed["source_sha256"].is_null());
            assert!(parsed["source_commit"].is_null());
            assert_eq!(parsed["version"], env!("CARGO_PKG_VERSION"));
            assert_eq!(parsed["os"], std::env::consts::OS);
            assert_eq!(parsed["arch"], std::env::consts::ARCH);
        }
    }

    #[gpui::test]
    fn copy_preserves_full_executable_identity_and_reports_feedback(cx: &mut TestAppContext) {
        let (view, cx) = crate::test_support::window(cx, |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.router.navigate(Tab::About);
            view.preferences.reduce_motion = true;
            view.build_info = BuildInfo {
                source_sha256: Some(
                    "a1b2c3d4e5f6012345678901234567890123456789012345678901234567890ef",
                ),
                source_commit: Some("1234567890abcdef1234567890abcdef12345678"),
                ..BuildInfo::default()
            };
            view
        });
        click(cx, "about-copy-build");
        let copied = cx.read_from_clipboard().and_then(|item| item.text());
        assert_eq!(
            copied,
            Some(view.read_with(cx, |view, _| view.build_info.diagnostic_text()))
        );
        assert!(view.read_with(cx, |view, _| {
            view.shell
                .toast
                .as_ref()
                .is_some_and(|toast| toast.message == "Build information copied")
        }));
        click(cx, "product-website");
        assert_eq!(cx.opened_url().as_deref(), Some(super::WEBSITE));
    }

    #[gpui::test]
    fn log_export_waits_for_a_destination_and_cancellation_clears_busy_state(
        cx: &mut TestAppContext,
    ) {
        assert!(block_logging::log_directory().is_ok());
        let (view, cx) = crate::test_support::window(cx, |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.router.navigate(Tab::About);
            view.preferences.reduce_motion = true;
            view
        });
        click(cx, "about-export-logs");
        assert!(cx.did_prompt_for_new_path());
        assert!(view.read_with(cx, |view, _| view.exporting_logs));
        cx.simulate_new_path_selection(|_| None);
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| {
            !view.exporting_logs
                && view
                    .shell
                    .toast
                    .as_ref()
                    .is_some_and(|toast| toast.message == "Log export canceled.")
        }));
    }

    #[gpui::test]
    fn stalled_log_picker_can_be_cancelled_and_its_late_result_cannot_affect_a_retry(
        cx: &mut TestAppContext,
    ) {
        let directory = tempfile::tempdir();
        assert!(directory.is_ok());
        let Ok(directory) = directory else {
            return;
        };
        let late_path = directory.path().join("late-export.zip");
        let (view, cx) = crate::test_support::window(cx, |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.router.navigate(Tab::About);
            view.preferences.reduce_motion = true;
            view
        });
        click(cx, "about-export-logs");
        cx.executor()
            .advance_clock(std::time::Duration::from_secs(120));
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| view.exporting_logs));
        click(cx, "about-cancel-log-export");
        assert!(!view.read_with(cx, |view, _| view.exporting_logs));

        click(cx, "about-export-logs");
        cx.simulate_new_path_selection(|_| Some(late_path.clone()));
        cx.run_until_parked();
        assert!(!late_path.exists(), "a canceled selection must never write");
        assert!(view.read_with(cx, |view, _| view.exporting_logs));
        cx.simulate_new_path_selection(|_| None);
        cx.run_until_parked();
        assert!(!view.read_with(cx, |view, _| view.exporting_logs));
    }

    #[gpui::test]
    fn releasing_the_view_abandons_the_picker_without_writing(cx: &mut TestAppContext) {
        let directory = tempfile::tempdir();
        assert!(directory.is_ok());
        let Ok(directory) = directory else {
            return;
        };
        let path = directory.path().join("released-view.zip");
        let (view, cx) = crate::test_support::window(cx, |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.router.navigate(Tab::About);
            view.preferences.reduce_motion = true;
            view
        });
        click(cx, "about-export-logs");
        drop(view);
        cx.update(|window, _| window.remove_window());
        cx.simulate_new_path_selection(|_| Some(path.clone()));
        cx.run_until_parked();
        assert!(!path.exists(), "a released view must not start an export");
    }

    #[gpui::test]
    fn releasing_a_visual_snapshot_does_not_cancel_the_live_log_picker(cx: &mut TestAppContext) {
        let (view, cx) = crate::test_support::window(cx, |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.router.navigate(Tab::About);
            view.preferences.reduce_motion = true;
            view
        });
        click(cx, "about-export-logs");
        view.update(cx, |view, cx| {
            drop(view.visual_snapshot(cx));
            assert!(
                view.log_export_cancel
                    .as_ref()
                    .is_some_and(|cancel| !cancel.is_cancelled())
            );
        });
        cx.simulate_new_path_selection(|_| None);
        cx.run_until_parked();
        assert!(!view.read_with(cx, |view, _| view.exporting_logs));
    }

    #[gpui::test]
    fn about_keeps_its_brand_and_footer_usable_while_diagnostics_scroll(cx: &mut TestAppContext) {
        let (_view, cx) = crate::test_support::window(cx, |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.router.navigate(Tab::About);
            view.preferences.reduce_motion = true;
            view
        });
        for (width, height) in [(1152., 720.), (960., 600.)] {
            cx.simulate_resize(size(px(width), px(height)));
            cx.run_until_parked();
            for expanded in [false, true] {
                let brand = cx.debug_bounds("about-brand");
                let body = cx.debug_bounds("about-body");
                let footer = cx.debug_bounds("about-footer");
                assert!(brand.is_some() && body.is_some() && footer.is_some());
                if let (Some(brand), Some(body), Some(footer)) = (brand, body, footer) {
                    assert!(body.size.height > px(0.) && body.bottom() <= footer.top());
                    assert!(footer.right() <= px(width) && footer.bottom() <= px(height));
                    assert!(brand.right() <= body.left());
                    for selector in ["product-website", "show-output"] {
                        let bounds = crate::test_support::control_bounds(cx, selector);
                        assert!(bounds.is_some_and(|bounds| bounds.left() >= footer.left()
                            && bounds.right() <= footer.right()
                            && bounds.top() >= footer.top()
                            && bounds.bottom() <= footer.bottom()));
                    }
                    cx.simulate_mouse_move(
                        point(body.left() + px(12.), body.top() + px(12.)),
                        None,
                        gpui::Modifiers::none(),
                    );
                    if expanded {
                        let content_before = cx.debug_bounds("about-scroll-content");
                        cx.simulate_event(gpui::ScrollWheelEvent {
                            position: point(body.left() + px(12.), body.top() + px(12.)),
                            delta: gpui::ScrollDelta::Pixels(point(px(0.), px(-1600.))),
                            ..gpui::ScrollWheelEvent::default()
                        });
                        cx.run_until_parked();
                        let content_after = cx.debug_bounds("about-scroll-content");
                        assert!(
                            content_before
                                .zip(content_after)
                                .is_some_and(|(before, after)| after.top() < before.top())
                        );
                        assert_eq!(cx.debug_bounds("about-footer"), Some(footer));
                        cx.simulate_event(gpui::ScrollWheelEvent {
                            position: point(body.left() + px(12.), body.top() + px(12.)),
                            delta: gpui::ScrollDelta::Pixels(point(px(0.), px(1600.))),
                            ..gpui::ScrollWheelEvent::default()
                        });
                    }
                }
                if !expanded {
                    click(cx, "show-output");
                }
            }
            click(cx, "show-output");
        }
    }
}
