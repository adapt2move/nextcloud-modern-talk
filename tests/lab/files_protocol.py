#!/usr/bin/env python3
"""Exercise the account file protocol against an owned development lab."""
import argparse
import asyncio
import json
import os
from pathlib import Path
import shutil
import sys
import tempfile
import urllib.parse

sys.dont_write_bytecode = True
from alpha_protocol import Relay
from dev import owned_state, restore


async def run(state, output):
    root = Path(__file__).resolve().parents[2]
    with owned_state(state) as (_, record):
        lab = restore(record)
        room = lab.request("lab_alice", "POST", "/ocs/v2.php/apps/spreed/api/v4/room",
                           body={"roomType": 2, "roomName": "Files fixture"}, expected=(201,),
                           name="files-create-room")
        directory = Path(tempfile.mkdtemp(prefix="nmt-files-protocol-", dir="/run"))
        os.chmod(directory, 0o700)
        intake = directory / "alice.json"
        intake.write_text(json.dumps({"origin": record["development"]["client_url"],
                                      "user": "lab_alice", "password": lab.passwords["lab_alice"],
                                      "room": room}))
        os.chmod(intake, 0o600)
        client = urllib.parse.urlsplit(record["development"]["client_url"])
        host = urllib.parse.urlsplit(record["development"]["vm_url"]).hostname
        env = dict(os.environ, NMT_FIXTURE_DIR=str(directory), RUST_BACKTRACE="0")
        env["PATH"] = "/root/.cargo/bin:" + env.get("PATH", "")
        target = Path(output)
        target.mkdir(mode=0o700, parents=True, exist_ok=False)
        try:
            async with Relay(host, client.port):
                with (target / "backend.log").open("w") as log:
                    process = await asyncio.create_subprocess_exec(
                        "python3", "tools/cargo.py", "test", "--locked", "-p", "talk-adapters",
                        "--features", "backend-test", "--test", "content_backend",
                        "released_server_file_listing_and_existing_share", "--", "--ignored", "--nocapture",
                        cwd=root, env=env, stdout=log, stderr=asyncio.subprocess.STDOUT)
                    try:
                        code = await asyncio.wait_for(process.wait(), 600)
                    except BaseException:
                        process.kill()
                        await process.wait()
                        raise
            print(json.dumps({"profile": record["development"]["profile"], "exit_code": code}), flush=True)
            return code
        finally:
            shutil.rmtree(directory)
            lab.request("lab_alice", "DELETE", "/ocs/v2.php/apps/spreed/api/v4/room/" + room["token"],
                        expected=(200, 404), name="files-cleanup-room")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--state", required=True)
    parser.add_argument("--output", required=True)
    args = parser.parse_args()
    sys.exit(asyncio.run(run(args.state, args.output)))
