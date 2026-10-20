#!/usr/bin/env python3
"""Reproduce the diagnostic native helper from an already pinned fetch.py root."""
import argparse
import ast
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import time

REPO = Path(__file__).resolve().parents[2]


def sha(path):
    h = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--root", type=Path, required=True)
    p.add_argument("--jobs", type=int, default=8)
    p.add_argument("--output-name", default="nmt-i2")
    p.add_argument("--evidence", type=Path)
    args = p.parse_args()
    root = args.root.resolve()
    if root.is_relative_to(REPO) or REPO.is_relative_to(root) or not 1 <= args.jobs <= 16:
        raise SystemExit("Separate build root and 1..16 jobs required")
    src = root / "src"
    if not args.output_name.replace("-", "").isalnum():
        raise SystemExit("Invalid output name")
    pins = json.loads((REPO / "native/media/pins.json").read_text())
    for relative, pin in [("", pins["webrtc"]["commit"]), ("third_party", pins["chromium_third_party_commit"]),
                          ("third_party/libvpx/source/libvpx", pins["libvpx_commit"])]:
        actual = subprocess.check_output(["git", "-C", str(src / relative), "rev-parse", "HEAD"], text=True).strip()
        if actual != pin:
            raise SystemExit("Native dependency pin mismatch")
    evidence = args.evidence.resolve() if args.evidence else root / "evidence-i2"
    evidence.mkdir(parents=True, exist_ok=True)
    # The explicit prior patch manifest recognizes only the known I1 edits in
    # a reused cache. It does not adopt arbitrary modified upstream files.
    if not (evidence / "patches.json").exists() and (root / "evidence/patches.json").exists():
        shutil.copyfile(root / "evidence/patches.json", evidence / "patches.json")
    spec = importlib.util.spec_from_file_location("media_patch", REPO / "native/media/patches/apply.py")
    patches = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(patches)
    patches.apply(src, evidence)
    owned = src / "nmt_media"
    owned.mkdir(exist_ok=True)
    for filename in ("BUILD.gn", "helper.cc", "sctp_limits.cc", "sctp_limits.h"):
        source = REPO / "native/media" / filename
        destination = owned / filename
        if not destination.exists() or source.read_bytes() != destination.read_bytes():
            shutil.copyfile(source, destination)
    env = dict(os.environ, PATH=str(root / "depot_tools") + os.pathsep + os.environ["PATH"],
               DEPOT_TOOLS_UPDATE="0", DEPOT_TOOLS_METRICS="0")
    # Only software VP8 and Opus are offered by the embedding. Inventory still
    # records the aggregate library's other compiled code, not just SDP offers.
    gn_args = 'is_debug=false symbol_level=1 is_component_build=false rtc_include_tests=false rtc_build_examples=false rtc_use_h264=false rtc_use_h265=false rtc_enable_sctp=true rtc_enable_protobuf=false rtc_use_x11=false use_remoteexec=false use_siso=false treat_warnings_as_errors=false'
    gn = src / "buildtools" / ("mac" if platform.system() == "Darwin" else "linux64") / "gn"
    out_relative = "out/" + args.output_name
    out = src / out_relative
    started = time.monotonic()
    commands = [[str(gn), "gen", out_relative, "--args=" + gn_args],
                [str(src / "third_party/ninja/ninja"), "-C", out_relative, "-j", str(args.jobs), "nmt-media-helper"]]
    before = {"schema": 2, "source_root": str(REPO), "cwd": str(src), "commands": commands,
              "recipe_sha256": sha(Path(__file__)), "patch_sha256": sha(evidence / "upstream.patch"),
              "inputs": {str(f.relative_to(REPO)): sha(f) for f in sorted((REPO / "native/media").rglob("*")) if f.is_file() and "__pycache__" not in f.parts},
              "environment": {k: env.get(k) for k in ("PATH", "DEPOT_TOOLS_UPDATE", "DEPOT_TOOLS_METRICS", "CC", "CXX", "CFLAGS", "CXXFLAGS", "LDFLAGS")},
              "incremental_cache": out.exists()}
    (evidence / "build-input.json").write_text(json.dumps(before, indent=2) + "\n")
    for command in commands:
        subprocess.run(command, cwd=src, env=env, check=True)
    binary = out / "nmt-media-helper"
    identity = {"schema": 1, "diagnostic_only": True, "platform": platform.platform(),
        "pins": pins, "commands": commands, "elapsed_seconds": time.monotonic() - started,
        "binary_sha256": sha(binary), "binary_bytes": binary.stat().st_size,
        "inputs": {str(f.relative_to(REPO)): sha(f) for f in sorted((REPO / "native/media").rglob("*")) if f.is_file() and "__pycache__" not in f.parts},
        "gn_args_sha256": sha(out / "args.gn"),
        "recipe_sha256": sha(Path(__file__)),
        "DEPS_sha256": sha(src / "DEPS"),
        "gn_sha256": sha(gn),
        "ninja_sha256": sha(src / "third_party/ninja/ninja"),
        "compiler": subprocess.check_output([str(src / "third_party/llvm-build/Release+Asserts/bin/clang++"), "--version"], text=True),
        "compiler_sha256": sha(src / "third_party/llvm-build/Release+Asserts/bin/clang++"),
        "patch_sha256": sha(evidence / "upstream.patch"),
        "upstream_rust_compiler": subprocess.check_output([str(src / "third_party/rust-toolchain/bin/rustc"), "--version"], text=True),
        "upstream_rust_compiler_sha256": sha(src / "third_party/rust-toolchain/bin/rustc")}
    # DEPS already pins GCS archive bytes and generations. Extract that public
    # metadata without executing source, and bind the fetched archive URLs.
    active = {entry["url"].split("/", 3)[-1] for entry in json.loads((root / "dependency-revisions.json").read_text()).values() if entry["url"].startswith("gs://")}
    archives = []
    for node in ast.walk(ast.parse((src / "DEPS").read_text())):
        if not isinstance(node, ast.Dict):
            continue
        fields = {key.value:value.value for key,value in zip(node.keys,node.values)
                  if isinstance(key, ast.Constant) and isinstance(value, ast.Constant)}
        if fields.get("object_name") in active and "sha256sum" in fields:
            archives.append({key:fields[key] for key in ("object_name","sha256sum","size_bytes","generation") if key in fields})
    identity["active_GCS_archives_from_pinned_DEPS"] = archives
    if identity["inputs"] != before["inputs"]:
        raise RuntimeError("Native source changed during build")
    identity["build_input_sha256"] = sha(evidence / "build-input.json")
    (evidence / "build.json").write_text(json.dumps(identity, indent=2) + "\n")
    subprocess.run([str(binary), "--version"], check=True)


if __name__ == "__main__":
    main()
