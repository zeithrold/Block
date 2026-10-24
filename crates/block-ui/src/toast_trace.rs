//! Opt-in, debug-only Toast replay and native paint measurements.

use crate::feedback::{Toast, ToastKind, ToastLayer};
use gpui::{Bounds, Context, IntoElement, Pixels, Styled as _, Window, point, px, size};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::Write as _,
    sync::{Mutex, OnceLock},
    time::Duration,
};

#[derive(Clone, Copy)]
struct PaintBounds {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

struct Trace {
    origin: std::time::Instant,
    file: File,
    bounds: HashMap<u64, PaintBounds>,
    paint_t_ms: HashMap<u64, f64>,
    entered: HashSet<u64>,
}

static TRACE: OnceLock<Option<Mutex<Trace>>> = OnceLock::new();

fn trace() -> Option<&'static Mutex<Trace>> {
    TRACE
        .get_or_init(|| {
            let path = std::env::var_os("BLOCK_TOAST_MOTION_TRACE")?;
            match File::create(path) {
                Ok(file) => Some(Mutex::new(Trace {
                    origin: std::time::Instant::now(),
                    file,
                    bounds: HashMap::new(),
                    paint_t_ms: HashMap::new(),
                    entered: HashSet::new(),
                })),
                Err(error) => {
                    eprintln!("Could not create toast motion trace: {error}");
                    None
                }
            }
        })
        .as_ref()
}

pub(crate) fn enabled() -> bool {
    trace().is_some()
}

fn record(mut value: Value) {
    let Some(trace) = trace() else { return };
    let Ok(mut trace) = trace.lock() else { return };
    value["t_ms"] = json!(trace.origin.elapsed().as_secs_f64() * 1000.);
    if serde_json::to_writer(&mut trace.file, &value).is_ok() {
        let _ = trace.file.write_all(b"\n");
    }
}

pub(super) fn probe(
    id: u64,
    enter_progress: Option<f32>,
    exit_progress: Option<f32>,
    ending: bool,
    reduced: bool,
) -> impl IntoElement {
    gpui::canvas(
        |_, _, _| (),
        move |bounds: Bounds<Pixels>, (), window, _| {
            let sample = PaintBounds {
                x: f32::from(bounds.origin.x),
                y: f32::from(bounds.origin.y),
                width: f32::from(bounds.size.width),
                height: f32::from(bounds.size.height),
            };
            if let Some(trace) = trace()
                && let Ok(mut trace) = trace.lock()
            {
                let t_ms = trace.origin.elapsed().as_secs_f64() * 1000.;
                trace.bounds.insert(id, sample);
                trace.paint_t_ms.insert(id, t_ms);
                if !ending && enter_progress == Some(1.) {
                    trace.entered.insert(id);
                }
                let value = json!({
                    "type":"paint", "id":id, "x":sample.x, "y":sample.y,
                    "width":sample.width, "height":sample.height,
                    "enter_progress":enter_progress, "exit_progress":exit_progress,
                    "ending":ending, "effective_reduced":reduced,
                    "os":std::env::consts::OS, "arch":std::env::consts::ARCH,
                    "active":window.is_window_active(), "dpr":window.scale_factor(),
                    "viewport_width":f32::from(window.viewport_size().width),
                    "viewport_height":f32::from(window.viewport_size().height),
                    "t_ms":t_ms
                });
                if serde_json::to_writer(&mut trace.file, &value).is_ok() {
                    let _ = trace.file.write_all(b"\n");
                }
            }
        },
    )
    .absolute()
    .inset_0()
}

/// Record the library's completed Presence state separately from card paint.
/// This has no geometry and does not claim the compositor presented a frame.
pub(super) fn unmounted(id: u64, ending: bool, reduced: bool, window: &Window) {
    let Some(trace) = trace() else { return };
    let Ok(mut trace) = trace.lock() else { return };
    let t_ms = trace.origin.elapsed().as_secs_f64() * 1000.;
    let value = json!({
        "type":"unmounted", "id":id, "observation":"Presence::should_render=false",
        "phase":"absent", "progress":0.0,
        "ending":ending, "effective_reduced":reduced,
        "last_paint_t_ms":trace.paint_t_ms.get(&id).copied(),
        "os":std::env::consts::OS, "arch":std::env::consts::ARCH,
        "active":window.is_window_active(), "dpr":window.scale_factor(),
        "t_ms":t_ms
    });
    if serde_json::to_writer(&mut trace.file, &value).is_ok() {
        let _ = trace.file.write_all(b"\n");
    }
}

/// Record a manager-removal observation for previously painted toasts.
/// This is render state only; it does not claim Presence Absent or display presentation.
pub(super) fn observe_managed_ids(managed_ids: &[u64], reduced: bool, window: &Window) {
    let Some(trace) = trace() else { return };
    let Ok(mut trace) = trace.lock() else { return };
    let managed_ids = managed_ids.iter().copied().collect::<HashSet<_>>();
    let removed_ids = trace
        .bounds
        .keys()
        .filter(|id| !managed_ids.contains(id))
        .copied()
        .collect::<Vec<_>>();

    for id in removed_ids {
        let last_paint_t_ms = trace.paint_t_ms.remove(&id);
        trace.bounds.remove(&id);
        trace.entered.remove(&id);
        let value = json!({
            "type":"removed", "id":id,
            "observation":"ToastLayer render without managed toast",
            "effective_reduced":reduced, "last_paint_t_ms":last_paint_t_ms,
            "os":std::env::consts::OS, "arch":std::env::consts::ARCH,
            "active":window.is_window_active(), "dpr":window.scale_factor(),
            "t_ms":trace.origin.elapsed().as_secs_f64() * 1000.
        });
        if serde_json::to_writer(&mut trace.file, &value).is_ok() {
            let _ = trace.file.write_all(b"\n");
        }
    }
}

fn latest_bounds(id: u64) -> Option<PaintBounds> {
    trace()?.lock().ok()?.bounds.get(&id).copied()
}

fn entered(id: u64) -> bool {
    trace().is_some_and(|trace| trace.lock().is_ok_and(|trace| trace.entered.contains(&id)))
}

fn record_action(action: &str, id: u64, scheduled_ms: u64, window: &Window) {
    record(
        json!({"type":"action","action":action,"id":id,"scheduled_ms":scheduled_ms,
        "os":std::env::consts::OS,"arch":std::env::consts::ARCH,
        "active":window.is_window_active(),"dpr":window.scale_factor()}),
    );
}

pub(crate) fn start(
    window: &mut Window,
    cx: &mut Context<'_, crate::Block>,
    scenario: &'static str,
    requested_reduced: bool,
) {
    if !enabled() {
        return;
    }
    record(json!({
        "type":"meta", "schema":2,
        "clock":"monotonic native render/paint observation; not display presentation",
        "os":std::env::consts::OS, "arch":std::env::consts::ARCH,
        "scenario":scenario, "app_requested_reduced":requested_reduced,
        "system_reduced":crate::platform::system_reduce_motion(),
        "effective_reduced":cx.reduce_motion(),
        "active":window.is_window_active(), "dpr":window.scale_factor(),
    }));

    window.spawn(cx, async move |cx| {
        loop {
            match cx.update(|window, _| window.is_window_active()) {
                Ok(true) => break,
                Ok(false) => cx.background_executor().timer(Duration::from_millis(50)).await,
                Err(_) => return,
            }
        }
        cx.background_executor().timer(Duration::from_millis(500)).await;
        let first = Toast::new("Toast motion replay: enter and dismiss".into(), ToastKind::Info);
        let first_id = first.id;
        if cx.update(|window, cx| {
            record_action("enqueue-enter", first_id, 0, window);
            ToastLayer::enqueue(vec![first], window, cx);
        }).is_err() { return; }

        cx.background_executor().timer(Duration::from_millis(700)).await;
        // A slow native host may not paint the settled endpoint within the
        // nominal hold. Wait for its real card probe, with a bounded timeout.
        for _ in 0..40 {
            if entered(first_id) { break; }
            cx.background_executor().timer(Duration::from_millis(50)).await;
        }
        let at = 700;
        if cx.update(|window, cx| {
            record_action("dismiss", first_id, at, window);
            let Some(layer) = window.root::<gpui_base::Root>().flatten()
                .and_then(|root| root.read(cx).plugin::<ToastLayer>()) else { return };
            layer.update(cx, |layer, cx| layer.dismiss(first_id, cx));
        }).is_err() { return; }

        cx.background_executor().timer(Duration::from_millis(700)).await;
        let second = Toast::new("Toast motion replay: swipe dismiss".into(), ToastKind::Info);
        let second_id = second.id;
        if cx.update(|window, cx| {
            record_action("enqueue-swipe", second_id, 1400, window);
            ToastLayer::enqueue(vec![second], window, cx);
        }).is_err() { return; }

        cx.background_executor().timer(Duration::from_millis(700)).await;
        for _ in 0..40 {
            if entered(second_id) { break; }
            cx.background_executor().timer(Duration::from_millis(50)).await;
        }
        let mut swiped = false;
        for attempt in 0..20 {
            let Some(bounds) = latest_bounds(second_id) else {
                cx.background_executor().timer(Duration::from_millis(50)).await;
                continue;
            };
            let gpui_bounds = Bounds {
                origin: point(px(bounds.x), px(bounds.y)),
                size: size(px(bounds.width), px(bounds.height)),
            };
            if cx.update(|window, cx| {
                let dismissed = std::cell::Cell::new(false);
                let Some(layer) = window.root::<gpui_base::Root>().flatten()
                    .and_then(|root| root.read(cx).plugin::<ToastLayer>()) else { return false };
                layer.update(cx, |layer, cx| {
                    record_action("programmatic-swipe-release", second_id, 2100 + attempt * 50, window);
                    dismissed.set(layer.trace_swipe(second_id, gpui_bounds, cx));
                });
                dismissed.get()
            }).unwrap_or(false) {
                swiped = true;
            }
            break;
        }
        if !swiped {
            let _ = cx.update(|window, _| record(json!({"type":"action","action":"swipe-skipped","id":second_id,
                "reason":"no real paint bounds or swipe dismissal was available","os":std::env::consts::OS,
                "arch":std::env::consts::ARCH,"active":window.is_window_active(),"dpr":window.scale_factor()})));
        }
        cx.background_executor().timer(Duration::from_millis(800)).await;
        let _ = cx.update(|window, _| record(json!({"type":"complete","swipe_triggered":swiped,
            "os":std::env::consts::OS,"arch":std::env::consts::ARCH,
            "active":window.is_window_active(),"dpr":window.scale_factor()})));
    }).detach();
}
