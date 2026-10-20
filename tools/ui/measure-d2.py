#!/usr/bin/env python3
"""Assert semantic roles on actual D2 native pixels; keep original images unchanged."""
import argparse,json,re
from pathlib import Path
from measure_d1_2 import control,rgb,sha
p=argparse.ArgumentParser(description=__doc__);p.add_argument('--captures',type=Path,required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args()
checks=[];measurements=[]
def check(name,condition,detail):checks.append({'check':name,'result':'pass' if condition else 'FAIL','detail':detail})
def color(value):return tuple(bytes.fromhex(value))
def near(actual,wanted):return max(abs(x-y) for x,y in zip(actual,color(wanted)))<=3
for mode,theme,muted,quiet,danger in [('light','blue','596b85','4a5b73','9e3e32'),('light','sage','566358','4c584e','9e3e32'),('dark','blue','b1c1d6','b1c1d6','ffb8a9'),('dark','sage','b3c1b8','b3c1b8','ffb8a9')]:
    gallery=a.captures/f'controls-{mode}-{theme}.png'
    for style,y,role in [('Primary',160,'ffffff'),('Danger',264,danger),('Secondary',368,muted),('Quiet',472,quiet)]:
        actual=control(gallery,(450,y-12,3,3),(454,y-8,18,18));measurements.append(actual)
        check(f'{mode}-{theme}-{style}-active-icon-role',near(actual['strongest_glyph'],role),actual)
        check(f'{mode}-{theme}-{style}-active-icon-contrast',actual['contrast']>=4.5,actual)
        for kind,bg,glyph in [('text',(159,y-13,5,5),(165,y-8,77,18)),('icon',(276,y-12,4,4),(280,y-8,18,18))]:
            actual=control(gallery,bg,glyph);measurements.append(actual)
            check(f'{mode}-{theme}-{style}-{kind}-unavailable-contrast',2.0<=actual['contrast']<=3.6,actual)
    observations=json.loads((a.captures/f'validating-{mode}-{theme}-d2-observations.json').read_text())
    frame=next(f for f in observations['frames'] if f['name']=='validating-keyboard-status')
    match=re.search(r'focus=Some\(Bounds \{ origin: Point \{ x: ([-\d.]+)px, y: ([-\d.]+)px',frame['geometry'][0]);x,y=map(float,match.groups());y=int(y+4)
    image=a.captures/f'validating-{mode}-{theme}.png'
    label=control(image,(523,y+8,4,4),(528,y+13,70,20));arrow=control(image,(898,y+9,3,3),(901,y+14,18,18));measurements.extend([label,arrow])
    check(f'{mode}-{theme}-validating-label-arrow-consistent',max(abs(x-y) for x,y in zip(label['strongest_glyph'],arrow['strongest_glyph']))<=3,{'label':label,'arrow':arrow})
    check(f'{mode}-{theme}-validating-unavailable-contrast',all(2.0<=m['contrast']<=3.6 for m in [label,arrow]),{'label':label,'arrow':arrow})
for mode,theme,focus in [('light','blue','356fc1'),('dark','sage','b7d7bf')]:
    baseline=a.captures/f'offline-{mode}-{theme}.png';focused=a.captures/f'offline-{mode}-{theme}-d2-offline-call-focus.png'
    for name,rectangle in [('own-icon',(1008,38,24,24)),('details-neighbor',(1050,33,34,34)),('title',(351,29,300,42)),('offline-status',(303,107,780,46))]:
        check(f'{mode}-{theme}-reason-preserves-{name}',rgb(baseline,rectangle)==rgb(focused,rectangle),{'baseline':sha(baseline),'focused':sha(focused),'rect':rectangle})
    ring_pixels=sum(near(pixel,focus) for pixel in rgb(focused,(998,28,45,45)))
    check(f'{mode}-{theme}-reason-leaves-focus-ring-visible',ring_pixels>=100,{'image':sha(focused),'focus_pixels':ring_pixels,'rect':[998,28,45,45]})
    chat=a.captures/f'chat-{mode}-{theme}.png';header=control(chat,(1008,37,3,3),(1011,42,18,18))
    check(f'{mode}-{theme}-subdued-header-role',near(header['strongest_glyph'],'596b85' if mode=='light' else 'b3c1b8') and header['contrast']>=4.5,header)
report={'recipe_sha256':sha(__file__),'checks':checks,'measurements':measurements,'scope':'Untouched native pixels; full German text/placement requires separate original-image author inspection. No AX/macOS claim.'}
a.output.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({'checks':len(checks),'failures':[c['check'] for c in checks if c['result']!='pass']}))
if any(c['result']!='pass' for c in checks):raise SystemExit(1)
