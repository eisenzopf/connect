#!/usr/bin/env python3
"""Qualify the current reusable Gate 6 M1 contract and its Connect consumer.

Default execution uses isolated local fixtures. No live provider, public ingress,
remote database, browser, Linux hosting, or publication lane is invoked.
"""
import argparse
import datetime
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import platform
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('fallback_profile', ROOT / 'scripts/check-gate6-websocket-media.py')
BASE = importlib.util.module_from_spec(spec)
spec.loader.exec_module(BASE)
PACKAGES = ['rvoip-core', 'rvoip-core-traits', 'rvoip-uctp', 'rvoip-auth-core',
            'rvoip-media-core', 'rvoip-rtp-core', 'rvoip-sip', 'rvoip-sip-dialog',
            'rvoip-webrtc', 'rvoip-websocket', 'rvoip-rtc', 'rvoip-webrtc-stack', 'rtc-interceptor', 'rvoip-vapi', 'rvoip-vcon-postgres']
FEATURES = 'rvoip-core/opus,rvoip-core/vcon-signing,rvoip-websocket/media-webrtc,rvoip-websocket/wss,rvoip-vcon-postgres/core-store,rvoip-vcon-postgres/live-tests'
CORE_TARGETS = ['participant_control', 'session_lifecycle', 'participant_roles', 'messaging',
                'p5_pause_and_listener', 'stream_readiness', 'tenant_and_capacity',
                'orchestrator_dispatch', 'provisional_early_media', 'per_connection_control',
                'v2b_semaphore_admission', 'conversation_lifecycle', 'peer_admission', 'peer_sessions',
                'vcon_emission', 'dtmf_auto_route']
UCTP_TARGETS = ['v2_peer_admission', 'v2_secure_profile', 'v2_canonical_transports',
                'v2_reachability', 'v2_auth_recovery', 'v2_step_up', 'v2_auth_peer',
                'v2_auth_dispatch', 'staged_media', 'peer_media_router', 'datagram_conformance',
                'sig9421_gate', 'pending_dispatch', 'jwt_integration', 'credential_diagnostics']
AUTH_TARGETS = ['principal', 'jwt', 'jwks', 'introspection', 'credential_diagnostics',
                'provider_contracts', 'dpop', 'aauth_principal']
WS_TARGETS = ['media_bridge_loopback', 'ws_bridge_flow', 'ws_envelope_sdp_bridge', 'trickle_ice', 'wss_loopback']
TARGETS = sorted(set(CORE_TARGETS + UCTP_TARGETS + AUTH_TARGETS + WS_TARGETS + ['owned_info', 'adapter_dtmf_network', 'dtmf_info_route', 'dtmf_wire', 'mock_transport']))
REQUIRED_REPORTS = {
    'device-replacement': 'required_tests', 'media-profiles': 'required_cases',
    'websocket-media': 'required_cases', 'secure-profile': 'required_secure_profile_tests',
    'auth-dispatch': 'required_dispatch_tests', 'auth-peer': 'required_peer_tests',
    'step-up': 'required_step_up_tests', 'auth-recovery': 'required_recovery_tests',
    'agent-observations': 'required_cases', 'record-finalization': 'required_cases',
    'es256': 'required_es256_tests',
}


CASE_REVISIONS = {
    'revocation_while_signature_replay_storage_is_pending_prevents_dispatch': {
        'current': 'current_authority_is_retained_while_signature_replay_storage_is_pending',
        'patch': 'patches/rvoip/gate6-store-peer-lifetime.patch',
        'contract': 'docs/developer/store-peer-authority.md',
        'basis': 'The recorded owned-admission change retains the original grant through pending signature replay storage and handler completion. The current regression proves a queued revocation writer cannot pass that reader, then proves committed revocation rejects the next signed invocation without another handler call. This is current-authority ordering, not permission after committed revocation.',
    },
}


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def tree(base):
    result = {}
    for folder, dirs, names in os.walk(base):
        dirs[:] = sorted(name for name in dirs if name != '.git')
        for name in sorted(names):
            path = Path(folder) / name
            if name != '.git' and path.is_file():
                result[str(path.relative_to(base))] = digest(path)
    return result


def source_inputs():
    inputs = BASE.source_inputs()
    for source in json.loads((ROOT / 'dependencies/source-lock.json').read_text())['sources']:
        for name, sha in tree(ROOT / '.deps' / source).items():
            inputs[f'.deps/{source}/{name}'] = sha
    for name in ('scripts/check-gate6-upstream.py', 'scripts/run-gate6-local-record-postgres.py',
                 'scripts/validate-gate6-record-postgres-run.py', 'scripts/check-gate6-contracts.py',
                 'contracts/uctp-gate-6-requirements.json', 'contracts/uctp-vectors/operation-transitions.json',
                 'docs/developer/upstream-contract.md', 'docs/developer/store-peer-authority.md', 'contracts/gate6-authentication-evidence.json', 'docs/evidence/gate-6/authentication-audit.json', 'docs/evidence/gate-6/media-primitives.json'):
        inputs[name] = digest(ROOT / name)
    for name in REQUIRED_REPORTS:
        relative = f'docs/evidence/gate-6/{name}.json'
        inputs[relative] = digest(ROOT / relative)
    return dict(sorted(inputs.items()))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--prepared-dir', type=Path, help='Reverify an existing freshly reproduced tree with the same exact source lock; default prepares a new tree.')
    args = parser.parse_args()
    directory = ROOT / '.local/gate-6/upstream' / datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S%fZ')
    directory.mkdir(parents=True)
    before = source_inputs()
    lock = json.loads((ROOT / 'dependencies/source-lock.json').read_text())
    plan = json.loads(re.search(r'<script id="plan-data" type="application/json">(.*?)</script>', (ROOT / 'docs/connect-gate-6-implementation.html').read_text(), re.S)[1])
    tasks = {task['id']: task for task in plan['tasks']}
    assert all(tasks[name]['status'] == 'done' for name in tasks['G6-028']['depends'])
    required = set()
    for name, field in REQUIRED_REPORTS.items():
        component = json.loads((ROOT / f'docs/evidence/gate-6/{name}.json').read_text())
        assert component['passed']
        required.update(component[field])
    case_revisions = {}
    for old, revision in CASE_REVISIONS.items():
        assert old in required
        required.remove(old)
        required.add(revision['current'])
        case_revisions[old] = {**revision, 'patch_sha256': digest(ROOT / revision['patch']), 'contract_sha256': digest(ROOT / revision['contract'])}
    # PostgreSQL's owned local fixture is mandatory in this combined run.
    required.add('tests::live_put_get_delete_list_and_hash')
    required.update(['endpoint::connect_retransmission_tests::lost_server_flight_duplicate_client_hello_retains_retransmission_and_completes', 'endpoint::connect_retransmission_tests::terminal_parse_alert_or_error_cancels_timer_without_key_completion'])
    steps = []
    artifacts = {}
    inventories = {}
    generated = directory / 'generated'
    generated.mkdir()
    (directory / 'state.json').write_text(json.dumps(dict(task='G6-028', directory=str(directory.relative_to(ROOT)), source_inputs_sha256=before), indent=2) + '\n')

    def run(name, command, minimum=0, artifact=None, node=False):
        log = directory / (name + '.log')
        env = dict(os.environ)
        env['CONNECT_G6_MEDIA_PROFILE_ARTIFACTS'] = str(generated)
        if artifact:
            env['CARGO_MANIFEST_DIR'] = str(Path(artifact['manifest']).parent)
            assert digest(Path(command[0])) == artifact['sha256']
        with log.open('w') as output:
            result = subprocess.run(command, cwd=ROOT, env=env, stdout=output, stderr=subprocess.STDOUT)
        raw = log.read_text(errors='replace')
        parse_error = None
        try:
            tests = re.findall(r'^ok \d+ - (.+)$', raw, re.M) if node else BASE.passed_tests(raw)
        except AssertionError:
            tests = []
            parse_error = 'libtest name/status/summary mismatch'
        ignored = re.findall(r'^test (.+?) \.\.\. ignored[^\n]*$', raw, re.M)
        binaries = {str(Path(command[0]).relative_to(ROOT)): digest(Path(command[0]))} if artifact else {}
        passed = result.returncode == 0 and len(tests) >= minimum and parse_error is None
        if artifact:
            passed = passed and artifact['sha256'] in binaries.values()
        step = dict(name=name, command=command,
                    environment={'CARGO_MANIFEST_DIR': env['CARGO_MANIFEST_DIR'], 'CONNECT_G6_MEDIA_PROFILE_ARTIFACTS': str(generated)} if artifact else {},
                    passed=passed, exit_code=result.returncode, tests_passed=tests, tests_ignored=ignored,
                    parse_error=parse_error, log=str(log.relative_to(ROOT)), log_sha256=digest(log),
                    test_binaries_sha256=binaries, runtime='node' if node else ('rust' if artifact else 'check'))
        if artifact:
            step['artifact_key'] = artifact['key']
        steps.append(step)
        (directory / 'steps.json').write_text(json.dumps(steps, indent=2) + '\n')
        print(json.dumps(dict(step=name, passed=passed, tests=len(tests), ignored=len(ignored))), flush=True)
        return passed

    ok = run('dependencies', ['bash', 'scripts/prepare-dependencies.sh'])
    clean = args.prepared_dir.absolute() if args.prepared_dir else directory / 'clean-deps'
    clean_comparison = {}
    if ok:
        ok = run('clean-preparation', ['bash', 'scripts/prepare-dependencies.sh', '--destination', str(clean), '--cache', str(ROOT / '.deps')])
    if ok:
        for name in lock['sources']:
            consumed, reproduced = tree(ROOT / '.deps' / name), tree(clean / name)
            assert consumed == reproduced, name
            clean_comparison[name] = dict(files=len(consumed), sha256=hashlib.sha256(json.dumps(consumed, sort_keys=True).encode()).hexdigest())
        (directory / 'clean-comparison.json').write_text(json.dumps(clean_comparison, indent=2) + '\n')
    if ok:
        ok = run('format', ['bash', 'scripts/format.sh', '--check'])
    metadata_path = directory / 'metadata.json'
    if ok:
        with metadata_path.open('w') as output:
            subprocess.run(['cargo', 'metadata', '--locked', '--offline', '--format-version', '1', '--manifest-path', '.deps/rvoip/Cargo.toml'], cwd=ROOT, stdout=output, check=True)
        metadata = json.loads(metadata_path.read_text())
        packages = {p['id']: p for p in metadata['packages']}
        chosen = {p['name']: p for p in metadata['packages'] if p['name'] in PACKAGES}
        assert set(chosen) == set(PACKAGES)
        editions = {}
        roots = sorted(((Path(p['manifest_path']).parent, p['edition']) for p in packages.values()), key=lambda pair: len(str(pair[0])), reverse=True)
        for patch in lock['patches']:
            for relative in re.findall(r'^\+\+\+ b/(.+\.rs)$', (ROOT / patch['path']).read_text(), re.M):
                path = ROOT / '.deps' / patch.get('target', 'rvoip') / relative
                if path.is_file():
                    edition = next((edition for base, edition in roots if path.is_relative_to(base)), '2021')
                    editions.setdefault(edition, set()).add(str(path))
        for edition, paths in sorted(editions.items()):
            if ok:
                ok = run('upstream-format-' + edition, ['rustfmt', '--edition', edition, '--config', 'skip_children=true', '--check'] + sorted(paths))
    schema_python = os.environ.get('CONNECT_G6_SCHEMA_PYTHON', str(ROOT / '.local/gate-6/verification-venv/bin/python'))
    if ok:
        ok = run('contracts', [schema_python, 'scripts/check-gate6-contracts.py', '--output', str(directory / 'contracts.json')])
    external_dtls = None
    if ok:
        ok = run('external-dtls-build', ['cargo', 'test', '--locked', '--offline', '-j2', '--target-dir', str(ROOT / 'target'), '--manifest-path', '.deps/rtc-dtls/Cargo.toml', '--lib', '--no-run', '--message-format=json'])
        for line in (directory / 'external-dtls-build.log').read_text(errors='replace').splitlines():
            try:
                item = json.loads(line)
            except ValueError:
                continue
            if item.get('reason') == 'compiler-artifact' and item.get('executable'):
                assert item['target']['name'] == 'rtc_dtls'
                external_dtls = dict(key='rtc-dtls/rtc_dtls', executable=item['executable'], manifest=item['manifest_path'], sha256=digest(Path(item['executable'])))
        ok = ok and external_dtls is not None
        steps[-1]['passed'] = ok
    if ok:
        ok = run('external-dtls-endpoint-regression', [external_dtls['executable'], 'endpoint::connect_retransmission_tests', '--test-threads=1'], 2, external_dtls)
    expected = {}
    if ok:
        for package in chosen.values():
            for target in package['targets']:
                if 'lib' in target['kind'] or ('test' in target['kind'] and target['name'] in TARGETS):
                    expected[(package['id'], target['name'])] = package
        command = ['cargo', 'test', '--locked', '--offline', '-j2', '--target-dir', 'target', '--manifest-path', '.deps/rvoip/Cargo.toml']
        for package in PACKAGES:
            command += ['-p', package]
        command += ['--features', FEATURES, '--lib']
        for target in TARGETS:
            command += ['--test', target]
        ok = run('upstream-build', command + ['--no-run', '--message-format=json'])
        for line in (directory / 'upstream-build.log').read_text(errors='replace').splitlines():
            try:
                item = json.loads(line)
            except ValueError:
                continue
            if item.get('reason') == 'compiler-artifact' and item.get('executable'):
                key = (item['package_id'], item['target']['name'])
                package = packages[item['package_id']]
                named_key = package['name'] + '/' + item['target']['name']
                artifacts[key] = dict(key=named_key, package=package['name'], target=item['target']['name'], executable=item['executable'], manifest=item['manifest_path'], sha256=digest(Path(item['executable'])))
        ok = ok and set(artifacts) == set(expected)
        steps[-1]['passed'] = ok
        steps[-1]['expected_artifacts'] = sorted(p['name'] + '/' + name for (pid, name), p in expected.items())
        steps[-1]['built_artifacts'] = sorted(a['key'] for a in artifacts.values())
        (directory / 'artifacts.json').write_text(json.dumps(sorted(artifacts.values(), key=lambda a: a['key']), indent=2) + '\n')

    def selected(package, target):
        return next(a for a in artifacts.values() if a['package'] == package and a['target'] == target)

    # The complete Core/UCTP libraries and M1 transition/peer targets are run.
    # Other backends use explicit local subsets; filtered/ignored cases are not passes.
    suites = [('rvoip-core', 'rvoip_core', '', 421), ('rvoip-core-traits', 'rvoip_core_traits', '', 1),
              ('rvoip-uctp', 'rvoip_uctp', '', 95), ('rvoip-auth-core', 'rvoip_auth_core', '', 1),
              ('rvoip-vapi', 'rvoip_vapi', '', 1), ('rvoip-vcon-postgres', 'rvoip_vcon_postgres', '', 1)]
    suites += [('rvoip-core', target, '', 1) for target in CORE_TARGETS]
    suites += [('rvoip-uctp', target, '', 1) for target in UCTP_TARGETS]
    suites += [('rvoip-auth-core', target, '', 1) for target in AUTH_TARGETS]
    suites += [('rvoip-websocket', target, '', 1) for target in WS_TARGETS]
    suites += [('rvoip-vapi', 'mock_transport', '', 1), ('rvoip-media-core', 'rvoip_media_core', 'dtmf', 30),
               ('rvoip-sip', 'rvoip_sip', 'dtmf', 5), ('rvoip-sip', 'adapter_dtmf_network', '', 3), ('rvoip-sip', 'dtmf_info_route', '', 8),
               ('rvoip-sip-dialog', 'owned_info', '', 4), ('rvoip-sip-dialog', 'rvoip_sip_dialog', 'transaction::egress::', 2),
               ('rvoip-sip-dialog', 'rvoip_sip_dialog', 'transaction::manager::', 137),
               ('rvoip-sip-dialog', 'rvoip_sip_dialog', 'transaction::client::', 31),
               ('rvoip-sip-dialog', 'rvoip_sip_dialog', 'transaction::completion::', 9),
               ('rvoip-sip-dialog', 'rvoip_sip_dialog', 'manager::transaction_integration::', 17),
               ('rvoip-rtp-core', 'rvoip_rtp_core', 'session::negotiated_telephone_tests::', 3),
               ('rvoip-rtp-core', 'rvoip_rtp_core', 'telephone_event::', 8),
               ('rvoip-webrtc', 'rvoip_webrtc', 'media::', 48), ('rvoip-webrtc', 'rvoip_webrtc', 'peer::source_clock::', 20),
               ('rvoip-webrtc', 'dtmf_wire', '', 12), ('rtc-interceptor', 'rtc_interceptor', '', 135),
               ('rvoip-rtc', 'rtc', 'rtx_feedback_tests', 2),
               ('rvoip-rtc', 'rtc', 'owned_dtmf', 1), ('rvoip-webrtc-stack', 'webrtc', '', 18)]
    available = {name for step in steps for name in step['tests_passed']}
    if ok:
        for index, (package, target, prefix, minimum) in enumerate(suites):
            artifact = selected(package, target)
            listing = subprocess.check_output([artifact['executable']] + ([prefix] if prefix else []) + ['--list', '--format', 'terse'], cwd=ROOT, text=True)
            names = re.findall(r'^(.+): test$', listing, re.M)
            inventory_key = f'{index:02d}-{package}-{target}'
            (directory / (inventory_key + '-inventory.log')).write_text(listing)
            inventories[inventory_key] = dict(artifact_key=artifact['key'], prefix=prefix, tests=names)
            available.update(names)
        missing_inventory = sorted(required - available)
        assert not missing_inventory, ('required cases absent from selected local inventories', missing_inventory)
    for index, (package, target, prefix, minimum) in enumerate(suites):
        if not ok:
            break
        artifact = selected(package, target)
        ok = run(f'{index:02d}-{package}-{target}', [artifact['executable']] + ([prefix] if prefix else []) + ['--test-threads=1'], minimum, artifact)
        inventory = inventories[f'{index:02d}-{package}-{target}']['tests']
        missing = sorted(set(inventory) - set(steps[-1]['tests_passed']) - set(steps[-1]['tests_ignored']))
        ok = ok and not missing
        steps[-1].update(passed=ok, missing_inventory_cases=missing)
    local_database = None
    if ok:
        artifact = selected('rvoip-vcon-postgres', 'rvoip_vcon_postgres')
        ok = run('owned-record-postgres', ['python3', 'scripts/run-gate6-local-record-postgres.py', artifact['executable'], '--evidence-dir', str(directory / 'local-postgres')])
        local_database = json.loads((directory / 'local-postgres/report.json').read_text())
        ok = ok and local_database['passed']
        if ok:
            log = directory / 'local-postgres/test.log'
            tests = BASE.passed_tests(log.read_text())
            assert tests == ['tests::live_put_get_delete_list_and_hash']
            steps[-1].update(log=str(log.relative_to(ROOT)), log_sha256=digest(log), wrapper_log=str((directory / 'owned-record-postgres.log').relative_to(ROOT)), runtime='rust', tests_passed=tests, artifact_key=artifact['key'], test_binaries_sha256={str(Path(artifact['executable']).relative_to(ROOT)): artifact['sha256']})
        if ok:
            ok = run('record-postgres-evidence', ['python3', 'scripts/validate-gate6-record-postgres-run.py', str(directory / 'local-postgres/report.json')])
    node_vectors = json.loads((ROOT / '.deps/rvoip/crates/uctp/rvoip-uctp/tests/fixtures/cross_runtime_media.json').read_text())
    required_node = {v['name'] for group in ('valid', 'invalid') for v in node_vectors[group]}
    if ok:
        ok = run('browser-module-vectors', ['node', '--test', '--test-reporter=tap', '.deps/rvoip/tests/browser-smoke/tests/media_framing.test.mjs'], len(required_node), node=True)
    consumer_artifacts = []
    if ok:
        ok = run('consumer-build', ['cargo', 'test', '--locked', '--offline', '-j2', '-p', 'connect-server', '--test', 'gate6_canonical_runtime', '--no-run', '--message-format=json'])
        for line in (directory / 'consumer-build.log').read_text(errors='replace').splitlines():
            try:
                item = json.loads(line)
            except ValueError:
                continue
            if item.get('reason') == 'compiler-artifact' and item.get('executable'):
                consumer_artifacts.append(dict(key='connect-server/' + item['target']['name'], executable=item['executable'], manifest=item['manifest_path'], sha256=digest(Path(item['executable']))))
        if ok:
            artifact = next(a for a in consumer_artifacts if a['key'] == 'connect-server/gate6_canonical_runtime')
            ok = run('consumer-canonical-runtime', [artifact['executable'], '--test-threads=1'], 11, artifact)
    if ok:
        ok = run('progress', ['python3', 'scripts/check-gate6-progress.py'])
    if ok:
        ok = run('diff', ['git', 'diff', '--check'])
    rust_steps = [step for step in steps if step['runtime'] == 'rust']
    observed = {name for step in rust_steps for name in step['tests_passed']}
    identities = {step['artifact_key'] + '::' + name for step in rust_steps for name in step['tests_passed']}
    missing = sorted(required - observed)
    node_observed = {name for step in steps if step['runtime'] == 'node' for name in step['tests_passed']}
    unchanged = before == source_inputs()
    trace = []
    for name in sorted(tasks):
        if 'G6-015' <= name <= 'G6-027':
            files = [part.strip() for part in tasks[name]['evidence'].split(';') if part.strip()]
            trace.append(dict(task=name, title=tasks[name]['title'], evidence={name: (dict(kind='directory', files=len(tree(ROOT / name)), sha256=hashlib.sha256(json.dumps(tree(ROOT / name), sort_keys=True).encode()).hexdigest()) if (ROOT / name).is_dir() else dict(kind='file', sha256=digest(ROOT / name))) for name in files}))
    result = dict(task='G6-028', implementation_status='runtime-qualified' if ok and unchanged and not missing and required_node <= node_observed else 'in-progress', passed=ok and unchanged and not missing and required_node <= node_observed,
                  qualification='Current-source local macOS arm64 M1 protocol/media/lifecycle/authentication/record recovery qualification in one upstream Cargo graph, a standalone packaged DTLS endpoint regression graph, and a separately built locked Connect canonical-runtime consumer graph. Owned local PostgreSQL; checked browser module on Node. No historical test totals are combined.',
                  not_qualified=['Production canonical session/media Store/service/HTTP/SDK installation and subsequent Gate 6 integration tasks.',
                                 'Linux x86_64, live browser/remote/provider operating and independent conformance/performance/acceptance profiles; publication.'],
                  executed_at=datetime.datetime.now(datetime.timezone.utc).isoformat(), platform=platform.platform(), rustc=subprocess.check_output(['rustc', '--version'], text=True).strip(),
                  patches=lock['patches'], source_revisions=lock['sources'], source_inputs_sha256=before, sources_unchanged=unchanged,
                  clean_comparison=clean_comparison, clean_preparation_mode='verified-prior-fresh-tree' if args.prepared_dir else 'fresh-tree', reproduced_tree=str(clean.relative_to(ROOT)), upstream_build_graph=dict(packages=PACKAGES, features=FEATURES, artifacts=sorted(artifacts.values(), key=lambda a: a['key'])),
                  external_dtls_build_graph=external_dtls, consumer_build_graph=consumer_artifacts, steps=steps, selected_test_inventories=inventories,
                  built_but_unexecuted_artifacts=sorted({a['key'] for a in artifacts.values()} - {step.get('artifact_key') for step in steps}),
                  required_cases=sorted(required), required_case_revisions=case_revisions, missing_required_cases=missing, node_checks=len(node_observed), missing_node_vectors=sorted(required_node - node_observed),
                  runtime_tests=sum(len(step['tests_passed']) for step in rust_steps), runtime_unique_tests=len(identities), test_identity='package/target::test_name',
                  ignored_tests=sorted({step.get('artifact_key', '') + '::' + name for step in rust_steps for name in step['tests_ignored']} - identities),
                  generated_artifacts_sha256={str(p.relative_to(ROOT)): digest(p) for p in generated.iterdir() if p.is_file()},
                  local_database=local_database, m1_component_trace=trace)
    path = ROOT / 'docs/evidence/gate-6/upstream.json'
    path.write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps({**{key: result[key] for key in ('passed', 'runtime_tests', 'runtime_unique_tests', 'node_checks', 'sources_unchanged')}, 'missing_required_case_count': len(missing)}))
    raise SystemExit(0 if result['passed'] else 1)


if __name__ == '__main__':
    main()
