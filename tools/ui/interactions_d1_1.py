"""R2 A/B native evidence. X11 keyboard/pointer input only; no injected UI state.

Original window PNGs remain untouched. Temporary OCR inversions are discarded.
Pixel comparisons identify the measured regions and never replace screenshots.
"""
import csv
import io
import json
import re
import time
from capture import run, sha
from interactions import normalized


def exercise(scenario, env, xid, out, label, large=False):
    checks = []
    frames = []
    step = 0
    def key(k):
        run(['xdotool', 'key', '--clearmodifiers', k], env)
        time.sleep(.15)
    def move(x=1100, y=705):
        run(['xdotool', 'mousemove', '--window', xid, str(x), str(y)], env)
    def click(x, y):
        move(x, y)
        run(['xdotool', 'click', '1'], env)
        time.sleep(.12)
    def type_text(text):
        run(['xdotool', 'type', '--clearmodifiers', '--delay', '3', '--', text], env)
    def observe(name, pointer=True):
        nonlocal step
        step += 1
        if pointer: move(700 if large else 1100, 570 if large else 705)
        time.sleep(.35)
        p = out / f'{label}-d11-{step:02}-{name}.png'
        run(['import', '-window', xid, str(p)], env)
        frames.append({'name': name, 'image': p.name, 'sha256': sha(p)})
        return p
    def text(p):
        value = run(['tesseract', str(p), 'stdout', '--psm', '11'], env).stdout
        value += run(['tesseract', str(p), 'stdout', '--psm', '6'], env).stdout
        derived = out / 'ocr-temporary.png'
        run(['convert', str(p), '-colorspace', 'gray', '-threshold', '55%', '-negate', '-resize', '200%', str(derived)], env)
        value += run(['tesseract', str(derived), 'stdout', '--psm', '11'], env).stdout
        derived.unlink()
        return normalized(value).replace(' ', '')
    def check(name, p, assertion, detail):
        result = {'check': name, 'result': 'pass' if assertion else 'FAIL', 'image': p.name, 'sha256': sha(p), 'observation': detail}
        checks.append(result)
        if not assertion: raise AssertionError(f'{name}: {p.name}: {detail}')
    def expect(name, required=(), absent=()):
        p = observe(name)
        t = text(p)
        check(name, p, all(normalized(x).replace(' ', '') in t for x in required) and all(normalized(x).replace(' ', '') not in t for x in absent), t)
        return p
    def region(p, rect):
        return run(['convert', str(p), '-crop', rect, '+repage', '-depth', '8', 'txt:-'], env).stdout
    def same(a, b, rect): return region(a, rect) == region(b, rect)
    def pixel(p, x, y):
        value = run(['convert', str(p), '-format', f'%[pixel:p{{{x},{y}}}]', 'info:'], env).stdout
        return tuple(float(n) for n in re.findall(r'[\d.]+', value)[:3])
    def contrast(a, b):
        def luminance(c):
            v = [x / 255 for x in c]
            v = [x / 12.92 if x <= .04045 else ((x + .055) / 1.055) ** 2.4 for x in v]
            return sum(x * k for x, k in zip(v, [.2126, .7152, .0722]))
        x, y = luminance(a), luminance(b)
        return (max(x,y) + .05) / (min(x,y) + .05)
    def click_word(word):
        p = observe('locate-' + word)
        tsv = run(['tesseract', str(p), 'stdout', '--psm', '11', 'tsv'], env).stdout
        words = [r for r in csv.DictReader(io.StringIO(tsv), delimiter='\t') if normalized(word) in normalized(r.get('text', ''))]
        if not words: raise AssertionError(f'Native visible word missing: {word}')
        w = words[-1]
        click(int(w['left']) + int(w['width'])//2, int(w['top']) + int(w['height'])//2)
    try:
        if large:
            initial = observe('B11-200pct-initial')
            if scenario == 'chat':
                check('B11-file-name-does-not-break', initial, normalized('Navigation-v3.fig') in text(initial), 'Native OCR has the complete filename on one line; full-name tooltip remains available')
            move(650, 430)
            run(['xdotool', 'click', '--repeat', '35', '--delay', '20', '5'], env)
            if scenario == 'chat':
                expect('B11-200pct-composer-reachable', ['Nachricht', 'senden'])
                click_word('Nachricht'); type_text('SKALENTWURF')
                before = observe('B11-200pct-before-send')
                key('Return')
                after = expect('B11-200pct-failed-draft-and-send-reachable', ['SKALENTWURF', 'Nicht gesendet', 'senden'])
                check('B11-feedback-does-not-move-composer', after, same(before, after, '620x60+40+440'), 'Bottom composer/action region stays pixel-identical; overlay allocates no height')
            elif scenario == 'settings':
                expect('200pct-settings-notice-reachable', ['bis zum Beenden'])
                key('Escape'); expect('200pct-settings-escape', ['Produktteam'], ['Darstellung'])
            elif scenario == 'server':
                p = observe('200pct-server-action-reachable')
                click(360, 390)
                expect('200pct-onboarding-action', ['Warten auf Freigabe', 'Wird verarbeitet', 'Anmeldung abbrechen'])
            return checks
        if scenario == 'server':
            base = observe('B6-unfocused')
            key('Tab')
            focused = observe('B6-server-container-focus')
            check('B6-outer-ring-includes-globe', focused, pixel(base, 540, 338) != pixel(focused, 540, 338), 'Outer top edge above globe changes; no nested TextInput focus border')
            key('ctrl+a'); type_text('probe.example.test'); key('Home'); key('Delete'); key('End'); key('Tab')
            expect('bounded-server-native-editing', ['robe.example.test'], ['probe.example.test'])
        elif scenario in ('chat', 'busy'):
            key('Escape')
            baseline = observe('B1-B2-unfocused-row')
            if scenario == 'chat':
                for _ in range(4): key('Tab')
                filter_all = observe('filter-All-focused')
                key('Right'); filter_unread = observe('filter-Unread-focused-not-selected')
                check('filter-arrow-does-not-select', filter_unread, same(filter_all,filter_unread,'250x380+13+220'), 'Conversation list stays unchanged while arrow moves focus inside its single tab stop')
                key('Return'); expect('filter-Enter-selects-unread', ['Mara Fischer'], ['Teamküche','Notiz an mich'])
                key('Left'); key('space'); expect('filter-Space-restores-All', ['Teamküche','Notiz an mich'])
                key('Escape')
                for _ in range(7): key('Tab')
                focused = observe('B1-B2-keyboard-Mara')
                check('B1-B2-unselected-row-interior-stable', focused, same(baseline, focused, '240x44+19+392'), 'Mara row interior rect 240x44+19+392 is byte-identical before/after native Tab focus')
                check('B1-B2-focus-ring-visible', focused, not same(baseline, focused, '252x7+13+383'), 'Only exterior ring appears; title/background semantic contrasts are separately checked for all four themes')
                key('Escape'); move(100, 525)
                hovered = observe('B3-Jonas-hover', pointer=False)
                colors = {'selected': pixel(hovered, 160, 280), 'hover': pixel(hovered, 160, 546), 'unfocused': pixel(baseline, 160, 546)}
                check('B3-hover-distinct-from-selection', hovered, len(set(colors.values())) == 3, colors)
            key('Escape'); click(500, 589); type_text('ALPHAOMEGA'); key('Home')
            for _ in range(5): key('Right')
            composer = observe('B7-composer-container-focus')
            check('B7-composer-card-ring', composer, pixel(baseline, 500, 557) != pixel(composer, 500, 557), 'Native focused border belongs to composer card, not its text-only child')
            key('ctrl+comma'); expect('B8-shortcut-opens-settings', ['Darstellung'])
            key('Escape')
            restored = observe('B8-Escape-restores-composer-caret')
            check('B8-gear-not-falsely-focused', restored, same(baseline, restored, '42x42+219+637'), 'Gear pixels equal unfocused baseline after shortcut modal closes')
            type_text('X'); key('End')
            expect('B8-caret-position-retained', ['ALPHAXOMEGA'])
            if scenario == 'chat':
                key('Tab')
                disabled = expect('B9-unavailable-attachment-keyboard-reason', ['derzeit nicht verfügbar'])
                key('Return'); key('space')
                no_action = observe('B9-disabled-Enter-Space-no-dispatch')
                check('B9-disabled-action-does-not-dispatch', no_action, same(disabled, no_action, '1120x690+0+0'), 'Enter and Space leave native UI identical; no submission/result banner')
            click(500, 589); key('ctrl+a'); type_text('WOLKENENTWURF')
            before = observe('B10-before-send')
            key('Return')
            failed = expect('B10-busy-preserves-draft' if scenario == 'busy' else 'B10-failed-send-preserves-draft', ['WOLKENENTWURF', 'ausgelastet' if scenario == 'busy' else 'Nicht gesendet'])
            check('B10-list-and-timeline-dont-shift', failed, same(before, failed, '278x690+0+0') and same(before, failed, '820x440+290+105'), 'Entire sidebar and timeline below overlay are pixel-identical')
            click(1067, 45)
            expect('B10-feedback-dismissible-keeps-draft', ['WOLKENENTWURF'], ['Nicht gesendet', 'ausgelastet'])
        elif scenario == 'settings':
            key('Escape'); click(240, 658); observe('B12-gear-opens-modal')
            for _ in range(3): key('Tab')
            initial = observe('B12-selected-appearance-focused')
            # Dark-Sage moves to unselected Hell; Light-Blue moves to unselected System.
            key('Left'); moved = observe('B12-arrow-focus-without-selection')
            check('B12-appearance-interior-stable', moved, same(initial, moved, '160x90+585+150') and same(initial, moved, '160x90+783+150'), 'Two miniature/card interiors unchanged; border-only ring moves')
            key('Return'); activated = observe('B12-Enter-activates')
            key('Right'); key('space'); selected = observe('B12-Space-activates')
            check('B12-Enter-Space-change-retain-appearance', selected, pixel(initial,970,280)==pixel(selected,970,280), 'Enter changes selection; Space restores initial effective appearance')
            click(70,420); type_text('MODALCANARY'); key('ctrl+k'); type_text('BLOCKEDTEXT')
            blocked = expect('B12-modal-background-inert', ['Darstellung'], ['MODALCANARY','BLOCKEDTEXT'])
            key('Escape'); gear = expect('B12-Escape-returns-gear', ['Produktteam'], ['Darstellung'])
            key('Return'); expect('B12-restored-gear-Enter-opens', ['Darstellung'])
            key('Escape'); key('space'); expect('B12-restored-gear-Space-opens', ['Darstellung'])
            # Two nav stops, one appearance, one palette, one scale group.
            for _ in range(5): key('Tab')
            scale100 = observe('B4-B5-selected-100-focused')
            key('Right'); scale125 = observe('B4-B5-focus-125-not-selected')
            check('B4-B5-arrow-preserves-scale', scale125, same(scale100,scale125,'450x230+375+140') and same(scale100,scale125,'36x20+800+449'), 'Moving focus keeps layout/text scale and selection; focus ring alone moves within scale group')
            key('Return'); enlarged = observe('B4-B5-Enter-selects-125')
            check('B4-B5-Enter-changes-actual-text-scale', enlarged, not same(scale100,enlarged,'260x45+375+45'), 'Native heading geometry changes from 100% to 125%; selected 125 label visually inspected')
            key('Left'); key('space'); restored = observe('B4-B5-Space-restores-100')
            check('B4-B5-Space-restores-text-scale', restored, same(scale100,restored,'260x45+375+45'), 'Heading pixels return exactly to 100%')
            key('Tab'); density = observe('density-relaxed-focused')
            key('Left'); compact_focus = observe('density-compact-focus-only')
            check('density-arrow-does-not-select', compact_focus, same(density,compact_focus,'560x60+380+549'), 'Preview message including avatar does not change when arrow moves focus')
            key('Return'); compact = observe('density-Enter-compact')
            key('Right'); key('space'); relaxed = observe('density-Space-relaxed')
            check('density-Enter-Space-select', relaxed, not same(compact,relaxed,'560x60+380+549') and same(density,relaxed,'560x60+380+549'), 'Enter removes spacing/avatar; Space restores actual relaxed preview pixels')
        return checks
    finally:
        (out / f'{label}-d11-observations.json').write_text(json.dumps({'frames':frames,'checks':checks},indent=2)+'\n')
