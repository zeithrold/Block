//! A real, explicitly requested network probe of the current Downloads draft.
//! Request, draft entity and options identities reject late or edited results.
#![expect(
    clippy::redundant_pub_crate,
    reason = "Settings and engine share private probe data."
)]
use super::{
    Block,
    components::{self, ButtonVariant, ControlState},
    note,
};
use block_launcher::DownloadOptions;
use gpui::{Context, InteractiveElement as _, ParentElement as _, Styled as _, Window, div};

pub(super) const ENDPOINT: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
#[derive(Clone, Debug)]
pub(super) struct Request {
    pub id: u64,
    pub draft: u64,
    pub options: DownloadOptions,
}
#[derive(Clone, Debug, Default)]
pub(super) struct State {
    sequence: u64,
    pending: Option<Request>,
    result: Option<(Request, TestResult)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum TestResult {
    Response(u16),
    Failure(String),
    EngineUnavailable,
}

impl Block {
    fn connection_identity(&self, cx: &gpui::App) -> Option<(u64, DownloadOptions)> {
        let draft = self.settings_form.as_ref()?;
        Some((
            draft.proxy_url.entity_id().as_u64(),
            draft.parse_downloads(cx).ok()?,
        ))
    }
    fn test_connection(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(draft) = &self.settings_form else {
            return;
        };
        let options = match draft.parse_downloads(cx) {
            Ok(options) => options,
            Err(error) => {
                self.settings_failure(error, window, cx);
                return;
            }
        };
        let draft = draft.proxy_url.entity_id().as_u64();
        if self
            .shell
            .connection
            .pending
            .as_ref()
            .is_some_and(|pending| pending.draft == draft && pending.options == options)
        {
            return;
        }
        self.shell.connection.sequence = self.shell.connection.sequence.wrapping_add(1);
        let request = Request {
            id: self.shell.connection.sequence,
            draft,
            options,
        };
        self.shell.connection.result = None;
        if let Some(engine) = &self.engine {
            self.shell.connection.pending = Some(request.clone());
            engine.test_connection(request);
        } else {
            self.shell.connection.result = Some((request, TestResult::EngineUnavailable));
        }
        cx.notify();
    }
    pub(super) fn connection_finished(
        &mut self,
        request: Request,
        result: Result<u16, String>,
        cx: &gpui::App,
    ) {
        let pending = self
            .shell
            .connection
            .pending
            .as_ref()
            .is_some_and(|pending| {
                pending.id == request.id
                    && pending.draft == request.draft
                    && pending.options == request.options
            });
        if !pending {
            return;
        }
        self.shell.connection.pending = None;
        if self
            .connection_identity(cx)
            .is_some_and(|(draft, options)| draft == request.draft && options == request.options)
        {
            self.shell.connection.result = Some((
                request,
                match result {
                    Ok(status) => TestResult::Response(status),
                    Err(error) => TestResult::Failure(error),
                },
            ));
        }
    }
    pub(super) fn render_connection_test(&self, cx: &Context<'_, Self>) -> gpui::Div {
        let language = self.language();
        let identity = self.connection_identity(cx);
        let current = |request: &Request| {
            identity.as_ref().is_some_and(|(draft, options)| {
                *draft == request.draft && *options == request.options
            })
        };
        let running = self.shell.connection.pending.as_ref().is_some_and(current);
        let result = self
            .shell
            .connection
            .result
            .as_ref()
            .filter(|(request, _)| current(request));
        let description = if running {
            tr!(language, "connection.testing")
        } else {
            result.map_or_else(
                || tr!(language, "connection.description"),
                |(_, result)| match result {
                    TestResult::Response(status) => {
                        tr!(language, "connection.success", status = status)
                    }
                    TestResult::Failure(error) => tr!(language, "connection.failed", error = error),
                    TestResult::EngineUnavailable => tr!(language, "connection.unavailable"),
                },
            )
        };

        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(components::button(
                "test-proxy-connection",
                if running {
                    tr!(language, "connection.testing_button")
                } else {
                    tr!(language, "connection.test_button")
                },
                self.palette,
                ButtonVariant::Outline,
                ControlState {
                    busy: running,
                    ..Default::default()
                },
                cx.listener(|view, (): &(), window, cx| view.test_connection(window, cx)),
            ))
            .child(div().id("connection-test-result").child(note(description)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[gpui::test]
    fn edited_replaced_and_saved_drafts_reject_late_connection_results(
        cx: &mut gpui::TestAppContext,
    ) {
        let (view, cx) = crate::test_support::window(cx, super::super::tests::test_view);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.open_settings(cx);
                let identity = view.connection_identity(cx);
                assert!(identity.is_some());
                let Some((draft, options)) = identity else {
                    return;
                };
                let old = Request {
                    id: 1,
                    draft,
                    options,
                };
                view.shell.connection.pending = Some(old.clone());
                if let Some(draft) = &mut view.settings_form {
                    draft.proxy_mode = 1;
                }
                view.connection_finished(old.clone(), Ok(200), cx);
                assert!(view.shell.connection.result.is_none());
                let identity = view.connection_identity(cx);
                assert!(identity.is_some());
                let Some((draft, options)) = identity else {
                    return;
                };
                let current = Request {
                    id: 2,
                    draft,
                    options,
                };
                view.shell.connection.pending = Some(current.clone());
                view.connection_finished(old, Err("late failure".into()), cx);
                assert!(
                    view.shell
                        .connection
                        .pending
                        .as_ref()
                        .is_some_and(|request| request.id == 2)
                );
                view.connection_finished(current.clone(), Ok(204), cx);
                assert!(
                    view.shell
                        .connection
                        .result
                        .as_ref()
                        .is_some_and(|(_, result)| *result == TestResult::Response(204))
                );
                view.shell.connection.pending = Some(current.clone());
                view.open_settings(cx); // Save/reopen creates a fresh draft identity.
                view.shell.connection.result = None;
                view.connection_finished(current, Ok(200), cx);
                assert!(view.shell.connection.result.is_none());
                assert!(!view.busy);
                // Real validation on invalid proxy prevents a request.
                if let Some(draft) = &mut view.settings_form {
                    draft.proxy_mode = 2;
                    draft.proxy_url.update(cx, |field, cx| {
                        field.set_value("ftp://127.0.0.1:1080", cx);
                    });
                }
                view.test_connection(window, cx);
                assert!(
                    view.settings_form
                        .as_ref()
                        .is_some_and(|draft| draft.error.is_some())
                );
                assert!(view.shell.connection.pending.is_none());
            });
        });
    }
}
