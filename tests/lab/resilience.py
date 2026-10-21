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
        self.injected = {"500": 0, "403": 0, "offline": 0}

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
            if len(line) == 3 and line[0] == b"GET" and b"/api/v1/chat/" in line[1] and mode in self.injected:
                self.injected[mode] += 1
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
    def smoke(self, key, pin):
        super().smoke(key, pin)
        port = 47407
        self.http_host = f"127.0.0.1:{port}"
        room = self.request("lab_alice", "POST", "/ocs/v2.php/apps/spreed/api/v4/room",
                            body={"roomType": 2, "roomName": "Disposable recovery fixture"},
                            expected=(201,), name="recovery.create-room")
        self.request("lab_alice", "POST", "/ocs/v2.php/apps/spreed/api/v4/room/" + room["token"] + "/participants",
                     body={"newParticipant": "lab_bob", "source": "users"}, name="recovery.add-member")
        directory = self.runtime / f"{key}-recovery"
        directory.mkdir(mode=0o700)
        save(directory / "alice.json", {"origin": f"http://127.0.0.1:{port}",
             "user": "lab_alice", "password": self.passwords["lab_alice"], "room": room})
        target = self.output / key
        target.mkdir(mode=0o700)
        result, injected = asyncio.run(self.client(directory, port, target))
        self.require("recovery.typed-client", result == 0, {"exit_code": result})
        for mode, count in injected.items():
            self.require("recovery.injected." + mode, count > 0, {"requests": count})
        self.request("lab_alice", "DELETE", "/ocs/v2.php/apps/spreed/api/v4/room/" + room["token"], expected=(200, 404), name="recovery.remove-room")
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
                    try:
                        code = await asyncio.wait_for(process.wait(), 360)
                    except BaseException:
                        process.kill()
                        await process.wait()
                        raise
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
