#!/usr/bin/env python3
"""Build semantic visual-review manifests and boards from captured PNGs.

Filename equality is only a way to locate a candidate image. It never establishes
semantic equivalence. Optional ``<image-stem>.capture.json`` receipts add capture
facts without changing the curated case semantics.
"""

from __future__ import annotations

import argparse
import hashlib
import html
import json
import re
import struct
from collections import defaultdict
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parent.parent
OUTPUT = ROOT / "target/parity-shots"
REGISTRY = ROOT / "docs/visual-review-cases.json"
MANIFEST = OUTPUT / "m2-semantic.json"
MAPPING = OUTPUT / "m2-mapping.md"
REQUIRED_FIELDS = {
    "case_id", "native_stems", "reference_candidates", "review_status", "object",
    "trigger", "data", "viewport", "focus", "scroll", "source_provenance",
    "comparable_scope", "rationale",
}


def load_registry(path: Path = REGISTRY) -> dict[str, Any]:
    registry = json.loads(path.read_text())
    seen: set[str] = set()
    for case in registry.get("cases", []):
        missing = REQUIRED_FIELDS - case.keys()
        if missing:
            raise ValueError(f"{case.get('case_id', '<unknown>')} missing {sorted(missing)}")
        if case["case_id"] in seen:
            raise ValueError(f"duplicate case_id: {case['case_id']}")
        seen.add(case["case_id"])
        if case["review_status"] == "unreviewed" and case["comparable_scope"] != "not_assessed":
            raise ValueError(f"unreviewed case {case['case_id']} cannot claim comparison scope")
    return registry


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def image_width(path: Path) -> int:
    with path.open("rb") as stream:
        header = stream.read(24)
    if len(header) != 24 or header[:8] != b"\x89PNG\r\n\x1a\n":
        return 0
    return struct.unpack(">I", header[16:20])[0]


def image_size(path: Path) -> dict[str, int]:
    with path.open("rb") as stream:
        header = stream.read(24)
    if len(header) != 24 or header[:8] != b"\x89PNG\r\n\x1a\n":
        return {"width": 0, "height": 0}
    width, height = struct.unpack(">II", header[16:24])
    return {"width": width, "height": height}


def scenario_names(source: Path = ROOT / "crates/block-ui/src/scenarios.rs") -> set[str]:
    match = re.search(r"pub const SCENARIOS: &\[&str\] = &\[(.*?)\n\];", source.read_text(), re.S)
    if not match:
        raise ValueError(f"could not read SCENARIOS from {source}")
    return set(re.findall(r'"([a-z0-9-]+)"', match.group(1)))


def image_path(folder: Path, stem: str, theme: str) -> Path | None:
    path = folder / f"{stem}-{theme}.png"
    return path if path.is_file() else None


def receipt_for(path: Path | None) -> dict[str, Any] | None:
    if path is None:
        return None
    receipt = path.with_suffix(".capture.json")
    appended = Path(str(path) + ".capture.json")
    if not receipt.is_file() and appended.is_file():
        receipt = appended
    if not receipt.is_file():
        return None
    value = json.loads(receipt.read_text())
    if value.get("format") != "block-ui-capture-receipt/v1" and value.get("schema_version") != 1:
        raise ValueError(f"unsupported capture receipt format: {receipt}")
    recorded_hash = value.get("sha256")
    if recorded_hash and recorded_hash != sha256(path):
        raise ValueError(f"capture receipt hash does not match image: {receipt}")
    return value


def collect_images(registry: dict[str, Any], platform: str) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    native_dir = OUTPUT / ("native" if platform == "macos-arm64" else f"{platform}/scenarios")
    proto_dir = OUTPUT / "proto"
    image_index: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for role, folder in (("native", native_dir), ("reference", proto_dir)):
        for path in sorted(folder.glob("*.png")) if folder.exists() else []:
            record = {
                "role": role,
                "path": str(path.relative_to(OUTPUT)),
                "stem": path.stem,
                "sha256": sha256(path),
                "size_px": image_size(path),
                "capture_receipt": receipt_for(path),
            }
            image_index[record["sha256"]].append(record)
    reuse_groups = []
    for digest, records in sorted(image_index.items()):
        if len(records) > 1:
            group_id = f"sha256:{digest[:16]}"
            reuse_groups.append({"reuse_group": group_id, "sha256": digest, "artifacts": records})
            for record in records:
                record["reuse_group"] = group_id
    reuse_by_hash = {group["sha256"]: group["reuse_group"] for group in reuse_groups}

    cases = []
    registry_stems = {stem for case in registry["cases"] for stem in case["native_stems"]}
    missing_registry = scenario_names() - registry_stems
    if missing_registry:
        raise ValueError(f"native scenarios missing semantic rows: {sorted(missing_registry)}")
    for curated in registry["cases"]:
        case = dict(curated)
        native = []
        references = []
        for stem in case["native_stems"]:
            for theme in ("light", "dark"):
                path = image_path(native_dir, stem, theme)
                if path:
                    artifact = _artifact(path, "native", theme)
                    if artifact["sha256"] in reuse_by_hash:
                        artifact["reuse_group"] = reuse_by_hash[artifact["sha256"]]
                    if case["case_id"] == "home-error" and not (artifact.get("capture_receipt") or {}).get("assertions", {}).get("passed"):
                        artifact["review_eligibility"] = "EXCLUDED: historical fixture changed status text without a Failed target. Replace it with a current asserted capture before reviewing the failure state."
                    native.append(artifact)
        for stem in case["reference_candidates"]:
            for theme in ("light", "dark"):
                path = image_path(proto_dir, stem, theme)
                if path:
                    artifact = _artifact(path, "reference_candidate", theme)
                    if artifact["sha256"] in reuse_by_hash:
                        artifact["reuse_group"] = reuse_by_hash[artifact["sha256"]]
                    references.append(artifact)
        case["artifacts"] = {"native": native, "reference_candidates": references}
        case["coverage_counts_as_independent"] = case["review_status"] == "semantic_equivalent" and case["comparable_scope"] == "full_state"
        cases.append(case)
    return cases, reuse_groups


def _artifact(path: Path, role: str, theme: str) -> dict[str, Any]:
    return {
        "role": role,
        "theme": theme,
        "path": str(path.relative_to(OUTPUT)),
        "sha256": sha256(path),
        "size_px": image_size(path),
        "capture_receipt": receipt_for(path),
    }


def build_semantic_manifest(platform: str, registry_path: Path = REGISTRY) -> dict[str, Any]:
    registry = load_registry(registry_path)
    cases, reuse_groups = collect_images(registry, platform)
    return {
        "schema_version": registry["schema_version"],
        "generated_from": str(registry_path.relative_to(ROOT)),
        "platform": platform,
        "archive_source_commits": "unknown unless supplied by per-image capture receipts",
        "summary": {
            "native_cases": len(cases),
            "reviewed_semantic_equivalences": sum(c["coverage_counts_as_independent"] for c in cases),
            "unreviewed_cases": sum(c["review_status"] == "unreviewed" for c in cases),
            "exact_byte_reuse_groups": len(reuse_groups),
        },
        "reuse_groups": reuse_groups,
        "cases": cases,
    }


def render_markdown(manifest: dict[str, Any]) -> str:
    lines = [
        "# Milestone II visual review: semantic mapping",
        "",
        f"Platform board: `{manifest['platform']}`. Native case rows: {manifest['summary']['native_cases']}; independently covered full-state pairs: {manifest['summary']['reviewed_semantic_equivalences']}; exact-byte reuse groups: {manifest['summary']['exact_byte_reuse_groups']}.",
        "",
        "Exact filename matches are candidate lookups only. An unreviewed stem is not a parity claim. Historical image commits, focus, and scroll are unknown unless a capture sidecar records them. PNGs were not modified by this generator.",
        "",
        "| Native case | Review | Object / state | Trigger and data | Scope | Native artifacts | Reference candidate artifacts | Reuse |",
        "| --- | --- | --- | --- | --- | --- | --- | --- |",
    ]
    for case in manifest["cases"]:
        obj = case["object"]
        object_text = f"{obj.get('type', 'unknown')}: {obj.get('identity', 'unknown')}"
        trigger = case["trigger"].get("value", "unknown")
        data = case["data"].get("value", "unknown")
        native = ", ".join(a["path"] for a in case["artifacts"]["native"]) or "missing"
        refs = ", ".join(a["path"] for a in case["artifacts"]["reference_candidates"]) or "none"
        reuses = sorted({a.get("reuse_group") for a in case["artifacts"]["native"] + case["artifacts"]["reference_candidates"] if a.get("reuse_group")})
        reuse = ", ".join(reuses) if reuses else "none detected"
        lines.append(f"| `{case['case_id']}` | **{case['review_status']}** | {object_text} | {trigger}; data: {data} | `{case['comparable_scope']}` — {case['rationale']} | {native} | {refs} | {reuse} |")
    lines += ["", "## Capture metadata limits", "", "Viewport values are only reported when a per-image `.capture.json` sidecar exists. Historical focus, scroll, and source commit remain unknown; current source-file references do not prove the source revision used to create an archived PNG.", "", "## Exact-byte reuse", ""]
    for group in manifest["reuse_groups"]:
        paths = ", ".join(f"`{a['path']}`" for a in group["artifacts"])
        lines.append(f"- `{group['reuse_group']}` ({group['sha256']}): {paths}. These artifacts are reused visual evidence, not independent coverage.")
    if not manifest["reuse_groups"]:
        lines.append("- No duplicate hashes found among captured images for this platform and the React reference folder.")
    return "\n".join(lines) + "\n"


def combine_platform_manifests(manifests: dict[str, dict[str, Any]]) -> dict[str, Any]:
    cases_by_id: dict[str, dict[str, Any]] = {}
    for platform, manifest in manifests.items():
        for source_case in manifest["cases"]:
            case = cases_by_id.setdefault(source_case["case_id"], {k: v for k, v in source_case.items() if k not in {"artifacts", "coverage_counts_as_independent"}})
            case.setdefault("artifacts_by_platform", {})[platform] = source_case["artifacts"]
            case["coverage_counts_as_independent"] = False
    # Recompute reuse over the complete set of board artifacts. Per-platform
    # groups miss equality that occurs only across platforms. Reference images
    # are repeated in every machine manifest, so count each distinct artifact
    # path once in the combined inventory.
    artifacts_by_hash: dict[str, dict[str, dict[str, Any]]] = defaultdict(dict)
    hash_by_path: dict[str, str] = {}
    for platform, manifest in manifests.items():
        for source_case in manifest["cases"]:
            for role in ("native", "reference_candidates"):
                for record in source_case["artifacts"].get(role, []):
                    artifact = dict(record, platform=platform)
                    path = artifact["path"]
                    prior_hash = hash_by_path.get(path)
                    if prior_hash and prior_hash != artifact["sha256"]:
                        raise ValueError(f"artifact path has conflicting hashes: {path}")
                    hash_by_path[path] = artifact["sha256"]
                    artifacts_by_hash[artifact["sha256"]].setdefault(path, artifact)
    groups = {}
    for digest, records_by_path in sorted(artifacts_by_hash.items()):
        records = list(records_by_path.values())
        if len(records) > 1:
            groups[digest] = {
                "reuse_group": f"sha256:{digest[:16]}",
                "sha256": digest,
                "artifacts": records,
            }
    ordered_cases = [cases_by_id[k] for k in sorted(cases_by_id)]
    return {
        "schema_version": 1,
        "generated_from": str(REGISTRY.relative_to(ROOT)),
        "platforms": {key: {"board": "index.html" if key == "windows-amd64" else f"{key}.html", "machine_manifest": f"m2-semantic-{key}.json"} for key in manifests},
        "archive_source_commits": "unknown unless supplied by per-image capture receipts",
        "summary": {
            "native_cases": len(ordered_cases),
            "platforms_included": list(manifests),
            "reviewed_semantic_equivalences": 0,
            "unreviewed_cases": sum(c["review_status"] == "unreviewed" for c in ordered_cases),
            "exact_byte_reuse_groups": len(groups),
        },
        "reuse_groups": list(groups.values()),
        "cases": ordered_cases,
    }


def render_cross_platform_markdown(manifest: dict[str, Any]) -> str:
    lines = [
        "# Milestone II visual review: semantic mapping",
        "",
        f"Platforms included: {', '.join(f'`{p}`' for p in manifest['summary']['platforms_included'])}. Native case rows: {manifest['summary']['native_cases']}; full-state semantic pairs accepted: 0; exact-byte reuse groups: {manifest['summary']['exact_byte_reuse_groups']}.",
        "",
        "Exact filename matches are candidate lookups only. Unreviewed candidates are not parity claims. Archived image source commits, focus, and scroll remain unknown unless per-image sidecars supply them. PNGs are not modified by the board generator.",
        "",
        "| Native case | Review | Object / state | Trigger | Data | Viewport | Focus | Scroll | Source provenance | Scope | Native images | Reference candidates |",
        "| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |",
    ]
    for case in manifest["cases"]:
        artifacts = case.get("artifacts_by_platform", {})
        native = [f"{platform}: {a['path']}" for platform, files in artifacts.items() for a in files.get("native", [])]
        refs = sorted({a["path"] for files in artifacts.values() for a in files.get("reference_candidates", [])})
        obj = case["object"]
        viewport = json.dumps(case.get("viewport", {}), ensure_ascii=False, sort_keys=True)
        provenance = json.dumps(case.get("source_provenance", {}), ensure_ascii=False, sort_keys=True)
        lines.append(f"| `{case['case_id']}` | **{case['review_status']}** | {obj.get('type')}: {obj.get('identity')} | {case['trigger'].get('value', 'unknown')} | {case['data'].get('value', 'unknown')} | {viewport} | {case.get('focus', 'unknown')} | {case.get('scroll', 'unknown')} | {provenance} | `{case['comparable_scope']}` — {case['rationale']} | {', '.join(native) or 'missing'} | {', '.join(refs) or 'none'} |")
    lines += ["", "## Observed artifact receipt metadata", "", "Values in this table come only from hash-checked per-image capture receipts. Missing receipts and omitted fields are shown as `unknown`; registry descriptions above are not substituted for capture observations.", "", "| Platform | Role | Artifact | Theme | Receipt | Viewport | Focus | Scroll | Source commit | Source diff/hash | Binary SHA-256 |", "| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |"]
    for case in manifest["cases"]:
        for platform, artifacts in case.get("artifacts_by_platform", {}).items():
            for role in ("native", "reference_candidates"):
                for artifact in artifacts.get(role, []):
                    receipt = artifact.get("capture_receipt") or {}
                    source = receipt.get("source") or {}
                    viewport = receipt.get("viewport")
                    viewport_text = json.dumps(viewport, ensure_ascii=False, sort_keys=True) if viewport else "unknown"
                    source_commit = source.get("commit", source.get("source_commit", "unknown"))
                    source_hash = source.get("tracked_source_diff_sha256", source.get("source_sha256", "unknown"))
                    lines.append(
                        f"| `{platform}` | `{role}` | `{artifact['path']}` | {artifact.get('theme', 'unknown')} | "
                        f"{'present' if receipt else 'missing'} | {viewport_text} | {json.dumps(receipt.get('focus', 'unknown'), ensure_ascii=False)} | "
                        f"{json.dumps(receipt.get('scroll', 'unknown'), ensure_ascii=False)} | `{source_commit}` | "
                        f"`{source_hash}` | `{receipt.get('binary_sha256', 'unknown')}` |"
                    )
    if not any(case.get("artifacts_by_platform") for case in manifest["cases"]):
        lines.append("| — | — | No image artifacts | — | missing | unknown | unknown | unknown | unknown | unknown | unknown |")
    lines += ["", "## Boards and machine data", ""]
    for platform, info in manifest["platforms"].items():
        lines.append(f"- [{platform} review board]({info['board']}) · [{platform} semantic JSON]({info['machine_manifest']})")
    lines += ["", "## Exact-byte reuse", ""]
    for group in manifest["reuse_groups"]:
        artifacts = ", ".join(f"{a.get('platform', 'reference')}: `{a['path']}`" for a in group["artifacts"])
        lines.append(f"- `{group['reuse_group']}` ({group['sha256']}): {artifacts}. Reused artifacts do not count as independent coverage.")
    if not manifest["reuse_groups"]:
        lines.append("- No duplicate hashes found among captured scenario images.")
    return "\n".join(lines) + "\n"


def render_mapping_html(manifest: dict[str, Any]) -> str:
    links = "".join(f'<li><a href="{html.escape(info["board"])}">{html.escape(platform)} review board</a> · <a href="{html.escape(info["machine_manifest"])}">machine JSON</a></li>' for platform, info in manifest["platforms"].items())
    reuse = "".join(f'<li><code>{html.escape(group["reuse_group"])}</code> · {html.escape(group["sha256"])} · {len(group["artifacts"])} artifacts (reused evidence; not independent coverage)</li>' for group in manifest["reuse_groups"])
    cases = "".join(f'<tr><td><code>{html.escape(case["case_id"])}</code></td><td>{html.escape(case["review_status"])}</td><td>{html.escape(str(case["object"].get("type")))} · {html.escape(str(case["object"].get("identity")))}</td><td><code>{html.escape(case["comparable_scope"])}</code><br>{html.escape(case["rationale"])}</td></tr>' for case in manifest["cases"])
    return f'''<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Semantic visual mapping</title><style>body{{font:14px system-ui;max-width:1200px;margin:32px auto;padding:0 16px;color:#171717}}.notice{{padding:12px;background:#fff2cc;border:1px solid #d9ad32;border-radius:8px}}table{{border-collapse:collapse;width:100%}}td,th{{border-bottom:1px solid #ddd;padding:8px;text-align:left;vertical-align:top}}td:last-child{{min-width:300px}}</style><h1>Milestone II semantic visual mapping</h1><p class="notice">Filename equality only locates candidate screenshots. It does not establish semantic equivalence. Unreviewed cases and reused byte-identical images do not count as independent parity coverage. Archived focus, scroll, and source commits remain unknown without receipts.</p><h2>Boards</h2><ul>{links}</ul><h2>Exact-byte reuse groups</h2><ul>{reuse or '<li>None found</li>'}</ul><h2>Case status</h2><table><thead><tr><th>Case</th><th>Review status</th><th>Object / state</th><th>Comparable scope</th></tr></thead><tbody>{cases}</tbody></table></html>'''


def render_board(manifest: dict[str, Any]) -> str:
    cards = []
    if (OUTPUT / "e01-e03.html").exists():
        cards.append('<section class="case"><a href="e01-e03.html">E01–E03 capture calibration and assertions</a></section>')
    for case in manifest["cases"]:
        status = case["review_status"]
        scope = case["comparable_scope"]
        if status == "unreviewed":
            banner = "UNREVIEWED CANDIDATE — same filename does not establish equivalence"
        elif scope == "none":
            banner = "NOT EQUIVALENT — separate objects, triggers, or task strategy"
        elif scope == "visual_component_only":
            banner = "COMPONENT-ONLY — state parity is not established"
        else:
            banner = f"{status.upper()} · scope: {scope}"
        groups = sorted({a.get("reuse_group") for a in case["artifacts"]["native"] + case["artifacts"]["reference_candidates"] if a.get("reuse_group")})
        reuse = f'<p class="reuse">Reused exact-byte evidence: {html.escape(", ".join(groups))}. It does not count as independent coverage.</p>' if groups else ""
        exclusions = sorted({a["review_eligibility"] for a in case["artifacts"]["native"] if a.get("review_eligibility")})
        if exclusions:
            reuse += '<p class="banner">' + html.escape(" ".join(exclusions)) + '</p>'
        rows = []
        for theme in ("light", "dark"):
            native = next((a for a in case["artifacts"]["native"] if a["theme"] == theme), None)
            refs = [a for a in case["artifacts"]["reference_candidates"] if a["theme"] == theme]
            cols = [_render_image(native, "Native capture")]
            cols.extend(_render_image(a, "React filename candidate · unverified") for a in refs)
            if not refs:
                cols.append('<div class="missing">No React candidate capture for this theme</div>')
            rows.append(f'<div class="theme"><h3>{theme.title()}</h3><div class="images">{"".join(cols)}</div></div>')
        obj = case["object"]
        details = [
            f"<dt>Object</dt><dd>{html.escape(str(obj.get('type')))} · {html.escape(str(obj.get('identity')))}</dd>",
            f"<dt>Trigger</dt><dd>{html.escape(case['trigger'].get('value', 'unknown'))}</dd>",
            f"<dt>Data</dt><dd>{html.escape(case['data'].get('value', 'unknown'))}</dd>",
            f"<dt>Viewport</dt><dd>{html.escape(json.dumps(case.get('viewport', {}), ensure_ascii=False))}</dd>",
            f"<dt>Focus / scroll</dt><dd>{html.escape(str(case.get('focus', 'unknown')))} / {html.escape(str(case.get('scroll', 'unknown')))}</dd>",
            f"<dt>Source provenance</dt><dd>{html.escape(json.dumps(case.get('source_provenance', {}), ensure_ascii=False))}</dd>",
            f"<dt>Image receipts</dt><dd><details><summary>{sum(1 for a in case['artifacts']['native'] + case['artifacts']['reference_candidates'] if a.get('capture_receipt'))} present · other capture facts remain unknown</summary><pre>{html.escape(json.dumps({a['path']: a['capture_receipt'] for a in case['artifacts']['native'] + case['artifacts']['reference_candidates'] if a.get('capture_receipt')}, indent=2, ensure_ascii=False) or 'No sidecars found for these images.' )}</pre></details></dd>",
            f"<dt>Comparable scope</dt><dd><code>{html.escape(scope)}</code> — {html.escape(case['rationale'])}</dd>",
        ]
        cards.append(f'<section class="case"><h2>{html.escape(case["case_id"])}</h2><p class="banner">{html.escape(banner)}</p>{reuse}<dl>{"".join(details)}</dl>{"".join(rows)}</section>')
    title = f"Visual semantic review · {manifest['platform']}"
    return f'''<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>{html.escape(title)}</title>
<style>*{{box-sizing:border-box}}body{{margin:0;padding:24px;background:#f3f4f6;color:#171717;font:14px system-ui}}header{{max-width:1100px;margin:0 auto 24px}}h1{{margin:0}}.summary{{padding:12px;background:#fff4d6;border:1px solid #e2bd55;border-radius:8px;line-height:1.5}}.case{{max-width:1200px;margin:18px auto;padding:18px;background:white;border:1px solid #d6d6d6;border-radius:10px;overflow:auto}}.case h2{{margin:0 0 8px;font-size:18px}}.banner{{margin:0 0 12px;padding:10px;font-weight:700;background:#ffedd5;border:1px solid #f0a74a;border-radius:6px}}.reuse{{color:#8b1b1b;font-weight:650}}dl{{display:grid;grid-template-columns:140px minmax(0,1fr);gap:6px 12px;margin:12px 0}}dt{{font-weight:650}}dd{{margin:0;overflow-wrap:anywhere}}.theme{{margin-top:14px}}.images{{display:flex;align-items:flex-start;gap:16px;flex-wrap:wrap}}figure{{margin:0;min-width:280px;max-width:48%}}figcaption{{font-size:12px;color:#555;margin-bottom:6px}}img{{display:block;width:min(100%,960px);height:auto;border:1px solid #bbb}}.missing{{padding:30px;border:1px dashed #aaa;color:#666}}code{{font-size:12px}}@media(max-width:700px){{body{{padding:10px}}figure{{max-width:100%}}dl{{grid-template-columns:1fr}}}}</style>
<header><h1>{html.escape(title)}</h1><p class="summary">Filename equality is candidate lookup only. Unreviewed references are shown beside the native image for discovery, not as a parity claim. No historical focus, scroll, or source commit is inferred. Exact-byte reuse is labeled and does not add independent coverage.</p><p>Cases: {manifest['summary']['native_cases']} · independently covered full-state pairs: {manifest['summary']['reviewed_semantic_equivalences']} · exact-byte reuse groups: {manifest['summary']['exact_byte_reuse_groups']}</p><p><a href="m2-mapping.md">Semantic mapping</a> · <a href="m2-semantic.json">Machine manifest</a></p></header>{''.join(cards)}</html>'''


def _render_image(artifact: dict[str, Any] | None, label: str) -> str:
    if artifact is None:
        return '<div class="missing">Native capture missing</div>' if label == "Native capture" else ""
    path = html.escape(artifact["path"], quote=True)
    receipt = artifact.get("capture_receipt")
    facts = f"SHA-256 {artifact['sha256']}"
    if receipt:
        facts += " · capture receipt present"
        viewport = receipt.get("viewport", {})
        if viewport:
            facts += f" · logical {viewport.get('width')}×{viewport.get('height')} @ {viewport.get('dpr')} DPR"
    else:
        facts += " · provenance sidecar missing"
    group = artifact.get("reuse_group")
    if group:
        facts += f" · reused {group}"
    if artifact.get("review_eligibility"):
        facts += " · " + artifact["review_eligibility"]
    return f'<figure><figcaption>{html.escape(label)} · {html.escape(facts)}</figcaption><a href="{path}"><img loading="lazy" src="{path}" alt="{html.escape(artifact["path"])}"></a></figure>'


def build(platform: str) -> Path:
    manifest = build_semantic_manifest(platform)
    OUTPUT.mkdir(parents=True, exist_ok=True)
    platform_manifest = OUTPUT / f"m2-semantic-{platform}.json"
    platform_mapping = OUTPUT / f"m2-mapping-{platform}.md"
    platform_manifest.write_text(json.dumps(manifest, indent=2, ensure_ascii=False) + "\n")
    platform_mapping.write_text(render_markdown(manifest))
    board = OUTPUT / ("index.html" if platform == "windows-amd64" else f"{platform}.html")
    board.write_text(render_board(manifest))
    manifests = {}
    for candidate in ("windows-amd64", "macos-arm64"):
        stored = OUTPUT / f"m2-semantic-{candidate}.json"
        if stored.is_file():
            manifests[candidate] = json.loads(stored.read_text())
    combined = combine_platform_manifests(manifests)
    MANIFEST.write_text(json.dumps(combined, indent=2, ensure_ascii=False) + "\n")
    MAPPING.write_text(render_cross_platform_markdown(combined))
    (OUTPUT / "m2-mapping.html").write_text(render_mapping_html(combined))
    return board


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--platform", choices=["windows-amd64", "macos-arm64", "linux-amd64", "linux-arm64"], default="windows-amd64")
    args = parser.parse_args()
    print(build(args.platform))


if __name__ == "__main__":
    main()
