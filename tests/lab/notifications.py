#!/usr/bin/env python3
"""Real-server notification diagnostics and the production read-only account owner.

Use an owned development lab. No push-device registration, credentials or content
are written to evidence. This does not qualify native OS notification delivery.
"""
import argparse
import asyncio
import json
import os
from pathlib import Path
import shutil
import tempfile
import time
import urllib.parse

from alpha_protocol import Relay
from dev import owned_state, restore
from lab import API, save

NOTIFICATIONS = "/ocs/v2.php/apps/notifications/api/v2/notifications"


async def exercise(state, output):
    target = Path(output)
    target.mkdir(parents=True, exist_ok=False)
    with owned_state(state) as (_, record):
        lab = restore(record)
        lab.deadline = time.monotonic() + 1200
        lab.occ("app:enable", "notifications")
        version = lab.occ("config:app:get", "notifications", "installed_version")
        for user in ("lab_alice", "lab_bob"):
            lab.request(user, "GET", "/ocs/v2.php/cloud/user", name="notifications-authenticate")
        room = lab.request("lab_alice", "POST", API + "v4/room",
                           body={"roomType": 1, "invite": "lab_bob", "source": "users"},
                           expected=(200, 201), name="notifications-direct-room")
        lab.sensitive.append(room["token"])
        # One-to-one membership can be created lazily. Admit the recipient using
        # the same supported create route before reading either projection.
        reciprocal = lab.request("lab_bob", "POST", API + "v4/room",
                                 body={"roomType": 1, "invite": "lab_alice"},
                                 expected=(200, 201), name="notifications-reciprocal")
        lab.require("notifications-shared-direct-room", reciprocal["token"] == room["token"])
        directory = Path(tempfile.mkdtemp(prefix="nmt-notifications-", dir="/run"))
        directory.chmod(0o700)
        env = os.environ.copy()
        env["RUST_TEST_THREADS"] = "1"
        env.setdefault("CARGO_BUILD_JOBS", "2")
        for user, key in (("lab_alice", "ALICE"), ("lab_bob", "BOB")):
            # Fetch each participant's projection, never reuse the creator's role.
            try:
                value = lab.request(user, "GET", API + "v4/room/" + room["token"], name="notifications-intake")
            except Exception:
                print(json.dumps(lab.current["operations"][-1]), flush=True)
                raise
            fixture = directory / (key + ".json")
            save(fixture, {"origin": record["development"]["client_url"],
                           "user": user, "password": lab.passwords[user], "room": value})
            env["NMT_BACKGROUND_" + key + "_FIXTURE"] = str(fixture)
        env["NMT_NOTIFICATION_CONTROL"] = str(directory)
        command = ["python3", "tools/cargo.py", "test", "--locked", "-p", "talk-adapters",
                   "--features", "backend-test", "--test", "background_backend", "--", "--ignored", "--nocapture"]
        start = time.monotonic()
        try:
            host = urllib.parse.urlsplit(record["development"]["vm_url"]).hostname
            port = urllib.parse.urlsplit(record["development"]["client_url"]).port
            async with Relay(host, port):
                with (target / "backend.log").open("w") as log:
                    process = await asyncio.create_subprocess_exec(*command, env=env, stdout=log, stderr=asyncio.subprocess.STDOUT)
                    async def trigger():
                        for _ in range(6000):
                            if (directory / "ready").exists():
                                break
                            if process.returncode is not None:
                                return
                            await asyncio.sleep(0.1)
                        else:
                            raise RuntimeError("Notification owner readiness deadline")
                        before = await asyncio.to_thread(lab.request, "lab_alice", "GET", NOTIFICATIONS, name="notifications-before")
                        await asyncio.to_thread(lab.occ, "notification:test-push", "--talk", "lab_alice")
                        after = await asyncio.to_thread(lab.request, "lab_alice", "GET", NOTIFICATIONS, name="notifications-after")
                        ids = {n["notification_id"] for n in before}
                        tests = [n for n in after if n["notification_id"] not in ids
                                 and n.get("app") == "admin_notification_talk"
                                 and n.get("object_type") == "admin_notifications"]
                        lab.require("notifications-test-push-persisted", len(tests) == 1)
                        save(directory / "generated", {"count": len(tests)})
                    async def reminder_trigger():
                        for _ in range(6000):
                            if (directory / "reminder-ready").exists():
                                break
                            if process.returncode is not None:
                                return
                            await asyncio.sleep(0.1)
                        else:
                            raise RuntimeError("Reminder owner readiness deadline")
                        job_class = r"OCA\Talk\BackgroundJob\Reminder"
                        rows = await asyncio.to_thread(lab.occ, "background-job:list", "--class", job_class)
                        jobs = [parts[1].strip() for row in rows.splitlines()
                                if len(parts := row.split("|")) >= 4 and parts[1].strip().isdigit()
                                and parts[2].strip() == job_class]
                        lab.require("notifications-reminder-job", len(jobs) == 1)
                        await asyncio.to_thread(lab.occ, "background-job:execute", jobs[0], "--force-execute")
                        save(directory / "reminder-generated", {"executed": True})
                    trigger_task = asyncio.create_task(trigger())
                    reminder_task = asyncio.create_task(reminder_trigger())
                    try:
                        await asyncio.wait_for(asyncio.gather(process.wait(), trigger_task, reminder_task), 900)
                    except BaseException:
                        if process.returncode is None:
                            process.kill()
                            await process.wait()
                        trigger_task.cancel()
                        reminder_task.cancel()
                        await asyncio.gather(trigger_task, reminder_task, return_exceptions=True)
                        raise
            report = {"profile": record["development"]["profile"], "notifications_version": version,
                      "test_push_persists_api_object": (directory / "generated").exists(), "exit_code": process.returncode,
                      "seconds": round(time.monotonic() - start, 2),
                      "scope": "released-server API and production account adapter; no native OS claim"}
            (target / "result.json").write_text(json.dumps(report, indent=2) + "\n")
            print(json.dumps(report), flush=True)
            return process.returncode
        finally:
            shutil.rmtree(directory)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--state", required=True)
    parser.add_argument("--output", required=True)
    args = parser.parse_args()
    raise SystemExit(asyncio.run(exercise(args.state, args.output)))
