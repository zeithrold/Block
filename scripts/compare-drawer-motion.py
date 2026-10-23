#!/usr/bin/env python3
"""Compare captured Native paint submissions with browser RAF measurements.

This reads the drawer motion captures and writes a self-contained HTML report,
raw-frame CSV, interpolated CSV, and JSON summary. The visualizations describe
captured geometry/timing; neither source is a GPU display-present timestamp.
"""

from __future__ import annotations

import argparse
import csv
import html
import json
import math
import statistics
import sys
from pathlib import Path
from typing import Any


SOURCES = ("native-before", "native-after", "prototype")
COLORS = {"native-before": "#ef8a24", "native-after": "#25a36f", "prototype": "#2878d0"}


def read_jsonl(path: Path) -> list[dict[str, Any]]:
    values: list[dict[str, Any]] = []
    with path.open(encoding="utf-8") as stream:
        for line_number, line in enumerate(stream, 1):
            if line.strip():
                try:
                    values.append(json.loads(line))
                except json.JSONDecodeError as error:
                    raise ValueError(f"{path}:{line_number}: invalid JSON: {error}") from error
    return values


def first_action(events: list[dict[str, Any]], name: str) -> dict[str, Any]:
    for event in events:
        if event.get("type") == "action" and event.get("action") == name:
            return event
    raise ValueError(f"capture has no {name!r} action event")


def first_revision(events: list[dict[str, Any]], part: str) -> list[dict[str, Any]]:
    candidates = [
        event for event in events
        if event.get("type") == "paint"
        and event.get("part") == part
        and isinstance(event.get("sample"), dict)
        and not event["sample"].get("closing", False)
    ]
    if not candidates:
        return []
    revision = candidates[0]["sample"].get("revision")
    return [event for event in candidates if event["sample"].get("revision") == revision]


def float_or_none(value: Any) -> float | None:
    try:
        result = float(value)
        return result if math.isfinite(result) else None
    except (TypeError, ValueError):
        return None


def transform_y(transform: Any) -> float | None:
    if not isinstance(transform, str):
        return None
    text = transform.strip()
    try:
        if text.startswith("matrix3d(") and text.endswith(")"):
            values = [float(value.strip()) for value in text[9:-1].split(",")]
            return values[13] if len(values) == 16 else None
        if text.startswith("matrix(") and text.endswith(")"):
            values = [float(value.strip()) for value in text[7:-1].split(",")]
            return values[5] if len(values) == 6 else None
        if text == "none":
            return 0.0
    except ValueError:
        return None
    return None


def animation_for(metrics: dict[str, Any], property_name: str) -> dict[str, Any] | None:
    items: list[Any] = []
    direct = metrics.get("animation")
    if isinstance(direct, dict):
        items.append(direct)
    animations = metrics.get("animations")
    if isinstance(animations, list):
        items.extend(animations)
    elif isinstance(animations, dict):
        items.extend(animations.values())
    for item in items:
        if isinstance(item, dict) and item.get("property") == property_name:
            return item
    return None


def browser_frames(capture: dict[str, Any]) -> list[dict[str, Any]]:
    captured = capture.get("frames", [])
    # Web Animations exposes the transition's own start time on the same
    # document timeline as RAF. Use that observed clock, never a curve fit.
    starts: dict[str, float] = {}
    for part, property_name in (("popup", "transform"), ("backdrop", "opacity")):
        observed = [
            value for frame in captured
            if (animation := animation_for(frame.get(part) or {}, property_name))
            and (value := float_or_none(animation.get("startTime"))) is not None
        ]
        if not observed:
            raise ValueError(f"prototype capture lacks {property_name} transition startTime; recapture with animation metadata")
        starts[part] = statistics.median(observed)
    closed_top = float(captured[0]["popup"]["y"])
    result = []
    for frame in captured:
        popup = frame.get("popup") or {}
        backdrop = frame.get("backdrop") or {}
        viewport = frame.get("viewport") or capture.get("viewport") or {}
        height = float_or_none(popup.get("height"))
        y = float_or_none(popup.get("y"))
        progress = None
        travel = None
        if height and float_or_none(viewport.get("height")) is not None and y is not None:
            travel = closed_top - (float(viewport["height"]) - height)
            progress = (closed_top - y) / travel
        transform_y_px = transform_y(popup.get("transform"))
        transform_progress = 1.0 - transform_y_px / travel if travel and transform_y_px is not None else None
        transform_animation = animation_for(popup, "transform")
        backdrop_animation = animation_for(backdrop, "opacity")
        raw_ms = float_or_none(frame.get("elapsedMs"))
        transform_current_time = float_or_none(transform_animation.get("currentTime")) if transform_animation else None
        raf_timestamp = float_or_none(frame.get("rafTimestamp"))
        phase_ms = transform_current_time
        if phase_ms is None and raf_timestamp is not None:
            phase_ms = max(0.0, raf_timestamp - starts["popup"])
        backdrop_time = float_or_none(backdrop_animation.get("currentTime")) if backdrop_animation else None
        if backdrop_time is None and raf_timestamp is not None:
            backdrop_time = max(0.0, raf_timestamp - starts["backdrop"])
        result.append({
            "source": "prototype",
            "kind": capture.get("kind"),
            "raw_ms": raw_ms,
            "phase_ms": phase_ms,
            "phase_clock": "observed CSS transition timeline (currentTime; RAF minus reported startTime outside transition)",
            "backdrop_phase_ms": backdrop_time,
            "raf_timestamp": raf_timestamp,
            "performance_now": float_or_none(frame.get("performanceNow")),
            "progress": progress,
            "progress_metric": "measured panel top, normalized by observed closed-to-rest travel",
            "travel_px": travel,
            "closed_top": closed_top,
            "css_start_time": starts["popup"],
            "transform_progress": transform_progress,
            "backdrop_progress": float_or_none(backdrop.get("opacity")),
            "x": float_or_none(popup.get("x")),
            "y": y,
            "width": float_or_none(popup.get("width")),
            "height": height,
            "backdrop_x": float_or_none(backdrop.get("x")),
            "backdrop_y": float_or_none(backdrop.get("y")),
            "backdrop_width": float_or_none(backdrop.get("width")),
            "backdrop_height": float_or_none(backdrop.get("height")),
            "active": frame.get("visibility") == "visible" and frame.get("focused") is True,
            "dpr": float_or_none((frame.get("viewport") or {}).get("dpr")),
            "viewport_width": float_or_none(viewport.get("width")),
            "viewport_height": float_or_none(viewport.get("height")),
            "reduced_motion": frame.get("reducedMotion"),
            "visibility": frame.get("visibility"),
            "focused": frame.get("focused"),
            "transform": popup.get("transform"),
            "opacity": float_or_none(popup.get("opacity")),
            "transition_duration": popup.get("transitionDuration"),
            "transition_timing_function": popup.get("transitionTimingFunction"),
            "transform_animation": transform_animation,
            "backdrop_animation": backdrop_animation,
            "children": frame.get("children", []),
        })
    return result


def load_capture(input_dir: Path, name: str) -> tuple[list[dict[str, Any]], dict[str, Any] | None]:
    if name == "prototype":
        payload = json.loads((input_dir / "drawer-prototype.json").read_text(encoding="utf-8"))
        captures = payload.get("captures") or []
        if not captures:
            raise ValueError("drawer-prototype.json contains no captures")
        capture = captures[-1]
        frames = browser_frames(capture)
        if not frames:
            raise ValueError("the selected prototype capture contains no RAF frames")
        return frames, {"metadata": payload, "capture": capture}

    events = read_jsonl(input_dir / f"{name}.jsonl")
    action = first_action(events, "open")
    action_ms = float_or_none(action.get("t_ms"))
    if action_ms is None:
        raise ValueError(f"{name}: open action has no numeric t_ms")
    panel = first_revision(events, "panel")
    backdrop = first_revision(events, "backdrop")
    if not panel or not backdrop:
        raise ValueError(f"{name}: missing panel or backdrop paint events in first non-closing revision")
    meta = next((event for event in events if event.get("type") == "meta"), {})
    first_render = float_or_none(panel[0].get("sample", {}).get("render_ms"))
    first_sample = panel[0]["sample"]
    closed_top = float(panel[0]["y"]) - (float(first_sample["height"]) - float(panel[0]["height"]))
    frames = []
    for panel_event in panel:
        sample = panel_event["sample"]
        t_ms = float_or_none(panel_event.get("t_ms"))
        render_ms = float_or_none(sample.get("render_ms"))
        sample_height = float_or_none(sample.get("height"))
        event_height = float_or_none(panel_event.get("height"))
        y = float_or_none(panel_event.get("y"))
        # Native probe bounds begin one pixel inside the panel top border.
        panel_y = y - (sample_height - event_height) if y is not None and sample_height is not None and event_height is not None else y
        travel = closed_top - (float(panel_event["viewport_height"]) - sample_height) if sample_height is not None else None
        progress = (closed_top - panel_y) / travel if panel_y is not None and travel else None
        backdrop_event = min(backdrop, key=lambda item: abs(float(item.get("sample", {}).get("render_ms", 0)) - (render_ms or 0)))
        backdrop_sample = backdrop_event.get("sample", {})
        phase_ms = render_ms - first_render if render_ms is not None and first_render is not None else None
        frames.append({
            "source": name,
            "kind": "native",
            "raw_ms": t_ms - action_ms if t_ms is not None else None,
            "phase_ms": phase_ms,
            "phase_clock": "Native sample.render_ms minus first entrance render_ms",
            "backdrop_phase_ms": phase_ms,
            "raf_timestamp": None,
            "performance_now": None,
            "progress": progress,
            "progress_metric": "measured panel top, normalized by observed closed-to-rest travel",
            "travel_px": travel,
            "closed_top": closed_top,
            "transform_progress": float_or_none(sample.get("progress")),
            "backdrop_progress": float_or_none(backdrop_sample.get("scrim")),
            "x": float_or_none(panel_event.get("x")),
            "y": panel_y,
            "width": float_or_none(panel_event.get("width")),
            "height": sample_height,
            "probe_y": y,
            "probe_height": event_height,
            "backdrop_x": float_or_none(backdrop_event.get("x")),
            "backdrop_y": float_or_none(backdrop_event.get("y")),
            "backdrop_width": float_or_none(backdrop_event.get("width")),
            "backdrop_height": float_or_none(backdrop_event.get("height")),
            "active": panel_event.get("active"),
            "dpr": float_or_none(panel_event.get("dpr")),
            "viewport_width": float_or_none(panel_event.get("viewport_width")),
            "viewport_height": float_or_none(panel_event.get("viewport_height")),
            "reduced_motion": sample.get("reduced"),
            "visibility": None,
            "focused": None,
            "transform": None,
            "opacity": None,
            "transition_duration": None,
            "transition_timing_function": None,
            "transform_animation": None,
            "backdrop_animation": None,
            "native_translation": float_or_none(sample.get("translation")),
            "native_scrim": float_or_none(sample.get("scrim")),
            "revision": sample.get("revision"),
            "render_ms": render_ms,
            "first_render_ms": first_render,
            "offset": float_or_none(sample.get("offset")),
            "meta": meta,
            "children": [],
        })
    return frames, {"events": events, "action": action, "meta": meta}


def sorted_series(frames: list[dict[str, Any]], clock: str, metric: str) -> list[tuple[float, float]]:
    if clock == "phase_ms" and metric == "backdrop_progress":
        clock = "backdrop_phase_ms"
    values = [(float_or_none(frame.get(clock)), float_or_none(frame.get(metric))) for frame in frames]
    clean = sorted((x, y) for x, y in values if x is not None and y is not None)
    unique: list[tuple[float, float]] = []
    for point in clean:
        if unique and point[0] == unique[-1][0]:
            unique[-1] = point
        else:
            unique.append(point)
    return unique


def interpolate(series: list[tuple[float, float]], time_ms: float) -> float | None:
    if not series or time_ms < series[0][0] or time_ms > series[-1][0]:
        return None
    if time_ms == series[-1][0]:
        return series[-1][1]
    for (x0, y0), (x1, y1) in zip(series, series[1:]):
        if x0 <= time_ms <= x1:
            if x1 == x0:
                return y1
            amount = (time_ms - x0) / (x1 - x0)
            return y0 + amount * (y1 - y0)
    return None


def common_grid(series_by_source: dict[str, list[dict[str, Any]]], clock: str, step_ms: float = 8.0) -> list[float]:
    ranges = []
    for frames in series_by_source.values():
        values = [float_or_none(frame.get(clock)) for frame in frames]
        values = [value for value in values if value is not None]
        if not values:
            return []
        ranges.append((min(values), max(values)))
    start = max(item[0] for item in ranges)
    # Do not dilute entrance error with hundreds of settled frames.
    end = min(min(item[1] for item in ranges), 450.0 if clock == "phase_ms" else 500.0)
    if end < start:
        return []
    count = int(math.floor((end - start) / step_ms))
    return [start + index * step_ms for index in range(count + 1)] + ([end] if count == 0 or abs(start + count * step_ms - end) > 1e-6 else [])


def error_summary(series_by_source: dict[str, list[dict[str, Any]]], clock: str) -> dict[str, Any]:
    grid = common_grid(series_by_source, clock)
    result: dict[str, Any] = {"clock": clock, "gridStepMs": 8, "overlapStartMs": grid[0] if grid else None, "overlapEndMs": grid[-1] if grid else None, "comparisons": {}}
    if not grid:
        result["note"] = "No shared time interval across all three captures; errors are unavailable."
        return result
    for candidate in ("native-before", "native-after"):
        for metric, label in (("progress", "panelTravelFraction"), ("backdrop_progress", "backdropProgress")):
            errors = []
            left = sorted_series(series_by_source[candidate], clock, metric)
            right = sorted_series(series_by_source["prototype"], clock, metric)
            for time_ms in grid:
                a, b = interpolate(left, time_ms), interpolate(right, time_ms)
                if a is not None and b is not None:
                    errors.append(abs(a - b))
            result["comparisons"].setdefault(candidate, {})[label] = {
                "meanAbsoluteError": statistics.mean(errors) if errors else None,
                "maxAbsoluteError": max(errors) if errors else None,
                "samples": len(errors),
            }
    return result


def reached_time(series: list[tuple[float, float]], threshold: float) -> float | None:
    for index, (time_ms, value) in enumerate(series):
        if value >= threshold:
            if index == 0:
                return time_ms
            prev_time, prev_value = series[index - 1]
            if value == prev_value:
                return time_ms
            return prev_time + (threshold - prev_value) * (time_ms - prev_time) / (value - prev_value)
    return None


def cadence(frames: list[dict[str, Any]], clock: str) -> dict[str, Any]:
    times = [value for value in (float_or_none(frame.get(clock)) for frame in frames) if value is not None]
    deltas = [later - earlier for earlier, later in zip(times, times[1:]) if later >= earlier]
    return {
        "sampleCount": len(times),
        "meanIntervalMs": statistics.mean(deltas) if deltas else None,
        "medianIntervalMs": statistics.median(deltas) if deltas else None,
        "minIntervalMs": min(deltas) if deltas else None,
        "maxIntervalMs": max(deltas) if deltas else None,
    }


def latency_fields(frames: list[dict[str, Any]], *, prototype: bool) -> dict[str, Any]:
    raw = sorted_series(frames, "raw_ms", "progress")
    phase = sorted_series(frames, "phase_ms", "progress")
    first = frames[0] if frames else {}
    return {
        "firstObservedFrameAfterOpenMs": first.get("raw_ms"),
        "firstObservedPhaseMs": first.get("phase_ms"),
        "firstFrameClockSource": "RAF elapsedMs from harness trigger" if prototype else "native paint t_ms minus open action t_ms",
        "firstMeasuredTravelAtOrAbove99PercentMs": reached_time(raw, 0.99),
        "phaseClockTravelAtOrAbove99PercentMs": reached_time(phase, 0.99),
        "phaseClockSource": first.get("phase_clock"),
        "firstPaintOrRAFIsDisplayPresent": False,
    }


def csv_rows(series_by_source: dict[str, list[dict[str, Any]]]) -> list[dict[str, Any]]:
    rows = []
    for source in SOURCES:
        for frame in series_by_source[source]:
            rows.append({
                "source": source,
                "kind": frame.get("kind"),
                "captureKind": frame.get("kind") if source == "prototype" else "",
                "raw_ms_from_open": frame.get("raw_ms"),
                "phase_ms": frame.get("phase_ms"),
                "phase_clock": frame.get("phase_clock"),
                "backdrop_phase_ms": frame.get("backdrop_phase_ms"),
                "raf_timestamp": frame.get("raf_timestamp"),
                "performance_now": frame.get("performance_now"),
                "progress_metric": frame.get("progress_metric"),
                "panel_travel_fraction": frame.get("progress"),
                "panel_travel_px": frame.get("travel_px"),
                "closed_top_px": frame.get("closed_top"),
                "transform_progress": frame.get("transform_progress"),
                "backdrop_progress": frame.get("backdrop_progress"),
                "x": frame.get("x"), "y": frame.get("y"),
                "width": frame.get("width"), "height": frame.get("height"),
                "backdrop_x": frame.get("backdrop_x"), "backdrop_y": frame.get("backdrop_y"),
                "backdrop_width": frame.get("backdrop_width"), "backdrop_height": frame.get("backdrop_height"),
                "native_probe_y": frame.get("probe_y"), "native_probe_height": frame.get("probe_height"),
                "native_translation": frame.get("native_translation"), "native_render_ms": frame.get("render_ms"),
                "transform": frame.get("transform"), "opacity": frame.get("opacity"),
                "transition_duration": frame.get("transition_duration"),
                "transition_timing_function": frame.get("transition_timing_function"),
                "transform_animation": json.dumps(frame.get("transform_animation"), separators=(",", ":")) if frame.get("transform_animation") is not None else "",
                "backdrop_animation": json.dumps(frame.get("backdrop_animation"), separators=(",", ":")) if frame.get("backdrop_animation") is not None else "",
                "active": frame.get("active"), "dpr": frame.get("dpr"),
                "viewport_width": frame.get("viewport_width"), "viewport_height": frame.get("viewport_height"),
                "reduced_motion": frame.get("reduced_motion"), "visibility": frame.get("visibility"),
                "focused": frame.get("focused"),
            })
    return rows


def write_csv(path: Path, rows: list[dict[str, Any]], fields: list[str]) -> None:
    with path.open("w", encoding="utf-8", newline="") as stream:
        writer = csv.DictWriter(stream, fieldnames=fields, extrasaction="ignore")
        writer.writeheader()
        writer.writerows(rows)


def make_summary(input_dir: Path, series_by_source: dict[str, list[dict[str, Any]]], details: dict[str, Any]) -> dict[str, Any]:
    all_frames = [frame for frames in series_by_source.values() for frame in frames]
    viewport = next((
        {"width": frame.get("viewport_width"), "height": frame.get("viewport_height"), "dpr": frame.get("dpr")}
        for frame in all_frames if frame.get("viewport_width") and frame.get("viewport_height")
    ), None)
    native_heights = [frame.get("height") for source in ("native-before", "native-after") for frame in series_by_source[source] if frame.get("height")]
    panel_height = statistics.median(native_heights) if native_heights else None
    all_viewport_matches = bool(viewport) and all(
        frame.get("viewport_width") == viewport["width"]
        and frame.get("viewport_height") == viewport["height"]
        and frame.get("dpr") == viewport["dpr"]
        for frame in all_frames
    )
    active_nonreduced = all(frame.get("active") is True and frame.get("reduced_motion") is False for frame in all_frames)
    matched_geometry = bool(
        all_viewport_matches
        and round(float(viewport["width"])) == 432
        and round(float(viewport["height"])) == 768
        and viewport["dpr"] == 1
        and panel_height is not None
        and abs(panel_height - 540) < 1.5
        and all(abs(float(frame["height"]) - 540) < 1.5 for frame in series_by_source["prototype"])
    )
    prototype_transitions: dict[str, Any] = {}
    for transition in details["prototype"]["capture"].get("transitions", []):
        key = f"{transition.get('target', 'unknown')}:{transition.get('propertyName', 'unknown')}:{transition.get('type', 'unknown')}"
        prototype_transitions.setdefault(key, []).append(float_or_none(transition.get("elapsedMs")))
    return {
        "inputDirectory": str(input_dir.resolve()),
        "comparisonScope": {
            "viewport": viewport,
            "nativePanelHeightMedianPx": panel_height,
            "prototypeCaptureKind": details["prototype"]["capture"].get("kind"),
            "prototypeReducedMotion": details["prototype"]["capture"].get("reducedMotion"),
            "allSourcesShareViewportAndDpr": all_viewport_matches,
            "allSamplesActiveAndNonreduced": active_nonreduced,
            "matchedGeometryClaim": matched_geometry,
            "matchedActiveNonreducedCaptureClaim": matched_geometry and active_nonreduced,
            "perSource": {
                source: {
                    "viewport": next(({"width": f.get("viewport_width"), "height": f.get("viewport_height"), "dpr": f.get("dpr")} for f in frames if f.get("viewport_width") is not None), None),
                    "medianPanelHeightPx": statistics.median([f["height"] for f in frames if f.get("height") is not None]) if any(f.get("height") is not None for f in frames) else None,
                    "allActive": all(f.get("active") is True for f in frames),
                    "allNonreduced": all(f.get("reduced_motion") is False for f in frames),
                    "closedPanelTopPx": frames[0].get("closed_top"),
                    "fullTravelPx": frames[0].get("travel_px"),
                }
                for source, frames in series_by_source.items()
            },
        },
        "clockDefinitions": {
            "nativeRaw": "paint event t_ms minus first open action t_ms; monotonic paint-submission clock, not display presentation",
            "nativePhase": "sample.render_ms minus first entrance sample.render_ms; render/sample clock from Native probe",
            "prototypeRaw": "RAF frame elapsedMs measured from the browser capture trigger; rafTimestamp and performanceNow are retained in CSV",
            "prototypePhase": "observed CSS currentTime; before/after active transition use max(0, rafTimestamp minus the reported transition startTime), preserving a single document timeline. Backdrop uses its own observed opacity timeline.",
            "alignment": "No fitted time shift is applied. Raw trigger-relative timing and phase-clock comparisons are both reported.",
        },
        "latency": {source: latency_fields(series_by_source[source], prototype=source == "prototype") for source in SOURCES},
        "prototypeTransitionEventElapsedMs": {
            key: [value for value in values if value is not None]
            for key, values in prototype_transitions.items()
        },
        "cadence": {source: {"raw": cadence(series_by_source[source], "raw_ms"), "phase": cadence(series_by_source[source], "phase_ms")} for source in SOURCES},
        "meanMaximumAbsoluteError": {
            "rawTriggerRelative": error_summary(series_by_source, "raw_ms"),
            "phaseClock": error_summary(series_by_source, "phase_ms"),
        },
        "limitations": [
            "Native probe events report paint submission and the prototype reports browser RAF/computed-style measurements. Neither is a GPU display-present timestamp.",
            "Native panel bounds are inset by its top border; reconstructed panel top is event.y minus (sample.height minus event.height). Native device-pixel rounding can leave one-pixel geometry differences. Native sample clock is recorded after layer construction, rather than inside the motion primitive.",
            "Panel curves use measured popup/panel top normalized by each source's observed closed-to-rest travel. Native before travels 540px; after and prototype travel 542px. Raw geometry remains in CSV and reconstructions.",
            "Prototype curves use actual computed bounds and backdrop opacity. CSS startTime/currentTime are recorded directly; no easing curve or time offset is fitted. Trigger-relative and animation-phase clocks answer different questions.",
            "Entrance errors use only the shared interval up to 450ms of phase time (500ms of trigger time), with an 8ms interpolation grid. Settled frames do not dilute the mean error. This controlled sample is not a cross-platform frame-pacing benchmark.",
            "Geometry panels are nearest measured-frame reconstructions, not screenshots or pixel-accurate rendering captures.",
        ],
    }


def make_interpolated_rows(series_by_source: dict[str, list[dict[str, Any]]], clock: str) -> list[dict[str, Any]]:
    grid = common_grid(series_by_source, clock)
    rows = []
    for time_ms in grid:
        row: dict[str, Any] = {"clock": clock, "time_ms": time_ms}
        for source in SOURCES:
            for metric, suffix in (("progress", "panel_travel_fraction"), ("backdrop_progress", "backdrop_progress")):
                row[f"{source}_{suffix}"] = interpolate(sorted_series(series_by_source[source], clock, metric), time_ms)
        rows.append(row)
    return rows


def render_html(summary: dict[str, Any], series_by_source: dict[str, list[dict[str, Any]]], details: dict[str, Any]) -> str:
    payload = {"summary": summary, "sources": {name: series_by_source[name] for name in SOURCES}}
    json_text = json.dumps(payload, ensure_ascii=False, separators=(",", ":")).replace("</", "<\\/")
    source_names = {"native-before": "Native before", "native-after": "Native after", "prototype": "Prototype"}
    source_html = "".join(f'<span class="legend"><i style="background:{COLORS[name]}"></i>{html.escape(source_names[name])}</span>' for name in SOURCES)
    return f'''<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Drawer motion comparison</title>
<style>
:root{{color-scheme:light;--ink:#18212b;--muted:#637181;--line:#d7dee7;--card:#fff;--bg:#f3f6fa}}
*{{box-sizing:border-box}}body{{margin:0;background:var(--bg);color:var(--ink);font:14px/1.5 system-ui,-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif}}
main{{max-width:1200px;margin:0 auto;padding:28px 22px 52px}}h1{{font-size:25px;margin:0 0 4px}}h2{{font-size:17px;margin:0 0 14px}}p{{margin:7px 0;color:var(--muted)}}.card{{background:var(--card);border:1px solid var(--line);border-radius:12px;padding:18px;margin:16px 0;box-shadow:0 2px 7px #15283a0b}}
.notice{{border-left:4px solid #607b96;padding:10px 13px;background:#edf3f8;border-radius:4px;color:#34495e}}.legend-row{{display:flex;gap:18px;flex-wrap:wrap;margin:10px 0}}.legend{{display:inline-flex;align-items:center;gap:7px}}.legend i{{width:12px;height:12px;border-radius:50%;display:inline-block}}
svg{{display:block;width:100%;height:auto;overflow:visible}}.axis{{stroke:#98a6b4;stroke-width:1}}.grid{{stroke:#e7ebf0;stroke-width:1}}.tick{{fill:#627184;font-size:12px}}.axis-label{{fill:#445467;font-size:13px}}.curve{{fill:none;stroke-width:2.5;vector-effect:non-scaling-stroke}}.dots{{stroke:white;stroke-width:1.5}}
.slider-row{{display:flex;align-items:center;gap:14px;margin:10px 0 16px}}input[type=range]{{flex:1}}.readout{{font-variant-numeric:tabular-nums;min-width:100px;text-align:right;font-weight:600}}
.previews{{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:14px}}.preview{{min-width:0}}.preview h3{{font-size:14px;margin:0 0 6px}}.screen{{position:relative;overflow:hidden;width:min(100%,270px);aspect-ratio:432/768;border:1px solid #8e9aaa;background:linear-gradient(#fff,#f9fafb);margin:0 auto;border-radius:4px;box-shadow:0 4px 14px #10203018}}.shade{{position:absolute;inset:0;background:#000}}.panel{{position:absolute;display:flex;flex-direction:column;overflow:hidden;border:1px solid #c9d1da;border-top-width:1px;border-radius:10px 10px 0 0;background:#fff;box-shadow:0 -2px 14px #1112;will-change:auto}}.preview-meta{{text-align:center;color:var(--muted);font-size:12px;margin-top:5px}}
table{{border-collapse:collapse;width:100%;font-variant-numeric:tabular-nums}}th,td{{text-align:left;padding:7px 8px;border-bottom:1px solid #e7ebf0;vertical-align:top}}th{{font-size:12px;color:#536477}}td.num{{text-align:right;white-space:nowrap}}code{{font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:12px}}ul{{padding-left:20px;color:var(--muted)}}.small{{font-size:12px;color:var(--muted)}}
@media(max-width:720px){{.previews{{grid-template-columns:1fr}}.preview{{display:grid;grid-template-columns:130px 1fr;align-items:start;gap:10px}}.preview h3,.preview-meta{{grid-column:2;text-align:left}}.screen{{width:130px;grid-column:1;grid-row:1 / span 3}}}}
</style></head><body><main>
<h1>Drawer motion comparison</h1><p>Measured entrance curves and sampled geometry across Native before, Native after, and the Base UI prototype.</p>
<div class="notice"><strong>Measurement boundary:</strong> Native rows are paint-submission events and browser rows are RAF/computed-style samples. Neither timestamp means the GPU displayed that frame. Bounds use the matched 432 × 768 viewport at DPR 1; native device-pixel rounding can produce one-pixel differences.</div>
<section class="card"><h2>Motion curves</h2><div class="legend-row">{source_html}</div><p>Solid lines show measured panel travel; dashed lines show independent backdrop progress. Phase clocks use the first Native render and the observed CSS transition timeline. Initial browser mounting time stays visible in the trigger-relative chart. No time shift is fitted.</p>
<svg id="curve-raw" viewBox="0 0 1000 320" role="img" aria-label="Panel and backdrop progress against trigger-relative milliseconds"></svg>
<svg id="curve-phase" viewBox="0 0 1000 320" role="img" aria-label="Panel and backdrop progress against phase-clock milliseconds"></svg>
</section>
<section class="card"><h2>Nearest measured-frame geometry</h2><p>Reconstruction of recorded bounds and progress metrics, not a screenshot. The backdrop shade visualizes 10% of the captured progress metric. Move the slider to choose the nearest sample in each capture.</p>
<div class="slider-row"><label for="time-slider">Phase time</label><input id="time-slider" type="range" min="0" max="500" step="1" value="0"><output id="time-readout" class="readout">0 ms</output></div>
<div class="previews">{''.join(f'<div class="preview"><h3 style="color:{COLORS[name]}">{html.escape(source_names[name])}</h3><div class="screen" id="screen-{name}"><div class="shade"></div><div class="panel"></div></div><div class="preview-meta" id="meta-{name}">No sample</div></div>' for name in SOURCES)}</div>
</section>
<section class="card"><h2>Capture alignment and latency</h2><div id="latency"></div></section>
<section class="card"><h2>Common-time interpolation error</h2><p>Linear interpolation uses an 8ms grid within the shared entrance interval: phase time up to 450ms; trigger time up to 500ms. Errors are absolute differences of normalized travel or backdrop progress. Closed-to-rest travel is 540px before, 542px after and in the prototype.</p><div id="errors"></div><p class="small">See <code>common-time.csv</code> for every interpolated point and <code>summary.json</code> for the exact overlap.</p></section>
<section class="card"><h2>Capture metadata and limitations</h2><div id="metadata"></div><ul>{''.join(f'<li>{html.escape(item)}</li>' for item in summary['limitations'])}</ul>
<p class="small">Prototype capture kind: {html.escape(str(details['prototype']['capture'].get('kind','unknown')))}. Raw browser <code>elapsedMs</code>, RAF timestamp and <code>performance.now()</code> remain in <code>raw-frames.csv</code>, along with native event and render clocks.</p></section>
<script id="capture-data" type="application/json">{json_text}</script>
<script>
const DATA=JSON.parse(document.getElementById('capture-data').textContent);
const SOURCES=['native-before','native-after','prototype'];
const COLORS={json.dumps(COLORS)};
const LABELS={{'native-before':'Native before','native-after':'Native after','prototype':'Prototype'}};
function esc(v){{return String(v??'—').replace(/[&<>"']/g,c=>({{'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}}[c]))}}
function drawChart(id,clock,title){{
 const svg=document.getElementById(id),W=1000,H=320,L=58,R=20,T=24,B=46,pw=W-L-R,ph=H-T-B;
 const all=SOURCES.flatMap(s=>DATA.sources[s].map(f=>f[clock]).filter(Number.isFinite));
 if(!all.length){{svg.innerHTML='<text x="20" y="35">No timing samples</text>';return}}
 let xmin=0,xmax=clock==='phase_ms'?450:500;if(xmax<=xmin)xmax=xmin+1;
 const sx=x=>L+(x-xmin)/(xmax-xmin)*pw,sy=y=>T+(1-y)*ph;
 let out=`<text class="axis-label" x="${{L}}" y="15">${{title}}</text>`;
 for(let k=0;k<=4;k++){{let v=k/4,y=sy(v);out+=`<line class="grid" x1="${{L}}" x2="${{W-R}}" y1="${{y}}" y2="${{y}}"/><text class="tick" x="${{L-10}}" y="${{y+4}}" text-anchor="end">${{v.toFixed(2)}}</text>`}}
 out+=`<line class="axis" x1="${{L}}" x2="${{W-R}}" y1="${{H-B}}" y2="${{H-B}}"/><text class="axis-label" x="${{W/2}}" y="${{H-8}}" text-anchor="middle">milliseconds (no extrapolation)</text>`;
 for(let k=0;k<=5;k++){{let x=xmin+(xmax-xmin)*k/5,px=sx(x);out+=`<line class="grid" x1="${{px}}" x2="${{px}}" y1="${{T}}" y2="${{H-B}}"/><text class="tick" x="${{px}}" y="${{H-B+18}}" text-anchor="middle">${{Math.round(x)}}</text>`}}
 const metrics=[['progress','panel travel'],['backdrop_progress','backdrop']];
 for(const [metric,label] of metrics)for(const source of SOURCES){{const frames=DATA.sources[source].filter(f=>Number.isFinite(f[clock])&&Number.isFinite(f[metric])&&f[clock]<=xmax).sort((a,b)=>a[clock]-b[clock]);if(!frames.length)continue;let d=frames.map((f,i)=>`${{i?'L':'M'}}${{sx(f[clock]).toFixed(2)}},${{sy(f[metric]).toFixed(2)}}`).join(' ');let dash=metric==='backdrop_progress'?' stroke-dasharray="5 4"':'';out+=`<path class="curve" d="${{d}}" stroke="${{COLORS[source]}}"${{dash}} opacity="${{metric==='progress'?1:.6}}"><title>${{LABELS[source]}} ${{label}}</title></path>`}}
 svg.innerHTML=out;
}}
drawChart('curve-raw','raw_ms','Trigger-relative clock');
drawChart('curve-phase','phase_ms','Animation phase (first Native render / observed CSS timeline)');
function nearest(frames,time){{let valid=frames.filter(f=>Number.isFinite(f.phase_ms));return valid.reduce((best,f)=>!best||Math.abs(f.phase_ms-time)<Math.abs(best.phase_ms-time)?f:best,null)}}
function renderGeometry(time){{document.getElementById('time-readout').value=Math.round(time)+' ms';for(const source of SOURCES){{const f=nearest(DATA.sources[source],time);const screen=document.getElementById('screen-'+source),panel=screen.querySelector('.panel'),shade=screen.querySelector('.shade'),meta=document.getElementById('meta-'+source);if(!f||![f.x,f.y,f.width,f.height].every(Number.isFinite)){{panel.style.display='none';meta.textContent='No nearby measured frame';continue}}panel.style.display='flex';const vw=f.viewport_width||432,vh=f.viewport_height||768;panel.style.left=(f.x/vw*100)+'%';panel.style.top=(f.y/vh*100)+'%';panel.style.width=(f.width/vw*100)+'%';panel.style.height=(f.height/vh*100)+'%';panel.style.opacity=f.opacity??1;shade.style.opacity=0.1*Math.max(0,Math.min(1,f.backdrop_progress??0));meta.textContent=`phase ${{Number(f.phase_ms).toFixed(1)}} ms · travel ${{Number(f.progress).toFixed(3)}} · backdrop metric ${{Number(f.backdrop_progress??0).toFixed(3)}} · ${{Math.round(f.width)}}×${{Math.round(f.height)}} at (${{Math.round(f.x)}}, ${{Math.round(f.y)}})`}}}}
document.getElementById('time-slider').addEventListener('input',e=>renderGeometry(Number(e.target.value)));
document.getElementById('time-slider').min='0';document.getElementById('time-slider').max='500';renderGeometry(0);
function table(headers,rows){{return '<table><thead><tr>'+headers.map(x=>'<th>'+esc(x)+'</th>').join('')+'</tr></thead><tbody>'+rows.map(row=>'<tr>'+row.map((cell,i)=>'<td class="'+(i?'num':'')+'">'+esc(cell)+'</td>').join('')+'</tr>').join('')+'</tbody></table>'}}
const lat=[];for(const s of SOURCES){{const x=DATA.summary.latency[s];lat.push([LABELS[s],x.firstObservedFrameAfterOpenMs==null?'—':x.firstObservedFrameAfterOpenMs.toFixed(2),x.firstObservedPhaseMs==null?'—':x.firstObservedPhaseMs.toFixed(2),x.firstMeasuredTravelAtOrAbove99PercentMs==null?'not reached':x.firstMeasuredTravelAtOrAbove99PercentMs.toFixed(2),x.phaseClockTravelAtOrAbove99PercentMs==null?'not reached':x.phaseClockTravelAtOrAbove99PercentMs.toFixed(2),x.phaseClockSource])}}document.getElementById('latency').innerHTML=table(['Source','First observed after trigger (ms)','First phase-clock sample (ms)','99% travel raw time (ms)','99% travel phase time (ms)','Phase clock'],lat);
const errRows=[];for(const clock of ['rawTriggerRelative','phaseClock'])for(const s of ['native-before','native-after']){{const block=DATA.summary.meanMaximumAbsoluteError[clock].comparisons[s]||{{}};for(const m of ['panelTravelFraction','backdropProgress']){{const x=block[m]||{{}};errRows.push([clock,LABELS[s],m,x.meanAbsoluteError==null?'no common interval':x.meanAbsoluteError.toFixed(4),x.maxAbsoluteError==null?'—':x.maxAbsoluteError.toFixed(4),x.samples??0])}}}}document.getElementById('errors').innerHTML=table(['Clock','Compared Native capture','Curve','Mean absolute error','Maximum absolute error','Points'],errRows);
const c=DATA.summary.comparisonScope;const cadence=DATA.summary.cadence;let metaRows=[['Viewport',c.viewport?`${{c.viewport.width}}×${{c.viewport.height}} at DPR ${{c.viewport.dpr}}`:'unavailable'],['Native median panel height',c.nativePanelHeightMedianPx==null?'unavailable':c.nativePanelHeightMedianPx.toFixed(2)+' px'],['Matched requested geometry',c.matchedGeometryClaim?'432×768 viewport and ~540px panel observed':'requested geometry not fully confirmed'],['All samples active and nonreduced',String(c.matchedActiveNonreducedCaptureClaim)],['Prototype capture kind',String(c.prototypeCaptureKind)],['Prototype reduced motion',String(c.prototypeReducedMotion)],['Prototype raw-frame cadence',`${{cadence.prototype.raw.meanIntervalMs?.toFixed(2)??'—'}} ms mean; ${{cadence.prototype.raw.sampleCount}} frames`],['Native before cadence',`${{cadence['native-before'].raw.meanIntervalMs?.toFixed(2)??'—'}} ms mean; ${{cadence['native-before'].raw.sampleCount}} paint events`],['Native after cadence',`${{cadence['native-after'].raw.meanIntervalMs?.toFixed(2)??'—'}} ms mean; ${{cadence['native-after'].raw.sampleCount}} paint events`]];document.getElementById('metadata').innerHTML=table(['Item','Observed value'],metaRows);
</script></main></body></html>'''


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input-dir", type=Path, default=Path(__file__).resolve().parents[1] / "target/drawer-motion")
    parser.add_argument("--output-dir", type=Path, default=None)
    args = parser.parse_args()
    input_dir = args.input_dir.resolve()
    output_dir = (args.output_dir or input_dir).resolve()
    missing = [name for name in ("native-before.jsonl", "native-after.jsonl", "drawer-prototype.json") if not (input_dir / name).is_file()]
    if missing:
        print("Cannot compare drawer motion: required capture file(s) are missing: " + ", ".join(missing), file=sys.stderr)
        if "native-after.jsonl" in missing:
            print("Capture the Native after state, save it as native-after.jsonl, then rerun this script.", file=sys.stderr)
        return 2
    try:
        series: dict[str, list[dict[str, Any]]] = {}
        details: dict[str, Any] = {}
        for name in SOURCES:
            series[name], details[name] = load_capture(input_dir, name)
        summary = make_summary(input_dir, series, details)
        output_dir.mkdir(parents=True, exist_ok=True)
        write_csv(output_dir / "raw-frames.csv", csv_rows(series), list(csv_rows(series)[0].keys()))
        interpolated = make_interpolated_rows(series, "raw_ms") + make_interpolated_rows(series, "phase_ms")
        if interpolated:
            write_csv(output_dir / "common-time.csv", interpolated, list(interpolated[0].keys()))
        else:
            write_csv(output_dir / "common-time.csv", [], ["clock", "time_ms"] + [f"{source}_{metric}" for source in SOURCES for metric in ("panel_travel_fraction", "backdrop_progress")])
        (output_dir / "summary.json").write_text(json.dumps(summary, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
        (output_dir / "report.html").write_text(render_html(summary, series, details), encoding="utf-8")
    except (OSError, ValueError, KeyError, TypeError) as error:
        print(f"Cannot compare drawer motion: {error}", file=sys.stderr)
        return 2
    print(f"Wrote {output_dir / 'report.html'}")
    print(f"Wrote {output_dir / 'raw-frames.csv'}, {output_dir / 'common-time.csv'}, and {output_dir / 'summary.json'}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
