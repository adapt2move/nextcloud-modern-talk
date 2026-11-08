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
    os.environ.setdefault('OMP_THREAD_LIMIT', '1')
    env = dict(os.environ)
    env.setdefault('LP_NUM_THREADS', '1')
    env.pop('WAYLAND_DISPLAY', None)
    env['VK_ICD_FILENAMES'] = str(next(Path('/usr/share/vulkan/icd.d').glob('lvp_icd*.json')))
    log = (output / 'native.log').open('w')
    process = subprocess.Popen([args.binary, '--dev-profile', str(profile)], env=env, stdout=log, stderr=log)
    deadline = time.monotonic() + 30
    while True:
        found = subprocess.run(['xdotool', 'search', '--onlyvisible', '--pid', str(process.pid)], text=True, capture_output=True)
        if found.returncode == 0:
            window = found.stdout.splitlines()[0]
            # Match the inspected native fixture viewport before using its
            # fixed sidebar gutter; the app otherwise opens centered at 1200 pt.
            run('xdotool', 'windowmove', '--sync', window, '0', '0')
            run('xdotool', 'windowsize', '--sync', window, '1280', '800')
            run('xdotool', 'windowfocus', '--sync', window)
            break
        if time.monotonic() >= deadline:
            process.terminate()
            raise RuntimeError('Native window unavailable')
        time.sleep(0.2)
    screenshot = output / 'current.png'
    def words():
        run('import', '-window', 'root', str(screenshot))
        # Modal headings, muted prompts and white button labels need different
        # contrast. Read the same pixels with a bounded set of OCR passes.
        rows = []
        enlarged = output / 'current-ocr.png'
        for scale, invert in ((2, False), (1, False), (2, True)):
            source = screenshot
            if scale != 1 or invert:
                command = ['convert', str(screenshot), '-resize', f'{scale * 100}%']
                if invert:
                    command += ['-colorspace', 'gray', '-negate', '-auto-level']
                run(*command, str(enlarged))
                source = enlarged
            data = run('tesseract', str(source), 'stdout', '--psm', '11', 'tsv')
            for row in csv.DictReader(io.StringIO(data), delimiter='\t'):
                if not row['text'].strip():
                    continue
                for field in ('left', 'top', 'width', 'height'):
                    row[field] = str(int(row[field]) // scale)
                rows.append(row)
        enlarged.unlink(missing_ok=True)
        return rows
    def click(word, timeout=30, rightmost=False, minimum_x=0, maximum_x=None):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            rows = words()
            for r in sorted(rows, key=lambda row: int(row['left']), reverse=rightmost):
                if (int(r['left']) >= minimum_x
                        and (maximum_x is None or int(r['left']) < maximum_x)
                        and word(r['text'])):
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
        # Discovery belongs to the conversation heading's + menu. The sidebar
        # is a fixed 260 pt column; only its vertical position follows the text.
        _, heading_y = click(lambda w: w in ('Gespräche', 'Conversations'), maximum_x=250)
        run('xdotool', 'mousemove', '201', str(heading_y), 'click', '1')
        click(lambda w: w in ('Offene', 'Open', 'Browse'))
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
        click(lambda w: w.lower() in ('passwort', 'password'), rightmost=True)
        run('xdotool', 'type', '--clearmodifiers', 'Synthetic room password')
        # From the password editor, native Tab reaches its adjacent Join action.
        # White button labels are unreliable OCR targets on the blue fill.
        run('xdotool', 'key', '--clearmodifiers', 'Tab')
        time.sleep(.5)
        run('xdotool', 'key', '--clearmodifiers', 'Return')
        wait_text('sentinel')
        run('import', '-window', 'root', str(output / 'history.png'))
        # Inspected text surface above the action row in the fixed viewport.
        run('xdotool', 'mousemove', '600', '700', 'click', '1')
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
