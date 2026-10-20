#!/usr/bin/env python3
"""Owned Linux lab: deterministic runtime lock races and header-only D-Bus counts.

The private GNOME service and unlock secret are owned by native_service.py. No
message body, object path, native error text or credential is retained here.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import threading
import time

from native_service import NativeService


def validate_outputs(binary, report, source_manifest, raw_log):
    inputs = [binary, source_manifest, Path(__file__)]
    outputs = [report, raw_log]
    resolved = [p.resolve() for p in inputs + outputs]
    if len(set(resolved)) != len(resolved):
        raise ValueError("native evidence paths collide")
    # A parent may already hold the output at offset zero. Reject aliases by
    # inode as well as spelling, before any service starts or log is opened.
    stdio = [(os.fstat(fd).st_dev, os.fstat(fd).st_ino) for fd in (1, 2)]
    for output in outputs:
        if output.exists() or output.is_symlink():
            st = output.stat()
            if (st.st_dev, st.st_ino) in stdio:
                raise ValueError("native evidence aliases controller output")
            raise FileExistsError("native evidence output already exists")


def run(binary, report, source_manifest, raw_log):
    validate_outputs(binary, report, source_manifest, raw_log)
    facts = {
        "schema": 1,
        "role": "author regression evidence; no independent approval",
        "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
        "source_manifest_sha256": hashlib.sha256(source_manifest.read_bytes()).hexdigest(),
        "controller_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        "native_service_sha256": hashlib.sha256(Path(__file__).with_name("native_service.py").read_bytes()).hexdigest(),
        "calls": {},
        "os_side_unlocks": 0,
    }
    monitor = process = None
    ready = threading.Event()
    try:
        with NativeService() as service:
            control = service.root / "race-control"
            control.mkdir(mode=0o700)
            (control / "phase").write_text("setup")
            monitor = subprocess.Popen(
                ["dbus-monitor", "--session",
                 "type='method_call',interface='org.freedesktop.Secret.Service',member='Unlock'",
                 "type='method_call',interface='org.freedesktop.Secret.Prompt',member='Prompt'"],
                env=service.env, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
            )

            def count_headers():
                # Read bounded lines; ignore all body lines without decoding/retaining.
                for raw in iter(monitor.stdout.readline, b""):
                    ready.set()
                    if len(raw) > 8192:
                        continue
                    if not raw.startswith(b"method call "):
                        continue
                    method = "Unlock" if b"member=Unlock" in raw else "Prompt" if b"member=Prompt" in raw else None
                    if method is not None:
                        phase = (control / "phase").read_text()
                        if phase not in {"setup", "restore-race", "restore-locked-retry", "maintain-race", "maintain-locked-retry", "os-unlock-1", "os-unlock-2", "complete"}:
                            phase = "unknown"
                        key = phase + ":" + method
                        facts["calls"][key] = min(100, facts["calls"].get(key, 0) + 1)

            reader = threading.Thread(target=count_headers, daemon=True)
            reader.start()
            if not ready.wait(3) or monitor.poll() is not None:
                raise RuntimeError("monitor startup")
            env = dict(service.env, NMT_NATIVE_RACE_CONTROL=str(control))
            command = [str(binary), "identity::tests::review::r1_native_lock_race_restore_and_exact_maintenance", "--exact", "--ignored", "--nocapture"]
            facts["command"] = command
            with raw_log.open("xb") as output:
                process = subprocess.Popen(command, env=env, stdout=output, stderr=subprocess.STDOUT)
                until = time.monotonic() + 120
                handled = ""
                while process.poll() is None:
                    request = (control / "unlock-request").read_text() if (control / "unlock-request").exists() else ""
                    if request in {"1", "2"} and request != handled:
                        service.restart()  # Explicit OS-side unlock using private stdin.
                        facts["os_side_unlocks"] += 1
                        (control / "unlock-done").write_text(request)
                        handled = request
                    if time.monotonic() > until:
                        raise TimeoutError("bounded native race case")
                    time.sleep(0.02)
            facts["exit_code"] = process.returncode
            facts["final_phase"] = (control / "phase").read_text()
            NativeService.stop(monitor)
            reader.join(3)
            facts["monitor_settled"] = not reader.is_alive()
            facts["unlock_calls"] = sum(v for k, v in facts["calls"].items() if k.endswith(":Unlock"))
            facts["prompt_calls"] = sum(v for k, v in facts["calls"].items() if k.endswith(":Prompt"))
            facts["raw_log"] = str(raw_log)
            facts["log_sha256"] = hashlib.sha256(raw_log.read_bytes()).hexdigest()
        facts["private_service_cleaned"] = True
        facts["pass"] = (facts["exit_code"] == 0 and facts["final_phase"] == "complete"
                         and facts["os_side_unlocks"] == 2 and facts["monitor_settled"]
                         and facts["unlock_calls"] == 0 and facts["prompt_calls"] == 0)
    except Exception as error:
        facts.update(result="fail", error_kind=type(error).__name__, **{"pass": False})
    finally:
        NativeService.stop(process)
        NativeService.stop(monitor)
        with report.open("x") as output:
            output.write(json.dumps(facts, indent=2) + "\n")
    print(json.dumps({"pass": facts["pass"], "unlock_calls": facts.get("unlock_calls"), "prompt_calls": facts.get("prompt_calls")}))
    return 0 if facts["pass"] else 1


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--report", required=True, type=Path)
    parser.add_argument("--source-manifest", required=True, type=Path)
    parser.add_argument("--raw-log", type=Path,
                        help="Rust test log, distinct from report and controller stdout")
    args = parser.parse_args()
    raw_log = args.raw_log or args.report.with_name(args.report.stem + ".rust.txt")
    raise SystemExit(run(args.binary, args.report, args.source_manifest, raw_log))
