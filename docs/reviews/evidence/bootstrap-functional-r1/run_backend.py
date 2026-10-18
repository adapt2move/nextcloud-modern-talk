#!/usr/bin/env python3
"""One bounded frozen-harness cycle on the designated VM; outputs remain external."""
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time

work = Path(sys.argv[1])
source, evidence = work/'source', work/'evidence'
label = 'org.nextcloud-modern-talk.lab-run'

def command(argv):
    p = subprocess.run(argv, capture_output=True, text=True, timeout=30)
    if p.returncode:
        raise RuntimeError(f'{argv[0]} inspection failed: {p.stderr[:500]}')
    return p.stdout.strip()

def inventory():
    return {'labelled_resources': {kind: len(command(['docker',*args,'--filter','label='+label]).split()) for kind,args in (
        ('containers',['ps','--all','--quiet']),('networks',['network','ls','--quiet']),('volumes',['volume','ls','--quiet']))},
        'lab_runtime_directories': len(list(Path('/run').glob('nmtlab-*')))}

def save(name, data):
    (evidence/name).write_text(json.dumps(data,indent=2,sort_keys=True)+'\n')

before = inventory()
assert all(v == 0 for v in before['labelled_resources'].values()) and before['lab_runtime_directories'] == 0, before
pins=json.loads((source/'tests/lab/pins.json').read_text())
before['run_mount_type'] = command(['findmnt','-n','-o','FSTYPE','/run'])
assert before['run_mount_type'] == 'tmpfs'
before['docker_server'] = command(['docker','version','--format','{{.Server.Version}}'])
before['docker_security_options'] = json.loads(command(['docker','info','--format','{{json .SecurityOptions}}']))
before['cached_images'] = {k: json.loads(command(['docker','image','inspect',v['image'],'--format','{"id":{{json .Id}},"digests":{{json .RepoDigests}}}'])) for k,v in pins['profiles'].items()}
before['network_policy_file_present'] = Path('/etc/codex/network-policy.json').is_file()
before['docker_endpoint_selectors_present'] = [k for k in ('DOCKER_HOST','DOCKER_CONTEXT','DOCKER_TLS','DOCKER_TLS_VERIFY','DOCKER_CERT_PATH') if os.environ.get(k)]
assert not before['docker_endpoint_selectors_present']
save('backend-before.json',before)
argv=['sh','tests/lab/reprovision.sh','--output',str(work/'backend-output'),'--budget-seconds','1800']
start=time.time()
proc=subprocess.Popen(argv,cwd=source,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True,start_new_session=True)
timed_out=False
try:
    stdout,stderr=proc.communicate(timeout=1900)
except subprocess.TimeoutExpired:
    timed_out=True
    os.killpg(proc.pid,signal.SIGTERM)
    stdout,stderr=proc.communicate(timeout=100)
stdout=stdout.replace(str(work),'[review-temp]')
stderr=stderr.replace(str(work),'[review-temp]')
save('backend-command.json',{'argv':[x.replace(str(work),'[review-temp]') for x in argv],'exit_code':proc.returncode,
     'elapsed_seconds':round(time.time()-start,3),'outer_timeout':timed_out,'stdout':stdout,'stderr':stderr})
report=(work/'backend-output/report.json').read_bytes()
save('backend-binding.json',{'candidate':'0b6e94db7faa4bee3d2ae88a5d2da2a5567e05be',
     'report_sha256':hashlib.sha256(report).hexdigest(),
     'source_sha256':{str(p.relative_to(source)):hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted((source/'tests/lab').iterdir()) if p.is_file()},
     'note':'Frozen harness has historical source_commit text; this companion binds the fresh report to the exact isolated committed input.'})
(evidence/'backend-report.json').write_bytes(report)
save('backend-after.json',inventory())
data=json.loads(report)
print(json.dumps({'result':data['result'],'run_id':data['run_id'],'profiles':{k:{'result':v['result'],
 'operations':len(v.get('operations',[])),'assertions':len(v.get('assertions',[])), 'error':v.get('error')} for k,v in data['profiles'].items()},
 'cleanup':data['cleanup']},indent=2),flush=True)
