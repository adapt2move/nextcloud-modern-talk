"""D1.2 C1-C5 and scoped-focus native checks; X11 input, untouched captures.

Development-only focus fixtures change a finite projection via F6/Refresh.
Assertions inspect pixels, visible text and the bounded synthetic intent sink.
"""
import json
import re
import time
from capture import run, sha
from interactions import normalized


def exercise(scenario, env, xid, out, label, large=False):
    frames, checks = [], []
    def key(value):
        run(['xdotool', 'key', '--clearmodifiers', value], env); time.sleep(.16)
    def move(x=None, y=None):
        run(['xdotool','mousemove','--window',xid,str(x if x is not None else 700 if large else 1100),str(y if y is not None else 570 if large else 705)],env)
    def click(x,y):
        move(x,y);run(['xdotool','click','1'],env);time.sleep(.16)
    def type_text(value): run(['xdotool','type','--clearmodifiers','--delay','3','--',value],env)
    def observe(name, park=True):
        if park: move()
        time.sleep(.4)
        p=out/f'{label}-d12-{len(frames)+1:02}-{name}.png'
        run(['import','-window',xid,str(p)],env)
        frames.append({'name':name,'image':p.name,'sha256':sha(p)})
        return p
    def text(p):
        s=''.join(run(['tesseract',str(p),'stdout','--psm',mode],env).stdout for mode in ['11','6'])
        temp=out/'ocr-temporary.png'
        run(['convert',str(p),'-colorspace','gray','-threshold','55%','-negate','-resize','200%',str(temp)],env)
        s+=run(['tesseract',str(temp),'stdout','--psm','11'],env).stdout
        if not large and 'reason' in p.name:
            # Whole-scene OCR skips small dark tooltip panels. These fixed,
            # inspected native regions isolate their text; originals are untouched.
            for rect,mode in [('282x25+319+509','7'),('300x46+810+489','6'),('235x26+160+105','7'),('235x26+160+209','7')]:
                run(['convert',str(p),'-crop',rect,'+repage','-resize','400%','-colorspace','gray','-negate',str(temp)],env)
                s+=run(['tesseract',str(temp),'stdout','--psm',mode],env).stdout
        temp.unlink()
        # Recorded OCR-only umlaut confusion, never a change to app pixels/copy.
        return normalized(s).replace(' ', '').replace('verfuigbar','verfugbar').replace('ausgewahit','ausgewahlt')
    def check(name,p,ok,detail):
        checks.append({'check':name,'result':'pass' if ok else 'FAIL','image':p.name,'sha256':sha(p),'observation':detail})
        if not ok: raise AssertionError(f'{name}: {p.name}: {detail}')
    def expect(name, words=(), absent=()):
        p=observe(name);s=text(p)
        check(name,p,all(normalized(w).replace(' ','') in s for w in words) and all(normalized(w).replace(' ','') not in s for w in absent),s)
        return p
    def region(p,r): return run(['convert',str(p),'-crop',r,'+repage','-depth','8','txt:-'],env).stdout
    def same(a,b,r): return region(a,r)==region(b,r)
    def pixel(p,x,y):
        value=run(['convert',str(p),'-format',f'%[pixel:p{{{x},{y}}}]','info:'],env).stdout
        return tuple(float(n) for n in re.findall(r'[\d.]+',value)[:3])
    def ring_measure(p,x,y,focus):
        colors=[pixel(p,n,y) for n in range(x-6,x+4)]
        starts=[x-6+i for i in range(len(colors)-5) if colors[i]==colors[i+1]==focus and colors[i+2]==colors[i+3] and colors[i+2]!=focus and colors[i+4]==colors[i+5]]
        return {'nominal_edge':x,'y':y,'pixels':colors,'ring_start_x':starts}
    def log_events():
        return [l for l in (out/f'{label}.log').read_text().splitlines() if l.startswith('UI_FOCUS_PROBE ')]
    def inert(a,name):
        key('Return');key('space');b=observe(name)
        check(name,b,same(a,b,f'{720 if large else 1120}x{554 if large else 690}+0+0'),'Native Enter/Space leave UI identical; no action or result overlay')
        return b
    try:
        if large:
            base=observe('200pct-initial')
            if scenario=='settings':
                for _ in range(3):key('Tab')
                observe('200pct-appearance-focus-before-scroll')
                # Existing settings scroll does not auto-reveal the lower card.
                # Preserve that limitation and expose the complete ring by native wheel.
                move(630,480);run(['xdotool','click','--repeat','3','--delay','30','5'],env)
                focused=observe('200pct-appearance-focused-after-scroll')
                key('Left');other=observe('200pct-appearance-arrow')
                check('200pct-card-focus-moves-without-selection',other,not same(focused,other,'720x554+0+0'),'Native keyboard moves focus at 200%; no selection activation')
                key('Escape');expect('200pct-Escape-closes',['Produktteam'],['Darstellung'])
            elif scenario in ('offline','chat'):
                move(650,430);run(['xdotool','click','--repeat','35','--delay','20','5'],env)
                visible=expect('200pct-composer-reachable',['Nachricht'])
                click(400,430);type_text('SKALENTWURF');key('Tab')
                reason=expect('200pct-attachment-reason',['nicht verfügbar','SKALENTWURF'])
                inert(reason,'200pct-disabled-no-dispatch')
                key('Tab');send=observe('200pct-send-focused')
                if scenario=='offline': inert(send,'200pct-disabled-send-no-dispatch')
            return checks
        if scenario=='settings':
            base=observe('C1-cards-unfocused')
            for _ in range(3):key('Tab')
            selected=observe('C1-selected-appearance-focus')
            card_x=768 if 'dark' in label else 572
            # Interior does not change; measure ring at the straight left edge.
            check('C1-selected-appearance-interior-stable',selected,same(base,selected,f'170x95+{card_x+8}+150'),'Miniature, fill and selection check are untouched')
            colors=[pixel(selected,x,195) for x in range(card_x-6,card_x+3)]
            selection_color=pixel(selected,card_x,195)
            matches=[card_x-6+i for i in range(len(colors)-5) if colors[i]==selection_color and colors[i]==colors[i+1]==colors[i+4]==colors[i+5] and colors[i+2]==colors[i+3] and colors[i]!=colors[i+2]]
            check('C1-full-two-pixel-ring-two-pixel-gap',selected,len(matches)==1,{'x_start':card_x-6,'y':195,'pixels':colors,'ring_start_x':matches})
            focus_color=colors[matches[0]-(card_x-6)]
            key('Left');unselected=observe('C2-unselected-appearance-focus')
            measure=ring_measure(unselected,572 if 'dark' in label else 375,195,focus_color)
            check('C2-full-ring-and-gap',unselected,len(measure['ring_start_x'])==1,measure)
            check('C2-arrow-preserves-interior',unselected,same(selected,unselected,'160x90+585+150') and same(selected,unselected,'160x90+783+150'),'Only ring moves; actual selection remains')
            key('Right')  # Restore selected appearance before leaving its roving stop.
            key('Tab');palette_selected=observe('C3-selected-palette-focus')
            key('Left' if 'sage' in label else 'Right');palette_unselected=observe('C3-unselected-palette-focus')
            for frame,x,which in [(palette_selected,672 if 'sage' in label else 375,'selected'),(palette_unselected,375 if 'sage' in label else 672,'unselected')]:
                measure=ring_measure(frame,x,350,focus_color)
                check('C3-'+which+'-full-ring-and-gap',frame,len(measure['ring_start_x'])==1,measure)
            check('C3-palette-interiors-stable',palette_unselected,same(palette_selected,palette_unselected,'265x100+385+305') and same(palette_selected,palette_unselected,'265x100+682+305'),'Focus leaves both orb previews, label, fill and chosen appearance badge unchanged')
            key('Right' if 'sage' in label else 'Left')
            key('Tab');scale=observe('B4-B5-100-selected-focus')
            key('Right');next_scale=observe('B4-B5-125-focus-only')
            check('B4-B5-selection-distinct-from-focus',next_scale,same(scale,next_scale,'450x230+375+140') and same(scale,next_scale,'36x20+800+449'),'Arrow keeps 100% selected and changes only focus')
            key('Return');enlarged=observe('B4-B5-Enter-125')
            check('B4-B5-Enter-activates',enlarged,not same(scale,enlarged,'260x45+375+45'),'Actual heading geometry grows')
            key('Left');key('space');restored=observe('B4-B5-Space-100')
            check('B4-B5-Space-restores',restored,same(scale,restored,'260x45+375+45'),'Actual heading geometry returns')
            key('Escape');click(240,658);move()
            for _ in range(3):key('Tab')
            modal=observe('B12-gear-modal-focused')
            click(70,420);type_text('MODALCANARY');key('ctrl+k');type_text('BLOCKEDTEXT')
            blocked=expect('B12-background-inert',['Darstellung'],['MODALCANARY','BLOCKEDTEXT'])
            check('B12-no-background-state-change',blocked,same(modal,blocked,'1120x690+0+0'),'Background click, typing and Ctrl+K leave exact native frame unchanged')
            key('Escape');expect('B12-Escape-gear',['Produktteam'],['Darstellung'])
            key('Return');expect('B12-gear-Enter-reopens',['Darstellung'])
            key('Escape');key('space');expect('B12-gear-Space-reopens',['Darstellung'])
            # Reachable ordinary disabled text control (not only the probe kit).
            click(230,170);move()
            for _ in range(3):key('Tab')
            disabled=expect('C5-selected-account-text-reason',['bereits ausgewählt'])
            inert(disabled,'C5-text-no-dispatch')
        elif scenario in ('offline','denied'):
            base=observe('C4-disabled-send-unfocused')
            click(500,589);type_text('ENTWURF');key('Tab');key('Tab')
            focused=expect('C4-disabled-send-reason',['Senden ist derzeit nicht','ENTWURF'])
            check('C4-disabled-send-arrow-distinguishable',focused,pixel(focused,1040,630)!=pixel(base,1052,625),'Drawn surface and arrow are independently measured in native-measurements.json')
            inert(focused,'C4-disabled-send-no-dispatch')
            click(1053,631);observe('C4-disabled-mouse-no-dispatch')
        elif scenario=='controls':
            base=observe('C5-four-style-surfaces')
            key('Tab');disabled=expect('C5-disabled-primary-text-reason',['nicht verfügbar'])
            inert(disabled,'C5-disabled-primary-text-no-dispatch')
            click(202,160);mouse=observe('C5-disabled-text-mouse')
            check('C5-disabled-text-mouse-no-dispatch',mouse,same(disabled,mouse,'1120x690+0+0'),'Mouse leaves the focused unavailable text control unchanged')
            for _ in range(4):key('Tab')
            danger=expect('C5-disabled-danger-text-reason',['nicht verfügbar'])
            inert(danger,'C5-disabled-danger-text-no-dispatch')
            # Same action name, enabled counterpart: no unavailable reason.
            key('Tab');key('Tab');key('Tab')
            enabled=expect('allowed-icon-name-only',absent=['nicht verfügbar'])
            check('allowed-counterpart-has-no-disabled-tip',enabled,'nichtverfugbar' not in text(enabled),'Enabled icon keyboard focus has no unavailable explanation; hover name is captured separately')
        elif scenario=='chat':
            key('Escape');base=observe('B1-unfocused')
            for _ in range(7):key('Tab')
            row=observe('B1-Mara-focused')
            check('B1-row-interior-stable',row,same(base,row,'240x44+19+392'),'Unselected row interior remains identical')
            key('Escape');click(500,589);type_text('ALPHAOMEGA');key('Home')
            for _ in range(5):key('Right')
            draft=observe('B9-draft-before-disabled-focus')
            key('Tab');reason=expect('B9-keyboard-attachment-reason',['derzeit nicht verfügbar'])
            check('R3-06-reason-clear-of-draft',reason,same(draft,reason,'600x22+450+577'),'Draft text/caret strip remains unobscured except caret; measured interior below tip')
            inert(reason,'B9-disabled-Enter-Space')
            move(330,631);time.sleep(1.2);hover=observe('R3-06-same-control-hover-and-focus',False)
            check('R3-06-single-reason-source',hover,same(reason,hover,'780x110+300+435'),'Simultaneous hover and focus reuse one reason view, no second native tooltip')
            click(500,589);key('Home')
            for _ in range(5):key('Right')
            click(330,631);type_text('X');key('End');after=expect('disabled-mouse-preserves-draft',['ALPHAXOMEGA'])
        elif scenario=='focus-rooms':
            key('Escape')
            for _ in range(7):key('Tab')
            observe('R3-07-Mara-before-reorder');key('F6');observe('R3-07-Mara-after-reorder')
            key('Return');selected=observe('R3-07-Enter-same-Mara')
            check('R3-07-reorder-Enter-same-room',selected,log_events()==['UI_FOCUS_PROBE open account=1 room=3'],log_events())
            key('F6');removed=observe('R3-07-Mara-removed');before=list(log_events());key('Return');key('space');p=observe('R3-07-removed-no-redirect')
            check('R3-07-removal-no-redirect',p,log_events()==before,log_events())
            # With Mara removed, Jonas occupies the eighth native tab stop.
            for _ in range(8):key('Tab')
            observe('R3-07-Jonas-before-account-change');key('F6');observe('R3-07-account-replaced');key('Return');key('space');p=observe('R3-07-cross-account-no-redirect')
            check('R3-07-account-change-no-redirect',p,log_events()==before,log_events())
        elif scenario=='focus-files':
            move(640,237);time.sleep(1.2);allowed=observe('allowed-download-tooltip',False)
            words=text(allowed)
            check('allowed-download-name-without-unavailable-reason',allowed,'dateiherunterladen' in words and 'nichtverfugbar' not in words,words)
            move();key('Escape')
            for _ in range(14):key('Tab')
            observe('R3-07-file-before-reorder');key('F6');observe('R3-07-file-after-reorder')
            key('Return');p=observe('R3-07-file-Enter-same-item')
            check('R3-07-file-reorder-Enter-same-id',p,log_events()==['UI_FOCUS_PROBE download account=1 file=1'],log_events())
            # Dismiss feedback by mouse without restoring a retired file handle.
            click(1067,45);key('Escape')
            for _ in range(15):key('Tab')
            observe('R3-07-file-before-removal');key('F6');before=list(log_events());key('Return');key('space');p=observe('R3-07-file-removed-no-redirect')
            check('R3-07-file-removal-no-redirect',p,log_events()==before,log_events())
            for _ in range(14):key('Tab')
            observe('R3-07-file-before-account-change');key('F6');key('Return');key('space');p=observe('R3-07-file-cross-account-no-redirect')
            check('R3-07-file-account-change-no-redirect',p,log_events()==before,log_events())
        return checks
    finally:
        (out/f'{label}-d12-observations.json').write_text(json.dumps({'frames':frames,'checks':checks},indent=2)+'\n')
