#!/usr/bin/env python3
"""Run the native UI author checks in the assigned Linux VM, preserving exact inputs.

The canonical lock is read-only. Use an output directory outside the repository.
This author command cannot approve a product/release gate or macOS support.
"""
import argparse
import hashlib
import json
import os
import platform
from pathlib import Path
import subprocess
import time
from capture import source_manifest,sha

p=argparse.ArgumentParser(description=__doc__)
p.add_argument('--output',required=True)
p.add_argument('--target-dir')
p.add_argument('--no-debug-info',action='store_true')
a=p.parse_args()
root=Path(__file__).resolve().parents[2]
out=Path(a.output).resolve()
if out==root or root in out.parents:raise SystemExit('Evidence output must be outside the repository')
out.mkdir(parents=True,exist_ok=False)
env=os.environ.copy()
if a.target_dir:env['CARGO_TARGET_DIR']=str(Path(a.target_dir).resolve())
if a.no_debug_info:env['CARGO_PROFILE_DEV_DEBUG']='0'
target=Path(env.get('CARGO_TARGET_DIR',root/'target'))
binary=target/'debug/talk-desktop'
report={'schema':1,'kind':'UI author checks; no product gate approval','source_files':source_manifest(root),'commands':[],
    'profile_dev_debug':env.get('CARGO_PROFILE_DEV_DEBUG','default'),'platform':'Linux native X11/Mesa software Vulkan'}
report['environment']={'kernel':platform.platform(), 'os_release':Path('/etc/os-release').read_text(),
    'rustc':subprocess.run(['rustc','--version','--verbose'],env=env,cwd=root,capture_output=True,text=True,check=True,timeout=20).stdout,
    'cargo':subprocess.run(['cargo','--version'],env=env,cwd=root,capture_output=True,text=True,check=True,timeout=20).stdout}
report['source_manifest_sha256']=hashlib.sha256(json.dumps(report['source_files'],sort_keys=True).encode()).hexdigest()
commands=[
    ('format-desktop',['cargo','fmt','--check','-p','talk-desktop']),
    ('format-ui-contract',['rustfmt','--edition','2024','--check','crates/talk-core/src/ui.rs','crates/talk-core/src/ui/presentation.rs']),
    ('build',['cargo','build','--locked','-p','talk-desktop']),
    ('tests',['cargo','test','--locked','-p','talk-desktop','-p','talk-core']),
    ('clippy',['cargo','clippy','--locked','-p','talk-desktop','--all-targets','--','-D','warnings']),
    ('architecture',['python3','tools/check_architecture.py']),
    ('dependency-provenance',['python3','tools/ui/dependency-evidence.py','--output',str(out/'dependencies-linux.json')]),
    ('native-linkage',['ldd',str(binary)]),
    ('captures',['python3','tools/ui/capture.py','--binary',str(binary),'--output',str(out/'screenshots'),
        '--scenarios','server,server:dark,browser,browser:dark,storage,storage:dark,ready,expired,chat,chat:dark,chat:light:sage,chat:dark:sage,settings,settings:dark,settings:dark:sage,incoming,outgoing,prejoin,call,direct-call,voice,gate,offline,loading,denied,error,empty,long-labels']),
    ('interactions',['python3','tools/ui/capture.py','--binary',str(binary),'--output',str(out/'interactions'),
        '--scenarios','server,chat,busy,clipboard-denied,incoming,prejoin','--interactions']),
    ('large-text',['python3','tools/ui/capture.py','--binary',str(binary),'--output',str(out/'large-text'),
        '--scenarios','chat,prejoin,call,settings','--width','720','--height','580','--scale','200','--interactions']),
    ('licenses',[str(binary),'--licenses']),
]
failed=False
try:
    for name,argv in commands:
        started=time.monotonic()
        with (out/(name+'.log')).open('w') as log:
            result=subprocess.run(argv,cwd=root,env=env,stdout=log,stderr=subprocess.STDOUT,timeout=900)
        record={'name':name,'argv':argv,'exit_code':result.returncode,'seconds':round(time.monotonic()-started,3),'log_sha256':sha(out/(name+'.log'))}
        report['commands'].append(record)
        print(name,result.returncode,flush=True)
        if result.returncode:failed=True;break
    if binary.exists():report['binary_sha256']=sha(binary)
    report['source_unchanged']=report['source_files']==source_manifest(root)
    report['status']='pass' if not failed and report['source_unchanged'] else 'fail'
finally:
    # No app secrets are logged, but VM paths are intentionally private handoff data.
    (out/'verification.json').write_text(json.dumps(report,indent=2)+'\n')
raise SystemExit(0 if report.get('status')=='pass' else 1)
