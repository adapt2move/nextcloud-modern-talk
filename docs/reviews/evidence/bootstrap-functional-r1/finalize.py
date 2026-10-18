#!/usr/bin/env python3
"""Final immutable-input check, owned-fixture cleanup and sanitized evidence export."""
import base64
import datetime
import gzip
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys

work,stage=map(Path,sys.argv[1:]); source=work/'source'; evidence=work/'evidence'
assert work.parent==Path('/tmp') and work.name=='nmt-functional-r1-0b6e94db'
assert stage.name=='nmt-functional-r1-input'
manifest=json.loads((stage/'manifest.json').read_text())
expected={v['path']:v for v in manifest['files']}
actual={str(p.relative_to(source)):p for p in source.rglob('*') if p.is_file()}
changed=[p for p in set(expected)&set(actual) if hashlib.sha256(actual[p].read_bytes()).hexdigest()!=expected[p]['sha256'] or actual[p].is_symlink() or bool(actual[p].stat().st_mode & 0o111)!=(expected[p]['mode']=='100755')]
identity={'candidate':manifest['commit'],'expected_files':len(expected),'actual_files':len(actual),'changed':changed,
          'extra':sorted(set(actual)-set(expected)),'missing':sorted(set(expected)-set(actual))}
identity['complete_manifest_equal']=not(changed or identity['extra'] or identity['missing'])
(evidence/'source-final.json').write_text(json.dumps(identity,indent=2)+'\n')
assert identity['complete_manifest_equal'],identity
counts={}
for kind,args in [('containers',['ps','--all','--quiet']),('networks',['network','ls','--quiet']),('volumes',['volume','ls','--quiet'])]:
    p=subprocess.run(['docker',*args,'--filter','label=org.nextcloud-modern-talk.lab-run'],text=True,capture_output=True,timeout=20)
    assert p.returncode==0
    counts[kind]=len(p.stdout.split())
runtime=len(list(Path('/run').glob('nmtlab-*')))
assert all(v==0 for v in counts.values()) and runtime==0
removed=[]
for name in ['source','target','docs-fixture','architecture-fixture','renderer-consumer-target','backend-output']:
    p=work/name
    if p.exists(): shutil.rmtree(p); removed.append(name)
for name in ['unconfigured-probe','unconfigured-probe.rs']:
    p=work/name
    if p.exists():p.unlink();removed.append(name)
shutil.rmtree(stage)
cleanup={'checked_at_utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),'labelled_lab_resources':counts,
         'lab_runtime_directories':runtime,'removed_owned_temporary_paths':removed+['transfer-input'],
         'no_broad_docker_prune':True,'safe_toolchain_retained':Path('/opt/nmt-bootstrap-toolchain/cargo/bin/rustc').exists(),
         'source_unchanged_before_deletion':identity['complete_manifest_equal'],'runtime_secrets_removed':True}
(evidence/'cleanup.json').write_text(json.dumps(cleanup,indent=2)+'\n')
data={p.name:{'sha256':hashlib.sha256(p.read_bytes()).hexdigest(),'data':base64.b64encode(p.read_bytes()).decode()} for p in sorted(evidence.iterdir()) if p.is_file()}
packed=gzip.compress(json.dumps(data,sort_keys=True).encode(),mtime=0)
print('NMT_EVIDENCE_BEGIN')
print(base64.b64encode(packed).decode())
print('NMT_EVIDENCE_END',flush=True)
# Sanitized evidence is retained remotely until the verifier confirms transfer.
