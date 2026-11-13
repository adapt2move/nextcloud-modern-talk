#!/usr/bin/env python3
"""Real coturn TLS media check inside an owned disposable Linux network namespace.

No firewall rules or host trust settings are changed. Logs and throwaway TLS keys
stay in a private temporary directory; only fixed test outcomes are reported.
"""
import argparse
import json
import os
from pathlib import Path
import secrets
import shutil
import signal
import time
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]
TEST = "media::engine::tests::media_flows_through_a_real_turn_server_over_tls"
BRIDGE_TESTS = "media::engine::turn_proxy::tests"


def run(argv, *, timeout=180, **kwargs):
    return subprocess.run(argv, cwd=ROOT, check=True, timeout=timeout, **kwargs)


def cargo(selection, *extra):
    return [sys.executable, str(ROOT / "tools/cargo.py"), "test", "--locked",
            "-p", "talk-adapters", "--lib", selection, *extra]


def inside(private):
    # A private veth pair supplies real non-loopback bind addresses and multicast
    # support for the engine's mDNS socket, without a route to the host/Internet.
    for command in (
        ["ip", "link", "set", "lo", "up"],
        ["ip", "link", "add", "nmt0", "type", "veth", "peer", "name", "nmt1"],
        ["ip", "address", "add", "192.0.2.1/24", "dev", "nmt0"],
        ["ip", "address", "add", "192.0.2.2/24", "dev", "nmt1"],
        ["ip", "link", "set", "nmt0", "up"],
        ["ip", "link", "set", "nmt1", "up"],
        ["ip", "route", "add", "default", "dev", "nmt0"],
    ):
        run(command, timeout=10, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    os.umask(0o077)
    if not private.is_dir():
        raise RuntimeError("missing owned private fixture")
    fixtures = ROOT / "crates/talk-adapters/src/media/fixtures"
    run(["openssl", "x509", "-inform", "DER", "-in", str(fixtures / "turn-test-cert.der"),
         "-out", str(private / "cert.pem")], timeout=10)
    run(["openssl", "pkey", "-inform", "DER", "-in", str(fixtures / "turn-test-key.der"),
         "-out", str(private / "key.pem")], timeout=10)
    password = secrets.token_hex(24)
    config = private / "turn.conf"
    config.write_text("\n".join([
        "listening-ip=127.0.0.1", "relay-ip=192.0.2.1", "tls-listening-port=5349",
        "no-cli", "no-udp", "no-tcp", "no-dtls", "no-tlsv1", "no-tlsv1_1",
        "lt-cred-mech", "realm=nmt.test", "user=nmt:" + password,
        "cert=" + str(private / "cert.pem"), "pkey=" + str(private / "key.pem"),
        "min-port=49160", "max-port=49200", "allow-loopback-peers",
        "log-file=stdout", "simple-log", "relay-threads=1",
        "pidfile=" + str(private / "turn.pid"),
    ]) + "\n")
    env = dict(os.environ, CARGO_NET_OFFLINE="true", NMT_TURN_USER="nmt",
               NMT_TURN_PASS=password, NMT_TURNS_URL="turns:127.0.0.1:5349?transport=tcp")
    with (private / "turn.log").open("w") as log:
        server = subprocess.Popen(["turnserver", "-c", str(config)], cwd=private,
                                  stdout=log, stderr=subprocess.STDOUT)
        try:
            # Establish readiness with the same certificate/name checks as
            # the fixture requires; startup delay alone is not evidence.
            ready = False
            for _ in range(40):
                if server.poll() is not None:
                    break
                probe = subprocess.run([
                    "openssl", "s_client", "-connect", "127.0.0.1:5349", "-brief",
                    "-CAfile", str(private / "cert.pem"), "-verify_hostname", "localhost",
                    "-verify_return_error"], input=b"", stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL, timeout=3)
                if probe.returncode == 0:
                    ready = True
                    break
                time.sleep(0.1)
            if not ready:
                raise RuntimeError("TLS fixture did not become ready")
            # Failure logs stay private and are removed with this namespace.
            with (private / "media.log").open("w") as media:
                run(cargo(TEST, "--", "--exact", "--ignored", "--nocapture"),
                    env=env, stdout=media, stderr=subprocess.STDOUT)
                run(cargo(BRIDGE_TESTS), env=env, stdout=media, stderr=subprocess.STDOUT)
        finally:
            server.terminate()
            try:
                server.wait(timeout=5)
            except subprocess.TimeoutExpired:
                server.kill()
                server.wait(timeout=5)
    return {"result": "pass", "real_tls_turn_media": True,
                      "talk_e2ee_media": True, "selected_relay_required": True,
                      "fresh_decode_window": "second half", "certificate_and_lifetime_tests": True,
                      "isolated_network": True, "physical_devices": False}


def in_namespace(private):
    env = dict(os.environ, NMT_TURN_PARENT_NETNS=str(os.stat("/proc/self/ns/net").st_ino),
               NMT_TURN_PRIVATE=str(private))
    command = ["unshare", "--net", "--", sys.executable, str(Path(__file__).resolve()), "--inside"]
    # Own the entire process group, including coturn and Cargo/test children.
    # Timeout/interruption must not leave an orphan holding the network namespace.
    with subprocess.Popen(command, cwd=ROOT, env=env, start_new_session=True) as child:
        try:
            code = child.wait(timeout=240)
        finally:
            for sig in (signal.SIGTERM, signal.SIGKILL):
                try:
                    os.killpg(child.pid, sig)
                except ProcessLookupError:
                    pass
                if sig == signal.SIGTERM:
                    try:
                        child.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        pass
            child.wait(timeout=5)
        if code:
            raise subprocess.CalledProcessError(code, command)
    return json.loads((private / "result.json").read_text())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    # Inner execution is always behind unshare; this is not a general host-server mode.
    parser.add_argument("--inside", action="store_true", help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.inside:
        parent = os.environ.get("NMT_TURN_PARENT_NETNS")
        private = os.environ.get("NMT_TURN_PRIVATE")
        if not parent or not private or parent == str(os.stat("/proc/self/ns/net").st_ino):
            parser.error("use the namespace-owning entrypoint")
        result = inside(Path(private))
        (Path(private) / "result.json").write_text(json.dumps(result))
        return
    if sys.platform != "linux" or os.geteuid() != 0:
        parser.error("run only as root on an owned disposable Linux VM")
    for name in ("turnserver", "openssl", "ip", "unshare"):
        if shutil.which(name) is None:
            parser.error("missing fixture prerequisite: " + name)
    # Resolve/download/build outside the namespace before removing network access.
    run(cargo(TEST, "--no-run"), timeout=900, stdout=subprocess.DEVNULL)
    with tempfile.TemporaryDirectory(prefix="nmt-turn-tls-") as temporary:
        result = in_namespace(Path(temporary))
    result["private_fixture_removed"] = True
    print(json.dumps(result))


if __name__ == "__main__":
    try:
        main()
    except (subprocess.SubprocessError, OSError, RuntimeError):
        if "--inside" not in sys.argv:
            print(json.dumps({"result": "fail", "reason": "TLS TURN fixture or media check failed"}))
        sys.exit(1)
