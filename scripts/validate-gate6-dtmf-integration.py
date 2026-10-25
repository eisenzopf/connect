#!/usr/bin/env python3
"""Validate a completed actual Connect DTMF run and every capture family."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import runpy
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('run_dir', type=Path)
    parser.add_argument('--output', type=Path)
    args = parser.parse_args()
    directory = args.run_dir.resolve()
    assert directory.is_relative_to((ROOT / '.local').resolve())
    output = (args.output or directory / 'post-validation' / 'summary.json').resolve()
    assert output.is_relative_to((ROOT / '.local').resolve())
    output.parent.mkdir(parents=True, exist_ok=True)
    summary_path = directory / 'summary.json'
    runtime = json.loads(summary_path.read_bytes())
    assert runtime['source_inputs_unchanged'] is True
    assert runtime['executable_unchanged'] is True
    cases = runtime['cases']
    assert len(cases) == len(runtime['selected_cases']) == 48
    assert {r['case'] for r in cases} == set(runtime['selected_cases'])
    for row in cases:
        assert row['passed'] is True and row['exit_code'] == 0, row['case']
        log = ROOT / row['log']
        assert log.resolve().is_relative_to(directory)
        assert digest(log) == row['log_sha256']
        assert re.search(r'test result: ok\. 1 passed; 0 failed; 0 ignored;', log.read_text())
    binary = Path(runtime['executable'])
    assert digest(binary) == runtime['executable_sha256']
    sources = runpy.run_path(str(ROOT / 'scripts/rehearse-gate6-fifty-participants.py'))['sources']
    before = sources()
    # Only this post-run verifier may be added after the recorded runtime.
    me = str(Path(__file__).resolve().relative_to(ROOT))
    recorded_sources = runtime['source_inputs_sha256']
    if me in recorded_sources:
        assert before == recorded_sources
    else:
        assert {k: v for k, v in before.items() if k != me} == recorded_sources
    captured = {str(p.relative_to(directory)): digest(p) for p in directory.iterdir()
                if p.suffix in ('.json', '.jsonl', '.log') and p.is_file()}
    result = {'passed': False, 'scope': runtime['scope'],
              'post_validator_sha256': digest(Path(__file__)),
              'post_validation_sources_sha256': before,
              'runtime_summary': str(summary_path.relative_to(ROOT)),
              'runtime_summary_sha256': digest(summary_path),
              'runtime_source_revision': runtime['source_revision'],
              'runtime_source_inputs_unchanged': True,
              'runtime_cases_passed': len(cases), 'runtime_cases_ignored': 0,
              'executable_sha256': runtime['executable_sha256'],
              'captured_inputs_sha256': captured, 'validations': []}
    def observation(name):
        return directory / (name + '.jsonl')
    def validate(name, script, *arguments):
        command = [sys.executable, str(ROOT / 'scripts' / script), *map(str, arguments)]
        path = output.parent / (name + '.log')
        started = time.monotonic()
        with path.open('w') as log:
            process = subprocess.run(command, cwd=ROOT, stdout=log,
                                     stderr=subprocess.STDOUT, timeout=60)
        row = {'name': name, 'command': command, 'exit_code': process.returncode,
               'elapsed_seconds': time.monotonic() - started,
               'log': str(path.relative_to(ROOT)), 'log_sha256': digest(path),
               'validator_sha256': digest(ROOT / 'scripts' / script), 'passed': False}
        if process.returncode == 0:
            row['result'] = json.loads(path.read_text())
            row['passed'] = row['result']['passed'] is True
        result['validations'].append(row)
        return row['passed']
    try:
        # Three serialized tests append two algorithm rows each. Preserve the
        # complete original file and bind the derived standard group to its
        # actual producers; validate the extra recovery rows separately below.
        original = observation('initial-output')
        lines = [line for line in original.read_bytes().splitlines() if line.strip()]
        rows = [json.loads(line) for line in lines]
        assert len(rows) == 6
        assert [(r['algorithm'], r['method']) for r in rows] == [
            ('EdDSA', 'info'), ('ES256', 'info'),
            ('EdDSA', 'info'), ('ES256', 'info'),
            ('EdDSA', 'rfc4733'), ('ES256', 'rfc4733')]
        expected = [
            'production_native_to_initial_sip_info_uses_original_call_and_never_replays',
            'production_native_to_initial_sip_requires_independent_named_receive_permission',
            'production_native_to_initial_sip_rfc4733_uses_actual_negotiated_event_profile']
        producers = [next(r for r in cases if r['case'].endswith(name)) for name in expected]
        assert [cases.index(r) for r in producers] == sorted(cases.index(r) for r in producers)
        standard = output.parent / 'initial-output-standard.jsonl'
        standard.write_bytes(b'\n'.join(lines[i] for i in [0, 1, 4, 5]) + b'\n')
        result['capture_grouping'] = {
            'original': str(original.relative_to(ROOT)), 'original_sha256': digest(original),
            'original_rows': 6, 'standard_row_indexes': [0, 1, 4, 5],
            'recovery_row_indexes': [2, 3],
            'producers_in_append_order': producers,
            'derived_standard': str(standard.relative_to(ROOT)),
            'derived_standard_sha256': digest(standard)}
        import jsonschema
        contract = json.loads((ROOT / 'contracts/openapi-operations.json').read_bytes())
        signal = jsonschema.Draft202012Validator(contract['components']['schemas']['NativeDtmfSubmission'])
        observed = set()
        for row in rows:
            completed = row['completed']
            assert completed['state'] == 'completed'
            value = completed['result']
            signal.validate(value)
            assert value['signal_source'] == 'uctp-native' and value['connection_id'] == 'initial-native'
            assert value['local_work_complete'] is True and value['local_submission_only'] is True
            assert len(value['recipients']) == row['sip_calls'] == 1
            assert value['recipients'][0]['disposition'] == 'local_submitted'
            assert value['source_observation_id'] not in observed
            observed.add(value['source_observation_id'])
        result['all_initial_output_rows_schema_checked'] = len(rows)
        validate('native-http', 'validate-gate6-auxiliary-dtmf-events.py', observation('native-http'))
        validate('initial-output', 'validate-gate6-native-sip-dtmf.py', standard,
                 '--failure-samples', observation('initial-output-failure'),
                 '--stop-samples', observation('initial-output-stops'))
        validate('initial-output-maximum', 'validate-gate6-native-sip-dtmf.py', observation('initial-output-maximum'), '--maximum')
        validate('initial-output-signal-only', 'validate-gate6-native-sip-dtmf.py', observation('initial-output-signal-only'))
        validate('initial-output-fanout', 'validate-gate6-dtmf-fanout.py', observation('initial-output-fanout'))
        validate('received-delivery', 'validate-gate6-received-dtmf-delivery.py', observation('received-delivery'), '--failure-samples', observation('received-failure'))
        for name, script in [
                ('received-authority', 'received-dtmf-authority'),
                ('received-outcome', 'received-dtmf-outcomes'),
                ('received-fanout', 'received-dtmf-fanout'),
                ('received-restart', 'received-dtmf-restart'),
                ('refresh', 'dtmf-refresh'), ('moved-output', 'moved-sip-dtmf'),
                ('moved-received', 'moved-received-dtmf')]:
            validate(name, 'validate-gate6-' + script + '.py', observation(name))
        for kind in ['initial-sip', 'established-sip']:
            validate('private-primary-' + kind, 'validate-gate6-private-sip-primary.py', observation('private-primary-' + kind), '--source-kind', kind)
            validate('retained-' + kind, 'validate-gate6-retained-sip.py', observation('retained-' + kind), '--source-kind', kind)
            label = 'received-private-initial' if kind == 'initial-sip' else 'received-private-established'
            validate(label, 'validate-gate6-received-dtmf-private.py', observation(label), '--source-kind', kind)
        for profile in ['candidate-switch', 'candidate-tail']:
            paths = [directory / (profile + '.' + alg + '.json') for alg in ['EdDSA', 'ES256']]
            extra = ['--require-pending-return'] if profile == 'candidate-tail' else []
            validate(profile, 'validate-gate6-private-sip-candidate.py', *paths, *extra)
        for algorithm in ['EdDSA', 'ES256']:
            row = json.loads((directory / ('candidate-quarantine.' + algorithm + '.json')).read_bytes())
            assert row['algorithm'] == algorithm
            assert row['identity_kind'] == row['participant_kind'] == 'ai'
            assert row['unclaimed_before_and_after_input'] is True
            assert row['marked_quarantine'] == [0, 0]
            assert all(n > 10 for n in row['marked_public_baseline'])
            assert all(n > 10 for n in row['marked_public_after_cleanup'])
            assert row['original_public_live'] is True and row['distinct_sip_calls'] == 2
            assert row['metadata_busy_preserved'] is True and row['authority_writer_hold_ms'] == 250
        result['quarantine_profiles_checked'] = 2
        result['passed'] = len(result['validations']) == 21 and all(r['passed'] for r in result['validations'])
    finally:
        result['post_validation_sources_unchanged'] = sources() == before
        result['captured_inputs_unchanged'] = all(digest(directory / name) == sha for name, sha in captured.items())
        result['executable_unchanged'] = digest(binary) == runtime['executable_sha256']
        result['passed'] = result['passed'] and result['post_validation_sources_unchanged'] and result['captured_inputs_unchanged'] and result['executable_unchanged']
        output.write_text(json.dumps(result, indent=2) + '\n')
        print(json.dumps({'passed': result['passed'], 'runtime_cases_passed': len(cases),
                          'capture_validators': len(result['validations']),
                          'summary': str(output.relative_to(ROOT))}))
    return 0 if result['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
