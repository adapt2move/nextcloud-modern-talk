#!/usr/bin/env python3
"""Build/check the supplied immutable input OUTSIDE dev-box automatic sync (VM only)."""
import argparse, hashlib, json, os, shutil, subprocess, tarfile, time
from pathlib import Path

def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def verify(root, manifest):
    actual={str(p.relative_to(root)):sha(p) for p in root.rglob('*') if p.is_file() and '__pycache__' not in p.parts}
    if actual!=manifest['files']:raise RuntimeError('Frozen source changed; do not capture this candidate')
def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('input',type=Path);p.add_argument('--target',type=Path,required=True)
    p.add_argument('--check-only',action='store_true');a=p.parse_args();base=a.input.resolve()
    manifest=json.loads((base/'source-manifest.json').read_text());binding=json.loads((base/'archive-binding.json').read_text())
    assert sha(base/'source.tar.gz')==binding['source_archive_sha256'];assert sha(base/'source-manifest.json')==binding['source_manifest_sha256']
    root=base/'source'
    if not root.exists():
        root.mkdir()
        with tarfile.open(base/'source.tar.gz') as tar:tar.extractall(root,filter='data')
    verify(root,manifest)
    env=os.environ.copy();env['PATH']='/root/.cargo/bin:'+env['PATH'];env['CARGO_TARGET_DIR']=str(a.target.resolve());env['CARGO_BUILD_JOBS']='6';env['CARGO_PROFILE_DEV_DEBUG']='0';env['PYTHONDONTWRITEBYTECODE']='1'
    # Reproducible archives use zero mtimes. Cargo's timestamp cache alone cannot
    # establish new input identity. Invalidate the compiled workspace closure,
    # including adapters supplied by an immutable peer overlay; keep dependency cache.
    commands=[['rustc','--version','--verbose'],['cargo','--version'],['cargo','clean','-p','talk-desktop','-p','talk-core','-p','talk-adapters'],['cargo','build','--locked','-p','talk-desktop']]
    if a.check_only:commands=[['cargo','fmt','--package','talk-desktop','--','--check'],['cargo','test','--locked','-p','talk-desktop','-p','talk-core'],['cargo','clippy','--locked','-p','talk-desktop','-p','talk-core','--all-targets','--','-D','warnings'],['python3','tools/check_architecture.py']]
    results=[]
    for i,cmd in enumerate(commands):
        log=base/(('check' if a.check_only else 'build')+f'-{i:02}.log');start=time.time()
        with log.open('w') as f: result=subprocess.run(cmd,cwd=root,env=env,stdout=f,stderr=subprocess.STDOUT)
        results.append({'command':cmd,'exit':result.returncode,'seconds':round(time.time()-start,3),'log':log.name,'sha256':sha(log)})
        print(json.dumps(results[-1]),flush=True)
        verify(root,manifest)
        if result.returncode:break
    report={'schema':1,'source_manifest_sha256':sha(base/'source-manifest.json'),'source_archive_sha256':sha(base/'source.tar.gz'),'lock_sha256':sha(root/'Cargo.lock'),'recipe_sha256':sha(Path(__file__)),'commands':results,'source_unchanged':True}
    if not a.check_only and len(results)==4 and results[-1]['exit']==0:
        binary=base/'talk-desktop';shutil.copy2(a.target/'debug/talk-desktop',binary);binary.chmod(0o555)
        report['binary_sha256']=sha(binary);report['binary']=binary.name
    (base/('checks.json' if a.check_only else 'build-binding.json')).write_text(json.dumps(report,indent=2)+'\n')
    raise SystemExit(results[-1]['exit'])
if __name__=='__main__':main()
