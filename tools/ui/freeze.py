#!/usr/bin/env python3
"""Assemble an explicit coordinator baseline + UI overlay, without building locally.

Never reads other owners' mutable source into the build. Manifests remain from the
approved archive. Run build-frozen.py on the assigned development VM.
"""
import argparse, hashlib, io, json, tarfile
from pathlib import Path

def digest(data): return hashlib.sha256(data).hexdigest()

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--baseline', required=True); p.add_argument('--baseline-sha256', required=True)
    p.add_argument('--guard',type=Path,required=True);p.add_argument('--guard-sha256',required=True)
    p.add_argument('--repo', type=Path, required=True); p.add_argument('--output', type=Path, required=True)
    a=p.parse_args(); raw=Path(a.baseline).read_bytes()
    if digest(raw)!=a.baseline_sha256: raise SystemExit('Coordinator archive digest mismatch')
    files={}
    with tarfile.open(fileobj=io.BytesIO(raw),mode='r:gz') as t:
        for m in t.getmembers():
            if not m.isfile() or m.name.startswith('/') or '..' in Path(m.name).parts: raise SystemExit('Invalid baseline member')
            files[m.name]=t.extractfile(m).read()
    for part in ['apps/desktop/src','apps/desktop/assets','tools/ui']:
        for path in sorted((a.repo/part).rglob('*')):
            if path.is_file() and '__pycache__' not in path.parts:
                if path.is_symlink(): raise SystemExit('No symlinks in UI overlay')
                files[str(path.relative_to(a.repo))]=path.read_bytes()
    files['apps/desktop/NOTICE.md']=(a.repo/'apps/desktop/NOTICE.md').read_bytes()
    guard=a.guard.read_bytes()
    if digest(guard)!=a.guard_sha256:raise SystemExit('Approved guard digest mismatch')
    files['tools/check_architecture.py']=guard
    a.output.mkdir(parents=True,exist_ok=False)
    manifest={'schema':1,'scope':'desktop overlay on an explicitly supplied coordinator baseline; other package inputs retain the baseline bytes',
              'coordinator_archive_sha256':a.baseline_sha256,'files':{k:digest(v) for k,v in sorted(files.items())}}
    encoded=(json.dumps(manifest,indent=2,sort_keys=True)+'\n').encode()
    (a.output/'source-manifest.json').write_bytes(encoded)
    with tarfile.open(a.output/'source.tar.gz','w:gz') as tar:
        for k,v in sorted(files.items()):
            m=tarfile.TarInfo(k);m.size=len(v);m.mode=0o444;m.mtime=0;tar.addfile(m,io.BytesIO(v))
    (a.output/'archive-binding.json').write_text(json.dumps({'source_manifest_sha256':digest(encoded),'source_archive_sha256':digest((a.output/'source.tar.gz').read_bytes())},indent=2)+'\n')
    print(json.dumps({'files':len(files),'output':str(a.output)}))
if __name__=='__main__': main()
