#!/usr/bin/env python3
"""Run native Rust checks and retain platform evidence.

Examples:
    python scripts/verify-platform.py --platform macos-arm64
    python3 scripts/verify-platform.py --platform linux-arm64 --evidence-dir /tmp/block-arm64

Linux ARM64 requires a Linux ARM64 host; cross-compilation is not a substitute.
Visual and animation acceptance requires opening the corresponding desktop app.
"""

import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
import sys
import time


PLATFORMS = {
    "windows-amd64": ("Windows", "x86_64", "x86_64-pc-windows-msvc", "X64"),
    "linux-amd64": ("Linux", "x86_64", "x86_64-unknown-linux-gnu", "X64"),
    "linux-arm64": ("Linux", "aarch64", "aarch64-unknown-linux-gnu", "ARM64"),
    "macos-arm64": ("Darwin", "aarch64", "aarch64-apple-darwin", "ARM64"),
}


def utc_now():
    return datetime.now(timezone.utc).isoformat()


def capture(command, root):
    try:
        result = subprocess.run(
            command, cwd=root, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
            text=True, encoding="utf-8", errors="replace", check=False,
        )
        return {"command": command, "exit_code": result.returncode, "output": result.stdout}
    except OSError as error:
        return {"command": command, "exit_code": 127, "output": str(error)}


def source_metadata(root, evidence_dir):
    """Fingerprint working files, including untracked source and copied snapshots."""
    commands = [capture(["git", *args], root) for args in (
        ["rev-parse", "HEAD"], ["status", "--porcelain=v1", "--untracked-files=all"],
        ["ls-files", "-z", "--cached", "--others", "--exclude-standard"],
    )]
    head, status, inventory = commands
    if inventory["exit_code"] == 0:
        names = sorted(set(inventory["output"].split("\0")) - {""})
    else:
        # Remote verification may use a source archive rather than a Git checkout.
        names = []
        for directory, dirs, files in os.walk(root):
            dirs[:] = sorted(name for name in dirs if name not in {
                ".git", "target", "node_modules", "__pycache__", ".venv"
            })
            names.extend((Path(directory) / name).relative_to(root).as_posix() for name in files)
        names.sort()
    manifest = []
    for name in names:
        path = root / name
        if path == evidence_dir or evidence_dir in path.parents:
            continue
        if path.is_symlink():
            digest = hashlib.sha256(os.readlink(path).encode()).hexdigest()
            manifest.append({"path": name, "kind": "symlink", "sha256": digest})
        elif path.is_file():
            digest = hashlib.sha256()
            with path.open("rb") as source:
                for chunk in iter(lambda: source.read(1024 * 1024), b""):
                    digest.update(chunk)
            manifest.append({"path": name, "kind": "file", "sha256": digest.hexdigest()})
        else:
            manifest.append({"path": name, "kind": "missing"})
    encoded = json.dumps(manifest, ensure_ascii=False, sort_keys=True).encode()
    return {
        "revision": head["output"].strip() if head["exit_code"] == 0 else None,
        "dirty": bool(status["output"]) if status["exit_code"] == 0 else None,
        "git_status": status["output"] if status["exit_code"] == 0 else None,
        "manifest_sha256": hashlib.sha256(encoded).hexdigest(),
        "manifest": manifest, "git_commands": commands[:2],
        "inventory": "git-working-files" if inventory["exit_code"] == 0 else "source-snapshot",
    }


def native_errors(requested, rustc):
    system, arch, expected_host, runner_arch = PLATFORMS[requested]
    actual_arch = {"amd64": "x86_64", "arm64": "aarch64"}.get(
        platform.machine().lower(), platform.machine().lower()
    )
    host = next((line[6:] for line in rustc["output"].splitlines() if line.startswith("host: ")), None)
    errors = []
    if (platform.system(), actual_arch) != (system, arch):
        errors.append(f"Expected {system}/{arch}; process host is {platform.system()}/{actual_arch}")
    if rustc["exit_code"] != 0 or host != expected_host:
        errors.append(f"Expected Rust host {expected_host}; found {host!r}")
    if os.environ.get("RUNNER_ARCH") not in (None, runner_arch):
        errors.append(f"Expected RUNNER_ARCH={runner_arch}; found {os.environ['RUNNER_ARCH']}")
    if os.environ.get("CARGO_BUILD_TARGET") not in (None, "", expected_host):
        errors.append("CARGO_BUILD_TARGET selects a different target; native tests are required")
    return expected_host, errors


def verification_commands(host, requested):
    # Explicit native targets override Cargo config that might cross-compile.
    # Workspace tests include core, UI, GPUI interactions, session supervision,
    # future integration tests, and documentation tests.
    commands = [
        ("format", ["cargo", "fmt", "--all", "--", "--check"]),
        ("clippy", ["cargo", "clippy", "--locked", "--workspace", "--all-targets",
                    "--all-features", "--target", host, "--", "-D", "warnings"]),
        ("tests", ["cargo", "test", "--locked", "--workspace", "--all-features", "--target", host]),
    ]
    if requested.startswith("linux-"):
        # This vendored standalone crate intentionally has no committed lockfile;
        # preserve its existing compatibility test. Workspace commands stay locked.
        commands.append(("xattr", ["cargo", "test", "--manifest-path",
                                   "vendor/xattr/Cargo.toml", "--target", host]))
    commands.append(("desktop-build", ["cargo", "build", "--locked", "-p", "block-app",
                                       "--bin", "block", "--target", host]))
    return commands


def run_logged(name, command, root, evidence_dir):
    log = evidence_dir / f"{name}.log"
    record = {"name": name, "command": command, "started_at": utc_now(), "log": log.name}
    started = time.monotonic()
    print(f"\n[{name}] {subprocess.list2cmdline(command)}", flush=True)
    with log.open("w", encoding="utf-8") as output:
        output.write(subprocess.list2cmdline(command) + "\n")
        output.flush()
        try:
            process = subprocess.Popen(command, cwd=root, stdout=subprocess.PIPE,
                                       stderr=subprocess.STDOUT, text=True, encoding="utf-8",
                                       errors="replace", bufsize=1)
            try:
                for line in process.stdout:
                    print(line, end="", flush=True)
                    output.write(line)
                    output.flush()
                record["exit_code"] = process.wait()
            except KeyboardInterrupt:
                process.terminate()
                process.wait()
                record["exit_code"] = 130
                record["interrupted"] = True
        except OSError as error:
            output.write(str(error) + "\n")
            record["exit_code"] = 127
    record.update(completed_at=utc_now(), duration_seconds=round(time.monotonic() - started, 3))
    return record


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--platform", choices=PLATFORMS, required=True)
    parser.add_argument("--evidence-dir", type=Path, help="New directory for logs and report; preserves target caches")
    parser.add_argument("--verify-host-only", action="store_true", help="Validate host only; does not run tests")
    parser.add_argument("--dry-run", action="store_true", help="Record a plan only; does not claim test success")
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    evidence_dir = (args.evidence_dir or root / "target" / "platform-verification" /
                    f"{args.platform}-{datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%S%fZ')}").resolve()
    if evidence_dir.exists() and any(evidence_dir.iterdir()):
        parser.error(f"Evidence directory is not empty: {evidence_dir}; choose a new directory")
    report = {
        "schema_version": 1, "requested_platform": args.platform, "started_at": utc_now(),
        "platform": {"system": platform.system(), "machine": platform.machine(),
                     "release": platform.release(), "runner_arch": os.environ.get("RUNNER_ARCH")},
        "source": source_metadata(root, evidence_dir),
        "rustc": capture(["rustc", "-vV"], root),
        "commands": [], "status": "running",
        "scope": "native-build-and-automated-tests; visual-and-animation-acceptance-is-separate",
    }
    evidence_dir.mkdir(parents=True, exist_ok=True)
    report_file = evidence_dir / "report.json"

    def save():
        report_file.write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")

    host, errors = native_errors(args.platform, report["rustc"])
    report.update(rust_host=host, host_errors=errors)
    report["planned_commands"] = [{"name": name, "command": command}
                                  for name, command in verification_commands(host, args.platform)]
    save()
    exit_code = 0
    if errors:
        report["status"] = "host-mismatch"
        print("\n".join(errors), file=sys.stderr)
        exit_code = 1
    elif args.verify_host_only:
        report["status"] = "host-verified-only"
    elif args.dry_run:
        report["commands"] = [{"name": name, "command": command, "exit_code": None}
                              for name, command in verification_commands(host, args.platform)]
        report["status"] = "planned-only"
    else:
        for name, command in verification_commands(host, args.platform):
            record = run_logged(name, command, root, evidence_dir)
            report["commands"].append(record)
            save()
            if record["exit_code"] != 0:
                exit_code = record["exit_code"] if record["exit_code"] > 0 else 1
                break
        report["status"] = "passed" if exit_code == 0 else "failed"
    report.update(completed_at=utc_now(), exit_code=exit_code)
    save()
    print(f"\n{report['status']}: {report_file}", flush=True)
    return exit_code


if __name__ == "__main__":
    sys.exit(main())
