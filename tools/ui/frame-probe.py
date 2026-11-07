#!/usr/bin/env python3
"""Compare bounded native history scrolling on two instrumented source copies.

Run in an isolated Linux development VM. This diagnostic times GPUI's CPU
window-draw boundary; it does not measure presentation, GPU time, or Q.PERF.
Both copies receive the same 100-row fixture and minimal timing instrumentation.
"""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import re
import shutil
import statistics
import subprocess
import tempfile
import time
from types import SimpleNamespace

import capture
import interactions

ROOT = Path(__file__).resolve().parents[2]


def replace_once(path, old, new):
    text = path.read_text()
    if text.count(old) != 1:
        raise RuntimeError(f"Expected one instrumentation site in {path}")
    path.write_text(text.replace(old, new, 1))


def instrument(tree):
    fixture = tree / "apps/desktop/src/composition/fixtures.rs"
    # Both revisions must exercise the same text and glyphs, even when the
    # candidate has improved the user-facing fixture copy.
    old_copy = '"Fiktiver Verlauf für die lokale Navigation."'
    workload_copy = '"Notizen aus der gemeinsamen Planung."'
    fixture_text = fixture.read_text()
    if old_copy in fixture_text:
        replace_once(fixture, old_copy, workload_copy)
    elif fixture_text.count(workload_copy) != 1:
        raise RuntimeError(f"Expected the reviewed history text in {fixture}")
    replace_once(fixture,
        'if scenario.starts_with("history") {\n        s.messages = page((1..=40).map(|n| {',
        'if scenario.starts_with("history") {\n        s.messages = page((1..=100).map(|n| {')
    replace_once(fixture,
        '        if scenario == "history-pagination"\n',
        '        assert_eq!(s.messages.as_slice().len(), 100);\n'
        '        eprintln!("MATERIAL_FIXTURE_MESSAGES 100");\n'
        '        if scenario == "history-pagination"\n')
    window = tree / "vendor/gpui/src/window.rs"
    replace_once(window,
        '    pub fn draw(&mut self, cx: &mut App) -> ArenaClearNeeded {\n',
        '    pub fn draw(&mut self, cx: &mut App) -> ArenaClearNeeded {\n'
        '        let material_probe = std::env::var_os("TALK_MATERIAL_FRAME_PROBE")\n'
        '            .is_some().then(std::time::Instant::now);\n')
    replace_once(window,
        '        self.needs_present.set(true);\n\n        ArenaClearNeeded\n',
        '        self.needs_present.set(true);\n'
        '        if let Some(started) = material_probe {\n'
        '            let elapsed = started.elapsed().as_nanos();\n'
        '            let timestamp = std::time::SystemTime::now()\n'
        '                .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();\n'
        '            eprintln!("MATERIAL_FRAME_NS {timestamp} {elapsed}");\n'
        '        }\n\n        ArenaClearNeeded\n')
    x11 = tree / "vendor/gpui/src/platform/linux/x11/window.rs"
    replace_once(x11,
        '        inner.renderer.draw(scene);\n',
        '        let material_probe = std::env::var_os("TALK_MATERIAL_FRAME_PROBE")\n'
        '            .is_some().then(std::time::Instant::now);\n'
        '        inner.renderer.draw(scene);\n'
        '        if let Some(started) = material_probe {\n'
        '            let elapsed = started.elapsed().as_nanos();\n'
        '            let timestamp = std::time::SystemTime::now()\n'
        '                .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();\n'
        '            eprintln!("MATERIAL_SUBMIT_NS {timestamp} {elapsed}");\n'
        '        }\n')


def sha(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def build(tree, target, output):
    environment = os.environ.copy()
    environment["CARGO_TARGET_DIR"] = str(target)
    command = ["cargo", "build", "--locked", "--release", "-p", "talk-desktop", "-j", "2"]
    with output.open("w") as log:
        subprocess.run(command, cwd=tree, env=environment, stdout=log, stderr=subprocess.STDOUT,
                       check=True, timeout=7200)
    return target / "release/talk-desktop"


def history_numbers(path, env):
    text = capture.run(["tesseract", str(path), "stdout", "--psm", "6"], env).stdout
    numbers = [int(value) for value in re.findall(r"Geladene\s+Nachricht\s+(\d{1,3})", text, re.I)]
    return text, numbers


def measure(binary, output, preview):
    observation = {}

    def exercise(scenario, env, xid, out, label, large=False):
        del scenario, large
        # Glyphs, images and shaders are warm before the timed input sequence.
        time.sleep(2)
        initial = out / "scroll-initial.png"
        capture.run(["import", "-window", xid, str(initial)], env)
        initial_text, initial_numbers = history_numbers(initial, env)
        capture.run(["xdotool", "mousemove", "--window", xid, "800", "400"], env)
        start = time.time_ns()
        capture.run(["xdotool", "click", "--repeat", "60", "--delay", "35", "4"], env)
        time.sleep(.4)
        middle = time.time_ns()
        top = out / "scroll-older.png"
        capture.run(["import", "-window", xid, str(top)], env)
        top_text, top_numbers = history_numbers(top, env)
        newer_start = time.time_ns()
        capture.run(["xdotool", "click", "--repeat", "60", "--delay", "35", "5"], env)
        time.sleep(.4)
        end = time.time_ns()
        final = out / "scroll-returned.png"
        capture.run(["import", "-window", xid, str(final)], env)
        final_text, final_numbers = history_numbers(final, env)
        if not initial_numbers or not top_numbers or not final_numbers:
            raise AssertionError("OCR did not observe message IDs at all three scroll positions")
        if max(initial_numbers) < 90 or min(top_numbers) >= min(initial_numbers) or max(final_numbers) <= max(top_numbers):
            raise AssertionError(f"Scroll did not reach older and then newer rows: {initial_numbers}, {top_numbers}, {final_numbers}")
        pid = int(capture.run(["xdotool", "getwindowpid", xid], env).stdout)
        status = Path(f"/proc/{pid}/status").read_text()
        memory = {line.split(":", 1)[0]: line.split(":", 1)[1].strip()
                  for line in status.splitlines() if line.startswith(("VmRSS:", "VmHWM:", "Threads:"))}
        observation.update({"input_start_unix_ns": start, "older_done_unix_ns": middle,
                            "newer_start_unix_ns": newer_start,
                            "input_end_unix_ns": end, "scheduled_wheel_events": 120,
                            "initial_numbers": initial_numbers, "older_numbers": top_numbers,
                            "returned_numbers": final_numbers, "process_status": memory,
                            "ocr": {"initial": initial_text, "older": top_text, "returned": final_text}})
        return [{"check": "100-row fixture scrolls to older and back to newer messages", "result": "pass"}]

    interactions.exercise = exercise
    os.environ["TALK_MATERIAL_FRAME_PROBE"] = "1"
    if preview:
        os.environ["TALK_UI_MATERIAL_PREVIEW"] = "1"
    else:
        os.environ.pop("TALK_UI_MATERIAL_PREVIEW", None)
    args = SimpleNamespace(binary=str(binary), output=str(output), scenarios="history", scale=100,
                           width=1280, height=800, settle_seconds=0, call_fullscreen=False, interactions=True)
    capture.capture(args)
    log = (output / "history-light-blue.log").read_text()
    if "MATERIAL_FIXTURE_MESSAGES 100" not in log:
        raise AssertionError("The instrumented 100-message fixture was not observed")
    phases = [(observation["input_start_unix_ns"], observation["older_done_unix_ns"]),
              (observation["newer_start_unix_ns"], observation["input_end_unix_ns"])]
    def read_samples(marker):
        return [[(int(stamp), int(elapsed) / 1_000_000) for stamp, elapsed in
                 re.findall(rf"{marker} (\d+) (\d+)", log) if start <= int(stamp) <= end]
                for start, end in phases]
    def summarize(samples):
        if not samples:
            raise AssertionError("No frame samples were recorded during input")
        ordered = sorted(samples)
        return {"count": len(samples), "median": statistics.median(samples),
                "p95": ordered[math.ceil(.95 * len(ordered)) - 1], "max": max(samples)}
    draw = read_samples("MATERIAL_FRAME_NS")
    submit = read_samples("MATERIAL_SUBMIT_NS")
    samples = [elapsed for phase in draw for _, elapsed in phase]
    submissions = [elapsed for phase in submit for _, elapsed in phase]
    intervals = [(later[0] - earlier[0]) / 1_000_000 for phase in submit
                 for earlier, later in zip(phase, phase[1:])]
    observation["cpu_draw_ms"] = summarize(samples)
    observation["renderer_submit_ms"] = summarize(submissions)
    observation["submit_intervals_ms"] = summarize(intervals)
    observation["binary_sha256"] = sha(binary)
    observation["transparency_preview"] = preview
    observation["raw_cpu_draw_ms"] = samples
    observation["raw_renderer_submit_ms"] = submissions
    observation["raw_submit_intervals_ms"] = intervals
    (output / "frame-observation.json").write_text(json.dumps(observation, indent=2) + "\n")
    return observation


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", type=Path, required=True, help="unmodified integration source tree")
    parser.add_argument("--baseline-id", required=True, help="integration revision or source identifier")
    parser.add_argument("--candidate-id", required=True, help="candidate revision and dirty-state identifier")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--runs", type=int, default=3, choices=range(1, 6))
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    metadata = {"platform": platform.platform(), "python": platform.python_version(),
                "baseline_id": args.baseline_id,
                # VM workspace synchronization need not include Git metadata.
                # Record the caller's revision and the exact instrumented inputs.
                "candidate_id": args.candidate_id,
                "candidate_fixture_sha256": sha(ROOT / "apps/desktop/src/composition/fixtures.rs"),
                "candidate_window_sha256": sha(ROOT / "vendor/gpui/src/window.rs"),
                "measurement": "GPUI Window::draw and X11 renderer CPU submission durations and intervals; excludes GPU execution and actual presentation",
                "workload": "100 loaded synthetic history rows, 60 wheel events up then 60 down, 35 ms cadence",
                "limitations": "Xvfb and Mesa software Vulkan; scheduled events are not individual completion counts; no network or real media"}
    # Release builds do not fit in small tmpfs-backed /tmp mounts.
    with tempfile.TemporaryDirectory(prefix="talk-frame-probe-", dir=output) as temporary:
        work = Path(temporary)
        binaries = {}
        for name, source in [("baseline", args.baseline.resolve()), ("candidate", ROOT)]:
            tree = work / name
            shutil.copytree(source, tree, ignore=shutil.ignore_patterns(".git", "target", "__pycache__", ".agent-dev-box"))
            instrument(tree)
            binaries[name] = build(tree, work / f"target-{name}", output / f"build-{name}.log")
        results = {}
        for name, preview in [("baseline", False), ("candidate", True)]:
            results[name] = []
            for index in range(args.runs):
                run_dir = output / f"{name}-{index + 1}"
                run_dir.mkdir(exist_ok=True)
                results[name].append(measure(binaries[name], run_dir, preview))
        metadata["results"] = results
    (output / "summary.json").write_text(json.dumps(metadata, indent=2) + "\n")


if __name__ == "__main__":
    main()
