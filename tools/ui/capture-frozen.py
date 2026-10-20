#!/usr/bin/env python3
"""Capture a compiled immutable candidate; never infer compiled inputs from a worktree."""
import argparse, json
from pathlib import Path
from capture import capture, sha

def validate(base):
    binding=json.loads((base/'build-binding.json').read_text())
    if 'binary_sha256' not in binding: raise RuntimeError('No successful build binding')
    manifest=json.loads((base/'source-manifest.json').read_text())
    assert sha(base/'source-manifest.json')==binding['source_manifest_sha256']
    assert sha(base/'source.tar.gz')==binding['source_archive_sha256']
    assert sha(base/binding['binary'])==binding['binary_sha256']
    actual={str(p.relative_to(base/'source')):sha(p) for p in (base/'source').rglob('*') if p.is_file() and '__pycache__' not in p.parts}
    if actual!=manifest['files']:raise RuntimeError('Compiled input changed since build')
    return binding,manifest

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--build-input',type=Path,required=True);p.add_argument('--output',type=Path,required=True)
    p.add_argument('--scenarios',default='server:light:blue,server:dark:blue,chat:light:blue,chat:dark:sage,settings:light:blue,settings:dark:sage')
    p.add_argument('--width',type=int,default=1120);p.add_argument('--height',type=int,default=716);p.add_argument('--scale',type=int,default=100);p.add_argument('--interactions',action='store_true');p.add_argument('--interaction-suite',choices=['d1','d1-1','d1-2'],default='d1')
    a=p.parse_args();base=a.build_input.resolve();binding,manifest=validate(base)
    a.binary=str(base/binding['binary']);a.output=str(a.output)
    try:capture(a)
    finally:
        validate(base)
        out=Path(a.output)/'scenario-manifest.json'
        if out.exists():
            report=json.loads(out.read_text());report['capture_harness_source_files']={k:v for k,v in report.pop('source_files').items() if k.startswith('tools/ui/')};report.pop('source_manifest_sha256')
            report['compiled_input']=binding;report['compiled_source_files']=manifest['files'];report['build_binding_sha256']=sha(base/'build-binding.json');report['capture_wrapper_sha256']=sha(__file__)
            report['build_input_unchanged_after_capture']=True
            out.write_text(json.dumps(report,indent=2)+'\n')
if __name__=='__main__':main()
