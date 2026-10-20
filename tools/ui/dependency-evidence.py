#!/usr/bin/env python3
"""Record the selected desktop dependency closure from the canonical lock, in the VM.

Metadata only; not a license approval or vulnerability audit. No registry paths or
usernames are exported. Every cached registry archive must match its lock checksum.
"""
import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import subprocess
import tomllib

p=argparse.ArgumentParser(description=__doc__)
p.add_argument('--output',required=True)
p.add_argument('--target',default='x86_64-unknown-linux-gnu')
a=p.parse_args()
root=Path(__file__).resolve().parents[2]
lock=(root/'Cargo.lock').read_bytes()
metadata=json.loads(subprocess.run(['cargo','metadata','--locked','--format-version','1','--filter-platform',a.target],cwd=root,capture_output=True,text=True,check=True,timeout=240).stdout)
packages={x['id']:x for x in metadata['packages']}
nodes={x['id']:x for x in metadata['resolve']['nodes']}
start=next(i for i,x in packages.items() if x['name']=='talk-desktop')
selected=set();pending=[start]
while pending:
    item=pending.pop()
    if item in selected: continue
    selected.add(item);pending.extend(nodes[item]['dependencies'])
checksums={(x['name'],x['version']):x.get('checksum') for x in tomllib.loads(lock.decode())['package']}
result={'schema':1,'target':a.target,'scope':'talk-desktop resolved Cargo closure including build dependencies; not license approval','canonical_lock_sha256':hashlib.sha256(lock).hexdigest(),'packages':[]}
for i in sorted(selected,key=lambda k:(packages[k]['name'],packages[k]['version'])):
    x=packages[i]
    checksum=checksums.get((x['name'],x['version']))
    item={'name':x['name'],'version':x['version'],'license':x['license'],'repository':x['repository'],'source':x['source'],'features':nodes[i]['features'],'archive_sha256':checksum}
    if checksum:
        archives=list((Path.home()/'.cargo/registry/cache').glob(f"*/{x['name']}-{x['version']}.crate"))
        item['cached_archive_verified']=bool(archives)
        for archive in archives:
            if hashlib.sha256(archive.read_bytes()).hexdigest()!=checksum: raise RuntimeError('Archive checksum mismatch: '+x['name'])
    result['packages'].append(item)
result['license_expression_counts']=dict(sorted(Counter(x['license'] or 'MISSING' for x in result['packages']).items()))
Path(a.output).write_text(json.dumps(result,indent=2)+'\n')
print(json.dumps({'packages':len(selected),'licenses':result['license_expression_counts'],'lock':result['canonical_lock_sha256']},indent=2))
