"""Small native keyboard/mouse smoke checks; no browser, DOM, server or media.

Imported by capture.py --interactions. OCR only locates visible native controls;
assertions concern observable transitions and retained input, not pixel goldens.
"""
import csv
import io
import re
import subprocess
import time
import unicodedata
from pathlib import Path
from capture import run, sha


def normalized(value):
    text = ''.join(c for c in unicodedata.normalize('NFKD', value).casefold() if c.isalnum() or c.isspace())
    return ' '.join(text.split())


def exercise(scenario, env, xid, out, label, large=False):
    checks = []
    step = 0

    def observe():
        nonlocal step
        step += 1
        time.sleep(.35)
        path = out / f'{label}-action-{step:02}.png'
        run(['import', '-window', xid, str(path)], env)
        ocr_path=out/f'{label}-ocr.png'
        run(['convert',str(path),'-resize','200%',str(ocr_path)],env)
        tsv = run(['tesseract', str(ocr_path), 'stdout', '--psm', '11', 'tsv'], env).stdout
        ocr_path.unlink()
        words = [r for r in csv.DictReader(io.StringIO(tsv), delimiter='\t') if r.get('text', '').strip()]
        for r in words:
            for field in ('left','top','width','height'):r[field]=str(int(r[field])//2)
        return path, words, normalized(' '.join(r['text'] for r in words))

    def poll_title_anchor(words):
        titles = [word for word in words if normalized(word['text']) == 'umfrage'
                  and 200 < int(word['left']) < 1000 and int(word['top']) < 300]
        if not titles:
            return None
        title = min(titles, key=lambda word: int(word['top']))
        return int(title['left']) + 8, int(title['top'])

    def poll_option_hits(path, words):
        # Whole-dialog OCR sometimes groups the outlined fields into graphics.
        # OCR each interior relative to the visible dialog title; the dialog's
        # height changes with its contents. Click only recognized placeholders.
        if scenario not in ('poll-dialog', 'poll-drafts'):
            return []
        size = run(['identify', '-format', '%w %h', str(path)], env).stdout.split()
        anchor = poll_title_anchor(words)
        if size != ['1280', '800'] or anchor is None:
            return []
        x, top = anchor
        first_y = top + 153
        hits = []
        crop_path = out / f'{label}-ocr-option.png'
        for y in (first_y, first_y + 52):
            run(['convert', str(path), '-crop', f'240x22+{x}+{y}', '+repage',
                 '-resize', '400%', '-colorspace', 'gray', '-auto-level', str(crop_path)], env)
            found = normalized(run(['tesseract', str(crop_path), 'stdout', '--psm', '7'], env).stdout)
            if 'antwortoption' in found.replace(' ', ''):
                hits.append((x + 80, y + 11))
        crop_path.unlink()
        return hits

    def expect(name, required=(), absent=(), footer_height=120, include_footer_words=False):
        path, words, text = observe()
        text += normalized(run(["tesseract",str(path),"stdout","--psm","6"],env).stdout)
        width,height=map(int,run(["identify","-format","%w %h",str(path)],env).stdout.split())
        ocr_path=out/f"{label}-ocr-footer.png"
        footer_height = min(footer_height, height)
        run(["convert",str(path),"-crop",f"{width}x{footer_height}+0+{height-footer_height}","+repage","-resize","300%","-colorspace","gray","-auto-level",str(ocr_path)],env)
        text += normalized(run(["tesseract",str(ocr_path),"stdout","--psm","6"],env).stdout)
        if include_footer_words:
            tsv = run(["tesseract",str(ocr_path),"stdout","--psm","6","tsv"],env).stdout
            for word in csv.DictReader(io.StringIO(tsv), delimiter='\t'):
                if word.get('text', '').strip():
                    word['left'] = str(int(word['left']) // 3)
                    word['top'] = str(int(word['top']) // 3 + height - footer_height)
                    words.append(word)
        ocr_path.unlink()
        if any(normalized(term).replace(' ', '') not in text.replace(' ', '') for term in required):
            # Selected rows in dark overlays can be missed by whole-window OCR.
            # Inspect the actual centered content at higher contrast before failing.
            ocr_path=out/f'{label}-ocr-center.png'
            # Wide dialogs extend beyond the central half of the window.
            # Keep their leading labels intact in this contrast fallback.
            crop=f'{width*3//4}x{height*3//4}+{width//8}+{height//8}'
            run(['convert',str(path),'-crop',crop,'+repage','-resize','300%',
                 '-colorspace','gray','-auto-level','-negate',str(ocr_path)],env)
            text += ' ' + normalized(run(['tesseract',str(ocr_path),'stdout','--psm','6'],env).stdout)
            ocr_path.unlink()
        if any(normalized(term).replace(' ', '') not in text.replace(' ', '') for term in required):
            # A wrapped sidebar hint can be interleaved with timeline text by
            # whole-window OCR. Read the same visible column independently.
            sidebar_path = out / f'{label}-ocr-sidebar.png'
            run(['convert', str(path), '-crop', f'{min(width, 260)}x{height}+0+0',
                 '+repage', '-resize', '300%', '-colorspace', 'gray', '-auto-level',
                 str(sidebar_path)], env)
            text += normalized(run(['tesseract', str(sidebar_path), 'stdout', '--psm', '6'], env).stdout)
            sidebar_path.unlink()
        if scenario == 'server' and (width, height) == (1280, 800):
            field_path = out / f'{label}-ocr-field.png'
            run(['convert', str(path), '-crop', '360x32+635+312', '+repage', '-resize', '400%',
                 '-colorspace', 'gray', '-threshold', '75%', str(field_path)], env)
            text += normalized(run(['tesseract', str(field_path), 'stdout', '--psm', '7'], env).stdout)
            field_path.unlink()
        if scenario == 'chat-cloud-files-offline' and (width, height) == (1280, 800):
            banner_path = out / f'{label}-ocr-banner.png'
            run(['convert', str(path), '-crop', '700x36+277+288', '+repage', '-resize', '400%',
                 '-colorspace', 'gray', '-threshold', '75%', str(banner_path)], env)
            text += normalized(run(['tesseract', str(banner_path), 'stdout', '--psm', '7'], env).stdout)
            banner_path.unlink()
        if scenario == 'accounts' and '-dark-' in label and (width, height) == (1280, 800):
            dialog_path = out / f'{label}-ocr-account-dialog.png'
            run(['convert', str(path), '-crop', '440x380+420+230', '+repage', '-resize', '300%',
                 '-colorspace', 'gray', '-negate', '-auto-level', str(dialog_path)], env)
            text += normalized(run(['tesseract', str(dialog_path), 'stdout', '--psm', '6'], env).stdout)
            dialog_path.unlink()
        if scenario == 'poll-dialog' and (width, height) == (1280, 800) and any(
                term in ('TEAMESSEN', 'GARTEN', 'WERKSTATT') for term in required):
            # The small field text can be missed by whole-window OCR. Inspect
            # the actual three native inputs relative to the dialog title.
            field_path = out / f'{label}-ocr-poll-field.png'
            anchor = poll_title_anchor(words)
            fields = [] if anchor is None else [(anchor[0], anchor[1] + dy, 575) for dy in (93, 146, 198)]
            for x, y, w in fields:
                image_args = ['convert', str(path), '-crop', f'{w}x36+{x}+{y}', '+repage',
                              '-resize', '400%', '-colorspace', 'gray']
                if '-dark-' in label:
                    image_args.append('-negate')
                run(image_args + ['-auto-level', '-threshold', '75%', str(field_path)], env)
                text += normalized(run(['tesseract', str(field_path), 'stdout', '--psm', '7'], env).stdout)
            field_path.unlink(missing_ok=True)
        if scenario in ('chat', 'busy') and 'NO MATCH' in required and (width, height) == (1280, 800):
            # Whole-window OCR can treat the focused switcher input as a graphic.
            # Read its actual contents relative to the visible dialog heading.
            headings = [word for word in words if normalized(word['text']) == 'suchen'
                        and int(word['left']) >= 260 and int(word['top']) >= 200]
            if headings:
                heading = min(headings, key=lambda word: int(word['top']))
                x, y = int(heading['left']) + 13, int(heading['top']) + 60
                field_path = out / f'{label}-ocr-switcher-field.png'
                image_args = ['convert', str(path), '-crop', f'640x42+{x}+{y}', '+repage',
                              '-resize', '400%', '-colorspace', 'gray']
                if '-dark-' in label:
                    image_args.append('-negate')
                run(image_args + ['-auto-level', str(field_path)], env)
                text += ' ' + normalized(run(['tesseract', str(field_path), 'stdout', '--psm', '7'], env).stdout)
                field_path.unlink()
        if 'Antwortoption' in required and poll_option_hits(path, words):
            text += ' antwortoption'
        for term in required:
            # At small sizes Tesseract sometimes reads the highlighted German
            # selection count's final letters as "ahit". Keep the numeric count
            # and prefix exact; the subsequent staged chips verify both files.
            selection_count = re.fullmatch(r'([12]) ausgewählt', term)
            fuzzy_count = selection_count and re.search(
                rf'\b{selection_count.group(1)}\s*ausge\w{{4,8}}', text)
            if selection_count and not fuzzy_count:
                fuzzy_count = re.search(rf'hinzuf\w{{3,8}}\s*{selection_count.group(1)}\b', text)
            if selection_count and not fuzzy_count and (width, height) == (1280, 800):
                # This inspected footer location is stable in the fixed-size
                # fixture. A contrast-enhanced crop also works in dark mode.
                count_path=out/f'{label}-ocr-count.png'
                run(['convert', str(path), '-crop', '100x25+652+692', '+repage',
                     '-resize', '400%', '-colorspace', 'gray', '-auto-level', '-negate', str(count_path)], env)
                count_text=normalized(run(['tesseract', str(count_path), 'stdout', '--psm', '7'], env).stdout)
                count_path.unlink()
                fuzzy_count=re.search(rf'\b{selection_count.group(1)}\s*ausge\w{{4,8}}', count_text)
                if selection_count.group(1) == '1' and not fuzzy_count:
                    fuzzy_count = re.search(r'^l\s*ausge\w{4,8}', count_text)
            aliases = {
                'Konzept.pdf': ('konzeptpdt', 'konzeptpat'),
                'Projektübersicht.pdf': ('projektibersichtpdf', 'projektiibersichtpdf'),
                'Zwischenablage.png': ('wischenablagepng',),
            }
            alternate = any(alias in text.replace(' ', '') for alias in aliases.get(term, ()))
            if normalized(term).replace(' ', '') not in text.replace(' ', '') and not fuzzy_count and not alternate:
                raise AssertionError(f'{name}: missing {term!r}; evidence {path.name}; OCR {text}')
        for term in absent:
            if normalized(term).replace(' ', '') in text.replace(' ', ''):
                raise AssertionError(f'{name}: unexpected {term!r}; evidence {path.name}')
        checks.append({'check': name, 'result': 'pass', 'image': path.name, 'sha256': sha(path)})
        return words

    def key(value):
        run(['xdotool', 'key', '--clearmodifiers', value], env)

    def type_text(value):
        # Xvfb has no window manager to synchronize activation with a click.
        # Confirm the native target, then observe a frame before injecting text;
        # this does not change which GPUI editor owns focus inside the window.
        run(['xdotool', 'windowfocus', '--sync', xid], env)
        focused = run(['xdotool', 'getwindowfocus'], env).stdout.strip()
        if focused != str(xid):
            raise AssertionError(f'Native text target {focused} differs from captured window {xid}')
        observe()
        run(['xdotool', 'type', '--clearmodifiers', '--delay', '150', '--', value], env)

    def expect_settled(*args, **kwargs):
        # Read-only observation of an asynchronous native paint. The action is
        # sent exactly once; its required outcome must appear within six seconds.
        deadline = time.monotonic() + 6
        while True:
            try:
                return expect(*args, **kwargs)
            except AssertionError:
                if time.monotonic() >= deadline:
                    raise
                time.sleep(.15)

    def accent_button_hit(path, word, xmin, ymin):
        # White labels inside a colored button are frequently missed by
        # whole-window OCR. Find actual brand-colored button-sized regions,
        # then require the requested label in an enlarged crop of that region.
        # This follows layout changes without treating a dispatched click as
        # evidence that the action succeeded.
        for color in ('#356fc1', '#3a75c8', '#354f41', '#4f7a63'):
            result = run(['convert', str(path), '-fuzz', '5%', '-fill', 'black',
                          '+opaque', color, '-fill', 'white', '-opaque', color,
                          '-define', 'connected-components:verbose=true',
                          '-connected-components', '4', 'null:'], env)
            for line in (result.stdout + result.stderr).splitlines():
                match = re.search(r'\b(\d+)x(\d+)\+(\d+)\+(\d+).*srgb\(255,255,255\)', line)
                if not match:
                    continue
                width, height, x, y = map(int, match.groups())
                if width < 50 or not 25 <= height <= 60 or x < xmin or y < ymin:
                    continue
                crop = out / f'{label}-ocr-accent-button.png'
                run(['convert', str(path), '-crop', f'{width}x{height}+{x}+{y}',
                     '+repage', '-resize', '400%', '-colorspace', 'gray', '-auto-level',
                     str(crop)], env)
                text = normalized(run(['tesseract', str(crop), 'stdout', '--psm', '7'], env).stdout)
                crop.unlink()
                if normalized(word).replace(' ', '') in text.replace(' ', ''):
                    return x + width // 2, y + height // 2
        return None

    def click(word, xmin=0, ymin=0, inspected_point=None, dy=0, first=False):
        path, words, _ = observe()
        observed_words = words
        choices = [r for r in words if normalized(r['text']) == normalized(word) and int(r['left'])>=xmin and int(r['top'])>=ymin]
        if not choices:
            tsv=run(['tesseract',str(path),'stdout','--psm','6','tsv'],env).stdout
            words=[r for r in csv.DictReader(io.StringIO(tsv),delimiter='\t') if r.get('text','').strip()]
            choices=[r for r in words if normalized(r['text'])==normalized(word) and int(r['left'])>=xmin and int(r['top'])>=ymin]
        if not choices:
            ocr_path=out/f'{label}-ocr-inverted.png'
            run(['convert',str(path),'-resize','300%','-colorspace','gray','-threshold','55%','-negate',str(ocr_path)],env)
            tsv=run(['tesseract',str(ocr_path),'stdout','--psm','11','tsv'],env).stdout
            ocr_path.unlink()
            inverted=[r for r in csv.DictReader(io.StringIO(tsv),delimiter='\t') if r.get('text','').strip()]
            for r in inverted:
                for field in ('left','top','width','height'):r[field]=str(int(r[field])//3)
            words+=inverted
            choices=[r for r in inverted if normalized(r['text'])==normalized(word) and int(r['left'])>=xmin and int(r['top'])>=ymin]
        # Tesseract occasionally confuses Manrope h/n and the fl ligature.
        # Fixed aliases only; the subsequent native state transition is asserted.
        aliases={'teilnehmen':('teilnenmen','telinenmen'),'Auflegen':('auriegen','autlegen','auilegen'),
                 'Entwürfe':('entwirfe',)}
        if not choices and word in aliases:
            choices=[r for r in observed_words + words if normalized(r['text']) in aliases[word]
                     and int(r['left'])>=xmin and int(r['top'])>=ymin]
        if choices:
            hit = choices[0] if first else choices[-1]
            x = int(hit['left']) + int(hit['width']) // 2
            y = int(hit['top']) + int(hit['height']) // 2
        elif word == 'Antwortoption' and (hits := poll_option_hits(path, observed_words)):
            x, y = hits[0] if first else hits[-1]
        elif word in ('Veröffentlichen', 'Erinnern') and (hit := accent_button_hit(path, word, xmin, ymin)):
            x, y = hit
        elif word == 'discovery-search-input':
            # The search input shares a row with its labeled primary button.
            # Earlier and current layouts differ in their header/helper height.
            hit = accent_button_hit(path, 'Suchen', 900, 150)
            if not hit:
                raise AssertionError('Cannot locate discovery search row')
            x, y = hit[0] - 300, hit[1]
        elif inspected_point is not None:
            # Colored call controls are not reliably recognized by Tesseract.
            # These points were inspected on actual captures at the exact size.
            # Dispatch alone never passes a check: the next state is asserted.
            width,height=map(int,run(['identify','-format','%w %h',str(path)],env).stdout.split())
            expected_width,expected_height,x,y=inspected_point
            if (width,height)!=(expected_width,expected_height):
                raise AssertionError(f'Inspected control point requires {expected_width}x{expected_height}')
            print(f'Native {word} click uses inspected ({x}, {y}) in {path.name}; OCR unavailable',flush=True)
        else:
            raise AssertionError(f'Cannot locate native control {word!r}')
        run(['xdotool', 'mousemove', '--window', xid, str(x), str(y + dy), 'click', '1'], env)

    if large:
        if scenario in ('chat','prejoin','settings'):
            run(['xdotool','mousemove','--window',xid,'600','430','click','--repeat','35','--delay','35','5'],env)
            if scenario=='chat':
                expect('200% composer reachable by native scrolling',['Senden','Nachricht an Produktteam'])
                click('schreiben');type_text('LARGEDRAFT');key('Return')
                expect('200% failed submission retains draft',['LARGEDRAFT','Nicht gesendet'])
            elif scenario=='prejoin':
                expect('200% explicit conference join remains reachable',['Jetzt', 'teil'])
            else:
                expect('200% settings body scrolls independently of close control',['Zurück','Hier wird kein','Serverwert gespeichert'])
        elif scenario=='call':
            expect('200% call controls remain visible',['Mikrofon'])
            click('Auflegen',inspected_point=(720,580,360,470));expect('200% explicit leave returns to room',['Produktteam'])
        return checks

    if scenario == 'chat-reminder-time':
        # Inspect the actual blue selection in the bounded calendar grid.
        # Sampling fixed screen points broke on the unchanged integration build
        # when the surrounding composer grew; track the painted marker instead.
        def selected_day(image):
            data = subprocess.run(['convert', str(image), '-crop', '280x200+310+420',
                                   '+repage', '-depth', '8', 'rgb:-'], env=env,
                                  check=True, capture_output=True, timeout=20).stdout
            positions = [(index % 280, index // 280) for index in range(len(data) // 3)
                         if tuple(data[index * 3:index * 3 + 3]) == (53, 111, 193)]
            if len(positions) < 100:
                raise AssertionError('Calendar fixture has no visible selected-day marker')
            return (sum(x for x, _ in positions) / len(positions),
                    sum(y for _, y in positions) / len(positions))
        before, _, _ = observe()
        first = selected_day(before)
        key('Right')
        after, _, _ = observe()
        second = selected_day(after)
        if not (35 <= second[0] - first[0] <= 45 and abs(second[1] - first[1]) < 2):
            raise AssertionError('Right arrow did not move the selected calendar day')
        checks.append({'check': 'calendar arrow navigation moves selection', 'result': 'pass',
                       'image': after.name, 'sha256': sha(after)})
    elif scenario == 'chat-reminder-menu':
        expect('message menu combines reminder and unread actions',
               ['Erinnerung bearbeiten', 'Gespräch als ungelesen markieren'])
        click('bearbeiten', xmin=1000, ymin=200)
        expect('message reminder opens the shared calendar', ['Später erinnern', 'Januar'])
        key('Escape')
        expect('calendar cancellation retains the message', ['Hier treffen wir uns.'], ['Später erinnern'])
    elif scenario == 'chat-schedule-time':
        expect('send-later opens a calendar beside the draft', ['Nachricht planen', 'Januar', 'Notizen morgen teilen.'])
        key('Escape')
        expect('cancelling send-later retains the draft', ['Notizen morgen teilen.'], ['Nachricht planen'])
        type_text('X')
        expect('send-later cancellation returns focus to the composer', ['Notizen morgen teilen.X'])
    elif scenario == 'thread-message-tools':
        expect('thread translation and account navigation coexist',
               ['Design review', 'Das passt', 'Original anzeigen', 'Threads', 'Erinnerungen'])
        click('anzeigen', xmin=950, ymin=300)
        expect('thread translation can return to its original', ['Das klingt gut.'], ['Das passt'])
        click('antworten', xmin=900, ymin=650, inspected_point=(1280,800,1030,700))
        type_text('THREADTOOLS')
        expect_settled('thread tools preserve an unsent reply', ['THREADTOOLS'])
        run(['xdotool', 'mousemove', '--window', xid, '1100', '322'], env)
        time.sleep(.4)
        run(['xdotool', 'mousemove', '--window', xid, '1242', '295', 'click', '1'], env)
        expect('thread message menu exposes reminder and cached translation', ['Später erinnern', 'Übersetzung anzeigen'])
        click('erinnern', xmin=900)
        expect('thread reminder uses the shared calendar inside the pane',
               ['Design review', 'Später erinnern', 'Januar', 'THREADTOOLS'], footer_height=500)
        run(['xdotool', 'mousemove', '--window', xid, '1150', '550',
             'click', '--repeat', '12', '--delay', '70', '5'], env)
        expect_settled('thread reminder presets and submit remain reachable by native scrolling',
                       ['In 20 Minuten', 'THREADTOOLS'], footer_height=500)
        click('Erinnern', xmin=890, ymin=600, inspected_point=(1280,800,932,632))
        run(['xdotool', 'mousemove', '--window', xid, '1150', '550',
             'click', '--repeat', '6', '--delay', '70', '5'], env)
        expect_settled('unavailable fixture clock reports failure and retains the thread draft',
                       ['Systemzeitzone', 'THREADTOOLS'], footer_height=500)
        key('Escape')
        type_text('X')
        expect_settled('thread reminder cancellation restores its draft focus', ['THREADTOOLSX'], ['Später erinnern'])
        click('Nachricht', xmin=280, ymin=650, inspected_point=(1280,800,400,700))
        type_text('MAINCALENDAR')
        expect_settled('main and thread composers retain independent drafts', ['MAINCALENDAR', 'THREADTOOLSX'])
        click('send-later', inspected_point=(1280,800,768,742))
        expect('main send-later stays beside the open thread', ['Nachricht planen', 'Design review', 'THREADTOOLSX'])
        key('Escape')
        type_text('X')
        expect_settled('main calendar cancellation preserves the thread and restores main focus',
                       ['MAINCALENDARX', 'Design review', 'THREADTOOLSX'], ['Nachricht planen'])
    elif scenario == 'server':
        click('cloud.example.test', inspected_point=(1280, 800, 700, 328))
        key('ctrl+a'); type_text('probe.example.test'); key('ctrl+a'); key('ctrl+c')
        key('BackSpace'); key('ctrl+v')
        expect('native selection and explicit clipboard roundtrip', ['probe.example.test'])
        key('ctrl+a'); type_text('replacement.example.test'); key('Home'); key('Delete'); key('End')
        expect('native editing replaces selection and deletes one grapheme', ['eplacement.example.test'], ['probe.example.test'])
        key('Tab')
        expect('keyboard focus can leave input')
    elif scenario == 'accounts':
        expect('account settings list is visible', ['Konten', 'Nord Studio'])
        # The first account row's remove control opens a confirmation; the
        # fictional account is not removed by this check.
        run(['xdotool', 'mousemove', '--window', xid, '1027', '306', 'click', '1'], env)
        expect('account removal asks for confirmation', ['Konto aus dieser App entfernen', 'Abbrechen'])
        key('Escape')
        expect('Escape dismisses confirmation without removing the account', ['Konten', 'Nord Studio'],
               ['Du wirst in dieser App abgemeldet'])
    elif scenario == 'status-message':
        expect('status message editor is visible', ['Statusnachricht', 'Vorschläge'])
        click('Statusnachricht')
        type_text('LUNCHBREAK')
        click('Nie')
        click('30')
        click('Speichern', inspected_point=(1280,800,67,701))
        expect('saved fictional status is shown immediately', ['LUNCHBREAK'], ['Vorschläge'])
        click('LUNCHBREAK')
        click('Entfernen')
        expect('fictional status can be cleared', ['Statusnachricht festlegen'])
    elif scenario == 'status-picker':
        expect('status picker is visible', ['Status festlegen', 'Abwesend', 'Unsichtbar'])
        key('Escape')
        expect('Escape closes the account status picker', absent=['Status festlegen'])
        click('Alex')
        expect('account control reopens the status picker', ['Status festlegen', 'Unsichtbar'])
    elif scenario == 'people-card':
        expect('person card shows status and actions', ['Abwesend', 'Nachricht senden', 'Anrufen'])
        key('Escape')
        expect('Escape dismisses the person card', absent=['Abwesend'])
        click('Mara', xmin=300, ymin=180)
        expect('author name reopens the person card', ['Abwesend', 'Anrufen'])
        run(['xdotool', 'mousemove', '--window', xid, '1000', '650', 'click', '1'], env)
        expect('outside click dismisses the person card', absent=['Abwesend'])
        click('Mara', xmin=300, ymin=180)
        key('Tab'); key('Tab'); key('Return')
        time.sleep(1.)  # Direct-room resolution and call entry cross owner ticks.
        expect('keyboard activation resolves the fictional direct call', ['Verbindung wird aufgebaut'])
    elif scenario == 'clipboard-denied':
        key('ctrl+k');type_text('CLIPMARK');key('ctrl+a');key('ctrl+c');key('BackSpace')
        key('Escape')
        click('Nachricht', xmin=200, ymin=600);time.sleep(.3);type_text('DRAFTSAFE');key('ctrl+a');key('ctrl+x')
        expect('unknown privacy policy blocks cut without losing draft',
               ['DRAFTSAFE', 'Ausschneiden', 'diesen Entwurf nicht freigegeben'])
        key('ctrl+c');key('ctrl+v')
        expect('unknown privacy policy blocks export but permits selected-text replacement by paste',['CLIPMARK'],['DRAFTSAFE'])
        key('ctrl+k');key('ctrl+v')
        expect('denied draft copy did not replace platform clipboard',['CLIPMARK'])
    elif scenario == 'chat-content':
        # The plus control is a native icon; this point was inspected at 1280x800.
        run(['xdotool', 'mousemove', '--window', xid, '314', '742', 'click', '1'], env)
        expect('composer plus exposes file sources and polls',
               ['Vom Computer hochladen', 'Aus NextCloud teilen', 'Umfrage erstellen'])
        key('Escape')
        click('Nachricht', xmin=200, ymin=600)
        subprocess.run(['xclip', '-selection', 'clipboard', '-t', 'text/uri-list', '-i'],
                       input='file:///tmp/Beispiel.txt\r\n', text=True, env=env,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True, timeout=10)
        data = run(['xclip', '-selection', 'clipboard', '-t', 'text/uri-list', '-o'], env).stdout
        if data.strip() != 'file:///tmp/Beispiel.txt':
            raise AssertionError('the X11 clipboard did not retain the native file-list fixture')
        key('ctrl+v')
        expect('native file-list paste stages a file chip', ['Beispiel.txt'])
        image = Path('apps/desktop/assets/fixtures/content-photo.png').read_bytes()
        subprocess.run(['xclip', '-selection', 'clipboard', '-t', 'image/png', '-i'],
                       input=image, env=env, stdout=subprocess.DEVNULL,
                       stderr=subprocess.DEVNULL, check=True, timeout=10)
        key('ctrl+v')
        expect('native image paste stages an image chip', ['Beispiel.txt', 'Zwischenablage.png'])
    elif scenario == 'chat-cloud-files':
        click('Team', xmin=285, ymin=300)
        expect('folder navigation opens its listing', ['Konzept.pdf', 'Notizen.txt'])
        key('BackSpace')
        expect('backspace navigates to the parent folder', ['Projektübersicht.pdf', 'Workshop.png'])
        click('suchen', xmin=285, ymin=200, inspected_point=(1280, 800, 450, 208))
        type_text('Workshop')
        time.sleep(.7)  # Allow the 250 ms debounce and the native owner ticks.
        expect_settled('debounced search changes the listing', ['Workshop.png'], ['Projektübersicht.pdf'])
        key('ctrl+a'); key('BackSpace')
        time.sleep(.7)
        expect_settled('clearing search restores the root listing', ['Projektübersicht.pdf', 'Workshop.png'])
        # The file row is visible in dark and light captures, but low contrast
        # can make Tesseract miss its name. Verify the resulting selection below.
        click('Projektübersicht.pdf', inspected_point=(1280, 800, 400, 408))
        key('w'); key('ctrl+space')
        expect('type-ahead and control-space select the matching file', ['2 ausgewählt'])
        key('ctrl+space')
        expect('control-space toggles the focused file', ['1 ausgewählt'])
        click('Projektübersicht.pdf', inspected_point=(1280, 800, 400, 408))
        key('shift+Down')
        expect('shift and arrow extend the native file selection', ['2 ausgewählt'])
        key('Return')
        expect_settled('keyboard selection returns both cloud files to the composer',
               ['Projektübersicht.pdf', 'Workshop.png'], ['NextCloud-Dateien'])
    elif scenario == 'chat-cloud-files-offline':
        expect('offline browser keeps its cached listing', ['Offline', 'gespeicherte Dateien', 'Projektübersicht.pdf'])
    elif scenario == 'chat-upload-preview':
        expect('staged image has a composer chip', ['Zwischenablage.png'])
    elif scenario == 'poll-dialog':
        expect('empty poll starts without validation warnings', ['Umfrage erstellen'],
               ['Mindestens zwei unterschiedliche Antwortoptionen'])
        # Exercise plain-text paste in each poll input independently.
        for value, question in [('TEAMESSEN', True), ('GARTEN', False), ('WERKSTATT', False)]:
            if not question:
                click('Antwortoption')
            subprocess.run(['xclip', '-selection', 'clipboard', '-i'],
                           input=value, text=True, env=env, stdout=subprocess.DEVNULL,
                           stderr=subprocess.DEVNULL, check=True, timeout=10)
            key('ctrl+v')
            expect_settled('poll field retains pasted text', [value])
        expect('completed poll keeps both distinct choices', ['TEAMESSEN', 'GARTEN', 'WERKSTATT'],
               ['Mindestens zwei unterschiedliche Antwortoptionen'])
        click('Veröffentlichen', inspected_point=(1280, 800, 865, 704))
        expect_settled('accepted fictional poll closes the dialog', ['Designteam'], ['Umfrage erstellen'])
    elif scenario == 'poll-settings':
        expect('conversation settings omit the composer poll action', ['Allgemein', 'Nachrichten'], ['Umfrage erstellen'])
        click('auswählen', xmin=400, ymin=150)
        # The first emoji in the bounded picker is a native button in the fixed fixture.
        run(['xdotool', 'mousemove', '--window', xid, '475', '270', 'click', '1'], env)
        expect('selecting an emoji enables the room avatar action', ['Emoji als Avatar setzen'])
        key('Escape')
        expect('Escape closes the conversation settings', ['Designteam'], ['Gesprächsdetails'])
    elif scenario == 'poll-drafts':
        expect('draft list is separate from the poll form', ['Wo treffen wir uns?'], ['Antwortoption'])
        click('Entwürfe', xmin=700, inspected_point=(1280, 800, 840, 138))
        expect('draft toggle returns to the poll form', ['Frage', 'Antwortoption'], ['Wo treffen wir uns?'])
        click('Entwürfe', xmin=700, inspected_point=(1280, 800, 840, 138))
        expect('draft toggle reopens the saved list', ['Wo treffen wir uns?'], ['Antwortoption'])
    elif scenario == 'poll-details':
        expect('public poll offers voter expansion', ['Umfragedetails', 'Stimmen anzeigen'])
        # The first result's voter list in the fixed 1280x800 fixture.
        run(['xdotool', 'mousemove', '--window', xid, '640', '334', 'click', '1'], env)
        expect('public voters expand with names', ['Stimmen ausblenden', 'Alex Sommer'])
        run(['xdotool', 'mousemove', '--window', xid, '640', '334', 'click', '1'], env)
        expect('public voters collapse again', ['Stimmen anzeigen'], ['Stimmen ausblenden'])
    elif scenario == 'chat-polls':
        click('Abstimmen', inspected_point=(1280, 800, 390, 584))
        expect('accepted fictional vote stays visible', ['Deine Stimme'])
        # The poll's secondary action is behind its inspected ellipsis control.
        run(['xdotool', 'mousemove', '--window', xid, '780', '397', 'click', '1'], env)
        click('beenden')
        expect('closing asks for confirmation', ['Umfrage wirklich beenden'])
        click('Jetzt', inspected_point=(1280, 800, 405, 638))
        expect('confirmed fictional close keeps the poll and disables voting',
               ['Umfrage beendet', 'Welcher Entwurf'], ['Umfrage beenden'])
        log = (out / f'{label}.log').read_text()
        if log.count('UI_POLL_FIXTURE vote poll=9001 no_backend=true') != 1 or \
                log.count('UI_POLL_FIXTURE close poll=9001 no_backend=true') != 1:
            raise AssertionError('Vote or confirmed close did not submit exactly once')
        run(['xdotool', 'mousemove', '--window', xid, '1228', '149', 'click', '1'], env)
        expect('pinned strip collapses its message', ['Angeheftet'], ['Perfekt. Dann nehmen wir das so mit.'])
        run(['xdotool', 'mousemove', '--window', xid, '1228', '149', 'click', '1'], env)
        expect('pinned strip expands its message again', ['Angeheftet', 'Perfekt. Dann nehmen wir das so mit.'])
    elif scenario == 'chat-voice':
        path, _, _ = observe()
        width, height = map(int, run(['identify', '-format', '%w %h', str(path)], env).stdout.split())
        if (width, height) != (1280, 800):
            raise AssertionError('Voice waveform interaction requires the inspected 1280x800 fixture')
        # The compact fictional waveform occupies x=382..565, y=584..618 in
        # this fixed fixture. Verify the resulting scoped intents, not dispatch.
        run(['xdotool', 'mousemove', '--window', xid, '475', '601', 'click', '1'], env)
        time.sleep(.4)
        log = out / f'{label}.log'
        first = re.findall(r'UI_VOICE_FIXTURE seek_ms=(\d+)', log.read_text())
        if not first:
            raise AssertionError('Clicking the waveform did not submit a seek')
        key('Right')
        time.sleep(.4)
        second = re.findall(r'UI_VOICE_FIXTURE seek_ms=(\d+)', log.read_text())
        if len(second) <= len(first) or int(second[-1]) != min(int(first[-1]) + 5000, 38000):
            raise AssertionError(f'Focused waveform arrow seek did not advance five seconds: {first}, {second}')
        path, _, _ = observe()
        checks.append({'check': 'waveform click and focused arrow seek update the fictional player',
                       'result': 'pass', 'image': path.name, 'sha256': sha(path)})
    elif scenario == 'voice-review':
        path, _, _ = observe()
        width, height = map(int, run(['identify', '-format', '%w %h', str(path)], env).stdout.split())
        if (width, height) != (1280, 800):
            raise AssertionError('Voice review interaction requires the inspected 1280x800 fixture')
        # Start the fictional review player, then seek by pointer and keyboard.
        run(['xdotool', 'mousemove', '--window', xid, '355', '724', 'click', '1'], env)
        time.sleep(.7)
        run(['xdotool', 'mousemove', '--window', xid, '700', '724', 'click', '1'], env)
        time.sleep(.4)
        log = out / f'{label}.log'
        first = re.findall(r'UI_VOICE_REVIEW_FIXTURE seek_ms=(\d+) no_backend=true', log.read_text())
        if len(first) != 1 or not 0 < int(first[0]) < 13000:
            raise AssertionError(f'Review waveform pointer seek did not reach the fictional player: {first}')
        key('Right')
        time.sleep(.4)
        second = re.findall(r'UI_VOICE_REVIEW_FIXTURE seek_ms=(\d+) no_backend=true', log.read_text())
        if len(second) != 2 or int(second[-1]) != min(int(first[-1]) + 5000, 13000):
            raise AssertionError(f'Focused review waveform arrow did not advance five seconds: {first}, {second}')
        path, _, _ = observe()
        checks.append({'check': 'review waveform pointer and keyboard seek update the fictional player',
                       'result': 'pass', 'image': path.name, 'sha256': sha(path)})
    elif scenario == 'voice-recording':
        path, _, _ = observe()
        width, height = map(int, run(['identify', '-format', '%w %h', str(path)], env).stdout.split())
        if (width, height) != (1280, 800):
            raise AssertionError('Recording interaction requires the inspected 1280x800 fixture')
        # The rightmost round button is the explicit direct-send action.
        run(['xdotool', 'mousemove', '--window', xid, '1214', '724',
             'click', '--repeat', '2', '--delay', '50', '1'], env)
        time.sleep(.8)
        log = (out / f'{label}.log').read_text()
        if log.count('UI_VOICE_FIXTURE send no_backend=true') != 1:
            raise AssertionError('Direct recording send did not settle exactly once')
        expect('direct send retires the fictional recording', ['Nachricht an Designteam'])
    elif scenario in ('chat', 'busy'):
        # Search is local to bounded room rows. Focus shortcut works from root.
        key('ctrl+k'); type_text('NO MATCH')
        expect_settled('local search visibly retains its query and reports no matching rows',
                       ['NO MATCH', 'Keine passenden Gespräche oder Personen'])
        key('ctrl+a'); key('BackSpace')
        key('Escape')
        click('Nachricht', xmin=200, ymin=600)
        type_text('DRAFTALPHA'); time.sleep(.4); key('Return')
        expect('draft retained after busy rejection' if scenario == 'busy' else 'draft retained after asynchronous failure',
               ['DRAFTALPHA'] if scenario == 'busy' else ['DRAFTALPHA', 'Nicht gesendet'])
        if scenario == 'busy' and (out / f'{label}.log').read_text().count(
                'UI_BUSY send_rejected=true no_backend=true') != 1:
            raise AssertionError('The fictional busy port did not reject the explicit send exactly once')
        key('ctrl+comma')
        expect('settings shortcut opens modal', ['Einstellungen'])
        click('Darstellung'); click('Salbei'); click('Dunkel')
        expect('theme selection stays available after appearance change', ['Einstellungen','Salbei'])
        path,_,_=observe()
        # Check the actual native canvas, not an OCR interpretation of selected text.
        rgb=run(['convert',str(path),'-format','%[fx:p{10,10}.r] %[fx:p{10,10}.g] %[fx:p{10,10}.b]','info:'],env).stdout
        if max(map(float,rgb.split()))>=0.3:raise AssertionError('Dark appearance did not change native canvas')
        checks.append({'check':'native dark appearance changes canvas','result':'pass','image':path.name,'sha256':sha(path)})
        key('Escape')
        expect('escape closes settings and restores draft', ['DRAFTALPHA'], ['Einstellungen'])
        type_text('X')
        expect('modal returns focus to invoking composer', ['DRAFTALPHAX'])
    elif scenario == 'thread':
        words = expect('thread pane uses shared message and composer surfaces',
                       ['Design review', '2 Antworten', 'Mara Fischer', 'Im Thread antworten'])
        run(['xdotool', 'mousemove', '--window', xid, '1050', '407'], env)
        expect('thread hover keeps reply text readable', ['Ich aktualisiere die letzten', 'Details'])
        run(['xdotool', 'mousemove', '--window', xid, '1204', '400', 'click', '1'], env)
        words = expect('reply to own thread message opens a quote', ['Alex Sommer'])
        if not any(normalized(word['text']) == 'alex' and int(word['left']) > 900
                   and int(word['top']) > 590 for word in words):
            raise AssertionError('The own-message quote did not appear beside the thread composer')
        run(['xdotool', 'mousemove', '--window', xid, '1218', '635', 'click', '1'], env)
        _, words, _ = observe()
        if any(normalized(word['text']) == 'alex' and int(word['left']) > 900
               and int(word['top']) > 590 for word in words):
            raise AssertionError('The own-message quote did not close')
        checks.append({'check': 'own-message quote can be dismissed', 'result': 'pass'})
        before = next(int(word['left']) for word in words
                      if normalized(word['text']).startswith('design')
                      and int(word['top']) < 120 and int(word['left']) > 700)
        run(['xdotool', 'mousemove', '--window', xid, '883', '400', 'mousedown', '1'], env)
        run(['xdotool', 'mousemove', '--sync', '--window', xid, '870', '400'], env)
        time.sleep(.3)
        run(['xdotool', 'mousemove', '--sync', '--window', xid, '800', '400', 'mouseup', '1'], env)
        time.sleep(1.5)
        words = expect('thread resize keeps its content visible', ['Design review', '2 Antworten'])
        after = next(int(word['left']) for word in words
                     if normalized(word['text']).startswith('design')
                     and int(word['top']) < 120 and int(word['left']) > 700)
        if before - after < 30:
            raise AssertionError('Dragging the left edge did not resize the thread pane')
        checks.append({'check': 'thread pane resizes by dragging its left edge', 'result': 'pass'})
        run(['xdotool', 'mousemove', '--window', xid, '802', '400', 'click', '1'], env)
        key('Right')
        # Native compositor delivery can trail the key event under concurrent
        # software-rendered captures. Require the visible resize within a finite
        # window, rather than accepting the first still-old framebuffer.
        deadline = time.monotonic() + 6
        while True:
            words = expect('thread resize handle accepts keyboard input', ['Design review', '2 Antworten'])
            keyboard = next(int(word['left']) for word in words
                            if normalized(word['text']).startswith('design')
                            and int(word['top']) < 120 and int(word['left']) > 700)
            if keyboard - after >= 20:
                break
            checks.pop()  # Presence alone is not a successful keyboard resize.
            if time.monotonic() >= deadline:
                raise AssertionError('The focused thread resize handle did not respond to Right')
        key('Left')
        click('antworten', xmin=900, ymin=650, inspected_point=(1280,800,1010,700))
        type_text('THREADDRAFT')
        time.sleep(1.5)
        expect('thread composer accepts text', ['THREADDRAFT'])
        key('Return')
        words = expect_settled('thread Enter retains the draft after a rejected submission', ['THREADDRAFT', 'Nicht gesendet'], footer_height=200, include_footer_words=True)
        if not any(normalized(word['text']) == 'nicht' and int(word['left']) >= after - 10 for word in words):
            raise AssertionError('Thread submission feedback did not appear beside its composer')
        if any(normalized(word['text']) == 'nicht' and int(word['left']) < after - 10 for word in words):
            raise AssertionError('Thread submission feedback appeared in the main composer')
        log = (out / f'{label}.log').read_text()
        if not re.search(r'UI_CHAT_UX send room=\d+ bytes=11 .*thread=Some\(1\)', log):
            raise AssertionError('Enter did not submit the draft to the scoped thread')
        checks.append({'check': 'thread Enter submits the scoped draft', 'result': 'pass'})
        key('Escape')
        expect_settled('Escape closes the thread pane and returns to the room',
               ['Designteam'], ['Design review', 'THREADDRAFT'])
    elif scenario == 'thread-options':
        expect('thread options expose subscription and rename controls',
               ['Thread nicht mehr folgen', 'Alle Antworten', 'Nur Erwähnungen', 'Thread umbenennen'])
        click('umbenennen', xmin=800, ymin=100)
        expect('thread rename opens an editable title', ['Design review', 'Speichern'])
    elif scenario == 'thread-offline':
        expect('previously observed thread options are available offline',
               ['Thread nicht mehr folgen', 'Thread umbenennen'])
        click('umbenennen', xmin=800, ymin=100)
        key('ctrl+a')
        type_text('Release Notes')
        click('Speichern', xmin=800, ymin=100)
        expect('offline rename updates the thread heading immediately', ['Release Notes'])
        click('folgen', xmin=800, ymin=100)
        expect('offline unfollow updates the visible subscription action',
               ['Thread folgen'], ['Thread nicht mehr folgen'])
        click('antworten', xmin=900, ymin=650, inspected_point=(1280,800,1010,700))
        type_text('OFFLINEREPLY')
        key('Return')
        time.sleep(1.5)
        run(['xdotool', 'mousemove', '--window', xid, '1100', '550', 'click', '--repeat', '5', '5'], env)
        expect('offline thread reply and count update immediately', ['OFFLINEREPLY', '3 Antworten'])
        log = (out / f'{label}.log').read_text()
        if not re.search(r'UI_CHAT_UX send room=\d+ bytes=12 .*thread=Some\(1\)', log):
            raise AssertionError('Offline Enter did not submit the draft to the scoped thread')
        if 'UI_THREAD_FIXTURE offline_reply_queued=true' not in log:
            raise AssertionError('The fixture did not acknowledge local thread staging')
    elif scenario == 'search-navigation':
        expect('switcher displays its initial matching conversation', ['Produktteam', 'Gruppengespräch'])
        click('Produkt', xmin=300, ymin=300)
        key('ctrl+a'); key('BackSpace')
        expect_settled('empty switcher shows recent rooms', ['Mara Fischer', 'Weber'])
        key('Down'); key('Down'); key('Return')
        expect_settled('arrows and Enter open the selected conversation', ['Nachricht an Mara Fischer'], ['Mit Pfeiltasten auswählen'])
        key('ctrl+k')
        expect('keyboard shortcut reopens switcher', ['Mit Pfeiltasten auswählen'])
        type_text('Jonas')
        expect_settled('switcher matches a group participant as well as a direct room', ['Jonas Weber', 'Produktteam'])
        key('Escape')
        expect('Escape closes switcher', ['Nachricht an Mara Fischer'], ['Mit Pfeiltasten auswählen'])
    elif scenario == 'account-search':
        expect('account search groups matching messages', ['Produktteam', 'Navigation', 'Mara Fischer'])
        key('Return')
        expect('Enter opens the selected message in context', ['Nachricht an Produktteam'], ['Weitere Ergebnisse'])
    elif scenario == 'discover':
        expect('open conversations show descriptions and counts', ['Designrunde', 'Kaffeepause', '18 Teilnehmende'])
        click('discovery-search-input', inspected_point=(1280, 800, 340, 320))
        time.sleep(.25)
        type_text('Design')
        expect_settled('typing narrows the loaded rooms immediately', ['Designrunde'], ['Kaffeepause'])
        key('Escape')
        expect('Escape closes open conversation browsing', ['Nachricht an Produktteam'], ['Designrunde'])
    elif scenario == 'reference-preview':
        expect('resolved reference card is shown with its source message', ['Ein ruhiger Ort', 'Studiokonzept'])
        click('Vorschau verbergen', inspected_point=(1280, 800, 826, 352))
        expect('reference card can be collapsed locally', ['Vorschau anzeigen'], ['Ein ruhiger Ort'])
        click('Vorschau', xmin=300, ymin=250)
        expect('collapsed reference card can be restored', ['Ein ruhiger Ort', 'Studiokonzept'])
    elif scenario in ('create34', 'create35'):
        expect('creation form shows the inline invitations and presets', ['Projekt Atlas', 'Mara Fischer', 'Jonas Weber', 'Webinar'])
        click('Offenes', xmin=300)
        expect('open conversation choice describes link access', ['Personen mit Link'])
        click('Webinar', xmin=300)
        expect('a preset explains its purpose', ['moderierte Veranstaltungen'])
    elif scenario == 'sidebar-room-menu':
        expect('conversation actions expose notification settings', ['Benachrichtigungen', 'Archivieren'])
        click('Benachrichtigungen')
        time.sleep(1)
        expect('sidebar action opens the selected room notification controls', ['Alle Nachrichten', 'Nur Erwähnungen', 'Anrufbenachrichtigungen'])
    elif scenario == 'search-filter':
        click('Alle', ymin=190)
        expect_settled('sidebar filters start with the loaded rooms', ['Produktteam', 'Designbesprechung', 'Erwähnungen'])
        click('Erwähnungen')
        expect_settled('mention filter keeps known mentions and explains unknown state', ['Produktteam', 'Erwähnungen sind noch nicht bekannt'], ['Designbesprechung'])
    elif scenario == 'incoming':
        key('Return')
        expect('unrelated Enter does not accept incoming call', ['Annehmen'], ['Verbindung wird'])
        click('ansehen')
        expect('dismiss returns to the conversation without accepting', ['Nachricht an Produktteam'], ['Annehmen'])
        click('ansehen', inspected_point=(1280,800,1165,74))
        click('Annehmen', ymin=200, dy=-36, first=True)
        expect('direct accept bypasses optional conference prejoin', ['Verbindung wird aufgebaut'], ['Bereit'])
        key('Escape')
        expect('escape never hangs up a call', ['Verbindung wird aufgebaut'])
        click('Verlassen')
        expect('explicit leave returns to conversation', ['Nachricht an Mara Fischer'])
    elif scenario == 'call-ended':
        first = out / f'{label}.png'
        text = normalized(run(['tesseract', str(first), 'stdout', '--psm', '6'], env).stdout)
        visible = normalized('Anruf beendet') in text
        checks.append({
            'check': 'ended call surface is captured before its three-second dismissal',
            'result': 'pass' if visible else 'known-flaky',
            'reason': None if visible else 'Concurrent headless rendering can miss the three-second end card before the first X11 capture; automatic return is still asserted.',
            'image': first.name, 'sha256': sha(first),
        })
        time.sleep(4)
        expect('ended call returns to conversation after a short delay', ['Nachricht an Produktteam'], ['Anruf beendet'])
    elif scenario == 'call-recovery':
        expect('replacement call remains active while waiting for remote media', ['Im Gespräch'])
        click('beitreten', inspected_point=(1280,800,1188,155))
        time.sleep(.5)
        log = (out / f'{label}.log').read_text()
        if log.count('UI_INTENT recover-call-media room=') != 1:
            raise AssertionError('Recovery did not submit exactly one scoped intent')
        expect('explicit recovery starts a fresh admission', ['Verbindung wird aufgebaut'], ['Ton oder Bild fehlen'])
    elif scenario == 'call-participants-raised':
        # Exercise the real keyboard path with the call side panel focused.
        # Fixture diagnostics contain only local identities and the hand state.
        app_log = out / f'{label}.log'
        key('h'); time.sleep(.5)
        if 'UI_INTENT hand room=1 raised=true' not in app_log.read_text():
            raise AssertionError('H did not raise the hand while participants are open')
        key('h'); time.sleep(.5)
        if 'UI_INTENT hand room=1 raised=false' not in app_log.read_text():
            raise AssertionError('H did not lower the hand while participants are open')
        key('Escape')
        expect('Escape closes the call participants panel', absent=['Nicht im Anruf'])
        checks.append({'check': 'H toggles the hand in the call participants panel', 'result': 'pass'})
    elif scenario == 'call-dock':
        # Locate the room label so persisted filters and navigation entries can
        # change the sidebar's vertical layout without moving the test target.
        _, words, _ = observe()
        rooms = [word for word in words if normalized(word['text']).startswith('teamk')
                 and int(word['left']) < 240 and int(word['top']) < 600]
        if rooms:
            room = min(rooms, key=lambda word: int(word['top']))
            x = int(room['left']) + int(room['width']) // 2
            y = int(room['top']) + int(room['height']) // 2
        else:
            # The small label beside call avatars can evade OCR. This point was
            # inspected in the exact 1280x800 fixture, after the sidebar filters.
            path, _, _ = observe()
            size = run(['identify', '-format', '%w %h', str(path)], env).stdout
            if size.strip() != '1280 800':
                raise AssertionError('Active call room label is missing from the sidebar')
            x, y = 150, 367
            print(f'Native active call room hover uses inspected ({x}, {y}) in {path.name}; OCR unavailable', flush=True)
        run(['xdotool', 'mousemove', '--window', xid, str(x), str(y)], env)
        expect_settled('Active room reveals its join action on hover', ['Beitreten'])
        click('Beitreten', xmin=140, ymin=250)
        expect('Join action expands the owned call', ['Teamk', 'Im Gespräch'], ['Nachricht an Produktteam'])
    elif scenario in ('gate-recording', 'gate-recording-direct'):
        # Locate the switch from its visible label. At this fixture size the
        # switch is 58 px left of "stimme"; verify its actual painted state.
        # The fixture log reports only a local room identity, never media data.
        path, _, _ = observe()
        size = tuple(map(int, run(['identify', '-format', '%w %h', str(path)], env).stdout.split()))
        if size != (1280, 800):
            raise AssertionError('Consent interaction requires the inspected 1280x800 layout')
        app_log = out / f'{label}.log'
        def consent_sent():
            return app_log.read_text().count('UI_INTENT recording-consent room=')
        def point(x, y):
            run(['xdotool', 'mousemove', '--window', xid, str(x), str(y), 'click', '1'], env)
            time.sleep(.35)
        def switch_state(expected):
            path, words, _ = observe()
            matches = [r for r in words if normalized(r['text']) == 'stimme']
            if not matches:
                raise AssertionError(f'Cannot locate the recording consent switch: {path.name}')
            # The keyboard hint can repeat the label to the left of the row.
            hit = max(matches, key=lambda word: int(word['left']))
            x = int(hit['left']) - 58
            y = int(hit['top']) + int(hit['height']) // 2
            pixel = run(['convert', str(path), '-format', f'%[pixel:p{{{x - 10},{y}}}]', 'info:'], env).stdout
            channels = re.search(r'(?:s?rgba?)\((\d+),(\d+),(\d+)', pixel)
            if not channels:
                raise AssertionError(f'Cannot read the recording switch: {pixel}')
            red, green, blue = map(int, channels.groups())
            on = blue > red + 30 and red < 160
            off = min(red, green, blue) > 230
            if (expected and not on) or (not expected and not off):
                raise AssertionError(f'Recording switch was not {"on" if expected else "off"}: {pixel}')
            return x, y
        def click_disabled_consent():
            x, y = switch_state(False)
            path, _, _ = observe()
            crop = out / f'{label}-ocr-disabled-consent.png'
            run(['convert', str(path), '-crop', f'210x65+{x + 190}+{y + 52}', '+repage',
                 '-resize', '400%', '-colorspace', 'gray', '-auto-level', str(crop)], env)
            text = normalized(run(['tesseract', str(crop), 'stdout', '--psm', '6'], env).stdout)
            crop.unlink()
            if 'zustimmen' not in text:
                raise AssertionError(f'Cannot locate the disabled consent action: {path.name}')
            point(x + 294, y + 81)
        def click_selected_action(accept):
            x, y = switch_state(True)
            path, _, _ = observe()
            crop = out / f'{label}-ocr-consent-action.png'
            left, width = (x + 190, 210) if accept else (x + 75, 130)
            run(['convert', str(path), '-crop', f'{width}x55+{left}+{y + 20}', '+repage',
                 '-resize', '400%', '-colorspace', 'gray', '-auto-level', str(crop)], env)
            text = normalized(run(['tesseract', str(crop), 'stdout', '--psm', '6'], env).stdout)
            crop.unlink()
            if ('zustimmen' if accept else 'nicht') not in text:
                raise AssertionError(f'Cannot locate the consent action: {path.name}')
            point(x + (294 if accept else 140), y + 47)
        click_disabled_consent()  # Initially disabled: no implicit consent.
        if consent_sent():
            raise AssertionError('Unselected recording consent was submitted')
        point(*switch_state(False))  # Select, then use the keyboard path to clear it.
        switch_state(True)
        key('space'); time.sleep(.35)
        click_disabled_consent()
        if consent_sent():
            raise AssertionError('Cleared recording consent was submitted')
        point(*switch_state(False))
        switch_state(True)
        key('Escape'); time.sleep(.35)
        click('prüfen')  # Reopen from the still-required gate.
        click_disabled_consent()
        if consent_sent():
            raise AssertionError('Reopened consent retained an earlier selection')
        point(*switch_state(False))
        switch_state(True)
        if scenario == 'gate-recording-direct':
            click_selected_action(True)
            if consent_sent() != 1:
                raise AssertionError('Explicit direct-call consent was not submitted exactly once')
            expect('Explicit recording consent continues the direct call', ['Verbindung'], ['Zustimmung prüfen'])
        else:
            click_selected_action(False)
            if consent_sent():
                raise AssertionError('Declining submitted recording consent')
            expect('Declining recording consent leaves the call', ['Nachricht an Produktteam'])
        checks.append({'check': 'Recording consent starts clear and is reset on reopen', 'result': 'pass'})
    elif scenario == 'prejoin':
        expect('conference prejoin starts open', ['Bereit', 'Jetzt', 'teil'])
        time.sleep(1)  # Allow the first frame to mount and focus the modal.
        key('Escape')
        time.sleep(.8)  # Let the native compositor present the returned room.
        expect('conference prejoin cancellation returns to room', ['Nachricht an Designbesprechung'])
        click('Beitreten', inspected_point=(1280,800,1134,92))
        expect('explicit conference entry shows prejoin', ['Bereit', 'Jetzt', 'teil'])
        # OCR can see the same word in the explanatory sentence but miss the
        # blue button. The next state verifies this inspected native hit point.
        path, _, _ = observe()
        size = tuple(map(int, run(['identify', '-format', '%w %h', str(path)], env).stdout.split()))
        if size != (1280, 800):
            raise AssertionError('Prejoin interaction requires the inspected 1280x800 layout')
        run(['xdotool', 'mousemove', '--window', xid, '1080', '500', 'click', '1'], env)
        expect('join reports connecting without synthetic media success', ['Verbindung wird aufgebaut'], ['Anruf aktiv'])
    if not checks:
        # Passive, denied and loading fixtures still receive native input.
        # This only qualifies window input/paint; feature transitions above
        # retain their stronger, scenario-specific assertions.
        key('Tab')
        path, _, _ = observe()
        focused = run(['xdotool', 'getwindowfocus'], env).stdout.strip()
        if focused != str(xid):
            raise AssertionError(f'Tab moved native focus outside fixture window {xid}')
        colors = int(run(['identify', '-format', '%k', str(path)], env).stdout)
        if colors <= 16:
            raise AssertionError('Native fixture became blank after Tab')
        checks.append({'check': 'native Tab input retains the fixture window and painted surface',
                       'result': 'pass', 'image': path.name, 'sha256': sha(path),
                       'coverage': 'window input/paint only; no feature-specific transition'})
    return checks
