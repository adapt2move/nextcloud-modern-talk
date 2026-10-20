#!/usr/bin/env python3
"""I2 ordinary-profile matrix on two explicitly owned Talk labs.

Four fresh calls per family, at most two additional calls only to observe a
missing actual role, then one explicit remote-call-leave per family. A failed
call/read is never replayed or removed. This is author evidence, not a gate.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for key in ("candidate", "archive", "manifest", "verification", "bridge", "helper", "r34", "r35", "output"):
        parser.add_argument("--" + key, type=Path, required=True)
    args = parser.parse_args()
    if args.output.exists():
        raise SystemExit("new output directory required")
    inputs = json.loads(args.manifest.read_text())
    checks = json.loads(args.verification.read_text())
    if checks["result"] != "pass":
        raise SystemExit("completed source-bound verification required")
    binding = {"schema":2, "candidate_manifest_sha256":sha(args.manifest),
        "candidate_archive_sha256":sha(args.archive), "verification_sha256":sha(args.verification),
        "runner_sha256":sha(__file__), "bridge_sha256":sha(args.bridge), "helper_sha256":sha(args.helper),
        "browser_harness_sha256":sha(args.candidate / "tools/media/talk_browser.py"),
        "plan":{"network":"default", "exchange_minimum_per_family":4, "exchange_maximum_per_family":6,
                "extra_call_condition":"missing actual negotiated role only, independent of pass/fail",
                "remote_leave_per_family":1, "absolute_synthetic_seconds":30,
                "failed_destructive_poll_replay":False, "new_calls_use_fresh_room_scope_and_explicit_intent":True,
                "abort_matrix_on_setup_or_cleanup_failure":True},
        "result":"running", "runs":[]}
    if any(binding[k] != checks[k] for k in ("bridge_sha256", "helper_sha256")):
        raise SystemExit("compiled artifact mismatch")

    def validate():
        for name, digest in inputs["files"].items():
            if sha(args.candidate / name) != digest:
                raise RuntimeError("frozen source changed")
        if sha(args.bridge) != binding["bridge_sha256"] or sha(args.helper) != binding["helper_sha256"]:
            raise RuntimeError("artifact changed")

    args.output.mkdir(parents=True)

    def save():
        (args.output / "matrix.json").write_text(json.dumps(binding, indent=2) + "\n")

    def run(family, scenario, attempt):
        validate()
        name = f"{family}-{scenario}-{attempt}"
        path = args.output / (name + ".json")
        log_path = args.output / (name + ".log")
        argv = [sys.executable, str(args.candidate / "tools/media/talk_browser.py"),
                "--manifest", str(getattr(args, family)), "--bridge", str(args.bridge),
                "--helper", str(args.helper), "--browser-network", "default",
                "--scenario", scenario, "--output", str(path)]
        entry = {"family":family, "scenario":scenario, "attempt":attempt, "network":"default",
                 "file":path.name, "argv":argv, "started_unix":time.time(), "result":"running"}
        binding["runs"].append(entry)
        save()
        try:
            with log_path.open("wb") as log:
                completed = subprocess.run(argv, stdout=log, stderr=subprocess.STDOUT, timeout=180,
                    env=dict(os.environ, PYTHONDONTWRITEBYTECODE="1"))
            report = json.loads(path.read_text())
            native = report.get("native", {})
            role = native.get("native_offerer")
            entry.update(sha256=sha(path), exit=completed.returncode, result=report["result"],
                         native_offerer=role if isinstance(role, bool) else None,
                         stage=report.get("stage"), room_cleanup=report.get("room_cleanup"),
                         departure=native.get("remote_departure"), local_stop=native.get("stop"),
                         server_leave=native.get("leave"), conversation_deactivate=native.get("deactivate"))
        except Exception as error:
            entry.update(result="fail", failure_type=type(error).__name__, native_offerer=None)
        entry["ended_unix"] = time.time()
        if log_path.exists():
            entry["log_sha256"] = sha(log_path)
        save()
        if entry.get("stage") != "native browser exchange" or entry.get("room_cleanup") != "absence_confirmed":
            raise RuntimeError("setup or cleanup failure; no further calls")
        return entry

    validate()
    save()
    try:
        for family in ("r34", "r35"):
            roles = set()
            for attempt in range(1, 7):
                entry = run(family, "exchange", attempt)
                if isinstance(entry["native_offerer"], bool):
                    roles.add(entry["native_offerer"])
                if attempt >= 4 and len(roles) == 2:
                    break
            run(family, "remote-leave", 1)
        validate()
        cells = [{"family":family, "native_offerer":role,
                  "pass_count":sum(r["family"] == family and r["scenario"] == "exchange"
                      and r["native_offerer"] is role and r["result"] == "pass" for r in binding["runs"])}
                 for family in ("r34", "r35") for role in (True, False)]
        departure = [r for r in binding["runs"] if r["scenario"] == "remote-leave"]
        observations = [(r.get("departure") or {}).get("observation") or {} for r in departure]
        stayed = all(obs.get("selected_peer_present") is True and obs.get("selected_peer_in_call") is False
                     for obs in observations)
        binding.update(result="completed-observations", actual_role_cells=cells,
                       all_calls_passed=all(r["result"] == "pass" for r in binding["runs"]),
                       all_four_actual_roles_passed=all(cell["pass_count"] > 0 for cell in cells),
                       both_call_departures_while_present=stayed and len(departure) == 2)
    except Exception as error:
        binding.update(result="incomplete", failure_type=type(error).__name__)
    finally:
        save()
    passed = all(binding.get(key) is True for key in
                 ("all_calls_passed", "all_four_actual_roles_passed", "both_call_departures_while_present"))
    print(json.dumps({"result":binding["result"], "calls":len(binding["runs"]), "author_matrix_pass":passed}))
    raise SystemExit(0 if passed else 1)


if __name__ == "__main__":
    main()
