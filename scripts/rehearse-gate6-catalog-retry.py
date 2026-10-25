#!/usr/bin/env python3
"""Controlled current-catalog contention and retained subscription verification."""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
PREFIX = 'peer_native::canonical_media::native_offers::future_subscriptions::'
REGRESSION = PREFIX + 'actual_catalog_contention_rereads_unchanged_sources_without_a_new_reconciliation'
OUTCOME = PREFIX + 'actual_failed_routing_replays_the_exact_error_with_metadata_only_custody'
FUTURE = [
    PREFIX + 'actual_future_participant_video_and_screen_reconcile_after_original_command_grant_releases',
    PREFIX + 'actual_future_selection_rechecks_denial_then_reuses_operation_after_bad_confirmation_and_queued_writer',
    PREFIX + 'actual_future_data_receiver_negotiates_only_with_supported_physical_profile',
]
CLASSIFIER = 'v2::native_subscriptions::catalog_tests::catalog_contention_does_not_reclassify_missing_stale_or_resource_capacity'


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def sources():
    paths = [ROOT / name for name in (
        'scripts/rehearse-gate6-catalog-retry.py', 'scripts/prepare-dependencies.py',
        'scripts/lib/prepared_archive.py', 'dependencies/source-lock.json',
        '.deps/rvoip/Cargo.toml', '.deps/rvoip/Cargo.lock')]
    for path in (ROOT / '.deps/rvoip/crates').rglob('*'):
        if path.is_file() and path.suffix in ('.rs', '.toml'):
            paths.append(path)
    lock = json.loads((ROOT / 'dependencies/source-lock.json').read_text())
    paths.extend(ROOT / item['path'] for item in lock['patches'])
    return {str(p.relative_to(ROOT)): digest(p) for p in sorted(set(paths))}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--target-dir', required=True, type=Path)
    parser.add_argument('--expect-regression', action='store_true',
                        help='Historical tests-only patch phase: expect exactly the unchanged-catalog assertion to fail. No passing test claim.')
    parser.add_argument('--regression', choices=('catalog', 'outcome'), default='catalog', help='Select the exact historical negative assertion; positive runs cover both regressions and retained subscription behavior.')
    args = parser.parse_args()
    directory = args.output.resolve()
    if not directory.is_relative_to((ROOT / '.local').resolve()):
        parser.error('Keep private evidence under ignored .local/')
    directory.mkdir(parents=True, exist_ok=False)
    (directory / 'tmp').mkdir()
    env = dict(os.environ, TMPDIR=str(directory / 'tmp'), CARGO_INCREMENTAL='0', CARGO_PROFILE_RELEASE_LTO='false',
               CARGO_PROFILE_RELEASE_CODEGEN_UNITS='16', CARGO_PROFILE_RELEASE_DEBUG='0',
               CARGO_PROFILE_RELEASE_PANIC='unwind')
    report = dict(passed=False, verified_expected_failure=False,
                  started_at=datetime.datetime.now(datetime.timezone.utc).isoformat(),
                  source_revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
                  host=platform.platform(), expect_regression=args.expect_regression, regression=args.regression,
                  scope='Controlled actual upstream QUIC/WT subscription/catalog behavior under its locked workspace and recorded release profile. No Connect host/browser media throughput, native SDK, full G6-050 or acceptance qualification.',
                  profile={key: env[key] for key in ('CARGO_INCREMENTAL', 'CARGO_PROFILE_RELEASE_LTO', 'CARGO_PROFILE_RELEASE_CODEGEN_UNITS', 'CARGO_PROFILE_RELEASE_DEBUG', 'CARGO_PROFILE_RELEASE_PANIC')},
                  steps=[], cases=[])
    before = sources()
    report['source_inputs_sha256'] = before
    executables = []

    def step(name, command, json_output=False):
        start = time.monotonic()
        log = directory / (name + '.log')
        with log.open('w') as err:
            if json_output:
                with (directory / (name + '.jsonl')).open('w') as output:
                    result = subprocess.run(command, cwd=ROOT, env=env, stdout=output, stderr=err)
            else:
                result = subprocess.run(command, cwd=ROOT, env=env, stdout=err, stderr=subprocess.STDOUT)
        report['steps'].append(dict(name=name, command=command, exit_code=result.returncode,
                                    elapsed_seconds=time.monotonic() - start, log_sha256=digest(log)))
        print(json.dumps(dict(step=name, exit_code=result.returncode)), flush=True)
        assert result.returncode == 0, name

    try:
        step('dependencies', ['bash', 'scripts/prepare-dependencies.sh'])
        step('format', ['bash', 'scripts/format.sh', '--check'])
        step('rustc', ['rustc', '--version', '--verbose'])
        test_targets = ['--test', 'v2_peer_admission'] + ([] if args.expect_regression else ['--lib'])
        step('build', ['cargo', 'test', '--manifest-path', '.deps/rvoip/Cargo.toml',
                       '-p', 'rvoip-uctp', *test_targets, '--no-run',
                       '--release', '--locked', '--offline', '-j2', '--target-dir',
                       str(args.target_dir.resolve()), '--message-format=json'], True)
        rows = [json.loads(line) for line in (directory / 'build.jsonl').read_text().splitlines()]
        for name in (('v2_peer_admission',) if args.expect_regression else ('v2_peer_admission', 'rvoip_uctp')):
            found = [r for r in rows if r.get('reason') == 'compiler-artifact' and r.get('executable')
                     and r['target']['name'] == name and r['profile']['test']]
            assert len(found) == 1, name
            path = Path(found[0]['executable'])
            executables.append(dict(target=name, path=str(path), sha256=digest(path)))
        report['executables'] = executables
        selected = [(REGRESSION if args.regression == 'catalog' else OUTCOME, 'v2_peer_admission')]
        if not args.expect_regression:
            selected.append((OUTCOME if args.regression == 'catalog' else REGRESSION, 'v2_peer_admission'))
        if not args.expect_regression:
            selected.extend((name, 'v2_peer_admission') for name in FUTURE)
            selected.append((CLASSIFIER, 'rvoip_uctp'))
        for index, (name, target) in enumerate(selected):
            path = next(e['path'] for e in executables if e['target'] == target)
            log = directory / ('case-' + str(index) + '.log')
            start = time.monotonic()
            with log.open('w') as output:
                result = subprocess.run([path, name, '--exact', '--test-threads=1', '--nocapture'],
                                        cwd=ROOT, env=env, stdout=output, stderr=subprocess.STDOUT, timeout=90)
            text = log.read_text()
            summary = re.search(r'test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored', text)
            normal = result.returncode == 0 and summary and tuple(map(int, summary.group(2, 3, 4))) == (1, 0, 0)
            expected = args.expect_regression and result.returncode == 101 and summary and tuple(map(int, summary.group(2, 3, 4))) == (0, 1, 0) and ('A busy read followed by the identical current catalog must not start another reconciliation' if args.regression == 'catalog' else 'metadata-only retry replaced the original Conflict outcome with another engine result') in text
            case = dict(case=name, exit_code=result.returncode, passed=bool(normal),
                        expected_failure=bool(expected), log=str(log.relative_to(ROOT)),
                        log_sha256=digest(log), elapsed_seconds=time.monotonic() - start)
            report['cases'].append(case)
            print(json.dumps(case), flush=True)
        step('dependencies-after', ['bash', 'scripts/prepare-dependencies.sh'])
    except Exception as error:
        report['failure'] = str(error)
    finally:
        report['source_inputs_unchanged'] = before == sources()
        report['executables_unchanged'] = bool(executables) and all(digest(Path(e['path'])) == e['sha256'] for e in executables)
        guarded = report['source_inputs_unchanged'] and report['executables_unchanged'] and 'failure' not in report
        report['passed'] = bool(not args.expect_regression and guarded and len(report['cases']) == 6 and all(c['passed'] for c in report['cases']))
        report['verified_expected_failure'] = bool(args.expect_regression and guarded and len(report['cases']) == 1 and report['cases'][0]['expected_failure'])
        report['finished_at'] = datetime.datetime.now(datetime.timezone.utc).isoformat()
        (directory / 'summary.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(dict(passed=report['passed'], verified_expected_failure=report['verified_expected_failure'], summary=str((directory / 'summary.json').relative_to(ROOT)))), flush=True)
    raise SystemExit(0 if report['passed'] or report['verified_expected_failure'] else 1)


if __name__ == '__main__':
    main()
