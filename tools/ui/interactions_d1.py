"""D1 native input/focus observations. Captures are originals; OCR crops are temporary."""
import csv, io, time
from pathlib import Path
from capture import run, sha
from interactions import normalized

def exercise(scenario,env,xid,out,label,large=False):
    checks=[];step=0
    def key(k):
        run(['xdotool','key','--clearmodifiers',k],env);time.sleep(.12)
    def click(x,y):run(['xdotool','mousemove','--window',xid,str(x),str(y),'click','1'],env)
    def type_text(s):run(['xdotool','type','--clearmodifiers','--delay','3','--',s],env)
    def observe(name):
        nonlocal step
        step+=1;run(['xdotool','mousemove','--window',xid,'1100' if not large else '700','705' if not large else '570'],env);time.sleep(.4)
        p=out/f'{label}-d1-{step:02}-{name}.png';run(['import','-window',xid,str(p)],env)
        text=normalized(run(['tesseract',str(p),'stdout','--psm','11'],env).stdout + run(['tesseract',str(p),'stdout','--psm','6'],env).stdout)
        # Derived inversion is OCR-only; never overwrites or publishes app pixels.
        derived=out/'ocr-inverted.png'
        run(['convert',str(p),'-colorspace','gray','-threshold','55%','-negate','-resize','200%',str(derived)],env)
        text+=normalized(run(['tesseract',str(derived),'stdout','--psm','11'],env).stdout)
        derived.unlink()
        return p,text
    def check(name,p,assertion,detail=None):
        if not assertion:raise AssertionError(f'{name}: {p.name}; {detail}')
        checks.append({'check':name,'result':'pass','image':p.name,'sha256':sha(p),'observation':detail})
    def expect(name,required=(),absent=()):
        p,text=observe(name)
        compact=text.replace(' ','')
        check(name,p,all(normalized(x).replace(' ','') in compact for x in required) and all(normalized(x).replace(' ','') not in compact for x in absent),text)
        return p
    def pixel(p,x,y):return run(['convert',str(p),'-format',f'%[pixel:p{{{x},{y}}}]','info:'],env).stdout
    def same_region(a,b,rect):
        values=[]
        for image in [a,b]:values.append(run(['convert',str(image),'-crop',rect,'+repage','-depth','8','txt:-'],env).stdout)
        return values[0]==values[1]
    def click_word(word):
        p,_=observe('locate')
        tsv=run(['tesseract',str(p),'stdout','--psm','11','tsv'],env).stdout
        words=[r for r in csv.DictReader(io.StringIO(tsv),delimiter='\t') if normalized(word) in normalized(r.get('text',''))]
        if not words:raise AssertionError(f'Native visible text missing: {word}')
        w=words[-1];click(int(w['left'])+int(w['width'])//2,int(w['top'])+int(w['height'])//2)
    if large:
        # Keep primary controls reachable by intentional native scrolling at 720x580.
        run(['xdotool','mousemove','--window',xid,'650','430'],env)
        run(['xdotool','click','--repeat','35','--delay','20','5'],env)
        if scenario=='server':
            p,_=observe('200pct-server-action-reachable')
            # White on blue is missed by whole-image OCR. This button was
            # visually inspected in this exact native 720x580 frame; activation
            # must still reach the illustrative next phase, never real auth.
            size=run(['identify','-format','%w %h',str(p)],env).stdout
            if size!='720 580':raise AssertionError('Inspected action requires 720x580')
            click(360,390)
            expect('200pct-server-action-enters-illustrative-browser-step',['Schritt 2 von 3'])
            check('200pct-primary-action-reachable',p,True,'Inspected Weiter at (360,390); native click reached fixture step 2')
        elif scenario=='settings':
            expect('200pct-settings-notice-reachable',['bis zum Beenden'])
            key('Escape');expect('200pct-settings-escape',['Produktteam'],['Darstellung'])
        elif scenario=='chat':
            expect('200pct-composer-reachable',['Nachricht','senden'])
            click_word('Nachricht');type_text('GROSSENTWURF');key('Return');expect('200pct-failed-send-retains-draft',['GROSSENTWURF','Nicht gesendet'])
        return checks
    if scenario=='settings':
        key('Escape');click(240,658);expect('gear-opens-layered-settings',['Einstellungen','Darstellung'])
        # The modal container owns focus; Tab traverses two navigation rows and
        # enters the appearance group at the selected (Dark) card.
        key('Tab');key('Tab');key('Tab');a,_=observe('focus-dark')
        key('Left');b,_=observe('focus-light-unselected')
        check('arrow-focus-does-not-select-or-move-card-content',b,same_region(a,b,'166x96+582+146') and same_region(a,b,'166x96+779+146'),'Light/Dark interior pixels unchanged; only exterior focus ring moves')
        check('arrow-moves-visible-focus-ring',b,pixel(a,568,180)!=pixel(b,568,180),'Focus ring at Light card exterior differs')
        key('Return');c=expect('enter-activates-light',['Darstellung','Hell'])
        check('enter-changes-actual-surface',c,pixel(c,970,280)!=pixel(b,970,280),'Actual modal surface changes from dark to light')
        key('Right');key('space');d=expect('space-activates-dark',['Darstellung','Dunkel'])
        check('space-restores-dark-surface',d,pixel(d,970,280)==pixel(b,970,280),'Actual modal surface returns to prior dark token')
        click(70,420);type_text('MODALCANARY');key('ctrl+k');type_text('BLOCKEDTEXT');expect('modal-blocks-background-click-and-typing',['Darstellung'],['MODALCANARY','BLOCKEDTEXT'])
        key('Escape');e=expect('escape-restores-workspace-and-gear-focus',['Produktteam'],['Darstellung','MODALCANARY','BLOCKEDTEXT'])
        key('Return');expect('restored-gear-focus-activates-with-enter',['Darstellung'])
        key('Escape');key('space');expect('restored-gear-focus-activates-with-space',['Darstellung'])
        key('Tab');key('Tab');key('Tab');f,_=observe('retained-dark-selection')
        check('appearance-selection-retained-after-close-reopen',f,pixel(f,970,280)==pixel(b,970,280),'Dark appearance remains selected after reopening')
    elif scenario in ('chat','busy'):
        key('ctrl+k');type_text('NO MATCH UI');expect('bounded-local-search',['Keine Gespräche'])
        key('ctrl+a');key('BackSpace');click_word('Nachricht');type_text('WOLKENENTWURF');key('Return')
        expect('busy-preserves-input' if scenario=='busy' else 'failed-submit-preserves-input',['WOLKENENTWURF','ausgelastet' if scenario=='busy' else 'Nicht gesendet'])
        key('ctrl+comma');expect('shortcut-opens-modal',['Darstellung']);key('Escape');expect('modal-return-preserves-draft',['WOLKENENTWURF'])
    elif scenario=='server':
        click_word('cloud.example.test');key('ctrl+a');type_text('probe.example.test');key('ctrl+a');key('ctrl+c');key('BackSpace');key('ctrl+v')
        expect('native-server-selection-clipboard-roundtrip',['probe.example.test'])
        key('Home');key('Delete');key('End');key('Tab')
        expect('native-home-delete-end-edit',['robe.example.test'],['probe.example.test'])
    return checks
