#!/usr/bin/env python3
"""Native diagnostic transport and two-native-peer intermediate test.

All generated sources are synthetic. This does not substitute for Talk browser
interoperability or native device qualification. Wire material is never logged.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import selectors
import signal
import struct
import subprocess
import time
from packets import Capture, process_observation, running

HEADER = struct.Struct(">4sHH64sQQ")
COUNTERS = ("video_source_frames", "video_decoded_frames", "audio_source_blocks_10ms",
            "audio_nonzero_blocks_10ms", "successful_socket_sends", "gate_denied_sends",
            "last_socket_send_us", "rejected_decode_frames", "helper_elapsed_us", "revoked_at_us",
            "data_channels_created", "data_channels_opened", "data_channels_closed", "data_messages_received",
            "data_bytes_received", "data_rejections", "negotiations_completed", "signaling_state")


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


class Peer:
    def __init__(self, driver, helper, intent=3):
        self.scope = os.urandom(64)
        self.operation = self.serial = 0
        self.started = time.monotonic()
        self.child = subprocess.Popen([str(driver), "--diagnostic-synthetic", str(helper)],
                                      stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        os.set_blocking(self.child.stdout.fileno(), False)
        self.buffer = bytearray()
        self.ready_ms = None
        self.connected = False
        self.stats = {}
        self.output_serial = 0
        self.sdp_operation = None
        self.helper_pid = None
        self.helper_directory = None
        self.rss_peak_kib = 0
        self.drained = False
        self.ports = set()
        self.revoke_wall = None
        self.failure_category = None
        self.failure_frontier = None
        self.send(1, bytes([intent]))

    def send(self, kind, payload=b"", operation=None):
        if len(payload) > 8192 - HEADER.size:
            raise ValueError("diagnostic record too large")
        if operation is None:
            self.operation += 1
            operation = self.operation
        self.serial += 1
        frame = HEADER.pack(b"NMTM", 2, kind, self.scope, operation, self.serial) + payload
        self.child.stdin.write(struct.pack(">I", len(frame)) + frame)
        self.child.stdin.flush()

    def forward(self, kind, payload):
        if kind == 101:
            offset = struct.unpack_from(">I", payload, 4)[0]
            if offset == 0:
                self.operation += 1
                self.sdp_operation = self.operation
            self.send(3, payload, self.sdp_operation)
        elif kind == 102:
            self.send(4, payload)

    def description(self, kind, text):
        raw = text.encode()
        if len(raw) > 65536 or not raw.startswith(b"v=0\r\n"):
            raise ValueError("diagnostic SDP bounds")
        self.operation += 1
        for offset in range(0, len(raw), 8095):
            self.send(3, struct.pack(">IIB", len(raw), offset, kind) + raw[offset:offset + 8095], self.operation)

    def candidate(self, candidate):
        mid = candidate["sdpMid"].encode()
        raw = candidate["candidate"].encode()
        if len(mid) > 64 or len(raw) > 2048:
            raise ValueError("diagnostic candidate bounds")
        self.send(4, bytes([len(mid), candidate["sdpMLineIndex"]]) + mid + raw)

    def read(self, cleanup=False):
        try:
            data = os.read(self.child.stdout.fileno(), 8192)
        except BlockingIOError:
            return []
        if not data:
            return []
        self.buffer.extend(data)
        if len(self.buffer) > 16384:
            raise RuntimeError("diagnostic event buffer overflow")
        result = []
        while len(self.buffer) >= 4:
            size = struct.unpack_from(">I", self.buffer)[0]
            if size < HEADER.size or size > 8192:
                raise RuntimeError("invalid helper event framing")
            if len(self.buffer) < size + 4:
                break
            magic, version, kind, scope, op, serial = HEADER.unpack_from(self.buffer, 4)
            if magic != b"NMTM" or version != 2 or scope != self.scope or serial <= self.output_serial:
                raise RuntimeError("invalid helper event identity")
            self.output_serial = serial
            payload = bytes(self.buffer[4 + HEADER.size:4 + size])
            del self.buffer[:4 + size]
            if kind == 100:
                self.ready_ms = (time.monotonic() - self.started) * 1000
            elif kind == 103:
                self.connected = True
            elif kind == 104:
                if len(payload) != 144:
                    raise RuntimeError("invalid native counters")
                self.stats = dict(zip(COUNTERS, struct.unpack(">18Q", payload)))
            elif kind == 105:
                self.drained = True
            elif kind == 106:
                self.failure_category = payload[0] if payload else 0
                self.failure_frontier = payload[1] if len(payload) > 1 else None
                if not cleanup:
                    raise RuntimeError("native failure")
            result.append((kind, op, payload))
        return result

    def sample(self):
        if self.helper_pid is None:
            try:
                children = Path(f"/proc/{self.child.pid}/task/{self.child.pid}/children").read_text().split()
                if len(children) == 1:
                    self.helper_pid = int(children[0])
                    self.helper_directory = Path(f"/proc/{self.helper_pid}/cwd").readlink()
            except (FileNotFoundError, ProcessLookupError):
                return
        rss = 0
        for pid in (self.child.pid, self.helper_pid):
            if pid is None:
                continue
            try:
                for line in Path(f"/proc/{pid}/status").read_text().splitlines():
                    if line.startswith("VmRSS:"):
                        rss += int(line.split()[1])
            except (FileNotFoundError, ProcessLookupError):
                pass
        self.rss_peak_kib = max(self.rss_peak_kib, rss)

    def stop(self, fault=None):
        self.sample()
        self.ports, devices = process_observation(self.helper_pid) if self.helper_pid else (set(),0)
        at = time.monotonic()
        self.revoke_wall = time.time()
        if fault == "owner-death" and self.child.poll() is None:
            self.child.kill()
        elif fault == "heartbeat-expiry" and self.child.poll() is None:
            self.child.send_signal(signal.SIGSTOP)
        if self.child.poll() is None:
            try:
                if fault is None:
                    self.send(5)
            except (BrokenPipeError, OSError):
                pass
        helper_gone_ms = None
        while time.monotonic() - at < 2:
            if self.helper_pid and not running(self.helper_pid):
                helper_gone_ms = (time.monotonic() - at) * 1000
                if fault == "heartbeat-expiry" and self.child.poll() is None:
                    self.child.send_signal(signal.SIGCONT)
            if self.child.poll() is not None and helper_gone_ms is not None:
                break
            if fault != "owner-death":
                self.read(cleanup=True)
            time.sleep(.005)
        if self.child.poll() is None:
            self.child.kill()
        self.child.wait(timeout=3)
        self.read(cleanup=True)
        outcome = self.child.stderr.read(4096).decode(errors="replace").strip()
        # SIGKILL of the owner cannot run its path cleanup. Only this harness's
        # observed child's private directory is removed after verified exit.
        if helper_gone_ms is not None and self.helper_directory and self.helper_directory.parent == Path("/tmp") and self.helper_directory.name.startswith("nmt-media-"):
            (self.helper_directory / "watchdog").unlink(missing_ok=True)
            if self.helper_directory.exists():
                self.helper_directory.rmdir()
        return {"driver_exit": self.child.returncode, "elapsed_ms": (time.monotonic() - at) * 1000,
                "native_failure_category": self.failure_category, "native_failure_frontier": self.failure_frontier,
                "fault": fault, "helper_gone_ms":helper_gone_ms,"sampled_open_video_audio_device_fds":devices,
                "observed_helper_udp_sockets":len(self.ports),
                "drained_synthetic": self.drained, "supervisor_outcome": outcome,
                "last_native_socket_send_minus_revoke_ms": (self.stats.get("last_socket_send_us",0)-self.stats.get("revoked_at_us",0))/1000 if self.stats.get("revoked_at_us") else None,
                "physical_device_release": "not applicable: synthetic source, no device opened"}


def pair(driver, helper, stall=False, fault=None, capture=None):
    peers = []
    select = selectors.DefaultSelector()
    started = time.monotonic()
    offered = False
    next_stats = started
    report = {"profile": "synthetic 640x360@20fps VP8 / 48kHz mono Opus; two native peers only"}
    try:
        for i in range(2):
            peer = Peer(driver, helper)
            peers.append(peer)
            select.register(peer.child.stdout, selectors.EVENT_READ, i)
        while time.monotonic() - started < 10:
            for key, _ in select.select(.02):
                i = key.data
                for kind, _, payload in peers[i].read():
                    if kind in (101, 102):
                        peers[1 - i].forward(kind, payload)
            if not offered and all(peer.ready_ms is not None for peer in peers):
                peers[0].send(2)
                offered = True
            for peer in peers:
                peer.sample()
                if peer.child.poll() is not None:
                    raise RuntimeError("native peer terminated before exchange")
            if time.monotonic() >= next_stats:
                for peer in peers:
                    if peer.ready_ms is not None:
                        peer.send(6)
                next_stats = time.monotonic() + .25
            if all(peer.stats.get("video_decoded_frames", 0) >= 20 and peer.stats.get("audio_nonzero_blocks_10ms", 0) >= 50 for peer in peers):
                break
        report["peers"] = [{"ready_ms": p.ready_ms, "connected": p.connected, "counters": p.stats,
                            "sampled_driver_plus_helper_peak_rss_kib": p.rss_peak_kib} for p in peers]
        report["bidirectional_video_audio"] = all(p.connected and p.stats.get("video_decoded_frames", 0) >= 20 and p.stats.get("audio_nonzero_blocks_10ms", 0) >= 50 for p in peers)
        if stall:
            peers[0].send(7)
            time.sleep(.05)
            report["fault"] = "signaling/main executor stalled for 2 seconds, then independent revoke"
    except Exception as error:
        report["failure_type"] = type(error).__name__
    finally:
        report["stop"] = []
        for i, p in enumerate(peers):
            try:
                report["stop"].append(p.stop(fault if i == 0 else None))
            except Exception as error:
                report["stop"].append({"cleanup_failure_type": type(error).__name__, "helper_gone_ms": None})
                if p.child.poll() is None:
                    p.child.kill()
                p.child.wait(timeout=3)
        report["peers"] = [{"ready_ms": p.ready_ms, "connected": p.connected, "counters": p.stats,
                            "native_failure_category": p.failure_category, "native_failure_frontier": p.failure_frontier,
                            "sampled_driver_plus_helper_peak_rss_kib": p.rss_peak_kib} for p in peers]
        select.close()
        if capture:
            try:
                report["packet_observation"]=capture.finish([(p.ports,p.revoke_wall) for p in peers])
            except Exception as error:
                report["packet_observer_failure_type"] = type(error).__name__
    report["shutdown_observed"] = all(s["helper_gone_ms"] is not None and s["helper_gone_ms"] <= (400 if s["fault"]=="heartbeat-expiry" else 250) for s in report["stop"])
    packet_result = all(p["observed_udp_packets"]>0 and p["last_packet_minus_revoke_ms"] <= (400 if fault=="heartbeat-expiry" else 250) for p in report.get("packet_observation",{}).get("peers",[]))
    report["result"] = "pass" if report.get("bidirectional_video_audio") and report["shutdown_observed"] and packet_result and not report.get("failure_type") and not report.get("packet_observer_failure_type") else "fail"
    report["limits"] = ["not Talk interoperability", "not accepted 720p30 workload", "not physical capture",
                        "RSS samples are diagnostics, not complete Q.PERF footprint/peak proof"]
    return report


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--driver", type=Path, required=True)
    p.add_argument("--helper", type=Path, required=True)
    p.add_argument("--output", type=Path, required=True)
    p.add_argument("--stall", action="store_true")
    p.add_argument("--fault", choices=["owner-death","heartbeat-expiry"])
    p.add_argument("--packets", action="store_true")
    args = p.parse_args()
    report = {"schema": 1, "driver_sha256": sha(args.driver), "helper_sha256": sha(args.helper),
              "harness_sha256": sha(__file__)}
    try:
        if args.packets:
            with Capture() as capture:
                report.update(pair(args.driver, args.helper, args.stall, args.fault, capture))
        else:
            report.update(pair(args.driver, args.helper, args.stall, args.fault))
    except Exception as error:
        # Only allowlisted local diagnostic failures, never exception text from a
        # server/browser or serialized signaling material.
        report.update(result="fail", failure_type=type(error).__name__)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({"result": report["result"], "output": str(args.output)}))
    raise SystemExit(0 if report["result"] == "pass" else 1)


if __name__ == "__main__":
    main()
