#!/usr/bin/env python3
"""Opt-in released-server messaging and recovery checks with isolated fault injection.

Runs only in the disposable Linux lab. Credentials stay in its private /run
directory; evidence contains enum outcomes and hashes, never request bytes.
"""
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


class FaultRelay(Relay):
    def __init__(self, host, port, control):
        super().__init__(host, port)
        self.control = control
        self.injected = {mode: 0 for mode in ("500", "403", "offline", "latency-1500", "latency-4000", "slow-poll", "intermittent", "disconnected")}
        self.requests = 0
        self.chat_reads = 0

    async def disconnect(self):
        # An owned lab network transition also drops already admitted sockets.
        active=list(self.tasks)
        for task in active: task.cancel()
        await asyncio.gather(*active,return_exceptions=True)

    async def connection(self, reader, writer):
        if len(self.tasks) >= 8:
            writer.close()
            return
        task = asyncio.current_task()
        self.tasks.add(task)
        remote = None
        pipes = []
        try:
            # Read only the bounded request headers. Never log or retain them.
            header = await asyncio.wait_for(reader.readuntil(b"\r\n\r\n"), 5)
            line = header.split(b"\r\n", 1)[0].split(b" ")
            mode = self.control.read_text() if self.control.exists() else "healthy"
            self.requests += 1
            if len(line) == 3 and line[0] == b"GET" and b"/api/v1/chat/" in line[1]:
                self.chat_reads += 1
            if mode in ("latency-1500", "latency-4000", "slow-poll", "intermittent"):
                self.injected[mode] += 1
                if mode.startswith("latency-"):
                    await asyncio.sleep(int(mode.split("-")[1]) / 1000)
                elif mode == "slow-poll" and (b"lookIntoFuture=1" in header or b"/signaling/" in header):
                    await asyncio.sleep(4)
                elif mode == "intermittent" and self.chat_reads > 0 and self.chat_reads % 3 == 0 and b"/api/v1/chat/" in line[1]:
                    # Deterministic, content-free fault cadence. Only reads are
                    # faulted: this is not a safe mutation-replay simulator.
                    mode = ("502", "504", "offline")[(self.chat_reads // 3) % 3]
            if mode == "disconnected":
                self.injected[mode] += 1
                return
            if len(line) == 3 and line[0] == b"GET" and b"/api/v1/chat/" in line[1] and mode in ("500", "403", "offline", "502", "504"):
                self.injected[mode] = self.injected.get(mode, 0) + 1
                if mode != "offline":
                    status = int(mode)
                    body = b"<html>Temporary gateway failure</html>"
                    writer.write(f"HTTP/1.1 {status} Temporary Failure\r\nContent-Type: text/html\r\nContent-Length: {len(body)}\r\nConnection: close\r\n\r\n".encode() + body)
                    await asyncio.wait_for(writer.drain(), 5)
                return
            upstream, remote = await asyncio.wait_for(asyncio.open_connection(self.host, 80, limit=65536), 3)
            # One request per upstream connection makes every request observable,
            # including when reqwest would otherwise reuse a healthy connection.
            fields = header[:-4].split(b"\r\n")
            fields = [field for field in fields if not field.lower().startswith(b"connection:")]
            remote.write(b"\r\n".join(fields) + b"\r\nConnection: close\r\n\r\n")
            await asyncio.wait_for(remote.drain(), 5)
            pipes = [asyncio.create_task(self.pipe(reader, remote)), asyncio.create_task(self.pipe(upstream, writer))]
            done, _ = await asyncio.wait(pipes, timeout=180, return_when=asyncio.FIRST_COMPLETED)
            for result in done:
                result.result()
        except (OSError, TimeoutError, ConnectionError, asyncio.IncompleteReadError, asyncio.LimitOverrunError):
            pass
        finally:
            for item in pipes:
                item.cancel()
            await asyncio.gather(*pipes, return_exceptions=True)
            for stream in (writer, remote):
                if stream:
                    stream.close()
                    await stream.wait_closed()
            self.tasks.discard(task)


class RecoveryLab(Lab):
    def execute_reminders(self, directory):
        # Run only Talk's registered reminder job in this disposable server.
        # The table contains no fixture credentials or message content.
        job_class = r"OCA\Talk\BackgroundJob\Reminder"
        rows = self.occ("background-job:list", "--class", job_class).splitlines()
        jobs = [parts[1].strip() for row in rows
                if len(parts := row.split("|")) >= 4
                and parts[1].strip().isdigit() and parts[2].strip() == job_class]
        self.require("recovery.reminder-job", len(jobs) == 1)
        self.occ("background-job:execute", jobs[0], "--force-execute")
        headers = {"X-Nextcloud-User-Status": None}
        notices = self.request("lab_alice", "GET", "/ocs/v2.php/apps/notifications/api/v2/notifications",
                               name="recovery.reminder-notifications", response_headers=headers)
        reminders = [item for item in notices if item.get("app") == "spreed"
                     and item.get("object_type") == "reminder"]
        message = int((directory / "reminder.request").read_text())
        token = self.recovery_token
        context = self.request("lab_alice", "GET",
                               f"/ocs/v2.php/apps/spreed/api/v1/chat/{token}/{message}/context",
                               query={"limit": 25}, name="recovery.reminder-context")
        targets = [item for item in context if item.get("id") == message]
        capabilities = self.request("lab_alice", "GET", "/ocs/v2.php/cloud/capabilities",
                                    name="recovery.reminder-capabilities")
        endpoints = capabilities.get("capabilities", {}).get("notifications", {}).get("ocs-endpoints", [])
        status = headers["X-Nextcloud-User-Status"]
        self.current["reminder_probe"] = {
            "count": len(reminders),
            "object_has_message": any(len(item.get("object_id", "").split("/")) >= 2 for item in reminders),
            "object_matches_target": any(item.get("object_id", "").split("/")[:2] == [token, str(message)] for item in reminders),
            "should_notify": [item.get("shouldNotify") for item in reminders],
            "status_allows_notification": status in ("online", "away", "busy", "offline", "invisible"),
            "status": status if status in ("online", "away", "busy", "offline", "invisible", "dnd") else "missing" if status is None else "other",
            "user_status_capability": "user-status" in endpoints,
            "target_count": len(targets),
            "target_token_matches": all(item.get("token") == token for item in targets),
            "target_kind": [item.get("messageType") for item in targets],
            "target_system_empty": [item.get("systemMessage") == "" for item in targets],
            "target_expiration_set": [bool(item.get("expirationTimestamp")) for item in targets],
        }
        self.require("recovery.reminder-event-created", bool(reminders))

    async def trigger_reminders(self, directory):
        async def wait_for_request():
            while not (directory / "reminder.request").exists():
                await asyncio.sleep(0.1)
        await asyncio.wait_for(wait_for_request(), 180)
        await asyncio.to_thread(self.execute_reminders, directory)
        (directory / "reminder.done").touch(mode=0o600)

    def smoke(self, key, pin):
        super().smoke(key, pin)
        port = 47407
        self.http_host = f"127.0.0.1:{port}"
        room = self.request("lab_alice", "POST", "/ocs/v2.php/apps/spreed/api/v4/room",
                            body={"roomType": 2, "roomName": "Disposable recovery fixture"},
                            expected=(201,), name="recovery.create-room")
        self.recovery_token = room["token"]
        self.sensitive.append(self.recovery_token)
        self.request("lab_alice", "POST", "/ocs/v2.php/apps/spreed/api/v4/room/" + room["token"] + "/participants",
                     body={"newParticipant": "lab_bob", "source": "users"}, name="recovery.add-member")
        source = self.request("lab_bob", "POST", "/ocs/v2.php/apps/spreed/api/v1/chat/" + room["token"],
                              body={"message": "Synthetic private reply source"}, expected=(201,), name="recovery.private-source")
        direct = self.request("lab_alice", "POST", "/ocs/v2.php/apps/spreed/api/v4/room",
                              body={"roomType": 1, "invite": "lab_bob"}, expected=(200, 201), name="recovery.private-direct")
        private = self.request("lab_alice", "POST", "/ocs/v2.php/apps/spreed/api/v1/chat/" + direct["token"],
                               body={"message": "Synthetic private reply", "replyTo": source["id"],
                                     "replyToToken": room["token"], "referenceId": "synthetic-private-reply"},
                               expected=(201,), name="recovery.private-reply")
        parent = private.get("parent") or {}
        metadata = parent.get("metaData") or {}
        self.current["private_reply_shape"] = {
            "keys": sorted(private.keys()), "parent_keys": sorted(parent.keys()),
            "parent_token_matches_destination": parent.get("token") == direct["token"],
            "parent_token_matches_source": parent.get("token") == room["token"],
            "parent_has_nested_parent": "parent" in parent,
            "parent_id_positive": isinstance(parent.get("id"), int) and parent["id"] > 0,
            "metadata_keys": sorted(metadata.keys()),
            "metadata_source_matches": metadata.get("replyToConversationToken") == room["token"],
            "metadata_id_matches_parent": str(metadata.get("replyToMessageId")) == str(parent.get("id")),
        }
        listed = self.request("lab_bob", "POST", "/ocs/v2.php/apps/spreed/api/v4/room",
                              body={"roomType": 2, "roomName": "Disposable discovery fixture"},
                              expected=(201,), name="recovery.create-listed-room")
        self.sensitive.append(listed["token"])
        listed_path = "/ocs/v2.php/apps/spreed/api/v4/room/" + listed["token"]
        self.request("lab_bob", "PUT", listed_path + "/listable", body={"scope": 1}, name="recovery.open-room")
        self.request("lab_bob", "PUT", listed_path + "/description", body={"description": "Synthetic open conversation"}, name="recovery.describe-room")
        directory = self.runtime / f"{key}-recovery"
        directory.mkdir(mode=0o700)
        save(directory / "alice.json", {"origin": f"http://127.0.0.1:{port}",
             "user": "lab_alice", "password": self.passwords["lab_alice"], "room": room})
        save(directory / "bob.json", {"origin": f"http://127.0.0.1:{port}",
             "user": "lab_bob", "password": self.passwords["lab_bob"], "room": room})
        target = self.output / key
        target.mkdir(mode=0o700)
        result, injected = asyncio.run(self.client(directory, port, target))
        self.require("recovery.typed-client", result == 0, {"exit_code": result})
        for mode, count in injected.items():
            if mode not in ("500", "403", "offline"):
                continue
            self.require("recovery.injected." + mode, count > 0, {"requests": count})
        self.request("lab_alice", "DELETE", "/ocs/v2.php/apps/spreed/api/v4/room/" + room["token"], expected=(200, 404), name="recovery.remove-room")
        self.request("lab_bob", "DELETE", listed_path, expected=(200, 404), name="recovery.remove-listed-room")
        self.current.update(result="pass", finished=now())

    async def client(self, directory, port, output):
        env = dict(os.environ, NMT_FIXTURE_DIR=str(directory), RUST_BACKTRACE="0")
        commands = [
            ["cargo", "test", "--locked", "-p", "talk-adapters", "--features", "backend-test",
             "--test", "resilience_backend", "--", "--ignored", "--nocapture"],
            ["cargo", "test", "--locked", "-p", "talk-adapters", "--features", "backend-test",
             "--test", "alpha_backend", "--", "--ignored", "--nocapture"],
        ]
        results = []
        async with FaultRelay(urlsplit(self.base).hostname, port, directory / "fault.mode") as relay:
            for index, command in enumerate(commands):
                start = time.monotonic()
                with (output / f"backend-{index}.log").open("w") as log:
                    process = await asyncio.create_subprocess_exec(*command, cwd=ROOT, env=env,
                        stdout=log, stderr=asyncio.subprocess.STDOUT)
                    reminder_job = asyncio.create_task(self.trigger_reminders(directory)) if index == 1 else None
                    try:
                        code = await asyncio.wait_for(process.wait(), 360)
                        if reminder_job is not None and code == 0:
                            await reminder_job
                    except BaseException:
                        if process.returncode is None:
                            process.kill()
                            await process.wait()
                        raise
                    finally:
                        if reminder_job is not None:
                            if not reminder_job.done():
                                reminder_job.cancel()
                            await asyncio.gather(reminder_job, return_exceptions=True)
                if (directory / "latency.json").exists():
                    save(output / "latency.json", __import__("json").loads((directory / "latency.json").read_text()))
                results.append({"command": command, "exit_code": code,
                                "seconds": round(time.monotonic() - start, 2)})
                save(output / "client-results.json", results)
                if code:
                    return code, relay.injected
            return 0, relay.injected


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--profile", choices=("all", "r34", "r35"), default="all")
    parser.add_argument("--output", required=True)
    parser.add_argument("--budget-seconds", type=int, default=2400)
    args = parser.parse_args()
    os.umask(0o077)
    lab = RecoveryLab(args)
    lab.report["scope"] = "Released-server API and typed messaging recovery after gateway 500/403 and connection loss; no native UI/media claim"
    lab.report["client_source_sha256"] = {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest()
        for base in ("crates", "apps") for p in sorted((ROOT / base).rglob("*.rs"))}
    print("recovery state: " + str(lab.runtime / "state.json"), flush=True)

    def interrupted(signum, frame):
        raise LabInterrupted("Recovery lab interrupted")

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
