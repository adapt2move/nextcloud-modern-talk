#!/usr/bin/env python3
"""Correct review-only parser to expand slash shorthand and include INT IDs."""
import json
from pathlib import Path
import re
import sys

work=Path(sys.argv[1]); source=work/'source'
text=(source/'docs/compatibility/feature-inventory.md').read_text()
rows=[r for r in text.splitlines() if re.match(r'^\| F-\d{3} \|',r)]
features={r.split('|')[1].strip() for r in rows}
matrix=(source/'docs/compatibility/test-matrix.md').read_text()
cts=re.findall(r'^### (CT-\d{3})$',matrix,re.M)
covered=set()
for expression in re.findall(r'F-\d{3}(?:[–/-](?:F-)?\d{3})*',matrix):
    for part in expression.removeprefix('F-').replace('F-','').split('/'):
        ends=re.split('[–-]',part)
        covered.update('F-'+str(n).zfill(3) for n in range(int(ends[0]),int(ends[-1])+1))
definitions={}
for p in [*(source/'docs/product').glob('*.md'),source/'docs/compatibility/README.md']:
    for ident in re.findall(r'^#{2,3} ([A-Z]+-\d{3})$',p.read_text(),re.M):
        definitions[ident]=str(p.relative_to(source))
missing=[]; unknown=[]; links=0
for row in rows:
    refs=[]
    for prefix,expression in re.findall(r'\[([A-Z]+)-(\d{3}(?:[/–-]\d{3})*)\]',row):
        if prefix in ('CT','F'): continue
        for part in expression.split('/'):
            ends=re.split('[–-]',part)
            refs.extend(prefix+'-'+str(n).zfill(3) for n in range(int(ends[0]),int(ends[-1])+1))
    links+=len(refs)
    if not refs: missing.append(row.split('|')[1].strip())
    unknown.extend(x for x in refs if x not in definitions)
result={'feature_rows':len(rows),'unique_feature_ids':len(features),'statuses':sorted({r.split('|')[3].strip() for r in rows}),
        'defined_requirement_journey_state_ids':len(definitions),'ct_ids':cts,'unique_ct_ids':len(set(cts)),
        'feature_requirement_link_occurrences':links,'features_without_requirement_link':missing,'undefined_requirement_links':unknown,
        'features_without_matrix_reference':sorted(features-covered),'unknown_matrix_features':sorted(covered-features),
        'note':'Corrects reviewer-only parser omissions: INT, slash-compressed feature references and the STATE-001–007 linked label. Initial outputs retained as traceability.json and traceability-recheck.json. No candidate/specification changes.'}
result['assertions_pass']=len(features)==len(rows)==140 and result['statuses']==['Planned'] and len(cts)==len(set(cts))==28 and not(missing or unknown or features-covered or covered-features)
(work/'evidence/traceability-final.json').write_text(json.dumps(result,indent=2,sort_keys=True)+'\n')
print(json.dumps(result,indent=2))
