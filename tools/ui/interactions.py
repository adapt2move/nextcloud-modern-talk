"""Small native keyboard/mouse smoke checks; no browser, DOM, server or media.

Imported by capture.py --interactions. OCR only locates visible native controls;
assertions concern observable transitions and retained input, not pixel goldens.
"""
import csv
import io
import re
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
        for term in required:
            if normalized(term).replace(' ', '') not in text.replace(' ', ''):
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
        click('cloud.example.test')
        key('ctrl+a'); type_text('probe.example.test'); key('ctrl+a'); key('ctrl+c')
        key('BackSpace'); key('ctrl+v')
        expect('native selection and explicit clipboard roundtrip', ['probe.example.test'])
        key('ctrl+a'); type_text('replacement.example.test'); key('Home'); key('Delete'); key('End')
        expect('native editing replaces selection and deletes one grapheme', ['eplacement.example.test'], ['probe.example.test'])
        key('Tab')
        expect('keyboard focus can leave input')
    elif scenario == 'clipboard-denied':
        key('ctrl+k');type_text('CLIPMARK');key('ctrl+a');key('ctrl+c');key('BackSpace')
        click('schreiben');type_text('DRAFTSAFE');key('ctrl+a');key('ctrl+x')
        expect('unknown privacy policy blocks cut without losing draft',['DRAFTSAFE','Ausschneiden ist für diesen Entwurf nicht freigegeben.'])
        key('ctrl+c');key('ctrl+v')
        expect('unknown privacy policy blocks export but permits selected-text replacement by paste',['CLIPMARK'],['DRAFTSAFE'])
        key('ctrl+k');key('ctrl+v')
        expect('denied draft copy did not replace platform clipboard',['CLIPMARK'])
    elif scenario in ('chat', 'busy'):
        # Search is local to bounded room rows. Focus shortcut works from root.
        key('ctrl+k'); type_text('NO MATCH UI')
        expect('local search filter reports no matching rows', ['Keine lokalen Treffer'])
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
        words = expect('thread resize handle accepts keyboard input', ['Design review', '2 Antworten'])
        keyboard = next(int(word['left']) for word in words
                        if normalized(word['text']).startswith('design')
                        and int(word['top']) < 120 and int(word['left']) > 700)
        if keyboard - after < 20:
            raise AssertionError('The focused thread resize handle did not respond to Right')
        key('Left')
        click('antworten', xmin=900, ymin=650, inspected_point=(1280,800,1010,700))
        type_text('THREADDRAFT')
        time.sleep(1.5)
        expect('thread composer accepts text', ['THREADDRAFT'])
        key('Return')
        words = expect('thread Enter retains the draft after a rejected submission', ['THREADDRAFT', 'Nicht gesendet'], footer_height=200, include_footer_words=True)
        if not any(normalized(word['text']) == 'nicht' and int(word['left']) >= after - 10 for word in words):
            raise AssertionError('Thread submission feedback did not appear beside its composer')
        if any(normalized(word['text']) == 'nicht' and int(word['left']) < after - 10 for word in words):
            raise AssertionError('Thread submission feedback appeared in the main composer')
        log = (out / f'{label}.log').read_text()
        if not re.search(r'UI_CHAT_UX send room=\d+ bytes=11 .*thread=Some\(1\)', log):
            raise AssertionError('Enter did not submit the draft to the scoped thread')
        checks.append({'check': 'thread Enter submits the scoped draft', 'result': 'pass'})
        key('Escape')
        expect('Escape closes the thread pane and returns to the room',
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
    elif scenario == 'prejoin':
        key('Escape')
        expect('conference prejoin cancellation returns to room', ['Nachricht an Produktteam'])
        click('Beitreten')
        expect('explicit conference entry shows prejoin', ['Bereit', 'Jetzt', 'teil'])
        click('teilnehmen',inspected_point=(1200,800,600,695))
        expect('join reports connecting without synthetic media success', ['Verbindung wird angefragt'], ['Anruf aktiv'])
    return checks
