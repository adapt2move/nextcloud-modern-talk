#!/usr/bin/env python3
"""Exercise public-room admission against an isolated pinned lab server."""
import argparse
import asyncio
import hashlib
import os
from pathlib import Path
import signal
import sys
import time
from urllib.parse import urlsplit

sys.dont_write_bytecode = True
from alpha_protocol import Relay
from lab import Lab, LabInterrupted, cleanup, now, save

ROOT = Path(__file__).resolve().parents[2]


class PublicRoomLab(Lab):
    def __init__(self, args):
        super().__init__(args)
        self.native_binary = args.native_binary

    def smoke(self, key, pin):
        super().smoke(key, pin)
        port = 47409
        self.http_host = f"127.0.0.1:{port}"
        directory = self.runtime / f"{key}-public-room"
        directory.mkdir(mode=0o700)
        for variant in ("public", "password", "lobby", "listed"):
            password = "Synthetic room password" if variant == "password" else ""
            room = self.request("lab_alice", "POST", "/ocs/v2.php/apps/spreed/api/v4/room",
                body={"roomType": 3, "roomName": f"Public access {variant}", "password": password},
                expected=(201,), name=f"public.{variant}.create")
            token = room["token"]
            if variant == "lobby":
                self.request("lab_alice", "PUT", f"/ocs/v2.php/apps/spreed/api/v4/room/{token}/webinar/lobby",
                    body={"state": 1}, name="public.lobby.enable")
            if variant == "listed":
                self.request("lab_alice", "PUT", f"/ocs/v2.php/apps/spreed/api/v4/room/{token}/listable",
                    body={"scope": 1}, name="public.listed.enable")
            self.request("lab_alice", "POST", f"/ocs/v2.php/apps/spreed/api/v1/chat/{token}",
                body={"message": "Public room history sentinel"}, expected=(201,), name=f"public.{variant}.seed")
            before = self.request("lab_bob", "GET", f"/ocs/v2.php/apps/spreed/api/v4/room/{token}", name=f"public.{variant}.before")
            self.require(f"public.{variant}.not-participant", before.get("participantType") in (0, 4), {"participant_type": before.get("participantType")})
            self.request("lab_bob", "GET", f"/ocs/v2.php/apps/spreed/api/v1/chat/{token}",
                query={"lookIntoFuture": 0, "limit": 50}, expected=(404,), name=f"public.{variant}.history-before-join")
            save(directory / f"{variant}.json", {"origin": f"http://127.0.0.1:{port}", "user": "lab_bob",
                "password": self.passwords["lab_bob"], "room": before})
        target = self.output / key
        target.mkdir(mode=0o700)
        result = asyncio.run(self.client(directory, port, target))
        self.require("public.typed-client", result == 0, {"exit_code": result})
        self.current.update(result="pass", finished=now())

    async def client(self, directory, port, output):
        command = ["cargo", "test", "--locked", "-p", "talk-adapters", "--features", "backend-test",
                   "--test", "public_room_backend", "--", "--ignored", "--nocapture", "--test-threads=1"]
        env = dict(os.environ, NMT_FIXTURE_DIR=str(directory), RUST_BACKTRACE="0")
        start = time.monotonic()
        async with Relay(urlsplit(self.base).hostname, port):
            with (output / "backend.log").open("w") as log:
                process = await asyncio.create_subprocess_exec(*command, cwd=ROOT, env=env,
                                                               stdout=log, stderr=asyncio.subprocess.STDOUT)
                try:
                    code = await asyncio.wait_for(process.wait(), 600)
                except BaseException:
                    process.kill()
                    await process.wait()
                    raise
            if code == 0 and self.native_binary:
                with (output / "native-driver.log").open("w") as log:
                    native = await asyncio.create_subprocess_exec("xvfb-run", "-a", "-s", "-screen 0 1280x900x24", "dbus-run-session", "--", "python3", str(ROOT / "tests/lab/public_room_native.py"), "--binary", self.native_binary, "--fixture", str(directory / "password.json"), "--output", str(output / "native"), stdout=log, stderr=asyncio.subprocess.STDOUT, start_new_session=True)
                    try:
                        code = await asyncio.wait_for(native.wait(), 240)
                    except BaseException:
                        os.killpg(native.pid, signal.SIGTERM)
                        await native.wait()
                        raise
                if code == 0:
                    import json
                    room = json.loads((directory / "password.json").read_text())["room"]
                    messages = self.request("lab_alice", "GET", f"/ocs/v2.php/apps/spreed/api/v1/chat/{room['token']}", query={"lookIntoFuture": 0, "limit": 50}, name="public.native.canonical-send")
                    self.require("public.native.canonical-send", any(message.get("message") == "Native public room reply" for message in messages))
        save(output / "client-result.json", {"command": command, "exit_code": code,
                                            "seconds": round(time.monotonic() - start, 2)})
        return code


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--profile", choices=("all", "r34", "r35"), default="r35")
    parser.add_argument("--output", required=True)
    parser.add_argument("--native-binary")
    parser.add_argument("--budget-seconds", type=int, default=2400)
    args = parser.parse_args()
    os.umask(0o077)
    lab = PublicRoomLab(args)
    lab.report["scope"] = "Released-server public-room admission, password, lobby, reading, writing and leaving through the typed adapter; no native UI claim"
    lab.report["client_source_sha256"] = {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest()
        for base in ("crates", "apps") for p in sorted((ROOT / base).rglob("*.rs"))}
    lab.report["harness_sha256"] = {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest() for p in (Path(__file__), ROOT / "tests/lab/public_room_native.py")}
    print("recovery state: " + str(lab.runtime / "state.json"), flush=True)

    def interrupted(signum, frame):
        raise LabInterrupted("Public-room lab interrupted")

    signal.signal(signal.SIGINT, interrupted)
    signal.signal(signal.SIGTERM, interrupted)
    try:
        lab.run(list(lab.pins["profiles"]) if args.profile == "all" else [args.profile])
    except Exception as error:
        lab.report.update(result="fail", error=lab.redact(str(error)))
    finally:
        signal.signal(signal.SIGINT, signal.SIG_IGN)
        signal.signal(signal.SIGTERM, signal.SIG_IGN)
        lab.report["cleanup"] = cleanup(lab.runtime / "state.json")
        if lab.report["cleanup"]["result"] != "pass":
            lab.report["result"] = "fail"
        lab.report["finished"] = now()
        save(lab.output / "report.json", lab.report)
    print(lab.report["result"], flush=True)
    return 0 if lab.report["result"] == "pass" else 1


if __name__ == "__main__":
    sys.exit(main())
