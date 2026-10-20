#!/usr/bin/env python3
"""Fetch ADR-005 inputs outside the checkout. Run only in the assigned build VM.

The upstream DEPS pins the native dependency/toolchain graph. No floating source
revision or prebuilt third-party WebRTC distribution is substituted.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess

REPO = Path(__file__).resolve().parents[2]
PINS = json.loads((REPO / "native/media/pins.json").read_text())


def run(args, cwd, env=None):
    subprocess.run(args, cwd=cwd, env=env, check=True)


def git_output(path, *args):
    return subprocess.check_output(["git", "-C", str(path), *args], text=True).strip()


def checkout(root, name, pin):
    path = root / name
    if not path.exists():
        path.mkdir()
        run(["git", "init", "-q"], path)
        run(["git", "remote", "add", "origin", pin["url"]], path)
    if git_output(path, "remote", "get-url", "origin") != pin["url"]:
        raise SystemExit("Unexpected upstream origin")
    try:
        head = git_output(path, "rev-parse", "HEAD")
    except subprocess.CalledProcessError:
        head = None
    if head != pin["commit"]:
        if head is not None:
            raise SystemExit("Existing checkout has different revision; use a new build root")
        run(["git", "-c", "http.version=HTTP/1.1", "fetch", "--depth=1", "origin", pin["commit"]], path)
        run(["git", "checkout", "--detach", "FETCH_HEAD"], path)
    if "tree" in pin and git_output(path, "rev-parse", "HEAD^{tree}") != pin["tree"]:
        raise SystemExit("Source tree mismatch")
    return path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--no-hooks", action="store_true")
    args = parser.parse_args()
    root = args.root.resolve()
    if root.is_relative_to(REPO) or REPO.is_relative_to(root):
        raise SystemExit("Build root must be separate from repository and sync root")
    root.mkdir(parents=True, exist_ok=True)
    depot = checkout(root, "depot_tools", PINS["depot_tools"])
    src = checkout(root, "src", PINS["webrtc"])
    config = ('solutions = [{"name": "src", "url": ' + repr(PINS["webrtc"]["url"])
              + ', "managed": False, "custom_deps": {}, "custom_vars": '
              + '{"checkout_android": False, "checkout_ios": False}}]\n')
    (root / ".gclient").write_text(config)
    env = dict(os.environ, PATH=str(depot) + os.pathsep + os.environ["PATH"],
               DEPOT_TOOLS_UPDATE="0", DEPOT_TOOLS_METRICS="0")
    run([str(depot / "gclient"), "sync", "--no-history", "--shallow", "--nohooks", "-j", "8"], root, env)
    checks = {"src": PINS["webrtc"]["commit"],
              "src/third_party": PINS["chromium_third_party_commit"],
              "src/third_party/libvpx/source/libvpx": PINS["libvpx_commit"]}
    for relative, expected in checks.items():
        if git_output(root / relative, "rev-parse", "HEAD") != expected:
            raise SystemExit("Dependency pin mismatch: " + relative)
    if not args.no_hooks:
        run([str(depot / "gclient"), "runhooks"], root, env)
    subprocess.run([str(depot / "gclient"), "revinfo", "--actual", "--output-json", str(root / "dependency-revisions.json")], cwd=root, env=env, check=True, stdout=subprocess.DEVNULL)
    (root / "fetch-identity.json").write_text(json.dumps({
        "pins": PINS, "DEPS_sha256": hashlib.sha256((src / "DEPS").read_bytes()).hexdigest(),
        "pins_sha256": hashlib.sha256((REPO / "native/media/pins.json").read_bytes()).hexdigest(),
        "hooks_run": not args.no_hooks}, indent=2) + "\n")


if __name__ == "__main__":
    main()
