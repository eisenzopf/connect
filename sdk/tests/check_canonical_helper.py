#!/usr/bin/env python3
"""Source-bound synthetic current Node helper/HTTP component; no real server/account."""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import time

ROOT = Path(__file__).resolve().parents[2]


def digest(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True, help='Fresh ignored directory under .local/')
    args = parser.parse_args()
    base = ROOT / '.local'
    base.mkdir(exist_ok=True)
    directory = args.output.absolute()
    assert directory.parent.resolve().is_relative_to(base.resolve())
    directory.mkdir(mode=0o700, parents=True, exist_ok=False)
    environment = dict(os.environ, NPM_CONFIG_LOGS_DIR=str(directory / 'npm-logs'))
    with (directory / 'build.log').open('w') as output:
        build = subprocess.run(['npm', 'run', 'build', '--prefix', 'sdk/typescript'], cwd=ROOT,
                               env=environment, stdout=output, stderr=subprocess.STDOUT, timeout=60)
    assert build.returncode == 0, 'Inspect private build.log'
    paths = list((ROOT / 'sdk/typescript/src').rglob('*.ts')) + list((ROOT / 'sdk/typescript/dist').rglob('*'))
    paths += [ROOT / name for name in ['sdk/typescript/package.json', 'sdk/typescript/package-lock.json',
             'sdk/tests/canonical_native.test.mjs', 'sdk/tests/canonical_http.test.mjs',
             'sdk/tests/canonical_helper_consumer.mjs', 'sdk/tests/fixtures/canonical_helper.mjs',
             'sdk/tests/node_sdk.test.mjs', 'sdk/tests/node_sessions.test.mjs', 'sdk/tests/node_connections.test.mjs', 'sdk/tests/node_movements.test.mjs', 'sdk/tests/node_receipts.test.mjs', 'sdk/tests/node_raw.test.mjs', 'sdk/tests/node_administration.test.mjs', 'sdk/tests/node_events.test.mjs', 'sdk/tests/node_incoming.test.mjs', 'sdk/tests/node_sdk_types.mts',
             'sdk/tests/node_administrative_server_consumer.mjs', 'sdk/tests/node_sdk_server_consumer.mjs', 'sdk/tests/node_consultation_server_consumer.mjs', 'sdk/tests/node_movement_server_consumer.mjs', 'sdk/tests/fixtures/sdk_helper.mjs',
             'sdk/tests/prepare_native_consumer.py']]
    paths.append(Path(__file__))
    before = {str(p.relative_to(ROOT)): digest(p) for p in sorted(set(paths)) if p.is_file()}
    (directory / 'source-inputs.json').write_text(json.dumps(before, indent=2) + '\n')
    started = time.monotonic()
    timed_out = False
    with (directory / 'tests.log').open('w') as output:
        child = subprocess.Popen(['node', '--test', '--test-reporter=tap',
            'sdk/tests/canonical_native.test.mjs', 'sdk/tests/canonical_http.test.mjs', 'sdk/tests/node_sdk.test.mjs', 'sdk/tests/node_sessions.test.mjs', 'sdk/tests/node_connections.test.mjs', 'sdk/tests/node_movements.test.mjs', 'sdk/tests/node_receipts.test.mjs', 'sdk/tests/node_raw.test.mjs', 'sdk/tests/node_administration.test.mjs', 'sdk/tests/node_events.test.mjs', 'sdk/tests/node_incoming.test.mjs'], cwd=ROOT,
            env=environment, stdout=output, stderr=subprocess.STDOUT, start_new_session=True)
        try:
            child.wait(timeout=30)
        except subprocess.TimeoutExpired:
            timed_out = True
            os.killpg(child.pid, signal.SIGKILL)
            child.wait(timeout=10)
    log = (directory / 'tests.log').read_text(errors='replace')
    functions = re.findall(r'^ok [0-9]+ - (.+)$', log, re.M)
    with (directory / 'types.log').open('w') as output:
        types = subprocess.run([str(ROOT / 'sdk/typescript/node_modules/.bin/tsc'), '--noEmit',
            '--strict', '--target', 'ES2022', '--module', 'NodeNext', '--types', 'node',
            '--typeRoots', str(ROOT / 'sdk/typescript/node_modules/@types'),
            'sdk/tests/node_sdk_types.mts'], cwd=ROOT, stdout=output, stderr=subprocess.STDOUT, timeout=30)
    changed = [p for p, value in before.items() if digest(ROOT / p) != value]
    result = {'passed': bool(child.returncode == 0 and not timed_out and not changed
                and types.returncode == 0 and len(functions) == 105 and re.search(r'^# pass 105$', log, re.M)
                and re.search(r'^# fail 0$', log, re.M) and re.search(r'^# skipped 0$', log, re.M)),
        'scope': 'Synthetic owned raw/typed helper lifecycle, typed local operation reconstruction, negative declaration checks and fake local HTTP only. Fixture-shaped signatures are not cryptographic, real-server, installed parity or full G6-062 qualification.',
        'typecheck_exit_code': types.returncode, 'typecheck_log_sha256': digest(directory / 'types.log'),
        'exit_code': child.returncode, 'timed_out': timed_out, 'forced_cleanup': timed_out,
        'seconds': time.monotonic() - started, 'functions': functions, 'source_count': len(before),
        'changed_inputs': changed, 'source_inputs_sha256': digest(directory / 'source-inputs.json'),
        'log_sha256': digest(directory / 'tests.log'),
        'node_version': subprocess.check_output(['node', '--version'], text=True).strip(),
        'typescript_version': subprocess.check_output([str(ROOT / 'sdk/typescript/node_modules/.bin/tsc'), '--version'], text=True).strip(),
        'finished_at': datetime.datetime.now(datetime.timezone.utc).isoformat()}
    (directory / 'summary.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps({k: v for k, v in result.items() if k != 'functions'}))
    raise SystemExit(0 if result['passed'] else 1)


if __name__ == '__main__':
    main()
