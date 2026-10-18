#!/usr/bin/env python3
"""Independent review runner. Run only on the assigned disposable VM.

Usage: python3 verify.py INPUT_DIRECTORY TEMPORARY_REVIEW_DIRECTORY
INPUT_DIRECTORY contains git-archive candidate.tar, manifest.json and identity.json.
This script does not alter the candidate. Fixtures and build artifacts are siblings.
"""
import difflib
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import time

stage, work = map(Path, sys.argv[1:])
work.mkdir(exist_ok=False)
source, evidence, target = (work / p for p in ('source', 'evidence', 'target'))
source.mkdir()
evidence.mkdir()
identity = json.loads((stage / 'identity.json').read_text())
manifest = json.loads((stage / 'manifest.json').read_text())
assert hashlib.sha256((stage / 'candidate.tar').read_bytes()).hexdigest() == identity['archive_sha256']
assert hashlib.sha256((stage / 'manifest.json').read_bytes()).hexdigest() == identity['manifest_sha256']
with tarfile.open(stage / 'candidate.tar') as archive:
    archive.extractall(source, filter='data')
env = dict(os.environ, CARGO_HOME='/opt/nmt-bootstrap-toolchain/cargo',
           RUSTUP_HOME='/opt/nmt-bootstrap-toolchain/rustup',
           CARGO_TARGET_DIR=str(target), PYTHONDONTWRITEBYTECODE='1')
env['PATH'] = '/opt/nmt-bootstrap-toolchain/cargo/bin:' + env['PATH']


def save(name, data):
    (evidence / name).write_text(json.dumps(data, indent=2, sort_keys=True) + '\n')


def sanitize(value):
    return value.replace(str(work), '[review-temp]').replace(str(stage), '[transfer-input]')


def run(argv, cwd=source, timeout=180):
    started = time.time()
    p = subprocess.run(argv, cwd=cwd, env=env, capture_output=True, text=True, timeout=timeout)
    return {'argv': [sanitize(str(x)) for x in argv], 'cwd': sanitize(str(cwd)),
            'exit_code': p.returncode, 'stdout': sanitize(p.stdout), 'stderr': sanitize(p.stderr),
            'elapsed_seconds': round(time.time() - started, 3)}


def verify_source():
    expected = {f['path']: f for f in manifest['files']}
    actual = {str(p.relative_to(source)): p for p in source.rglob('*') if p.is_file()}
    errors = []
    if set(expected) != set(actual):
        errors.append({'extra': sorted(set(actual)-set(expected)), 'missing': sorted(set(expected)-set(actual))})
    for path in set(actual) & set(expected):
        if actual[path].is_symlink() or hashlib.sha256(actual[path].read_bytes()).hexdigest() != expected[path]['sha256']:
            errors.append({'changed': path})
        if bool(actual[path].stat().st_mode & 0o111) != (expected[path]['mode'] == '100755'):
            errors.append({'mode_changed': path})
    return {'expected_files': len(expected), 'actual_files': len(actual), 'errors': errors,
            'complete_manifest_equal': not errors}


initial = verify_source()
assert initial['complete_manifest_equal'], initial
save('source-before.json', initial)
versions = [run(x) for x in (['rustc','--version','--verbose'], ['cargo','--version'],
             ['rustfmt','--version'], ['cargo','clippy','--version'], ['python3','--version'],
             ['uname','-srmo'], ['cat','/etc/os-release'])]
save('toolchain.json', versions)
assert 'rustc 1.99.1 ' in versions[0]['stdout']
assert 'cargo 1.99.1 ' in versions[1]['stdout']
commands = [
    ['python3','tools/check_docs.py'],
    ['python3','tools/check_architecture.py'],
    ['cargo','fmt','--all','--','--check'],
    ['cargo','check','--workspace','--all-targets','--locked'],
    ['cargo','clippy','--workspace','--all-targets','--locked','--','-D','warnings'],
    ['cargo','test','--workspace','--locked'],
    ['cargo','run','--locked','-p','talk-bootstrap'],
]
checks = []
for argv in commands:
    item = run(argv)
    item['source_after'] = verify_source()
    checks.append(item)
    save('bootstrap-checks.json', checks)
    print(json.dumps(item), flush=True)
    assert item['source_after']['complete_manifest_equal'], 'STOP: candidate was mutated'

# Documentation fixtures: valid syntax, independently broken cases, then restoration.
docs = work / 'docs-fixture'
docs.mkdir()
(docs/'guide').mkdir()
(docs/'guide'/'space name.md').write_text('# Heading\n\n## Duplicate\n\n## Duplicate\n\n<a id="stable-anchor"></a>\n')
(docs/'sample.rs').write_text('fn main() {\n    println!("fixture");\n}\n')
(docs/'guide'/'index.html').write_text('<h1 id="html-anchor">Fixture</h1>\n')
valid = '''# Fixture

[relative](guide/space%20name.md)
[heading](guide/space%20name.md#heading)
[duplicate](guide/space%20name.md#duplicate-1)
[explicit](guide/space%20name.md#stable-anchor)
[reference][target]
[target]: <guide/space name.md#heading>
[root](/guide/space%20name.md#heading)
[source](sample.rs#L1-L3)
[directory](guide/)
[directory HTML](guide/#html-anchor)
<a href="guide/index.html#html-anchor">HTML link</a>

```md
[code example](missing-in-fence.md#absent)
[code][undefined-in-fence]
```

`[inline example](missing-inline.md)`

    [indented](missing-indented.md)

<!-- [comment](missing-comment.md) -->
'''
(docs/'README.md').write_text(valid)
fixture_results = []
def docs_case(name, content, expected_error=None):
    (docs/'README.md').write_text(content)
    result = run(['python3',str(source/'tools/check_docs.py'),'--root',str(docs)])
    result.update(case=name, intended_error=expected_error,
                  assertion_pass=(result['exit_code'] == 0 if expected_error is None else
                  result['exit_code'] != 0 and expected_error in result['stderr'] and 'README.md:' in result['stderr']))
    result['fixture_diff'] = ''.join(difflib.unified_diff(valid.splitlines(True),content.splitlines(True),fromfile='valid/README.md',tofile=name+'/README.md'))
    fixture_results.append(result)
docs_case('docs-valid-links-and-ignored-examples',valid)
docs_case('docs-missing-file',valid+'\n[broken](absent.md)\n','missing file')
docs_case('docs-missing-fragment',valid+'\n[broken](guide/space%20name.md#absent)\n','missing fragment')
docs_case('docs-undefined-reference',valid+'\n[broken][not-defined]\n','undefined reference')
docs_case('docs-restored',valid)

# Architecture checker derives ROOT from its own path, so copy its minimal tree.
arch = work/'architecture-fixture'
arch.mkdir()
for name in ('Cargo.toml','Cargo.lock'):
    shutil.copy2(source/name,arch/name)
for name in ('apps','crates','tools'):
    shutil.copytree(source/name,arch/name)
def arch_case(name, path=None, addition='', expected_error=None, exploratory=False):
    original = (arch/path).read_text() if path else None
    if path:
        (arch/path).write_text(original + addition)
    result = run(['python3','tools/check_architecture.py'],arch)
    result.update(case=name, intended_error=expected_error, exploratory=exploratory,
                  assertion_pass=(result['exit_code'] == 0 if expected_error is None else
                   result['exit_code'] != 0 and expected_error in result['stderr']))
    if path:
        result['fixture_diff'] = ''.join(difflib.unified_diff(original.splitlines(True),(original+addition).splitlines(True),fromfile='valid/'+path,tofile=name+'/'+path))
        (arch/path).write_text(original)
        restored = run(['python3','tools/check_architecture.py'],arch)
        result['restored_baseline'] = restored
        assert restored['exit_code'] == 0, restored
    fixture_results.append(result)
arch_case('architecture-baseline')
arch_case('architecture-core-adapter-edge','crates/talk-core/Cargo.toml',
          '\n[dependencies]\ntalk-adapters.workspace = true\n','forbidden/missing dependency edge')
arch_case('architecture-target-hidden-edge','crates/talk-core/Cargo.toml',
          '\n[target.\'cfg(target_os = "macos")\'.dependencies]\ntalk-adapters.workspace = true\n',
          'hidden platform/build/test edges need review')
arch_case('architecture-presentation-adapter-import','apps/bootstrap/src/presentation.rs',
          '\nuse talk_adapters::UnconfiguredNextcloud;\n','presentation must depend only')
arch_case('architecture-unbounded-renderer-field','crates/talk-core/src/ui.rs',
          '\npub struct FixtureUnbounded { pub rows: Vec<u64> }\n','renderer contract exposes an unbounded')
arch_case('architecture-raw-ocs-route','crates/talk-core/src/policy.rs',
          '\npub const FIXTURE_ROUTE: &str = "/ocs/v2.php/cloud/capabilities";\n','raw protocol route leaked into core')
arch_case('exploratory-unbounded-map-field','crates/talk-core/src/ui.rs',
          '\npub struct FixtureUnboundedMap { pub rows: std::collections::BTreeMap<u64, u64> }\n',
          'renderer contract exposes an unbounded',exploratory=True)
fixture_results.append(dict(case='candidate-docs-after-fixtures', **run(['python3','tools/check_docs.py'])))
save('checker-fixtures.json',fixture_results)
save('source-after-checks-and-fixtures.json',verify_source())
print(json.dumps({'fixtures':[{'case':r['case'],'exit_code':r['exit_code'],'assertion_pass':r.get('assertion_pass')} for r in fixture_results]}),flush=True)
save('build-artifact.json',{'path':'target/debug/talk-bootstrap','sha256':hashlib.sha256((target/'debug/talk-bootstrap').read_bytes()).hexdigest(),
                          'profile':'debug, diagnostic only, not a client or release artifact'})
assert verify_source()['complete_manifest_equal']
