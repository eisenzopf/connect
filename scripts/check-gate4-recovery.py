#!/usr/bin/env python3
"""Bounded local Gate 4 recovery with actual historical binaries and disposable state.

Build Connect first; this command never builds or touches an existing installation.
The schema-10 baseline is the preserved pre-Gate-4 candidate, not a mock reader.
"""
import argparse
import datetime
import hashlib
import importlib.util
import json
import os
import pathlib
import platform
import re
import signal
import subprocess
import sys
import time
import traceback

from lib.local_process import ROOT, BINARY
from lib.profile_metrics import ProfileReport
from lib.recovery_state import baseline

sys.path.insert(0, str(ROOT / 'sdk/python'))
from lib.recovery_upgrade import qualify as upgrade  # noqa: E402
from lib.recovery_tasks import qualify as tasks  # noqa: E402
from lib.recovery_interruption import qualify as interruption  # noqa: E402

SCHEMA10 = '99243f3962cf25e780bbd176e56a4f27c752e121'
SCHEMA6 = '8bf5bd31a519a5fa84ef8f80e4da3b58374c6de5'


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def expired(_signal, _frame):
    raise TimeoutError('Recovery qualification exceeded its 600-second budget')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=pathlib.Path, default=BINARY)
    parser.add_argument('--previous-binary', type=pathlib.Path,
                        default=ROOT / '.local/gate-4-baseline/connect')
    parser.add_argument('--schema-six-binary', type=pathlib.Path,
                        default=ROOT / '.local/recovery-baselines' / SCHEMA6 / 'connect')
    args = parser.parse_args()
    os.umask(0o077)
    work = ROOT / '.local/gate4-recovery' / datetime.datetime.now(
        datetime.timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ')
    work.mkdir(parents=True)
    binary, previous, six = [p.resolve() for p in
                             (args.binary, args.previous_binary, args.schema_six_binary)]
    paths = [pathlib.Path(__file__), ROOT / 'scripts/check-schema-six.py',
             ROOT / 'scripts/build-recovery-baseline.py', ROOT / 'crates/connect-store/src/lib.rs',
             ROOT / 'scripts/lib/local_process.py', ROOT / 'scripts/lib/profile_metrics.py',
             ROOT / 'scripts/lib/recovery_state.py', ROOT / 'scripts/lib/recovery_upgrade.py',
             ROOT / 'scripts/lib/recovery_tasks.py', ROOT / 'Cargo.lock',
             ROOT / 'scripts/lib/recovery_interruption.py',
             ROOT / 'dependencies/source-lock.json']
    paths.extend((ROOT / 'sdk/python/connect_sdk').glob('*.py'))
    inputs = {str(p.relative_to(ROOT)): digest(p) for p in paths}
    report = ProfileReport(work, {
        'scope': 'local macOS arm64; disposable state and actual binaries; no live providers',
        'budget_seconds': 600, 'source_inputs_sha256': inputs,
        'source_revision': subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip(),
        'excluded': ['uncertain provider effect recovery',
                     'whole G4-032 dependency and installed-candidate closure'],
    })
    before = signal.signal(signal.SIGALRM, expired)
    signal.setitimer(signal.ITIMER_REAL, 600)
    started, error = time.monotonic(), None
    try:
        assert (platform.system(), platform.machine()) == ('Darwin', 'arm64')
        assert binary.is_file(), 'Build Connect before recovery qualification'
        current_hash = digest(binary)
        report.data['binary_sha256'] = current_hash
        report.data['baselines'] = [baseline(previous, SCHEMA10, 10), baseline(six, SCHEMA6, 6)]
        match = re.search(r'pub const SCHEMA_VERSION: u32 = (\d+);',
                          (ROOT / 'crates/connect-store/src/lib.rs').read_text())
        assert match
        schema = int(match[1])
        report.data['target_schema'] = schema
        with report.phase('schema-10-upgrade-refusal-rollback-ddl-recovery'):
            state, saved, evidence = upgrade(binary, previous, work, schema)
            report.checkpoint(schema10=evidence)
        with report.phase('task-backup-reply-and-current-authority'):
            report.checkpoint(tasks=tasks(binary, work, state, saved, schema))
        with report.phase('interrupted-snapshot-completion-marker'):
            report.checkpoint(interruption=interruption(binary, work, state, schema))
        with report.phase('schema-six-historical-regression'):
            # Execute in this process so the shared deadline unwinds every
            # owned Server context, rather than killing a driver with children.
            spec = importlib.util.spec_from_file_location('schema_six', ROOT / 'scripts/check-schema-six.py')
            module = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(module)
            module.qualify(six, binary=binary)
        assert digest(binary) == current_hash, 'Binary changed during qualification'
        assert all(digest(ROOT / p) == h for p, h in inputs.items()), 'Source changed during qualification'
        baseline(previous, SCHEMA10, 10)
        baseline(six, SCHEMA6, 6)
    except BaseException as caught:
        error = caught
        (work / 'failure-traceback.log').write_text(traceback.format_exc())
    finally:
        signal.setitimer(signal.ITIMER_REAL, 0)
        signal.signal(signal.SIGALRM, before)
        report.data['elapsed_seconds'] = round(time.monotonic() - started, 3)
        report.finish('failed' if error else 'passed', work / 'legacy', error)
    print(('FAIL' if error else 'PASS') + ' Gate 4 recovery: ' + str(work / 'result.json'))
    return 1 if error else 0


if __name__ == '__main__':
    sys.exit(main())
