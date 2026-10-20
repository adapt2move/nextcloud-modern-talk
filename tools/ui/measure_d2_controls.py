#!/usr/bin/env python3
"""D2-A actual glyph/surface measurements. VM only; leaves original PNGs untouched."""
import argparse,json
from pathlib import Path
from measure_d1_2 import control,sha
p=argparse.ArgumentParser();p.add_argument('--before',type=Path,required=True);p.add_argument('--after',type=Path,required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args()
r={'recipe_sha256':sha(__file__),'active_styles':[],'validating':[],'before_danger':[]}
for mode,theme in [('light','blue'),('dark','sage')]:
    for style,y in [('Primary',160),('Danger',264),('Secondary',368),('Quiet',472)]:
        file=a.after/f'controls-{mode}-{theme}.png'
        text=control(file,(333,y-12,4,4),(342,y-8,76,18));icon=control(file,(450,y-12,3,3),(454,y-8,18,18))
        r['active_styles'].append({'mode':mode,'theme':theme,'style':style,'text':text,'icon':icon,'both_readable':min(text['contrast'],icon['contrast'])>=4.5,'same_foreground':max(abs(x-y) for x,y in zip(text['strongest_glyph'],icon['strongest_glyph']))<=3})
    r['before_danger'].append(control(a.before/f'controls-{mode}-{theme}.png',(450,252,3,3),(454,256,18,18)))
    file=a.after/f'validating-{mode}-{theme}.png'
    r['validating'].append({'mode':mode,'theme':theme,'label':control(file,(523,446,4,4),(528,451,70,20)),'arrow':control(file,(898,447,3,3),(901,452,18,18))})
r['all_active_readable_and_consistent']=all(x['both_readable'] and x['same_foreground'] for x in r['active_styles'])
r['all_validating_in_unavailable_target']=all(2.0<=x[k]['contrast']<=3.6 for x in r['validating'] for k in ['label','arrow'])
a.output.write_text(json.dumps(r,indent=2)+'\n');print(json.dumps({k:v for k,v in r.items() if k.startswith('all_')}))
if not r['all_active_readable_and_consistent'] or not r['all_validating_in_unavailable_target']:raise SystemExit(1)
