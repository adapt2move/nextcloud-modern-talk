#!/usr/bin/env python3
"""Finite native chat-control checks against one frozen compiled input.

Synthetic chat-ux/chat-ux-call/chat-motion projections only; the disconnected fixture never
confirms server sending, calling or update installation. Every PNG is an original
native window. Actual native emoji insertion and IME composition need the
separate platform steps in the author handoff; this recipe does not simulate them.
"""
import argparse
import hashlib
import importlib.util
import json
import re
import subprocess
import sys
import time
from pathlib import Path

BOUNDS = r'Bounds \{ origin: Point \{ x: ([-\d.]+)px, y: ([-\d.]+)px \}, size: Size \{ ([-\d.]+)px × ([-\d.]+)px'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--build-input', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--batch', choices=['controls', 'search', 'header'], required=True)
    parser.add_argument('--scenarios', default='chat-ux:light:blue,chat-ux:dark:sage')
    parser.add_argument('--scale', type=int, choices=[100, 200], default=100)
    parser.add_argument('--width', type=int, default=1120)
    parser.add_argument('--height', type=int)
    args = parser.parse_args()
    if args.height is None:
        # Four chat-ux messages must overflow to exercise live scroll-behind.
        args.height = 620 if args.batch == 'header' else 716
    base, output = args.build_input.resolve(), args.output.resolve()
    cases = args.scenarios.split(',')
    allowed = ('chat-ux', 'chat-motion') if args.batch == 'header' else ('chat-ux', 'chat-ux-call')
    if any(case.split(':')[0] not in allowed for case in cases):
        parser.error('This batch requires its explicit synthetic chat scenarios')
    if args.batch == 'controls' and any(case.startswith('chat-ux-call:') for case in cases):
        parser.error('Use the search batch for the in-call surface')
    sys.path.insert(0, str(base / 'source/tools/ui'))
    from capture import run, sha
    import interactions_d1_2
    spec = importlib.util.spec_from_file_location('frozen', base / 'source/tools/ui/capture-frozen.py')
    frozen = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(frozen)
    binding, _ = frozen.validate(base)

    def exercise(scenario, env, xid, folder, label, large=False):
        events, frames, checks = [], [], []
        log = folder / (label + '.log')
        diagnostic_count = 0

        def key(value):
            run(['xdotool', 'key', '--clearmodifiers', value], env)
            events.append({'key': value})
            time.sleep(.16)

        def type_text(value):
            run(['xdotool', 'type', '--clearmodifiers', '--delay', '5', '--', value], env)
            events.append({'synthetic_text': value})
            time.sleep(.16)

        def move(x=None, y=None):
            point = [args.width-4 if x is None else round(x), args.height-4 if y is None else round(y)]
            run(['xdotool', 'mousemove', '--window', xid, *map(str, point)], env)
            events.append({'pointer': point})
            time.sleep(.65)

        def diagnostic():
            nonlocal diagnostic_count
            if diagnostic_count >= 60:
                raise AssertionError('Finite diagnostic budget exhausted; preserve this first failure')
            diagnostic_count += 1
            count = log.read_text().count('UI_GEOMETRY ')
            key('F7')
            deadline = time.monotonic() + 3
            while log.read_text().count('UI_GEOMETRY ') <= count:
                if time.monotonic() >= deadline:
                    raise AssertionError('No native diagnostic acknowledgement')
                time.sleep(.03)
            lines = log.read_text().splitlines()
            return {name: next((line for line in reversed(lines) if line.startswith(prefix)), '')
                    for name, prefix in [('chat', 'UI_CHAT_STATE'), ('state', 'UI_LOCAL_STATE'),
                                         ('focus', 'UI_ACTION_FOCUS'), ('hover', 'UI_HOVER'),
                                         ('geometry', 'UI_GEOMETRY'), ('history', 'UI_HISTORY'),
                                         ('header', 'UI_CHAT_HEADER'), ('motion', 'UI_MESSAGE_MOTION')]}

        def dispatch():
            return [line for line in log.read_text().splitlines()
                    if line.startswith(('UI_CHAT_UX ', 'UI_INTENT '))]

        def check(name, ok, observation):
            checks.append({'check': name, 'result': 'pass' if ok else 'FAIL', 'observation': observation})
            if not ok:
                raise AssertionError(name + ': ' + str(observation))

        def image(name, data=None):
            time.sleep(.3)
            data = data or diagnostic()
            path = folder / f'{label}-chat-ux-{len(frames)+1:02}-{name}.png'
            run(['import', '-window', xid, str(path)], env)
            frame = {'name': name, 'image': path.name, 'sha256': sha(path),
                     'diagnostics': data, 'events': events.copy(), 'dispatch': dispatch()}
            frames.append(frame)
            return frame

        def pixels(frame):
            raw = subprocess.run(
                ['convert', str(folder / frame['image']), '-depth', '8', 'rgba:-'],
                env=env, check=True, capture_output=True, timeout=20,
            ).stdout
            return hashlib.sha256(raw).hexdigest()

        def bounds(data, field):
            source = data['chat'].split(field + '=', 1)[1] if field.endswith('_bounds') else data['geometry'].split('focus=', 1)[1]
            match = re.search(BOUNDS, source)
            if not match:
                raise AssertionError('No rendered native bounds for ' + field + ': ' + source)
            return tuple(map(float, match.groups()))

        def click_composer(data):
            x, y, width, height = bounds(data, 'composer_bounds')
            check('composer-visible', 0 <= x < args.width and 0 <= y < args.height
                  and width > 10 and height > 10, [x, y, width, height])
            move(x + min(width/2, 100), y + min(height/2, 16))
            run(['xdotool', 'click', '1'], env)
            time.sleep(.16)

        def focus_named(name, backwards=False):
            for _ in range(24):
                key('shift+Tab' if backwards else 'Tab')
                data = diagnostic()
                if f'Some("{name}")' in data['focus']:
                    return data
            raise AssertionError('Native keyboard target not reached: ' + name)

        def header_bounds(data):
            match = re.search(r'inset=([\d.]+)px', data['header'])
            native = re.search(BOUNDS, data['header'].split('list=', 1)[1])
            if not match or not native:
                raise AssertionError('No measured native header/list bounds: ' + data['header'])
            x, y, width, height = map(float, native.groups())
            inset = float(match[1])
            check('header-and-history-have-a-visible-viewport',
                  0 < inset < height and width > 32, data['header'])
            return x, y, width, inset, height

        def header_pixels(frame):
            x, y, width, inset, _ = header_bounds(frame['diagnostics'])
            # The untouched full native PNG is the evidence/deliverable.
            # Cropping occurs only in memory to compare its header pixels.
            crop = f'{round(width-8)}x{round(inset-8)}+{round(x+4)}+{round(y+4)}'
            raw = subprocess.run(
                ['convert', str(folder / frame['image']), '-crop', crop, '-depth', '8', 'rgba:-'],
                env=env, check=True, capture_output=True, timeout=20,
            ).stdout
            return hashlib.sha256(raw).hexdigest()

        def wheel(data, button, count):
            x, y, width, inset, height = header_bounds(data)
            move(x + width / 2, y + inset + (height - inset) / 2)
            run(['xdotool', 'click', '--repeat', str(count), '--delay', '40', str(button)], env)
            events.append({'wheel': button, 'count': count})
            move()

        try:
            move()
            initial = image('initial-no-hover')
            if args.batch == 'header':
                header_bounds(initial['diagnostics'])
                click_composer(initial['diagnostics'])
                focus_named('room-info', backwards=True)
                key('Tab')
                history_focus = diagnostic()
                check('header-to-history-keeps-native-tab-order',
                      'history_focused=true' in history_focus['state'], history_focus)
                key('Home')
                click_composer(diagnostic())
                move()
                top = image('history-top-clear-of-header')
                wheel(top['diagnostics'], 5, 3)
                behind = image('live-content-under-translucent-header')
                anchors = [re.search(r'anchor=(.*?) at_bottom=', frame['diagnostics']['motion'])[1]
                           for frame in [top, behind]]
                check('wheel-moves-real-history', anchors[0] != anchors[1], anchors)
                check('scroll-retains-header-and-composer-geometry',
                      header_bounds(top['diagnostics']) == header_bounds(behind['diagnostics'])
                      and bounds(top['diagnostics'], 'composer_bounds')
                      == bounds(behind['diagnostics'], 'composer_bounds'), behind['diagnostics'])
                check('real-header-pixels-change-with-scrolling-content',
                      header_pixels(top) != header_pixels(behind),
                      [header_pixels(top), header_pixels(behind)])
                time.sleep(.7)
                settled = image('scroll-under-header-settled')
                check('header-settles-without-permanent-animation',
                      header_pixels(behind) == header_pixels(settled)
                      and behind['diagnostics']['motion'] == settled['diagnostics']['motion'],
                      [behind['diagnostics']['motion'], settled['diagnostics']['motion']])
                before = dispatch()
                x, y, _, inset, _ = header_bounds(settled['diagnostics'])
                move(x + 6, y + inset / 2)
                run(['xdotool', 'click', '1'], env)
                move()
                shielded = image('header-background-keeps-history-controls-inert')
                check('header-background-does-not-dispatch-hidden-message-actions',
                      dispatch() == before and 'menu=None' in shielded['diagnostics']['chat']
                      and 'reply=None' in shielded['diagnostics']['chat'], shielded['diagnostics'])
            elif args.batch == 'search':
                key('ctrl+f')
                opened = image('search-below-header')
                check('primary-F-focuses-current-conversation-search',
                      'search_open=true search_focused=true' in opened['diagnostics']['chat'],
                      opened['diagnostics'])
                type_text('Perfekt')
                key('Return')
                results = image('real-typed-search-results')
                check('search-is-scoped-to-fixture-room',
                      'UI_CHAT_UX search room=1 results=1' in dispatch(), dispatch())
                # Search input -> submit -> close -> actual first result.
                key('Tab'); key('Tab'); key('Tab'); key('Return')
                located = image('search-result-context')
                check('result-uses-context-action',
                      'UI_CHAT_UX locate room=1 message=4' in dispatch(), dispatch())
                key('Escape')
                closed = diagnostic()
                check('Escape-closes-field-without-leaving-call',
                      'search_open=false' in closed['chat']
                      and not any(line.startswith('UI_INTENT leave ') for line in dispatch()), closed)
                click_composer(closed)
                type_text('ALPHAOMEGA')
                key('Home')
                for _ in range(5):
                    key('Right')
                key('ctrl+f')
                focused = diagnostic()
                check('search-preserves-draft',
                      'search_open=true search_focused=true' in focused['chat']
                      and 'editor_chars=10' in focused['state'], focused)
                key('Escape')
                type_text('X')
                key('ctrl+a'); key('ctrl+c')
                copied = run(['xclip', '-selection', 'clipboard', '-o'], env).stdout
                check('Escape-restores-exact-composer-caret', copied == 'ALPHAXOMEGA', copied)
                key('Right')
                image('draft-and-caret-retained')
                key('ctrl+f'); key('ctrl+f')
                toggled = diagnostic()
                check('second-primary-F-toggles-focused-field-closed',
                      'search_open=false' in toggled['chat'], toggled)
                key('ctrl+comma')
                modal = diagnostic()
                key('ctrl+f')
                blocked = image('modal-does-not-open-background-search')
                check('modal-keeps-search-shortcut-scoped',
                      'overlay=Some(Settings)' in modal['state']
                      and 'overlay=Some(Settings)' in blocked['diagnostics']['state']
                      and 'search_open=false' in blocked['diagnostics']['chat'],
                      blocked['diagnostics'])
                key('Escape')
                image('modal-return')
            else:
                click_composer(initial['diagnostics'])
                title = focus_named('conversation-title', backwards=True)
                image('title-focus', title)
                key('Return')
                shared = image('title-shared-content-dialog')
                check('title-opens-current-room-shared-content',
                      'overlay=Some(Files)' in shared['diagnostics']['state']
                      and 'UI_CHAT_UX shared room=1' in dispatch(), shared)
                key('ctrl+f')
                blocked = diagnostic()
                check('shared-dialog-blocks-background-search',
                      'overlay=Some(Files)' in blocked['state'] and 'search_open=false' in blocked['chat'], blocked)
                key('Escape')
                favorite = focus_named('favorite-conversation')
                image('favorite-focus-name', favorite)
                key('Return')
                changed = image('favorite-confirmed-sidebar-star')
                check('favorite-dispatches-typed-current-room-action',
                      any(line.startswith('UI_CHAT_UX favorite room=1 value=') for line in dispatch()), dispatch())
                x, y, width, height = bounds(changed['diagnostics'], 'focus')
                move(x+width/2, y+height/2)
                first = image('favorite-hover-and-focus-one-hint')
                time.sleep(.65)
                settled = image('favorite-hover-settled')
                check('finite-hover-settles', pixels(first) == pixels(settled)
                      and first['diagnostics']['hover'] == settled['diagnostics']['hover'],
                      [pixels(first), pixels(settled), settled['diagnostics']['hover']])
                move()
                click_composer(settled['diagnostics'])
                menu = focus_named('message-menu', backwards=True)
                image('message-menu-focus', menu)
                key('Return')
                expanded = image('message-menu-open')
                check('message-menu-opens-for-canonical-message',
                      'menu=Some(' in expanded['diagnostics']['chat'], expanded['diagnostics'])
                key('Return')
                reply = focus_named('message-reply', backwards=True)
                image('reply-focus', reply)
                x, y, width, height = bounds(reply, 'focus')
                click_composer(reply)
                move()
                hidden = image('message-controls-no-hover')
                move(x+width/2, y+height/2)
                hovered = image('message-controls-hover')
                check('hover-keeps-message-and-composer-layout',
                      bounds(hidden['diagnostics'], 'composer_bounds') == bounds(hovered['diagnostics'], 'composer_bounds')
                      and hidden['diagnostics']['history'] == hovered['diagnostics']['history'],
                      [hidden['diagnostics'], hovered['diagnostics']])
                run(['xdotool', 'click', '1'], env)
                target = image('reply-composer-target')
                check('quick-reply-selects-real-quote-target',
                      'reply=Some(4)' in target['diagnostics']['chat'], target['diagnostics'])
                type_text('QUOTE')
                key('Return')
                sent = image('quote-intent-failure-retains-draft')
                check('quote-send-dispatch-retains-target',
                      any('UI_CHAT_UX send room=1 bytes=5 reply=Some(4)' in line for line in dispatch()), dispatch())
                click_composer(sent['diagnostics'])
                emoji = focus_named('emoji-picker')
                image('native-picker-support-or-unavailable-explanation', emoji)
            return checks
        finally:
            (folder / (label + '-chat-ux-observations.json')).write_text(json.dumps({
                'synthetic_fixture': True, 'batch': args.batch, 'frames': frames, 'checks': checks,
                'manual_original_inspection_required': [
                    'Lucide geometry, one styled hint, clear focus and selected star',
                    'Reply/menu reveal only on owning row hover or control focus',
                    'No clipping or overlap at selected scale; shared media/member/settings separation',
                    'Header batch: live text scrolls underneath; first row, focus and toolbar stay readable',
                ],
                'header_effect': 'Live timeline under a translucent header; backdrop blur is requested from GPUI',
                'not_qualified': ['backend delivery', 'media', 'native emoji insertion', 'IME', 'accessibility semantics',
                                  'Windows/macOS', 'energy performance', 'update installation'],
            }, indent=2) + '\n')

    interactions_d1_2.exercise = exercise
    sys.argv = ['capture-frozen.py', '--build-input', str(base), '--output', str(output),
                '--width', str(args.width), '--height', str(args.height), '--scale', str(args.scale),
                '--scenarios', args.scenarios, '--interactions', '--interaction-suite', 'd1-2']
    try:
        frozen.main()
    finally:
        frozen.validate(base)
        output.mkdir(parents=True, exist_ok=True)
        (output / 'chat-ux-recipe-binding.json').write_text(json.dumps({
            'recipe_sha256': sha(__file__), 'compiled_input': binding, 'synthetic_fixture': True,
            'batch': args.batch, 'scale': args.scale, 'first_failures_preserved': True,
        }, indent=2) + '\n')


if __name__ == '__main__':
    main()
