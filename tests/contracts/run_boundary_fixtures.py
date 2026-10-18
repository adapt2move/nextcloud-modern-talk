#!/usr/bin/env python3
"""Presentation and architecture regressions; run on the assigned VM/CI runner.

Copies the candidate's minimal Cargo/source tree to disposable directories. Never
mutates the candidate. No backend/service/provisioning operations. Uses installed
pinned cargo and Python; no toolchain setup or network work is authorized by this file.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

parser = argparse.ArgumentParser()
parser.add_argument('--candidate', type=Path, required=True)
parser.add_argument('--output', type=Path, required=True)
args = parser.parse_args()
candidate = args.candidate.resolve()
positive = Path(__file__).with_name('presentation_consumer.rs').read_text()
records = []


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def copy_source(destination):
    destination.mkdir()
    for name in ('Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml', 'rustfmt.toml'):
        shutil.copy2(candidate / name, destination / name)
    for name in ('crates', 'apps'):
        shutil.copytree(candidate / name, destination / name)
    (destination / 'tools').mkdir()
    shutil.copy2(candidate / 'tools/check_architecture.py', destination / 'tools/check_architecture.py')


def append(root, path, text):
    item = root / path
    item.write_text(item.read_text() + '\n' + text + '\n')


with tempfile.TemporaryDirectory(prefix='identity-ui-fixtures-') as temporary:
    scratch = Path(temporary)
    env = dict(os.environ, CARGO_TARGET_DIR=str(scratch / 'target'), PYTHONDONTWRITEBYTECODE='1')

    def run(root, argv):
        result = subprocess.run(argv, cwd=root, env=env, capture_output=True, text=True, timeout=180)
        return {
            'argv': argv, 'exit_code': result.returncode,
            'stdout': result.stdout.replace(str(scratch), '[fixture-temp]').replace(str(candidate), '[candidate]'),
            'stderr': result.stderr.replace(str(scratch), '[fixture-temp]').replace(str(candidate), '[candidate]'),
        }

    def case(name, mutation=None, expected=0, diagnostic=None, compile_positive=False):
        root = scratch / name
        copy_source(root)
        lock_before = digest(root / 'Cargo.lock')
        if mutation:
            mutation(root)
        checker = run(root, ['python3', 'tools/check_architecture.py'])
        record = {'case': name, 'checker': checker, 'expected_exit': expected}
        record['pass'] = checker['exit_code'] == expected and (
            diagnostic is None or diagnostic in checker['stderr']
        )
        if compile_positive:
            compiler = run(root, ['cargo', 'check', '--locked', '--workspace', '--all-targets'])
            record['compiler'] = compiler
            record['pass'] = record['pass'] and compiler['exit_code'] == 0
        record['lock_unchanged'] = lock_before == digest(root / 'Cargo.lock')
        record['pass'] = record['pass'] and record['lock_unchanged']
        records.append(record)

    case('baseline')
    case('positive-consumer', lambda root: (root / 'apps/bootstrap/src/presentation.rs').write_text(positive), compile_positive=True)
    grouped = positive.replace('use talk_core::ui::{', 'use talk_core::{ui::{').replace('    MediaIntent, Page, PersistenceState, Text, UserIntent,\n};', '    MediaIntent, Page, PersistenceState, Text, UserIntent,\n}};')
    case('positive-grouped-consumer', lambda root: (root / 'apps/bootstrap/src/presentation.rs').write_text(grouped), compile_positive=True)
    case('core-adapter-edge', lambda root: append(root, 'crates/talk-core/Cargo.toml', '[dependencies]\ntalk-adapters = { workspace = true }'), 1, 'forbidden/missing dependency edge')
    case('target-specific-edge', lambda root: append(root, 'crates/talk-core/Cargo.toml', "[target.'cfg(unix)'.dependencies]\ntalk-adapters = { workspace = true }"), 1, 'unreviewed manifest section')
    case('presentation-adapter', lambda root: append(root, 'apps/bootstrap/src/presentation.rs', 'use talk_adapters::identity::UnconfiguredIdentity;'), 1, 'presentation import')
    case('unbounded-vec', lambda root: append(root, 'crates/talk-core/src/ui.rs', 'pub struct FixtureUnbounded { pub rows: Vec<u64> }'), 1, 'unbounded container')
    case('raw-route', lambda root: append(root, 'crates/talk-core/src/identity.rs', 'pub const FIXTURE_ROUTE: &str = "/ocs/v2.php/fixture";'), 1, 'raw protocol route')
    # Exact independently executed BFR1-01 fixture, unchanged.
    case('unbounded-btreemap', lambda root: append(root, 'crates/talk-core/src/ui.rs', 'pub struct FixtureUnboundedMap { pub rows: std::collections::BTreeMap<u64, u64> }'), 1, 'unbounded container')
    for name, value in (
        ('hashmap', 'std::collections::HashMap<u64, u64>'),
        ('hashset', 'std::collections::HashSet<u64>'),
        ('btreeset', 'std::collections::BTreeSet<u64>'),
        ('vecdeque', 'std::collections::VecDeque<u64>'),
        ('linkedlist', 'std::collections::LinkedList<u64>'),
        ('binaryheap', 'std::collections::BinaryHeap<u64>'),
    ):
        case('unbounded-' + name, lambda root, value=value: append(root, 'crates/talk-core/src/ui.rs', 'pub struct FixtureContainer { pub rows: ' + value + ' }'), 1, 'unbounded container')
    case('worker-reexport', lambda root: append(root, 'crates/talk-core/src/ui.rs', 'pub use crate::ports::IdentityPort;'), 1, 'unreviewed data export/import')
    case('grouped-worker-import', lambda root: append(root, 'apps/bootstrap/src/presentation.rs', 'use talk_core::{ui::DesktopPort as Desktop, ports::IdentityPort};'), 1, 'explicit paths without aliases/globs')
    case('grouped-worker-without-alias', lambda root: append(root, 'apps/bootstrap/src/presentation.rs', 'use talk_core::{ports::IdentityPort};'), 1, 'presentation import')
    case('data-wildcard', lambda root: append(root, 'crates/talk-core/src/ui.rs', 'pub use crate::ids::*;'), 1, 'explicit paths without aliases/globs')
    case('frame-field', lambda root: append(root, 'crates/talk-core/src/ui.rs', 'pub struct Frame; pub struct FixtureFrame { pub frame: Frame }'), 1, 'worker or frame')
    case('secret-field', lambda root: append(root, 'crates/talk-core/src/ui.rs', 'pub struct FixtureSecret { pub password: u64 }'), 1, 'secret/transport-bearing')
    case('restored-baseline')

result = {
    'status': 'pass' if all(item['pass'] for item in records) else 'fail',
    'scope': 'isolated compile/lexical-boundary fixtures only; no native qualification',
    'records': records,
}
args.output.parent.mkdir(parents=True, exist_ok=True)
args.output.write_text(json.dumps(result, indent=2, sort_keys=True) + '\n')
print(json.dumps({'status': result['status'], 'cases': len(records)}))
raise SystemExit(0 if result['status'] == 'pass' else 1)
