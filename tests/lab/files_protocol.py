#!/usr/bin/env python3
"""Exercise the account file protocol against an owned development lab."""
import argparse
import asyncio
import base64
import json
import os
from pathlib import Path
import shutil
import sys
import tempfile
import urllib.parse
import urllib.request

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
        file_url = lab.base + "/remote.php/dav/files/lab_alice/nmt-files-fixture.txt"
        thread_file_url = lab.base + "/remote.php/dav/files/lab_alice/nmt-files-thread.txt"
        folder_url = lab.base + "/remote.php/dav/files/lab_alice/nmt-files-pages"
        auth = base64.b64encode(("lab_alice:" + lab.passwords["lab_alice"]).encode()).decode()
        headers = {"Authorization": "Basic " + auth, "Host": lab.http_host, "If-None-Match": "*"}
        target = Path(output)
        file_created = False
        thread_file_created = False
        folder_created = False
        try:
            target.mkdir(mode=0o700, parents=True, exist_ok=False)
            with lab.clients["lab_alice"].open(urllib.request.Request(file_url, data=b"Fictional file browser fixture\n", headers=headers, method="PUT"), timeout=20) as response:
                if response.status != 201:
                    raise RuntimeError("Could not create the disposable DAV fixture")
                file_created = True
            with lab.clients["lab_alice"].open(urllib.request.Request(thread_file_url, data=b"Fictional thread fixture\n", headers=headers, method="PUT"), timeout=20) as response:
                if response.status != 201:
                    raise RuntimeError("Could not create the disposable thread fixture")
                thread_file_created = True
            with lab.clients["lab_alice"].open(urllib.request.Request(folder_url, headers=headers, method="MKCOL"), timeout=20) as response:
                if response.status != 201:
                    raise RuntimeError("Could not create the disposable DAV folder")
                folder_created = True
            for number in range(85):
                url = f"{folder_url}/page-{number:03}.txt"
                with lab.clients["lab_alice"].open(urllib.request.Request(url, data=b"fixture", headers=headers, method="PUT"), timeout=20) as response:
                    if response.status != 201:
                        raise RuntimeError("Could not create a disposable DAV page item")
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
            if code == 0:
                capabilities = lab.request("lab_alice", "GET", "/ocs/v2.php/cloud/capabilities",
                                           name="files-share-capabilities")
                features = capabilities["capabilities"]["spreed"]["features"]
                if "silent-send-state" in features:
                    history = lab.request("lab_alice", "GET", "/ocs/v2.php/apps/spreed/api/v1/chat/" + room["token"],
                                          query={"lookIntoFuture": 0, "limit": 50, "setReadMarker": 0},
                                          name="files-share-history")
                    if not any(item.get("message") == "Fixture caption" and item.get("silent") is True
                               for item in history):
                        raise RuntimeError("The server did not retain the fixture share's silent state")
                    print("files-backend silent-share-state: pass", flush=True)
                else:
                    print("files-backend silent-share-state: unavailable on this server", flush=True)
            print(json.dumps({"profile": record["development"]["profile"], "exit_code": code}), flush=True)
            return code
        finally:
            shutil.rmtree(directory, ignore_errors=True)
            cleanup_headers = {"Authorization": "Basic " + auth, "Host": lab.http_host}
            try:
                if folder_created:
                    with lab.clients["lab_alice"].open(urllib.request.Request(folder_url, headers=cleanup_headers, method="DELETE"), timeout=20) as response:
                        if response.status not in (200, 204):
                            raise RuntimeError("Could not delete the disposable DAV folder")
            finally:
                try:
                    if file_created:
                        with lab.clients["lab_alice"].open(urllib.request.Request(file_url, headers=cleanup_headers, method="DELETE"), timeout=20) as response:
                            if response.status not in (200, 204):
                                raise RuntimeError("Could not delete the disposable DAV fixture")
                finally:
                    try:
                        if thread_file_created:
                            with lab.clients["lab_alice"].open(urllib.request.Request(thread_file_url, headers=cleanup_headers, method="DELETE"), timeout=20) as response:
                                if response.status not in (200, 204):
                                    raise RuntimeError("Could not delete the disposable thread fixture")
                    finally:
                        lab.request("lab_alice", "DELETE", "/ocs/v2.php/apps/spreed/api/v4/room/" + room["token"],
                                    expected=(200, 404), name="files-cleanup-room")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--state", required=True)
    parser.add_argument("--output", required=True)
    args = parser.parse_args()
    sys.exit(asyncio.run(run(args.state, args.output)))
