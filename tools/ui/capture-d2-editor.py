#!/usr/bin/env python3
"""Exercise the compiled native multiline editor with real keyboard events."""
import argparse, importlib.util, json, sys, time
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('--build-input',type=Path,required=True);p.add_argument('--output',type=Path,required=True);p.add_argument('--scale',type=int,default=100);a=p.parse_args();base=a.build_input.resolve();out=a.output.resolve()
sys.path.insert(0,str(base/'source/tools/ui'))
from capture import run,sha
import interactions_d1_2
spec=importlib.util.spec_from_file_location('frozen',base/'source/tools/ui/capture-frozen.py');frozen=importlib.util.module_from_spec(spec);spec.loader.exec_module(frozen)
binding,_=frozen.validate(base)
def exercise(scenario,env,xid,folder,label,large=False):
    events=[];frames=[]
    def key(value):run(['xdotool','key','--clearmodifiers',value],env);events.append({'key':value});time.sleep(.16)
    def type_text(value):run(['xdotool','type','--clearmodifiers','--delay','8',value],env);events.append({'text':value});time.sleep(.15)
    def image(name):
        time.sleep(.3);q=folder/f'{label}-editor-{name}.png';run(['import','-window',xid,str(q)],env);frames.append({'name':name,'image':q.name,'sha256':sha(q),'events':events.copy()})
    run(['xdotool','mousemove','--window',xid,'500','588','click','1'],env)
    type_text('ALPHA');key('shift+Return');type_text('BRAVO');key('shift+Return');type_text('CHARLIE');image('three-lines')
    key('Up');key('Home');type_text('X');image('vertical-home-edit')
    key('ctrl+comma');image('modal-over-draft');key('Escape');type_text('Y');image('modal-restores-caret')
    key('Return');image('send-retains-draft')
    key('ctrl+End');key('shift+Return');type_text('DELTA');image('failure-retains-editing')
    for n in range(6):key('shift+Return');type_text('LINE'+str(n))
    image('bounded-visible-lines-scroll');key('ctrl+Home');image('keyboard-scroll-to-start')
    (folder/f'{label}-editor-observations.json').write_text(json.dumps({'frames':frames,'keyboard_events':events,'native_editor':True,'assertion_status':'author visual and intent-log inspection required'},indent=2)+'\n')
    return []
interactions_d1_2.exercise=exercise
sys.argv=['capture-frozen.py','--build-input',str(base),'--output',str(out),'--width','1120','--height','716','--scale',str(a.scale),'--scenarios','chat:light:blue,busy:dark:sage','--interactions','--interaction-suite','d1-2']
try:frozen.main()
finally:
    frozen.validate(base)
    (out/'editor-recipe-binding.json').write_text(json.dumps({'recipe_sha256':sha(__file__),'compiled_input':binding,'external_recipe':str(Path(__file__).name)},indent=2)+'\n')
