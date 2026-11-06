#!/usr/bin/env python3
"""Exercise content adapters against an owned development lab, then remove fixtures."""
import argparse
import asyncio
import json
import os
from pathlib import Path
import shutil
import tempfile
from urllib.parse import urlsplit

from alpha_protocol import Relay
from dev import owned_state, restore
from lab import save


async def exercise(state, output):
    root = Path(__file__).resolve().parents[2]
    with owned_state(state) as (_, record):
        lab = restore(record)
        room = lab.request('lab_alice', 'POST', '/ocs/v2.php/apps/spreed/api/v4/room',
                           body={'roomType': 2, 'roomName': 'Content fixture'},
                           expected=(201,), name='content-create-room')
        path = '/ocs/v2.php/apps/spreed/api/v4/room/' + room['token']
        directory = Path(tempfile.mkdtemp(prefix='nmt-content-', dir='/run'))
        os.chmod(directory, 0o700)
        try:
            lab.request('lab_alice', 'POST', path + '/participants',
                        body={'newParticipant': 'lab_bob', 'source': 'users'}, name='content-add-member')
            bob_room = lab.request('lab_bob', 'GET', path, name='content-bob-room')
            for user, data in [('alice', room), ('bob', bob_room)]:
                name = 'lab_' + user
                save(directory / (user + '.json'), {'origin': record['development']['client_url'],
                     'user': name, 'password': lab.passwords[name], 'room': data})
            parsed = urlsplit(record['development']['client_url'])
            host = urlsplit(record['development']['vm_url']).hostname
            env = dict(os.environ, NMT_FIXTURE_DIR=str(directory), RUST_BACKTRACE='0')
            command = ['python3', 'tools/cargo.py', 'test', '--locked', '-p', 'talk-adapters',
                       '--features', 'backend-test', '--test', 'content_backend', '--', '--ignored', '--nocapture']
            target = Path(output)
            target.mkdir(parents=True, exist_ok=True)
            async with Relay(host, parsed.port):
                with (target / 'content.log').open('w') as log:
                    process = await asyncio.create_subprocess_exec(*command, cwd=root, env=env,
                                                                   stdout=log, stderr=asyncio.subprocess.STDOUT)
                    try:
                        status = await asyncio.wait_for(process.wait(), 1200)
                    except BaseException:
                        process.kill()
                        await process.wait()
                        raise
            (target / 'result.json').write_text(json.dumps({'profile': record['development']['profile'],
                'exit_code': status, 'scope': 'synthetic released Talk content adapter; no native media claim'}, indent=2) + '\n')
            print('content adapter exit:', status, flush=True)
            return status
        finally:
            shutil.rmtree(directory)
            lab.request('lab_alice', 'DELETE', path, expected=(200, 404), name='content-remove-room')


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--state', required=True)
    parser.add_argument('--output', required=True)
    args = parser.parse_args()
    raise SystemExit(asyncio.run(exercise(args.state, args.output)))
