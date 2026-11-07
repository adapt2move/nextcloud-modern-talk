#!/usr/bin/env python3
"""Run the real call protocol owner against one private disposable lab."""
import argparse
import asyncio
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import urllib.parse

sys.dont_write_bytecode = True
from alpha_protocol import Relay
from dev import owned_state, restore
from lab import API, LabError


def run_test(lab, record):
    room = lab.request("lab_alice", "POST", API + "v4/room",
                       body={"roomType": 2, "roomName": "Call client fixture"},
                       expected=(201,), name="call-client-create")
    token = room["token"]
    lab.sensitive.append(token)
    path = API + "v4/room/" + token
    directory = None
    try:
        lab.request("lab_alice", "POST", path + "/participants",
                    body={"newParticipant": "lab_bob", "source": "users"},
                    name="call-client-invite")
        directory = Path(tempfile.mkdtemp(prefix="nmt-call-client-", dir="/run"))
        os.chmod(directory, 0o700)
        fixture = directory / "fixture.json"
        fixture.write_text(json.dumps({"origin": record["development"]["client_url"],
                                       "room": token, "alice_password": lab.passwords["lab_alice"],
                                       "bob_password": lab.passwords["lab_bob"]}))
        os.chmod(fixture, 0o600)
        env = os.environ.copy()
        env["NMT_CALL_FIXTURE"] = str(fixture)
        env["RUST_TEST_THREADS"] = "1"
        env["TMPDIR"] = "/root/call-tmp"
        result = subprocess.run([sys.executable, "tools/cargo.py", "test", "--locked",
                                 "-p", "nextcloud-talk-api", "--features", "backend-test",
                                 "--test", "call_backend", "--", "--ignored", "--nocapture"],
                                env=env, timeout=240, check=False)
        if result.returncode:
            raise LabError("Call protocol owner test failed")
    finally:
        if directory:
            shutil.rmtree(directory)
        lab.request("lab_alice", "DELETE", path, expected=(200, 404), name="call-client-cleanup")
    print(json.dumps({"profile": record["development"]["profile"], "result": "pass",
                      "scope": "Rust protocol owner and internal signaling; no media"}), flush=True)


def check(state):
    with owned_state(state) as (_, record):
        lab = restore(record)
        client = urllib.parse.urlsplit(record["development"]["client_url"])
        bridge = urllib.parse.urlsplit(record["development"]["vm_url"])

        async def run():
            async with Relay(bridge.hostname, client.port):
                lab.base = record["development"]["client_url"]
                await asyncio.to_thread(run_test, lab, record)

        asyncio.run(run())


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--state", required=True)
    args = parser.parse_args()
    try:
        check(args.state)
    except (LabError, OSError, subprocess.SubprocessError) as error:
        print(json.dumps({"result": "fail", "failure": type(error).__name__,
                          "detail": str(error) if isinstance(error, LabError) else "fixture or execution failure"}))
        raise SystemExit(1)
