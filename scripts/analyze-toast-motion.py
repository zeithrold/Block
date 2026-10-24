#!/usr/bin/env python3
"""Validate an opt-in BLOCK_TOAST_MOTION_TRACE JSONL capture."""

import argparse
import hashlib
import json
import math
import sys
from pathlib import Path


def read_trace(path: Path):
    events = []
    for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        try:
            event = json.loads(line)
        except json.JSONDecodeError as error:
            raise ValueError(f"line {number}: invalid JSON: {error}") from error
        if not isinstance(event, dict) or "type" not in event or "t_ms" not in event:
            raise ValueError(f"line {number}: expected an event object with type and t_ms")
        events.append(event)
    if not events:
        raise ValueError("trace is empty")
    return events


def finite_number(value):
    return isinstance(value, (int, float)) and math.isfinite(value)


def find_action(actions, name):
    matches = [event for event in actions if event.get("action") == name]
    return matches[0] if len(matches) == 1 else None


def phase_metrics(samples, absent_events, removed_events, action, key, label, errors, end_ms=None):
    candidates = [
        event for event in samples
        if event["t_ms"] >= action["t_ms"]
        and (end_ms is None or event["t_ms"] <= end_ms)
        and event.get("ending") is (label != "enter")
        and event.get(key) is not None
    ]
    candidate_values = [float(event[key]) for event in candidates]
    if label == "enter":
        if not candidates:
            errors.append(f"{label}: no paint progress samples after its action")
            return None
        began = min(candidate_values) <= 0.01
        completion_index = next((index for index, value in enumerate(candidate_values) if value >= 0.99), None)
        completion = candidates[completion_index] if completion_index is not None else None
        completion_source = "paint" if completion is not None else None
        phase = candidates[:completion_index + 1] if completion_index is not None else candidates
    else:
        absent = [
            event for event in absent_events
            if event.get("t_ms", -1) >= action["t_ms"]
            and (end_ms is None or event["t_ms"] <= end_ms)
            and event.get("phase") == "absent"
            and event.get("ending") is True
        ]
        manager_removed = [
            event for event in removed_events
            if event.get("t_ms", -1) >= action["t_ms"]
            and (end_ms is None or event["t_ms"] <= end_ms)
            and event.get("effective_reduced") is True
        ]
        endpoints = [
            (event, "paint") for event in candidates
            if float(event[key]) <= 0.01
        ] + [(event, "presence-absent") for event in absent] + [
            (event, "manager-removed") for event in manager_removed
        ]
        endpoints.sort(key=lambda item: (item[0]["t_ms"], item[1] != "paint"))
        completion, completion_source = endpoints[0] if endpoints else (None, None)
        phase = [event for event in candidates if completion is None or event["t_ms"] <= completion["t_ms"]]
        if not phase and (absent or manager_removed):
            # Reduced motion may move directly from the last visible state to
            # Presence Absent without an intermediate ending paint. Keep that
            # earlier visible paint as a reference; never relabel it as exit paint.
            phase = [
                event for event in samples
                if event["t_ms"] < action["t_ms"] and event.get("ending") is False
            ][-1:]
        began = max((
            float(event[key]) if event.get(key) is not None
            else float(event.get("enter_progress", 0.))
            for event in phase
        ), default=0.) >= 0.99

    completed = completion is not None
    values = [float(event[key]) for event in phase if event.get(key) is not None]
    terminal_observations = phase + (
        [completion]
        if completion_source in ("presence-absent", "manager-removed")
        else []
    )
    if not all(event.get("active") is True for event in terminal_observations):
        errors.append(f"{label}: one or more phase observations were captured while inactive")
    effective_values = {event.get("effective_reduced") for event in terminal_observations}
    if len(effective_values) > 1:
        errors.append(f"{label}: effective reduced-motion policy changed during the phase")
    effective_reduced = effective_values == {True}
    if not phase:
        errors.append(f"{label}: no progress paint or prior visible endpoint was observed")
        return None
    first = phase[0]
    if not completed:
        errors.append(f"{label}: no completed paint endpoint or Presence Absent event was observed")
    if not effective_reduced and not began:
        errors.append(f"{label}: capture missed its starting endpoint; timing cannot be verified")
    if label != "enter" and not effective_reduced and not candidates:
        errors.append(f"{label}: no post-action exit paint sample; timing cannot be verified")
    if effective_reduced and any(0.01 < value < 0.99 for value in values):
        errors.append(f"{label}: reduced-motion paint included an intermediate progress value")
    if completed:
        previous_samples = [event for event in phase if event["t_ms"] < completion["t_ms"]]
        previous = previous_samples[-1] if previous_samples else first
    else:
        completion = phase[-1]
        previous = phase[-1]
    observed_ms = completion["t_ms"] - first["t_ms"]
    completion_interval_ms = (
        completion["t_ms"] - previous["t_ms"] if completed else None
    )
    if not effective_reduced and completed and completion_source == "paint":
        low, high = (250, 700) if label != "swipe-exit" else (120, 400)
        if not low <= observed_ms <= high:
            target = "200 ms" if label == "swipe-exit" else "400 ms"
            errors.append(f"{label}: observed paint span {observed_ms:.0f} ms; expected about {target}")
    last_paint_to_absent = None
    if completion_source == "presence-absent":
        last_paint_to_absent = completion["t_ms"] - previous["t_ms"]
        if not effective_reduced:
            low, high = (250, 700) if label != "swipe-exit" else (120, 400)
            if not low <= observed_ms <= high:
                target = "200 ms" if label == "swipe-exit" else "400 ms"
                errors.append(f"{label}: paint-to-absent span {observed_ms:.0f} ms; expected about {target}")
    last_paint_to_removal = None
    if completion_source == "manager-removed":
        last_paint_to_removal = completion["t_ms"] - previous["t_ms"]
    return {
        "action_t_ms": action["t_ms"],
        "first_paint_t_ms": first["t_ms"] if first["t_ms"] >= action["t_ms"] else None,
        "action_to_first_paint_ms": first["t_ms"] - action["t_ms"] if first["t_ms"] >= action["t_ms"] else None,
        "first_reference_to_completion_observation_ms": observed_ms,
        "action_to_completion_observation_ms": completion["t_ms"] - action["t_ms"],
        "completion_source": completion_source,
        "completion_observation": (
            "Presence Absent observed during render; no card paint was submitted at this endpoint"
            if completion_source == "presence-absent"
            else "Toast manager removal observed during render; no card paint was submitted at this endpoint"
            if completion_source == "manager-removed"
            else "card paint submission; not compositor presentation"
            if completion_source == "paint"
            else None
        ),
        "last_paint_to_absent_ms": last_paint_to_absent,
        "last_paint_to_manager_removal_ms": last_paint_to_removal,
        "pre_action_reference_used": first["t_ms"] < action["t_ms"],
        "post_action_paint_count": sum(event["t_ms"] >= action["t_ms"] for event in phase),
        "completion_observation_is_paint_presentation": False,
        "completion_interval_start_t_ms": previous["t_ms"],
        "completion_interval_end_t_ms": completion["t_ms"],
        "completion_sample_interval_ms": completion_interval_ms,
        "sample_count": len(phase),
        "starting_endpoint_observed": began or effective_reduced,
        "completed_endpoint_observed": completed,
        "reduced_motion_settled_endpoint_observed": effective_reduced and completed,
        "effective_reduced_values": sorted(effective_values, key=str),
    }


def analyze(events):
    errors = []
    meta = events[0] if events[0].get("type") == "meta" else None
    if not meta or not all(meta.get(key) for key in ("os", "arch", "scenario")):
        errors.append("first event must be meta with OS, architecture and scenario")
    elif meta.get("schema") != 2:
        errors.append("meta schema 2 is required for explicit Presence Absent exit observations")
    if meta and not all(key in meta for key in (
        "app_requested_reduced", "system_reduced", "effective_reduced"
    )):
        errors.append("meta must distinguish requested, system and effective reduced motion")

    complete_events = [event for event in events if event["type"] == "complete"]
    if len(complete_events) != 1 or events[-1].get("type") != "complete":
        errors.append("trace is truncated or has events after its unique final complete event")
    elif complete_events[0].get("swipe_triggered") is not True:
        errors.append("replay did not complete the requested swipe dismissal")
    for index, event in enumerate(events):
        for field in ("os", "arch", "active", "dpr"):
            if field not in event:
                errors.append(f"event {index + 1} ({event['type']}) is missing {field}")
                break
        if "dpr" in event and not finite_number(event["dpr"]):
            errors.append(f"event {index + 1} has non-finite dpr")
        if "active" in event and not isinstance(event["active"], bool):
            errors.append(f"event {index + 1} has invalid active state")
        if not finite_number(event.get("t_ms")):
            errors.append(f"event {index + 1} has non-finite t_ms")
        if index and finite_number(event.get("t_ms")) and finite_number(events[index - 1].get("t_ms")):
            if event["t_ms"] < events[index - 1]["t_ms"]:
                errors.append(f"t_ms moved backwards at event {index + 1}")
                break

    paints = [event for event in events if event["type"] == "paint"]
    unmounts = [event for event in events if event["type"] == "unmounted"]
    removals = [event for event in events if event["type"] == "removed"]
    for index, event in enumerate(paints, 1):
        for field in ("x", "y", "width", "height", "dpr"):
            if not finite_number(event.get(field)):
                errors.append(f"paint sample {index} has invalid {field}")
                break
        if finite_number(event.get("width")) and finite_number(event.get("height")):
            if event["width"] <= 0 or event["height"] <= 0:
                errors.append(f"paint sample {index} has an empty card boundary")
        for key in ("enter_progress", "exit_progress"):
            value = event.get(key)
            if value is not None and (not finite_number(value) or not 0 <= value <= 1):
                errors.append(f"paint sample {index} has invalid {key}")
        if "effective_reduced" not in event:
            errors.append(f"paint sample {index} is missing effective_reduced")
        elif not isinstance(event["effective_reduced"], bool):
            errors.append(f"paint sample {index} has invalid effective_reduced")
        if not isinstance(event.get("ending"), bool):
            errors.append(f"paint sample {index} has invalid ending state")

    for index, event in enumerate(unmounts, 1):
        if (
            event.get("observation") != "Presence::should_render=false"
            or event.get("phase") != "absent"
            or event.get("ending") is not True
        ):
            errors.append(f"unmounted event {index} is not an ending Presence Absent observation")
        if event.get("progress") != 0.0:
            errors.append(f"unmounted event {index} has invalid absent progress")
        if not isinstance(event.get("effective_reduced"), bool):
            errors.append(f"unmounted event {index} has invalid effective_reduced")
        if any(field in event for field in ("x", "y", "width", "height")):
            errors.append(f"unmounted event {index} must not claim paint geometry")
        last_paint_t = event.get("last_paint_t_ms")
        if last_paint_t is not None:
            if not finite_number(last_paint_t) or last_paint_t > event["t_ms"]:
                errors.append(f"unmounted event {index} has invalid last_paint_t_ms")
            elif not any(
                paint.get("id") == event.get("id") and paint["t_ms"] == last_paint_t
                for paint in paints
            ):
                errors.append(f"unmounted event {index} last_paint_t_ms does not match a paint event")
            else:
                previous_paints = [
                    paint["t_ms"] for paint in paints
                    if paint.get("id") == event.get("id") and paint["t_ms"] <= event["t_ms"]
                ]
                if previous_paints and last_paint_t != max(previous_paints):
                    errors.append(f"unmounted event {index} does not reference the latest prior paint")

    for index, event in enumerate(removals, 1):
        if event.get("observation") != "ToastLayer render without managed toast":
            errors.append(f"removed event {index} has an invalid observation")
        if not isinstance(event.get("effective_reduced"), bool):
            errors.append(f"removed event {index} has invalid effective_reduced")
        if any(field in event for field in ("x", "y", "width", "height", "phase", "progress")):
            errors.append(f"removed event {index} must not claim Presence state or paint geometry")
        if event.get("os") != (meta.get("os") if meta else None):
            errors.append(f"removed event {index} OS does not match trace metadata")
        if event.get("arch") != (meta.get("arch") if meta else None):
            errors.append(f"removed event {index} architecture does not match trace metadata")
        last_paint_t = event.get("last_paint_t_ms")
        prior_paints = [
            paint for paint in paints
            if paint.get("id") == event.get("id") and paint["t_ms"] <= event["t_ms"]
        ]
        if not prior_paints or not finite_number(last_paint_t):
            errors.append(f"removed event {index} must reference a prior paint for the same toast")
        elif last_paint_t != max(paint["t_ms"] for paint in prior_paints):
            errors.append(f"removed event {index} does not reference the latest prior paint")
        else:
            matching_paint = next(
                paint for paint in prior_paints if paint["t_ms"] == last_paint_t
            )
            if matching_paint.get("effective_reduced") is not event.get("effective_reduced"):
                errors.append(f"removed event {index} reduced-motion state differs from its last paint")

    actions = [event for event in events if event["type"] == "action"]
    replay_names = (
        "enqueue-enter", "dismiss", "enqueue-swipe", "programmatic-swipe-release"
    )
    replay = {name: find_action(actions, name) for name in replay_names}
    for name, event in replay.items():
        if event is None:
            errors.append(f"replay is missing a unique {name} action")
        elif event.get("active") is not True:
            errors.append(f"replay action {name} was not issued while the window was active")
    if any(event.get("action") == "swipe-skipped" for event in actions):
        errors.append("swipe replay was skipped because no real card paint bounds were available")
    if replay["enqueue-enter"] and replay["dismiss"]:
        if replay["enqueue-enter"].get("id") != replay["dismiss"].get("id"):
            errors.append("normal dismiss action id does not match its entered toast")
    if replay["enqueue-swipe"] and replay["programmatic-swipe-release"]:
        if replay["enqueue-swipe"].get("id") != replay["programmatic-swipe-release"].get("id"):
            errors.append("swipe release action id does not match its entered toast")

    phases = {}
    if replay["enqueue-enter"]:
        toast_id = replay["enqueue-enter"].get("id")
        phases["enter"] = phase_metrics(
            [event for event in paints if event.get("id") == toast_id],
            [],
            [],
            replay["enqueue-enter"], "enter_progress", "enter", errors,
            replay["dismiss"].get("t_ms") if replay["dismiss"] else None,
        )
    if replay["dismiss"]:
        toast_id = replay["dismiss"].get("id")
        next_action = replay["enqueue-swipe"]
        phases["normal_exit"] = phase_metrics(
            [event for event in paints if event.get("id") == toast_id],
            [event for event in unmounts if event.get("id") == toast_id],
            [event for event in removals if event.get("id") == toast_id],
            replay["dismiss"], "exit_progress", "normal-exit", errors,
            next_action.get("t_ms") if next_action else None,
        )
    if replay["programmatic-swipe-release"]:
        toast_id = replay["programmatic-swipe-release"].get("id")
        phases["swipe_exit"] = phase_metrics(
            [event for event in paints if event.get("id") == toast_id],
            [event for event in unmounts if event.get("id") == toast_id],
            [event for event in removals if event.get("id") == toast_id],
            replay["programmatic-swipe-release"], "exit_progress",
            "swipe-exit", errors,
            complete_events[0].get("t_ms") if complete_events else None,
        )

    receipt = {
        "schema": 2,
        "valid": not errors,
        "scenario": meta.get("scenario") if meta else None,
        "os": meta.get("os") if meta else None,
        "arch": meta.get("arch") if meta else None,
        "app_requested_reduced": meta.get("app_requested_reduced") if meta else None,
        "system_reduced": meta.get("system_reduced") if meta else None,
        "effective_reduced_at_start": meta.get("effective_reduced") if meta else None,
        "paint_count": len(paints),
        "unmounted_count": len(unmounts),
        "manager_removed_count": len(removals),
        "toast_ids": sorted({event.get("id") for event in paints if isinstance(event.get("id"), int)}),
        "phases": phases,
        "errors": errors,
    }
    return receipt


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("trace", type=Path)
    parser.add_argument("--output-dir", type=Path)
    args = parser.parse_args()
    try:
        events = read_trace(args.trace)
    except (OSError, ValueError) as error:
        print(f"ERROR: {error}", file=sys.stderr)
        return 2

    receipt = analyze(events)
    receipt["trace_path"] = str(args.trace.resolve())
    receipt["trace_sha256"] = hashlib.sha256(args.trace.read_bytes()).hexdigest()
    if args.output_dir:
        args.output_dir.mkdir(parents=True, exist_ok=True)
        output = args.output_dir / "toast-motion-analysis.json"
        output.write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        receipt["receipt_path"] = str(output)
    print(json.dumps(receipt, indent=2, sort_keys=True))
    return 0 if receipt["valid"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
