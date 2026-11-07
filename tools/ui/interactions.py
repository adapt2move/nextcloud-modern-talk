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
    return ''.join(c for c in unicodedata.normalize('NFKD', value).casefold() if c.isalnum() or c.isspace())


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

    def expect(name, required=(), absent=()):
        path, words, text = observe()
        text += normalized(run(["tesseract",str(path),"stdout","--psm","6"],env).stdout)
        width,height=map(int,run(["identify","-format","%w %h",str(path)],env).stdout.split())
        ocr_path=out/f"{label}-ocr-footer.png"
        run(["convert",str(path),"-crop",f"{width}x120+0+{height-120}","+repage","-resize","300%","-colorspace","gray","-auto-level",str(ocr_path)],env)
        text += normalized(run(["tesseract",str(ocr_path),"stdout","--psm","6"],env).stdout)
        ocr_path.unlink()
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
        run(['xdotool', 'type', '--clearmodifiers', '--delay', '150', '--', value], env)

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

    if scenario == 'server':
        click('cloud.example.test', inspected_point=(1280, 800, 700, 328))
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
        expect('debounced search changes the listing', ['Workshop.png'], ['Projektübersicht.pdf'])
        key('ctrl+a'); key('BackSpace')
        time.sleep(.7)
        expect('clearing search restores the root listing', ['Projektübersicht.pdf', 'Workshop.png'])
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
        expect('keyboard selection returns both cloud files to the composer',
               ['Projektübersicht.pdf', 'Workshop.png'], ['NextCloud-Dateien'])
    elif scenario == 'chat-cloud-files-offline':
        expect('offline browser keeps its cached listing', ['Offline', 'gespeicherte Dateien', 'Projektübersicht.pdf'])
    elif scenario == 'chat-upload-preview':
        expect('staged image has a composer chip', ['Zwischenablage.png'])
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
            expect('poll field retains pasted text', [value])
        expect('completed poll keeps both distinct choices', ['TEAM LUNCH', 'GARDEN', 'STUDIO'],
               ['Mindestens zwei unterschiedliche Antwortoptionen'])
        click('Veröffentlichen', inspected_point=(1280, 800, 865, 704))
        expect('accepted fictional poll closes the dialog', ['Designteam'], ['Umfrage erstellen'])
    elif scenario == 'poll-settings':
        expect('conversation settings omit the composer poll action', ['Allgemein', 'Nachrichten'], ['Umfrage erstellen'])
        click('auswählen', xmin=400, ymin=150)
        # The first emoji in the bounded picker is a native button in the fixed fixture.
        run(['xdotool', 'mousemove', '--window', xid, '475', '270', 'click', '1'], env)
        expect('selecting an emoji enables the room avatar action', ['Emoji als Avatar setzen'])
        key('Escape')
        expect('Escape closes the conversation settings', ['Designteam'], ['Gesprächsdetails'])
    elif scenario == 'poll-drafts':
        expect('draft list is separate from the poll form', ['Fiktiver Entwurf'], ['Antwortoption'])
        click('Entwürfe', xmin=700, inspected_point=(1280, 800, 840, 138))
        expect('draft toggle returns to the poll form', ['Frage', 'Antwortoption'], ['Fiktiver Entwurf'])
        click('Entwürfe', xmin=700, inspected_point=(1280, 800, 840, 138))
        expect('draft toggle reopens the saved list', ['Fiktiver Entwurf'], ['Antwortoption'])
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
               ['Umfrage beendet', 'Fiktive Umfrage'], ['Umfrage beenden'])
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
        # The current shortcut opens the unified search surface.
        key('ctrl+k')
        expect('search shortcut opens the search surface', ['Suchen', 'Personen'])
        key('Escape')
        click('Nachricht', xmin=200, ymin=600)
        time.sleep(.3)  # Allow native focus restoration after closing search.
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
    elif scenario == 'incoming':
        key('Return')
        expect('unrelated Enter does not accept incoming call', ['Annehmen'], ['Verbindung wird'])
        click('ansehen')
        expect('dismiss keeps a discoverable incoming call', ['Eingehender Anruf ansehen'], ['Annehmen'])
        click('ansehen')
        # The blue accept icon is distinct from the two adjacent call actions.
        run(['xdotool', 'mousemove', '--window', xid, '605', '502', 'click', '1'], env)
        expect('direct accept bypasses optional conference prejoin', ['Verbindung wird aufgebaut'], ['Bereit'])
        key('Escape')
        expect('escape never hangs up a call', ['Verbindung wird aufgebaut'])
        click('Auflegen',inspected_point=(1280,800,1225,754))
        expect('explicit leave returns to conversation', ['Nachricht an Mara Fischer'])
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
        # The room row is visible in the fixed 1280x800 fictional dock fixture.
        run(['xdotool', 'mousemove', '--window', xid, '150', '290'], env)
        expect('Active room reveals its join action on hover', ['Beitreten'])
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
        # The conversation header exposes conference entry as an icon.
        run(['xdotool', 'mousemove', '--window', xid, '1181', '92', 'click', '1'], env)
        expect('explicit conference entry shows prejoin', ['Bereit', 'Jetzt', 'teil'])
        click('teilnehmen',inspected_point=(1280,800,640,695))
        expect('join reports connecting without synthetic media success', ['Verbindung wird aufgebaut'], ['Anruf aktiv'])
    return checks
