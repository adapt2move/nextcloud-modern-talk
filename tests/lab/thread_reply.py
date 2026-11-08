#!/usr/bin/env python3
"""Native thread editor -> account outbox -> released server -> thread pane.

Synthetic loopback account only. Xvfb/Mesa proves Linux interaction, not macOS.
Run against one owned dev.py profile at a time with a dev-probes binary.
"""
import argparse
import asyncio
import hashlib
import json
import os
from pathlib import Path
import select
import shutil
import subprocess
import tempfile
import time
from urllib.parse import urlsplit

from alpha_protocol import Relay
from dev import owned_state, restore
from lab import API, save


async def exercise(args):
    output = Path(args.output).absolute()
    output.mkdir(parents=True, exist_ok=False)
    binary = Path(args.binary).absolute()
    with owned_state(args.state) as (_, record):
        lab = restore(record)
        root = Path(tempfile.mkdtemp(prefix="nmt-thread-", dir="/run"))
        profile = root / "profile"
        profile.mkdir(mode=0o700)
        room = lab.request("lab_alice", "POST", API + "v4/room",
                           body={"roomType": 2, "roomName": "A native thread fixture"},
                           expected=(201,), name="thread.create-room")
        route = API + "v1/chat/" + room["token"]
        lab.request("lab_alice", "POST", route, body={"message": "Ordinary message"},
                    expected=(201,), name="thread.ordinary-message")
        thread = lab.request("lab_alice", "POST", route,
                             body={"message": "Native thread root", "threadTitle": "Native thread topic"},
                             expected=(201,), name="thread.create-root")
        env = dict(os.environ)
        env.pop("WAYLAND_DISPLAY", None)
        env["VK_ICD_FILENAMES"] = str(sorted(Path("/usr/share/vulkan/icd.d").glob("lvp_icd*.json"))[0])
        for key in ("XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_RUNTIME_DIR"):
            directory = root / key
            directory.mkdir(mode=0o700)
            env[key] = str(directory)
        fixture = root / "intake.json"
        save(fixture, {"origin": record["development"]["client_url"], "user": "lab_alice",
                       "password": lab.passwords["lab_alice"]})
        subprocess.run([str(binary), "--dev-seed", str(profile), str(fixture)], check=True,
                       stdout=subprocess.DEVNULL)
        fixture.unlink()
        read_fd, write_fd = os.pipe()
        xvfb = subprocess.Popen(["Xvfb", "-displayfd", str(write_fd), "-screen", "0", "1440x900x24",
                                 "-nolisten", "tcp", "-noreset"], pass_fds=(write_fd,),
                                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        os.close(write_fd)
        process = None
        traces, acks = [], {}
        sequence = 0
        result = {"profile": record["development"]["profile"],
                  "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
                  "scope": "Linux GPUI/Xvfb/Mesa, synthetic released-server account; no macOS qualification"}
        try:
            if not select.select([read_fd], [], [], 15)[0]:
                raise RuntimeError("Xvfb did not start")
            env["DISPLAY"] = ":" + os.read(read_fd, 32).decode().strip()
            parsed = urlsplit(record["development"]["client_url"])
            async with Relay(urlsplit(lab.base).hostname, parsed.port):
                process = await asyncio.create_subprocess_exec(
                    str(binary), "--dev-profile", str(profile), "--dev-latency", "--ui-locale", "de",
                    env=env, stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.STDOUT)

                async def read():
                    while line := await process.stdout.readline():
                        text = line.decode(errors="replace").strip()
                        if text.startswith("DEV_"):
                            traces.append(text)
                            with (output / "native.trace").open("a") as trace:
                                trace.write(text + "\n")
                        if text.startswith("DEV_ACK "):
                            _, number, value = text.split(" ", 2)
                            acks[int(number)] = value

                reader = asyncio.create_task(read())

                async def act(action):
                    nonlocal sequence
                    sequence += 1
                    command = profile / "probe.command"
                    temporary = profile / "probe.next"
                    temporary.write_text(f"{sequence} {action}\n")
                    temporary.replace(command)
                    if action == "quit":
                        return "quitting"
                    deadline = time.monotonic() + 10
                    while sequence not in acks:
                        if process.returncode is not None or time.monotonic() >= deadline:
                            raise RuntimeError("Native probe acknowledgement missing")
                        await asyncio.sleep(.05)
                    return acks[sequence]

                async def wait_for(action, predicate):
                    deadline = time.monotonic() + 120
                    while time.monotonic() < deadline:
                        value = await act(action)
                        if predicate(value):
                            return value
                        await asyncio.sleep(.25)
                    raise RuntimeError(f"Native {action} did not settle: {value}")

                await wait_for("state", lambda value: int(value.split()[0]) > 0)
                await act("open-thread-room")
                await wait_for("state", lambda value: value.endswith("ready") and value.split()[1] != "0")
                found = subprocess.check_output(["xdotool", "search", "--name", "^NextCloud Talk$"], env=env, text=True)
                xid, = found.splitlines()
                subprocess.run(["xdotool", "windowfocus", "--sync", xid], env=env, check=True)
                await act("open-thread")
                result["opened"] = await wait_for("thread-state", lambda value: value.startswith("Idle "))
                await asyncio.sleep(.5)
                subprocess.run(["import", "-window", xid, str(output / "thread-before.png")], env=env, check=True)
                width, height = map(int, subprocess.check_output(
                    ["identify", "-format", "%w %h", str(output / "thread-before.png")], text=True).split())
                subprocess.run(["xdotool", "mousemove", "--window", xid, str(width - 240), str(height - 100), "click", "1"], env=env, check=True)
                subprocess.run(["xdotool", "type", "--clearmodifiers", "--delay", "35", "Native thread reply"], env=env, check=True)
                await asyncio.sleep(.2)
                subprocess.run(["import", "-window", xid, str(output / "thread-typed.png")], env=env, check=True)
                result["typed"] = await act("thread-state")
                if not result["typed"].endswith("false"):
                    raise RuntimeError("Native typing did not reach the thread editor")
                subprocess.run(["xdotool", "key", "--clearmodifiers", "Return"], env=env, check=True)
                result["pane"] = await wait_for("thread-state", lambda value: value.startswith("Idle ") and value.split()[2:] == ["1", "true"])
                rows = lab.request("lab_alice", "GET", route,
                                   query={"lookIntoFuture": 0, "limit": 50, "threadId": thread["id"], "setReadMarker": 0},
                                   name="thread.verify-server")
                replies = [row for row in rows if row.get("message") == "Native thread reply"]
                if len(replies) != 1 or replies[0].get("threadId") != thread["id"]:
                    raise RuntimeError("Expected exactly one canonical server reply in the requested thread")
                result["server_reply_count"] = len(replies)
                result["passed"] = True
                await asyncio.sleep(.5)
                subprocess.run(["import", "-window", xid, str(output / "thread-after.png")], env=env, check=True)
                await act("quit")
                await asyncio.wait_for(process.wait(), 15)
                await reader
        finally:
            if process is not None and process.returncode is None:
                process.kill()
                await process.wait()
            xvfb.terminate()
            xvfb.wait(timeout=10)
            os.close(read_fd)
            (output / "native.trace").write_text("\n".join(traces) + "\n")
            (output / "result.json").write_text(json.dumps(result, indent=2) + "\n")
            lab.request("lab_alice", "DELETE", API + "v4/room/" + room["token"],
                        expected=(200, 404), name="thread.cleanup-room")
            shutil.rmtree(root)
        print(json.dumps(result), flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--state", required=True)
    parser.add_argument("--binary", required=True)
    parser.add_argument("--output", required=True)
    asyncio.run(exercise(parser.parse_args()))
