#!/usr/bin/env python3
"""Exercise typing via the real HPB on the isolated released-server lab."""
import argparse
import asyncio
import hashlib
import json
import os
import secrets
import signal
import sys
import time
import urllib.error
import urllib.request
from urllib.parse import urlsplit

sys.dont_write_bytecode = True
from alpha_protocol import Relay
from lab import LABEL, LabInterrupted, cleanup, now, save
from presence import PresenceLab, ROOT

# Official 2.1.1 image, pinned to the multi-platform index digest. No MCU is
# needed for ordinary chat signaling; this does not test calls or media.
HPB_IMAGE = "docker.io/strukturag/nextcloud-spreed-signaling@sha256:b306eae9d1e62c4b2eaf10a6683dfe1fe76b833bd0f68d1606861e33aff5bca1"

# Each proxy is bound only to the container's loopback. It makes the same
# loopback-only test origins reachable from three separate network namespaces.
# The lab labels and owns both containers and publishes no host ports.
PROXY = r'''
pcntl_async_signals(true); pcntl_signal(SIGCHLD, SIG_IGN);
$target = getenv("TARGET"); $port = (int)getenv("PORT");
$server = stream_socket_server("tcp://127.0.0.1:" . $port, $errno, $error);
if (!$server) { exit(1); }
while ($client = stream_socket_accept($server, -1)) {
    $pid = pcntl_fork();
    if ($pid !== 0) { fclose($client); continue; }
    fclose($server);
    $remote = @stream_socket_client("tcp://" . $target . ":80", $errno, $error, 3);
    if (!$remote) { fclose($client); exit(1); }
    stream_set_blocking($client, false); stream_set_blocking($remote, false);
    while (true) {
        $read = [$client, $remote]; $write = null; $except = null;
        if (@stream_select($read, $write, $except, 90) < 1) { break; }
        foreach ($read as $src) {
            $data = fread($src, 65536);
            if ($data === false || $data === "") { break 2; }
            $dst = $src === $client ? $remote : $client;
            while ($data !== "") {
                $n = fwrite($dst, $data);
                if ($n === false || $n === 0) { break 3; }
                $data = substr($data, $n);
            }
        }
    }
    fclose($remote); fclose($client); exit(0);
}
'''


class HpbTypingLab(PresenceLab):
    async def client(self, directory, port, output):
        key = output.name
        network = self.run_id + "-" + key + "-net"
        hpb = self.run_id + "-" + key + "-hpb"
        proxy = self.run_id + "-" + key + "-proxy"
        hpb_port = 47409
        shared = secrets.token_hex(32)
        hash_key = secrets.token_hex(32)
        block_key = secrets.token_hex(16)
        self.sensitive.extend((shared, hash_key, block_key))
        self.remember("containers", hpb)
        self.docker("run", "--detach", "--name", hpb, "--label", LABEL + "=" + self.run_id,
                    "--network", network, "--memory", "512m", "--pids-limit", "128",
                    "--security-opt", "no-new-privileges:true", "--log-opt", "max-size=1m",
                    "--env", "HTTP_LISTEN=0.0.0.0:80", "--env", "BACKENDS_ALLOWALL=true",
                    "--env", "BACKENDS_ALLOWALL_SECRET=" + shared, "--env", "HASH_KEY=" + hash_key,
                    "--env", "BLOCK_KEY=" + block_key, HPB_IMAGE, timeout=90)
        ports = self.docker("inspect", hpb, "--format", "{{json .NetworkSettings.Ports}}")
        self.require("hpb.no-published-ports", not any((json.loads(ports) or {}).values()))
        networks = json.loads(self.docker("inspect", hpb, "--format", "{{json .NetworkSettings.Networks}}"))
        hpb_host = networks[network]["IPAddress"]
        nc_host = urlsplit(self.base).hostname
        end = min(self.deadline, time.monotonic() + 40)
        while time.monotonic() < end:
            try:
                with urllib.request.build_opener(urllib.request.ProxyHandler({})).open(
                    "http://" + hpb_host + "/api/v1/welcome", timeout=2
                ) as response:
                    if response.status == 200:
                        break
            except (OSError, urllib.error.URLError):
                await asyncio.sleep(0.5)
        else:
            raise RuntimeError("HPB readiness timed out")
        self.occ("talk:signaling:add", f"http://127.0.0.1:{hpb_port}", shared)
        # The Apache workers have already cached the initial app configuration.
        # Restart the disposable container so they observe the CLI change.
        self.docker("restart", self.container, timeout=60)
        nc_networks = json.loads(self.docker("inspect", self.container, "--format", "{{json .NetworkSettings.Networks}}"))
        nc_host = nc_networks[network]["IPAddress"]
        self.base = "http://" + nc_host
        end = min(self.deadline, time.monotonic() + 60)
        while time.monotonic() < end:
            try:
                request = urllib.request.Request(self.base + "/status.php", headers={"Host": "127.0.0.1"})
                with self.clients[None].open(request, timeout=2) as response:
                    if response.status == 200:
                        break
            except OSError:
                await asyncio.sleep(0.5)
        else:
            raise RuntimeError("NextCloud restart timed out")
        helper_image = self.pins["profiles"][key]["image"]
        # HPB -> NextCloud, for its authentication and room callbacks.
        self.remember("containers", proxy)
        self.state["containers"].remove(proxy)
        self.state["containers"].insert(0, proxy)
        save(self.runtime / "state.json", self.state)
        self.docker("run", "--detach", "--name", proxy, "--label", LABEL + "=" + self.run_id,
                    "--network", "container:" + hpb, "--memory", "128m", "--pids-limit", "64",
                    "--security-opt", "no-new-privileges:true", "--entrypoint", "php",
                    "--env", "TARGET=" + nc_host, "--env", "PORT=" + str(port),
                    helper_image, "-r", PROXY, timeout=60)
        # NextCloud -> HPB, for backend notifications. This process retires
        # with the lab-owned NextCloud container.
        self.docker("exec", "--detach", "--env", "TARGET=" + hpb_host,
                    "--env", "PORT=" + str(hpb_port), self.container, "php", "-r", PROXY)
        settings = self.request("lab_alice", "GET", "/ocs/v2.php/apps/spreed/api/v3/signaling/settings",
                                query={"token": self.request("lab_alice", "GET", "/ocs/v2.php/apps/spreed/api/v4/room",
                                                            name="hpb.rooms")[0]["token"]}, name="hpb.settings")
        self.require("hpb.external-mode", settings.get("signalingMode") == "external",
                     {"mode": settings.get("signalingMode")})
        self.current["hpb"] = {"image": HPB_IMAGE, "media": "not configured", "published_ports": []}
        command = ["cargo", "test", "--locked", "-p", "talk-adapters", "--features", "backend-test",
                   "--test", "presence_backend", "--", "--ignored", "--nocapture", "--test-threads=1"]
        env = dict(os.environ, NMT_FIXTURE_DIR=str(directory), RUST_BACKTRACE="0")
        start = time.monotonic()
        async with Relay(nc_host, port), Relay(hpb_host, hpb_port):
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
    parser.add_argument("--profile", choices=("r34", "r35"), default="r35")
    parser.add_argument("--output", required=True)
    parser.add_argument("--budget-seconds", type=int, default=2400)
    args = parser.parse_args()
    os.umask(0o077)
    lab = HpbTypingLab(args)
    lab.report["scope"] = "HPB chat typing on released Talk; no media or native UI claim"
    lab.report["client_source_sha256"] = {
        str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest()
        for base in ("crates", "apps") for p in sorted((ROOT / base).rglob("*.rs"))
    }
    print("recovery state: " + str(lab.runtime / "state.json"), flush=True)

    def interrupted(signum, frame):
        raise LabInterrupted("HPB typing lab interrupted")

    signal.signal(signal.SIGINT, interrupted)
    signal.signal(signal.SIGTERM, interrupted)
    try:
        lab.run([args.profile])
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
