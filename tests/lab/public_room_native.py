#!/usr/bin/env python3
"""Native X11 interaction check against the private public-room lab intake."""
import argparse
import csv
import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import time


def run(*args):
    return subprocess.run(args, check=True, text=True, capture_output=True).stdout


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', required=True)
    parser.add_argument('--fixture', required=True)
    parser.add_argument('--output', required=True)
    args = parser.parse_args()
    output = Path(args.output)
    output.mkdir(mode=0o700)
    fixture = Path(args.fixture)
    profile = fixture.parent / 'native-profile'
    profile.mkdir(mode=0o700)
    run(args.binary, '--dev-seed', str(profile), str(fixture))
    env = dict(os.environ)
    env.pop('WAYLAND_DISPLAY', None)
    env['VK_ICD_FILENAMES'] = str(next(Path('/usr/share/vulkan/icd.d').glob('lvp_icd*.json')))
    log = (output / 'native.log').open('w')
    process = subprocess.Popen([args.binary, '--dev-profile', str(profile)], env=env, stdout=log, stderr=log)
    deadline = time.monotonic() + 30
    while True:
        found = subprocess.run(['xdotool', 'search', '--onlyvisible', '--pid', str(process.pid)], text=True, capture_output=True)
        if found.returncode == 0:
            window = found.stdout.splitlines()[0]
            run('xdotool', 'windowfocus', '--sync', window)
            break
        if time.monotonic() >= deadline:
            process.terminate()
            raise RuntimeError('Native window unavailable')
        time.sleep(0.2)
    screenshot = output / 'current.png'
    def words():
        run('import', '-window', 'root', str(screenshot))
        data = run('tesseract', str(screenshot), 'stdout', 'tsv')
        return [r for r in csv.DictReader(io.StringIO(data), delimiter='\t') if r['text'].strip()]
    def click(word, timeout=30, rightmost=False, minimum_x=0):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            rows = words()
            for r in sorted(rows, key=lambda row: int(row['left']), reverse=rightmost):
                if int(r['left']) >= minimum_x and word(r['text']):
                    x = int(r['left']) + int(r['width']) // 2
                    y = int(r['top']) + int(r['height']) // 2
                    run('xdotool', 'mousemove', str(x), str(y), 'click', '1')
                    return x, y
            time.sleep(0.5)
        raise RuntimeError('Native control not found; screenshot retained')
    def wait_text(expected, timeout=40):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            text = ' '.join(r['text'] for r in words())
            if any(word.lower() in text.lower() for word in (expected if isinstance(expected, tuple) else (expected,))):
                return
            time.sleep(0.5)
        raise RuntimeError('Native result missing; screenshot retained')
    try:
        # Real native mouse/key input; the intake grants no room membership.
        wait_text('Fixture group', timeout=60)
        click(lambda w: w in ('Offene', 'Open'))
        time.sleep(0.3)
        click(lambda w: w in ('Offene', 'Open'))
        heading_x, heading_y = click(lambda w: w in ('Offene', 'Open'), minimum_x=250)
        x, y = heading_x + 665, heading_y + 70
        run('xdotool', 'mousemove', str(heading_x+260), str(y), 'click', '1')
        data = json.loads(fixture.read_text())
        link = data['origin'].rstrip('/') + '/call/' + data['room']['token']
        run('xdotool', 'key', 'ctrl+a')
        run('xdotool', 'type', '--clearmodifiers', link)
        run('xdotool', 'mousemove', str(x), str(y), 'click', '1')
        wait_text(('Conversation via link', 'Gespräch über Link', 'Public access password'))
        heading_x, heading_y = click(lambda w: w in ('Offene', 'Open'), minimum_x=250)
        run('xdotool', 'mousemove', str(heading_x+665), str(heading_y+137), 'click', '1')
        wait_text(('This conversation requires a password', 'Dieses Gespräch benötigt ein Passwort'))
        run('import', '-window', 'root', str(output / 'password.png'))
        _, password_y = click(lambda w: w.lower() in ('passwort', 'password'), rightmost=True)
        run('xdotool', 'type', '--clearmodifiers', 'Synthetic room password')
        run('xdotool', 'mousemove', '1180', str(password_y), 'click', '1')
        wait_text('sentinel')
        run('import', '-window', 'root', str(output / 'history.png'))
        run('xdotool', 'mousemove', '600', '750', 'click', '1')
        run('xdotool', 'type', '--clearmodifiers', 'Native public room reply')
        run('xdotool', 'key', 'Return')
        time.sleep(3)
        run('import', '-window', 'root', str(output / 'sent.png'))
        (output / 'result.json').write_text(json.dumps({'result':'pass', 'binary_sha256':hashlib.sha256(Path(args.binary).read_bytes()).hexdigest(), 'scope':'Linux Xvfb/Mesa native link entry, password, history and send; no media/platform parity claim'})+'\n')
    finally:
        process.terminate()
        try:
            process.wait(timeout=15)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
        log.close()


if __name__ == '__main__':
    main()
