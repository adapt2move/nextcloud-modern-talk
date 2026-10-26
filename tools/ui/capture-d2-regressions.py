#!/usr/bin/env python3
"""Native D2 scoped-focus, modal, clipboard and call-control regressions (VM only)."""
import argparse, importlib.util, json, re, subprocess, sys, time
from pathlib import Path

p=argparse.ArgumentParser(description=__doc__)
p.add_argument('--build-input',type=Path,required=True)
p.add_argument('--output',type=Path,required=True)
p.add_argument('--scenarios',default='chat:light:blue,chat:dark:sage,focus-rooms:light:blue,focus-files:light:blue,clipboard-denied:dark:sage,call-controls:light:blue,call-denied:dark:sage')
a=p.parse_args();base=a.build_input.resolve();out=a.output.resolve()
sys.path.insert(0,str(base/'source/tools/ui'))
from capture import run,sha
import interactions_d1_2
spec=importlib.util.spec_from_file_location('frozen',base/'source/tools/ui/capture-frozen.py');frozen=importlib.util.module_from_spec(spec);spec.loader.exec_module(frozen)
binding,_=frozen.validate(base)

def exercise(scenario,env,xid,folder,label,large=False):
    events=[];frames=[];checks=[]
    def key(k,n=1):
        for _ in range(n):run(['xdotool','key','--clearmodifiers',k],env);events.append({'key':k});time.sleep(.22)
    def move(x,y):run(['xdotool','mousemove','--window',xid,str(x),str(y)],env);events.append({'pointer':[x,y]});time.sleep(.7)
    def click(x,y):move(x,y);run(['xdotool','click','1'],env);events.append({'click':1});time.sleep(.25)
    def text(s):run(['xdotool','type','--clearmodifiers','--delay','5',s],env);events.append({'synthetic_text':s});time.sleep(.2)
    def log(prefix):return [l for l in (folder/f'{label}.log').read_text().splitlines() if l.startswith(prefix)]
    def image(name):
        key('F7');q=folder/f'{label}-d2r-{name}.png';run(['import','-window',xid,str(q)],env)
        frames.append({'name':name,'image':q.name,'sha256':sha(q),'events':events.copy(),'dispatch':log('UI_DISPATCH'),'intent':log('UI_INTENT')+log('UI_FOCUS_PROBE'),'geometry':log('UI_GEOMETRY')[-1:]});return q
    def check(name,condition,detail):
        checks.append({'check':name,'result':'pass' if condition else 'FAIL','detail':detail})
        if not condition:raise AssertionError(checks[-1])
    def draft():
        key('ctrl+a');key('ctrl+c');value=run(['xclip','-selection','clipboard','-o'],env).stdout;events.append({'observed_synthetic_draft':value});key('End');return value
    try:
        move(1100,705)
        if scenario=='chat':
            image('baseline');key('Tab',7);image('unselected-Mara-keyboard');move(145,527);image('Jonas-hover-selection-retained')
            click(480,588);text('ALPHAOMEGA');key('Home');key('Right',5);key('Tab');image('attachment-focus-above-card')
            move(331,630);image('attachment-focus-hover-700ms');key('shift+Tab');click(331,630);text('X');image('disabled-click-retains-caret')
            check('disabled-click-keeps-caret',draft()=='ALPHAXOMEGA','native synthetic clipboard content')
            key('Home');key('Right',6);key('ctrl+comma');modal=image('modal-from-composer')
            click(70,400);text('MODALCANARY');key('ctrl+k');blocked=image('modal-background-inert')
            check('modal-background-pixels-inert',sha(modal)==sha(blocked),'same full native frame after scrim click, typing and Ctrl+K')
            key('Escape');text('Y');image('modal-returns-composer-caret')
            check('modal-invoker-caret-restored',draft()=='ALPHAXYOMEGA','Escape returns actual composer invoker')
            click(240,658);image('gear-opens-modal');key('Escape');image('Escape-returns-gear')
            key('Return');image('gear-Enter-opens');key('Escape');key('space');image('gear-Space-opens');key('Escape')
            check('local-navigation-no-port-dispatch',not log('UI_DISPATCH'),log('UI_DISPATCH'))
        elif scenario=='focus-rooms':
            key('Tab',7);image('Mara-focused');key('F6');image('Mara-reordered');key('Return');image('Mara-opened')
            check('room-reorder-same-target-once',log('UI_FOCUS_PROBE open')==['UI_FOCUS_PROBE open account=1 room=3'],log('UI_FOCUS_PROBE open'))
            key('F6');image('Mara-removed');before=log('UI_DISPATCH');key('Return');key('space');check('removed-room-no-redirect',log('UI_DISPATCH')==before,before)
            key('Tab',7);image('next-room-focused');key('F6');image('account-replaced');before=log('UI_DISPATCH');key('Return');key('space');check('room-account-replacement-no-redirect',log('UI_DISPATCH')==before,before)
        elif scenario=='focus-files':
            key('Tab',15);image('file-download-focused');key('F6');image('file-reordered');key('Return');image('file-download-result')
            check('file-reorder-same-target-once',log('UI_FOCUS_PROBE download')==['UI_FOCUS_PROBE download account=1 file=1'],log('UI_FOCUS_PROBE download'))
            key('Escape');image('feedback-dismiss-restores-file-focus')
            key('F6');image('file-removed');before=log('UI_DISPATCH');key('Return');key('space');check('removed-file-no-redirect',log('UI_DISPATCH')==before,before)
            key('F6');image('file-account-replaced');before=log('UI_DISPATCH');key('Return');key('space');check('file-account-replacement-no-redirect',log('UI_DISPATCH')==before,before)
        elif scenario=='clipboard-denied':
            clipboard=folder/f'{label}-synthetic.txt';clipboard.write_text('BEFORE')
            owner=subprocess.Popen(['xclip','-selection','clipboard','-i',str(clipboard),'-loops','0'],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
            try:
                click(480,588);text('ALPHA');key('ctrl+a');key('ctrl+c');key('ctrl+x');key('ctrl+v');image('privacy-blocks-export-allows-paste')
                check('unknown-privacy-preserves-clipboard',run(['xclip','-selection','clipboard','-o'],env).stdout=='BEFORE','synthetic clipboard owner')
                key('End');text('Z');image('imported-draft-editable-after-denied-cut')
                check('privacy-no-port-dispatch',not log('UI_DISPATCH'),log('UI_DISPATCH'))
            finally:owner.terminate();owner.wait(timeout=3)
        else:
            # Wide call chat precedes the stage controls: composer, attachment,
            # send, then microphone. Do not label the composer as a mic probe.
            key('Tab',4);image('microphone-keyboard')
            match=re.search(r'focus=Some\(Bounds \{ origin: Point \{ x: ([-\d.]+)px, y: ([-\d.]+)px \}, size: Size \{ ([-\d.]+)px × ([-\d.]+)px',frames[-1]['geometry'][0]);x,y,w,h=map(float,match.groups())
            move(int(x+w/2),int(y+h/2));image('microphone-focus-hover-700ms')
            before=log('UI_DISPATCH');key('Return');key('space');image('microphone-Enter-Space')
            if scenario=='call-denied':check('disabled-media-no-dispatch',log('UI_DISPATCH')==before,log('UI_DISPATCH'))
            else:check('active-media-one-intent-per-key',len(log('UI_INTENT microphone'))==2,log('UI_INTENT microphone'))
            key('Tab');image('camera-keyboard');key('Tab');image('share-keyboard');key('Tab');key('Return');image('call-participants-modal');key('Escape')
            key('Tab');key('Return');image('call-chat-toggle');before=log('UI_DISPATCH');key('Escape');check('call-Escape-never-leaves',log('UI_DISPATCH')==before,before)
        return checks
    finally:(folder/f'{label}-d2r-observations.json').write_text(json.dumps({'frames':frames,'checks':checks,'pointer_hover_wait_ms':700,'pointer_wheel_events':0},indent=2)+'\n')

interactions_d1_2.exercise=exercise
sys.argv=['capture-frozen.py','--build-input',str(base),'--output',str(out),'--width','1120','--height','716','--scenarios',a.scenarios,'--interactions','--interaction-suite','d1-2']
try:frozen.main()
finally:
    frozen.validate(base);(out/'regression-recipe-binding.json').write_text(json.dumps({'recipe_sha256':sha(__file__),'compiled_input':binding},indent=2)+'\n')
