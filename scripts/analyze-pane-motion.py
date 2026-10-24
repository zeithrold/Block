#!/usr/bin/env python3
"""Check the debug native pane replay's scope, distance and start-time ordering.

Use BLOCK_FRAME_MOTION_TRACE=<file> BLOCK_PANE_MOTION_REPLAY=1 and
--scenario home-ready. Keep the window active for the short replay.
"""
import argparse
import json
import math
from pathlib import Path

from motion_trace import bounded_pose, geometry, paint_groups, replay, stage_order


def analyze(records):
    records, actions, paints = replay(records)
    origins, heights = geometry(paints)
    summaries = []
    for n, (index, action) in enumerate(actions):
        stop = actions[n + 1][0] if n + 1 < len(actions) else len(records)
        frames = [row for row in records[index:stop] if row["type"] == "paint"]
        if not frames:
            raise ValueError(f"Missing paints for {action}")
        child = action["tab"].startswith("SettingsPage")
        page = any(row["navigation"]["panes"]["detail"]["running"] for row in frames)
        if not (page or child):
            continue  # Home's independently measured window morph.
        widths = sorted({row["native_width"] for row in frames})
        if len(widths) != 1:
            raise ValueError("Management navigation resized the window")
        for row in frames:
            navigation = row["navigation"]
            tracks = [(navigation["panes"]["sidebar"], 12), (navigation["panes"]["detail"], 20),
                      (navigation["detail"], 20), (navigation["wizard"], 20)]
            for track, distance in tracks:
                for layer in ("current", "outgoing"):
                    pose = track[layer]
                    if pose and not bounded_pose(pose, distance):
                        raise ValueError(f"Unbounded pane motion: {pose}")
            if child:
                sidebar = navigation["panes"]["sidebar"]
                pose = sidebar["current"]
                if (sidebar["running"] or sidebar["outgoing"] or pose is None
                        or not bounded_pose(pose, 0) or abs(pose["opacity"] - 1) > 1e-6):
                    raise ValueError("A child-page selection moved the sidebar")
        if len({(row["body_x"], row["body_y"]) for row in frames}) != 1:
            raise ValueError("The body origin changed during pane navigation")
        fragments = []
        for frame, probes in paint_groups(records[index:stop]):
            pane_samples = [row for row in probes if row["type"] == "pane" and not row["outgoing"]]
            if any(not math.isfinite(row["y"]) or abs(row["y"] - frame["body_y"]) > .01 for row in pane_samples):
                raise ValueError("Management panes moved vertically")
            if child:
                current = [row for row in probes if row["type"] == "fragment" and row["scope"] == 1]
                # The sidebar is 360 logical px including its border. body_x
                # supplies the measured content inset (e.g. 21 px on X11),
                # independent of the native window origin or DPI scale.
                if any(not all(math.isfinite(row[key]) for key in ("x", "y"))
                       or abs(row["x"] - (frame["body_x"] + 360)) > 20.001
                       or abs(row["y"] - frame["body_y"]) > .01 for row in current):
                    raise ValueError("Child content did not keep a small horizontal-only displacement")
                fragments.extend(current)
        if child and not fragments:
            raise ValueError("Missing child content paint measurements")

        def first(predicate):
            row = next((row for row in frames if predicate(row)), None)
            return round(row["t_ms"] - action["t_ms"], 2) if row else None

        result = {"destination": action["tab"], "scope": "detail" if child else "page", "paint_count": len(frames)}
        if child:
            result["settled_ms"] = first(lambda row: not row["navigation"]["detail"]["running"])
        else:
            for track in ("sidebar", "detail"):
                result[track + "_visible_ms"] = first(lambda row: row["navigation"]["panes"][track]["current"] is not None
                                                     and row["navigation"]["panes"][track]["current"]["opacity"] > .001)
            if result["detail_visible_ms"] is not None and result["sidebar_visible_ms"] is None:
                raise ValueError("Content painted without the sidebar")
            result.update(stage_order(frames, action, {
                track: lambda row, track=track: row["navigation"]["panes"][track]["current"] is not None
                and row["navigation"]["panes"][track]["current"]["opacity"] > .001
                for track in ("sidebar", "detail")
            }))
            result["settled_ms"] = first(lambda row: not row["navigation"]["panes"]["detail"]["running"])
        summaries.append(result)
    if sum(row["scope"] == "detail" for row in summaries) != 2:
        raise ValueError("Both child-page selections must be measured")
    return {"measurement": "native paint submission, not display presentation", "foreground_paint_count": len(paints),
            "native_origins": origins, "native_heights": heights, "maximum_sidebar_distance": 12,
            "maximum_content_distance": 20,
            "body_origins": sorted({(row["body_x"], row["body_y"]) for row in paints}),
            "opacity_epsilon": 1e-6,
            "sampling_limited_sequence_count": sum(row.get("stage_order") == "sampling-limited" for row in summaries),
            "page_detail_start_delay_ms": 40, "sequences": summaries}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("trace", type=Path)
    parser.add_argument("--output-dir", type=Path)
    args = parser.parse_args()
    result = analyze([json.loads(line) for line in args.trace.read_text().splitlines()])
    output = args.output_dir or args.trace.parent
    output.mkdir(parents=True, exist_ok=True)
    (output / "measurement.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
