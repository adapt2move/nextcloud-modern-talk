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

    def expect(name, required=(), absent=()):
        path, words, text = observe()
        text += normalized(run(["tesseract",str(path),"stdout","--psm","6"],env).stdout)
        width,height=map(int,run(["identify","-format","%w %h",str(path)],env).stdout.split())
        ocr_path=out/f"{label}-ocr-footer.png"
        run(["convert",str(path),"-crop",f"{width}x120+0+{height-120}","+repage","-resize","300%","-colorspace","gray","-auto-level",str(ocr_path)],env)
        text += normalized(run(["tesseract",str(ocr_path),"stdout","--psm","6"],env).stdout)
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
    elif scenario == 'search-navigation':
        expect('switcher displays its initial matching conversation', ['Produktteam', 'Gruppengespräch'])
        click('Produkt', xmin=300, ymin=300)
        key('ctrl+a'); key('BackSpace')
        expect('empty switcher shows recent rooms', ['Mara Fischer', 'Jonas Weber'])
        key('Down'); key('Down'); key('Return')
        expect('arrows and Enter open the selected conversation', ['Nachricht an Mara Fischer'], ['Mit Pfeiltasten auswählen'])
        key('ctrl+k')
        expect('keyboard shortcut reopens switcher', ['Mit Pfeiltasten auswählen'])
        type_text('Jonas')
        expect('switcher matches a group participant as well as a direct room', ['Jonas Weber', 'Produktteam'])
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
        # Lavapipe can present the previous frame after the input event. The
        # assertion still requires the native result, with a finite render wait.
        for attempt in range(3):
            try:
                expect('typing narrows the loaded rooms immediately', ['Designrunde'], ['Kaffeepause'])
                break
            except AssertionError:
                if attempt == 2: raise
                time.sleep(.75)
        key('Escape')
        expect('Escape closes open conversation browsing', ['Nachricht an Produktteam'], ['Designrunde'])
    elif scenario == 'reference-preview':
        expect('resolved reference card is shown with its source message', ['Ein ruhiger Ort', 'Studiokonzept'])
        click('Vorschau verbergen', inspected_point=(1280, 800, 826, 400))
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
        expect('sidebar filters start with the loaded rooms', ['Produktteam', 'Teamküche', 'Erwähnungen'])
        click('Erwähnungen')
        expect('mention filter keeps known mentions and explains unknown state', ['Produktteam', 'Erwähnungen sind noch nicht bekannt'], ['Teamküche'])
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
