#!/usr/bin/env python3
"""Capture actual compiled GPUI X11 windows under owned Xvfb + Mesa Vulkan.

Run in a development VM; never starts a browser or a backend. Captures are
explicit UI development scenarios and cannot establish server/media support.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import select
import signal
import subprocess
import tempfile
import time


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def run(argv, env, **kwargs):
    return subprocess.run(argv, env=env, check=True, text=True, capture_output=True, timeout=20, **kwargs)


def source_manifest(root):
    paths = [root / "Cargo.toml", root / "Cargo.lock", root / "rust-toolchain.toml", root / "LICENSE", root / "crates/talk-core/src/ui.rs"]
    for directory in ["apps/desktop", "crates/talk-core", "tools/ui"]:
        paths += [p for p in (root / directory).rglob("*") if p.is_file() and "__pycache__" not in p.parts]
    return {str(p.relative_to(root)): sha(p) for p in sorted(paths) if p.exists()}


def capture(args, before_runtime_cleanup=None):
    root = Path(__file__).resolve().parents[2]
    binary = Path(args.binary).resolve()
    out = Path(args.output).resolve()
    out.mkdir(parents=True, exist_ok=True)
    manifest = {"schema": 1, "kind": "compiled GPUI native X11 UI development evidence", "backend_connected": False,
        "media_engine_attached": False, "media_frames": False, "platform_qualified": False,
        "binary_sha256": sha(binary), "source_files": source_manifest(root), "captures": []}
    manifest["source_manifest_sha256"] = hashlib.sha256(json.dumps(manifest["source_files"], sort_keys=True).encode()).hexdigest()
    env = os.environ.copy()
    env.pop("WAYLAND_DISPLAY", None)
    icds = sorted(Path("/usr/share/vulkan/icd.d").glob("lvp_icd*.json"))
    if not icds:
        raise RuntimeError("Mesa lavapipe Vulkan ICD missing; run prepare-linux.sh in the VM")
    env["VK_ICD_FILENAMES"] = str(icds[0])
    with tempfile.TemporaryDirectory(prefix="talk-native-ui-") as runtime:
        env["XDG_RUNTIME_DIR"] = runtime
        read_fd, write_fd = os.pipe()
        xlog = open(out / "xvfb.log", "w")
        xvfb = subprocess.Popen(["Xvfb", "-displayfd", str(write_fd), "-screen", "0", "1600x1200x24", "-nolisten", "tcp", "-noreset"], pass_fds=(write_fd,), stdout=xlog, stderr=xlog)
        os.close(write_fd)
        try:
            if not select.select([read_fd], [], [], 15)[0]:
                raise RuntimeError("Owned Xvfb did not expose a display within 15 seconds")
            display = os.read(read_fd, 32).decode().strip()
            if not display.isdigit():
                raise RuntimeError("Xvfb returned an invalid display")
            env["DISPLAY"] = ":" + display
            manifest["vulkan"] = run(["vulkaninfo", "--summary"], env).stdout
            cases = args.scenarios.split(",")
            for case in cases:
                parts = case.split(":")
                scenario = parts[0]
                appearance = parts[1] if len(parts) > 1 else "light"
                theme = parts[2] if len(parts) > 2 else "blue"
                label = f"{scenario}-{appearance}-{theme}"
                command = [str(binary), "--ui-scenario", scenario, "--ui-appearance", appearance, "--ui-theme", theme, "--ui-font-metrics", "--ui-scale", str(args.scale)]
                if args.scale != 100: label += f"-{args.scale}pct"
                started = time.monotonic()
                with open(out / f"{label}.log", "w") as app_log:
                    app = subprocess.Popen(command, env=env, stdout=app_log, stderr=app_log, start_new_session=True)
                    try:
                        xid = None
                        deadline = started + 25
                        while time.monotonic() < deadline:
                            if app.poll() is not None:
                                raise RuntimeError(f"Native app exited {app.returncode}; inspect {label}.log")
                            probe = subprocess.run(["xdotool", "search", "--onlyvisible", "--pid", str(app.pid)], env=env, text=True, capture_output=True, timeout=2)
                            if probe.returncode == 0 and probe.stdout.strip():
                                xid = probe.stdout.strip().splitlines()[0]
                                break
                            time.sleep(0.1)
                        if xid is None:
                            raise RuntimeError(f"No visible native window: {label}")
                        first_window_ms = round((time.monotonic() - started) * 1000, 2)
                        run(["xdotool", "windowfocus", "--sync", xid], env)
                        run(["xdotool", "windowsize", "--sync", xid, str(args.width), str(args.height)], env)
                        time.sleep(2.0)
                        screenshot = out / f"{label}.png"
                        run(["import", "-window", xid, str(screenshot)], env)
                        status = Path(f"/proc/{app.pid}/status").read_text()
                        memory = {line.split(":", 1)[0]: line.split(":", 1)[1].strip() for line in status.splitlines() if line.startswith(("VmRSS:", "VmHWM:", "Threads:"))}
                        manifest["captures"].append({"scenario": scenario, "appearance": appearance, "theme": theme, "image": screenshot.name,
                            "sha256": sha(screenshot), "first_visible_window_ms": first_window_ms, "process_sample": memory,
                            "measurement_limit": "single Linux diagnostic sample; excludes GPU/Xvfb/other processes; not Q.PERF acceptance",
                            "window_size": [args.width, args.height], "command": ["talk-desktop"] + command[1:]})
                        if args.interactions:
                            if getattr(args, "interaction_suite", "i1") == "d1-2":
                                from interactions_d1_2 import exercise
                            elif getattr(args, "interaction_suite", "i1") == "d1-1":
                                from interactions_d1_1 import exercise
                            elif getattr(args, "interaction_suite", "i1") == "d1":
                                from interactions_d1 import exercise
                            else:
                                from interactions import exercise
                            manifest["captures"][-1]["interactions"] = exercise(scenario, env, xid, out, label, args.scale>=150)
                        print(label, flush=True)
                    finally:
                        if app.poll() is None:
                            os.killpg(app.pid, signal.SIGTERM)
                        try:
                            app.wait(timeout=5)
                        except subprocess.TimeoutExpired:
                            os.killpg(app.pid, signal.SIGKILL)
                            app.wait(timeout=5)
        except Exception as error:
            manifest["failure"] = str(error)[:4096]
            raise
        finally:
            if before_runtime_cleanup is not None:
                before_runtime_cleanup(env)
            os.close(read_fd)
            xvfb.terminate()
            try:
                xvfb.wait(timeout=5)
            except subprocess.TimeoutExpired:
                xvfb.kill(); xvfb.wait(timeout=5)
            xlog.close()
            (out / "scenario-manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default="target/debug/talk-desktop")
    parser.add_argument("--output", required=True)
    parser.add_argument("--scenarios", default="server,browser,storage,expired,chat,chat:dark,settings,settings:dark:sage,incoming,prejoin,call,direct-call,voice,gate,offline,denied")
    parser.add_argument("--width", type=int, default=1200)
    parser.add_argument("--height", type=int, default=800)
    parser.add_argument("--scale", type=int, choices=[100,125,150,200], default=100)
    parser.add_argument("--interactions", action="store_true", help="Run native keyboard/mouse checks; requires tesseract-ocr")
    capture(parser.parse_args())
