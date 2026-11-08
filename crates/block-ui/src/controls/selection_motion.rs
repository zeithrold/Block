//! Selection presentation shares one sample without owning native interaction.

#![expect(
    clippy::redundant_pub_crate,
    reason = "Selection presentation belongs to the private shared controls."
)]

use std::time::Duration;

use gpui::{
    AnyElement, App, Bounds, FillOptions, InteractiveElement as _, IntoElement, ParentElement as _,
    Path, PathBuilder, PathStyle, Pixels, Point, RenderOnce, SharedString, Styled as _, Window,
    canvas, div, point, px, rgb,
};
use gpui_base::motion::{Easing, Transition, transition_with_status};

use super::{appearance::Palette, base_controls::Control};

const INDICATOR_SIZE: f32 = 16.;
const CHECK_SCALE: f32 = 14. / 24.;

/// `control` is the native variant. Caller children are queued separately so
/// the decorator's Indicator and label retain their original flex order.
#[derive(IntoElement)]
pub(super) struct SelectionPresentation {
    pub control: Control,
    pub id: SharedString,
    pub checked: bool,
    pub disabled: bool,
    pub decorate: Box<dyn FnOnce(Control, f32) -> Control>,
    pub children: Vec<AnyElement>,
}

impl RenderOnce for SelectionPresentation {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        // Zero duration settles Base's stored value too, so enabling the
        // control again does not resume an obsolete disabled transition.
        let duration = if self.disabled {
            Duration::ZERO
        } else {
            Duration::from_millis(160)
        };
        let progress = transition_with_status(
            (self.id.clone(), "selection"),
            if self.checked { 1_f32 } else { 0_f32 },
            Transition::new(duration).easing(Easing::CubicBezier {
                x1: 0.2,
                y1: 0.,
                x2: 0.,
                y2: 1.,
            }),
            window,
            cx,
        )
        .value;
        (self.decorate)(self.control, progress).children(self.children)
    }
}

/// A fixed layout box; all selection geometry is painted around its center.
pub(super) fn indicator(
    id: SharedString,
    radio: bool,
    progress: f32,
    palette: Palette,
) -> impl IntoElement {
    let selector = format!("{id}-indicator");
    div()
        .debug_selector(move || selector.clone())
        .size(px(INDICATOR_SIZE))
        .flex_shrink_0()
        .child(
            canvas(
                |_, _, _| (),
                move |bounds, (), window, _| {
                    let center = bounds.center();
                    let selected = rgb(palette.primary).opacity(progress);
                    let border = rgb(palette.input).blend(selected);
                    let background = rgb(palette.input_background).blend(selected);
                    let prefix = if radio { None } else { check_prefix(progress) };
                    if radio {
                        for (radius, color) in [
                            (px(8.), border),
                            (px(7.), background),
                            (px(4. * progress), rgb(palette.on_primary)),
                        ] {
                            if radius > px(0.)
                                && let Ok(path) = circle_path(center, radius)
                            {
                                window.paint_path(path, color);
                            }
                        }
                    } else {
                        window.paint_quad(gpui::quad(
                            bounds,
                            px(4.),
                            background,
                            px(1.),
                            border,
                            Default::default(),
                        ));
                        if let Some(prefix) = prefix
                            && let Ok(path) = check_path(bounds, prefix)
                        {
                            window.paint_path(path, rgb(palette.on_primary));
                        }
                    }
                    #[cfg(any(debug_assertions, test))]
                    paint_trace::record(
                        &id,
                        if radio { "radio" } else { "checkbox" },
                        progress,
                        None,
                        bounds,
                        window,
                        radio.then_some((center, px(4. * progress))),
                        prefix.map(|prefix| check_point(bounds, prefix.tip)),
                    );
                },
            )
            .size_full(),
        )
}

#[derive(Clone, Copy, Debug)]
struct CheckPrefix {
    bend: Option<Point<f32>>,
    tip: Point<f32>,
}

fn check_prefix(progress: f32) -> Option<CheckPrefix> {
    let length = ((progress - 0.15) / 0.85).clamp(0., 1.) * 16.;
    if length == 0. {
        return None;
    }
    // A(4,12) -> B(9,17) -> C(20,6). Both segments are diagonal,
    // so lengths 5 and 11 share the same sqrt(2) arc-length factor.
    Some(if length <= 5. {
        CheckPrefix {
            bend: None,
            tip: point(4. + length, 12. + length),
        }
    } else {
        CheckPrefix {
            bend: Some(point(9., 17.)),
            tip: point(4. + length, 22. - length),
        }
    })
}

fn check_point(bounds: Bounds<Pixels>, source: Point<f32>) -> Point<Pixels> {
    bounds.center()
        + point(
            px((source.x - 12.) * CHECK_SCALE),
            px((source.y - 12.) * CHECK_SCALE),
        )
}

fn fill_path() -> PathBuilder {
    // EvenOdd, the default, would cut holes where the segment rectangles and
    // round caps overlap. Consistent clockwise contours form one filled union.
    PathBuilder::fill().with_style(PathStyle::Fill(
        FillOptions::non_zero().with_tolerance(0.01),
    ))
}

fn append_circle(path: &mut PathBuilder, center: Point<Pixels>, radius: Pixels) {
    let right = center + point(radius, px(0.));
    let left = center - point(radius, px(0.));
    path.move_to(right);
    path.arc_to(point(radius, radius), px(0.), false, true, left);
    path.arc_to(point(radius, radius), px(0.), false, true, right);
    path.close();
}

fn circle_path(center: Point<Pixels>, radius: Pixels) -> anyhow::Result<Path<Pixels>> {
    let mut path = fill_path();
    append_circle(&mut path, center, radius);
    path.build()
}

fn append_segment(
    path: &mut PathBuilder,
    start: Point<Pixels>,
    end: Point<Pixels>,
    radius: Pixels,
) {
    let delta = end - start;
    let dx = f32::from(delta.x);
    let dy = f32::from(delta.y);
    let length = dx.hypot(dy);
    // Very short prefixes can coincide after translation to window pixels.
    if length == 0. {
        return;
    }
    let normal = point(radius * (-dy / length), radius * (dx / length));
    path.add_polygon(
        &[start + normal, start - normal, end - normal, end + normal],
        true,
    );
}

fn check_path(bounds: Bounds<Pixels>, prefix: CheckPrefix) -> anyhow::Result<Path<Pixels>> {
    let mut path = fill_path();
    let radius = px(CHECK_SCALE); // SVG stroke-width 2, scaled from 24 to 14.
    let start = check_point(bounds, point(4., 12.));
    let tip = check_point(bounds, prefix.tip);
    if let Some(bend) = prefix.bend {
        let bend = check_point(bounds, bend);
        append_segment(&mut path, start, bend, radius);
        append_segment(&mut path, bend, tip, radius);
        append_circle(&mut path, bend, radius);
    } else {
        append_segment(&mut path, start, tip, radius);
    }
    append_circle(&mut path, start, radius);
    append_circle(&mut path, tip, radius);
    path.build()
}

#[cfg(any(debug_assertions, test))]
mod paint_trace {
    use std::{
        fs::File,
        io::Write as _,
        sync::{Mutex, OnceLock},
        time::Instant,
    };

    use gpui::{Bounds, Pixels, Point, Window};
    use serde_json::json;

    struct Trace {
        origin: Instant,
        file: File,
    }

    static TRACE: OnceLock<Option<Mutex<Trace>>> = OnceLock::new();

    #[expect(
        clippy::too_many_arguments,
        reason = "A paint record describes the actual canvas and its mark geometry."
    )]
    pub(super) fn record(
        id: &str,
        kind: &str,
        progress: f32,
        checked: Option<bool>,
        bounds: Bounds<Pixels>,
        window: &Window,
        circle: Option<(Point<Pixels>, Pixels)>,
        check_tip: Option<Point<Pixels>>,
    ) {
        let trace = TRACE.get_or_init(|| {
            let path = std::env::var_os("BLOCK_SELECTION_MOTION_TRACE")?;
            match File::create(path) {
                Ok(file) => Some(Mutex::new(Trace {
                    origin: Instant::now(),
                    file,
                })),
                Err(_) => {
                    eprintln!("Could not create selection motion trace.");
                    None
                }
            }
        });
        let Some(trace) = trace else { return };
        let Ok(mut trace) = trace.lock() else { return };
        let coordinates = |point: Point<Pixels>| [f32::from(point.x), f32::from(point.y)];
        let value = json!({
            "type": "paint",
            "id": id,
            "kind": kind,
            "p": progress,
            "checked": checked,
            "scale": window.scale_factor(),
            "bounds": [
                f32::from(bounds.origin.x), f32::from(bounds.origin.y),
                f32::from(bounds.size.width), f32::from(bounds.size.height)
            ],
            "circle_center": circle.map(|(center, _)| coordinates(center)),
            "circle_radius": circle.map(|(_, radius)| f32::from(radius)),
            "check_tip": check_tip.map(coordinates),
            "t_ms": trace.origin.elapsed().as_secs_f64() * 1000.
        });
        if serde_json::to_writer(&mut trace.file, &value).is_ok() {
            let _ = trace.file.write_all(b"\n");
        }
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Context as _;
    use gpui::{Bounds, Path, Pixels, Point, point, px, size};

    use super::{CHECK_SCALE, check_path, check_point, check_prefix, circle_path};

    fn bounds() -> Bounds<Pixels> {
        Bounds::new(point(px(10.25), px(20.5)), size(px(16.), px(16.)))
    }

    fn near(actual: f32, expected: f32) {
        assert!((actual - expected).abs() < 0.02, "{actual} != {expected}");
    }

    fn near_point(actual: Point<f32>, expected: Point<f32>) {
        near(actual.x, expected.x);
        near(actual.y, expected.y);
    }

    // Query the real tessellated triangles, including overlaps at the join.
    fn contains(path: &Path<Pixels>, sample: Point<Pixels>) -> bool {
        let cross = |a: Point<Pixels>, b: Point<Pixels>, c: Point<Pixels>| {
            f32::from(b.x - a.x) * f32::from(c.y - a.y)
                - f32::from(b.y - a.y) * f32::from(c.x - a.x)
        };
        path.vertices.as_chunks::<3>().0.iter().any(|triangle| {
            let a = triangle[0].xy_position;
            let b = triangle[1].xy_position;
            let c = triangle[2].xy_position;
            if cross(a, b, c).abs() < 0.000_001 {
                return false;
            }
            let edges = [
                cross(a, b, sample),
                cross(b, c, sample),
                cross(c, a, sample),
            ];
            edges.iter().all(|edge| *edge >= -0.000_001)
                || edges.iter().all(|edge| *edge <= 0.000_001)
        })
    }

    #[test]
    fn check_starts_empty_and_draws_the_short_segment_first() -> anyhow::Result<()> {
        assert!(check_prefix(0.).is_none());
        assert!(check_prefix(0.15).is_none());
        let short = check_prefix(0.15 + 0.85 * 2.5 / 16.).context("short prefix")?;
        assert!(short.bend.is_none());
        near_point(short.tip, point(6.5, 14.5));
        let bend = check_prefix(0.15 + 0.85 * 5. / 16.).context("bend prefix")?;
        near_point(bend.tip, point(9., 17.));
        let long = check_prefix(0.15 + 0.85 * 10.5 / 16.).context("long prefix")?;
        assert_eq!(long.bend, Some(point(9., 17.)));
        near_point(long.tip, point(14.5, 11.5));
        near_point(check_prefix(1.).context("full prefix")?.tip, point(20., 6.));
        Ok(())
    }

    #[test]
    fn check_path_has_round_caps_and_join_without_overlap_holes() -> anyhow::Result<()> {
        let bounds = bounds();
        let path = check_path(bounds, check_prefix(1.).context("full prefix")?)?;
        let start = check_point(bounds, point(4., 12.));
        let bend = check_point(bounds, point(9., 17.));
        let radius = px(CHECK_SCALE);
        let diagonal = radius * std::f32::consts::FRAC_1_SQRT_2;
        let direction = point(diagonal, diagonal);
        let normal = point(-diagonal, diagonal);
        assert!(contains(&path, start - direction * 0.8));
        assert!(!contains(&path, start - direction * 0.9 + normal * 0.9));
        assert!(contains(&path, bend));
        assert!(contains(&path, bend + point(px(0.), radius * 0.8)));
        assert!(!contains(&path, bend + point(px(0.), radius * 1.2)));
        Ok(())
    }

    #[test]
    fn completed_check_keeps_the_existing_fourteen_pixel_mapping() -> anyhow::Result<()> {
        let bounds = bounds();
        let path = check_path(bounds, check_prefix(1.).context("full prefix")?)?;
        let start = check_point(bounds, point(4., 12.));
        let tip = check_point(bounds, point(20., 6.));
        let bend = check_point(bounds, point(9., 17.));
        near(
            f32::from(path.bounds.left()),
            f32::from(start.x) - CHECK_SCALE,
        );
        near(
            f32::from(path.bounds.right()),
            f32::from(tip.x) + CHECK_SCALE,
        );
        near(f32::from(path.bounds.top()), f32::from(tip.y) - CHECK_SCALE);
        near(
            f32::from(path.bounds.bottom()),
            f32::from(bend.y) + CHECK_SCALE,
        );
        Ok(())
    }

    #[test]
    fn radio_paths_keep_the_center_at_fractional_display_scales() -> anyhow::Result<()> {
        let center = bounds().center();
        for radius in [0.25, 1., 2., 4., 8.] {
            let path = circle_path(center, px(radius))?;
            assert!(contains(&path, center));
            near(f32::from(path.bounds.size.width), radius * 2.);
            near(f32::from(path.bounds.size.height), radius * 2.);
            for scale in [1., 1.25, 1.5, 2.] {
                let scaled = path.scale(scale);
                near(scaled.bounds.center().x.0 / scale, f32::from(center.x));
                near(scaled.bounds.center().y.0 / scale, f32::from(center.y));
            }
        }
        Ok(())
    }

    #[test]
    fn every_visible_check_prefix_tessellates_and_can_retract() -> anyhow::Result<()> {
        let bounds = bounds();
        for step in (0_u16..=100).rev() {
            let progress = f32::from(step) / 100.;
            if let Some(prefix) = check_prefix(progress) {
                let path = check_path(bounds, prefix)?;
                assert!(!path.vertices.is_empty());
                assert!(contains(&path, check_point(bounds, prefix.tip)));
                assert!(path.vertices.iter().all(|vertex| {
                    f32::from(vertex.xy_position.x).is_finite()
                        && f32::from(vertex.xy_position.y).is_finite()
                }));
            } else {
                assert!(progress <= 0.15);
            }
        }
        Ok(())
    }
}
