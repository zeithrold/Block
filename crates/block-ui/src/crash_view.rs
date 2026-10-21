//! A local help window for an already sanitized game-session report.

use std::{fmt, fmt::Write as _, path::PathBuf};

use block_launcher::crash::{Confidence, CrashReport, ExitKind};
use gpui::{
    App, ClipboardItem, Context, FocusHandle, InteractiveElement as _, IntoElement, KeyBinding,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Window,
    div, rgb,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HelpAction {
    ReturnToProfile,
    OpenFolder,
    Export,
    CopySummary,
    ToggleDetails,
    Close,
}

const ACTIONS: [HelpAction; 6] = [
    HelpAction::ReturnToProfile,
    HelpAction::OpenFolder,
    HelpAction::Export,
    HelpAction::CopySummary,
    HelpAction::ToggleDetails,
    HelpAction::Close,
];

#[derive(Clone, Debug, Default, PartialEq, Eq, gpui::Action)]
#[action(namespace = crash_help)]
struct NextControl;
#[derive(Clone, Debug, Default, PartialEq, Eq, gpui::Action)]
#[action(namespace = crash_help)]
struct PreviousControl;
#[derive(Clone, Debug, Default, PartialEq, Eq, gpui::Action)]
#[action(namespace = crash_help)]
struct ActivateControl;
#[derive(Clone, Debug, Default, PartialEq, Eq, gpui::Action)]
#[action(namespace = crash_help)]
struct CloseHelp;

type ReturnToProfile = Box<dyn FnMut(&mut Window, &mut App)>;

/// Displays facts and possible explanations without reading original game logs.
///
/// The report must already be redacted, including its context and attachments.
/// Returning to the launcher does not restart Minecraft automatically.
pub struct CrashView {
    report: CrashReport,
    report_directory: PathBuf,
    return_to_profile: ReturnToProfile,
    focus: FocusHandle,
    focused_control: usize,
    show_details: bool,
    exporting: bool,
    status: Option<String>,
}

impl fmt::Debug for CrashView {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CrashView")
            .field("run_id", &self.report.context.run_id)
            .field("show_details", &self.show_details)
            .field("exporting", &self.exporting)
            .finish_non_exhaustive()
    }
}

impl CrashView {
    #[must_use]
    pub fn new(
        report: CrashReport,
        report_directory: PathBuf,
        return_to_profile: impl FnMut(&mut Window, &mut App) + 'static,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        super::initialize(cx);
        cx.bind_keys([
            KeyBinding::new("tab", NextControl, Some("CrashHelp")),
            KeyBinding::new("shift-tab", PreviousControl, Some("CrashHelp")),
            KeyBinding::new("enter", ActivateControl, Some("CrashHelp")),
            KeyBinding::new("space", ActivateControl, Some("CrashHelp")),
            KeyBinding::new("escape", CloseHelp, Some("CrashHelp")),
        ]);
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        Self {
            report,
            report_directory,
            return_to_profile: Box::new(return_to_profile),
            focus,
            focused_control: 0,
            show_details: false,
            exporting: false,
            status: None,
        }
    }

    fn handle(&mut self, action: HelpAction, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.focus.focus(window, cx);
        if let Some(index) = ACTIONS.iter().position(|candidate| *candidate == action) {
            self.focused_control = index;
        }
        match action {
            HelpAction::ReturnToProfile => (self.return_to_profile)(window, cx),
            HelpAction::OpenFolder => cx.reveal_path(&self.report_directory),
            HelpAction::Export => self.export(cx),
            HelpAction::CopySummary => {
                cx.write_to_clipboard(ClipboardItem::new_string(diagnostic_summary(&self.report)));
                self.status = Some("Diagnostic summary copied. Review it before sharing.".into());
            }
            HelpAction::ToggleDetails => self.show_details = !self.show_details,
            HelpAction::Close => window.remove_window(),
        }
        cx.notify();
    }

    fn export(&mut self, cx: &Context<'_, Self>) {
        if self.exporting {
            return;
        }
        let Ok(serialized) = serde_json::to_vec_pretty(&self.report) else {
            self.status =
                Some("Could not prepare diagnostics. You can still copy the summary.".into());
            return;
        };
        let selection =
            cx.prompt_for_new_path(&self.report_directory, Some("minecraft-diagnostics.json"));
        self.exporting = true;
        self.status = Some("Choose where to save the sanitized diagnostic report.".into());
        cx.spawn(async move |view, cx| {
            let status = match selection.await {
                Ok(Ok(Some(path))) => {
                    let saved = cx
                        .background_executor()
                        .spawn(async move { std::fs::write(path, serialized) })
                        .await;
                    if saved.is_ok() {
                        "Diagnostics exported. Review the report before sharing."
                    } else {
                        "Could not save diagnostics. Try another folder."
                    }
                }
                Ok(Ok(None)) => "Export canceled.",
                Ok(Err(_)) | Err(_) => {
                    "The save dialog could not open. You can still copy the summary."
                }
            };
            let _ = view.update(cx, |view, cx| {
                view.exporting = false;
                view.status = Some(status.into());
                cx.notify();
            });
        })
        .detach();
    }

    fn button(
        &self,
        id: &'static str,
        label: impl Into<SharedString>,
        action: HelpAction,
        cx: &Context<'_, Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let primary = action == HelpAction::ReturnToProfile;
        div()
            .id(id)
            .debug_selector(move || id.to_owned())
            .px_3()
            .py_2()
            .rounded_md()
            .border_1()
            .border_color(rgb(if ACTIONS[self.focused_control] == action {
                0x008f_bde0
            } else {
                0x0033_4857
            }))
            .bg(rgb(if primary { 0x002d_617c } else { 0x0029_3c4a }))
            .text_sm()
            .cursor_pointer()
            .hover(|style| style.bg(rgb(0x0036_5363)))
            .on_click(cx.listener(move |view, _, window, cx| view.handle(action, window, cx)))
            .child(label.into())
    }

    fn details(&self) -> gpui::Stateful<gpui::Div> {
        let mut details = super::panel()
            .id("crash-technical-details")
            .debug_selector(|| "crash-technical-details".to_owned())
            .gap_3()
            .child(div().text_sm().child("Technical details"))
            .child(super::note(exit_details(&self.report)))
            .child(super::note(format!(
                "Session {}",
                self.report.context.run_id
            )))
            .child(div().text_sm().child("Recent game output"))
            .child(log_text(if self.report.sanitized_tail.is_empty() {
                "No game output was captured.".into()
            } else {
                self.report.sanitized_tail.clone()
            }));
        for attachment in &self.report.attachments {
            details = details
                .child(div().text_sm().child(attachment.name.clone()))
                .child(log_text(attachment.sanitized_text.clone()));
            if attachment.truncated {
                details = details.child(super::note("This diagnostic attachment was truncated."));
            }
        }
        details
    }
}

impl Render for CrashView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let context = &self.report.context;
        let edition = context.loader.as_deref().unwrap_or("Vanilla");
        let mut causes = super::panel()
            .gap_3()
            .child(div().text_lg().child("What might have happened"));
        if self.report.diagnoses.is_empty() {
            causes = causes.child(super::note("No diagnosis is available for this session."));
        }
        for diagnosis in &self.report.diagnoses {
            let confidence = match diagnosis.confidence {
                Confidence::High => "Strong log evidence",
                Confidence::Possible => "Possible cause",
                Confidence::Unknown => "Cause not determined",
            };
            let mut cause = div()
                .flex()
                .flex_col()
                .gap_2()
                .child(super::note(confidence))
                .child(div().text_sm().child(diagnosis.title.clone()));
            for evidence in &diagnosis.evidence {
                cause = cause.child(log_text(evidence.clone()));
            }
            causes = causes
                .child(cause.child(div().text_sm().child(diagnosis.suggested_action.clone())));
        }
        let mut body = div()
            .id("crash-help-content")
            .size_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .p_6()
            .gap_4()
            .child(super::note("BLOCK · GAME SESSION HELP"))
            .child(div().text_2xl().child(headline(self.report.outcome.kind)))
            .child(div().text_sm().child(format!("{} · Minecraft {} · {edition}", context.profile_name, context.minecraft_version)))
            .child(super::note(self.report.summary.clone()))
            .child(causes)
            .child(div().flex().flex_wrap().gap_2()
                .child(self.button("crash-return", "Return to instance", HelpAction::ReturnToProfile, cx))
                .child(self.button("crash-open-folder", "Open report folder", HelpAction::OpenFolder, cx))
                .child(self.button("crash-export", if self.exporting { "Exporting…" } else { "Export diagnostics" }, HelpAction::Export, cx))
                .child(self.button("crash-copy", "Copy summary", HelpAction::CopySummary, cx))
                .child(self.button("crash-details", if self.show_details { "Hide technical details" } else { "View logs and details" }, HelpAction::ToggleDetails, cx))
                .child(self.button("crash-close", "Close", HelpAction::Close, cx)))
            .child(super::note("Diagnostics stay on this device until you choose to share them. Returning to the instance does not restart the game."));
        if let Some(status) = &self.status {
            body = body.child(div().text_sm().child(status.clone()));
        }
        if self.show_details {
            body = body.child(self.details());
        }
        div()
            .key_context("CrashHelp")
            .track_focus(&self.focus)
            .on_action(cx.listener(|view, _: &NextControl, _, cx| {
                view.focused_control = (view.focused_control + 1) % ACTIONS.len();
                cx.notify();
            }))
            .on_action(cx.listener(|view, _: &PreviousControl, _, cx| {
                view.focused_control = (view.focused_control + ACTIONS.len() - 1) % ACTIONS.len();
                cx.notify();
            }))
            .on_action(cx.listener(|view, _: &ActivateControl, window, cx| {
                view.handle(ACTIONS[view.focused_control], window, cx);
            }))
            .on_action(cx.listener(|view, _: &CloseHelp, window, cx| {
                view.handle(HelpAction::Close, window, cx);
            }))
            .size_full()
            .bg(rgb(0x0017_232d))
            .text_color(rgb(0x00e9_f0f4))
            .child(body)
    }
}

const fn headline(kind: ExitKind) -> &'static str {
    match kind {
        ExitKind::Normal => "Minecraft closed normally",
        ExitKind::Stopped => "Minecraft was stopped",
        ExitKind::StartFailed => "Minecraft could not start",
        ExitKind::Abnormal => "Minecraft closed unexpectedly",
        ExitKind::MonitoringInterrupted => "Game monitoring was interrupted",
    }
}

fn exit_details(report: &CrashReport) -> String {
    let mut facts = Vec::new();
    if let Some(code) = report.outcome.code {
        facts.push(format!("Exit code: {code}"));
    }
    if let Some(signal) = report.outcome.signal {
        facts.push(format!("Signal: {signal}"));
    }
    if let Some(status) = report.outcome.windows_status {
        facts.push(format!("Windows status: 0x{status:08X}"));
    }
    if facts.is_empty() {
        "No native exit status was available.".into()
    } else {
        facts.join(" · ")
    }
}

fn diagnostic_summary(report: &CrashReport) -> String {
    let mut text = format!(
        "{}\nProfile: {}\nMinecraft: {}\nLoader: {}\n{}\n",
        report.summary,
        report.context.profile_name,
        report.context.minecraft_version,
        report.context.loader.as_deref().unwrap_or("Vanilla"),
        exit_details(report)
    );
    for diagnosis in &report.diagnoses {
        let _ = write!(
            text,
            "\n{}\n{}\n",
            diagnosis.title, diagnosis.suggested_action
        );
        for evidence in &diagnosis.evidence {
            let _ = writeln!(text, "Evidence: {evidence}");
        }
    }
    text
}

fn log_text(text: String) -> gpui::Div {
    div()
        .rounded_md()
        .bg(rgb(0x0010_1923))
        .p_3()
        .text_xs()
        .child(text)
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, rc::Rc};

    use block_launcher::crash::{CrashContext, ExitOutcome};

    use super::{CrashReport, CrashView, ExitKind};

    fn report() -> CrashReport {
        CrashReport::new(
            CrashContext {
                run_id: "test-session".into(),
                profile_id: "profile".into(),
                profile_name: "My world".into(),
                minecraft_version: "1.20.6".into(),
                loader: Some("NeoForge".into()),
            },
            ExitOutcome {
                kind: ExitKind::Abnormal,
                code: Some(1),
                signal: None,
                windows_status: None,
            },
            "java.lang.OutOfMemoryError: Java heap space\naccessToken=[redacted]",
            Vec::new(),
        )
    }

    #[gpui::test]
    fn help_supports_keyboard_details_and_returns_without_relaunching(
        cx: &mut gpui::TestAppContext,
    ) {
        let returned = Rc::new(Cell::new(0));
        let returned_for_view = Rc::clone(&returned);
        let (view, cx) = crate::test_support::window(cx, move |window, cx| {
            CrashView::new(
                report(),
                std::env::temp_dir(),
                move |_, _| returned_for_view.set(returned_for_view.get() + 1),
                window,
                cx,
            )
        });
        assert!(!view.read_with(cx, |view, _| view.show_details));
        assert!(cx.debug_bounds("crash-technical-details").is_none());
        crate::test_support::press(cx, "tab tab tab tab enter");
        assert!(view.read_with(cx, |view, _| view.show_details));
        assert_eq!(returned.get(), 0);
        crate::test_support::press(cx, "shift-tab shift-tab shift-tab shift-tab enter");
        assert_eq!(returned.get(), 1);
    }

    #[gpui::test]
    fn copy_summary_uses_sanitized_diagnosis_without_dumping_game_output(
        cx: &mut gpui::TestAppContext,
    ) {
        let (_, cx) = crate::test_support::window(cx, |window, cx| {
            CrashView::new(report(), std::env::temp_dir(), |_, _| {}, window, cx)
        });
        crate::test_support::press(cx, "tab tab tab enter");
        let text = cx
            .update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text()))
            .unwrap_or_default();
        assert!(text.contains("My world"));
        assert!(text.contains("Java heap space"));
        assert!(!text.contains("accessToken="));
    }

    #[gpui::test]
    fn export_waits_for_user_path_and_only_writes_the_sanitized_report(
        cx: &mut gpui::TestAppContext,
    ) {
        let result = (|| -> anyhow::Result<()> {
            let directory = tempfile::tempdir()?;
            let report_directory = directory.path().to_path_buf();
            std::fs::write(
                directory.path().join("not-an-attachment.txt"),
                "original logs",
            )?;
            let (view, cx) = crate::test_support::window(cx, move |window, cx| {
                CrashView::new(report(), report_directory, |_, _| {}, window, cx)
            });
            crate::test_support::press(cx, "tab tab enter");
            assert!(cx.did_prompt_for_new_path());
            assert!(view.read_with(cx, |view, _| view.exporting));
            assert!(!directory.path().join("export.json").exists());
            cx.simulate_new_path_selection(|base| Some(base.join("export.json")));
            cx.run_until_parked();
            assert!(!view.read_with(cx, |view, _| view.exporting));
            let exported = std::fs::read_to_string(directory.path().join("export.json"))?;
            let expected_tail = report().sanitized_tail;
            let report: CrashReport = serde_json::from_str(&exported)?;
            assert_eq!(report.context.run_id, "test-session");
            assert_eq!(report.sanitized_tail, expected_tail);
            assert!(!exported.contains("accessToken="));
            assert!(!exported.contains("original logs"));
            Ok(())
        })();
        assert!(result.is_ok(), "{result:?}");
    }
}
