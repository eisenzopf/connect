#!/usr/bin/env python3
"""Run explicitly selected, bounded local Gate5 lanes; whole-gate claims stay separate."""
import argparse
import datetime
import hashlib
import json
import os
import pathlib
import platform
import re
import shutil
import signal
import subprocess
import sys

from lib.gate5_phases import EXTERNAL_MEDIA_TESTS, LANES, UPSTREAM_MEDIA_TEST_COUNTS, phases
from lib.local_process import ROOT
from lib.media_failure_proof import EXTERNAL_FAILURE_TESTS, verify_external_failures
from lib.qualification_process import run_phase
from lib.qualification_report import IGNORED, check_test_results, environment, save, source_inputs, versions


def interrupted(_signal, _frame):
    raise InterruptedError('Qualification interrupted')


def run_plan(steps, report, directory, env, minimum_free_bytes=4 * 1024 ** 3,
             snapshot=source_inputs, disk=shutil.disk_usage):
    """Retain every failed attempt and mark later phases explicitly not run."""
    save(directory, report)
    try:
        for step in steps:
            if snapshot() != report['source_inputs_sha256']:
                raise RuntimeError('Source changed before phase')
            free = disk(ROOT).free
            report['disk_free_bytes'] = free
            if free < minimum_free_bytes:
                raise RuntimeError('Insufficient disk headroom before phase')
            report['active_phase'] = step['name']
            save(directory, report)
            print('RUN', step['name'], flush=True)
            phase_env = dict(env)
            if step['lane'] == 'actual-model':
                # Only explicitly selected actual inference receives model credentials.
                phase_env.update({key: os.environ[key] for key in ('CODEX_HOME', 'CODEX_API_KEY') if key in os.environ})
            result = run_phase(step['name'], step['command'], step['timeout_seconds'], directory, ROOT, phase_env)
            result['lane'] = step['lane']
            report['steps'].append(result)
            try:
                # Reuse retained test-total invariants without changing phase identity.
                original = result['name']
                result['name'] = original.split('--', 1)[-1]
                check_test_results(result, directory,
                    allowed_ignored=IGNORED | {'isolated_native_keychain', 'native_keychain_child'}
                    | set(EXTERNAL_MEDIA_TESTS) | EXTERNAL_FAILURE_TESTS)
                expected = {'gate5-driver-tests': 21, 'independent-vector-tests': 6,
                            'video-measurement-tests': 57, 'grant-boundary-tests': 6,
                            'native-example-cleanup-guards': 13, 'package-proof-tests': 29,
                            'registry-license-tests': 9,
                            'prepared-archive-tests': 6,
                            'connected-app-checker-tests': 3}.get(result['name'])
                text = (directory / result['log']).read_text(errors='replace')
                if result['status'] == 'passed' and expected is not None:
                    if re.findall(r'^Ran (\d+) tests? in ', text, re.M) != [str(expected)] or '\nOK\n' not in text or 'skipped=' in text:
                        raise ValueError('Required Gate5 unit phase did not execute every check')
                    result['python_tests'] = {'passed': expected, 'failed': 0, 'ignored': 0}
                required_rust = UPSTREAM_MEDIA_TEST_COUNTS.get(result['name'])
                if result['status'] == 'passed' and required_rust is not None:
                    counts = result.get('rust_tests', {})
                    if (counts.get('passed', 0) < required_rust or counts.get('failed') != 0
                            or counts.get('ignored') != 0):
                        raise ValueError('Required upstream media checks did not execute with their feature flags')
                if result['status'] == 'passed' and result['name'] == 'external-runtime-media-failures':
                    result['external_runtime_failures'] = verify_external_failures(result, directory,
                        pathlib.Path(env['CONNECT_TEST_BINARY']), ROOT, env['CONNECT_SOURCE_REVISION'])
                if result['status'] == 'passed' and result['name'] == 'typescript-public-media-tests':
                    totals = {name: int(value) for name, value in re.findall(
                        r'^# (tests|pass|fail|cancelled|skipped|todo) (\d+)$', text, re.M)}
                    if (totals.get('tests', 0) < 206 or totals.get('pass') != totals['tests']
                            or any(totals.get(name) != 0 for name in ('fail', 'cancelled', 'skipped', 'todo'))):
                        raise ValueError('Public SDK media/lifecycle/authority suites did not all execute')
                    result['typescript_tests'] = totals
                if result['status'] == 'passed' and result['name'] == 'isolated-native-keychain':
                    if result.get('rust_tests') != {'passed': 2, 'failed': 0, 'ignored': 0}:
                        raise ValueError('Both isolated parent and actual native child must execute')
                if result['status'] == 'passed' and result['name'] == 'external-decoder-fixtures':
                    if result.get('rust_tests') != {'passed': 3, 'failed': 0, 'ignored': 0}:
                        raise ValueError('All three explicit decoder fixtures must execute')
            except (ValueError, OSError) as error:
                result.update(status='failed', verification_error=str(error))
            finally:
                result['name'] = original
            report.pop('active_phase', None)
            save(directory, report)
            print(result['status'].upper(), step['name'], flush=True)
            if result['status'] != 'passed':
                raise RuntimeError('Phase failed: ' + step['name'])
        if snapshot() != report['source_inputs_sha256']:
            raise RuntimeError('Source changed during qualification')
        report['source_stable'] = True
        report['status'] = 'passed-selected-lanes'
    except BaseException as error:
        report.update(status='failed', error={'type': type(error).__name__, 'detail': str(error)})
    finally:
        complete = {s['name'] for s in report['steps']}
        report['not_run'] = [s['name'] for s in steps if s['name'] not in complete]
        report['whole_gate_qualified'] = False
        report['finished_at'] = datetime.datetime.now(datetime.timezone.utc).isoformat()
        save(directory, report)
    return 1 if report['status'] == 'failed' else 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--lane', action='append', choices=LANES,
                        help='Select repeatable lanes; default source + independent-client')
    parser.add_argument('--source-only', action='store_true', help='Select source lane only; explicit partial CI proof')
    parser.add_argument('--codex-binary', type=pathlib.Path)
    parser.add_argument('--candidate', type=pathlib.Path)
    parser.add_argument('--independent-python', type=pathlib.Path,
                        help='Prepared pinned aioquic executable for installed qualification')
    parser.add_argument('--node-binary', type=pathlib.Path)
    parser.add_argument('--ffmpeg-binary', type=pathlib.Path)
    parser.add_argument('--chrome-binary', type=pathlib.Path)
    parser.add_argument('--playwright-module', type=pathlib.Path)
    parser.add_argument('--runtime', type=pathlib.Path, help='Extracted installed runtime for separately selected actual-model lane')
    parser.add_argument('--qualification-report', type=pathlib.Path)
    parser.add_argument('--allow-model-inference', action='store_true')
    parser.add_argument('--list', action='store_true')
    args = parser.parse_args()
    if args.source_only and args.lane:
        parser.error('--source-only and --lane are mutually exclusive')
    lanes = ['source'] if args.source_only else args.lane or ['source', 'independent-client']
    if len(set(lanes)) != len(lanes):
        parser.error('Duplicate lanes are not allowed')
    if args.allow_model_inference and 'actual-model' not in lanes:
        parser.error('Inference flag requires the actual-model lane')
    tools = {'node': args.node_binary, 'ffmpeg': args.ffmpeg_binary,
             'chrome': args.chrome_binary, 'playwright': args.playwright_module}
    if any(tools[key] for key in ('node', 'chrome', 'playwright')) and not all(tools.values()):
        parser.error('Select all four media tool inputs together')
    media_tools = {name: path.resolve(strict=True) for name, path in tools.items()} if all(tools.values()) else None
    if media_tools and any(not path.is_file() for path in media_tools.values()):
        parser.error('Select ordinary tool files, including the Playwright module entry file (index.mjs)')
    source_revision = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
    steps, unresolved = phases(lanes, args.codex_binary, args.candidate, args.runtime,
        args.qualification_report, args.allow_model_inference,
        independent_python=args.independent_python, media_tools=media_tools,
        source_revision=source_revision, native_ffmpeg=args.ffmpeg_binary.resolve(strict=True) if args.ffmpeg_binary else None)
    if args.list:
        print(json.dumps({'lanes': lanes, 'steps': steps, 'unresolved': unresolved,
                          'whole_gate_qualified': False}, indent=2))
        return 0
    os.umask(0o077)
    directory = ROOT / '.local/gate-5-checks' / datetime.datetime.now(
        datetime.timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ')
    directory.mkdir(parents=True, mode=0o700)
    report = {'schema_version': 1, 'scope': 'local macOS arm64; providers disabled; no deployment or public ingress',
              'status': 'running', 'selected_lanes': lanes, 'steps': [], 'unresolved': unresolved,
              'whole_gate_qualified': False, 'required_steps': [s['name'] for s in steps],
              'started_at': datetime.datetime.now(datetime.timezone.utc).isoformat(),
              'source_inputs_sha256': source_inputs(),
              'build_profile': 'dev/test; debug assertions enabled; no debug symbols or incremental cache',
              'model_scope': 'explicit actual inference' if 'actual-model' in lanes and args.allow_model_inference else 'not run',
              'provider_scope': 'historical evidence must be reviewed separately; no live provider operations'}
    print('Private Gate5 results:', directory, flush=True)
    save(directory, report)
    previous = signal.signal(signal.SIGTERM, interrupted)
    try:
        if (platform.system(), platform.machine()) != ('Darwin', 'arm64'):
            raise RuntimeError('Only local macOS arm64 is qualified')
        if sys.flags.optimize:
            raise RuntimeError('Verification assertions must remain enabled')
        selected_missing = [u for u in unresolved if u['lane'] in lanes and u['lane'] != 'operating']
        if selected_missing:
            raise ValueError('Selected lane prerequisites are unresolved')
        env = environment(args.codex_binary)
        env['CONNECT_SOURCE_REVISION'] = source_revision
        if media_tools:
            env.update(CONNECT_CHROME_BINARY=str(media_tools['chrome']),
                       CONNECT_FFMPEG_BINARY=str(media_tools['ffmpeg']),
                       CONNECT_PLAYWRIGHT_MODULE=str(media_tools['playwright']),
                       CONNECT_NODE_BINARY=str(media_tools['node']),
                       CONNECT_VIDEO_BINARY=str(ROOT / 'target/debug/connect-video'))
        report['versions'] = dict(versions(env, args.codex_binary), python=sys.version, system=platform.platform())
        report['source_revision'] = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
        report['dirty'] = bool(subprocess.check_output(['git', 'status', '--porcelain'], cwd=ROOT))
        if args.candidate:
            sums = args.candidate / 'SHA256SUMS'
            report['candidate'] = {'name': args.candidate.name,
                                   'checksums_file_sha256': hashlib.sha256(sums.read_bytes()).hexdigest()}
        if args.runtime:
            report['runtime_manifest_sha256'] = hashlib.sha256((args.runtime / 'manifest.json').read_bytes()).hexdigest()
        return run_plan(steps, report, directory, env)
    except BaseException as error:
        report.update(status='failed', error={'type': type(error).__name__, 'detail': str(error)},
                      not_run=[s['name'] for s in steps],
                      finished_at=datetime.datetime.now(datetime.timezone.utc).isoformat())
        save(directory, report)
        return 1
    finally:
        signal.signal(signal.SIGTERM, previous)
        print(report['status'].upper(), directory / 'result.json', flush=True)


if __name__ == '__main__':
    sys.exit(main())
