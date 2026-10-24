"""Validate a candidate window capture before publishing evidence, with provenance."""
import datetime
import hashlib
import json
from pathlib import Path
import re
import shutil
import struct
import subprocess
import unicodedata

ROOT = Path(__file__).resolve().parent.parent


def sha256(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def source_state():
    def git(*args):
        return subprocess.check_output(["git", "-C", str(ROOT), *args], text=True).strip()
    # Include only tracked source changes, never credentials or ignored data.
    diff = subprocess.check_output(["git", "-C", str(ROOT), "diff", "HEAD", "--", "crates", "Cargo.toml", "Cargo.lock"])
    return {"commit": git("rev-parse", "HEAD"), "dirty": bool(diff),
            "tracked_source_diff_sha256": hashlib.sha256(diff).hexdigest()}


def mac_helper():
    source = ROOT / "scripts/evidence-macos.swift"
    helper = ROOT / "target/evidence-tools/macos"
    helper.parent.mkdir(parents=True, exist_ok=True)
    if not helper.exists() or helper.stat().st_mtime < source.stat().st_mtime:
        subprocess.run(["swiftc", "-module-cache-path", str(helper.parent / "module-cache"), str(source), "-o", str(helper)], check=True)
    return helper


def ocr(path):
    return json.loads(subprocess.check_output([str(mac_helper()), "ocr", str(path)], text=True))


def normalized(text):
    text = unicodedata.normalize("NFKC", text).casefold()
    return re.sub(r"\s+", " ", re.sub(r"[^\w\s]", " ", text)).strip()


def validate(candidate, window, *, title, viewport, required, observations):
    """Require all independent assertions; a filename or image hash proves no page."""
    header = Path(candidate).read_bytes()[:24]
    if len(header) != 24 or header[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError("Candidate is not a PNG")
    width, height = struct.unpack(">II", header[16:24])
    if window["title"] != title:
        raise ValueError(f"Wrong page title: {window['title']!r}; expected {title!r}")
    logical = (round(window["width"]), round(window["height"]))
    if logical != tuple(viewport):
        raise ValueError(f"Wrong logical viewport: {logical}; expected {viewport}")
    if width / viewport[0] != height / viewport[1] or width / viewport[0] not in (1, 2):
        raise ValueError(f"PNG {width}×{height} does not match logical viewport {viewport}")
    text = normalized(" ".join(item["text"] for item in observations))
    missing = [phrase for phrase in required if normalized(phrase) not in text]
    if missing:
        raise ValueError("Page content assertions failed: " + ", ".join(missing))
    return {"title": title, "logical_viewport": list(viewport), "pixel_size": [width, height],
            "dpr": width / viewport[0], "required_text": required,
            "observations": observations, "passed": True}


def publish(candidate, destination, receipt, assertions):
    """Archive the original before replacement; never publish an unchecked candidate."""
    if assertions.get("passed") is not True:
        raise ValueError("Only an asserted capture may be published")
    candidate, destination = Path(candidate), Path(destination)
    destination.parent.mkdir(parents=True, exist_ok=True)
    if destination.exists():
        relative = destination.relative_to(ROOT / "target/parity-shots")
        baseline = ROOT / "target/parity-shots/archive/e01-e03-before-20261024" / relative
        baseline.parent.mkdir(parents=True, exist_ok=True)
        previous_hash = sha256(destination)
        archived = baseline
        if baseline.exists() and sha256(baseline) != previous_hash:
            archived = baseline.with_name(f"{baseline.stem}.{previous_hash[:16]}{baseline.suffix}")
        if not archived.exists():
            shutil.copy2(destination, archived)
        if sha256(archived) != previous_hash:
            raise ValueError("Previous capture archive does not match the image being replaced")
        receipt["replaced_original"] = {"path": str(archived.relative_to(ROOT)), "sha256": previous_hash}
        if archived != baseline:
            receipt["baseline_original"] = {"path": str(baseline.relative_to(ROOT)), "sha256": sha256(baseline)}
        previous_receipt = destination.with_suffix(".capture.json")
        if previous_receipt.exists():
            receipt_hash = sha256(previous_receipt)
            archived_receipt = archived.with_name(f"{archived.stem}.receipt-{receipt_hash[:16]}.json")
            if not archived_receipt.exists():
                shutil.copy2(previous_receipt, archived_receipt)
            if sha256(archived_receipt) != receipt_hash:
                raise ValueError("Previous capture receipt archive hash mismatch")
            receipt["replaced_receipt"] = {"path": str(archived_receipt.relative_to(ROOT)), "sha256": receipt_hash}
    shutil.copyfile(candidate, destination)
    receipt.update({"schema_version": 1, "captured_at": receipt.get("captured_at") or datetime.datetime.now(datetime.timezone.utc).isoformat(),
                    "artifact": str(destination.relative_to(ROOT)), "sha256": sha256(destination),
                    "assertions": assertions,
                    "viewport": {"width": assertions["logical_viewport"][0], "height": assertions["logical_viewport"][1], "dpr": assertions["dpr"]}})
    destination.with_suffix(".capture.json").write_text(json.dumps(receipt, indent=2) + "\n")
    return receipt
