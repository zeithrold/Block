//! Allowlisted UI lifecycle events and change-only window observations.
#![expect(
    clippy::redundant_pub_crate,
    reason = "These diagnostics are private to the UI."
)]

use std::time::{Duration, Instant};

use block_launcher::{crash::ExitKind, session_host::RunState};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Operation {
    CreateProfile,
    SaveSetup,
    CreateGame,
    SaveInstance,
    SaveProfile,
    SeparateProfiles,
    CopyProfile,
    DeleteProfile,
    RemoveAccount,
    Repair,
    Launch,
    LaunchRecipe,
    DiscoverAuthProvider,
    SelectAuthCharacter,
    AuthenticateProvider,
    AuthenticateMicrosoft,
    SaveAccount,
    SaveProvider,
    RemoveProvider,
}

impl Operation {
    const fn label(self) -> &'static str {
        match self {
            Self::CreateProfile => "create_profile",
            Self::SaveSetup => "save_setup",
            Self::CreateGame => "create_game",
            Self::SaveInstance => "save_instance",
            Self::SaveProfile => "save_profile",
            Self::SeparateProfiles => "separate_profiles",
            Self::CopyProfile => "copy_profile",
            Self::DeleteProfile => "delete_profile",
            Self::RemoveAccount => "remove_account",
            Self::Repair => "repair",
            Self::Launch => "launch",
            Self::LaunchRecipe => "launch_recipe",
            Self::DiscoverAuthProvider => "discover_auth_provider",
            Self::SelectAuthCharacter => "select_auth_character",
            Self::AuthenticateProvider => "authenticate_provider",
            Self::AuthenticateMicrosoft => "authenticate_microsoft",
            Self::SaveAccount => "save_account",
            Self::SaveProvider => "save_provider",
            Self::RemoveProvider => "remove_provider",
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum Outcome {
    Succeeded,
    Failed,
    Cancelled,
    Accepted,
    ViewClosed,
}

impl Outcome {
    const fn label(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Accepted => "accepted",
            Self::ViewClosed => "view_closed",
        }
    }
}

pub(super) fn operation_started(view_id: u64, revision: u64, operation: Operation) {
    tracing::info!(
        event = "ui.operation.started",
        view_id,
        operation_revision = revision,
        operation = operation.label(),
    );
}

pub(super) fn operation_finished(
    view_id: u64,
    revision: u64,
    operation: Operation,
    outcome: Outcome,
) {
    if matches!(outcome, Outcome::Failed) {
        tracing::error!(
            event = "ui.operation.finished",
            view_id,
            operation_revision = revision,
            operation = operation.label(),
            outcome = outcome.label(),
        );
    } else {
        tracing::info!(
            event = "ui.operation.finished",
            view_id,
            operation_revision = revision,
            operation = operation.label(),
            outcome = outcome.label(),
        );
    }
}

/// Session records originate in storage; only canonical UUIDs are loggable.
pub(super) fn session_id(value: &str) -> Option<uuid::Uuid> {
    let id = uuid::Uuid::parse_str(value).ok()?;
    (id.to_string() == value).then_some(id)
}

pub(super) fn session_handoff(view_id: u64, revision: u64, id: uuid::Uuid) {
    tracing::info!(
        event = "ui.session.handoff",
        view_id,
        operation_revision = revision,
        session_id = %id,
    );
}

pub(super) fn session_state(
    view_id: u64,
    id: uuid::Uuid,
    state: RunState,
    outcome: Option<ExitKind>,
) {
    let state = match state {
        RunState::Starting => "starting",
        RunState::Preparing => "preparing",
        RunState::Running => "running",
        RunState::Finished => "finished",
        RunState::MonitoringInterrupted => "monitoring_interrupted",
    };
    let outcome = match outcome {
        Some(ExitKind::Normal) => "normal",
        Some(ExitKind::Stopped) => "stopped",
        Some(ExitKind::StartFailed) => "start_failed",
        Some(ExitKind::Abnormal) => "abnormal",
        Some(ExitKind::MonitoringInterrupted) => "monitoring_interrupted",
        None => "none",
    };
    tracing::info!(
        event = "ui.session.state_changed",
        view_id,
        session_id = %id,
        state,
        outcome,
    );
}

const SIZE_SETTLE: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, Debug, PartialEq)]
struct Size {
    width: f32,
    height: f32,
}

#[derive(Debug, Default)]
pub(super) struct WindowChanges {
    observed_size: Option<Size>,
    logged_size: Option<Size>,
    size_changed_at: Option<Instant>,
    scale: Option<f32>,
    reduced_motion: Option<bool>,
}

impl WindowChanges {
    pub(super) fn observe(
        &mut self,
        view_id: u64,
        width: f32,
        height: f32,
        scale: f32,
        reduced_motion: bool,
        now: Instant,
    ) {
        let size = Size { width, height };
        match self.observed_size {
            None => self.logged_size = Some(size),
            Some(previous) if previous != size => self.size_changed_at = Some(now),
            Some(_) => {}
        }
        self.observed_size = Some(size);
        if self.scale.is_some_and(|previous| previous != scale) {
            tracing::debug!(event = "ui.window.scale_changed", view_id, scale);
        }
        self.scale = Some(scale);
        if self
            .reduced_motion
            .is_some_and(|previous| previous != reduced_motion)
        {
            tracing::debug!(
                event = "ui.window.reduced_motion_changed",
                view_id,
                reduced_motion,
            );
        }
        self.reduced_motion = Some(reduced_motion);
    }

    pub(super) fn flush_size(&mut self, view_id: u64, now: Instant) {
        let Some(changed_at) = self.size_changed_at else {
            return;
        };
        if now.saturating_duration_since(changed_at) < SIZE_SETTLE {
            return;
        }
        self.size_changed_at = None;
        if let Some(size) = self.observed_size
            && self.logged_size != Some(size)
        {
            tracing::debug!(
                event = "ui.window.size_changed",
                view_id,
                width = size.width,
                height = size.height,
            );
            self.logged_size = Some(size);
        }
    }
}

#[derive(Default)]
pub(super) struct FailureNotice(pub bool);
impl gpui::Global for FailureNotice {}

pub(super) const FAILURE_MESSAGE: &str =
    "Launcher logs are unavailable. Check disk space and permissions.";

pub(super) fn claim_failure_notice(failed: bool, cx: &mut gpui::App) -> bool {
    let notice = cx.default_global::<FailureNotice>();
    if !failed || notice.0 {
        return false;
    }
    notice.0 = true;
    true
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        fmt, io,
        sync::{Arc, Mutex},
        time::{Duration, Instant},
    };

    use tracing::{
        Event, Level, Metadata, Subscriber,
        field::{Field, Visit},
        span::{Attributes, Id, Record},
    };

    use super::{Operation, WindowChanges};
    use crate::{Block, engine::Message};

    #[derive(Clone, Debug)]
    struct Captured {
        level: Level,
        fields: BTreeMap<String, String>,
    }

    impl Captured {
        fn event(&self) -> Option<&str> {
            self.fields.get("event").map(String::as_str)
        }
    }

    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Vec<Captured>>>);

    #[derive(Default)]
    struct Fields(BTreeMap<String, String>);

    impl Visit for Fields {
        fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
            self.0.insert(field.name().into(), format!("{value:?}"));
        }

        fn record_str(&mut self, field: &Field, value: &str) {
            self.0.insert(field.name().into(), value.into());
        }
    }

    impl Subscriber for Capture {
        fn enabled(&self, _: &Metadata<'_>) -> bool {
            true
        }

        fn new_span(&self, _: &Attributes<'_>) -> Id {
            Id::from_u64(1)
        }

        fn record(&self, _: &Id, _: &Record<'_>) {}

        fn record_follows_from(&self, _: &Id, _: &Id) {}

        fn event(&self, event: &Event<'_>) {
            let mut fields = Fields::default();
            event.record(&mut fields);
            self.0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(Captured {
                    level: *event.metadata().level(),
                    fields: fields.0,
                });
        }

        fn enter(&self, _: &Id) {}

        fn exit(&self, _: &Id) {}
    }

    impl Capture {
        fn events(&self) -> Vec<Captured> {
            self.0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
        }
    }

    #[derive(Clone, Default)]
    struct JsonSink(Arc<Mutex<Vec<u8>>>);

    impl io::Write for JsonSink {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[gpui::test]
    fn json_records_include_event_identity_and_outcome_severity(cx: &mut gpui::TestAppContext) {
        let directory = tempfile::tempdir();
        assert!(directory.is_ok());
        let Ok(directory) = directory else {
            return;
        };
        let destination = directory.path().join("private-existing.zip");
        assert!(std::fs::write(&destination, b"original").is_ok());
        let selected_destination = destination.clone();
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        let sink = JsonSink::default();
        let writer = sink.clone();
        let subscriber = tracing_subscriber::fmt()
            .json()
            .with_max_level(Level::DEBUG)
            .without_time()
            .with_ansi(false)
            .with_writer(move || writer.clone())
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            super::operation_started(1, 7, Operation::Repair);
            super::operation_finished(1, 7, Operation::Repair, super::Outcome::Failed);
            super::operation_finished(1, 8, Operation::Repair, super::Outcome::Cancelled);
            super::operation_finished(1, 9, Operation::Repair, super::Outcome::Succeeded);
            let mut window = WindowChanges::default();
            let now = Instant::now();
            window.observe(1, 360., 720., 1., false, now);
            window.observe(1, 360., 720., 2., false, now);
            view.update(cx, |view, _| {
                let state = &mut view.shell.import_games;
                state.batch = Some(uuid::Uuid::from_u128(10));
                state.running = true;
                state.submitted = vec!["private-candidate".into()];
                state.outcomes.insert(
                    "private-candidate".into(),
                    crate::import_games::Outcome::Failed("private error".into()),
                );
                view.finish_game_import(uuid::Uuid::from_u128(10));
            });
            view.update(cx, |view, cx| view.export_launcher_logs(cx));
            assert!(cx.did_prompt_for_new_path());
            cx.simulate_new_path_selection(|_| None);
            cx.run_until_parked();
            view.update(cx, |view, cx| view.export_launcher_logs(cx));
            cx.simulate_new_path_selection(move |_| Some(selected_destination));
            cx.run_until_parked();
        });
        assert_eq!(
            std::fs::read(destination).ok().as_deref(),
            Some(&b"original"[..])
        );
        assert!(!view.read_with(cx, |view, _| view.exporting_logs));
        let bytes = sink
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let output = std::str::from_utf8(&bytes);
        assert!(output.is_ok());
        let Ok(output) = output else {
            return;
        };
        let rows = output
            .lines()
            .map(serde_json::from_str::<serde_json::Value>)
            .collect::<Result<Vec<_>, _>>();
        assert!(rows.is_ok());
        let Ok(rows) = rows else {
            return;
        };
        assert!(rows.iter().all(|row| {
            !row["fields"]["event"]
                .as_str()
                .is_some_and(|event| event.starts_with("ui."))
                || !row.to_string().contains("private")
        }));
        let contains = |event: &str, outcome: &str, level: &str| {
            rows.iter().any(|row| {
                row["fields"]["event"] == event
                    && row["fields"]["outcome"] == outcome
                    && row["level"] == level
            })
        };
        assert!(
            rows.iter()
                .any(|row| row["fields"]["event"] == "ui.operation.started"
                    && row["fields"]["view_id"] == 1
                    && row["fields"]["operation_revision"] == 7
                    && row["level"] == "INFO")
        );
        assert!(contains("ui.operation.finished", "failed", "ERROR"));
        assert!(contains("ui.operation.finished", "cancelled", "INFO"));
        assert!(contains("ui.operation.finished", "succeeded", "INFO"));
        assert!(
            rows.iter()
                .any(|row| row["fields"]["event"] == "ui.window.scale_changed"
                    && row["fields"]["scale"] == 2.0
                    && row["level"] == "DEBUG")
        );
        assert!(contains("ui.import.finished", "failed", "ERROR"));
        assert!(contains("ui.logs.export_finished", "cancelled", "INFO"));
        assert!(contains("ui.logs.export_finished", "failed", "ERROR"));
    }

    #[test]
    fn window_logs_only_real_changes_and_coalesces_resize_frames() {
        let capture = Capture::default();
        tracing::subscriber::with_default(capture.clone(), || {
            let mut changes = WindowChanges::default();
            let now = Instant::now();
            changes.observe(5, 360., 720., 1., false, now);
            changes.observe(5, 360., 720., 1., false, now);
            changes.observe(5, 500., 720., 1., false, now);
            changes.flush_size(5, now + Duration::from_millis(100));
            changes.observe(5, 800., 720., 1., false, now + Duration::from_millis(150));
            changes.flush_size(5, now + Duration::from_millis(300));
            assert!(capture.events().is_empty());
            changes.flush_size(5, now + Duration::from_millis(400));
            changes.flush_size(5, now + Duration::from_millis(800));
            changes.observe(5, 800., 720., 2., true, now + Duration::from_millis(800));
            changes.observe(5, 800., 720., 2., true, now + Duration::from_millis(900));
        });
        let events = capture.events();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].event(), Some("ui.window.size_changed"));
        assert_eq!(events[0].level, Level::DEBUG);
        assert_eq!(
            events[0].fields.get("view_id").map(String::as_str),
            Some("5")
        );
        assert_eq!(
            events[0].fields.get("width").map(String::as_str),
            Some("800.0")
        );
        assert_eq!(events[1].event(), Some("ui.window.scale_changed"));
        assert_eq!(events[2].event(), Some("ui.window.reduced_motion_changed"));
    }

    #[test]
    fn session_ids_must_be_canonical_before_logging() {
        assert!(super::session_id("00000000-0000-4000-8000-000000000001").is_some());
        assert!(super::session_id("private-session-name").is_none());
        assert!(super::session_id("00000000000040008000000000000001").is_none());
    }

    #[test]
    fn operation_revisions_are_scoped_to_their_view() {
        let capture = Capture::default();
        tracing::subscriber::with_default(capture.clone(), || {
            super::operation_started(1, 7, Operation::Repair);
            super::operation_started(2, 7, Operation::Launch);
            super::operation_finished(1, 7, Operation::Repair, super::Outcome::Succeeded);
        });
        let events = capture.events();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].event(), Some("ui.operation.started"));
        assert_eq!(events[2].event(), Some("ui.operation.finished"));
        assert_eq!(
            events[0].fields.get("view_id"),
            events[2].fields.get("view_id")
        );
        assert_ne!(
            events[0].fields.get("view_id"),
            events[1].fields.get("view_id")
        );
        assert_eq!(
            events[0].fields.get("operation_revision"),
            events[2].fields.get("operation_revision")
        );
    }

    #[gpui::test]
    fn overlay_events_identify_only_the_surface_not_its_payload(cx: &mut gpui::TestAppContext) {
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        let capture = Capture::default();
        tracing::subscriber::with_default(capture.clone(), || {
            cx.update(|window, cx| {
                view.update(cx, |view, cx| {
                    assert!(view.shell.surfaces.open(
                        crate::shell::Surface::RemoveAccount("private-account-id".into()),
                        window,
                        cx,
                    ));
                    view.shell.surfaces.discard();
                });
            });
        });
        let events = capture.events();
        assert_eq!(events.len(), 2);
        assert!(
            events
                .iter()
                .all(|event| event.event() == Some("ui.overlay.changed")
                    && event.level == Level::DEBUG
                    && event.fields.get("overlay").map(String::as_str) == Some("remove_account"))
        );
        assert!(!format!("{events:?}").contains("private-account-id"));
    }

    #[gpui::test]
    fn cancelled_import_reports_counts_without_candidate_data(cx: &mut gpui::TestAppContext) {
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        let batch = uuid::Uuid::from_u128(9);
        view.update(cx, |view, _| {
            let state = &mut view.shell.import_games;
            state.batch = Some(batch);
            state.running = true;
            state.submitted = vec![
                "private-saved".into(),
                "private-failed".into(),
                "private-pending".into(),
            ];
            state
                .outcomes
                .insert("private-saved".into(), crate::import_games::Outcome::Saved);
            state.outcomes.insert(
                "private-failed".into(),
                crate::import_games::Outcome::Failed("private error".into()),
            );
            state.cancel.cancel();
        });
        let capture = Capture::default();
        tracing::subscriber::with_default(capture.clone(), || {
            view.update(cx, |view, _| {
                view.finish_game_import(batch);
                view.finish_game_import(batch);
            });
        });
        let events = capture.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event(), Some("ui.import.finished"));
        assert_eq!(events[0].level, Level::INFO);
        for (field, value) in [
            ("outcome", "cancelled"),
            ("saved", "1"),
            ("failed", "1"),
            ("cancelled", "1"),
        ] {
            assert_eq!(events[0].fields.get(field).map(String::as_str), Some(value));
        }
        assert!(!format!("{events:?}").contains("private"));
    }

    #[gpui::test]
    fn cancelled_operation_logs_only_its_current_revision_and_safe_outcome(
        cx: &mut gpui::TestAppContext,
    ) {
        let (view, cx) = crate::test_support::window(cx, crate::tests::test_view);
        let (sender, receiver) = tokio::sync::mpsc::channel(4);
        view.update(cx, |view, _| {
            view.operation_revision = 7;
            view.logged_operation = Some(Operation::AuthenticateProvider);
            view.messages = Some(receiver);
            view.busy = true;
            view.operation_cancellable = true;
            view.cancel.cancel();
        });
        for revision in [6, 7] {
            assert!(
                sender
                    .try_send(Message::ForOperation {
                        operation: revision,
                        message: Box::new(Message::Finished(Err(
                            "private error, path and token".into(),
                        ))),
                    })
                    .is_ok()
            );
        }
        let capture = Capture::default();
        tracing::subscriber::with_default(capture.clone(), || view.update(cx, Block::poll));
        let events = capture.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event(), Some("ui.operation.finished"));
        assert_eq!(events[0].level, Level::INFO);
        assert_eq!(
            events[0]
                .fields
                .get("operation_revision")
                .map(String::as_str),
            Some("7")
        );
        assert_eq!(
            events[0].fields.get("operation").map(String::as_str),
            Some("authenticate_provider")
        );
        assert_eq!(
            events[0].fields.get("outcome").map(String::as_str),
            Some("cancelled")
        );
        assert!(!format!("{events:?}").contains("private error"));
        assert!(view.read_with(cx, |view, _| view.status == "Cancelled · retry when ready"));
    }
}
