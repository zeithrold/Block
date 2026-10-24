#!/usr/bin/env python3
"""Summarize the debug-only native Home/frame replay, stopping at its receipt.

Run Block with BLOCK_FRAME_MOTION_TRACE=<file> and --scenario home-ready.
Raise the disposable window to start the replay. These are paint submissions,
not compositor presentation timestamps. Only geometry/tab/pose data is logged.
"""

import argparse
import html
import json
import math
from pathlib import Path
from statistics import median

from motion_trace import geometry, paint_groups, replay, stage_order


def analyze(records):
    records, actions, replay_paints = replay(records)
    paints = [row for row in records if row["type"] == "paint"]
    origins, heights = geometry(replay_paints)
    groups = list(paint_groups(records[actions[0][0]:]))
    details = {id(frame): [probe for probe in probes if probe["type"] == "pane"
                          and probe["part"] == "detail" and not probe["outgoing"]]
               for frame, probes in groups}
    if any(not all(math.isfinite(pane[key]) for key in ("x", "y", "width", "height"))
           for panes in details.values() for pane in panes):
        raise ValueError("Non-finite current detail pane bounds")
    # Measure the right content inset from settled management panes, instead of
    # assuming a particular OS's client decorations. Ignore pending native
    # resize acknowledgements whose viewport still clips the settled pane.
    right_insets = [frame["viewport_width"] - (pane["x"] + pane["width"])
                    for frame, _ in groups if not frame["home"] and not frame["running"]
                    for pane in details[id(frame)]
                    if frame["viewport_width"] >= pane["x"] + pane["width"] - .01]
    if not right_insets:
        raise ValueError("Missing settled management pane bounds for viewport clipping")
    right_inset = median(right_insets)

    def detail_visible(row):
        if row["detail_opacity"] <= .001:
            return False
        panes = details.get(id(row), [])
        if not panes:
            raise ValueError("Missing current detail bounds during forward motion")
        return any(min(pane["x"] + pane["width"], row["viewport_width"] - right_inset)
                   - max(pane["x"], row["body_x"]) > .01 for pane in panes)

    sequences = []
    for n, (index, action) in enumerate(actions):
        end = actions[n + 1][0] if n + 1 < len(actions) else len(records)
        frames = [row for row in records[index:end] if row["type"] == "paint"]
        if not frames:
            raise ValueError(f"No painted frames for {action}")

        def first(predicate):
            row = next((row for row in frames if predicate(row)), None)
            return round(row["t_ms"] - action["t_ms"], 2) if row else None

        forward = action["tab"] != "Home"
        entry = {
            "action": action["tab"], "scheduled_ms": action["scheduled_ms"],
            "paint_count": len(frames),
            "sidebar_visible_ms": first(lambda row: row["sidebar_opacity"] > .001) if forward else None,
            "native_width_change_ms": first(lambda row: abs(row["native_width"] - frames[0]["native_width"]) > 1),
            "detail_opacity_started_ms": first(lambda row: row["detail_opacity"] > .001) if forward else None,
            "detail_visible_ms": first(detail_visible) if forward else None,
            "settled_ms": first(lambda row: not row["running"]),
            "final_width": frames[-1]["native_width"],
        }
        if forward and entry["settled_ms"] is not None:
            entry.update(stage_order(frames, action, {
                "sidebar": lambda row: row["sidebar_opacity"] > .001,
                "width": lambda row: abs(row["native_width"] - frames[0]["native_width"]) > 1,
                "detail": detail_visible,
            }))
            if any(value is None for value in entry["stage_start_intervals_ms"].values()):
                raise ValueError(f"Completed forward track did not paint every stage: {entry}")
        sequences.append(entry)
    return {
        "measurement": "native paint submission, not display presentation",
        "paint_count": len(paints), "foreground_replay_paint_count": len(replay_paints), "native_origins": origins, "native_heights": heights,
        "startup_paint_count": len(paints) - len(replay_paints),
        "detail_visibility_basis": "current detail paint bounds intersect the observed content viewport",
        "right_content_inset": right_inset,
        "native_width_range": [min(row["native_width"] for row in replay_paints), max(row["native_width"] for row in replay_paints)],
        "sidebar_inside_border_widths": sorted({row["width"] for row in records[actions[0][0]:] if row["type"] == "pane" and row["part"] == "sidebar"}),
        "sampling_limited_sequence_count": sum(row.get("stage_order") == "sampling-limited" for row in sequences),
        "sequences": sequences,
    }


def report(result):
    def value(item):
        return "—" if item is None else str(item)

    rows = "".join(
        "<tr>" + "".join(f"<td>{html.escape(value(row[key]))}</td>" for key in (
            "scheduled_ms", "action", "paint_count", "sidebar_visible_ms",
            "native_width_change_ms", "detail_visible_ms", "settled_ms", "final_width"))
        + f"<td>{html.escape(row.get('stage_order', 'interrupted / reverse'))}</td></tr>"
        for row in result["sequences"]
    )
    return f"""<!doctype html><html lang="en"><meta charset="utf-8">
<title>Home / management native frame measurements</title>
<style>body{{font:15px system-ui;margin:32px;line-height:1.5;color:#181818}}table{{border-collapse:collapse}}th,td{{padding:8px 12px;border:1px solid #ddd;text-align:left}}img{{display:block;max-width:100%;border:1px solid #ddd;margin:12px 0 28px}}code{{background:#f5f5f5;padding:3px}}</style>
<h1>Home / management frame</h1>
<p>{result['foreground_replay_paint_count']} continuously active-window paint submissions across ten navigation actions;
{result['startup_paint_count']} startup paints excluded from replay geometry.
Fixed native origin <code>{result['native_origins']}</code>; fixed height <code>{result['native_heights']}</code>;
native width range <code>{result['native_width_range']}</code>.</p>
<p>Times are milliseconds after the dispatched action. A dash means the track did not become visible before interruption, or the column does not describe that reverse track.
These samples measure native paint submission, not compositor presentation. See <a href="measurement.json">measurement.json</a> for the per-stage observation intervals.
{result['sampling_limited_sequence_count']} forward sequences have starts within the same paint interval; their exact stagger is unresolved at this sampling cadence.</p>
<table><tr><th>Scheduled</th><th>Destination</th><th>Paints</th><th>Sidebar visible</th><th>Native width changes</th><th>Detail visible</th><th>Settled</th><th>Final width</th><th>Stage order observation</th></tr>{rows}</table>
<p>Disposable synthetic data; no live sign-in or game operation. Display fitting and decoration insets affect native sizes. These measurements do not establish compositor smoothness or user acceptance.</p></html>"""


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("trace", type=Path)
    parser.add_argument("--output-dir", type=Path)
    args = parser.parse_args()
    result = analyze([json.loads(line) for line in args.trace.read_text().splitlines()])
    output = args.output_dir or args.trace.parent
    output.mkdir(parents=True, exist_ok=True)
    (output / "measurement.json").write_text(json.dumps(result, indent=2) + "\n")
    (output / "report.html").write_text(report(result))
    print(f"{result['foreground_replay_paint_count']} foreground replay paints: origin/height stable; "
          f"{result['sampling_limited_sequence_count']} stage orders unresolved at paint cadence")


if __name__ == "__main__":
    main()
