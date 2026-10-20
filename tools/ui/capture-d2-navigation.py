#!/usr/bin/env python3
"""Native pure-keyboard D2 navigation probe; never scrolls with the pointer."""
import argparse,importlib.util,json,re,sys,time
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('--build-input',type=Path,required=True);p.add_argument('--output',type=Path,required=True);p.add_argument('--scale',type=int,default=200);p.add_argument('--scenarios',default='settings:light:blue,settings:dark:sage,settings:dark:blue,settings:light:sage');a=p.parse_args();base=a.build_input.resolve();out=a.output.resolve()
sys.path.insert(0,str(base/'source/tools/ui'))
from capture import run,sha
import interactions_d1_2
spec=importlib.util.spec_from_file_location('frozen',base/'source/tools/ui/capture-frozen.py');frozen=importlib.util.module_from_spec(spec);spec.loader.exec_module(frozen)
binding,_=frozen.validate(base)
def exercise(scenario,env,xid,folder,label,large=False):
    frames=[];events=[];checks=[]
    def key(value):run(['xdotool','key','--clearmodifiers',value],env);events.append(value);time.sleep(.22)
    def image(name):
        key('F7');time.sleep(.35);q=folder/f'{label}-d2b-{name}.png';run(['import','-window',xid,str(q)],env);frames.append({'name':name,'image':q.name,'sha256':sha(q),'keys':events.copy()})
        lines=[line for line in (folder/f'{label}.log').read_text().splitlines() if line.startswith('UI_GEOMETRY')]
        frames[-1]['geometry']=lines[-1:]
        if name!='escape-returns-invoker':
            m=re.search(r'focus=Some\(Bounds \{ origin: Point \{ x: ([-\d.]+)px, y: ([-\d.]+)px \}, size: Size \{ ([-\d.]+)px × ([-\d.]+)px',lines[-1] if lines else '')
            b=tuple(map(float,m.groups())) if m else None
            valid=b and b[0]>=0 and b[1]>=0 and b[0]+b[2]<=(720 if a.scale==200 else 1120) and b[1]+b[3]<=(554 if a.scale==200 else 690)
            checks.append({'check':name+'-native-focus-visible','result':'pass' if valid else 'FAIL','bounds':b})
            if not valid:raise AssertionError(checks[-1])
    run(['xdotool','mousemove','--window',xid,'710','570'],env)
    # D2 opens with the selected appearance card visibly focused.
    image('appearance-selected-visible')
    key('Down');image('appearance-unselected-visible')
    key('Tab');image('tab-leaves-appearance-for-palette')
    key('Left');image('palette-unselected-visible')
    key('Tab');image('tab-leaves-palette-for-scale')
    key('Left');image('scale-unselected-visible')
    key('shift+Tab');image('backtab-leaves-scale-for-palette')
    key('Up');key('shift+Tab');image('backtab-leaves-palette-for-appearance')
    key('Escape');image('escape-returns-invoker')
    (folder/f'{label}-d2b-observations.json').write_text(json.dumps({'frames':frames,'pointer_wheel_events':0,'keyboard_only':True,'checks':checks,'assertion_status':'native focus bounds checked; visual interiors require separate inspection'},indent=2)+'\n')
    return checks
interactions_d1_2.exercise=exercise
sys.argv=['capture-frozen.py','--build-input',str(base),'--output',str(out),'--width','720' if a.scale==200 else '1120','--height','580' if a.scale==200 else '716','--scale',str(a.scale),'--scenarios',a.scenarios,'--interactions','--interaction-suite','d1-2']
try:frozen.main()
finally:
    frozen.validate(base)
    (out/'d2b-recipe-binding.json').write_text(json.dumps({'recipe_sha256':sha(__file__),'compiled_input':binding,'external_recipe':str(Path(__file__).name),'keyboard_only':True},indent=2)+'\n')
