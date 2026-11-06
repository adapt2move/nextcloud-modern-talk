#!/usr/bin/env python3
"""Exercise direct-message presence against an isolated pinned lab server."""
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


class PresenceLab(Lab):
    def smoke(self, key, pin):
        super().smoke(key, pin)
        port = 47408
        self.http_host = f"127.0.0.1:{port}"
        caps = self.request("lab_alice", "GET", "/ocs/v2.php/cloud/capabilities", name="presence.capabilities")
        self.require("presence.enabled", caps.get("capabilities", {}).get("user_status", {}).get("enabled") is True)
        room = self.request("lab_alice", "POST", "/ocs/v2.php/apps/spreed/api/v4/room",
                            body={"roomType": 1, "invite": "lab_bob"}, expected=(200, 201), name="presence.direct")
        self.request("lab_bob", "PUT", "/ocs/v2.php/apps/user_status/api/v1/user_status/status",
                     body={"statusType": "away"}, name="presence.set-away")
        self.request("lab_bob", "PUT", "/ocs/v2.php/apps/user_status/api/v1/user_status/message/custom",
                     body={"statusIcon": "☕", "message": "Presence fixture", "clearAt": None}, name="presence.set-message")
        directory = self.runtime / f"{key}-presence"
        directory.mkdir(mode=0o700)
        save(directory / "alice.json", {"origin": f"http://127.0.0.1:{port}", "user": "lab_alice",
                                      "password": self.passwords["lab_alice"], "room": room})
        target = self.output / key
        target.mkdir(mode=0o700)
        result = asyncio.run(self.client(directory, port, target))
        self.require("presence.typed-client", result == 0, {"exit_code": result})
        self.current.update(result="pass", finished=now())

    async def client(self, directory, port, output):
        command = ["cargo", "test", "--locked", "-p", "talk-adapters", "--features", "backend-test",
                   "--test", "presence_backend", "--", "--ignored", "--nocapture"]
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
        save(output / "client-result.json", {"command": command, "exit_code": code,
                                            "seconds": round(time.monotonic() - start, 2)})
        return code


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--profile", choices=("all", "r34", "r35"), default="r35")
    parser.add_argument("--output", required=True)
    parser.add_argument("--budget-seconds", type=int, default=2400)
    args = parser.parse_args()
    os.umask(0o077)
    lab = PresenceLab(args)
    lab.report["scope"] = "Released-server direct-message status read through the typed adapter; no native UI claim"
    lab.report["client_source_sha256"] = {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest()
        for base in ("crates", "apps") for p in sorted((ROOT / base).rglob("*.rs"))}
    print("recovery state: " + str(lab.runtime / "state.json"), flush=True)

    def interrupted(signum, frame):
        raise LabInterrupted("Presence lab interrupted")

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
