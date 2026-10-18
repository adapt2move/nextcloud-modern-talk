#!/usr/bin/env python3
"""Independent probes/traceability on the already isolated candidate; VM only."""
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys

work, scripts = map(Path, sys.argv[1:])
source, evidence = work/'source', work/'evidence'
env = dict(os.environ, CARGO_HOME='/opt/nmt-bootstrap-toolchain/cargo', RUSTUP_HOME='/opt/nmt-bootstrap-toolchain/rustup', PYTHONDONTWRITEBYTECODE='1')
env['PATH'] = '/opt/nmt-bootstrap-toolchain/cargo/bin:' + env['PATH']
def save(name, data): (evidence/name).write_text(json.dumps(data,indent=2,sort_keys=True)+'\n')
def run(argv, cwd=source):
    p=subprocess.run(argv,cwd=cwd,env=env,capture_output=True,text=True,timeout=120)
    sanitize=lambda t:t.replace(str(work),'[review-temp]').replace(str(scripts),'[review-scripts]')
    return {'argv':[sanitize(str(x)) for x in argv],'exit_code':p.returncode,'stdout':sanitize(p.stdout),'stderr':sanitize(p.stderr)}

# Correct a reviewer assertion matcher; preserve the original run verbatim.
old=json.loads((evidence/'checker-fixtures.json').read_text())
case=next(r for r in old if r['case']=='docs-missing-file')
print('Original missing-file fixture stderr: '+case['stderr'],flush=True)
root=work/'docs-fixture'
original=(root/'README.md').read_text()
try:
    (root/'README.md').write_text(original+'\n[broken](absent.md)\n')
    follow=run(['python3',str(source/'tools/check_docs.py'),'--root',str(root)])
    follow['assertion_pass']=follow['exit_code']==1 and 'missing target' in follow['stderr'] and 'README.md:' in follow['stderr'] and 'absent.md' in follow['stderr']
    follow['note']='Original candidate already rejected missing target; reviewer expected diagnostic wording "missing file" instead of actual "missing target". No checker change.'
finally:
    (root/'README.md').write_text(original)
follow['restored']=run(['python3',str(source/'tools/check_docs.py'),'--root',str(root)])
save('docs-missing-file-recheck.json',follow)

shutil.copy2(scripts/'unconfigured-probe.rs',work/'unconfigured-probe.rs')
deps=work/'target/debug/deps'
core=list(deps.glob('libtalk_core-*.rlib')); adapters=list(deps.glob('libtalk_adapters-*.rlib'))
assert len(core)==len(adapters)==1
compile_result=run(['rustc','--edition=2024','--forbid','unsafe_code',str(work/'unconfigured-probe.rs'),'-L','dependency='+str(deps),'--extern','talk_core='+str(core[0]),'--extern','talk_adapters='+str(adapters[0]),'-o',str(work/'unconfigured-probe')])
probe={'compile':compile_result,'source_sha256':hashlib.sha256((work/'unconfigured-probe.rs').read_bytes()).hexdigest()}
if compile_result['exit_code']==0:
    probe['run']=run([str(work/'unconfigured-probe')])
save('unconfigured-probe.json',probe)

inventory=(source/'docs/compatibility/feature-inventory.md').read_text()
rows=[r for r in inventory.splitlines() if re.match(r'^\| F-\d{3} \|',r)]
features={r.split('|')[1].strip() for r in rows}
statuses={r.split('|')[3].strip() for r in rows}
matrix=(source/'docs/compatibility/test-matrix.md').read_text()
cts=re.findall(r'^### (CT-\d{3})$',matrix,re.M)
covered=set()
for start,end in re.findall(r'F-(\d{3})(?:[–-](?:F-)?(\d{3}))?',matrix):
    covered.update('F-'+str(n).zfill(3) for n in range(int(start),int(end or start)+1))
missing_requirements=[]
for row in rows:
    if not re.search(r'\[(?:AUTH|CONV|MSG|CONTENT|CALL|MOD|DESK|DOM|JNY|STATE|COMP)-\d{3}',row):
        missing_requirements.append(row.split('|')[1].strip())
trace={'feature_rows':len(rows),'unique_feature_ids':len(features),'statuses':sorted(statuses),'ct_ids':cts,'unique_ct_ids':len(set(cts)),
       'features_without_requirement_link':missing_requirements,'features_without_matrix_reference':sorted(features-covered),'unknown_matrix_features':sorted(covered-features),
       'note':'Text/link coverage only. Q.VER-04 per-feature implementation/evidence ownership expands as slices are implemented; ADR-002 currently maps requirement families.'}
save('traceability.json',trace)
print(json.dumps({'traceability':trace,'probe':probe,'missing_file_recheck':follow},indent=2),flush=True)
