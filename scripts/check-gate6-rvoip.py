#!/usr/bin/env python3
"""Test recorded Gate 6 rvoip patches and save scoped qualification evidence."""
import argparse
import datetime
import hashlib
import json
import pathlib
import re
import subprocess

ROOT = pathlib.Path(__file__).resolve().parents[1]


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--component', choices=['wire', 'relationships'], default='wire')
    parser.add_argument('--output', type=pathlib.Path)
    args = parser.parse_args()
    subprocess.run(['bash', 'scripts/prepare-dependencies.sh'], cwd=ROOT, check=True)
    upstream = ROOT / '.deps/rvoip/crates/uctp/rvoip-uctp/src/v2'
    for source, destination in [('contracts/schemas/uctp-envelope-2.schema.json', 'envelope.schema.json'),
                                ('contracts/uctp-vectors/gate6-envelope.json', 'golden-vectors.json')]:
        if digest(ROOT / source) != digest(upstream / destination):
            raise SystemExit(f'Published and compiled contract differ: {source}')
    wire = args.component == 'wire'
    package = 'rvoip-uctp' if wire else 'rvoip-core'
    prefix = 'v2::tests' if wire else 'relationships::tests'
    command = ['cargo', 'test', '--locked', '--manifest-path', '.deps/rvoip/Cargo.toml', '-p', package,
               *([] if wire else ['--features', 'opus']), '--lib', prefix, '--', '--nocapture']
    directory = ROOT / '.local/gate-6'
    directory.mkdir(exist_ok=True)
    log = directory / ('wire-types.log' if wire else 'relationships.log')
    with log.open('w') as output:
        result = subprocess.run(command, cwd=ROOT, stdout=output, stderr=subprocess.STDOUT)
    text = log.read_text()
    names = re.findall(r'^test (' + re.escape(prefix) + r'::\S+) \.\.\. ok$', text, re.M)
    vectors = json.loads((upstream / 'golden-vectors.json').read_text())['vectors'] if wire else []
    source_paths = sorted(upstream.glob('*.rs')) if wire else [
        ROOT / '.deps/rvoip/crates/foundation/rvoip-core/src/relationships.rs',
        ROOT / '.deps/rvoip/crates/foundation/rvoip-core/src/relationships/tests.rs',
        ROOT / '.deps/rvoip/crates/foundation/rvoip-core-traits/src/ids.rs',
    ]
    report = dict(
        format=1, task='G6-016' if wire else 'G6-017', executed_at=datetime.datetime.now(datetime.timezone.utc).isoformat(),
        scope=('Shared wire decoding, typed payloads and transition guards.' if wire else 'Shared relationship transitions, membership epochs, session roles and endpoint lineage.') + ' Not server dispatch, external authentication, live media, provider or independent client conformance.',
        command=command, exit_code=result.returncode,
        source_lock_sha256=digest(ROOT / 'dependencies/source-lock.json'),
        schema_sha256=digest(upstream / 'envelope.schema.json'),
        vectors_sha256=digest(upstream / 'golden-vectors.json'),
        upstream_sources={str(p.relative_to(ROOT / '.deps/rvoip')): digest(p) for p in source_paths},
        schema_vectors=len(vectors), valid_type_vectors=sum(v['valid'] for v in vectors),
        tests_passed=names, private_log=str(log.relative_to(ROOT)), log_sha256=digest(log),
        passed=result.returncode == 0 and len(names) >= (11 if wire else 7),
    )
    output = args.output or ROOT / 'docs/evidence/gate-6' / ('wire-types.json' if wire else 'relationships.json')
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({k: report[k] for k in ('task', 'exit_code', 'schema_vectors', 'valid_type_vectors', 'passed')}))
    if not report['passed']:
        print(text[-6000:])
        raise SystemExit(1)


if __name__ == '__main__':
    main()
