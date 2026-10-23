#!/usr/bin/env python3
"""Summarize the debug-only native Home/frame replay, stopping at its receipt.

Run Block with BLOCK_FRAME_MOTION_TRACE=<file> and --scenario home-ready.
Raise the disposable window to start the replay. These are paint submissions,
not compositor presentation timestamps. Only geometry/tab/pose data is logged.
"""

import argparse
import html
import json
from pathlib import Path


def analyze(records):
    complete = next((i for i, row in enumerate(records) if row["type"] == "complete"), None)
    if complete is None:
        raise ValueError("The native replay has not completed")
    records = records[:complete + 1]
    paints = [row for row in records if row["type"] == "paint"]
    actions = [(i, row) for i, row in enumerate(records) if row["type"] == "action"]
    if not paints or len(actions) != 10:
        raise ValueError("Expected native paints and all ten replay actions")
    replay_paints = [row for row in records[actions[0][0]:] if row["type"] == "paint"]
    if not all(row["active"] for row in replay_paints):
        raise ValueError("The replay lost window activation; repeat in the foreground before comparing animation phases")
    origins = sorted({(row["native_x"], row["native_y"]) for row in paints})
    heights = sorted({row["native_height"] for row in paints})
    if len(origins) != 1 or len(heights) != 1:
        raise ValueError(f"Origin/height changed during replay: {origins}, {heights}")
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
            "detail_visible_ms": first(lambda row: row["detail_opacity"] > .001) if forward else None,
            "settled_ms": first(lambda row: not row["running"]),
            "final_width": frames[-1]["native_width"],
        }
        if forward and entry["settled_ms"] is not None:
            stages = [entry[key] for key in ("sidebar_visible_ms", "native_width_change_ms", "detail_visible_ms")]
            if any(value is None for value in stages) or not stages[0] < stages[1] < stages[2]:
                raise ValueError(f"Forward stages did not paint in order: {entry}")
        sequences.append(entry)
    return {
        "measurement": "native paint submission, not display presentation",
        "paint_count": len(paints), "foreground_replay_paint_count": len(replay_paints), "native_origins": origins, "native_heights": heights,
        "native_width_range": [min(row["native_width"] for row in paints), max(row["native_width"] for row in paints)],
        "sidebar_inside_border_widths": sorted({row["width"] for row in records if row["type"] == "pane" and row["part"] == "sidebar"}),
        "sequences": sequences,
    }


def report(result):
    def value(item):
        return "—" if item is None else str(item)

    rows = "".join(
        "<tr>" + "".join(f"<td>{html.escape(value(row[key]))}</td>" for key in (
            "scheduled_ms", "action", "paint_count", "sidebar_visible_ms",
            "native_width_change_ms", "detail_visible_ms", "settled_ms", "final_width")) + "</tr>"
        for row in result["sequences"]
    )
    return f"""<!doctype html><html lang="en"><meta charset="utf-8">
<title>Home / management native frame measurements</title>
<style>body{{font:15px system-ui;margin:32px;line-height:1.5;color:#181818}}table{{border-collapse:collapse}}th,td{{padding:8px 12px;border:1px solid #ddd;text-align:left}}img{{display:block;max-width:100%;border:1px solid #ddd;margin:12px 0 28px}}code{{background:#f5f5f5;padding:3px}}</style>
<h1>Home / management frame</h1>
<p>{result['paint_count']} native paint submissions; {result['foreground_replay_paint_count']} continuously active-window paints across ten navigation actions.
Fixed native origin <code>{result['native_origins']}</code>; fixed height <code>{result['native_heights']}</code>;
native width range <code>{result['native_width_range']}</code>.</p>
<p>Times are milliseconds after the dispatched action. A dash means the track did not become visible before interruption, or the column does not describe that reverse track.
These samples measure native paint submission, not compositor presentation. See <a href="measurement.json">measurement.json</a> and <a href="native.jsonl">raw replay</a>.</p>
<table><tr><th>Scheduled</th><th>Destination</th><th>Paints</th><th>Sidebar visible</th><th>Native width changes</th><th>Detail visible</th><th>Settled</th><th>Final width</th></tr>{rows}</table>
<h2>Home: 360 × 720</h2><img src="home-light.jpg" alt="Compact Home">
<h2>Accounts: 1152 × 720, sidebar 360</h2><img src="accounts-light.jpg" alt="Accounts management">
<h2>Settings: same frame</h2><img src="settings-light.jpg" alt="Settings management">
<p>Disposable synthetic data; no live sign-in or game operation. User acceptance and physical Windows/Linux frame measurements remain pending.</p></html>"""


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
    print(f"{result['paint_count']} paints ({result['foreground_replay_paint_count']} foreground replay paints): origin/height stable; completed forward tracks paint in order")


if __name__ == "__main__":
    main()
