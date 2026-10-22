#!/usr/bin/env python3
"""Small source policy plus upstream actionlint; not a YAML parser or sandbox.

Tests belong on Linux. Windows/macOS jobs compile/package only. Helper behavior
is ordinary reviewed source, rather than a second hash-manifest build system.
"""
import argparse
import ast
import hashlib
import io
import json
import platform
from pathlib import Path
import re
import subprocess
import tarfile
import tempfile
import urllib.request

ACTIONLINT_VERSION = "1.7.12"
ACTIONLINT_SHA256 = "8aca8db96f1b94770f1b0d72b6dddcb1ebb8123cb3712530b08cc387b349a3d8"
ACTIONLINT_URL = "https://github.com/rhysd/actionlint/releases/download/v" + ACTIONLINT_VERSION + "/actionlint_" + ACTIONLINT_VERSION + "_linux_amd64.tar.gz"
LINUX = {"blacksmith-2vcpu-ubuntu-2404", "blacksmith-4vcpu-ubuntu-2404"}
COMPILE = {"blacksmith-4vcpu-windows-2025", "blacksmith-6vcpu-macos-latest"}
TEST = re.compile(r"cargo(?:\.py)?\s+(?:test|bench|run|nextest)\b|tests?\.py\b|capture\.py|--ui-scenario|--licenses|xvfb-run|pytest")


def check(root, actionlint=False, placement=True):
    root = Path(root)
    pins = {a["action"]: a["commit"] for a in json.loads((root / "tools/release/actions.json").read_text())}
    workflows = sorted((root / ".github/workflows").glob("*.yml"))
    if not workflows:
        raise ValueError("No workflows")
    for file in workflows:
        body = file.read_text()
        if re.search(r"\bpull_request_target\s*:", body):
            raise ValueError("Privileged contributor-code event forbidden")
        if not re.search(r"(?m)^permissions:\n  contents: read\s*$", body):
            raise ValueError("Default token must be contents: read")
        if "persist-credentials: true" in body:
            raise ValueError("Checkout must not persist credentials")
        for action, commit in re.findall(r"(?m)^\s*-?\s*uses:\s*([^@\s]+)@([^\s#]+)", body):
            if pins.get(action) != commit or not re.fullmatch(r"[0-9a-f]{40}", commit):
                raise ValueError("Unrecorded/mutable action: " + action)
        if "\njobs:\n" not in body:
            raise ValueError("Jobs section missing")
        blocks = re.split(r"(?m)^  ([a-z][a-z0-9_-]*):\n(?=    )", body.split("\njobs:\n", 1)[1])
        for name, job in zip(blocks[1::2], blocks[2::2]):
            runner = re.search(r"(?m)^    runs-on: ([^\s]+)$", job)
            if not runner or runner[1] not in LINUX | COMPILE:
                raise ValueError("Unexpected runner in " + name)
            if not re.search(r"(?m)^    timeout-minutes: [1-9][0-9]*$", job):
                raise ValueError("Missing timeout in " + name)
            if "contents: write" in job and (file.name, name) != ("release.yml", "publish"):
                raise ValueError("Only release publication may write contents")
            if runner[1] in COMPILE:
                if TEST.search(job):
                    raise ValueError("Tests/application execution must run on Linux: " + name)
                if not any(command in job for command in (
                    "cargo build --locked", "tools/cargo.py build --locked", "tools/release/build.py",
                )):
                    raise ValueError("Compile-only job lacks its build: " + name)
                if re.search(r"(?m)^    if:|^        if:", job):
                    raise ValueError("Compile-only builds must not be conditionally skipped: " + name)
    for folder in ("tools/release", "tools/ci"):
        for source in (root / folder).glob("*.py"):
            ast.parse(source.read_text(), filename=str(source))
    print(f"Workflow policy: {len(workflows)} workflows; tests only on Linux", flush=True)
    if actionlint:
        if (platform.system(), platform.machine()) != ("Linux", "x86_64"):
            raise ValueError("Pinned actionlint binary is Linux x86_64 only")
        with urllib.request.urlopen(ACTIONLINT_URL, timeout=60) as response:
            payload = response.read(20 * 1024 * 1024 + 1)
        if len(payload) > 20 * 1024 * 1024 or hashlib.sha256(payload).hexdigest() != ACTIONLINT_SHA256:
            raise ValueError("actionlint download size/hash mismatch")
        with tempfile.TemporaryDirectory(prefix="nmt-actionlint-") as tmp:
            binary = Path(tmp) / "actionlint"
            with tarfile.open(fileobj=io.BytesIO(payload), mode="r:gz") as archive:
                members = [m for m in archive.getmembers() if m.name == "actionlint"]
                if len(members) != 1 or not members[0].isfile() or members[0].size > 30 * 1024 * 1024:
                    raise ValueError("Unexpected actionlint archive")
                binary.write_bytes(archive.extractfile(members[0]).read())
            binary.chmod(0o700)
            subprocess.run([str(binary), "-version"], check=True, timeout=15)
            # Explicit config: actionlint only auto-discovers it inside a Git checkout.
            config = root / ".github/actionlint.yaml"
            options = ["-config-file", str(config)] if config.is_file() else []
            subprocess.run([str(binary), "-oneline", *options, *map(str, workflows)], cwd=root, check=True, timeout=90)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[2])
    parser.add_argument("--actionlint", action="store_true")
    args = parser.parse_args()
    check(args.root, actionlint=args.actionlint)
