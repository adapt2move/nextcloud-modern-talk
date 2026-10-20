#!/usr/bin/env python3
"""Exercise native absolute expiry with uninterrupted independent renewals.

Direct helper invocation deliberately bypasses the Rust deadline to prove the
native watchdog's own bound. No devices, SDP or network peers are involved.
"""
import argparse
import json
import os
from pathlib import Path
import socket
import struct
import subprocess
import tempfile
import time
from interop import HEADER, COUNTERS, sha


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--helper', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    report = {'schema': 1, 'result': 'fail', 'profile': 'synthetic-only hard 30s; no capture intent',
              'helper_sha256': sha(args.helper), 'harness_sha256': sha(__file__),
              'authority_ttl_ms': 100, 'renew_interval_ms': 25, 'supervisor_used': False}
    child = None
    started = time.monotonic()
    try:
        with tempfile.TemporaryDirectory(prefix='nmt-expiry-') as name:
            scope = os.urandom(64)
            server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            server.bind(name + '/watchdog')
            server.listen(1)
            server.settimeout(3)
            child = subprocess.Popen([str(args.helper), '--diagnostic-synthetic', name + '/watchdog'],
                                     stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            serial = 0
            def send(kind, payload=b''):
                nonlocal serial
                serial += 1
                frame = HEADER.pack(b'NMTM', 2, kind, scope, serial, serial) + payload
                child.stdin.write(struct.pack('>I', len(frame)) + frame)
                child.stdin.flush()
            send(1, b'\x00')
            watchdog, _ = server.accept()
            watchdog.settimeout(.02)
            os.set_blocking(child.stdout.fileno(), False)
            buffer = bytearray()
            previous_serial = 0
            heartbeat = 0
            counters = {}
            ready = drained = False
            next_pulse = next_stats = time.monotonic()
            while True:
                now = time.monotonic()
                if now - started > 31:
                    raise RuntimeError('hard deadline exceeded')
                if child.poll() is None and now >= next_pulse:
                    heartbeat += 1
                    try:
                        watchdog.sendall(b'NMW1' + scope + struct.pack('>IQ', 100, heartbeat))
                    except (BrokenPipeError, ConnectionResetError):
                        pass
                    next_pulse = now + .025
                if child.poll() is None and ready and now >= next_stats:
                    try:
                        send(6)
                    except BrokenPipeError:
                        pass
                    next_stats = now + .2
                try:
                    buffer.extend(os.read(child.stdout.fileno(), 8192))
                except BlockingIOError:
                    pass
                if len(buffer) > 16384:
                    raise RuntimeError('bounded observer overflow')
                while len(buffer) >= 4:
                    length = struct.unpack_from('>I', buffer)[0]
                    if not HEADER.size <= length <= 8192:
                        raise RuntimeError('invalid event length')
                    if len(buffer) < length + 4:
                        break
                    magic, version, kind, actual_scope, operation, event_serial = HEADER.unpack_from(buffer, 4)
                    if (magic, version, actual_scope) != (b'NMTM', 2, scope) or event_serial <= previous_serial:
                        raise RuntimeError('invalid event identity')
                    previous_serial = event_serial
                    payload = bytes(buffer[4+HEADER.size:4+length])
                    del buffer[:4+length]
                    if kind == 100:
                        ready = True
                    elif kind == 104:
                        if len(payload) != 144:
                            raise RuntimeError('invalid counters')
                        counters = dict(zip(COUNTERS, struct.unpack('>18Q', payload)))
                    elif kind == 105:
                        drained = True
                    elif kind == 106:
                        raise RuntimeError('native failure')
                if child.poll() is not None:
                    break
                time.sleep(.002)
            child.wait(timeout=1)
            # Renewals and new commands after termination cannot revive this child.
            late_write_rejected = False
            try:
                send(6)
            except BrokenPipeError:
                late_write_rejected = True
            time.sleep(.12)
            elapsed = time.monotonic() - started
            report.update(elapsed_ms=elapsed*1000, renewals_sent=heartbeat, counters=counters,
                          ready=ready, drained=drained, exit=child.returncode,
                          late_write_rejected=late_write_rejected, revived=child.poll() is None)
            revoke = counters.get('revoked_at_us', 0)
            if not (ready and drained and child.returncode == 0 and late_write_rejected
                    and 30_000_000 <= revoke < 30_050_000 and 30 <= elapsed < 30.5
                    and counters.get('video_source_frames') == 0
                    and counters.get('audio_source_blocks_10ms') == 0):
                raise RuntimeError('native absolute expiry assertion')
            watchdog.close()
            server.close()
            report['result'] = 'pass'
    except (OSError, RuntimeError, subprocess.TimeoutExpired) as error:
        report['failure_type'] = type(error).__name__
    finally:
        if child is not None and child.poll() is None:
            child.kill()
            child.wait(timeout=2)
        args.output.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'result': report['result']}))
    raise SystemExit(0 if report['result'] == 'pass' else 1)


if __name__ == '__main__':
    main()
