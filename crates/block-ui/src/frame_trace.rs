//! Debug-only paint measurements and replay against disposable scenario data.
use super::Presentation;
use crate::{Block, Tab, UiAction};
use gpui::{Context, IntoElement, Styled as _, WeakEntity, Window};
use serde_json::{Value, json};
use std::{
    fs::File,
    io::Write as _,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

struct Trace {
    origin: Instant,
    file: File,
}
static TRACE: OnceLock<Option<Mutex<Trace>>> = OnceLock::new();
fn trace() -> Option<&'static Mutex<Trace>> {
    TRACE
        .get_or_init(|| {
            let path = std::env::var_os("BLOCK_FRAME_MOTION_TRACE")?;
            File::create(path).ok().map(|file| {
                Mutex::new(Trace {
                    origin: Instant::now(),
                    file,
                })
            })
        })
        .as_ref()
}
pub(crate) fn enabled() -> bool {
    trace().is_some()
}
fn record(mut value: Value) {
    if let Some(trace) = trace()
        && let Ok(mut trace) = trace.lock()
    {
        value["t_ms"] = json!(trace.origin.elapsed().as_secs_f64() * 1000.);
        if serde_json::to_writer(&mut trace.file, &value).is_ok() {
            let _ = trace.file.write_all(b"\n");
        }
    }
}
pub(super) fn probe(value: Presentation, home: bool, navigation: Value) -> impl IntoElement {
    gpui::canvas(|_, _, _| {}, move |bounds, (), window, _| {
        let native = window.bounds();
        record(json!({"type":"paint","home":home,"running":value.active,
            "native_x":f32::from(native.origin.x),"native_y":f32::from(native.origin.y),
            "native_width":f32::from(native.size.width),"native_height":f32::from(native.size.height),
            "viewport_width":f32::from(window.viewport_size().width),"viewport_height":f32::from(window.viewport_size().height),
            "body_x":f32::from(bounds.origin.x),"body_y":f32::from(bounds.origin.y),
            "home_opacity":value.home.opacity,"home_y":value.home.y,
            "sidebar_opacity":value.sidebar.opacity,"sidebar_y":value.sidebar.y,
            "detail_opacity":value.detail.opacity,"detail_x":value.detail.x,
            "requested_width":value.outer_width,"layout_width":value.manage_width,
            "navigation":navigation,"active":window.is_window_active(),"dpr":window.scale_factor()}));
    }).absolute().inset_0()
}
pub(super) fn part_probe(part: &'static str, outgoing: bool) -> impl IntoElement {
    gpui::canvas(|_, _, _| {}, move |bounds, (), _, _| {
        record(json!({"type":"pane","part":part,"outgoing":outgoing,"x":f32::from(bounds.origin.x),"y":f32::from(bounds.origin.y),"width":f32::from(bounds.size.width),"height":f32::from(bounds.size.height)}));
    }).absolute().inset_0()
}
pub(crate) fn fragment_probe(scope: usize) -> impl IntoElement {
    gpui::canvas(|_, _, _| {}, move |bounds, (), _, _| {
        record(json!({"type":"fragment","scope":scope,"x":f32::from(bounds.origin.x),"y":f32::from(bounds.origin.y),"width":f32::from(bounds.size.width),"height":f32::from(bounds.size.height)}));
    }).absolute().inset_0()
}
pub(crate) fn start(owner: WeakEntity<Block>, window: &mut Window, cx: &mut Context<'_, Block>) {
    if !enabled() {
        return;
    }
    record(
        json!({"type":"meta","clock":"native paint submission, not display presentation","schema":1}),
    );
    window
        .spawn(cx, async move |cx| {
            loop {
                match cx.update(|window, _| window.is_window_active()) {
                    Ok(true) => break,
                    Ok(false) => {
                        cx.background_executor()
                            .timer(Duration::from_millis(50))
                            .await
                    }
                    Err(_) => return,
                }
            }
            cx.background_executor()
                .timer(Duration::from_millis(1500))
                .await;
            let start = cx.background_executor().now();
            let actions = if std::env::var_os("BLOCK_PANE_MOTION_REPLAY").is_some() {
                [
                    (0, UiAction::Tab(Tab::Accounts)),
                    (700, UiAction::Tab(Tab::Settings)),
                    (1200, UiAction::SettingsPage(1)),
                    (1550, UiAction::SettingsPage(3)),
                    (1900, UiAction::Tab(Tab::Accounts)),
                    (2300, UiAction::Tab(Tab::Settings)),
                    (2400, UiAction::Tab(Tab::About)),
                    (2900, UiAction::Tab(Tab::Instances)),
                    (3400, UiAction::Tab(Tab::Home)),
                    (4100, UiAction::Tab(Tab::Accounts)),
                ]
            } else {
                [
                    (0, UiAction::Tab(Tab::Accounts)),
                    (1000, UiAction::Tab(Tab::Home)),
                    (2000, UiAction::Tab(Tab::Settings)),
                    (3000, UiAction::Tab(Tab::Home)),
                    (4000, UiAction::Tab(Tab::Accounts)),
                    (4100, UiAction::Tab(Tab::Home)),
                    (4600, UiAction::Tab(Tab::Accounts)),
                    (4900, UiAction::Tab(Tab::Home)),
                    (6000, UiAction::Tab(Tab::About)),
                    (7000, UiAction::Tab(Tab::Home)),
                ]
            };
            for (at, action) in actions {
                let delay = (start + Duration::from_millis(at))
                    .saturating_duration_since(cx.background_executor().now());
                cx.background_executor().timer(delay).await;
                if cx
                    .update(|window, cx| {
                        let label = match &action {
                            UiAction::Tab(tab) => format!("{tab:?}"),
                            action => format!("{action:?}"),
                        };
                        record(json!({"type":"action","tab":label,"scheduled_ms":at}));
                        let _ = owner.update(cx, |view, cx| view.handle(action, window, cx));
                    })
                    .is_err()
                {
                    return;
                }
            }
            cx.background_executor()
                .timer(Duration::from_millis(800))
                .await;
            record(json!({"type":"complete"}));
        })
        .detach();
}
