#!/usr/bin/env python3
"""Check the debug native pane replay's scope, distance and start-time ordering.

Use BLOCK_FRAME_MOTION_TRACE=<file> BLOCK_PANE_MOTION_REPLAY=1 and
--scenario home-ready. Keep the window active for the short replay.
"""
import argparse
import json
from pathlib import Path


def analyze(records):
    end = next((i for i, row in enumerate(records) if row["type"] == "complete"), None)
    if end is None:
        raise ValueError("The pane replay has not completed")
    records = records[:end + 1]
    actions = [(i, row) for i, row in enumerate(records) if row["type"] == "action"]
    paints = [row for row in records[actions[0][0]:] if row["type"] == "paint"]
    if len(actions) != 10 or not paints or not all(row["active"] for row in paints):
        raise ValueError("Expected ten actions painted continuously in the foreground")
    origins = sorted({(row["native_x"], row["native_y"]) for row in paints})
    heights = sorted({row["native_height"] for row in paints})
    if len(origins) != 1 or len(heights) != 1:
        raise ValueError("Native origin/height changed during pane navigation")
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
            for track in [*navigation["panes"].values(), navigation["detail"], navigation["wizard"]]:
                for layer in ("current", "outgoing"):
                    pose = track[layer]
                    if pose and (abs(pose["x"]) > 12.001 or not 0 <= pose["opacity"] <= 1):
                        raise ValueError(f"Unbounded pane motion: {pose}")
            if child:
                sidebar = navigation["panes"]["sidebar"]
                if sidebar["running"] or sidebar["outgoing"] or sidebar["current"] != {"x": 0.0, "opacity": 1.0}:
                    raise ValueError("A child-page selection moved the sidebar")
        pane_samples = [row for row in records[index:stop] if row["type"] == "pane" and not row["outgoing"]]
        if any(abs(row["y"] - frames[0]["body_y"]) > .01 for row in pane_samples):
            raise ValueError("Management panes moved vertically")
        if child:
            fragments = [row for row in records[index:stop] if row["type"] == "fragment" and row["scope"] == 1]
            if not fragments or any(abs(row["x"] - 360) > 12.001 or abs(row["y"] - frames[0]["body_y"]) > .01 for row in fragments):
                raise ValueError("Child content did not keep a small horizontal-only displacement")

        def first(predicate):
            row = next((row for row in frames if predicate(row)), None)
            return round(row["t_ms"] - action["t_ms"], 2) if row else None

        result = {"destination": action["tab"], "scope": "detail" if child else "page", "paint_count": len(frames)}
        if child:
            result["settled_ms"] = first(lambda row: not row["navigation"]["detail"]["running"])
        else:
            for track in ("sidebar", "detail"):
                result[track + "_visible_ms"] = first(lambda row: row["navigation"]["panes"][track]["current"]["opacity"] > .001)
            if result["detail_visible_ms"] is not None and not result["sidebar_visible_ms"] < result["detail_visible_ms"]:
                raise ValueError("Sidebar and content did not start in order")
            result["settled_ms"] = first(lambda row: not row["navigation"]["panes"]["detail"]["running"])
        summaries.append(result)
    if sum(row["scope"] == "detail" for row in summaries) != 2:
        raise ValueError("Both child-page selections must be measured")
    return {"measurement": "native paint submission, not display presentation", "foreground_paint_count": len(paints),
            "native_origins": origins, "native_heights": heights, "maximum_horizontal_distance": 12,
            "page_detail_start_delay_ms": 40, "sequences": summaries}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("trace", type=Path)
    args = parser.parse_args()
    result = analyze([json.loads(line) for line in args.trace.read_text().splitlines()])
    output = args.trace.parent / "measurement.json"
    output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
