#!/usr/bin/env python3
"""Run bounded Gate 4 offline qualification; live providers are always separate."""
import argparse
import datetime
import json
import os
import pathlib
import platform
import signal
import subprocess
import sys

from lib.gate4_phases import phases
from lib.local_process import ROOT
from lib.qualification_process import run_phase
from lib.qualification_report import check_test_results, environment, save, source_inputs, versions


def interrupted(_signal, _frame):
    raise InterruptedError('Qualification was interrupted')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--codex-binary', type=pathlib.Path,
                        help='Explicit installed Codex; scripted loopback Responses, no model account')
    parser.add_argument('--source-only', action='store_true',
                        help='CI source coverage only; omit external host checks and report partial qualification')
    parser.add_argument('--list', action='store_true', help='List coverage/budgets without running checks')
    args = parser.parse_args()
    steps = phases(args.source_only)
    if args.list:
        print(json.dumps(steps, indent=2))
        return 0
    os.umask(0o077)
    directory = ROOT / '.local/gate-4-checks' / datetime.datetime.now(
        datetime.timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ')
    directory.mkdir(parents=True)
    report = {'scope': 'local macOS arm64; offline fixtures, no live providers or deployment',
              'mode': 'source-only' if args.source_only else 'complete-offline',
              'status': 'running', 'steps': [], 'required_steps': [s[0] for s in steps],
              'live_qualification': 'not run; required separately for whole Gate 4 closure',
              'external_host': 'omitted; source-only qualification' if args.source_only else 'required',
              'build_profile': 'dev/test with debug assertions; debug symbols and incremental caching disabled',
              'started_at': datetime.datetime.now(datetime.timezone.utc).isoformat()}
    save(directory, report)
    print('Private Gate 4 results:', directory, flush=True)
    previous = signal.signal(signal.SIGTERM, interrupted)
    try:
        if (platform.system(), platform.machine()) != ('Darwin', 'arm64'):
            raise RuntimeError('Only local macOS arm64 is qualified')
        if sys.flags.optimize:
            raise RuntimeError('Do not disable verification assertions with Python optimization')
        if args.source_only and args.codex_binary:
            raise ValueError('Choose full host qualification or explicit source-only scope')
        codex = args.codex_binary.resolve() if args.codex_binary else None
        if not args.source_only and (codex is None or not codex.is_file()):
            raise ValueError('Full offline qualification requires --codex-binary; no implicit host omission')
        env = environment(codex)
        report['versions'] = dict(versions(env, codex), python=sys.version, system=platform.platform())
        report['source_revision'] = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
        report['source_inputs_sha256'] = source_inputs()
        report['dirty'] = bool(subprocess.check_output(['git', 'status', '--porcelain'], cwd=ROOT))
        save(directory, report)
        for name, command, timeout in steps:
            if source_inputs() != report['source_inputs_sha256']:
                raise RuntimeError('Source changed before phase; retain results and qualify stable source')
            report['active_phase'] = name
            save(directory, report)
            print('RUN', name, flush=True)
            result = run_phase(name, command, timeout, directory, ROOT, env)
            report['steps'].append(result)
            try:
                check_test_results(result, directory)
            except (ValueError, OSError) as error:
                result.update(status='failed', verification_error=str(error))
            report.pop('active_phase', None)
            save(directory, report)
            print(result['status'].upper(), name, f"({result['seconds']}s)", flush=True)
            if result['status'] != 'passed':
                raise RuntimeError(f'Phase failed: {name}; see private log')
        if source_inputs() != report['source_inputs_sha256']:
            raise RuntimeError('Source changed during qualification')
        report['source_stable'] = True
        report['status'] = 'passed-source-only' if args.source_only else 'passed'
    except BaseException as error:
        report.update(status='failed', error={'type': type(error).__name__, 'detail': str(error)})
        completed = {s['name'] for s in report['steps']}
        report['not_run'] = [name for name, _, _ in steps if name not in completed]
    finally:
        signal.signal(signal.SIGTERM, previous)
        report['finished_at'] = datetime.datetime.now(datetime.timezone.utc).isoformat()
        save(directory, report)
    print(report['status'].upper(), 'Gate 4 offline:', directory / 'result.json', flush=True)
    return 1 if report['status'] == 'failed' else 0


if __name__ == '__main__':
    sys.exit(main())
