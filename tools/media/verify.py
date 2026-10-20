#!/usr/bin/env python3
"""Source-bound Linux native diagnostics, suitable for an isolated CI worker.

Run only on the assigned build VM or CI worker. Talk/browser tests are separate
because they require an explicitly owned disposable server manifest.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
import time

REPO = Path(__file__).resolve().parents[2]


def sha(path):
    h = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def sources():
    paths = []
    for name in ("native/media", "tools/media", "crates/talk-adapters/src", "crates/talk-core/src"):
        paths.extend(p for p in (REPO / name).rglob("*") if p.is_file() and "__pycache__" not in p.parts)
    paths.extend(REPO / name for name in ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "crates/talk-adapters/Cargo.toml", "crates/talk-core/Cargo.toml"))
    return {str(path.relative_to(REPO)): sha(path) for path in sorted(paths)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--packets", action="store_true", help="requires tcpdump and permission for this worker's packet capture")
    parser.add_argument("--native-output-name", default="nmt-i2")
    args = parser.parse_args()
    if platform.system() != "Linux":
        raise SystemExit("This diagnostic collector currently requires Linux /proc")
    out, root = args.output.resolve(), args.root.resolve()
    if out.is_relative_to(REPO) or REPO.is_relative_to(out) or out.exists():
        raise SystemExit("Use a new external evidence directory; earlier results are never overwritten")
    out.mkdir(parents=True)
    report = {"schema": 2, "result": "fail", "production_qualification": False,
              "sources": sources(), "platform": platform.platform(), "commands": []}
    driver, helper = out / "nmt-media-driver", root / "src/out" / args.native_output_name / "nmt-media-helper"
    env = dict(os.environ, PYTHONDONTWRITEBYTECODE="1")
    report["environment"] = {k: env.get(k) for k in ("PATH", "CARGO_TARGET_DIR", "RUSTUP_TOOLCHAIN", "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "LC_ALL")}
    report["cwd"] = str(REPO)
    report["started_unix"] = time.time()
    (out / "inputs-before-build.json").write_text(json.dumps(report, indent=2) + "\n")
    report["inputs_before_build_sha256"] = sha(out / "inputs-before-build.json")

    def run(label, argv, timeout=600):
        if sources() != report["sources"]:
            raise RuntimeError("immutable input changed before " + label)
        started = time.monotonic()
        binary_before = sha(Path(argv[0])) if Path(argv[0]).is_file() else None
        with (out / (label + ".log")).open("wb") as log:
            result = subprocess.run([str(x) for x in argv], cwd=REPO, env=env,
                                    stdout=log, stderr=subprocess.STDOUT, timeout=timeout)
        report["commands"].append({"label": label, "argv": [str(x) for x in argv],
                                   "exit": result.returncode, "seconds": time.monotonic() - started,
                                   "log_sha256": sha(out / (label + ".log")),
                                   "executed_binary_sha256": binary_before})
        (out / "run.json").write_text(json.dumps(report, indent=2) + "\n")
        if result.returncode:
            raise RuntimeError(label)

    try:
        run("build", ["python3", "tools/media/build.py", "--root", root, "--jobs", "8", "--output-name", args.native_output_name, "--evidence", out / "native-build"])
        run("rust-version", ["rustc", "+1.99.1", "--version", "--verbose"])
        run("admission-compiler", ["c++", "--version"])
        run("admission-build", ["c++", "-std=c++20", "-O2", "-Wall", "-Wextra", "-Werror", "native/media/sctp_limits.cc", "native/media/sctp_limits_test.cc", "-o", out / "sctp-limits-test"])
        run("admission", [out / "sctp-limits-test"])
        run("driver-build", ["rustc", "+1.99.1", "--edition=2024", "tools/media/driver.rs", "-o", driver])
        run("unit-build", ["rustc", "+1.99.1", "--edition=2024", "--test", "tools/media/driver.rs", "-o", out / "media-unit-tests"])
        run("unit", [out / "media-unit-tests"])
        run("supervisor-build", ["rustc", "+1.99.1", "--edition=2024", "tools/media/supervisor_checks.rs", "-o", out / "supervisor-checks"])
        run("supervisor", [out / "supervisor-checks", helper])
        run("runtime-core-build", ["rustc", "+1.99.1", "--edition=2024", "--crate-type=rlib", "--crate-name=talk_core", "crates/talk-core/src/lib.rs", "-o", out / "libtalk_core.rlib"])
        run("runtime-build", ["rustc", "+1.99.1", "--edition=2024", "tools/media/runtime_checks.rs", "--extern", "talk_core=" + str(out / "libtalk_core.rlib"), "-o", out / "runtime-checks"])
        run("runtime", [out / "runtime-checks", helper])
        run("supervisor-absolute-expiry", [out / "supervisor-checks", helper, "--absolute-expiry"], timeout=40)
        run("native-absolute-expiry", ["python3", "tools/media/expiry.py", "--helper", helper, "--output", out / "native-absolute-expiry.json"], timeout=40)
        run("ipc-faults", ["python3", "tools/media/faults.py", "--driver", driver, "--helper", helper, "--output", out / "ipc-faults.json"])
        for name, fault in (("pair", []), ("stall", ["--stall"]), ("owner-death", ["--fault", "owner-death"]), ("heartbeat-expiry", ["--fault", "heartbeat-expiry"])):
            run(name, ["python3", "tools/media/interop.py", "--driver", driver, "--helper", helper,
                       "--output", out / (name + ".json"), *fault, *(["--packets"] if args.packets else [])])
        run("bridge-build", ["cargo", "+1.99.1", "build", "--locked", "-p", "talk-adapters", "--features", "backend-test", "--example", "media-talk-bridge"])
        run("bridge-clippy", ["cargo", "+1.99.1", "clippy", "--locked", "-p", "talk-adapters", "--features", "backend-test", "--example", "media-talk-bridge", "--", "-D", "warnings"])
        if sources() != report["sources"]:
            raise RuntimeError("source changed during verification")
        report.update(result="pass", helper_sha256=sha(helper), driver_sha256=sha(driver),
                      bridge_sha256=sha(Path(env.get("CARGO_TARGET_DIR", REPO / "target")) / "debug/examples/media-talk-bridge"),
                      native_build_identity_sha256=sha(out / "native-build/build.json"))
    except (OSError, RuntimeError, subprocess.TimeoutExpired) as error:
        report["failure_type"] = type(error).__name__
    finally:
        (out / "run.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({"result": report["result"], "report": str(out / "run.json")}))
    raise SystemExit(0 if report["result"] == "pass" else 1)


if __name__ == "__main__":
    main()
