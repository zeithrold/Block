//! A local help window for an already sanitized game-session report.

use std::{cell::RefCell, fmt, fmt::Write as _, path::PathBuf, rc::Rc};

use block_launcher::{
    CancellationToken,
    crash::{Confidence, CrashReport, DiagnosisKind, ExitKind},
};
use gpui::{
    App, AppContext as _, Bounds, ClipboardItem, Context, Entity, FocusHandle, Focusable as _,
    InteractiveElement as _, IntoElement, KeyBinding, ParentElement as _, Pixels, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window,
    WindowControlArea, div, px, rgb,
};
use gpui_base::ElementExt as _;
use gpui_component::{
    IndexPath,
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
    searchable_list::SearchableListItem,
    select::{Select, SelectEvent, SelectState},
};

use super::components::{self, ButtonSize, ButtonVariant, Control, ControlState};
use super::scroll_area::ScrollAreaExt as _;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HelpAction {
    ReturnToProfile,
    OpenFolder,
    Export,
    CopySummary,
    CopySource,
    CancelExport,
    Overview,
    Logs,
    Evidence(usize),
    PreviousMatch,
    NextMatch,
    ToggleWrap,
    Close,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, gpui::Action)]
#[action(namespace = crash_help)]
struct NextControl;
#[derive(Clone, Debug, Default, PartialEq, Eq, gpui::Action)]
#[action(namespace = crash_help)]
struct PreviousControl;
#[derive(Clone, Debug, Default, PartialEq, Eq, gpui::Action)]
#[action(namespace = crash_help)]
struct FindText;
#[derive(Clone, Debug, Default, PartialEq, Eq, gpui::Action)]
#[action(namespace = crash_help)]
struct CloseHelp;

type ReturnToProfile = Box<dyn FnMut(&mut Window, &mut App)>;

#[derive(Clone)]
struct SourceChoice {
    label: SharedString,
    index: usize,
}

impl SearchableListItem for SourceChoice {
    type Value = usize;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn value(&self) -> &usize {
        &self.index
    }
}

struct SourceState {
    reader: Entity<TextareaState>,
    query: Entity<InputState>,
    wrap: bool,
}

struct FocusBounds {
    focus: FocusHandle,
    bounds: Bounds<Pixels>,
}

struct BodyFocus {
    actions: Vec<(HelpAction, FocusHandle)>,
    bounds: Rc<RefCell<Vec<FocusBounds>>>,
}

impl BodyFocus {
    fn new(diagnoses: usize, cx: &Context<'_, CrashView>) -> Self {
        let actions = [
            HelpAction::Overview,
            HelpAction::Logs,
            HelpAction::OpenFolder,
            HelpAction::CopySource,
            HelpAction::PreviousMatch,
            HelpAction::NextMatch,
            HelpAction::ToggleWrap,
        ]
        .into_iter()
        .chain((0..diagnoses).map(HelpAction::Evidence))
        .map(|action| (action, cx.focus_handle().tab_stop(true)))
        .collect();
        Self {
            actions,
            bounds: Rc::new(RefCell::new(Vec::new())),
        }
    }

    fn observe<E: gpui::ParentElement>(&self, element: E, focus: FocusHandle) -> E {
        let recorded = Rc::clone(&self.bounds);
        element.on_prepaint(move |bounds, _, _| {
            let mut recorded = recorded.borrow_mut();
            if let Some(previous) = recorded.iter_mut().find(|entry| entry.focus == focus) {
                previous.bounds = bounds;
            } else {
                recorded.push(FocusBounds { focus, bounds });
            }
        })
    }

    fn control(&self, control: Control, action: HelpAction) -> Control {
        let Some((_, focus)) = self
            .actions
            .iter()
            .find(|(candidate, _)| *candidate == action)
        else {
            return control;
        };
        self.observe(control.track_focus(focus), focus.clone())
    }

    fn reveal(&self, scroll: &gpui::ScrollHandle, window: &Window, cx: &App) -> bool {
        let Some(focused) = window.focused(cx) else {
            return false;
        };
        let Some(target) = self
            .bounds
            .borrow()
            .iter()
            .find_map(|entry| (entry.focus == focused).then_some(entry.bounds))
        else {
            return false;
        };
        let viewport = scroll.bounds();
        // Control observations are inside the one-pixel border; keep that
        // border and the design's three-pixel focus ring within the viewport.
        let top = viewport.top() + px(4.);
        let bottom = viewport.bottom() - px(4.);
        let movement = if target.top() < top {
            top - target.top()
        } else if target.bottom() > bottom {
            bottom - target.bottom()
        } else {
            px(0.)
        };
        let mut offset = scroll.offset();
        let next = (offset.y + movement).clamp(-scroll.max_offset().y.max(px(0.)), px(0.));
        if next == offset.y {
            return false;
        }
        offset.y = next;
        scroll.set_offset(offset);
        true
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Status {
    Copied,
    SourceCopied,
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
            Self::SourceCopied => tr!(language, "crash.source_copied"),
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
    preferences: super::Preferences,
    return_to_profile: ReturnToProfile,
    focus: FocusHandle,
    return_focus: FocusHandle,
    copy_focus: FocusHandle,
    cancel_focus: FocusHandle,
    body_scroll: gpui::ScrollHandle,
    body_focus: BodyFocus,
    show_logs: bool,
    expanded_evidence: Vec<bool>,
    selected_source: usize,
    selector: Entity<SelectState<Vec<SourceChoice>>>,
    sources: Vec<SourceState>,
    _subscriptions: Vec<Subscription>,
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
            .field("show_logs", &self.show_logs)
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

    /// Apply the preferences already loaded for this standalone window.
    pub fn set_saved_preferences(
        &mut self,
        preferences: &super::SavedDesktopPreferences,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.set_preferences(preferences.0.clone(), window, cx);
    }

    pub(super) fn set_preferences(
        &mut self,
        preferences: super::Preferences,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.preferences == preferences {
            return;
        }
        let language = preferences.language.unwrap_or_default();
        if language != self.language {
            self.language = language;
            let mut choices = vec![SourceChoice {
                label: tr!(language, "crash.recent_output").into(),
                index: 0,
            }];
            choices.extend(self.report.attachments.iter().enumerate().map(
                |(index, attachment)| SourceChoice {
                    label: attachment.name.clone().into(),
                    index: index + 1,
                },
            ));
            self.selector.update(cx, |selector, cx| {
                selector.set_items(choices, window, cx);
            });
            for source in &self.sources {
                source.query.update(cx, |query, cx| {
                    query.set_placeholder(tr!(language, "crash.find"), window, cx);
                });
            }
        }
        self.preferences = preferences;
        cx.notify();
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
            KeyBinding::new("ctrl-f", FindText, Some("CrashHelp")),
            KeyBinding::new("cmd-f", FindText, Some("CrashHelp")),
            KeyBinding::new("escape", CloseHelp, Some("CrashHelp")),
        ]);
        let focus = cx.focus_handle();
        let return_focus = cx.focus_handle().tab_stop(true);
        return_focus.focus(window, cx);
        let mut choices = vec![SourceChoice {
            label: tr!(language, "crash.recent_output").into(),
            index: 0,
        }];
        choices.extend(
            report
                .attachments
                .iter()
                .enumerate()
                .map(|(index, attachment)| SourceChoice {
                    label: attachment.name.clone().into(),
                    index: index + 1,
                }),
        );
        let selector = cx.new(|cx| SelectState::new(choices, Some(IndexPath::new(0)), window, cx));
        let mut subscriptions = vec![cx.subscribe(
            &selector,
            |view: &mut Self, _, event: &SelectEvent<Vec<SourceChoice>>, cx| {
                if let SelectEvent::Confirm(Some(index)) = event {
                    view.selected_source = *index;
                    cx.notify();
                }
            },
        )];
        let sources = std::iter::once(report.sanitized_tail.as_str())
            .chain(
                report
                    .attachments
                    .iter()
                    .map(|attachment| attachment.sanitized_text.as_str()),
            )
            .enumerate()
            .map(|(index, text)| {
                let reader = cx.new(|cx| {
                    let mut state = TextareaState::new(window, cx)
                        .default_value(text.to_owned())
                        .searchable(false);
                    state.set_readonly(true, cx);
                    state
                });
                let query = cx
                    .new(|cx| InputState::new(window, cx).placeholder(tr!(language, "crash.find")));
                subscriptions.push(cx.subscribe(
                    &query,
                    move |view: &mut Self, input, event: &InputEvent, cx| match event {
                        InputEvent::Change => {
                            let value = input.read(cx).value();
                            view.sources[index]
                                .reader
                                .update(cx, |reader, cx| reader.set_search_query(value, true, cx));
                            cx.notify();
                        }
                        InputEvent::PressEnter { shift, .. } => {
                            view.search_step(index, !shift, cx);
                        }
                        InputEvent::Focus | InputEvent::Blur => {}
                    },
                ));
                SourceState {
                    reader,
                    query,
                    wrap: true,
                }
            })
            .collect();
        let expanded_evidence = vec![false; report.diagnoses.len()];
        let body_focus = BodyFocus::new(report.diagnoses.len(), cx);
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
            preferences: super::Preferences::default(),
            return_to_profile: Box::new(return_to_profile),
            focus,
            return_focus,
            copy_focus: cx.focus_handle().tab_stop(true),
            cancel_focus: cx.focus_handle().tab_stop(true),
            body_scroll: gpui::ScrollHandle::new(),
            body_focus,
            show_logs: false,
            expanded_evidence,
            selected_source: 0,
            selector,
            sources,
            _subscriptions: subscriptions,
            exporting: false,
            export_cancel: None,
            status: None,
            chrome_dragging: false,
        }
    }

    fn handle(&mut self, action: HelpAction, window: &mut Window, cx: &mut Context<'_, Self>) {
        match action {
            HelpAction::ReturnToProfile => (self.return_to_profile)(window, cx),
            HelpAction::OpenFolder => cx.reveal_path(&self.report_directory),
            HelpAction::Export => self.export(window, cx),
            HelpAction::CancelExport => {
                if let Some(cancel) = &self.export_cancel {
                    cancel.cancel();
                }
            }
            HelpAction::CopySummary => {
                cx.write_to_clipboard(ClipboardItem::new_string(diagnostic_summary(&self.report)));
                self.status = Some(Status::Copied);
            }
            HelpAction::CopySource => {
                cx.write_to_clipboard(ClipboardItem::new_string(self.source_text().to_owned()));
                self.status = Some(Status::SourceCopied);
            }
            HelpAction::Overview => self.show_logs = false,
            HelpAction::Logs => self.show_logs = true,
            HelpAction::Evidence(index) => {
                self.expanded_evidence[index] = !self.expanded_evidence[index];
            }
            HelpAction::PreviousMatch => self.search_step(self.selected_source, false, cx),
            HelpAction::NextMatch => self.search_step(self.selected_source, true, cx),
            HelpAction::ToggleWrap => {
                let source = &mut self.sources[self.selected_source];
                source.wrap = !source.wrap;
                source.reader.update(cx, |reader, cx| {
                    reader.set_soft_wrap(source.wrap, window, cx)
                });
            }
            HelpAction::Close => window.remove_window(),
        }
        cx.notify();
    }

    fn source_text(&self) -> &str {
        if self.selected_source == 0 {
            &self.report.sanitized_tail
        } else {
            &self.report.attachments[self.selected_source - 1].sanitized_text
        }
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

    fn search_step(&self, index: usize, forward: bool, cx: &mut Context<'_, Self>) {
        self.sources[index].reader.update(cx, |reader, cx| {
            if forward {
                reader.next_search_match(cx);
            } else {
                reader.previous_search_match(cx);
            }
        });
        cx.notify();
    }

    fn reveal_details(&self) {
        let details_index = if self.report.outcome.kind == ExitKind::MonitoringInterrupted {
            4
        } else {
            3
        };
        self.body_scroll.scroll_to_top_of_item(details_index);
    }

    fn focus_find(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.show_logs = true;
        self.reveal_details();
        self.sources[self.selected_source]
            .query
            .update(cx, |query, cx| query.focus(window, cx));
        cx.notify();
    }

    fn reveal_focused_control(&self, window: &Window, cx: &mut Context<'_, Self>) {
        if self.body_focus.reveal(&self.body_scroll, window, cx) {
            cx.notify();
        }
    }

    fn escape(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let action = if self.export_cancel.is_some() {
            HelpAction::CancelExport
        } else {
            HelpAction::Close
        };
        self.handle(action, window, cx);
    }

    fn export(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
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
        self.status = Some(Status::ChooseDestination);
        self.cancel_focus.focus(window, cx);
        let window_handle = window.window_handle();
        let cancel_focus = self.cancel_focus.clone();
        let copy_focus = self.copy_focus.clone();
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
                            cx.notify();
                        })
                        .is_err()
                    {
                        return;
                    }
                    let _ = window_handle.update(cx, |_, window, cx| {
                        if cancel_focus.is_focused(window) {
                            copy_focus.focus(window, cx);
                        }
                    });
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
                view.status = Some(status);
                cx.notify();
            });
            let _ = window_handle.update(cx, |_, window, cx| {
                if cancel_focus.is_focused(window) {
                    copy_focus.focus(window, cx);
                }
            });
        })
        .detach();
    }

    fn button(
        &self,
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        action: HelpAction,
        variant: ButtonVariant,
        palette: super::Palette,
        cx: &Context<'_, Self>,
    ) -> Control {
        let view = cx.weak_entity();
        let control = components::button(
            id,
            label,
            palette,
            variant,
            ControlState {
                disabled: action == HelpAction::Export && self.exporting,
                ..ControlState::default()
            },
            move |_, window, cx| {
                let _ = view.update(cx, |view, cx| view.handle(action, window, cx));
            },
        );
        match action {
            HelpAction::ReturnToProfile => control.track_focus(&self.return_focus),
            HelpAction::CopySummary => control.track_focus(&self.copy_focus),
            HelpAction::CancelExport => control.track_focus(&self.cancel_focus),
            _ => self.body_focus.control(control, action),
        }
    }

    fn toolbar_button(
        &self,
        id: &'static str,
        label: impl Into<SharedString>,
        action: HelpAction,
        presentation: (super::design::Icon, bool),
        palette: super::Palette,
        cx: &Context<'_, Self>,
    ) -> Control {
        let (icon, compact) = presentation;
        if !compact {
            return self.button(id, label, action, ButtonVariant::Ghost, palette, cx);
        }
        let view = cx.weak_entity();
        let label = label.into();
        let control = components::button_sized(
            id,
            "",
            palette,
            ButtonVariant::Ghost,
            ButtonSize::Icon,
            ControlState::default(),
            move |_, window, cx| {
                let _ = view.update(cx, |view, cx| view.handle(action, window, cx));
            },
        )
        .aria_label(label.clone())
        .child(icon.render(16., palette.foreground));
        self.body_focus
            .control(super::tooltip::trigger(control, label, cx), action)
    }

    fn details(
        &self,
        palette: super::Palette,
        compact: bool,
        short: bool,
        cx: &Context<'_, Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let source = &self.sources[self.selected_source];
        let matcher = &source.reader.read(cx).search_session().matcher;
        let current = matcher.current().map_or(0, |index| index + 1);
        let total = matcher.len();
        let view = cx.weak_entity();
        let mut details = div()
            .id("crash-technical-details")
            .debug_selector(|| "crash-technical-details".to_owned())
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .text_size(px(super::design::SMALL_FONT_SIZE))
                    .text_color(rgb(palette.muted))
                    .child(localized_exit_details(&self.report, self.language))
                    .child(tr!(
                        self.language,
                        "crash.session",
                        id = &self.report.context.run_id
                    )),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(div().child(tr!(self.language, "crash.source")))
                    .child(
                        self.body_focus.observe(
                            div()
                                .id("crash-source-field")
                                .debug_selector(|| "crash-source-field".into())
                                .w(px(if compact { 180. } else { 260. }))
                                .max_w_full()
                                .child(
                                    Select::new(&self.selector)
                                        .id("crash-source")
                                        .accessibility_label(tr!(self.language, "crash.source"))
                                        .w_full(),
                                ),
                            self.selector.focus_handle(cx),
                        ),
                    )
                    .child(self.toolbar_button(
                        "crash-open-folder",
                        tr!(self.language, "crash.open_folder"),
                        HelpAction::OpenFolder,
                        (super::design::Icon::Folder, compact),
                        palette,
                        cx,
                    ))
                    .child(self.toolbar_button(
                        "crash-copy-source",
                        tr!(self.language, "crash.copy_source"),
                        HelpAction::CopySource,
                        (super::design::Icon::Copy, compact),
                        palette,
                        cx,
                    )),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(
                        self.body_focus.observe(
                            div()
                                .id("crash-find-field")
                                .debug_selector(|| "crash-find-field".into())
                                .w(px(if compact { 160. } else { 240. }))
                                .max_w_full()
                                .child(
                                    Input::new(&source.query)
                                        .id("crash-find")
                                        .aria_label(tr!(self.language, "crash.find"))
                                        .w_full(),
                                ),
                            source.query.focus_handle(cx),
                        ),
                    )
                    .child(self.toolbar_button(
                        "crash-previous-match",
                        tr!(self.language, "crash.previous_match"),
                        HelpAction::PreviousMatch,
                        (super::design::Icon::ChevronLeft, compact),
                        palette,
                        cx,
                    ))
                    .child(self.toolbar_button(
                        "crash-next-match",
                        tr!(self.language, "crash.next_match"),
                        HelpAction::NextMatch,
                        (super::design::Icon::ChevronRight, compact),
                        palette,
                        cx,
                    ))
                    .child(
                        div()
                            .id("crash-match-count")
                            .debug_selector(|| "crash-match-count".into())
                            .text_color(rgb(palette.muted))
                            .child(tr!(
                                self.language,
                                "crash.match_count",
                                current = current,
                                total = total
                            )),
                    )
                    .child(self.body_focus.control(
                        components::checkbox(
                            "crash-wrap",
                            tr!(self.language, "crash.wrap"),
                            source.wrap,
                            palette,
                            ControlState::default(),
                            move |_, window, cx| {
                                let _ = view.update(cx, |view, cx| {
                                    view.handle(HelpAction::ToggleWrap, window, cx);
                                });
                            },
                        ),
                        HelpAction::ToggleWrap,
                    )),
            )
            .children(
                (self.selected_source == 0 && self.source_text().is_empty()).then(|| {
                    div()
                        .id("crash-no-output")
                        .debug_selector(|| "crash-no-output".into())
                        .text_color(rgb(palette.muted))
                        .child(tr!(self.language, "crash.no_output"))
                }),
            )
            .child(
                self.body_focus.observe(
                    div()
                        .id("crash-reader")
                        .debug_selector(|| "crash-reader".into())
                        .w_full()
                        .min_w_0()
                        .child(
                            Textarea::new(&source.reader)
                                .readonly(true)
                                .aria_label(tr!(self.language, "crash.logs_details"))
                                .font_family(
                                    gpui_component::Theme::global(cx).mono_font_family.clone(),
                                )
                                .text_size(px(super::design::BODY_FONT_SIZE))
                                .h(px(if short { 128. } else { 240. }))
                                .w_full(),
                        ),
                    source.reader.focus_handle(cx),
                ),
            );
        if self.selected_source > 0 && self.report.attachments[self.selected_source - 1].truncated {
            details = details.child(
                div()
                    .id("crash-truncated")
                    .debug_selector(|| "crash-truncated".into())
                    .text_color(rgb(palette.muted))
                    .child(tr!(self.language, "crash.attachment_truncated")),
            );
        }
        details
    }

    fn overview(&self, palette: super::Palette, cx: &Context<'_, Self>) -> gpui::Div {
        let mut causes = div().flex().flex_col().gap_3();
        if self.report.diagnoses.is_empty() {
            causes = causes.child(
                div()
                    .text_color(rgb(palette.muted))
                    .child(tr!(self.language, "crash.no_diagnosis")),
            );
        }
        for (index, diagnosis) in self.report.diagnoses.iter().enumerate() {
            let confidence_key = match diagnosis.confidence {
                Confidence::High => "crash.confidence_high",
                Confidence::Possible => "crash.confidence_possible",
                Confidence::Unknown => "crash.confidence_unknown",
            };
            let (title_key, action_key) = diagnosis_keys(diagnosis.kind);
            let mut cause = div()
                .flex()
                .flex_col()
                .min_w_0()
                .gap_1()
                .py_2()
                .border_t_1()
                .border_color(rgb(palette.border))
                .child(
                    div()
                        .text_color(rgb(palette.muted))
                        .text_size(px(super::design::SMALL_FONT_SIZE))
                        .child(tr!(self.language, confidence_key)),
                )
                .child(
                    div()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(tr!(self.language, title_key)),
                )
                .child(
                    div()
                        .text_color(rgb(palette.muted))
                        .text_size(px(super::design::SMALL_FONT_SIZE))
                        .child(tr!(self.language, "crash.suggested_action")),
                )
                .child(
                    div()
                        .id(("crash-advice", index))
                        .debug_selector(move || format!("crash-advice-{index}"))
                        .child(tr!(self.language, action_key)),
                );
            if !diagnosis.evidence.is_empty() {
                let key = if self.expanded_evidence[index] {
                    "crash.hide_evidence"
                } else {
                    "crash.show_evidence"
                };
                cause = cause.child(self.button(
                    format!("crash-evidence-{index}"),
                    tr!(self.language, key, count = diagnosis.evidence.len()),
                    HelpAction::Evidence(index),
                    ButtonVariant::Ghost,
                    palette,
                    cx,
                ));
                if self.expanded_evidence[index] {
                    cause = cause.child(
                        div()
                            .id(("crash-evidence-text", index))
                            .debug_selector(move || format!("crash-evidence-text-{index}"))
                            .children(diagnosis.evidence.iter().map(|evidence| {
                                div()
                                    .font_family(
                                        gpui_component::Theme::global(cx).mono_font_family.clone(),
                                    )
                                    .text_color(rgb(palette.muted))
                                    .child(evidence.clone())
                            })),
                    );
                }
            }
            causes = causes.child(cause);
        }
        causes
    }

    fn content(
        &self,
        palette: super::Palette,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement + gpui::Styled {
        let context = &self.report.context;
        let edition = context.loader.as_deref().unwrap_or("Vanilla");
        div()
            .id("crash-help-content")
            .debug_selector(|| "crash-help-content".into())
            .w_full()
            .flex()
            .flex_col()
            .p_4()
            .gap_2()
            .child(
                div()
                    .id("crash-headline")
                    .debug_selector(|| "crash-headline".into())
                    .text_xl()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(headline(self.report.outcome.kind, self.language)),
            )
            .child(
                div()
                    .id("crash-metadata")
                    .debug_selector(|| "crash-metadata".into())
                    .text_color(rgb(palette.muted))
                    .child(format!(
                        "{} · Minecraft {} · {edition}",
                        context.profile_name, context.minecraft_version
                    )),
            )
            .children(
                (self.report.outcome.kind == ExitKind::MonitoringInterrupted).then(|| {
                    div()
                        .id("crash-may-still-run")
                        .debug_selector(|| "crash-may-still-run".into())
                        .child(tr!(self.language, "crash.may_still_run"))
                }),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(self.button(
                        "crash-overview",
                        tr!(self.language, "crash.overview"),
                        HelpAction::Overview,
                        if self.show_logs {
                            ButtonVariant::Ghost
                        } else {
                            ButtonVariant::Secondary
                        },
                        palette,
                        cx,
                    ))
                    .child(self.button(
                        "crash-details",
                        tr!(self.language, "crash.logs_details"),
                        HelpAction::Logs,
                        if self.show_logs {
                            ButtonVariant::Secondary
                        } else {
                            ButtonVariant::Ghost
                        },
                        palette,
                        cx,
                    )),
            )
            .child(if self.show_logs {
                self.details(
                    palette,
                    window.viewport_size().width < px(600.),
                    window.viewport_size().height < px(650.),
                    cx,
                )
                .into_any_element()
            } else {
                self.overview(palette, cx).into_any_element()
            })
            .scroll_area(&self.body_scroll, "crash-help-content", palette, false)
    }

    fn footer(&self, palette: super::Palette, cx: &Context<'_, Self>) -> gpui::Stateful<gpui::Div> {
        div()
            .id("crash-footer")
            .debug_selector(|| "crash-footer".into())
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap_2()
            .p_4()
            .border_t_1()
            .border_color(rgb(palette.border))
            .child(
                div()
                    .text_size(px(super::design::SMALL_FONT_SIZE))
                    .text_color(rgb(palette.muted))
                    .child(tr!(self.language, "crash.privacy")),
            )
            .children(self.status.as_ref().map(|status| {
                div()
                    .text_color(rgb(palette.foreground))
                    .child(status.text(self.language))
            }))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_1()
                            .child(self.button(
                                "crash-copy",
                                tr!(self.language, "crash.copy"),
                                HelpAction::CopySummary,
                                ButtonVariant::Ghost,
                                palette,
                                cx,
                            ))
                            .child(self.button(
                                "crash-export",
                                self.export_label(),
                                HelpAction::Export,
                                ButtonVariant::Ghost,
                                palette,
                                cx,
                            ))
                            .children(self.export_cancel.is_some().then(|| {
                                self.button(
                                    "crash-cancel-export",
                                    tr!(self.language, "crash.cancel_export"),
                                    HelpAction::CancelExport,
                                    ButtonVariant::Ghost,
                                    palette,
                                    cx,
                                )
                            })),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_1()
                            .child(self.button(
                                "crash-close",
                                tr!(self.language, "common.close"),
                                HelpAction::Close,
                                ButtonVariant::Ghost,
                                palette,
                                cx,
                            ))
                            .child(self.button(
                                "crash-return",
                                tr!(self.language, "crash.return"),
                                HelpAction::ReturnToProfile,
                                ButtonVariant::Primary,
                                palette,
                                cx,
                            )),
                    ),
            )
    }

    fn header(&self, palette: super::Palette, cx: &Context<'_, Self>) -> gpui::Stateful<gpui::Div> {
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
            .border_color(rgb(palette.border))
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
                    .hover(move |style| style.bg(rgb(palette.accent)))
                    .on_click(cx.listener(|_, _, window, _| window.minimize_window()))
                    .child(super::design::Icon::Minus.render(16., palette.muted)),
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
                    .hover(move |style| style.bg(rgb(palette.accent)))
                    .on_click(cx.listener(|_, _, window, _| window.remove_window()))
                    .child(super::design::Icon::Close.render(16., palette.muted)),
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
        let palette = self.preferences.palette(window.appearance());
        cx.set_global(palette);
        super::sync_component_theme(palette, cx);
        let body = self.content(palette, window, cx);
        let header = self.header(palette, cx);
        let footer = self.footer(palette, cx);
        div()
            .key_context("CrashHelp")
            .track_focus(&self.focus)
            .on_mouse_up(
                gpui::MouseButton::Left,
                cx.listener(|view, _, _, _| view.chrome_dragging = false),
            )
            .on_action(cx.listener(|view, _: &NextControl, window, cx| {
                window.focus_next(cx);
                view.reveal_focused_control(window, cx);
            }))
            .on_action(cx.listener(|view, _: &PreviousControl, window, cx| {
                window.focus_prev(cx);
                view.reveal_focused_control(window, cx);
            }))
            .on_action(cx.listener(|view, _: &FindText, window, cx| {
                view.focus_find(window, cx);
            }))
            .on_action(
                cx.listener(|view, _: &gpui_base::input::Search, window, cx| {
                    view.focus_find(window, cx);
                }),
            )
            .on_action(cx.listener(|view, _: &CloseHelp, window, cx| {
                view.escape(window, cx);
            }))
            .on_action(
                cx.listener(|view, _: &gpui_base::input::Escape, window, cx| {
                    view.escape(window, cx);
                }),
            )
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(palette.background))
            .text_color(rgb(palette.foreground))
            .text_size(px(super::design::BODY_FONT_SIZE))
            .line_height(px(super::design::BODY_LINE_HEIGHT))
            .font_family(super::design::FONT_FAMILY)
            .child(header)
            .child(body.flex_1().min_h_0())
            .child(footer)
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

#[cfg(test)]
mod tests {
    use std::{cell::Cell, rc::Rc};

    use block_launcher::crash::{AttachmentKind, CrashContext, DiagnosticAttachment, ExitOutcome};
    #[cfg(debug_assertions)]
    use gpui::AppContext as _;

    use super::{CrashReport, CrashView, ExitKind};

    #[cfg(debug_assertions)]
    struct EnterProbe {
        _subscription: gpui::Subscription,
    }

    fn click(cx: &mut gpui::VisualTestContext, selector: &'static str) {
        let bounds = crate::test_support::control_bounds(cx, selector);
        assert!(bounds.is_some(), "missing control {selector}");
        if let Some(bounds) = bounds {
            cx.simulate_click(bounds.center(), gpui::Modifiers::none());
        }
        cx.run_until_parked();
    }

    fn tab_to_direction(cx: &mut gpui::VisualTestContext, selector: &'static str, key: &str) {
        let mut focused = false;
        for _ in 0..20 {
            focused = cx.update(|window, _| {
                gpui_base::test_support::find(window, &[], &selector.into())
                    .and_then(|node| node.focused())
                    == Some(true)
            });
            if focused {
                break;
            }
            crate::test_support::press(cx, key);
        }
        assert!(focused, "{selector} is not reachable by Tab");
    }

    fn tab_to(cx: &mut gpui::VisualTestContext, selector: &'static str) {
        tab_to_direction(cx, selector, "tab");
    }

    fn native_enter(cx: &mut gpui::VisualTestContext, shift: bool) {
        let keystroke = gpui::Keystroke {
            key: "enter".into(),
            key_char: None,
            modifiers: if shift {
                gpui::Modifiers::shift()
            } else {
                gpui::Modifiers::none()
            },
        };
        cx.simulate_event(gpui::KeyDownEvent {
            keystroke: keystroke.clone(),
            is_held: false,
            prefer_character_input: false,
        });
        cx.simulate_event(gpui::KeyUpEvent { keystroke });
        cx.run_until_parked();
    }

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

    fn report_with_attachment() -> CrashReport {
        let mut report = report();
        report.context.run_id = "00000000-0000-4000-8000-000000000001".into();
        report.sanitized_tail = "第一行\n  issue 🧱\nissue end\n".into();
        report.attachments.push(DiagnosticAttachment {
            name: "crash-example.txt".into(),
            kind: AttachmentKind::MinecraftCrashReport,
            sanitized_text: "附件\n    issue details\n".into(),
            truncated: true,
        });
        report
    }

    fn multiple_cause_report() -> CrashReport {
        let report = report();
        CrashReport::new(
            report.context,
            report.outcome,
            "OutOfMemoryError: Java heap space\nUnsupportedClassVersionError",
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
    fn standalone_uses_the_loaded_language_and_appearance_without_rewriting_them(
        cx: &mut gpui::TestAppContext,
    ) {
        let result = (|| -> anyhow::Result<()> {
            let root = tempfile::tempdir()?;
            crate::Preferences {
                appearance: crate::Appearance::Dark,
                language: Some(crate::Language::Japanese),
                ..Default::default()
            }
            .save(root.path())?;
            let path = root.path().join(crate::global_config::FILE);
            let original = std::fs::read(&path)?;
            let (preferences, error) = crate::initialize_saved_desktop_preferences(root.path());
            assert!(error.is_none(), "{error:?}");
            assert_eq!(preferences.language(), crate::Language::Japanese);
            assert_eq!(std::fs::read(&path)?, original);
            let (view, cx) = crate::test_support::window(cx, |window, cx| {
                let mut view = CrashView::new(
                    report(),
                    root.path().to_path_buf(),
                    preferences.language(),
                    |_, _| {},
                    window,
                    cx,
                );
                view.set_saved_preferences(&preferences, window, cx);
                view
            });
            assert_eq!(
                view.read_with(cx, |view, _| view.language),
                crate::Language::Japanese
            );
            assert_eq!(
                cx.update(|_, cx| cx.global::<crate::Palette>().background),
                crate::Palette::DARK.background
            );
            assert_eq!(std::fs::read(path)?, original);
            Ok(())
        })();
        assert!(result.is_ok(), "{result:?}");
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
        assert!(!view.read_with(cx, |view, _| view.show_logs));
        assert!(cx.debug_bounds("crash-technical-details").is_none());
        tab_to(cx, "crash-details");
        crate::test_support::press(cx, "enter");
        assert!(view.read_with(cx, |view, _| view.show_logs));
        assert_eq!(returned.get(), 0);
        tab_to(cx, "crash-return");
        crate::test_support::press(cx, "enter");
        assert_eq!(returned.get(), 1);
    }

    #[gpui::test]
    fn reader_search_selection_and_source_keep_real_keyboard_context(
        cx: &mut gpui::TestAppContext,
    ) {
        let returned = Rc::new(Cell::new(0));
        let returned_for_view = Rc::clone(&returned);
        let (view, cx) = crate::test_support::window(cx, move |window, cx| {
            CrashView::new(
                report_with_attachment(),
                std::env::temp_dir(),
                crate::Language::English,
                move |_, _| returned_for_view.set(returned_for_view.get() + 1),
                window,
                cx,
            )
        });
        crate::test_support::press(cx, "ctrl-f");
        assert!(view.read_with(cx, |view, _| view.show_logs));
        cx.simulate_input("issue");
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, cx| {
            let reader = view.sources[0].reader.read(cx);
            view.sources[0].query.read(cx).value() == "issue"
                && reader.search_session().matcher.len() == 2
                && reader.search_session().matcher.current() == Some(0)
        }));
        click(cx, "crash-next-match");
        assert!(view.read_with(cx, |view, cx| {
            view.sources[0]
                .reader
                .read(cx)
                .search_session()
                .matcher
                .current()
                == Some(1)
        }));
        click(cx, "crash-next-match");
        assert!(view.read_with(cx, |view, cx| {
            view.sources[0]
                .reader
                .read(cx)
                .search_session()
                .matcher
                .current()
                == Some(0)
        }));
        click(cx, "crash-previous-match");
        assert!(view.read_with(cx, |view, cx| {
            view.sources[0]
                .reader
                .read(cx)
                .search_session()
                .matcher
                .current()
                == Some(1)
        }));
        crate::test_support::press(cx, "ctrl-f");
        crate::test_support::press(cx, "enter");
        assert!(view.read_with(cx, |view, cx| {
            view.sources[0]
                .reader
                .read(cx)
                .search_session()
                .matcher
                .current()
                == Some(0)
        }));
        crate::test_support::press(cx, "enter");
        assert!(view.read_with(cx, |view, cx| {
            view.sources[0]
                .reader
                .read(cx)
                .search_session()
                .matcher
                .current()
                == Some(1)
        }));
        crate::test_support::press(cx, "shift-enter");
        assert!(view.read_with(cx, |view, cx| {
            view.sources[0]
                .reader
                .read(cx)
                .search_session()
                .matcher
                .current()
                == Some(0)
        }));
        crate::test_support::press(cx, "space");
        assert_eq!(returned.get(), 0);
        assert_eq!(cx.windows().len(), 1);
        cx.simulate_input("space");
        cx.run_until_parked();
        assert_eq!(returned.get(), 0);
        assert!(view.read_with(cx, |view, cx| {
            view.sources[0].query.read(cx).value() == "issue space"
        }));

        // The real selector owns opening, option navigation and Escape.
        tab_to(cx, "crash-source");
        crate::test_support::press(cx, "enter escape");
        assert_eq!(cx.windows().len(), 1);
        crate::test_support::press(cx, "enter down enter");
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| view.selected_source == 1));
        assert!(view.read_with(cx, |view, cx| {
            view.sources[1].reader.read(cx).value() == "附件\n    issue details\n"
                && view.sources[1].query.read(cx).value().is_empty()
        }));
        click(cx, "crash-wrap");
        assert!(view.read_with(cx, |view, _| !view.sources[1].wrap));
        click(cx, "crash-copy-source");
        let copied = cx
            .update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text()))
            .unwrap_or_default();
        assert_eq!(copied, "附件\n    issue details\n");
        click(cx, "crash-overview");
        click(cx, "crash-details");
        assert!(view.read_with(cx, |view, _| view.selected_source == 1));
        tab_to(cx, "crash-source");
        crate::test_support::press(cx, "enter up enter");
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, cx| {
            view.selected_source == 0 && view.sources[0].query.read(cx).value() == "issue space"
        }));
        click(cx, "crash-reader");
        let select_copy = if cfg!(target_os = "macos") {
            "cmd-a cmd-c"
        } else {
            "ctrl-a ctrl-c"
        };
        crate::test_support::press(cx, select_copy);
        let selected = cx
            .update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text()))
            .unwrap_or_default();
        assert_eq!(selected, "第一行\n  issue 🧱\nissue end\n");
        tab_to(cx, "crash-source");
        crate::test_support::press(cx, "enter down enter");
        assert!(view.read_with(cx, |view, _| {
            view.selected_source == 1 && !view.sources[1].wrap
        }));
        assert_eq!(returned.get(), 0);
    }

    #[gpui::test]
    fn search_accepts_a_native_enter_without_a_text_character(cx: &mut gpui::TestAppContext) {
        let returned = Rc::new(Cell::new(0));
        let returned_for_view = Rc::clone(&returned);
        let (view, cx) = crate::test_support::window(cx, move |window, cx| {
            let mut report = report_with_attachment();
            report.sanitized_tail = "issue one\nissue two\nissue three\n".into();
            CrashView::new(
                report,
                std::env::temp_dir(),
                crate::Language::English,
                move |_, _| returned_for_view.set(returned_for_view.get() + 1),
                window,
                cx,
            )
        });
        crate::test_support::press(cx, "ctrl-f");
        cx.simulate_input("issue");
        cx.run_until_parked();
        click(cx, "crash-find-field");
        native_enter(cx, false);
        let current = view.read_with(cx, |view, cx| {
            view.sources[0]
                .reader
                .read(cx)
                .search_session()
                .matcher
                .current()
        });
        assert_eq!(current, Some(1), "one Enter must advance exactly once");
        native_enter(cx, true);
        let previous = view.read_with(cx, |view, cx| {
            view.sources[0]
                .reader
                .read(cx)
                .search_session()
                .matcher
                .current()
        });
        assert_eq!(previous, Some(0), "one Shift-Enter must go back once");
        assert_eq!(returned.get(), 0);
        assert_eq!(cx.windows().len(), 1);
    }

    #[cfg(debug_assertions)]
    #[gpui::test]
    fn search_enter_runs_once_after_real_launcher_and_crash_construction(
        cx: &mut gpui::TestAppContext,
    ) {
        {
            let (launcher, cx) = crate::test_support::window(cx, |window, cx| {
                crate::Block::new(
                    window,
                    cx,
                    crate::LaunchOptions {
                        scenario: Some(crate::ScenarioOptions {
                            name: "home-empty".into(),
                            dark: false,
                        }),
                        ..Default::default()
                    },
                )
            });
            cx.run_until_parked();
            drop(launcher);
            cx.update(|window, _| window.remove_window());
        }
        let (view, cx) = crate::test_support::window(cx, |window, cx| {
            let mut report = report_with_attachment();
            report.sanitized_tail = "issue one\nissue two\nissue three\n".into();
            CrashView::new(
                report,
                std::env::temp_dir(),
                crate::Language::English,
                |_, _| {},
                window,
                cx,
            )
        });
        let count = Rc::new(Cell::new(0));
        let count_events = Rc::clone(&count);
        let _probe = cx.update(|_, cx| {
            let query = view.read(cx).sources[0].query.clone();
            cx.new(|cx| EnterProbe {
                _subscription: cx.subscribe(
                    &query,
                    move |_: &mut EnterProbe, _, event: &gpui_base::input::InputEvent, _| {
                        if matches!(event, gpui_base::input::InputEvent::PressEnter { .. }) {
                            count_events.set(count_events.get() + 1);
                        }
                    },
                ),
            })
        });
        crate::test_support::press(cx, "ctrl-f");
        cx.simulate_input("issue");
        cx.run_until_parked();
        click(cx, "crash-find-field");
        native_enter(cx, false);
        assert_eq!(count.get(), 1);
        assert!(view.read_with(cx, |view, cx| {
            view.sources[0]
                .reader
                .read(cx)
                .search_session()
                .matcher
                .current()
                == Some(1)
        }));
    }

    #[gpui::test]
    fn failed_export_preserves_the_selected_reader_and_search(cx: &mut gpui::TestAppContext) {
        let directory = tempfile::tempdir();
        assert!(directory.is_ok());
        let Ok(directory) = directory else {
            return;
        };
        let (view, cx) = crate::test_support::window(cx, |window, cx| {
            CrashView::new(
                report_with_attachment(),
                directory.path().to_path_buf(),
                crate::Language::English,
                |_, _| {},
                window,
                cx,
            )
        });
        crate::test_support::press(cx, "ctrl-f");
        tab_to(cx, "crash-source");
        crate::test_support::press(cx, "enter down enter");
        crate::test_support::press(cx, "ctrl-f");
        cx.simulate_input("issue");
        cx.run_until_parked();
        click(cx, "crash-export");
        assert!(cx.did_prompt_for_new_path());
        cx.simulate_new_path_selection(|base| Some(base.join("missing").join("export.json")));
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, cx| {
            matches!(view.status, Some(super::Status::ExportFailed))
                && view.show_logs
                && view.selected_source == 1
                && view.sources[1].query.read(cx).value() == "issue"
                && view.sources[1]
                    .reader
                    .read(cx)
                    .search_session()
                    .matcher
                    .current()
                    == Some(0)
        }));
    }

    #[gpui::test]
    fn long_sources_keep_scrolled_selection_across_tabs_sources_and_export_error(
        cx: &mut gpui::TestAppContext,
    ) {
        let directory = tempfile::tempdir();
        assert!(directory.is_ok());
        let Ok(directory) = directory else {
            return;
        };
        let mut report = report_with_attachment();
        report.sanitized_tail = (0..180)
            .map(|line| format!("Recent {line:03}  世界 🧱\n"))
            .collect::<String>()
            + "Recent END";
        report.attachments[0].sanitized_text = (0..180)
            .map(|line| format!("Report {line:03}  例 🧱\n"))
            .collect::<String>()
            + "Report EOF";
        let (view, cx) = crate::test_support::window(cx, |window, cx| {
            CrashView::new(
                report,
                directory.path().to_path_buf(),
                crate::Language::English,
                |_, _| {},
                window,
                cx,
            )
        });
        crate::test_support::press(cx, "ctrl-f");
        click(cx, "crash-reader");
        let select_last_word = if cfg!(target_os = "macos") {
            "cmd-down shift-left shift-left shift-left"
        } else {
            "ctrl-end shift-left shift-left shift-left"
        };
        crate::test_support::press(cx, select_last_word);
        cx.run_until_parked();
        let first = view.read_with(cx, |view, cx| {
            let reader = view.sources[0].reader.read(cx);
            (
                reader.scroll_offset(),
                reader.selected_range(),
                reader.selected_value(),
            )
        });
        assert_ne!(first.0.y, gpui::px(0.));
        assert_eq!(first.2, "END");

        tab_to(cx, "crash-overview");
        crate::test_support::press(cx, "enter");
        tab_to(cx, "crash-details");
        crate::test_support::press(cx, "enter");
        tab_to(cx, "crash-source");
        crate::test_support::press(cx, "enter down enter");
        assert!(view.read_with(cx, |view, _| view.selected_source == 1));
        click(cx, "crash-reader");
        crate::test_support::press(cx, select_last_word);
        cx.run_until_parked();
        let second = view.read_with(cx, |view, cx| {
            let reader = view.sources[1].reader.read(cx);
            (
                reader.scroll_offset(),
                reader.selected_range(),
                reader.selected_value(),
            )
        });
        assert_ne!(second.0.y, gpui::px(0.));
        assert_eq!(second.2, "EOF");

        tab_to(cx, "crash-source");
        crate::test_support::press(cx, "enter up enter");
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, cx| {
            let reader = view.sources[0].reader.read(cx);
            view.selected_source == 0
                && reader.scroll_offset() == first.0
                && reader.selected_range() == first.1
                && reader.selected_value() == first.2
        }));
        click(cx, "crash-export");
        cx.simulate_new_path_selection(|base| Some(base.join("missing").join("export.json")));
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, cx| {
            let reader = view.sources[0].reader.read(cx);
            matches!(view.status, Some(super::Status::ExportFailed))
                && reader.scroll_offset() == first.0
                && reader.selected_range() == first.1
                && reader.selected_value() == first.2
        }));
        tab_to(cx, "crash-source");
        crate::test_support::press(cx, "enter down enter");
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, cx| {
            let reader = view.sources[1].reader.read(cx);
            view.selected_source == 1
                && reader.scroll_offset() == second.0
                && reader.selected_range() == second.1
                && reader.selected_value() == second.2
        }));
    }

    #[gpui::test]
    fn escape_cancels_a_pending_picker_before_closing_help(cx: &mut gpui::TestAppContext) {
        let directory = tempfile::tempdir();
        assert!(directory.is_ok());
        let Ok(directory) = directory else {
            return;
        };
        let late = directory.path().join("late.json");
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
        click(cx, "crash-export");
        assert!(cx.did_prompt_for_new_path());
        crate::test_support::press(cx, "escape");
        cx.run_until_parked();
        assert_eq!(cx.windows().len(), 1);
        assert!(view.read_with(cx, |view, _| {
            !view.exporting && matches!(view.status, Some(super::Status::Cancelled))
        }));
        cx.simulate_new_path_selection(|_| Some(late.clone()));
        cx.run_until_parked();
        assert!(!late.exists());
    }

    #[gpui::test]
    fn empty_output_and_truncation_are_shown_for_the_selected_source(
        cx: &mut gpui::TestAppContext,
    ) {
        let (view, cx) = crate::test_support::window(cx, |window, cx| {
            let mut report = report_with_attachment();
            report.sanitized_tail.clear();
            CrashView::new(
                report,
                std::env::temp_dir(),
                crate::Language::English,
                |_, _| {},
                window,
                cx,
            )
        });
        click(cx, "crash-details");
        assert!(cx.debug_bounds("crash-no-output").is_some());
        assert!(cx.debug_bounds("crash-truncated").is_none());
        tab_to(cx, "crash-source");
        crate::test_support::press(cx, "enter down enter");
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| view.selected_source == 1));
        assert!(cx.debug_bounds("crash-truncated").is_some());
    }

    #[gpui::test]
    fn unknown_start_failure_offers_advice_without_inventing_evidence(
        cx: &mut gpui::TestAppContext,
    ) {
        let (view, cx) = crate::test_support::window(cx, |window, cx| {
            let report = CrashReport::new(
                report().context,
                ExitOutcome::start_failed(),
                "",
                Vec::new(),
            );
            CrashView::new(
                report,
                std::env::temp_dir(),
                crate::Language::English,
                |_, _| {},
                window,
                cx,
            )
        });
        assert!(view.read_with(cx, |view, _| {
            view.report.outcome.kind == ExitKind::StartFailed
                && view.report.diagnoses[0].kind == block_launcher::crash::DiagnosisKind::Unknown
                && view.report.diagnoses[0].evidence.is_empty()
        }));
        assert!(cx.debug_bounds("crash-advice-0").is_some());
        assert!(cx.debug_bounds("crash-evidence-0").is_none());
        click(cx, "crash-details");
        assert!(cx.debug_bounds("crash-no-output").is_some());
        assert!(cx.debug_bounds("crash-truncated").is_none());
    }

    #[gpui::test]
    fn interrupted_monitoring_explains_the_game_may_still_run(cx: &mut gpui::TestAppContext) {
        let (_view, cx) = crate::test_support::window(cx, |window, cx| {
            let report = CrashReport::new(
                report().context,
                ExitOutcome::monitoring_interrupted(),
                "",
                Vec::new(),
            );
            CrashView::new(
                report,
                std::env::temp_dir(),
                crate::Language::English,
                |_, _| {},
                window,
                cx,
            )
        });
        assert!(cx.debug_bounds("crash-may-still-run").is_some());
        assert_eq!(
            super::headline(ExitKind::MonitoringInterrupted, crate::Language::English),
            "Game monitoring was interrupted"
        );
    }

    #[gpui::test]
    fn overview_exposes_advice_before_local_evidence(cx: &mut gpui::TestAppContext) {
        let (view, cx) = crate::test_support::window(cx, |window, cx| {
            let mut report = report();
            report.diagnoses = CrashReport::new(
                report.context.clone(),
                report.outcome.clone(),
                "OutOfMemoryError: Java heap space\nUnsupportedClassVersionError",
                Vec::new(),
            )
            .diagnoses;
            CrashView::new(
                report,
                std::env::temp_dir(),
                crate::Language::English,
                |_, _| {},
                window,
                cx,
            )
        });
        assert!(cx.debug_bounds("crash-advice-0").is_some());
        assert!(cx.debug_bounds("crash-evidence-text-0").is_none());
        assert_eq!(
            view.read_with(cx, |view, _| view.expanded_evidence.clone()),
            vec![false, false]
        );
        tab_to(cx, "crash-evidence-0");
        crate::test_support::press(cx, "space");
        assert_eq!(
            view.read_with(cx, |view, _| view.expanded_evidence.clone()),
            vec![true, false]
        );
        assert!(cx.debug_bounds("crash-evidence-text-0").is_some());
    }

    #[gpui::test]
    fn compact_overview_shows_the_first_suggestion(cx: &mut gpui::TestAppContext) {
        let (_view, cx) = crate::test_support::window(cx, |window, cx| {
            CrashView::new(
                report(),
                std::env::temp_dir(),
                crate::Language::English,
                |_, _| {},
                window,
                cx,
            )
        });
        cx.simulate_resize(gpui::size(gpui::px(360.), gpui::px(540.)));
        cx.run_until_parked();
        let body = cx.debug_bounds("crash-help-content").unwrap_or_default();
        let advice = cx.debug_bounds("crash-advice-0").unwrap_or_default();
        assert!(
            advice.size.height > gpui::px(0.)
                && advice.top() >= body.top()
                && advice.bottom() <= body.bottom(),
            "advice {advice:?} must be visible in body {body:?}"
        );
    }

    fn second_evidence_is_visible_after_tab(cx: &mut gpui::TestAppContext, key: &str) {
        for language in [
            crate::Language::English,
            crate::Language::SimplifiedChinese,
            crate::Language::Japanese,
        ] {
            let (view, cx) = crate::test_support::window(cx, |window, cx| {
                CrashView::new(
                    multiple_cause_report(),
                    std::env::temp_dir(),
                    language,
                    |_, _| {},
                    window,
                    cx,
                )
            });
            cx.simulate_resize(gpui::size(gpui::px(360.), gpui::px(540.)));
            tab_to_direction(cx, "crash-evidence-1", key);
            cx.run_until_parked();
            let body = cx.debug_bounds("crash-help-content").unwrap_or_default();
            let target = cx.debug_bounds("crash-evidence-1").unwrap_or_default();
            assert!(
                target.size.height > gpui::px(0.)
                    && target.top() >= body.top()
                    && target.bottom() <= body.bottom(),
                "{language:?} {key}: focused evidence {target:?}, body {body:?}"
            );
            crate::test_support::press(cx, "space");
            assert!(view.read_with(cx, |view, _| view.expanded_evidence[1]));
            drop(view);
            cx.update(|window, _| window.remove_window());
        }
    }

    #[gpui::test]
    fn tab_reveals_later_diagnosis_evidence(cx: &mut gpui::TestAppContext) {
        second_evidence_is_visible_after_tab(cx, "tab");
    }

    #[gpui::test]
    fn shift_tab_from_footer_reveals_later_diagnosis_evidence(cx: &mut gpui::TestAppContext) {
        second_evidence_is_visible_after_tab(cx, "shift-tab");
    }

    #[gpui::test]
    fn tab_reveals_each_log_toolbar_control_without_scrolling_the_footer(
        cx: &mut gpui::TestAppContext,
    ) {
        for language in [
            crate::Language::English,
            crate::Language::SimplifiedChinese,
            crate::Language::Japanese,
        ] {
            let (view, cx) = crate::test_support::window(cx, |window, cx| {
                CrashView::new(
                    report_with_attachment(),
                    std::env::temp_dir(),
                    language,
                    |_, _| {},
                    window,
                    cx,
                )
            });
            cx.simulate_resize(gpui::size(gpui::px(360.), gpui::px(540.)));
            crate::test_support::press(cx, "ctrl-f");
            let footer = cx.debug_bounds("crash-footer").unwrap_or_default();
            for key in ["tab", "shift-tab"] {
                for id in [
                    "crash-source",
                    "crash-open-folder",
                    "crash-copy-source",
                    "crash-previous-match",
                    "crash-next-match",
                    "crash-wrap",
                ] {
                    tab_to_direction(cx, id, key);
                    cx.run_until_parked();
                    let body = cx.debug_bounds("crash-help-content").unwrap_or_default();
                    let bounds_id = if id == "crash-source" {
                        "crash-source-field"
                    } else {
                        id
                    };
                    let target = cx.debug_bounds(bounds_id).unwrap_or_default();
                    assert!(
                        target.size.height > gpui::px(0.)
                            && target.top() >= body.top()
                            && target.bottom() <= body.bottom(),
                        "{language:?} {key} {id}: {target:?}, body {body:?}"
                    );
                    assert_eq!(cx.debug_bounds("crash-footer"), Some(footer));
                }
            }
            drop(view);
            cx.update(|window, _| window.remove_window());
        }
    }

    #[gpui::test]
    fn footer_remains_in_bounds_at_compact_and_wide_sizes(cx: &mut gpui::TestAppContext) {
        for (width, height) in [(360., 540.), (400., 540.), (1152., 720.)] {
            for (language, appearance) in [
                (crate::Language::English, crate::Appearance::Light),
                (crate::Language::SimplifiedChinese, crate::Appearance::Dark),
                (crate::Language::Japanese, crate::Appearance::Light),
            ] {
                let (view, cx) = crate::test_support::window(cx, |window, cx| {
                    let mut report = report();
                    report.context.profile_name =
                        "很长的世界名称とても長いインスタンスの名前测试世界很长的世界名称とても長い名前"
                            .into();
                    let mut view = CrashView::new(
                        report,
                        std::env::temp_dir(),
                        language,
                        |_, _| {},
                        window,
                        cx,
                    );
                    view.set_preferences(
                        crate::Preferences {
                            appearance,
                            language: Some(language),
                            ..Default::default()
                        },
                        window,
                        cx,
                    );
                    view
                });
                cx.simulate_resize(gpui::size(gpui::px(width), gpui::px(height)));
                cx.run_until_parked();
                let footer = cx.debug_bounds("crash-footer").unwrap_or_default();
                let body = cx.debug_bounds("crash-help-content").unwrap_or_default();
                assert!(
                    footer.top() >= body.bottom(),
                    "body {body:?} footer {footer:?}"
                );
                let metadata = cx.debug_bounds("crash-metadata").unwrap_or_default();
                assert!(
                    metadata.size.width > gpui::px(0.)
                        && metadata.left() >= body.left()
                        && metadata.right() <= body.right(),
                    "long metadata at {width}x{height} {language:?}: {metadata:?}"
                );
                let tabs =
                    crate::test_support::control_bounds(cx, "crash-details").unwrap_or_default();
                assert!(
                    tabs.size.width > gpui::px(0.)
                        && tabs.top() >= body.top()
                        && tabs.bottom() <= body.bottom(),
                    "tabs at {width}x{height} {language:?}: {tabs:?}, body {body:?}"
                );
                for id in ["crash-copy", "crash-export", "crash-close", "crash-return"] {
                    let bounds = crate::test_support::control_bounds(cx, id).unwrap_or_default();
                    assert!(
                        bounds.size.width > gpui::px(0.)
                            && bounds.left() >= gpui::px(0.)
                            && bounds.right() <= gpui::px(width)
                            && bounds.top() >= footer.top()
                            && bounds.bottom() <= gpui::px(height),
                        "{id} at {width}x{height} {language:?}: {bounds:?}, footer {footer:?}"
                    );
                }
                let actual = cx.update(|_, cx| cx.global::<crate::Palette>().background);
                let expected = if appearance == crate::Appearance::Dark {
                    crate::Palette::DARK.background
                } else {
                    crate::Palette::LIGHT.background
                };
                assert_eq!(actual, expected);
                click(cx, "crash-export");
                let footer = cx.debug_bounds("crash-footer").unwrap_or_default();
                for id in [
                    "crash-copy",
                    "crash-export",
                    "crash-cancel-export",
                    "crash-close",
                    "crash-return",
                ] {
                    let bounds = crate::test_support::control_bounds(cx, id).unwrap_or_default();
                    assert!(
                        bounds.size.width > gpui::px(0.)
                            && bounds.left() >= gpui::px(0.)
                            && bounds.right() <= gpui::px(width)
                            && bounds.top() >= footer.top()
                            && bounds.bottom() <= gpui::px(height),
                        "pending {id} at {width}x{height} {language:?}: {bounds:?}, {footer:?}"
                    );
                }
                click(cx, "crash-cancel-export");
                cx.simulate_new_path_selection(|_| None);
                cx.run_until_parked();
                drop(view);
                cx.update(|window, _| window.remove_window());
            }
        }
    }

    #[gpui::test]
    fn find_keyboard_focus_is_visible_in_a_short_window(cx: &mut gpui::TestAppContext) {
        for width in [360., 400.] {
            for language in [
                crate::Language::English,
                crate::Language::SimplifiedChinese,
                crate::Language::Japanese,
            ] {
                let (view, cx) = crate::test_support::window(cx, |window, cx| {
                    CrashView::new(
                        report_with_attachment(),
                        std::env::temp_dir(),
                        language,
                        |_, _| {},
                        window,
                        cx,
                    )
                });
                cx.simulate_resize(gpui::size(gpui::px(width), gpui::px(540.)));
                crate::test_support::press(cx, "ctrl-f");
                cx.run_until_parked();
                let body = cx.debug_bounds("crash-help-content").unwrap_or_default();
                let query = cx.debug_bounds("crash-find-field").unwrap_or_default();
                let reader = cx.debug_bounds("crash-reader").unwrap_or_default();
                assert!(
                    query.size.height > gpui::px(0.)
                        && query.top() >= body.top()
                        && query.bottom() <= body.bottom(),
                    "{width} {language:?}: query {query:?}, body {body:?}"
                );
                assert!(
                    reader.size.height > gpui::px(0.)
                        && reader.top() >= body.top()
                        && reader.bottom() <= body.bottom(),
                    "{width} {language:?}: reader {reader:?}, body {body:?}"
                );
                for (id, key) in [
                    ("crash-open-folder", "crash.open_folder"),
                    ("crash-copy-source", "crash.copy_source"),
                    ("crash-previous-match", "crash.previous_match"),
                    ("crash-next-match", "crash.next_match"),
                ] {
                    let name = cx.update(|window, _| {
                        gpui_base::test_support::find(window, &[], &id.into())
                            .and_then(|node| node.label().map(str::to_owned))
                    });
                    assert_eq!(name.as_deref(), Some(tr!(language, key).as_str()));
                }
                drop(view);
                cx.update(|window, _| window.remove_window());
            }
        }
    }

    #[gpui::test]
    fn tab_reveals_search_and_reader_in_a_short_window(cx: &mut gpui::TestAppContext) {
        use gpui::Focusable as _;

        let (view, cx) = crate::test_support::window(cx, |window, cx| {
            CrashView::new(
                report_with_attachment(),
                std::env::temp_dir(),
                crate::Language::English,
                |_, _| {},
                window,
                cx,
            )
        });
        cx.simulate_resize(gpui::size(gpui::px(360.), gpui::px(540.)));
        tab_to(cx, "crash-details");
        crate::test_support::press(cx, "enter");
        for _ in 0..20 {
            if cx.update(|window, cx| {
                view.read(cx).sources[0]
                    .query
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window)
            }) {
                break;
            }
            crate::test_support::press(cx, "tab");
        }
        cx.run_until_parked();
        assert!(cx.update(|window, cx| {
            view.read(cx).sources[0]
                .query
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        }));
        let body = cx.debug_bounds("crash-help-content").unwrap_or_default();
        let query = cx.debug_bounds("crash-find-field").unwrap_or_default();
        assert!(
            query.size.height > gpui::px(0.)
                && query.top() >= body.top()
                && query.bottom() <= body.bottom(),
            "query {query:?}, body {body:?}"
        );
        for _ in 0..20 {
            if cx.update(|window, cx| {
                view.read(cx).sources[0]
                    .reader
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window)
            }) {
                break;
            }
            crate::test_support::press(cx, "tab");
        }
        cx.run_until_parked();
        assert!(cx.update(|window, cx| {
            view.read(cx).sources[0]
                .reader
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        }));
        let reader = cx.debug_bounds("crash-reader").unwrap_or_default();
        assert!(
            reader.size.height > gpui::px(0.)
                && reader.top() >= body.top()
                && reader.bottom() <= body.bottom(),
            "reader {reader:?}, body {body:?}"
        );
    }

    #[test]
    fn exit_facts_remain_distinct_from_diagnosis() {
        let mut report = report();
        for (kind, title) in [
            (ExitKind::Normal, "Minecraft closed normally"),
            (ExitKind::Stopped, "Minecraft was stopped"),
            (ExitKind::StartFailed, "Minecraft could not start"),
            (ExitKind::Abnormal, "Minecraft closed unexpectedly"),
            (
                ExitKind::MonitoringInterrupted,
                "Game monitoring was interrupted",
            ),
        ] {
            report.outcome.kind = kind;
            assert_eq!(super::headline(kind, crate::Language::English), title);
        }
        report.outcome = ExitOutcome::start_failed();
        assert_eq!(
            super::localized_exit_details(&report, crate::Language::English),
            "No native exit status was available."
        );
        report.outcome.signal = Some(9);
        report.outcome.windows_status = Some(0xC000_0005);
        assert_eq!(
            super::localized_exit_details(&report, crate::Language::English),
            "Signal: 9 · Windows status: 0xC0000005"
        );
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
        click(cx, "crash-copy");
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
            click(cx, "crash-export");
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
        assert!(view.read_with(cx, |view, _| !view.exporting));
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
        click(cx, "crash-export");
        assert!(cx.did_prompt_for_new_path());
        assert!(view.read_with(cx, |view, _| view.exporting));

        // Cancellation remains available while the destination is pending.
        click(cx, "crash-cancel-export");
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
        click(cx, "crash-export");
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
        click(cx, "crash-export");
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
