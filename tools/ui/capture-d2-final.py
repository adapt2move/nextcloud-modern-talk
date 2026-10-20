#!/usr/bin/env python3
"""Source-bound D2 native interactions. Run only on the assigned Linux VM.

No application-state injection: keyboard, pointer and synthetic clipboard input.
Raw OCR is retained separately from full-copy/placement author inspection.
"""
import argparse, importlib.util, json, re, subprocess, sys, time
from pathlib import Path

p=argparse.ArgumentParser(description=__doc__)
p.add_argument('--build-input',type=Path,required=True)
p.add_argument('--output',type=Path,required=True)
p.add_argument('--batch',choices=['controls','editor','accounts','calls','surfaces','narrow'],required=True)
p.add_argument('--scale',type=int,default=100)
a=p.parse_args();base=a.build_input.resolve();out=a.output.resolve()
sys.path.insert(0,str(base/'source/tools/ui'))
from capture import run,sha
import interactions_d1_2
spec=importlib.util.spec_from_file_location('frozen',base/'source/tools/ui/capture-frozen.py')
frozen=importlib.util.module_from_spec(spec);spec.loader.exec_module(frozen)
binding,_=frozen.validate(base)

def exercise(scenario,env,xid,folder,label,large=False):
    events=[];frames=[];checks=[]
    def key(value,n=1):
        for _ in range(n):
            run(['xdotool','key','--clearmodifiers',value],env);events.append({'key':value});time.sleep(.18)
    def move(x,y):
        run(['xdotool','mousemove','--window',xid,str(x),str(y)],env);events.append({'pointer':[x,y]});time.sleep(.7)
    def click(x,y):
        move(x,y);run(['xdotool','click','1'],env);events.append({'click':1});time.sleep(.2)
    def text(value):
        run(['xdotool','type','--clearmodifiers','--delay','6',value],env);events.append({'typed_synthetic':value});time.sleep(.2)
    def dispatch():
        return [l for l in (folder/f'{label}.log').read_text().splitlines() if l.startswith(('UI_DISPATCH','UI_INTENT','UI_FOCUS_PROBE'))]
    def image(name,expected=None):
        key('F7');time.sleep(.3);q=folder/f'{label}-d2-{name}.png';run(['import','-window',xid,str(q)],env)
        raw=run(['tesseract',str(q),'stdout','-l','deu+eng','--psm','11'],env).stdout
        (folder/f'{q.stem}-raw-ocr.txt').write_text(raw)
        frames.append({'name':name,'image':q.name,'sha256':sha(q),'events':events.copy(),'dispatch':dispatch(),'geometry': [l for l in (folder/f'{label}.log').read_text().splitlines() if l.startswith('UI_GEOMETRY')][-1:],'raw_ocr':raw,'expected_full_copy_for_visual_inspection':expected})
        if name.startswith('keyboard-') and name not in ['keyboard-navigation-closed']:
            geometry=frames[-1]['geometry']
            focus=native_focus(geometry[-1] if geometry else '')
            check(name+'-native-focus-in-viewport',focus is not None and focus[0]>=0 and focus[1]>=0 and focus[0]+focus[2]<=720 and focus[1]+focus[3]<=554,focus)
        return q
    def native_focus(line):
        match=re.search(r'focus=Some\(Bounds \{ origin: Point \{ x: ([-\d.]+)px, y: ([-\d.]+)px \}, size: Size \{ ([-\d.]+)px × ([-\d.]+)px',line)
        return tuple(map(float,match.groups())) if match else None
    def check(name,condition,detail):
        checks.append({'check':name,'result':'pass' if condition else 'FAIL','detail':detail})
        if not condition:raise AssertionError(f'{name}: {detail}')
    def copy_draft():
        key('ctrl+a');key('ctrl+c');value=run(['xclip','-selection','clipboard','-o'],env).stdout;events.append({'observed_synthetic_draft':value});key('End');return value
    def paste(value):
        path=folder/f'{label}-synthetic-clipboard.txt';path.write_text(value)
        owner=subprocess.Popen(['xclip','-selection','clipboard','-i',str(path),'-loops','1'],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        try:time.sleep(.15);key('ctrl+v');time.sleep(.15)
        finally:
            if owner.poll() is None:owner.terminate()
            owner.wait(timeout=2)
    try:
        move(1100 if a.scale<150 else 710,705 if a.scale<150 else 570)
        if a.batch=='controls':
            if scenario=='chat':
                move(240,88);image('plus-hover-only-700ms','Neues Gespräch')
                move(1100,705);key('Tab',2);image('plus-keyboard','Neues Gespräch')
                move(240,88);image('plus-focus-hover-700ms','Neues Gespräch')
                key('Return');image('plus-local-preview','Neues Gespräch')
                check('local-create-preview-no-server-dispatch',not dispatch(),dispatch())
            elif scenario=='offline':
                move(1020,50);image('offline-call-hover-only-700ms','Anruf beitreten\nDu bist offline. Anrufe sind erst nach der Verbindung verfügbar.')
                move(1100,705);key('Tab',13);before=image('offline-call-focus','Anruf beitreten\nDu bist offline. Anrufe sind erst nach der Verbindung verfügbar.')
                move(1020,50);image('offline-call-focus-hover-700ms')
                before_dispatch=dispatch();key('Return');key('space');click(1020,50);after=image('offline-call-disabled-click-enter-space')
                check('disabled-call-no-port-dispatch',dispatch()==before_dispatch,dispatch())
                check('disabled-call-state-unchanged',sha(before)==sha(after),'untouched full-window hashes, separate from direct dispatch assertion')
            elif scenario=='validating':
                image('validating-persistent-status','Die Serveradresse wird geprüft. Bitte warte.')
                key('Tab',2);image('validating-keyboard-status','Die Serveradresse wird geprüft. Bitte warte.')
                focus=native_focus(frames[-1]['geometry'][-1]);check('validating-focus-measured',focus is not None,focus)
                x,y=int(focus[0]+focus[2]/2),int(focus[1]+focus[3]/2)
                move(x,y);image('validating-focus-hover-700ms')
                before_dispatch=dispatch();key('Return');key('space');click(x,y);image('validating-disabled-click-enter-space')
                check('disabled-validating-no-port-dispatch',dispatch()==before_dispatch,dispatch())
            elif scenario=='controls':
                image('style-matrix')
                move(198,160);image('disabled-text-hover-only-700ms','In diesem UI-Beispiel nicht verfügbar.');move(1100,705)
                key('Tab');image('disabled-text-reason','In diesem UI-Beispiel nicht verfügbar.')
                before_dispatch=dispatch();key('Return');key('space');click(198,160);image('disabled-text-click-enter-space')
                check('disabled-text-no-port-dispatch',dispatch()==before_dispatch,dispatch())
                move(1100,705);key('Tab',7);image('active-danger-keyboard','Fortfahren')
        elif a.batch=='editor':
            click(480,588);text('ALPHA');key('shift+Return');text('BRAVO');key('shift+Return');text('CHARLIE');image('multiline')
            key('Up');key('Home');text('X');key('ctrl+comma');image('modal-visible-initial-focus');key('Escape');text('Y');image('modal-restores-caret')
            check('modal-caret-and-multiline-retained',copy_draft()=='ALPHA\nXYBRAVO\nCHARLIE','native clipboard observation of synthetic draft')
            key('ctrl+z');check('undo-restores-edit',copy_draft()=='ALPHA\nXBRAVO\nCHARLIE','Ctrl+Z after modal/copy')
            key('ctrl+shift+z');check('redo-restores-edit',copy_draft()=='ALPHA\nXYBRAVO\nCHARLIE','Ctrl+Shift+Z')
            key('Return');image('send-result-retains-draft')
            check('send-keeps-draft',copy_draft()=='ALPHA\nXYBRAVO\nCHARLIE',scenario)
            sends=[l for l in dispatch() if l.startswith('UI_INTENT send')]
            check('exactly-one-send-per-Enter',len(sends)==1,sends)
            if scenario in ['pending','uncertain']:
                key('Return');check('pending-uncertain-no-blind-resend',len([l for l in dispatch() if l.startswith('UI_INTENT send')])==1,dispatch())
            key('ctrl+a');paste('Grüße 👋\nZeile zwei');image('utf8-paste-and-native-selection')
            check('unicode-multiline-paste',copy_draft()=='Grüße 👋\nZeile zwei','full UTF-8 content through native clipboard')
            key('ctrl+Home');key('shift+End');text('NEU');image('replace-selected-line')
            check('selection-replaces-only-line',copy_draft()=='NEU\nZeile zwei','selection and newline retained')
            paste('x'*4097);image('oversize-paste-rejected')
            check('oversize-retains-draft',copy_draft()=='NEU\nZeile zwei','4097-byte input rejected before editor mutation')
        elif a.batch=='accounts':
            # Initial close -> navigation -> current row -> second live account.
            key('Tab',4);image('live-account-before-reorder')
            key('F6');image('live-account-focus-follows-reorder')
            key('Return');image('select-same-scoped-account')
            selects=[l for l in dispatch() if l.startswith('UI_FOCUS_PROBE select')]
            check('reorder-Enter-selects-same-account-once',selects==['UI_FOCUS_PROBE select account=2 runtime=1'],selects)
            key('F6');image('removed-account-focus-retired');before=dispatch();key('Return');key('space')
            check('removed-account-no-retarget',dispatch()==before,dispatch())
            key('F6');image('new-runtime-no-old-focus');before=dispatch();key('Return');key('space')
            check('changed-runtime-no-stale-action',dispatch()==before,dispatch())
        elif a.batch=='calls':
            before=dispatch();key('Return');image('unrelated-Enter-does-not-act')
            if scenario=='incoming':
                check('incoming-Enter-does-not-accept',dispatch()==before,dispatch())
                key('Tab',2);image('incoming-accept-focus');key('Return');image('accepted-direct-connecting')
                check('direct-accept-once',len([l for l in dispatch() if l.startswith('UI_INTENT accept-direct')])==1,dispatch())
                check('direct-no-conference-join',not any('join-conference' in l for l in dispatch()),dispatch())
                before=dispatch();key('Escape');image('Escape-does-not-leave');check('Escape-never-leaves',dispatch()==before,dispatch())
            elif scenario in ['chat','voice-room']:
                key('Tab',13);key('Return');image('deliberate-prejoin')
                check('opening-prejoin-does-not-join',not any('UI_INTENT join-conference' in l for l in dispatch()),dispatch())
                key('Tab',7);image('join-prepared-focus');key('Return');image('conference-connecting')
                joins=[l for l in dispatch() if l.startswith('UI_INTENT join-conference')]
                expected='UI_INTENT join-conference room='+('2' if scenario=='voice-room' else '1')
                check('deliberate-join-correct-context-once',joins==[expected],joins)
                before=dispatch();key('Escape');check('Escape-never-leaves',dispatch()==before,dispatch())
            else:
                before=dispatch();key('Escape');image('Escape-keeps-call');check('Escape-never-leaves',dispatch()==before,dispatch())
                move(735,614);image('leave-hover-700ms')
                click(735,614);image('explicit-leave')
                check('priority-leave-once',len([l for l in dispatch() if l.startswith('UI_INTENT leave')])==1,dispatch())
        elif a.batch=='narrow':
            image('large-text-initial')
            if scenario in ['chat','long-labels']:
                key('Tab');key('Return');image('keyboard-open-navigation')
                key('Tab',7);image('keyboard-row-focus-visible');key('Escape');image('keyboard-navigation-closed')
                key('Tab',6);image('keyboard-composer-action-revealed')
            elif scenario.startswith('prejoin'):
                key('Tab',7);image('keyboard-join-visible');before=dispatch();key('Escape');image('prejoin-cancelled');check('prejoin-cancel-no-join',dispatch()==before,dispatch())
            elif scenario=='create35':
                key('Tab',6);image('keyboard-create-name-visible');key('Tab');image('keyboard-create-disabled-next')
            elif scenario=='storage':
                key('Tab',3);image('keyboard-storage-option-visible');key('Tab');image('keyboard-storage-cancel-visible')
            elif scenario=='call-denied':
                key('Tab');image('keyboard-disabled-call-control');key('Tab',5);image('keyboard-priority-leave-visible')
        elif a.batch=='surfaces':
            image('surface-initial')
            if scenario=='search':
                key('ctrl+a');text('Produktteam');image('local-loaded-search');key('Tab',3);image('local-search-result-focus')
            elif scenario=='create35':
                key('Tab',6);text('Planung');image('bounded-create-draft');key('Tab');image('unavailable-create-reason');before=dispatch();key('Return');key('space');check('create-unknown-no-dispatch',dispatch()==before,dispatch())
            elif scenario=='thread':
                key('Tab');image('unavailable-thread-reply');before=dispatch();key('Return');key('space');check('unavailable-thread-no-dispatch',dispatch()==before,dispatch())
            key('Escape');image('Escape-restores-workspace')
        return checks
    finally:
        (folder/f'{label}-d2-observations.json').write_text(json.dumps({'batch':a.batch,'frames':frames,'checks':checks,'pointer_hover_wait_ms':700,'pointer_wheel_events':0},indent=2)+'\n')

interactions_d1_2.exercise=exercise
scenarios={
 'controls':'chat:light:blue,chat:dark:sage,offline:light:blue,offline:dark:sage,validating:light:blue,validating:dark:sage,controls:light:blue,controls:dark:sage,controls:light:sage,controls:dark:blue,validating:light:sage,validating:dark:blue',
 'editor':'chat:light:blue,busy:dark:sage,uncertain:light:sage,pending:dark:blue',
 'accounts':'focus-accounts:light:blue',
 'calls':'incoming:light:blue,chat:light:blue,voice-room:dark:sage,call-controls:dark:sage',
 'narrow':'chat:light:blue,long-labels:dark:sage,prejoin:light:blue,storage:dark:sage,create35:dark:sage,call-denied:dark:sage',
 'surfaces':'search:light:blue,create35:dark:sage,thread:dark:blue,details:light:sage,discover:light:blue',
}[a.batch]
sys.argv=['capture-frozen.py','--build-input',str(base),'--output',str(out),'--width','720' if a.scale>=150 else '1120','--height','580' if a.scale>=150 else '716','--scale',str(a.scale),'--scenarios',scenarios,'--interactions','--interaction-suite','d1-2']
try:frozen.main()
finally:
    frozen.validate(base)
    (out/'d2-final-recipe-binding.json').write_text(json.dumps({'recipe_sha256':sha(__file__),'batch':a.batch,'compiled_input':binding,'unchanged_D2_A_before_has_no_validating_fixture':True,'historical_binding_not_modified':True},indent=2)+'\n')
