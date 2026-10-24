"""Shared validation for native motion paint traces, independent of the OS."""

import math


# The motion primitives use f32 interpolation. One rounding step near 1.0 must
# not turn a valid opacity into a failure; larger overshoots still fail.
OPACITY_EPSILON = 1e-6
POSITION_EPSILON = .001


def replay(records):
    complete = next((i for i, row in enumerate(records) if row["type"] == "complete"), None)
    if complete is None:
        raise ValueError("The native replay has not completed")
    records = records[:complete + 1]
    actions = [(i, row) for i, row in enumerate(records) if row["type"] == "action"]
    if len(actions) != 10:
        raise ValueError("Expected all ten replay actions")
    paints = [row for row in records[actions[0][0]:] if row["type"] == "paint"]
    if not paints or not all(row["active"] for row in paints):
        raise ValueError("Expected replay paints continuously in the foreground")
    # Window managers may configure the requested client bounds into decorated
    # outer bounds during startup. Only the action-to-complete interval measures
    # the replay; geometry changes or lost activation inside it still fail.
    for row in paints:
        if not all(math.isfinite(row[key]) for key in (
                "t_ms", "native_x", "native_y", "native_width", "native_height",
                "body_x", "body_y", "viewport_width", "viewport_height")):
            raise ValueError("Non-finite replay geometry or timestamp")
    if any(left["t_ms"] > right["t_ms"] for left, right in zip(paints, paints[1:])):
        raise ValueError("Replay paint timestamps moved backwards")
    return records, actions, paints


def geometry(paints):
    origins = sorted({(row["native_x"], row["native_y"]) for row in paints})
    heights = sorted({row["native_height"] for row in paints})
    if len(origins) != 1 or len(heights) != 1:
        raise ValueError(f"Origin/height changed during replay: {origins}, {heights}")
    return origins, heights


def bounded_pose(pose, distance):
    return (
        math.isfinite(pose["x"]) and math.isfinite(pose["opacity"])
        and abs(pose["x"]) <= distance + POSITION_EPSILON
        and -OPACITY_EPSILON <= pose["opacity"] <= 1 + OPACITY_EPSILON
    )


def stage_order(frames, action, predicates):
    """Reject inversions; retain same-paint starts as unresolved observations.

    A 40 ms stagger can fall entirely between paints on a slow host. Equal
    observations do not prove simultaneous starts or the intended stagger.
    Report the sampling interval instead of either rejecting the host or
    claiming that the exact start order was verified.
    """
    indices = {name: next((i for i, row in enumerate(frames) if predicate(row)), None)
               for name, predicate in predicates.items()}
    visible = [(name, i) for name, i in indices.items() if i is not None]
    if any(left > right for (_, left), (_, right) in zip(visible, visible[1:])):
        raise ValueError(f"Stages painted in reverse order: {indices}")
    ties = [[left_name, right_name] for (left_name, left), (right_name, right)
            in zip(visible, visible[1:]) if left == right]
    intervals = {}
    for name, index in indices.items():
        intervals[name] = None if index is None else [
            round((frames[index - 1]["t_ms"] if index else action["t_ms"]) - action["t_ms"], 2),
            round(frames[index]["t_ms"] - action["t_ms"], 2),
        ]
    return {
        "stage_order": "sampling-limited" if ties else "observed" if all(
            index is not None for index in indices.values()) else "interrupted",
        "unresolved_stage_pairs": ties,
        "stage_start_intervals_ms": intervals,
    }


def paint_groups(records):
    """Associate child probes with the enclosing body paint in the same pass."""
    probes = []
    for row in records:
        if row["type"] in ("pane", "fragment"):
            probes.append(row)
        elif row["type"] == "paint":
            yield row, probes
            probes = []
