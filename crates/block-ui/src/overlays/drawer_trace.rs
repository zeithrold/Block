//! Opt-in, debug-only live drawer paint measurements using disposable scenarios.

use crate::{Block, UiAction, shell::Drawer};
use gpui::{Context, IntoElement, Styled as _, WeakEntity, Window};
use serde::Serialize;
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
            let path = std::env::var_os("BLOCK_DRAWER_MOTION_TRACE")?;
            match File::create(path) {
                Ok(file) => Some(Mutex::new(Trace {
                    origin: Instant::now(),
                    file,
                })),
                Err(error) => {
                    eprintln!("Could not create drawer motion trace: {error}");
                    None
                }
            }
        })
        .as_ref()
}

pub(super) fn enabled() -> bool {
    trace().is_some()
}

fn elapsed_ms() -> f64 {
    trace()
        .and_then(|trace| {
            trace
                .lock()
                .ok()
                .map(|trace| trace.origin.elapsed().as_secs_f64() * 1000.)
        })
        .unwrap_or_default()
}

fn record(mut value: Value) {
    let Some(trace) = trace() else {
        return;
    };
    let Ok(mut trace) = trace.lock() else {
        return;
    };
    value["t_ms"] = json!(trace.origin.elapsed().as_secs_f64() * 1000.);
    if serde_json::to_writer(&mut trace.file, &value).is_ok() {
        let _ = trace.file.write_all(b"\n");
    }
}

#[derive(Clone, Copy, Serialize)]
pub(super) struct FrameSample {
    pub revision: usize,
    pub render_ms: f64,
    pub progress: f32,
    pub translation: f32,
    pub scrim: f32,
    pub height: f32,
    pub offset: f32,
    pub closing: bool,
    pub reduced: bool,
}

pub(super) fn sample(mut sample: FrameSample) -> Option<FrameSample> {
    if !enabled() {
        return None;
    }
    sample.render_ms = elapsed_ms();
    Some(sample)
}

pub(super) fn probe(part: &'static str, sample: FrameSample) -> impl IntoElement {
    gpui::canvas(
        |_, _, _| elapsed_ms(),
        move |bounds, prepaint_ms, window, _| {
            record(json!({
                "type":"paint", "part":part, "sample":sample, "prepaint_ms":prepaint_ms,
                "x":f32::from(bounds.origin.x),"y":f32::from(bounds.origin.y),
                "width":f32::from(bounds.size.width),"height":f32::from(bounds.size.height),
                "viewport_width":f32::from(window.viewport_size().width),
                "viewport_height":f32::from(window.viewport_size().height),
                "active":window.is_window_active(),"dpr":window.scale_factor(),
            }))
        },
    )
    .absolute()
    .inset_0()
}

pub(super) fn start(owner: WeakEntity<Block>, window: &mut Window, cx: &mut Context<'_, Block>) {
    if !enabled() {
        return;
    }
    record(json!({"type":"meta","source":"native","schema":1,
        "clock":"monotonic paint submission; not display presentation",
        "os":std::env::consts::OS,"arch":std::env::consts::ARCH}));
    window.spawn(cx, async move |cx| {
        loop {
            match cx.update(|window, _| window.is_window_active()) {
                Ok(true) => break,
                Ok(false) => cx.background_executor().timer(Duration::from_millis(50)).await,
                Err(_) => return,
            }
        }
        cx.background_executor().timer(Duration::from_millis(1500)).await;
        let start = cx.background_executor().now();
        for (at, open) in [(0,true),(900,false),(1600,true),(2500,false)] {
            let delay = (start + Duration::from_millis(at)).saturating_duration_since(cx.background_executor().now());
            cx.background_executor().timer(delay).await;
            if cx.update(|window, cx| {
                record(json!({"type":"action","action":if open { "open" } else { "close" },"scheduled_ms":at}));
                let _ = owner.update(cx, |view, cx| view.handle(UiAction::Drawer(open.then_some(Drawer::Accounts)), window, cx));
            }).is_err() { return; }
        }
        cx.background_executor().timer(Duration::from_millis(700)).await;
        record(json!({"type":"complete"}));
    }).detach();
}
