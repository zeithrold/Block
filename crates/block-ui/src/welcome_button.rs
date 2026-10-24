//! Circular Play control used by first-run Welcome. The solid glyph follows
//! the bundled Lucide `play` path; the hover reveal inverts it as one filled
//! polygon.

#![expect(
    clippy::redundant_pub_crate,
    reason = "The welcome control is shared only by private product views."
)]

use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};

use gpui::{
    App, Bounds, Context, InteractiveElement as _, IntoElement, ParentElement as _, Point,
    StatefulInteractiveElement as _, Styled as _, Window, div, point, px, rgb,
};
use gpui_base::motion::{Easing, Interpolate as _, Timing};

use crate::{appearance::Palette, motion::Preferences};

pub(super) const DIAMETER: f32 = 160.;
const ICON_SIZE: f32 = 48.;
/// Extra circle diameter at full hover, as a fraction of the resting size.
const HOVER_GROWTH: f32 = 0.08;
/// Shadow opacity at full hover.
const HOVER_SHADOW_ALPHA: f32 = 0.22;
const DURATION: Duration = Duration::from_millis(180);

/// Number of line segments used to sample each rounded corner of the Lucide
/// `play` outline. Ten segments over the widest (about 120°) corner keep the
/// chord error far below a display pixel at the rendered icon size.
const CORNER_SEGMENTS: usize = 10;

#[derive(Clone, Copy, Debug)]
struct Run {
    from: f32,
    started: Instant,
}

#[derive(Debug, Default)]
struct Animation {
    hovered: bool,
    origin: Point<f32>,
    last_pointer: Option<Point<f32>>,
    initial_masks: Vec<Vec<Point<f32>>>,
    exit_radius: f32,
    run: Option<Run>,
}

struct Visual {
    fraction: f32,
    masks: Vec<Vec<Point<f32>>>,
    running: bool,
}

/// Persistent state for one welcome button. Its root hit area stays fixed
/// while the inner circle scales visually on hover.
#[derive(Clone, Debug, Default)]
pub(super) struct WelcomeButton {
    animation: Rc<RefCell<Animation>>,
    bounds: Rc<RefCell<Option<Bounds<gpui::Pixels>>>>,
}

impl WelcomeButton {
    #[cfg(test)]
    pub(super) fn is_hovered(&self) -> bool {
        self.animation.borrow().hovered
    }

    fn pointer_moved(
        &self,
        position: Point<gpui::Pixels>,
        force_exit: bool,
        now: Instant,
        preferences: Preferences,
    ) -> bool {
        let Some(bounds) = *self.bounds.borrow() else {
            return false;
        };
        let local = point(
            (position.x - bounds.origin.x) / px(1.),
            (position.y - bounds.origin.y) / px(1.),
        );
        let mut animation = self.animation.borrow_mut();
        let previous = animation.last_pointer.replace(local);
        let radius = DIAMETER * 0.5;
        let hovered = !force_exit && (local.x - radius).hypot(local.y - radius) <= radius;
        if animation.hovered == hovered {
            return false;
        }
        let visual = sample_visual(&mut animation, now, preferences);
        animation.origin = previous
            .and_then(|previous| circle_crossing(previous, local, hovered))
            .unwrap_or_else(|| entry_point_on_circle(local));
        animation.hovered = hovered;
        animation.initial_masks = visual.masks;
        let origin = animation.origin - point(radius, radius);
        animation.exit_radius = animation
            .initial_masks
            .iter()
            .flatten()
            .map(|p| (p.x - origin.x).hypot(p.y - origin.y))
            .fold(0., f32::max);
        animation.run = (!preferences.reduced()).then_some(Run {
            from: visual.fraction,
            started: now,
        });
        true
    }

    #[cfg(debug_assertions)]
    pub(super) fn trace_sample(&self, now: Instant) -> serde_json::Value {
        let animation = self.animation.borrow();
        serde_json::json!({
            "hovered": animation.hovered,
            "origin": {"x": animation.origin.x, "y": animation.origin.y},
            "progress": animation.run.map(|run| progress(run, now).0),
            "initial_masks": animation.initial_masks.len(),
            "bounds": self.bounds.borrow().map(|bounds| serde_json::json!({
                "x": bounds.origin.x / px(1.), "y": bounds.origin.y / px(1.),
                "width": bounds.size.width / px(1.), "height": bounds.size.height / px(1.),
            })),
        })
    }
}

fn circle_crossing(from: Point<f32>, to: Point<f32>, entering: bool) -> Option<Point<f32>> {
    let radius = DIAMETER * 0.5;
    let start = from - point(radius, radius);
    let direction = to - from;
    let a = direction.x * direction.x + direction.y * direction.y;
    let b = 2. * (start.x * direction.x + start.y * direction.y);
    let c = start.x * start.x + start.y * start.y - radius * radius;
    let discriminant = b * b - 4. * a * c;
    if a <= f32::EPSILON || discriminant < 0. {
        return None;
    }
    let root = discriminant.sqrt();
    let times = [(-b - root) / (2. * a), (-b + root) / (2. * a)];
    let times = if entering {
        times
    } else {
        [times[1], times[0]]
    };
    for t in times {
        if (0. ..=1.).contains(&t) {
            return Some(from + direction * t);
        }
    }
    None
}

fn sample_visual(animation: &mut Animation, now: Instant, preferences: Preferences) -> Visual {
    let phase = animation.run.map_or(1., |run| progress(run, now).0);
    let fraction = sample(animation, now, preferences);
    let radius = DIAMETER * 0.5 * (1. + fraction * HOVER_GROWTH);
    let zero = point(0., 0.);
    let running = animation.run.is_some();
    let masks = if !running {
        animation.initial_masks.clear();
        if animation.hovered {
            vec![reveal_outline(zero, radius, zero, radius)]
        } else {
            Vec::new()
        }
    } else if animation.hovered {
        // Preserve the visible masks when an exit reverses. A new reveal adds
        // only foreground pixels, so changing its entry origin cannot teleport
        // an existing partial fill.
        let mut masks = animation.initial_masks.clone();
        let origin = animation.origin - point(DIAMETER * 0.5, DIAMETER * 0.5);
        let reveal = reveal_outline(
            zero,
            radius,
            origin,
            (radius + origin.x.hypot(origin.y)) * phase,
        );
        if reveal.len() >= 3 {
            masks.push(reveal);
        }
        masks
    } else {
        // Start with precisely the currently painted fill, then shrink its
        // intersection with a disc centered on the pointer's exit crossing.
        let origin = animation.origin - point(DIAMETER * 0.5, DIAMETER * 0.5);
        let clip = reveal_outline(zero, radius, origin, animation.exit_radius * (1. - phase));
        animation
            .initial_masks
            .iter()
            .map(|mask| clip_to_convex(mask.clone(), &clip))
            .filter(|mask| mask.len() >= 3)
            .collect()
    };
    Visual {
        fraction,
        masks,
        running,
    }
}

fn progress(run: Run, now: Instant) -> (f32, bool) {
    let progress = Timing::new(DURATION)
        .ease(Easing::CubicBezier {
            x1: 0.22,
            y1: 1.,
            x2: 0.36,
            y2: 1.,
        })
        .sample(now.saturating_duration_since(run.started));
    (progress.directed_progress, progress.finished)
}

fn entry_point_on_circle(position: Point<f32>) -> Point<f32> {
    let radius = DIAMETER * 0.5;
    let dx = position.x - radius;
    let dy = position.y - radius;
    let distance = dx.hypot(dy);
    if distance < f32::EPSILON {
        return point(0., radius);
    }
    point(
        radius + dx * radius / distance,
        radius + dy * radius / distance,
    )
}

fn sample(animation: &mut Animation, now: Instant, preferences: Preferences) -> f32 {
    let target = if animation.hovered { 1. } else { 0. };
    let Some(run) = animation.run else {
        return target;
    };
    if preferences.reduced() {
        animation.run = None;
        return target;
    }
    let (progress, finished) = progress(run, now);
    if finished {
        animation.run = None;
        target
    } else {
        run.from.interpolate(&target, progress)
    }
}

/// Render the native control, preserving the standard button role and activation.
pub(super) fn render<T: 'static>(
    state: &WelcomeButton,
    palette: Palette,
    preferences: Preferences,
    window: &Window,
    cx: &Context<'_, T>,
    on_activate: impl Fn(&(), &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let now = cx.background_executor().now();
    let visual = {
        let mut animation = state.animation.borrow_mut();
        sample_visual(&mut animation, now, preferences)
    };
    if visual.running {
        window.request_animation_frame();
    }
    let fraction = visual.fraction;

    let entity_id = cx.entity().entity_id();
    let hover_state = state.clone();
    let move_state = state.clone();
    let root_bounds = state.bounds.clone();
    let visual_diameter = DIAMETER * (1. + fraction * HOVER_GROWTH);
    let inner = div()
        .id("welcome-play-visual")
        .relative()
        .w(px(visual_diameter))
        .h(px(visual_diameter))
        .flex_shrink_0()
        .rounded_full()
        .overflow_hidden()
        .border_1()
        .border_color(rgb(palette.border))
        .bg(rgb(palette.background))
        .shadow(vec![gpui::BoxShadow {
            inset: false,
            color: {
                let mut color: gpui::Hsla = rgb(palette.foreground).into();
                color.a = HOVER_SHADOW_ALPHA * fraction;
                color
            },
            offset: point(px(0.), px(3.)),
            blur_radius: px(10.) * fraction,
            spread_radius: px(0.),
        }])
        .child(button_face(fraction, visual.masks, palette));

    let capture_root_bounds = root_bounds.clone();
    let root_bounds_probe = gpui::canvas(
        move |layout_bounds, _, _| {
            *capture_root_bounds.borrow_mut() = Some(layout_bounds);
        },
        |_, _, _, _| {},
    )
    .absolute()
    .inset_0();

    crate::base_controls::Control::button(
        "welcome-play".into(),
        "Start setup".into(),
        gpui::Role::Button,
        false,
        on_activate,
    )
    .aria_label("Start setup")
    .debug_selector(|| "welcome-play".into())
    .w(px(DIAMETER))
    .h(px(DIAMETER))
    .p_0()
    .bg(rgb(palette.background))
    .relative()
    .flex()
    .items_center()
    .justify_center()
    .child(inner)
    .child(root_bounds_probe)
    .on_mouse_move(move |event, _, cx| {
        if move_state.pointer_moved(
            event.position,
            false,
            cx.background_executor().now(),
            preferences,
        ) {
            cx.notify(entity_id);
        }
    })
    .on_hover(move |hovered, window, cx| {
        if hover_state.pointer_moved(
            window.mouse_position(),
            !*hovered,
            cx.background_executor().now(),
            preferences,
        ) {
            cx.notify(entity_id);
        }
    })
}

fn button_face(fraction: f32, masks: Vec<Vec<Point<f32>>>, palette: Palette) -> impl IntoElement {
    gpui::canvas(
        |_, _, _| (),
        move |layout_bounds, _, window, _| {
            let icon_center = point(
                layout_bounds.center().x / px(1.),
                layout_bounds.center().y / px(1.),
            );
            let glyph = play_glyph(icon_center, ICON_SIZE / 24.);
            paint_polygon(&glyph, palette.foreground, window);
            let model_radius = DIAMETER * 0.5 * (1. + fraction * HOVER_GROWTH);
            let actual_radius = layout_bounds.size.width.min(layout_bounds.size.height) / px(2.);
            let masks = masks
                .iter()
                .map(|mask| {
                    mask.iter()
                        .map(|p| icon_center + *p * (actual_radius / model_radius))
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            // Paint the union before its inverted glyph, so overlapping masks
            // cannot paint foreground back over an already inverted icon.
            for mask in &masks {
                paint_polygon(mask, palette.foreground, window);
            }
            for mask in masks {
                paint_polygon(&clip_to_convex(mask, &glyph), palette.background, window);
            }
        },
    )
    .absolute()
    .inset_0()
    .size_full()
}

/// The solid Lucide `play` glyph sampled into a convex polygon. The outline is
/// the bundled `assets/icons/play.svg` path in its 24 × 24 viewBox: a triangle
/// with 2-unit rounded corners, right-pointing tip and a right optical shift.
/// A polygon keeps the inverted reveal identical to the painted glyph.
fn play_glyph(center: Point<f32>, scale: f32) -> Vec<Point<f32>> {
    let mut outline = vec![point(5., 5.)];
    append_corner(&mut outline, point(7., 5.), point(8.008, 3.272));
    outline.push(point(20.005, 10.27));
    append_corner(&mut outline, point(19., 12.), point(20.008, 13.728));
    outline.push(point(8.008, 20.728));
    append_corner(&mut outline, point(7., 19.), point(5., 19.));
    outline
        .into_iter()
        .map(|p| center + point((p.x - 12.) * scale, (p.y - 12.) * scale))
        .collect()
}

/// Append one 2-unit rounded corner of the Lucide outline, ending at the
/// corner's `to` point. The SVG arc sweeps clockwise from the previous point.
fn append_corner(outline: &mut Vec<Point<f32>>, center: Point<f32>, to: Point<f32>) {
    let Some(&from) = outline.last() else {
        return;
    };
    let radius = 2.;
    let start_angle = (from.y - center.y).atan2(from.x - center.x);
    let mut sweep = (to.y - center.y).atan2(to.x - center.x) - start_angle;
    while sweep < 0. {
        sweep += std::f32::consts::TAU;
    }
    for index in 1..=CORNER_SEGMENTS {
        let angle = start_angle + sweep * index as f32 / CORNER_SEGMENTS as f32;
        outline.push(point(
            center.x + radius * angle.cos(),
            center.y + radius * angle.sin(),
        ));
    }
}

fn reveal_outline(
    button: Point<f32>,
    button_radius: f32,
    reveal: Point<f32>,
    reveal_radius: f32,
) -> Vec<Point<f32>> {
    fn arc(center: Point<f32>, radius: f64, start: f64, end: f64) -> Vec<Point<f32>> {
        let segments = (((end - start) / std::f64::consts::TAU * 96.).ceil() as usize).max(1);
        let full_circle = (end - start - std::f64::consts::TAU).abs() < 1e-10;
        (0..if full_circle { segments } else { segments + 1 })
            .map(|index| {
                let angle = start + (end - start) * index as f64 / segments as f64;
                center + point((radius * angle.cos()) as f32, (radius * angle.sin()) as f32)
            })
            .collect()
    }
    if button_radius <= 0. || reveal_radius <= 0. {
        return Vec::new();
    }
    let button_radius = f64::from(button_radius);
    let reveal_radius = f64::from(reveal_radius);
    let dx = f64::from(reveal.x) - f64::from(button.x);
    let dy = f64::from(reveal.y) - f64::from(button.y);
    let distance = dx.hypot(dy);
    if distance + button_radius <= reveal_radius {
        return arc(button, button_radius, 0., std::f64::consts::TAU);
    }
    if distance + reveal_radius <= button_radius {
        return arc(reveal, reveal_radius, 0., std::f64::consts::TAU);
    }
    if distance >= button_radius + reveal_radius {
        return Vec::new();
    }
    let direction = dy.atan2(dx);
    let button_angle = ((distance * distance + button_radius * button_radius
        - reveal_radius * reveal_radius)
        / (2. * distance * button_radius))
        .clamp(-1., 1.)
        .acos();
    let reveal_angle = ((distance * distance + reveal_radius * reveal_radius
        - button_radius * button_radius)
        / (2. * distance * reveal_radius))
        .clamp(-1., 1.)
        .acos();
    let mut outline = arc(
        button,
        button_radius,
        direction - button_angle,
        direction + button_angle,
    );
    let other = arc(
        reveal,
        reveal_radius,
        direction + std::f64::consts::PI - reveal_angle,
        direction + std::f64::consts::PI + reveal_angle,
    );
    // Both arcs meet at the same two intersections. Retain each endpoint only
    // once; recomputing an almost-identical endpoint creates a spurious edge.
    if other.len() > 2 {
        outline.extend_from_slice(&other[1..other.len() - 1]);
    }
    outline
}

fn paint_polygon(polygon: &[Point<f32>], color: u32, window: &mut Window) {
    if polygon.len() < 3 {
        return;
    }
    let mut path = gpui::PathBuilder::fill();
    for (index, p) in polygon.iter().enumerate() {
        let p = point(px(p.x), px(p.y));
        if index == 0 {
            path.move_to(p);
        } else {
            path.line_to(p);
        }
    }
    path.close();
    if let Ok(path) = path.build() {
        window.paint_path(path, rgb(color));
    }
}

fn clip_to_convex(mut polygon: Vec<Point<f32>>, clip: &[Point<f32>]) -> Vec<Point<f32>> {
    if clip.len() < 3 {
        return Vec::new();
    }
    let cross = |a: Point<f32>, b: Point<f32>| a.x * b.y - a.y * b.x;
    let reference = clip[0];
    let area = (0..clip.len())
        .map(|i| {
            let a = clip[i];
            let b = clip[(i + 1) % clip.len()];
            (f64::from(a.x) - f64::from(reference.x)) * (f64::from(b.y) - f64::from(reference.y))
                - (f64::from(a.y) - f64::from(reference.y))
                    * (f64::from(b.x) - f64::from(reference.x))
        })
        .sum::<f64>();
    // Near the endpoint a disc can quantize into a line. Its area must be
    // measured relative to a local origin to avoid cancellation at x ≈ 80.
    if area.abs() < 0.0001 {
        return Vec::new();
    }
    let winding = area.signum() as f32;
    let mut applied_edge = false;
    for edge in 0..clip.len() {
        let a = clip[edge];
        let b = clip[(edge + 1) % clip.len()];
        let length = (b.x - a.x).hypot(b.y - a.y);
        // Circle arcs share endpoints. Their f32 copies can differ by a few
        // ulps; treating that tiny closing segment as a half-plane can erase
        // an entire otherwise valid mask for one frame.
        if length < 0.0001 {
            continue;
        }
        applied_edge = true;
        let side = |p: Point<f32>| cross(b - a, p - a) * winding / length;
        let Some(mut previous) = polygon.last().copied() else {
            break;
        };
        let mut previous_side = side(previous);
        for current in std::mem::take(&mut polygon) {
            let current_side = side(current);
            if (current_side >= -0.0001) != (previous_side >= -0.0001) {
                let t = (previous_side / (previous_side - current_side)).clamp(0., 1.);
                polygon.push(previous + (current - previous) * t);
            }
            if current_side >= -0.0001 {
                polygon.push(current);
            }
            previous = current;
            previous_side = current_side;
        }
    }
    if applied_edge { polygon } else { Vec::new() }
}

#[cfg(test)]
mod tests {
    use super::{
        Animation, DIAMETER, Preferences, Run, WelcomeButton, circle_crossing, clip_to_convex,
        entry_point_on_circle, play_glyph, reveal_outline, sample, sample_visual,
    };
    use gpui::{Point, point, px};
    use std::time::{Duration, Instant};

    fn inside_convex(target: Point<f32>, polygon: &[Point<f32>]) -> bool {
        let signed_area = (0..polygon.len())
            .map(|index| {
                let a = polygon[index];
                let b = polygon[(index + 1) % polygon.len()];
                a.x * b.y - a.y * b.x
            })
            .sum::<f32>();
        let winding = signed_area.signum();
        (0..polygon.len()).all(|index| {
            let a = polygon[index];
            let b = polygon[(index + 1) % polygon.len()];
            let side = (b.x - a.x) * (target.y - a.y) - (b.y - a.y) * (target.x - a.x);
            side * winding >= -0.01
        })
    }

    #[test]
    fn reveal_fill_stays_inside_both_circles_and_covers_the_button_when_finished() {
        let button = point(40., 40.);
        for origin in [point(0., 40.), point(40., 0.), point(80., 40.)] {
            for radius in [1., 10., 40., 80., 120.] {
                let outline = reveal_outline(button, 40., origin, radius);
                assert!(outline.len() >= 3);
                for p in outline {
                    assert!((p.x - button.x).hypot(p.y - button.y) <= 40.01);
                    assert!((p.x - origin.x).hypot(p.y - origin.y) <= radius + 0.01);
                }
            }
        }
        let full = reveal_outline(button, 40., point(0., 40.), 120.);
        let area = (0..full.len())
            .map(|i| {
                let a = full[i];
                let b = full[(i + 1) % full.len()];
                a.x * b.y - a.y * b.x
            })
            .sum::<f32>()
            .abs()
            * 0.5;
        assert!((area / (std::f32::consts::PI * 40. * 40.) - 1.).abs() < 0.01);
        assert!(reveal_outline(button, 40., point(200., 40.), 10.).is_empty());
    }

    #[test]
    fn reentry_during_reverse_preserves_the_visible_fill_when_the_origin_changes() {
        let state = WelcomeButton::default();
        let radius = DIAMETER * 0.5;
        *state.bounds.borrow_mut() = Some(gpui::Bounds {
            origin: point(px(0.), px(0.)),
            size: gpui::size(px(DIAMETER), px(DIAMETER)),
        });
        let start = Instant::now();
        let preferences = Preferences::default();
        state.pointer_moved(point(px(-12.), px(radius)), false, start, preferences);
        state.pointer_moved(point(px(4.), px(radius)), false, start, preferences);
        state.pointer_moved(
            point(px(DIAMETER + 12.), px(radius)),
            false,
            start + Duration::from_millis(300),
            preferences,
        );
        let now = start + Duration::from_millis(330);
        let before = sample_visual(&mut state.animation.borrow_mut(), now, preferences);
        let old_origin = state.animation.borrow().origin;
        state.pointer_moved(point(px(radius), px(4.)), false, now, preferences);
        let after = sample_visual(&mut state.animation.borrow_mut(), now, preferences);
        assert_ne!(state.animation.borrow().origin, old_origin);
        assert_eq!(before.fraction, after.fraction);
        assert_eq!(before.masks, after.masks);
        assert!(state.animation.borrow().hovered);
        let settled = sample_visual(
            &mut state.animation.borrow_mut(),
            start + Duration::from_millis(600),
            preferences,
        );
        assert!(!settled.running);
        assert!(state.animation.borrow().initial_masks.is_empty());
    }

    #[test]
    fn exit_retargets_the_shrinking_disc_to_the_opposite_leave_point() {
        let state = WelcomeButton::default();
        let r = DIAMETER * 0.5;
        *state.bounds.borrow_mut() = Some(gpui::Bounds {
            origin: point(px(0.), px(0.)),
            size: gpui::size(px(DIAMETER), px(DIAMETER)),
        });
        let start = Instant::now();
        let preferences = Preferences::default();
        state.pointer_moved(point(px(-12.), px(r)), false, start, preferences);
        state.pointer_moved(point(px(4.), px(r)), false, start, preferences);
        state.pointer_moved(
            point(px(r), px(r)),
            false,
            start + Duration::from_millis(250),
            preferences,
        );
        let leaving = start + Duration::from_millis(300);
        state.pointer_moved(point(px(DIAMETER + 12.), px(r)), true, leaving, preferences);
        assert_eq!(state.animation.borrow().origin, point(DIAMETER, r));
        assert!(!state.animation.borrow().hovered);
        let radius = state.animation.borrow().exit_radius;
        let now = leaving + Duration::from_millis(60);
        let visual = sample_visual(&mut state.animation.borrow_mut(), now, preferences);
        assert!(!visual.masks.is_empty());
        let shrink = state
            .animation
            .borrow()
            .run
            .map_or(-1., |run| radius * (1. - super::progress(run, now).0));
        assert!(shrink >= 0.);
        assert!(
            visual
                .masks
                .iter()
                .flatten()
                .all(|p| (p.x - r).hypot(p.y) <= shrink + 0.05)
        );
    }

    #[test]
    fn off_axis_crossing_uses_the_pointer_segment_instead_of_a_center_ray() {
        let r = DIAMETER * 0.5;
        let crossing = circle_crossing(point(r, r + 60.), point(DIAMETER + 20., r + 60.), false);
        assert!(
            crossing.is_some_and(|p| (p.x - (r + (r * r - 3600.).sqrt())).abs() < 0.01
                && (p.y - r - 60.).abs() < 0.01)
        );
    }

    #[test]
    fn exit_fill_area_shrinks_monotonically_without_blank_flashes() {
        let state = WelcomeButton::default();
        let r = DIAMETER * 0.5;
        *state.bounds.borrow_mut() = Some(gpui::Bounds {
            origin: point(px(0.), px(0.)),
            size: gpui::size(px(DIAMETER), px(DIAMETER)),
        });
        let start = Instant::now();
        let preferences = Preferences::default();
        state.pointer_moved(point(px(4.), px(r)), false, start, preferences);
        let leaving = start + Duration::from_millis(300);
        state.pointer_moved(point(px(DIAMETER + 12.), px(r)), true, leaving, preferences);
        let mut previous = f32::MAX;
        for ms in (0..=180).step_by(5) {
            let frame = sample_visual(
                &mut state.animation.borrow_mut(),
                leaving + Duration::from_millis(ms),
                preferences,
            );
            let area = frame
                .masks
                .iter()
                .map(|mask| {
                    (0..mask.len())
                        .map(|i| {
                            let a = mask[i];
                            let b = mask[(i + 1) % mask.len()];
                            a.x * b.y - a.y * b.x
                        })
                        .sum::<f32>()
                        .abs()
                        * 0.5
                })
                .sum::<f32>();
            assert!(
                area <= previous + 1.,
                "fill grew from {previous} to {area} at {ms}ms"
            );
            if ms <= 90 {
                assert!(area > 0., "fill vanished at {ms}ms");
            }
            previous = area;
        }
        assert_eq!(previous, 0.);
    }

    #[test]
    fn leaving_during_entry_keeps_the_current_fill_instead_of_teleporting_it() {
        let state = WelcomeButton::default();
        let r = DIAMETER * 0.5;
        *state.bounds.borrow_mut() = Some(gpui::Bounds {
            origin: point(px(0.), px(0.)),
            size: gpui::size(px(DIAMETER), px(DIAMETER)),
        });
        let start = Instant::now();
        let preferences = Preferences::default();
        state.pointer_moved(point(px(4.), px(r)), false, start, preferences);
        let now = start + Duration::from_millis(40);
        let before = sample_visual(&mut state.animation.borrow_mut(), now, preferences);
        state.pointer_moved(point(px(DIAMETER + 12.), px(r)), true, now, preferences);
        let after = sample_visual(&mut state.animation.borrow_mut(), now, preferences);
        let area = |masks: &[Vec<Point<f32>>]| {
            masks
                .iter()
                .map(|mask| {
                    (0..mask.len())
                        .map(|i| {
                            let a = mask[i];
                            let b = mask[(i + 1) % mask.len()];
                            a.x * b.y - a.y * b.x
                        })
                        .sum::<f32>()
                        .abs()
                        * 0.5
                })
                .sum::<f32>()
        };
        assert_eq!(before.fraction, after.fraction);
        assert!((area(&before.masks) - area(&after.masks)).abs() < area(&before.masks) * 0.003);
    }

    #[test]
    fn reveal_origins_are_on_the_original_circle_even_for_coarse_pointer_samples() {
        let radius = DIAMETER * 0.5;
        for position in [
            point(radius, 10.),
            point(1., 1.),
            point(DIAMETER - 10., radius),
            point(radius, radius),
        ] {
            let origin = entry_point_on_circle(position);
            assert!(((origin.x - radius).hypot(origin.y - radius) - radius).abs() < 0.001);
        }
        assert_eq!(entry_point_on_circle(point(radius, 10.)), point(radius, 0.));
    }

    #[test]
    fn hover_reversal_starts_from_the_current_fraction() {
        let start = Instant::now();
        let preferences = Preferences::default();
        let mut state = Animation {
            hovered: true,
            origin: point(10., 10.),
            run: Some(Run {
                from: 0.,
                started: start,
            }),
            ..Animation::default()
        };
        let halfway = sample(&mut state, start + Duration::from_millis(90), preferences);
        assert!(halfway > 0.5 && halfway < 1.);
        state.hovered = false;
        state.run = Some(Run {
            from: halfway,
            started: start + Duration::from_millis(90),
        });
        assert_eq!(
            sample(&mut state, start + Duration::from_millis(90), preferences),
            halfway
        );
        assert!(sample(&mut state, start + Duration::from_millis(270), preferences) < 0.01);
    }

    #[test]
    fn reduced_motion_settles_immediately() {
        let start = Instant::now();
        let preferences = Preferences {
            app_reduced: true,
            system_reduced: false,
        };
        let mut state = Animation {
            hovered: true,
            origin: point(0., 0.),
            run: Some(Run {
                from: 0.4,
                started: start,
            }),
            ..Animation::default()
        };
        assert_eq!(sample(&mut state, start, preferences), 1.);
        assert!(state.run.is_none());
    }

    #[test]
    fn solid_lucide_play_glyph_matches_the_bundled_icon_box() {
        let glyph = play_glyph(point(0., 0.), 2.);
        assert!(glyph.len() >= 30);
        let min_x = glyph.iter().map(|p| p.x).fold(f32::MAX, f32::min);
        let max_x = glyph.iter().map(|p| p.x).fold(f32::MIN, f32::max);
        let min_y = glyph.iter().map(|p| p.y).fold(f32::MAX, f32::min);
        let max_y = glyph.iter().map(|p| p.y).fold(f32::MIN, f32::max);
        // The 24-unit Lucide outline spans x 5..21 and y 3..21: at 48 px the
        // solid glyph is 32 px wide and 36 px tall, optically right of centre.
        assert!((min_x + 14.).abs() < 0.02, "left edge was {min_x}");
        assert!((max_x - 18.).abs() < 0.02, "tip ended at {max_x}");
        assert!((min_y + 18.).abs() < 0.1, "top edge was {min_y}");
        assert!((max_y - 18.).abs() < 0.1, "bottom edge was {max_y}");
        // The reveal clips the glyph as one convex polygon.
        let signed_area = (0..glyph.len())
            .map(|index| {
                let a = glyph[index];
                let b = glyph[(index + 1) % glyph.len()];
                a.x * b.y - a.y * b.x
            })
            .sum::<f32>();
        let winding = signed_area.signum();
        for index in 0..glyph.len() {
            let a = glyph[index];
            let b = glyph[(index + 1) % glyph.len()];
            let c = glyph[(index + 2) % glyph.len()];
            let turn = ((b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)) * winding;
            assert!(turn >= -0.01, "glyph is not convex near vertex {index}");
        }
    }

    #[test]
    fn reveal_clips_the_solid_play_glyph_to_the_circular_front() {
        let glyph = play_glyph(point(0., 0.), 2.);
        let mask = reveal_outline(point(0., 0.), 20., point(-8., 0.), 8.);
        let clipped = clip_to_convex(mask, &glyph);
        assert!(clipped.len() >= 3);
        for p in clipped {
            assert!(inside_convex(p, &glyph), "{p:?} left the solid glyph");
            assert!((p.x + 8.).hypot(p.y) <= 8.01);
        }
    }
}
