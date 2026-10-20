#!/usr/bin/env python3
"""Small source-bound R7 native probe: one hint owner and visible history focus.

Uses real keyboard/pointer input in fictional disconnected GPUI windows. Original
PNGs and raw OCR are retained; pixel assertions supplement author inspection.
No state injection, backend, media, AX or platform qualification.
"""
import argparse
import importlib.util
import json
import re
import subprocess
import sys
import time
from pathlib import Path

p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--build-input', type=Path, required=True)
p.add_argument('--output', type=Path, required=True)
p.add_argument('--batch', choices=['hints', 'history'], required=True)
p.add_argument('--scenarios', required=True)
p.add_argument('--scale', type=int, default=100)
p.add_argument('--width', type=int, default=1120)
p.add_argument('--height', type=int, default=716)
a = p.parse_args()
base, out = a.build_input.resolve(), a.output.resolve()
sys.path.insert(0, str(base / 'source/tools/ui'))
from capture import run, sha
import interactions_d1_2
spec = importlib.util.spec_from_file_location('frozen', base / 'source/tools/ui/capture-frozen.py')
f = importlib.util.module_from_spec(spec)
spec.loader.exec_module(f)
binding, _ = f.validate(base)
BOUNDS = r'Bounds \{ origin: Point \{ x: ([-\d.]+)px, y: ([-\d.]+)px \}, size: Size \{ ([-\d.]+)px × ([-\d.]+)px'


def exercise(scenario, env, xid, folder, label, large=False):
    events, frames, checks = [], [], []
    log = folder / (label + '.log')

    def key(value):
        run(['xdotool', 'key', '--clearmodifiers', value], env)
        events.append({'key': value})
        time.sleep(.18)

    def move(x, y):
        run(['xdotool', 'mousemove', '--window', xid, str(x), str(y)], env)
        events.append({'pointer': [x, y], 'wait_ms': 700})
        time.sleep(.7)

    def diag():
        count = log.read_text().count('UI_GEOMETRY ')
        key('F7')
        deadline = time.monotonic() + 10
        while log.read_text().count('UI_GEOMETRY ') <= count:
            if time.monotonic() > deadline:
                raise AssertionError('No native diagnostic acknowledgement')
            time.sleep(.03)
        lines = log.read_text().splitlines()
        return {name: next((s for s in reversed(lines) if s.startswith(prefix)), '')
                for name, prefix in [('state', 'UI_LOCAL_STATE'), ('history', 'UI_HISTORY'),
                                     ('geometry', 'UI_GEOMETRY'), ('focus', 'UI_ACTION_FOCUS'),
                                     ('hover', 'UI_HOVER')]}

    def dispatch():
        return [s for s in log.read_text().splitlines()
                if s.startswith(('UI_INTENT', 'UI_DISPATCH', 'UI_FOCUS_PROBE'))]

    def check(name, ok, evidence):
        checks.append({'check': name, 'result': 'pass' if ok else 'FAIL', 'evidence': evidence})
        if not ok:
            raise AssertionError(name + ': ' + str(evidence))

    def focus_until(predicate):
        for _ in range(25):
            key('Tab')
            d = diag()
            if predicate(d):
                return d
        raise AssertionError('Keyboard target not reached: ' + str(d))

    def pixels(path):
        return subprocess.run(['convert', str(path), '-depth', '8', 'rgb:-'],
                              env=env, check=True, capture_output=True).stdout

    def image(name, expected=None):
        time.sleep(.35)
        d = diag()
        q = folder / (label + '-r7-' + name + '.png')
        run(['import', '-window', xid, str(q)], env)
        raw = run(['tesseract', str(q), 'stdout', '-l', 'deu+eng', '--psm', '11'], env).stdout
        (folder / (q.stem + '-raw-ocr.txt')).write_text(raw)
        frame = {'name': name, 'image': q.name, 'sha256': sha(q), 'events': events.copy(),
                 'diagnostics': d, 'dispatch': dispatch(), 'raw_ocr': raw,
                 'full_expected_hint_for_original_inspection': expected}
        frames.append(frame)
        return frame

    def hint_boxes(frame):
        # Connected native tooltip-background pixels. Text glyphs elsewhere
        # cannot meet the minimum area; a merged two-tip shape is caught by its
        # height as well as its component count (not just OCR substrings).
        rgb = pixels(folder / frame['image'])
        color = bytes.fromhex('e9eeea' if '-dark-' in label else '26354b')
        points = {i // 3 for i in range(0, len(rgb), 3) if rgb[i:i+3] == color}
        boxes = []
        while points:
            todo = [points.pop()]
            xmin, ymin, xmax, ymax, area = a.width, a.height, 0, 0, 0
            while todo:
                v = todo.pop(); x, y = v % a.width, v // a.width
                xmin, ymin, xmax, ymax, area = min(xmin, x), min(ymin, y), max(xmax, x), max(ymax, y), area + 1
                for n in (v-1 if x else -1, v+1 if x+1 < a.width else -1, v-a.width, v+a.width):
                    if n in points:
                        points.remove(n); todo.append(n)
            if area > 600 and xmax-xmin >= 60 and ymax-ymin >= 14:
                boxes.append([xmin, ymin, xmax-xmin+1, ymax-ymin+1, area])
        return boxes

    def ring(frame):
        b = re.findall(BOUNDS, frame['diagnostics']['geometry'].split('viewports=')[1])[3]
        x, y, w, h = map(lambda s: round(float(s)), b)
        rgb = pixels(folder / frame['image'])
        color = bytes.fromhex('b7d7bf' if '-dark-' in label else '356fc1')
        def has(xx, yy):
            return 0 <= xx < a.width and 0 <= yy < a.height and rgb[(yy*a.width+xx)*3:(yy*a.width+xx)*3+3] == color
        edges = [sum(any(has(xx, yy+d) for d in range(-3, 4)) for xx in range(x+4, x+w-4)) for yy in [y, y+h]]
        edges += [sum(any(has(xx+d, yy) for d in range(-3, 4)) for yy in range(y+4, y+h-4)) for xx in [x, x+w]]
        check('history-focus-visible-on-all-four-sides', min(edges[:2]) > .8*(w-8) and min(edges[2:]) > .8*(h-8), {'bounds': b, 'edge_pixels': edges})

    def offset(d):
        m = re.search(r'y: ([-\d.]+)px.*× ([-\d.]+)px', d['history'])
        return tuple(map(float, m.groups()))

    try:
        move(a.width-5, a.height-5)
        admitted = dispatch()
        if a.batch == 'hints':
            focus_until(lambda d: 'Some("call")' in d['focus'])
            focused = image('call-focus-only', 'Anruf beitreten\nDu bist offline.')
            y = 50 if a.scale == 100 else 110
            move(a.width-54, y)
            hover = image('call-focus-details-hover-700ms', 'Gesprächsdetails')
            boxes = hint_boxes(hover)
            check('one-unambiguous-details-hint', len(boxes) == 1 and boxes[0][3] <= 16*a.scale/100+12, boxes)
            check('hover-does-not-move-call-focus', 'Some("call")' in hover['diagnostics']['focus'], hover['diagnostics'])
            # The actual focused call rectangle is unchanged while its hint is
            # suppressed. Compare the complete ring/control rectangle.
            b = re.search('focus=Some\\(' + BOUNDS, focused['diagnostics']['geometry'])
            x, yy, w, h = map(lambda s: round(float(s)), b.groups())
            left, right = pixels(folder/focused['image']), pixels(folder/hover['image'])
            equal = all(left[(j*a.width+x)*3:(j*a.width+x+w)*3] == right[(j*a.width+x)*3:(j*a.width+x+w)*3] for j in range(yy, yy+h))
            check('call-control-and-ring-retained', equal, [x, yy, w, h])
            move(a.width-5, a.height-5)
            restored = image('hover-exit-restores-call-reason', 'Anruf beitreten\nDu bist offline.')
            check('focus-only-recovery-pixel-exact', restored['sha256'] == focused['sha256'], [focused['sha256'], restored['sha256']])
            focus_until(lambda d: 'history_focused=true' in d['state'])
            move(a.width-54, y)
            only = image('details-hover-only-700ms', 'Gesprächsdetails')
            boxes = hint_boxes(only)
            check('hover-only-name-remains-useful', len(boxes) == 1 and boxes[0][3] <= 16*a.scale/100+12, boxes)
            key('ctrl+comma'); modal = diag()
            check('modal-cancels-hover-owner', '(false, false,' in modal['hover'], modal)
            key('Escape'); time.sleep(.7); returned = diag()
            check('retired-hover-does-not-reappear', '(false, false,' in returned['hover'], returned)
        else:
            if scenario.startswith(('call', 'reconnecting')):
                key('Tab')
                focused = image('first-Tab-history-focus')
            else:
                focus_until(lambda d: 'history_focused=true' in d['state'])
                focused = image('conversation-history-focus')
            check('actual-history-focus', 'history_focused=true' in focused['diagnostics']['state'], focused['diagnostics'])
            ring(focused)
            key('Home'); oldest = diag(); key('Next'); middle = diag(); key('End'); latest = diag()
            check('history-keyboard-scrolling', abs(offset(oldest)[0]) < 2 and offset(middle)[0] < -2 and abs(sum(offset(latest))) < 2, [oldest, middle, latest])
            key('ctrl+comma'); modal = diag(); saved = offset(modal); key('Prior'); blocked = diag()
            check('modal-excludes-history', 'history_focused=false' in modal['state'] and offset(blocked) == saved, [modal, blocked])
            key('Escape'); returned = diag()
            check('modal-restores-history-focus', 'history_focused=true' in returned['state'], returned)
            if scenario.startswith(('call', 'reconnecting')):
                focus_until(lambda d: 'Some("microphone")' in d['focus'])
                image('toolbar-keyboard-focus')
                key('Escape')
        check('navigation-only-no-dispatch', dispatch() == admitted, dispatch())
        return checks
    finally:
        (folder / (label + '-r7-observations.json')).write_text(json.dumps({'batch': a.batch, 'frames': frames, 'checks': checks, 'hover_wait_ms': 700, 'wheel_events': 0}, indent=2)+'\n')


interactions_d1_2.exercise = exercise
sys.argv = ['capture-frozen.py', '--build-input', str(base), '--output', str(out),
            '--scenarios', a.scenarios, '--scale', str(a.scale), '--width', str(a.width),
            '--height', str(a.height), '--interactions', '--interaction-suite', 'd1-2']
try:
    f.main()
finally:
    f.validate(base)
    (out / 'r7-recipe-binding.json').write_text(json.dumps({'recipe_sha256': sha(__file__), 'compiled_input': binding, 'source_unchanged': True}, indent=2)+'\n')
