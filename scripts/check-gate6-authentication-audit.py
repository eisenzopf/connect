#!/usr/bin/env python3
"""Bind the reviewed G6-026 scope/clauses to observed source-bound runtime cases.

This validates the reviewed evidence mapping; it is not independent protocol
conformance and does not promote any Gate 6 acceptance criterion.
"""
import hashlib
import json
import pathlib
import re

ROOT = pathlib.Path(__file__).resolve().parents[1]


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    runtime_path = ROOT / 'docs/evidence/gate-6/es256.json'
    runtime = json.loads(runtime_path.read_text())
    assert runtime['passed'] and runtime['sources_unchanged']
    assert not runtime['missing_required_tests']
    for relative, expected in runtime['source_inputs_sha256'].items():
        assert sha(ROOT / relative) == expected, relative
    mapping_path = ROOT / 'contracts/gate6-authentication-evidence.json'
    mapping = json.loads(mapping_path.read_text())
    ledger_path = ROOT / 'contracts/uctp-gate-6-requirements.json'
    ledger = json.loads(ledger_path.read_text())
    obligations = {row['id']: row for row in ledger['requirements']
                   if 'G6-026' in row['implementation_items']}
    plan = json.loads(re.search(r'<script id="plan-data" type="application/json">(.*?)</script>',
                               (ROOT / 'docs/connect-gate-6-implementation.html').read_text(), re.S)[1])
    tasks = {row['id']: row for row in plan['tasks']}
    task = tasks['G6-026']
    assert mapping['reviewed_work'] == task['work']
    assert mapping['reviewed_verify'] == task['verify']
    assert mapping['reviewed_dependencies'] == task['depends']
    assert all(tasks[key]['status'] == 'done' for key in task['depends'])
    observed = {step['name']: set(step['tests_passed']) for step in runtime['steps']}
    clauses = {}
    for group in mapping['groups']:
        assert group['task_owned_behavior'] and group['required_cases']
        for ref in group['required_cases']:
            step, case = ref.split('#', 1)
            assert case in observed[step], ref
        for key in group['requirements']:
            assert key in obligations and key not in clauses, key
            row = obligations[key]
            clauses[key] = dict(
                section=row['section'], applicability=row['applicability'],
                source_excerpt_sha256=hashlib.sha256(row['source_excerpt'].encode()).hexdigest(),
                task_owned_behavior=group['task_owned_behavior'],
                observed_case_refs=group['required_cases'],
                remaining_shared_scope=group['remaining_shared_scope'])
    assert set(clauses) == set(obligations) == {f'U6-REQ-{n:04}' for n in range(25, 67)}
    refs = {ref for group in mapping['groups'] for ref in group['required_cases']}
    output = dict(
        format=1, task='G6-026', reviewed_work=task['work'], reviewed_verify=task['verify'],
        qualification='Reviewed reusable-auth task scope mapped to current observed behavior cases; shared integration and independent conformance remain unqualified.',
        runtime_report='docs/evidence/gate-6/es256.json', runtime_report_sha256=sha(runtime_path),
        reviewed_mapping_sha256=sha(mapping_path), ledger_sha256=sha(ledger_path),
        checker_sha256=sha(pathlib.Path(__file__)),
        source_inputs_sha256=runtime['source_inputs_sha256'], clauses=clauses,
        distinct_observed_case_refs=len(refs), reviewed_clause_count=len(clauses),
        independent_conformance=False, gate_acceptance_qualified=False,
        passed=True)
    (ROOT / 'docs/evidence/gate-6/authentication-audit.json').write_text(json.dumps(output, indent=2) + '\n')
    print(json.dumps(dict(task='G6-026', reviewed_clauses=len(clauses), observed_cases=len(refs),
                          independent_conformance=False, acceptance_qualified=False, passed=True)))


if __name__ == '__main__':
    main()
