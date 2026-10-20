#!/usr/bin/env python3
"""Measure untouched D1.2 native control pixels (VM only; no image rewriting).

Reports strongest drawn glyph/background contrast within stated inset rectangles.
This is author diagnostic evidence, not AX/macOS or general visual acceptance.
"""
import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import subprocess


def sha(p): return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def rgb(path, rect):
    x,y,w,h=rect
    raw=subprocess.run(['convert',str(path),'-crop',f'{w}x{h}+{x}+{y}','+repage','-depth','8','RGB:-'],check=True,stdout=subprocess.PIPE).stdout
    assert len(raw)==w*h*3
    return [tuple(raw[i:i+3]) for i in range(0,len(raw),3)]
def contrast(a,b):
    def lum(c):
        v=[n/255 for n in c];v=[n/12.92 if n<=.04045 else ((n+.055)/1.055)**2.4 for n in v]
        return sum(n*k for n,k in zip(v,[.2126,.7152,.0722]))
    a,b=lum(a),lum(b)
    return (max(a,b)+.05)/(min(a,b)+.05)
def control(path,bg_rect,glyph_rect):
    bg=Counter(rgb(path,bg_rect)).most_common(1)[0][0]
    fg=max(rgb(path,glyph_rect),key=lambda color:contrast(color,bg))
    return {'image':str(path),'sha256':sha(path),'background_rect':bg_rect,'glyph_rect':glyph_rect,'background':bg,'strongest_glyph':fg,'contrast':round(contrast(fg,bg),4)}
def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--before',type=Path,required=True);p.add_argument('--final',type=Path,required=True);p.add_argument('--output',type=Path,required=True)
    a=p.parse_args();results={'script_sha256':sha(__file__),'before':[],'corrected':[],'style_matrix':[]}
    for appearance in ['light','dark']:
        for theme in ['blue','sage']:
            name=f'offline-{appearance}-{theme}.png'
            for folder,key in [(a.before,'before'),(a.final,'corrected')]:
                value=control(folder/name,(1039,617,5,6),(1045,622,16,19));value['theme']=theme;value['appearance']=appearance;results[key].append(value)
            gallery=a.final/f'controls-{appearance}-{theme}.png'
            for style,y in [('Primary',160),('Danger',264),('Secondary',368),('Quiet',472)]:
                for kind,bg,glyph in [('text',(159,y-13,5,5),(165,y-8,77,18)),('icon',(276,y-12,4,4),(280,y-8,18,18))]:
                    value=control(gallery,bg,glyph);value.update(theme=theme,appearance=appearance,style=style,kind=kind)
                    value['within_unavailable_target']=2.0<=value['contrast']<=3.6
                    results['style_matrix'].append(value)
    results['all_corrected_unavailable_contrasts_in_target']=all(2.0<=v['contrast']<=3.6 for v in results['corrected']+results['style_matrix'])
    a.output.write_text(json.dumps(results,indent=2)+'\n')
    print(json.dumps({'all_in_target':results['all_corrected_unavailable_contrasts_in_target'],'corrected_send':[v['contrast'] for v in results['corrected']],'style_range':[min(v['contrast'] for v in results['style_matrix']),max(v['contrast'] for v in results['style_matrix'])]}))
    if not results['all_corrected_unavailable_contrasts_in_target']: raise SystemExit(1)
if __name__=='__main__': main()
