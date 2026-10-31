//! A local help window for an already sanitized game-session report.

use std::{fmt, fmt::Write as _, path::PathBuf};

use block_launcher::{
    CancellationToken,
    crash::{Confidence, CrashReport, DiagnosisKind, ExitKind},
};
use gpui::{
    App, ClipboardItem, Context, FocusHandle, InteractiveElement as _, IntoElement, KeyBinding,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Window,
    WindowControlArea, div, px, rgb,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HelpAction {
    ReturnToProfile,
    OpenFolder,
    Export,
    CopySummary,
    CancelExport,
    ToggleDetails,
    Close,
}

/// Presentation order of the action row; conditional actions keep their slot.
const ACTION_ORDER: [HelpAction; 7] = [
    HelpAction::ReturnToProfile,
    HelpAction::OpenFolder,
    HelpAction::Export,
    HelpAction::CopySummary,
    HelpAction::CancelExport,
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

#[derive(Clone, Debug, PartialEq, Eq)]
enum Status {
    Copied,
    PrepareFailed,
    ChooseDestination,
    Exported,
    ExportFailed,
    Cancelled,
    ChooserFailed,
    LanguageInitializationFailed(String),
}

impl Status {
    fn text(&self, language: crate::Language) -> String {
        match self {
            Self::Copied => tr!(language, "crash.copied"),
            Self::PrepareFailed => tr!(language, "crash.prepare_failed"),
            Self::ChooseDestination => tr!(language, "crash.choose_destination"),
            Self::Exported => tr!(language, "crash.exported"),
            Self::ExportFailed => tr!(language, "crash.export_failed"),
            Self::Cancelled => tr!(language, "crash.export_cancelled"),
            Self::ChooserFailed => tr!(language, "crash.chooser_failed"),
            Self::LanguageInitializationFailed(error) => {
                tr!(
                    language,
                    "common.language_initialization_failed",
                    error = error
                )
            }
        }
    }
}

/// Displays facts and possible explanations without reading original game logs.
///
/// The report must already be redacted, including its context and attachments.
/// Returning to the launcher does not restart Minecraft automatically.
pub struct CrashView {
    report: CrashReport,
    report_directory: PathBuf,
    language: crate::Language,
    return_to_profile: ReturnToProfile,
    focus: FocusHandle,
    focused: HelpAction,
    show_details: bool,
    exporting: bool,
    export_cancel: Option<CancellationToken>,
    status: Option<Status>,
    chrome_dragging: bool,
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

impl Drop for CrashView {
    fn drop(&mut self) {
        if let Some(cancel) = &self.export_cancel {
            cancel.cancel();
        }
    }
}

impl CrashView {
    /// Display a startup language-preference failure without hiding the report.
    pub fn report_language_initialization_error(&mut self, error: String) {
        self.status = Some(Status::LanguageInitializationFailed(error));
    }

    #[must_use]
    pub fn new(
        report: CrashReport,
        report_directory: PathBuf,
        language: crate::Language,
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
        cx.spawn(async move |view, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(100))
                    .await;
                let failed = block_logging::failure().is_some();
                if view
                    .update(cx, |_, cx| {
                        if failed {
                            cx.notify();
                        }
                    })
                    .is_err()
                    || failed
                {
                    break;
                }
            }
        })
        .detach();
        Self {
            report,
            report_directory,
            language,
            return_to_profile: Box::new(return_to_profile),
            focus,
            focused: HelpAction::ReturnToProfile,
            show_details: false,
            exporting: false,
            export_cancel: None,
            status: None,
            chrome_dragging: false,
        }
    }

    fn handle(&mut self, action: HelpAction, window: &mut Window, cx: &mut Context<'_, Self>) {
        if !self.shows(action) || !self.enables(action) {
            return;
        }
        self.focus.focus(window, cx);
        self.focused = action;
        match action {
            HelpAction::ReturnToProfile => (self.return_to_profile)(window, cx),
            HelpAction::OpenFolder => cx.reveal_path(&self.report_directory),
            HelpAction::Export => self.export(cx),
            HelpAction::CancelExport => {
                if let Some(cancel) = &self.export_cancel {
                    cancel.cancel();
                }
            }
            HelpAction::CopySummary => {
                cx.write_to_clipboard(ClipboardItem::new_string(diagnostic_summary(&self.report)));
                self.status = Some(Status::Copied);
            }
            HelpAction::ToggleDetails => self.show_details = !self.show_details,
            HelpAction::Close => window.remove_window(),
        }
        cx.notify();
    }

    /// Whether a rendered action currently accepts activation.
    fn enables(&self, action: HelpAction) -> bool {
        action != HelpAction::Export || !self.exporting
    }

    /// Whether an action has a place in the current action row.
    fn shows(&self, action: HelpAction) -> bool {
        action != HelpAction::CancelExport || self.export_cancel.is_some()
    }

    fn export_label(&self) -> String {
        let key = if self.export_cancel.is_some() {
            "crash.waiting_save"
        } else if self.exporting {
            "crash.exporting"
        } else {
            "crash.export"
        };
        tr!(self.language, key)
    }

    /// Rove to the next action that is offered and enabled.
    fn move_focus(&mut self, forward: bool) {
        let count = ACTION_ORDER.len();
        let current = ACTION_ORDER
            .iter()
            .position(|candidate| *candidate == self.focused)
            .unwrap_or(0);
        let step = if forward { 1 } else { count - 1 };
        for offset in 1..=count {
            let candidate = ACTION_ORDER[(current + step * offset) % count];
            if self.shows(candidate) && self.enables(candidate) {
                self.focused = candidate;
                return;
            }
        }
    }

    /// Keep the drawn focus on a control that is still offered.
    fn settle_focus(&mut self) {
        if !self.shows(self.focused) || !self.enables(self.focused) {
            self.focused = if self.enables(HelpAction::Export) {
                HelpAction::Export
            } else {
                HelpAction::CopySummary
            };
        }
    }

    fn export(&mut self, cx: &Context<'_, Self>) {
        if self.exporting {
            return;
        }
        let Ok(serialized) = serde_json::to_vec_pretty(&self.report) else {
            self.status = Some(Status::PrepareFailed);
            return;
        };
        let selection =
            cx.prompt_for_new_path(&self.report_directory, Some("minecraft-diagnostics.json"));
        let cancel = CancellationToken::new();
        self.exporting = true;
        self.export_cancel = Some(cancel.clone());
        self.focused = HelpAction::CancelExport;
        self.status = Some(Status::ChooseDestination);
        cx.spawn(async move |view, cx| {
            // A native chooser may wait for the user indefinitely. Race explicit
            // cancellation, not a deadline that could expire a healthy dialog.
            let selection = tokio::select! {
                biased;
                () = cancel.cancelled() => None,
                selection = selection => Some(selection),
            };
            let status = match selection {
                Some(Ok(Ok(Some(path)))) => {
                    // Once writing starts, cancellation is no longer offered.
                    // A released view must not start an unobserved export.
                    if view
                        .update(cx, |view, cx| {
                            view.export_cancel = None;
                            view.settle_focus();
                            cx.notify();
                        })
                        .is_err()
                    {
                        return;
                    }
                    let saved = cx
                        .background_executor()
                        .spawn(async move { std::fs::write(path, serialized) })
                        .await;
                    if saved.is_ok() {
                        Status::Exported
                    } else {
                        Status::ExportFailed
                    }
                }
                None | Some(Ok(Ok(None))) => Status::Cancelled,
                Some(Ok(Err(_)) | Err(_)) => Status::ChooserFailed,
            };
            let _ = view.update(cx, |view, cx| {
                view.exporting = false;
                view.export_cancel = None;
                view.settle_focus();
                view.status = Some(status);
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
    ) -> gpui_base::Button {
        let label = label.into();
        let enabled = self.enables(action);
        let primary = action == HelpAction::ReturnToProfile;
        let mut button = gpui_base::Button::new(id)
            .focusable(false)
            .tab_stop(false)
            .disabled(!enabled)
            .accessibility_label(label.clone())
            .debug_selector(move || id.to_owned())
            .px_3()
            .py_2()
            .rounded_md()
            .border_1()
            .border_color(rgb(if self.focused == action {
                0x008f_bde0
            } else {
                0x0033_4857
            }))
            .bg(rgb(if primary { 0x002d_617c } else { 0x0029_3c4a }))
            .text_sm()
            .child(label);
        if enabled {
            button = button
                .cursor_pointer()
                .hover(|style| style.bg(rgb(0x0036_5363)))
                .on_click(cx.listener(move |view, _, window, cx| view.handle(action, window, cx)));
        }
        button
    }

    fn details(&self) -> gpui::Stateful<gpui::Div> {
        let mut details = super::panel()
            .id("crash-technical-details")
            .debug_selector(|| "crash-technical-details".to_owned())
            .gap_3()
            .child(
                div()
                    .text_sm()
                    .child(tr!(self.language, "crash.technical_details")),
            )
            .child(super::note(localized_exit_details(
                &self.report,
                self.language,
            )))
            .child(super::note(tr!(
                self.language,
                "crash.session",
                id = &self.report.context.run_id
            )))
            .child(
                div()
                    .text_sm()
                    .child(tr!(self.language, "crash.recent_output")),
            )
            .child(log_text(if self.report.sanitized_tail.is_empty() {
                tr!(self.language, "crash.no_output")
            } else {
                self.report.sanitized_tail.clone()
            }));
        for attachment in &self.report.attachments {
            details = details
                .child(div().text_sm().child(attachment.name.clone()))
                .child(log_text(attachment.sanitized_text.clone()));
            if attachment.truncated {
                details = details.child(super::note(tr!(
                    self.language,
                    "crash.attachment_truncated"
                )));
            }
        }
        details
    }

    fn content(&self, cx: &Context<'_, Self>) -> gpui::Stateful<gpui::Div> {
        let context = &self.report.context;
        let edition = context.loader.as_deref().unwrap_or("Vanilla");
        let mut causes = super::panel()
            .gap_3()
            .child(div().text_lg().child(tr!(self.language, "crash.causes")));
        if self.report.diagnoses.is_empty() {
            causes = causes.child(super::note(tr!(self.language, "crash.no_diagnosis")));
        }
        for diagnosis in &self.report.diagnoses {
            let confidence_key = match diagnosis.confidence {
                Confidence::High => "crash.confidence_high",
                Confidence::Possible => "crash.confidence_possible",
                Confidence::Unknown => "crash.confidence_unknown",
            };
            let (title_key, action_key) = diagnosis_keys(diagnosis.kind);
            let mut cause = div()
                .flex()
                .flex_col()
                .gap_2()
                .child(super::note(tr!(self.language, confidence_key)))
                .child(div().text_sm().child(tr!(self.language, title_key)));
            for evidence in &diagnosis.evidence {
                cause = cause.child(log_text(evidence.clone()));
            }
            causes =
                causes.child(cause.child(div().text_sm().child(tr!(self.language, action_key))));
        }
        let mut body = div()
            .id("crash-help-content")
            .w_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .p_6()
            .gap_4()
            .child(super::note(tr!(self.language, "crash.help_heading")))
            .child(
                div()
                    .text_2xl()
                    .child(headline(self.report.outcome.kind, self.language)),
            )
            .child(div().text_sm().child(format!(
                "{} · Minecraft {} · {edition}",
                context.profile_name, context.minecraft_version
            )))
            .child(super::note(summary(
                self.report.outcome.kind,
                self.language,
            )))
            .child(causes)
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(self.button(
                        "crash-return",
                        tr!(self.language, "crash.return"),
                        HelpAction::ReturnToProfile,
                        cx,
                    ))
                    .child(self.button(
                        "crash-open-folder",
                        tr!(self.language, "crash.open_folder"),
                        HelpAction::OpenFolder,
                        cx,
                    ))
                    .child(self.button("crash-export", self.export_label(), HelpAction::Export, cx))
                    .child(self.button(
                        "crash-copy",
                        tr!(self.language, "crash.copy"),
                        HelpAction::CopySummary,
                        cx,
                    ))
                    .children(self.export_cancel.is_some().then(|| {
                        self.button(
                            "crash-cancel-export",
                            tr!(self.language, "crash.cancel_export"),
                            HelpAction::CancelExport,
                            cx,
                        )
                    }))
                    .child(self.button(
                        "crash-details",
                        tr!(
                            self.language,
                            if self.show_details {
                                "crash.hide_details"
                            } else {
                                "crash.view_details"
                            }
                        ),
                        HelpAction::ToggleDetails,
                        cx,
                    ))
                    .child(self.button(
                        "crash-close",
                        tr!(self.language, "common.close"),
                        HelpAction::Close,
                        cx,
                    )),
            )
            .child(super::note(tr!(self.language, "crash.privacy")));
        if let Some(status) = &self.status {
            body = body.child(div().text_sm().child(status.text(self.language)));
        }
        if self.show_details {
            body = body.child(self.details());
        }
        body
    }

    fn header(&self, cx: &Context<'_, Self>) -> gpui::Stateful<gpui::Div> {
        div()
            .id("crash-window-chrome")
            .debug_selector(|| "crash-window-chrome".into())
            .h(px(40.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_2()
            .px_4()
            .border_b_1()
            .border_color(rgb(0x0033_4857))
            .child(
                div()
                    .id("crash-window-title")
                    .debug_selector(|| "crash-window-title".into())
                    .flex_1()
                    .h_full()
                    .flex()
                    .items_center()
                    .window_control_area(WindowControlArea::Drag)
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(|view, _, _, _| view.chrome_dragging = true),
                    )
                    .on_mouse_up(
                        gpui::MouseButton::Left,
                        cx.listener(|view, _, _, _| view.chrome_dragging = false),
                    )
                    .on_mouse_move(
                        cx.listener(|view, event: &gpui::MouseMoveEvent, window, _| {
                            if view.chrome_dragging
                                && event.pressed_button == Some(gpui::MouseButton::Left)
                            {
                                view.chrome_dragging = false;
                                window.start_window_move();
                            } else if event.pressed_button.is_none() {
                                view.chrome_dragging = false;
                            }
                        }),
                    )
                    .child(div().text_sm().child(window_title(self.language))),
            )
            .child(
                div()
                    .id("crash-window-minimize")
                    .debug_selector(|| "crash-window-minimize".into())
                    .role(gpui::accesskit::Role::Button)
                    .aria_label(tr!(self.language, "crash.minimize"))
                    .size(px(32.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_md()
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(0x0029_3c4a)))
                    .on_click(cx.listener(|_, _, window, _| window.minimize_window()))
                    .child(super::design::Icon::Minus.render(16., 0x0095_a4b1)),
            )
            .child(
                div()
                    .id("crash-window-close")
                    .debug_selector(|| "crash-window-close".into())
                    .role(gpui::accesskit::Role::Button)
                    .aria_label(tr!(self.language, "crash.close_window"))
                    .size(px(32.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_md()
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(0x0029_3c4a)))
                    .on_click(cx.listener(|_, _, window, _| window.remove_window()))
                    .child(super::design::Icon::Close.render(16., 0x0095_a4b1)),
            )
    }
}

impl Render for CrashView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        super::feedback::ToastLayer::set_language(self.language, window, cx);
        if super::ui_logging::claim_failure_notice(block_logging::failure().is_some(), cx) {
            super::feedback::ToastLayer::enqueue(
                vec![super::feedback::Toast::new(
                    msg!("common.logging_unavailable"),
                    super::feedback::ToastKind::Error,
                )],
                window,
                cx,
            );
        }
        let body = self.content(cx);
        let header = self.header(cx);
        div()
            .key_context("CrashHelp")
            .track_focus(&self.focus)
            .on_mouse_up(
                gpui::MouseButton::Left,
                cx.listener(|view, _, _, _| view.chrome_dragging = false),
            )
            .on_action(cx.listener(|view, _: &NextControl, _, cx| {
                view.move_focus(true);
                cx.notify();
            }))
            .on_action(cx.listener(|view, _: &PreviousControl, _, cx| {
                view.move_focus(false);
                cx.notify();
            }))
            .on_action(cx.listener(|view, _: &ActivateControl, window, cx| {
                let action = view.focused;
                view.handle(action, window, cx);
            }))
            .on_action(cx.listener(|view, _: &CloseHelp, window, cx| {
                view.handle(HelpAction::Close, window, cx);
            }))
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x0017_232d))
            .text_color(rgb(0x00e9_f0f4))
            .child(header)
            .child(body.flex_1().min_h_0())
    }
}

/// The title used by the native crash-help window and its product chrome.
#[must_use]
pub fn window_title(language: crate::Language) -> String {
    tr!(language, "crash.window_title")
}

fn headline(kind: ExitKind, language: crate::Language) -> String {
    let key = match kind {
        ExitKind::Normal => "crash.normal",
        ExitKind::Stopped => "crash.stopped",
        ExitKind::StartFailed => "crash.start_failed",
        ExitKind::Abnormal => "crash.abnormal",
        ExitKind::MonitoringInterrupted => "crash.interrupted",
    };
    tr!(language, key)
}

fn summary(kind: ExitKind, language: crate::Language) -> String {
    let key = match kind {
        ExitKind::Normal => "crash.summary_normal",
        ExitKind::Stopped => "crash.summary_stopped",
        ExitKind::StartFailed => "crash.summary_start_failed",
        ExitKind::Abnormal => "crash.summary_abnormal",
        ExitKind::MonitoringInterrupted => "crash.summary_interrupted",
    };
    tr!(language, key)
}

fn diagnosis_keys(kind: DiagnosisKind) -> (&'static str, &'static str) {
    match kind {
        DiagnosisKind::JavaHeapMemory => ("crash.heap_title", "crash.heap_action"),
        DiagnosisKind::NativeMemory => ("crash.native_title", "crash.native_action"),
        DiagnosisKind::JavaVersion => ("crash.java_title", "crash.java_action"),
        DiagnosisKind::ModDependencies => ("crash.mods_title", "crash.mods_action"),
        DiagnosisKind::GraphicsInitialization => ("crash.graphics_title", "crash.graphics_action"),
        DiagnosisKind::SessionReservation => {
            ("crash.reservation_title", "crash.reservation_action")
        }
        DiagnosisKind::Unknown => ("crash.unknown_title", "crash.unknown_action"),
    }
}

fn localized_exit_details(report: &CrashReport, language: crate::Language) -> String {
    let mut facts = Vec::new();
    if let Some(code) = report.outcome.code {
        facts.push(tr!(language, "crash.exit_code", code = code));
    }
    if let Some(signal) = report.outcome.signal {
        facts.push(tr!(language, "crash.signal", signal = signal));
    }
    if let Some(status) = report.outcome.windows_status {
        facts.push(tr!(
            language,
            "crash.windows_status",
            status = format!("0x{status:08X}")
        ));
    }
    if facts.is_empty() {
        tr!(language, "crash.no_exit_status")
    } else {
        facts.join(" · ")
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

    use super::{CrashReport, CrashView, ExitKind, HelpAction};

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
    fn crash_help_uses_its_explicit_language_without_changing_the_report(
        cx: &mut gpui::TestAppContext,
    ) {
        for (language, title, return_label) in [
            (
                crate::Language::English,
                "Block · Crash help",
                "Return to instance",
            ),
            (
                crate::Language::SimplifiedChinese,
                "Block · 崩溃帮助",
                "返回实例",
            ),
            (
                crate::Language::Japanese,
                "Block · クラッシュのヘルプ",
                "インスタンスに戻る",
            ),
        ] {
            let (view, cx) = crate::test_support::window(cx, |window, cx| {
                CrashView::new(
                    report(),
                    std::env::temp_dir(),
                    language,
                    |_, _| {},
                    window,
                    cx,
                )
            });
            assert_eq!(super::window_title(language), title);
            let label = cx.update(|window, _| {
                gpui_base::test_support::find(window, &[], &"crash-return".into())
                    .and_then(|node| node.label().map(str::to_owned))
            });
            assert_eq!(label.as_deref(), Some(return_label));
            assert_eq!(
                view.read_with(cx, |view, _| view.report.summary.clone()),
                "Minecraft closed unexpectedly."
            );
        }
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
                crate::Language::English,
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
            CrashView::new(
                report(),
                std::env::temp_dir(),
                crate::Language::English,
                |_, _| {},
                window,
                cx,
            )
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
                CrashView::new(
                    report(),
                    report_directory,
                    crate::Language::English,
                    |_, _| {},
                    window,
                    cx,
                )
            });
            crate::test_support::press(cx, "tab tab enter");
            assert!(cx.did_prompt_for_new_path());
            assert!(view.read_with(cx, |view, _| {
                view.exporting
                    && view.export_cancel.is_some()
                    && view.export_label() == "Waiting for save dialog…"
            }));
            assert!(!directory.path().join("export.json").exists());
            cx.simulate_new_path_selection(|base| Some(base.join("export.json")));
            cx.run_until_parked();
            assert!(!view.read_with(cx, |view, _| {
                view.exporting || view.export_cancel.is_some()
            }));
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

    #[gpui::test]
    fn rendered_actions_publish_button_role_label_and_export_state(cx: &mut gpui::TestAppContext) {
        let (view, cx) = crate::test_support::window(cx, |window, cx| {
            CrashView::new(
                report(),
                std::env::temp_dir(),
                crate::Language::English,
                |_, _| {},
                window,
                cx,
            )
        });
        let ready = cx
            .update(|window, _| gpui_base::test_support::find(window, &[], &"crash-export".into()));
        assert!(ready.as_ref().is_some_and(|node| {
            node.role() == Some(gpui::Role::Button)
                && node.label() == Some("Export diagnostics")
                && node.disabled() != Some(true)
        }));
        let export = crate::test_support::control_bounds(cx, "crash-export");
        assert!(export.is_some());
        if let Some(export) = export {
            cx.simulate_click(export.center(), gpui::Modifiers::none());
        }
        cx.run_until_parked();
        assert!(cx.did_prompt_for_new_path());
        let waiting = cx
            .update(|window, _| gpui_base::test_support::find(window, &[], &"crash-export".into()));
        assert!(waiting.as_ref().is_some_and(|node| {
            node.role() == Some(gpui::Role::Button)
                && node.label() == Some("Waiting for save dialog…")
                && node.disabled() == Some(true)
        }));
        let cancel = cx.update(|window, _| {
            gpui_base::test_support::find(window, &[], &"crash-cancel-export".into())
        });
        assert!(cancel.as_ref().is_some_and(|node| {
            node.role() == Some(gpui::Role::Button)
                && node.label() == Some("Cancel export")
                && node.disabled() != Some(true)
        }));
        let cancel = crate::test_support::control_bounds(cx, "crash-cancel-export");
        assert!(cancel.is_some());
        if let Some(cancel) = cancel {
            cx.simulate_click(cancel.center(), gpui::Modifiers::none());
        }
        cx.run_until_parked();
        assert!(!view.read_with(cx, |view, _| view.exporting));
        cx.simulate_new_path_selection(|_| None);
        cx.run_until_parked();
        view.update(cx, |view, _| {
            view.exporting = true;
            view.focused = HelpAction::CancelExport;
            view.export_cancel = None;
            view.settle_focus();
            assert_eq!(view.focused, HelpAction::CopySummary);
            assert!(view.enables(view.focused));
            view.exporting = false;
        });
    }

    #[gpui::test]
    fn stalled_export_can_be_cancelled_and_a_late_choice_cannot_write(
        cx: &mut gpui::TestAppContext,
    ) {
        let directory = tempfile::tempdir();
        assert!(directory.is_ok());
        let Ok(directory) = directory else {
            return;
        };
        let late_path = directory.path().join("late-export.json");
        let (view, cx) = crate::test_support::window(cx, |window, cx| {
            CrashView::new(
                report(),
                directory.path().to_path_buf(),
                crate::Language::English,
                |_, _| {},
                window,
                cx,
            )
        });
        crate::test_support::press(cx, "tab tab enter");
        assert!(cx.did_prompt_for_new_path());
        assert!(view.read_with(cx, |view, _| view.exporting));
        assert_eq!(
            view.read_with(cx, |view, _| view.focused),
            HelpAction::CancelExport
        );

        // The stalled picker hands the roving focus to the offered cancellation.
        crate::test_support::press(cx, "enter");
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| {
            !view.exporting
                && view.export_cancel.is_none()
                && view
                    .status
                    .as_ref()
                    .map(|status| status.text(view.language))
                    .as_deref()
                    == Some("Export canceled.")
        }));

        // A second request keeps its own picker; the abandoned late choice
        // must never be written.
        crate::test_support::press(cx, "enter");
        assert!(view.read_with(cx, |view, _| view.exporting));
        cx.simulate_new_path_selection(|_| Some(late_path.clone()));
        cx.run_until_parked();
        assert!(!late_path.exists(), "a canceled choice must never write");
        assert!(view.read_with(cx, |view, _| view.exporting));
        cx.simulate_new_path_selection(|_| None);
        cx.run_until_parked();
        assert!(!view.read_with(cx, |view, _| view.exporting));
    }

    #[gpui::test]
    fn releasing_the_view_abandons_the_export_picker_without_writing(
        cx: &mut gpui::TestAppContext,
    ) {
        let directory = tempfile::tempdir();
        assert!(directory.is_ok());
        let Ok(directory) = directory else {
            return;
        };
        let path = directory.path().join("released-view.json");
        let (view, cx) = crate::test_support::window(cx, |window, cx| {
            CrashView::new(
                report(),
                directory.path().to_path_buf(),
                crate::Language::English,
                |_, _| {},
                window,
                cx,
            )
        });
        crate::test_support::press(cx, "tab tab enter");
        assert!(cx.did_prompt_for_new_path());
        let cancel = view.read_with(cx, |view, _| view.export_cancel.clone());
        assert!(cancel.is_some());
        drop(view);
        cx.update(|window, _| window.remove_window());
        assert!(cancel.is_some_and(|cancel| cancel.is_cancelled()));
        cx.simulate_new_path_selection(|_| Some(path.clone()));
        cx.run_until_parked();
        assert!(!path.exists(), "a released view must not start an export");
    }

    #[gpui::test]
    fn inline_return_releases_the_waiting_export_without_writing(cx: &mut gpui::TestAppContext) {
        let result = (|| -> anyhow::Result<()> {
            let directory = tempfile::tempdir()?;
            let path = directory.path().join("inline-late.json");
            let guard = std::fs::File::create(directory.path().join("presentation.lock"))?;
            let (launcher, cx) = crate::test_support::window(cx, crate::tests::test_view);
            launcher.update(cx, |launcher, cx| {
                launcher.pending_crashes.push_back(crate::PendingCrash {
                    report: report(),
                    directory: directory.path().to_path_buf(),
                    guard,
                });
                cx.notify();
            });
            cx.run_until_parked();
            let export = crate::test_support::control_bounds(cx, "crash-export");
            assert!(export.is_some());
            if let Some(export) = export {
                cx.simulate_click(export.center(), gpui::Modifiers::none());
            }
            cx.run_until_parked();
            assert!(cx.did_prompt_for_new_path());
            let cancel = launcher.read_with(cx, |launcher, cx| {
                launcher
                    .crash_help
                    .as_ref()
                    .and_then(|help| help.read(cx).export_cancel.clone())
            });
            assert!(cancel.is_some());
            let return_action = crate::test_support::control_bounds(cx, "crash-return");
            assert!(return_action.is_some());
            if let Some(return_action) = return_action {
                cx.simulate_click(return_action.center(), gpui::Modifiers::none());
            }
            cx.run_until_parked();
            assert!(launcher.read_with(cx, |launcher, _| launcher.crash_help.is_none()));
            assert!(cancel.is_some_and(|cancel| cancel.is_cancelled()));
            cx.simulate_new_path_selection(|_| Some(path.clone()));
            cx.run_until_parked();
            assert!(!path.exists(), "inline return must abandon a late export");
            Ok(())
        })();
        assert!(result.is_ok(), "{result:?}");
    }
}
