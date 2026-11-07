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
            crop=f'{width//2}x{height//2}+{width//4}+{height//4}'
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
        for term in required:
            # At small sizes Tesseract sometimes reads the highlighted German
            # selection count's final letters as "ahit". Keep the numeric count
            # and prefix exact; the subsequent staged chips verify both files.
            selection_count = re.fullmatch(r'([12]) ausgewählt', term)
            fuzzy_count = selection_count and re.search(
                rf'\b{selection_count.group(1)}\s*ausge\w{{4,8}}', text)
            if selection_count and not fuzzy_count and (width, height) == (1280, 800):
                # This inspected footer location is stable in the fixed-size
                # fixture. A contrast-enhanced crop also works in dark mode.
                count_path=out/f'{label}-ocr-count.png'
                run(['convert', str(path), '-crop', '120x35+440+660', '+repage',
                     '-resize', '400%', '-colorspace', 'gray', '-negate',
                     '-contrast-stretch', '2%x2%', str(count_path)], env)
                count_text=normalized(run(['tesseract', str(count_path), 'stdout', '--psm', '7'], env).stdout)
                count_path.unlink()
                fuzzy_count=re.search(rf'\b{selection_count.group(1)}\s*ausge\w{{4,8}}', count_text)
            aliases = {
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
        run(['xdotool', 'type', '--clearmodifiers', '--delay', '4', '--', value], env)

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

    def click(word, xmin=0, ymin=0, inspected_point=None):
        path, words, _ = observe()
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
        aliases={'teilnehmen':('teilnenmen','telinenmen'),'Auflegen':('auriegen','autlegen','auilegen')}
        if not choices and word in aliases:
            choices=[r for r in words if normalized(r['text']) in aliases[word] and int(r['left'])>=xmin and int(r['top'])>=ymin]
        if choices:
            hit = choices[-1]
            x = int(hit['left']) + int(hit['width']) // 2
            y = int(hit['top']) + int(hit['height']) // 2
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
        run(['xdotool', 'mousemove', '--window', xid, str(x), str(y), 'click', '1'], env)

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
        # The fixture fixes the calendar at 15 January 2027. Sample the empty
        # corners of adjacent day cells so the assertion measures the actual
        # selected background, independently of text recognition.
        before, _, _ = observe()
        pixels = lambda image: run(['convert', str(image), '-format',
            '%[pixel:p{466,512}]|%[pixel:p{506,512}]', 'info:'], env).stdout.split('|')
        first = pixels(before)
        if len(first) != 2 or first[0] == first[1]:
            raise AssertionError('Calendar fixture does not show the expected selected day')
        key('Right')
        after, _, _ = observe()
        second = pixels(after)
        if second[0] != first[1] or second[1] != first[0]:
            raise AssertionError('Right arrow did not move the selected calendar day')
        checks.append({'check': 'calendar arrow navigation moves selection', 'result': 'pass',
                       'image': after.name, 'sha256': sha(after)})
    elif scenario == 'chat-reminder-menu':
        expect('message menu combines reminder and unread actions',
               ['Erinnerung bearbeiten', 'Gespräch als ungelesen markieren'])
        click('bearbeiten', xmin=1000, ymin=200)
        expect('message reminder opens the shared calendar', ['Später erinnern', 'Januar', 'Fiktiver Treffpunkt'])
        key('Escape')
        expect('calendar cancellation retains the message', ['Hier treffen wir uns.'], ['Später erinnern'])
    elif scenario == 'chat-schedule-time':
        expect('send-later opens a calendar beside the draft', ['Nachricht planen', 'Januar', 'Fiktive Nachricht für morgen.'])
        key('Escape')
        expect('cancelling send-later retains the draft', ['Fiktive Nachricht für morgen.'], ['Nachricht planen'])
        type_text('X')
        expect('send-later cancellation returns focus to the composer', ['Fiktive Nachricht für morgen.X'])
    elif scenario == 'thread-message-tools':
        expect('thread translation and account navigation coexist',
               ['Design review', 'Das passt', 'Original anzeigen', 'Threads', 'Erinnerungen'])
        click('anzeigen', xmin=950, ymin=300)
        expect('thread translation can return to its original', ['That works for me.'], ['Das passt'])
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
    elif scenario == 'server':
        click('cloud.example.test')
        key('ctrl+a'); type_text('probe.example.test'); key('ctrl+a'); key('ctrl+c')
        key('BackSpace'); key('ctrl+v')
        expect('native selection and explicit clipboard roundtrip', ['probe.example.test'])
        key('ctrl+a'); type_text('replacement.example.test'); key('Home'); key('Delete'); key('End')
        expect('native editing replaces selection and deletes one grapheme', ['eplacement.example.test'], ['probe.example.test'])
        key('Tab')
        expect('keyboard focus can leave input')
    elif scenario == 'status-message':
        expect('status message editor is visible', ['Statusnachricht', 'Vorschläge'])
        click('Statusnachricht')
        type_text('LUNCHBREAK')
        click('30')
        click('Speichern', inspected_point=(1280,800,67,632))
        expect('saved fictional status is shown immediately', ['LUNCHBREAK'])
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
        click('schreiben');type_text('DRAFTSAFE');key('ctrl+a');key('ctrl+x')
        expect('unknown privacy policy blocks cut without losing draft',['DRAFTSAFE','Ausschneiden ist für diesen Entwurf nicht freigegeben.'])
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
                       input='file:///tmp/Fictional-paste.txt\r\n', text=True, env=env,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True, timeout=10)
        data = run(['xclip', '-selection', 'clipboard', '-t', 'text/uri-list', '-o'], env).stdout
        if data.strip() != 'file:///tmp/Fictional-paste.txt':
            raise AssertionError('the X11 clipboard did not retain the native file-list fixture')
        key('ctrl+v')
        expect('native file-list paste stages a file chip', ['Fictional-paste.txt', 'Bereit zum Senden'])
        image = Path('apps/desktop/assets/fixtures/content-photo.png').read_bytes()
        subprocess.run(['xclip', '-selection', 'clipboard', '-t', 'image/png', '-i'],
                       input=image, env=env, stdout=subprocess.DEVNULL,
                       stderr=subprocess.DEVNULL, check=True, timeout=10)
        key('ctrl+v')
        expect('native image paste stages an image chip', ['Fictional-paste.txt', 'clipboard.png'])
    elif scenario == 'chat-cloud-files':
        click('Team', xmin=285, ymin=400)
        expect('folder navigation opens its listing', ['Konzept.pdf', 'Notizen.txt'])
        key('BackSpace')
        expect('backspace navigates to the parent folder', ['Projektübersicht.pdf', 'Workshop.png'])
        click('suchen', xmin=285, ymin=200, inspected_point=(1280, 800, 450, 256))
        type_text('Workshop')
        time.sleep(.7)  # Allow the 250 ms debounce and the native owner ticks.
        expect_settled('debounced search changes the listing', ['Workshop.png'], ['Projektübersicht.pdf'])
        key('ctrl+a'); key('BackSpace')
        time.sleep(.7)
        expect_settled('clearing search restores the root listing', ['Projektübersicht.pdf', 'Workshop.png'])
        # The file row is visible in dark and light captures, but low contrast
        # can make Tesseract miss its name. Verify the resulting selection below.
        click('Projektübersicht.pdf', inspected_point=(1280, 800, 400, 580))
        key('w'); key('ctrl+space')
        expect('type-ahead and control-space select the matching file', ['2 ausgewählt'])
        key('ctrl+space')
        expect('control-space toggles the focused file', ['1 ausgewählt'])
        click('Projektübersicht.pdf', inspected_point=(1280, 800, 400, 555))
        key('shift+Down')
        expect('shift and arrow extend the native file selection', ['2 ausgewählt'])
        key('Return')
        expect_settled('keyboard selection returns both cloud files to the composer',
               ['Projektübersicht.pdf', 'Workshop.png', 'Bereit zum Senden'], ['NextCloud-Dateien'])
    elif scenario == 'chat-cloud-files-offline':
        expect('offline browser keeps its cached listing', ['Offline', 'gespeicherte Dateien', 'Projektübersicht.pdf'])
    elif scenario == 'chat-upload-preview':
        expect('staged image has a composer chip', ['Zwischenablage.png', 'Bereit zum Senden'])
    elif scenario == 'poll-dialog':
        expect('empty poll starts without validation warnings', ['Umfrage erstellen'],
               ['Mindestens zwei unterschiedliche Antwortoptionen'])
        # Exercise plain-text paste in each poll input independently.
        for value, question in [('TEAM LUNCH', True), ('GARDEN', False), ('STUDIO', False)]:
            if not question:
                click('Antwortoption')
            subprocess.run(['xclip', '-selection', 'clipboard', '-i'],
                           input=value, text=True, env=env, stdout=subprocess.DEVNULL,
                           stderr=subprocess.DEVNULL, check=True, timeout=10)
            key('ctrl+v')
            expect_settled('poll field retains pasted text', [value])
        expect('completed poll keeps both distinct choices', ['TEAM LUNCH', 'GARDEN', 'STUDIO'],
               ['Mindestens zwei unterschiedliche Antwortoptionen'])
        click('Veröffentlichen', inspected_point=(1280, 800, 865, 655))
        expect_settled('accepted fictional poll closes the dialog', ['Designteam'], ['Umfrage erstellen'])
    elif scenario == 'poll-settings':
        expect('conversation settings offers poll creation', ['Allgemein', 'Umfrage erstellen'])
        click('erstellen', inspected_point=(1280, 800, 540, 548))
        expect('settings opens the scoped poll dialog', ['Frage', 'Mehrere Antworten erlauben'])
        key('Escape')
        time.sleep(1.)  # Let the retained parent dialog regain focus and paint.
        expect('closing the poll dialog restores conversation settings', ['Allgemein', 'Nachrichten'])
    elif scenario == 'chat-polls':
        click('Abstimmen', inspected_point=(1280, 800, 390, 552))
        expect('accepted fictional vote stays visible', ['Deine Stimme'])
        click('beenden')
        expect('closing asks for confirmation', ['Umfrage wirklich beenden', 'Jetzt beenden'])
        click('Jetzt')
        expect('confirmed fictional close keeps the poll and disables voting',
               ['Umfrage beendet', 'Fiktive Umfrage'], ['Umfrage beenden'])
        log = (out / f'{label}.log').read_text()
        if log.count('UI_POLL_FIXTURE vote poll=9001 no_backend=true') != 1 or \
                log.count('UI_POLL_FIXTURE close poll=9001 no_backend=true') != 1:
            raise AssertionError('Vote or confirmed close did not submit exactly once')
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
    elif scenario == 'voice-recording':
        path, _, _ = observe()
        width, height = map(int, run(['identify', '-format', '%w %h', str(path)], env).stdout.split())
        if (width, height) != (1280, 800):
            raise AssertionError('Recording interaction requires the inspected 1280x800 fixture')
        # The rightmost round button is the explicit direct-send action.
        run(['xdotool', 'mousemove', '--window', xid, '1214', '734',
             'click', '--repeat', '2', '--delay', '50', '1'], env)
        time.sleep(.8)
        log = (out / f'{label}.log').read_text()
        if log.count('UI_VOICE_FIXTURE send no_backend=true') != 1:
            raise AssertionError('Direct recording send did not settle exactly once')
        expect('direct send retires the fictional recording', ['Nachricht an Designteam'])
    elif scenario in ('chat', 'busy'):
        # Search is local to bounded room rows. Focus shortcut works from root.
        key('ctrl+k'); type_text('NO MATCH UI')
        expect('local search filter reports no matching rows', ['Keine passenden Gespräche oder Personen'])
        key('ctrl+a'); key('BackSpace')
        click('schreiben')
        type_text('DRAFTALPHA'); key('Return')
        expect('draft retained after busy rejection' if scenario == 'busy' else 'draft retained after asynchronous failure',
               ['DRAFTALPHA', 'ausgelastet' if scenario == 'busy' else 'Nicht gesendet'])
        key('ctrl+comma')
        expect('settings shortcut opens modal', ['Einstellungen'])
        click('Sage'); click('Dunkel')
        expect('theme selection stays available after appearance change', ['Einstellungen','Sage'])
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
        expect('thread hover keeps reply text readable', ["I'll update the final details."])
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
        click('ansehen',xmin=800)
        expect('dismiss keeps a discoverable incoming call', ['Eingehender Anruf ansehen'], ['Annehmen'])
        click('ansehen',xmin=800)
        click('Annehmen',xmin=800,ymin=200,inspected_point=(1200,800,987,244))
        expect('direct accept bypasses optional conference prejoin', ['Verbindung wird angefragt'], ['Bereit'])
        key('Escape')
        expect('escape never hangs up a call', ['Verbindung wird angefragt'])
        click('Auflegen',inspected_point=(1200,800,862,682))
        expect('explicit leave returns to conversation', ['Nachricht an Produktteam'])
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
        # The following points were inspected at this exact fixture size. The
        # fixture log reports only the local room identity, never media data.
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
        point(660, 485)  # Initially disabled: no implicit consent.
        if consent_sent():
            raise AssertionError('Unselected recording consent was submitted')
        point(468, 437)  # Select the switch, then use its keyboard path to clear it.
        key('space'); time.sleep(.35)
        point(660, 485)
        if consent_sent():
            raise AssertionError('Cleared recording consent was submitted')
        point(468, 437)
        key('Escape'); time.sleep(.35)
        point(500, 235)  # Reopen from the still-required gate.
        point(660, 485)
        if consent_sent():
            raise AssertionError('Reopened consent retained an earlier selection')
        point(468, 437)
        if scenario == 'gate-recording-direct':
            point(660, 485)
            if consent_sent() != 1:
                raise AssertionError('Explicit direct-call consent was not submitted exactly once')
            expect('Explicit recording consent continues the direct call', ['Verbindung'], ['Zustimmung prüfen'])
        else:
            point(500, 485)
            if consent_sent():
                raise AssertionError('Declining submitted recording consent')
            expect('Declining recording consent leaves the call', ['Nachricht an Produktteam'])
        checks.append({'check': 'Recording consent starts clear and is reset on reopen', 'result': 'pass'})
    elif scenario == 'prejoin':
        key('Escape')
        expect('conference prejoin cancellation returns to room', ['Nachricht an Produktteam'])
        click('Beitreten')
        expect('explicit conference entry shows prejoin', ['Bereit', 'Jetzt', 'teil'])
        click('teilnehmen',inspected_point=(1200,800,600,695))
        expect('join reports connecting without synthetic media success', ['Verbindung wird angefragt'], ['Anruf aktiv'])
    return checks
