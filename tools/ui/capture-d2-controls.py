#!/usr/bin/env python3
"""D2-A native helper milestone. Real keys/hover, no injected application state.

--before captures the unchanged D1.2 executable; it has no Validating scenario.
This separate recipe is explicitly hashed alongside the compiled-input binding.
"""
import argparse, importlib.util, json, os, sys, time
from pathlib import Path

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--build-input',type=Path,required=True);p.add_argument('--output',type=Path,required=True);p.add_argument('--before',action='store_true')
    a=p.parse_args();base=a.build_input.resolve();out=a.output.resolve()
    sys.path.insert(0,str(base/'source/tools/ui'))
    from capture import run,sha
    import interactions_d1_2
    from interactions import normalized
    spec=importlib.util.spec_from_file_location('frozen_capture',base/'source/tools/ui/capture-frozen.py');frozen=importlib.util.module_from_spec(spec);spec.loader.exec_module(frozen)
    binding,_=frozen.validate(base)
    def exercise(scenario,env,xid,folder,label,large=False):
        frames=[];checks=[]
        def move(x=1100,y=705):run(['xdotool','mousemove','--window',xid,str(x),str(y)],env)
        def key(k):run(['xdotool','key','--clearmodifiers',k],env);time.sleep(.17)
        def image(name):
            time.sleep(.35);q=folder/f'{label}-d2a-{name}.png';run(['import','-window',xid,str(q)],env);frames.append({'name':name,'image':q.name,'sha256':sha(q)});return q
        def text(q, region=None):
            if region:
                # DERIVED OCR crop only; the native original is never changed.
                crop=q.with_name(q.stem+'-derived-ocr.png')
                run(['convert',str(q),'-crop',region,'+repage','-resize','200%',str(crop)],env)
                return normalized(run(['tesseract',str(crop),'stdout','--psm','6'],env).stdout)
            return normalized(run(['tesseract',str(q),'stdout','--psm','11'],env).stdout)
        def check(name,q,condition,detail):
            checks.append({'check':name,'result':'pass' if condition else 'FAIL','image':q.name,'sha256':sha(q),'observation':detail})
            if not condition:raise AssertionError(f'{name}: {detail}')
        def no_dispatch(q):
            key('Return');key('space');r=image('disabled-no-dispatch')
            check('disabled-native-Enter-Space-inert',r,sha(q)==sha(r),'Original complete window pixels identical after Enter and Space')
        try:
            move()
            if scenario=='chat':
                # Initial root -> account switcher -> new conversation.
                key('Tab');key('Tab');q=image('new-conversation-keyboard')
                if not a.before:check('disabled-plus-name-and-reason',q,'neues' in text(q) and 'derzeit' in text(q),text(q))
                no_dispatch(q)
                move(240,88);r=image('new-conversation-focus-hover')
                check('one-reason-focus-plus-hover',r,sha(q)==sha(r),'Focus and simultaneous hover draw one identical explanation')
            elif scenario=='offline':
                # Root through bounded sidebar into first conversation-header action.
                for _ in range(12):key('Tab')
                q=image('offline-call-keyboard')
                if not a.before:check('disabled-call-name-and-reason',q,'anruf' in text(q) and 'derzeit' in text(q),text(q))
                no_dispatch(q)
            elif scenario=='validating':
                key('Tab');key('Tab');q=image('validating-continue-keyboard')
                reason=text(q,'430x45+505+390')
                check('validating-truthful-reason',q,'serveradresse' in reason and 'bitte warte' in reason,reason);no_dispatch(q)
            elif scenario=='controls':
                q=image('four-style-native-colors')
                for _ in range(8):key('Tab')
                image('active-danger-keyboard')
            return checks
        finally:(folder/f'{label}-d2a-observations.json').write_text(json.dumps({'frames':frames,'checks':checks},indent=2)+'\n')
    interactions_d1_2.exercise=exercise
    scenarios='chat:light:blue,chat:dark:sage,offline:light:blue,offline:dark:sage,controls:light:blue,controls:dark:sage'
    if not a.before:scenarios+=',validating:light:blue,validating:dark:sage'
    sys.argv=['capture-frozen.py','--build-input',str(base),'--output',str(out),'--width','1120','--height','716','--scenarios',scenarios,'--interactions','--interaction-suite','d1-2']
    try:frozen.main()
    finally:
        frozen.validate(base)
        (out/'d2a-recipe-binding.json').write_text(json.dumps({'recipe_sha256':sha(__file__),'kind':'D2-A before' if a.before else 'D2-A corrected author milestone','compiled_input':binding,'interaction_override':'This exact separate D2-A recipe replaces exercise; no source/binary mutation','before_has_no_validating_fixture':a.before},indent=2)+'\n')
if __name__=='__main__':main()
