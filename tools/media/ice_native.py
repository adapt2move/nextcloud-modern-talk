#!/usr/bin/env python3
"""Finite generated-AV probe of observed ICE candidates and selected pairs.

The fixture references real disposable STUN/TURN infrastructure. A successful
process or allocation alone does not pass: relay requires selected relay pairs
and decoded AV at both ends. No SDP, candidate addresses or credentials persist.
"""
import argparse
import asyncio
import hashlib
import json
from pathlib import Path
import time


async def probe(args, report):
    peers = []
    readers = []
    results = [{}, {}]
    connected = [False, False]
    first_failure = None

    async def read(index):
        nonlocal first_failure
        child = peers[index]
        other = peers[1 - index]
        count = 0
        try:
            while line := await child.stdout.readline():
                count += 1
                if len(line) > 70000 or count > 400:
                    raise ValueError('bounded peer output exceeded')
                message = json.loads(line)
                kind = message.get('type')
                if kind in ('offer', 'answer', 'candidate'):
                    other.stdin.write(line)
                    await asyncio.wait_for(other.stdin.drain(), 2)
                elif kind == 'connection':
                    connected[index] = bool(message['connected'])
                elif kind == 'result':
                    results[index] = message
                else:
                    raise ValueError('unexpected peer event')
        except Exception as error:
            if first_failure is None:
                first_failure = {'peer': index, 'category': type(error).__name__}

    try:
        for index in range(2):
            peers.append(await asyncio.create_subprocess_exec(
                str(args.executable), *(['--offer'] if index == 0 else []),
                '--ice-fixture', str(args.fixture), stdin=asyncio.subprocess.PIPE,
                stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE, limit=140000))
        readers = [asyncio.create_task(read(i)) for i in range(2)]
        start = time.monotonic()
        both_at = None
        while time.monotonic() - start < 20:
            if all(connected) and both_at is None:
                both_at = time.monotonic()
            if both_at is not None and time.monotonic() - both_at >= 6:
                break
            if first_failure or any(p.returncode is not None for p in peers):
                break
            await asyncio.sleep(.05)
        for child in peers:
            if child.returncode is None:
                child.stdin.write(b'{"type":"stop"}\n')
                await asyncio.wait_for(child.stdin.drain(), 2)
        await asyncio.wait_for(asyncio.gather(*readers), 5)
        await asyncio.wait_for(asyncio.gather(*(p.wait() for p in peers)), 5)
        report['peers'] = results
        report['exit'] = [p.returncode for p in peers]
        report['first_failure'] = first_failure
        complete = all(p.returncode == 0 for p in peers) and all(r.get('closed') for r in results)
        av = all(r.get('audio_nonzero', 0) > 50 and r.get('video_received', 0) > 10 for r in results)
        if args.expect == 'relay':
            proof = av and all(r.get('ice', {}).get('relay_required')
                and r['ice'].get('selected', {}).get('local') == 'relay'
                and r['ice']['selected'].get('udp') for r in results)
        elif args.expect == 'srflx':
            proof = av and all(r.get('ice', {}).get('srflx_candidates', 0) > 0 for r in results)
        else:
            # Bad/expired credentials: a Relay call must never succeed via host.
            proof = not any(connected) and all(r.get('ice', {}).get('relay_required')
                and r['ice'].get('selected') is None
                and r.get('audio_received', 0) == 0 and r.get('video_received', 0) == 0 for r in results)
        report['passed'] = bool(complete and proof and first_failure is None)
    finally:
        for task in readers:
            if not task.done():
                task.cancel()
        await asyncio.gather(*readers, return_exceptions=True)
        for child in peers:
            if child.returncode is None:
                child.kill()
            await child.wait()
        # The native harness emits only bounded categories, never wire text.
        report['native_errors'] = [(await p.stderr.read(4096)).decode(errors='replace') for p in peers]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--executable', type=Path, required=True)
    parser.add_argument('--fixture', type=Path, required=True)
    parser.add_argument('--expect', choices=['srflx', 'relay', 'refused'], required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    report = {'passed': False, 'expect': args.expect, 'physical_devices': False,
              'harness_sha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest()}
    try:
        asyncio.run(asyncio.wait_for(probe(args, report), 35))
    except Exception as error:
        report.setdefault('first_failure', {'category': type(error).__name__})
    args.output.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'passed': report['passed'], 'output': str(args.output)}))
    raise SystemExit(0 if report['passed'] else 1)


if __name__ == '__main__':
    main()
