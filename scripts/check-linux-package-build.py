#!/usr/bin/env python3
"""Build/package/recovery evidence with raw output retained privately, not uploaded."""
import argparse
import hashlib
import json
import os
import re
from pathlib import Path
import runpy
import subprocess
import time
from lib.package_environment import fixture_environment

ROOT = Path(__file__).resolve().parents[1]
BASE = ROOT / '.local/gate6-installation-build'
EVIDENCE = BASE / 'evidence'
PRIVATE = BASE / 'private'


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--phase', choices=('all', 'recovery', 'package'), default='all')
    args = parser.parse_args()
    os.umask(0o077)
    EVIDENCE.mkdir(parents=True, exist_ok=True)
    PRIVATE.mkdir(parents=True, exist_ok=True)
    source = runpy.run_path(str(ROOT / 'scripts/check-gate6-linux-hosting.py'))['sources']
    source_path = EVIDENCE / 'source-inputs.json'
    report_path = EVIDENCE / 'build.json'
    head = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
    expected = os.environ.get('CONNECT_INSTALLATION_SOURCE_REVISION', head)
    assert re.fullmatch('[0-9a-f]{40}', expected) and expected == head, 'Checkout differs from requested exact source'
    if args.phase == 'package':
        inputs = json.loads(source_path.read_text())
        report = json.loads(report_path.read_text())
        assert report['source_revision'] == head and report['requested_source_revision'] == expected
        assert report['controller_source_revision'] == os.environ.get('GITHUB_SHA')
        assert report['source_inputs_sha256'] == digest(source_path) and report['source_count'] == len(inputs), 'Recovery source manifest changed between phases'
    else:
        inputs = source()
        source_path.write_text(json.dumps(inputs, sort_keys=True) + '\n')
        report = {'source_revision': head, 'requested_source_revision': expected,
                  'controller_source_revision': os.environ.get('GITHUB_SHA'),
                  'source_inputs_sha256': digest(source_path), 'source_count': len(inputs),
                  'steps': [], 'passed': False, 'compilation_completed': False, 'whole_gate_qualified': False}
    env = fixture_environment()
    try:
        assert inputs == source(), 'Source changed between build phases'
        if args.phase == 'package':
            assert len(report['steps']) == 1 and report['steps'][0]['name'] == 'recovery-tests'
            if report['steps'][0]['exit_code'] != 0 or not report['compilation_completed']:
                raise RuntimeError('Recovery did not pass; package assembly remains unrun')
        for name, command, budget in [
            ('recovery-tests', ['cargo', 'test', '--locked', '--release', '--message-format=json', '-p', 'connect-installation-conformance', '--test', 'gate6_canonical_recovery', '--', '--test-threads=1'], 5400),
            ('package', ['python3', 'scripts/package-linux.py'], 5400),
        ]:
            if (args.phase == 'package' and name == 'recovery-tests') or (args.phase == 'recovery' and name == 'package'):
                continue
            started = time.monotonic()
            log = PRIVATE / (name + '.log')
            try:
                with log.open('w') as output:
                    result = subprocess.run(command, cwd=ROOT, stdout=output, stderr=subprocess.STDOUT, env=env, timeout=budget)
                status = result.returncode
            except subprocess.TimeoutExpired:
                status = 'timeout'
            step = {'name': name, 'exit_code': status, 'elapsed_seconds': round(time.monotonic() - started, 3)}
            if name == 'recovery-tests':
                lines = log.read_text().splitlines()
                step['test_results'] = [line for line in lines if line.startswith('test result:')]
                step['failed_tests'] = [line for line in lines if line.startswith('test ') and line.endswith(' ... FAILED')]
                # Retain source coordinates, never assertion values, config or keys.
                step['failure_locations'] = []
                for line in lines:
                    location = re.search(r'panicked at ([^\s]+\.rs):(\d+):(\d+)', line)
                    if location:
                        step['failure_locations'].append({'file': location[1], 'line': int(location[2]), 'column': int(location[3])})

                step['compiler_errors'] = []
                step['test_executables'] = []
                for line in lines:
                    try:
                        event = json.loads(line)
                    except ValueError:
                        continue
                    if event.get('reason') == 'compiler-artifact' and event.get('executable') and event.get('profile', {}).get('test'):
                        step['test_executables'].append({'name': event['target']['name'], 'sha256': digest(Path(event['executable']))})
                    if event.get('reason') == 'compiler-message' and event['message']['level'] == 'error':
                        message = event['message']
                        step['compiler_errors'].append({'message': message['message'], 'code': (message.get('code') or {}).get('code'), 'locations': [{'file': span['file_name'], 'line': span['line_start']} for span in message['spans'] if span['is_primary']]})
            if name == 'recovery-tests' and any(p['name'] == 'gate6_canonical_recovery' for p in step['test_executables']) and not step['compiler_errors']:
                report['compilation_completed'] = True
            if name == 'recovery-tests' and status == 0:
                assert len(step['test_results']) == 1 and re.fullmatch(r'test result: ok\. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in .+', step['test_results'][0])
                assert any(p['name'] == 'gate6_canonical_recovery' for p in step['test_executables'])
                report['compilation_completed'] = True
            report['steps'].append(step)
            print(json.dumps(step), flush=True)
            if status != 0:
                raise RuntimeError('Bounded build step failed; raw diagnostics retained privately')
        if args.phase == 'recovery':
            return
        packages = list((ROOT / 'dist').glob('connect-*-ubuntu-24.04-x86_64-*.tar.gz'))
        assert len(packages) == 1
        package = packages[0]
        report['archive'] = package.name
        report['archive_sha256'] = digest(package)
        report['package_manifest_sha256'] = digest(package.with_suffix('').with_suffix('') / 'manifest.json')
        report['source_inputs_unchanged'] = inputs == source()
        assert report['source_inputs_unchanged']
        report['passed'] = True
        with open(os.environ['GITHUB_OUTPUT'], 'a') as output:
            output.write('package=' + str(package.with_suffix('').with_suffix('')) + '\n')
    finally:
        report['source_inputs_unchanged'] = inputs == source()
        report['cacheable_build'] = report['compilation_completed'] and report['source_inputs_unchanged']
        if os.environ.get('GITHUB_OUTPUT'):
            with open(os.environ['GITHUB_OUTPUT'], 'a') as output:
                output.write('cacheable_build=' + str(report['cacheable_build']).lower() + '\n')
        report_path.write_text(json.dumps(report, indent=2) + '\n')


if __name__ == '__main__':
    main()
