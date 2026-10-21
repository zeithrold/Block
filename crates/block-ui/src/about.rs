//! Product information from the running binary, with no preview build metadata.

use super::scroll_area::ScrollAreaExt as _;
use gpui::{
    ClipboardItem, Context, InteractiveElement as _, IntoElement, ParentElement as _, Styled as _,
    Window, div, px, relative, rgb,
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
    pub(super) fn render_about(
        &self,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement + use<> {
        let narrow = patterns::is_narrow(window.viewport_size());
        let brand = self.about_brand(narrow);
        let body = self.about_body(narrow, cx);
        let footer = patterns::management_footer(window.viewport_size(), self.palette)
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
        div()
            .id("about-page")
            .size_full()
            .min_h_0()
            .min_w_0()
            .flex()
            .overflow_hidden()
            .child(brand)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(body)
                    .child(footer),
            )
    }

    fn about_brand(&self, narrow: bool) -> impl IntoElement + use<> {
        let info = self.build_info;
        div()
            .id("about-brand")
            .debug_selector(|| "about-brand".into())
            .w(relative(if narrow {
                0.24
            } else {
                super::design::BROWSER_FRACTION
            }))
            .min_w_0()
            .h_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(if narrow { 8. } else { 20. }))
            .px(px(if narrow { 8. } else { 20. }))
            .border_r_1()
            .border_color(rgb(self.palette.border))
            .bg(rgb(self.palette.sidebar))
            .child(super::shell::brand(if narrow { 48. } else { 144. }))
            .child(
                div()
                    .text_size(px(if narrow { 14. } else { 30. }))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child("Block"),
            )
            .child(components::badge(
                format!("v{}", info.version),
                self.palette,
                BadgeVariant::Secondary,
            ))
            .children((!narrow).then(|| note("One block at a time.")))
    }

    fn about_body(&self, narrow: bool, cx: &Context<'_, Self>) -> impl IntoElement + use<> {
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
                    .gap(px(if narrow { 20. } else { 28. }))
                    .p(px(if narrow { 16. } else { 32. }))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(note("MINECRAFT: JAVA EDITION LAUNCHER"))
                            .child(
                                div()
                                    .text_size(px(if narrow { 18. } else { 24. }))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child("Made for your next world."),
                            )
                            .child(note("A quiet home for your accounts, games, and the way you play. Built around persistent Instances and the Profiles that make them yours.")),
                    )
                    .child(self.about_build_information(narrow, cx))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_2()
                            .child(self.about_link("about-source", "View source", REPOSITORY, ButtonVariant::Outline))
                            .child(self.about_link("about-feedback", "Feedback", FEEDBACK, ButtonVariant::Ghost)),
                    )
                    .child(note("Built with Rust and GPUI. Game errors open Crash help with recovery actions and diagnostic export. You can inspect the current game output below."))
                    .child(note("Brand artwork: Block-Icon, icon-v4. Interface icons: Lucide. Font: Inter.")),
            );
        if self.show_output {
            body = body.child(
                div()
                    .id("about-diagnostics")
                    .p(px(if narrow { 16. } else { 32. }))
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

    fn about_build_information(
        &self,
        narrow: bool,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement + use<> {
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
                    div().w(if narrow { relative(1.) } else { relative(0.5) })
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
            view.tab = Tab::About;
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
    fn about_keeps_its_brand_and_footer_usable_while_diagnostics_scroll(cx: &mut TestAppContext) {
        let (_view, cx) = crate::test_support::window(cx, |window, cx| {
            let mut view = crate::tests::test_view(window, cx);
            view.tab = Tab::About;
            view.preferences.reduce_motion = true;
            view
        });
        for (width, height) in [(960., 600.), (640., 540.), (360., 540.)] {
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
