#!/usr/bin/env python3
"""Run native Rust checks without generating receipts or source fingerprints.

Linux ARM64 requires a Linux ARM64 host; cross-compilation is not a substitute.
Visual acceptance requires opening the corresponding desktop application.
"""

import argparse
import json
import os
from pathlib import Path
import platform
import subprocess
import sys


PLATFORMS = {
    "windows-amd64": ("Windows", "x86_64", "x86_64-pc-windows-msvc", "X64"),
    "windows-arm64": ("Windows", "aarch64", "aarch64-pc-windows-msvc", "ARM64"),
    "linux-amd64": ("Linux", "x86_64", "x86_64-unknown-linux-gnu", "X64"),
    "linux-arm64": ("Linux", "aarch64", "aarch64-unknown-linux-gnu", "ARM64"),
    "macos-arm64": ("Darwin", "aarch64", "aarch64-apple-darwin", "ARM64"),
}


def capture(command, root):
    return subprocess.run(command, cwd=root, stdout=subprocess.PIPE,
                          stderr=subprocess.STDOUT, text=True, encoding="utf-8",
                          errors="replace", check=False, timeout=30)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--platform", choices=PLATFORMS, required=True)
    parser.add_argument("--verify-host-only", action="store_true")
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    system, arch, host, runner_arch = PLATFORMS[args.platform]
    actual_arch = {"amd64": "x86_64", "arm64": "aarch64"}.get(
        platform.machine().lower(), platform.machine().lower())
    rustc = capture(["rustc", "-vV"], root)
    actual_host = next((line[6:] for line in rustc.stdout.splitlines()
                        if line.startswith("host: ")), None)
    if (platform.system(), actual_arch) != (system, arch):
        parser.error(f"Expected {system}/{arch}; found {platform.system()}/{actual_arch}")
    if rustc.returncode or actual_host != host:
        parser.error(f"Expected Rust host {host}; found {actual_host!r}")
    if os.environ.get("RUNNER_ARCH") not in (None, runner_arch):
        parser.error(f"Expected RUNNER_ARCH={runner_arch}")
    if os.environ.get("CARGO_BUILD_TARGET") not in (None, "", host):
        parser.error("CARGO_BUILD_TARGET selects a different target")
    print(f"Native host verified: {args.platform} ({host})", flush=True)
    if args.verify_host_only:
        return 0
    commands = [
        ["cargo", "fmt", "--all", "--", "--check"],
        ["cargo", "check", "--locked", "--workspace", "--all-targets", "--all-features", "--target", host],
        ["cargo", "clippy", "--locked", "--workspace", "--all-targets", "--all-features", "--target", host, "--", "-D", "warnings"],
        ["cargo", "test", "--locked", "--workspace", "--all-features", "--target", host],
        ["cargo", "build", "--locked", "-p", "block-app", "--bin", "block", "--target", host],
    ]
    for command in commands:
        print(subprocess.list2cmdline(command), flush=True)
        if not args.dry_run:
            result = subprocess.run(command, cwd=root, check=False)
            if result.returncode:
                return result.returncode if result.returncode > 0 else 1
    if args.dry_run:
        print("Plan only; no build or test success claimed.")
        return 0
    metadata = capture(["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"], root)
    if metadata.returncode:
        print(metadata.stdout, file=sys.stderr)
        return 1
    executable = "block.exe" if system == "Windows" else "block"
    binary = Path(json.loads(metadata.stdout)["target_directory"]) / host / "debug" / executable
    info = capture([str(binary), "--build-info"], root)
    if info.returncode:
        print(info.stdout, file=sys.stderr)
        return 1
    build = json.loads(info.stdout)
    expected = {"product_id": "one.ztd.block", "arch": arch,
                "os": {"Windows": "windows", "Linux": "linux", "Darwin": "macos"}[system]}
    if any(build.get(key) != value for key, value in expected.items()):
        print(f"Built executable does not match {args.platform}", file=sys.stderr)
        return 1
    output = os.environ.get("GITHUB_OUTPUT")
    if output:
        with open(output, "a", encoding="utf-8") as stream:
            stream.write(f"executable={binary}\n")
    print(f"Native checks and desktop build passed: {args.platform}")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, subprocess.TimeoutExpired) as error:
        print(error, file=sys.stderr)
        sys.exit(1)
