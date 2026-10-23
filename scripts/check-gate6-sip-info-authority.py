#!/usr/bin/env python3
"""Qualify owned canonical SIP INFO writes for the open G6-024 task."""
import datetime
import hashlib
import importlib.util
import json
import os
import pathlib
import platform
import re
import subprocess

ROOT = pathlib.Path(__file__).resolve().parents[1]
REQUIRED = [
    'transaction::egress::tests::owner_drop_or_deadline_discards_pending_actual_io_before_a_ready_poll',
    'transaction::egress::tests::cancellation_stops_delayed_policy_even_if_it_catches_up_after_owner_ends',
    'transaction::manager::tests::tests::owned_info_default_denial_precedes_first_transport_poll',
    'transaction::manager::tests::tests::owned_info_expired_or_dropped_owner_cannot_start_first_write',
    'transaction::manager::tests::tests::owned_info_owner_drop_aborts_pending_io_and_releases_write_guard',
    'transaction::manager::tests::tests::owned_info_timer_e_preserves_exact_branch_cseq_body_and_route',
    'transaction::manager::tests::tests::owned_info_timer_e_reacquires_without_idle_reader_and_refuses_revocation',
    'owned_info_public_401_is_exact_final_response_without_automatic_effect_retry',
    'owned_info_public_drop_stops_udp_retry_after_first_effect',
    'owned_info_public_refusal_and_expiry_produce_no_datagram',
    'owned_info_public_udp_retransmits_exact_request_under_fresh_permission',
    'manager::transaction_integration::outward_error_redaction_tests::info_notify_snapshots_preserve_auth_headers_and_exact_wire_entities',
    'manager::transaction_integration::outward_error_redaction_tests::every_info_notify_facade_delegates_the_single_snapshot_authority',
]


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def source_inputs():
    spec = importlib.util.spec_from_file_location('previous_dtmf', ROOT / 'scripts/check-gate6-dtmf-routing.py')
    previous = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(previous)
    inputs = previous.source_inputs()
    for name in ('scripts/check-gate6-sip-info-authority.py', 'docs/developer/sip-info-authority.md'):
        inputs[name] = digest(ROOT / name)
    return dict(sorted(inputs.items()))


def main():
    directory = ROOT / '.local/gate-6/sip-info-authority' / datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S%fZ')
    directory.mkdir(parents=True)
    before = source_inputs()
    steps = []

    def run(name, command, minimum=0, artifact=None):
        log = directory / (name + '.log')
        env = dict(os.environ)
        if artifact:
            env['CARGO_MANIFEST_DIR'] = str(pathlib.Path(artifact['manifest']).parent)
            assert digest(pathlib.Path(command[0])) == artifact['sha256']
        with log.open('w') as output:
            result = subprocess.run(command, cwd=ROOT, env=env, stdout=output, stderr=subprocess.STDOUT)
        raw = log.read_text(errors='replace')
        tests = re.findall(r'^test (.+?) \.\.\. ok$', raw, re.M)
        ignored = re.findall(r'^test (.+?) \.\.\. ignored[^\n]*$', raw, re.M)
        passed = result.returncode == 0 and len(tests) >= minimum
        binaries = {str(pathlib.Path(command[0]).relative_to(ROOT)): digest(pathlib.Path(command[0]))} if artifact else {}
        if artifact:
            passed = passed and artifact['sha256'] in binaries.values()
        steps.append(dict(name=name, command=command, environment={'CARGO_MANIFEST_DIR': env['CARGO_MANIFEST_DIR']} if artifact else {},
                          passed=passed, exit_code=result.returncode, tests_passed=tests, tests_ignored=ignored,
                          log=str(log.relative_to(ROOT)), log_sha256=digest(log), test_binaries_sha256=binaries))
        print(json.dumps(dict(step=name, passed=passed, tests=len(tests), ignored=len(ignored))), flush=True)
        return passed

    ok = run('dependencies', ['bash', 'scripts/prepare-dependencies.sh'])
    if ok:
        ok = run('format', ['bash', 'scripts/format.sh', '--check'])
    files = re.findall(r'^\+\+\+ b/(.+\.rs)$', (ROOT / 'patches/rvoip/gate6-sip-info-authority.patch').read_text(), re.M)
    if ok:
        ok = run('upstream-format', ['rustfmt', '--edition', '2021', '--config', 'skip_children=true', '--check'] + ['.deps/rvoip/' + name for name in files])
    artifacts = {}
    if ok:
        ok = run('build', ['cargo', 'test', '--locked', '--offline', '-j2', '--target-dir', 'target', '--manifest-path', '.deps/rvoip/Cargo.toml', '-p', 'rvoip-sip-dialog', '--lib', '--test', 'owned_info', '--no-run', '--message-format=json'])
        for line in (directory / 'build.log').read_text(errors='replace').splitlines():
            try:
                item = json.loads(line)
            except ValueError:
                continue
            if item.get('reason') == 'compiler-artifact' and item.get('executable'):
                artifacts[item['target']['name']] = dict(executable=item['executable'], manifest=item['manifest_path'], sha256=digest(pathlib.Path(item['executable'])))
        ok = ok and set(artifacts) == {'rvoip_sip_dialog', 'owned_info'}
        steps[-1]['passed'] = ok
    suites = [
        ('owned-io', 'rvoip_sip_dialog', 'transaction::egress::', 2),
        ('transaction-manager', 'rvoip_sip_dialog', 'transaction::manager::', 137),
        ('client-transactions', 'rvoip_sip_dialog', 'transaction::client::', 31),
        ('exact-completions', 'rvoip_sip_dialog', 'transaction::completion::', 9),
        ('canonical-materializers', 'rvoip_sip_dialog', 'manager::transaction_integration::', 16),
        ('public-udp-info', 'owned_info', '', 4),
    ]
    for name, target, selected, minimum in suites:
        if not ok:
            break
        artifact = artifacts[target]
        ok = run(name, [artifact['executable']] + ([selected] if selected else []) + ['--test-threads=1'], minimum, artifact)
    if ok:
        ok = run('server-integration', ['cargo', 'check', '--locked', '--offline', '-j2', '-p', 'connect-server'])
    if ok:
        ok = run('progress', ['python3', 'scripts/check-gate6-progress.py'])
    if ok:
        ok = run('diff', ['git', 'diff', '--check'])
    observed = {test for step in steps for test in step['tests_passed']}
    missing = sorted(set(REQUIRED) - observed)
    unchanged = before == source_inputs()
    report = dict(task='G6-024', implementation_status='in-progress', full_task_qualification=False,
                  qualification='Canonical immutable owned SIP INFO API and first/Timer-E per-write transport authority. Actual isolated UDP, identical branch/CSeq/body/request bytes and prepared route on retransmission, fresh permission without idle readers, denial before any transport poll, cancellation/drop/deadline custody, exact final 200/401 and no guarded candidate or authentication retry.',
                  not_qualified=['Core method-specific INFO policy, exact SIP adapter lifecycle/negotiation composition and sealed native/SIP/WebRTC source and egress routing remain open.',
                                 'This component qualifies actual UDP; TCP/TLS/WebSocket physical IO and broader endpoint profiles have not been qualified by these tests.',
                                 'A SIP final response does not prove remote tone playback. No production journal/service/HTTP/SDK installation, remaining media profiles, Linux x86_64, remote/browser/provider or full Gate 6 acceptance is claimed.'],
                  executed_at=datetime.datetime.now(datetime.timezone.utc).isoformat(), platform=platform.platform(),
                  rustc=subprocess.check_output(['rustc', '--version'], text=True).strip(),
                  source_inputs_sha256=before, sources_unchanged=unchanged, steps=steps,
                  runtime_tests=sum(len(step['tests_passed']) for step in steps), runtime_unique_tests=len(observed),
                  required_cases=REQUIRED, missing_required_cases=missing,
                  passed=ok and unchanged and not missing and all(step['passed'] for step in steps))
    path = ROOT / 'docs/evidence/gate-6/sip-info-authority.json'
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({k: report[k] for k in ('passed', 'runtime_tests', 'runtime_unique_tests', 'sources_unchanged', 'missing_required_cases')}))
    raise SystemExit(0 if report['passed'] else 1)


if __name__ == '__main__':
    main()
