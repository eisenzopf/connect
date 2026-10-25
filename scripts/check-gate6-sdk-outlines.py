#!/usr/bin/env python3
"""Compile G6-059 target declarations/outlines, including six negative cases.

This checks the proposed API's static usability, never SDK implementation parity.
Run with the verification Python containing mypy; outputs stay under .local/.
"""
import argparse
from collections import Counter
import datetime
import hashlib
import importlib.metadata
import json
from pathlib import Path
import re
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    output = args.output.resolve()
    assert output.is_relative_to((ROOT / '.local').resolve()), 'Use ignored .local output'
    output.mkdir(parents=True, exist_ok=False)
    names = sorted(str(p.relative_to(ROOT)) for p in (ROOT / 'contracts/sdk-api').rglob('*') if p.is_file())
    names += ['contracts/sdk-gate-6-api.json', 'scripts/check-gate6-sdk-outlines.py', 'scripts/format.sh', 'sdk/typescript/package-lock.json']
    before = {n: digest(ROOT / n) for n in names}
    checks = []

    def run(name, command, expected=0):
        result = subprocess.run([str(x) for x in command], cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=120)
        log = output / f'{name}.log'
        log.write_text(result.stdout)
        assert result.returncode == expected, f'{name}: exit {result.returncode}, see {log}'
        checks.append({'name': name, 'command': [str(x) for x in command], 'exit': result.returncode, 'log_sha256': digest(log)})
        return result.stdout

    tsc = ROOT / 'sdk/typescript/node_modules/.bin/tsc'
    versions = {
        'typescript': run('tsc-version', [tsc, '--version']).strip(),
        'rustc': run('rustc-version', ['rustc', '--version']).strip(),
        'python': sys.version.split()[0],
        'python_typecheck_target': '3.12',
        'mypy': importlib.metadata.version('mypy'),
    }
    run('format', ['bash', 'scripts/format.sh', '--check'])
    for target in ('browser', 'node'):
        run(f'{target}-positive', [tsc, '-p', f'contracts/sdk-api/typescript/{target}.tsconfig.json'])
    python = [sys.executable, '-m', 'mypy', '--strict', '--python-version', '3.12', '--warn-unused-ignores', '--cache-dir', output / 'mypy-cache']
    run('python-positive', python + [f'contracts/sdk-api/python/{name}.py' for name in ('contract', 'outline', 'negative')])
    rust_output = output / 'rust-target'
    rust_output.mkdir()
    rust = ['rustc', '--edition=2024', '--crate-type=lib']
    run('rust-positive', rust + ['--crate-name', 'gate6_sdk_contract', 'contracts/sdk-api/rust/outline.rs', '--out-dir', rust_output])

    # Unsuppressed copies must each produce exactly six expected diagnostics.
    negative = output / 'negative'
    negative.mkdir()
    ts_dir = negative / 'typescript'
    shutil.copytree(ROOT / 'contracts/sdk-api/typescript', ts_dir)
    p = ts_dir / 'negative.ts'
    p.write_text(re.sub(r'^.*@ts-expect-error.*\n', '', p.read_text(), flags=re.M))
    diagnostics = {}
    for target in ('browser', 'node'):
        p = ts_dir / f'{target}.tsconfig.json'
        config = json.loads(p.read_text())
        if target == 'node':
            config['compilerOptions']['typeRoots'] = [str(ROOT / 'sdk/typescript/node_modules/@types')]
        p.write_text(json.dumps(config))
        errors = run(f'{target}-negative', [tsc, '--pretty', 'false', '-p', p], expected=1)
        codes = re.findall(r'error (TS\d+):', errors)
        assert Counter(codes) == Counter(['TS2322'] * 4 + ['TS2741', 'TS2339']), (target, codes)
        assert errors.count('negative.ts(') == 6, errors
        diagnostics[target] = codes
    py_dir = negative / 'python'
    shutil.copytree(ROOT / 'contracts/sdk-api/python', py_dir)
    p = py_dir / 'negative.py'
    p.write_text(re.sub(r'  # type: ignore\[[^\]]+\]', '', p.read_text()))
    errors = run('python-negative', python + [py_dir / 'negative.py'], expected=1)
    codes = re.findall(r'error:.*\[([\w-]+)\]', errors)
    assert Counter(codes) == Counter(['assignment'] * 2 + ['call-arg'] * 2 + ['attr-defined', 'arg-type']), codes
    diagnostics['python'] = codes
    errors = run('rust-negative', rust + ['--crate-name', 'gate6_sdk_contract_negative', '--error-format=json', 'contracts/sdk-api/rust/negative.rs', '--out-dir', rust_output], expected=1)
    codes = [row['code']['code'] for line in errors.splitlines() if (row := json.loads(line)).get('level') == 'error' and row.get('code')]
    assert Counter(codes) == Counter(['E0308'] * 2 + ['E0061', 'E0063', 'E0599', 'E0277']), codes
    diagnostics['rust'] = codes
    unchanged = all(digest(ROOT / name) == value for name, value in before.items())
    assert unchanged, 'Inputs changed during checking'
    finalized = json.loads((ROOT / 'contracts/sdk-gate-6-api.json').read_text())['status'] == 'frozen'
    report = {
        'schema': 'connect-gate6-sdk-contract-outlines/1', 'task': 'G6-059', 'passed': True,
        'scope': 'Declaration-only target contract; Rust, browser/Node TypeScript, Python static outlines and six negative constraints per compiler target. No SDK runtime, package, media, transport or acceptance qualification.',
        'candidate_finalized': finalized, 'versions': versions, 'checks': checks, 'negative_diagnostics': diagnostics,
        'inputs_sha256': before, 'inputs_unchanged': unchanged,
        'checked_at': datetime.datetime.now(datetime.timezone.utc).isoformat(),
    }
    (output / 'summary.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'passed': True, 'candidate_finalized': finalized, 'compiler_targets': len(diagnostics), 'negative_cases_per_target': 6, 'inputs_unchanged': unchanged}))


if __name__ == '__main__':
    main()
