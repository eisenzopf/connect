#!/usr/bin/env python3
"""Actual interceptor startup-clock component; no media/runtime qualification."""
from pathlib import Path
import argparse
import hashlib
import json
import os
import platform
import re
import runpy
import shutil
import subprocess
import tomllib

ROOT = Path(__file__).resolve().parents[1]
BOUNDED = runpy.run_path(str(ROOT / 'scripts/rehearse-gate6-turn-relay.py'))['bounded_child']
INPUTS = runpy.run_path(str(ROOT / 'scripts/check-gate6-ice-restart.py'))['inputs']


def digest(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--expect-missing-prompt-report', action='store_true')
    args = parser.parse_args()
    assert platform.system() == 'Linux' and platform.machine() == 'x86_64'
    output = args.output.resolve()
    assert output.is_relative_to(ROOT / '.local')
    output.mkdir(parents=True, mode=0o700, exist_ok=False)
    subprocess.run(['bash', 'scripts/prepare-dependencies.sh'], cwd=ROOT, check=True)
    source = output / 'probe'
    (source / 'src').mkdir(parents=True)
    (source / 'Cargo.toml').write_text(
        '[package]\nname="g6-first-source-report-probe"\nversion="0.0.0"\nedition="2024"\n'
        '[workspace]\n[dependencies]\nrtc-interceptor={path=' + json.dumps(str(ROOT / '.deps/rtc-interceptor')) + '}\n'
        'rtp={package="rtc-rtp",version="=0.20.0-alpha.1"}\n'
        'rtcp={package="rtc-rtcp",version="=0.20.0-alpha.1"}\nsansio="1"\n'
        '[profile.release]\nopt-level=3\ndebug=0\n'
        '[profile.release.package.g6-first-source-report-probe]\nopt-level=0\n')
    shutil.copyfile(ROOT / 'scripts/tests/rtc_source_clock_startup.rs', source / 'src/lib.rs')
    shutil.copyfile(ROOT / 'Cargo.lock', source / 'Cargo.lock')
    before = INPUTS()
    manifest = output / 'source-inputs.json'
    manifest.write_text(json.dumps(before, indent=2) + '\n')
    report = dict(scope=__doc__, expected_counterexample=args.expect_missing_prompt_report,
        source_revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
        source_count=len(before), source_manifest_sha256=digest(manifest),
        full_task_qualified=False, acceptance_qualified=False, tests_executed=False,
        expected_counterexample_reproduced=False, qualification_passed=False)
    success = False
    try:
        environment = dict(os.environ, CARGO_PROFILE_RELEASE_DEBUG='0', CARGO_PROFILE_RELEASE_OPT_LEVEL='3',
            CARGO_INCREMENTAL='0', CARGO_BUILD_JOBS='1')
        metadata = BOUNDED(['cargo', 'metadata', '--offline', '--format-version', '1', '--manifest-path',
            str(source / 'Cargo.toml')], environment, output / 'metadata-private.log', 60)
        report['metadata'] = metadata
        assert metadata['exit_code'] == 0 and not metadata['timed_out'] and not metadata['forced_kill']
        root_lock = tomllib.loads((ROOT / 'Cargo.lock').read_text())
        expected = {(p['name'], p['version'], p.get('source')):p.get('checksum') for p in root_lock['package']}
        probe_lock = tomllib.loads((source / 'Cargo.lock').read_text())
        registry = [p for p in probe_lock['package'] if p.get('source', '').startswith('registry+')]
        assert all(expected.get((p['name'], p['version'], p['source'])) == p.get('checksum')
            and (p['name'], p['version'], p['source']) in expected for p in registry), 'Unpinned probe dependency'
        report['registry_packages_matched_root_lock'] = len(registry)
        generated = {str(p.relative_to(output)):digest(p) for p in
            [source / 'Cargo.toml', source / 'Cargo.lock', source / 'src/lib.rs']}
        report['generated_inputs_sha256'] = generated
        report['rustc'] = subprocess.check_output(['rustc', '-Vv'], cwd=ROOT, text=True).strip()
        log = output / 'test-private.log'
        test = BOUNDED(['cargo', 'test', '--release', '--offline', '--locked', '--manifest-path',
            str(source / 'Cargo.toml'), '--lib', '--', '--test-threads=1'], environment, log, 900)
        report['test'] = test
        text = log.read_text(errors='replace')
        results = re.findall(r'^test result: (ok|FAILED)\. (\d+) passed; (\d+) failed;', text, re.M)
        report['test_results'] = results
        report['tests_executed'] = bool(results)
        complete = not test['timed_out'] and not test['forced_kill']
        reproduced = complete and test['exit_code'] == 101 and results == [('FAILED', '2', '1')] \
            and 'first source mapping waits for the periodic timer' in text
        passed = complete and test['exit_code'] == 0 and results == [('ok', '3', '0')]
        report['expected_counterexample_reproduced'] = reproduced
        report['qualification_passed'] = passed
        report['generated_inputs_unchanged'] = all(digest(output / name) == sha for name, sha in generated.items())
        success = (reproduced if args.expect_missing_prompt_report else passed) and report['generated_inputs_unchanged']
        report['status'] = 'counterexample-reproduced' if reproduced else 'component-passed' if passed else 'failed-or-unrun'
    except Exception as error:
        report['status'] = 'failed-or-unrun'
        report['error_class'] = type(error).__name__ if type(error).__name__ in ['AssertionError', 'TimeoutExpired'] else 'other'
    finally:
        report['sources_unchanged'] = INPUTS() == before
        success = success and report['sources_unchanged']
        (output / 'summary.json').write_text(json.dumps(report, indent=2) + '\n')
        print(json.dumps({key:report.get(key) for key in ['status', 'tests_executed',
            'expected_counterexample_reproduced', 'qualification_passed', 'sources_unchanged']}))
    if not success:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
