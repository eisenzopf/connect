#!/usr/bin/env python3
"""Qualify Store-backed signed peer authority with prior dispatch regressions.

Production policy, complete writer coverage and Gate 6 acceptance remain open.
"""
import argparse
import datetime
import hashlib
import importlib.metadata
import json
import os
import pathlib
import platform
import re
import subprocess
import sys
import tempfile

import jsonschema

ROOT = pathlib.Path(__file__).resolve().parents[1]


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def sources():
    paths = [ROOT / name for name in (
        'Cargo.toml', 'Cargo.lock', 'dependencies/source-lock.json',
        'scripts/check-gate6-store-peer-authority.py', 'docs/developer/store-peer-authority.md', 'scripts/check-gate6-provider-reconciliation.py', 'scripts/check-gate6-provider-dispatch.py', 'docs/developer/provider-reconciliation.md', 'scripts/check-gate6-owned-runtime.py', 'scripts/format.sh', 'docs/developer/provider-dispatch.md', 'scripts/check-gate6-operation-authority.py', 'scripts/check-gate6-runtime-authority.py', 'scripts/check-gate6-contracts.py',
        'scripts/generate-gate6-event-reference.py', 'docs/developer/authority-fences.md',
        'docs/developer/authority-writers.md', 'docs/developer/runtime-authority.md',
        'docs/developer/operation-authority.md', 'docs/developer/owned-runtime.md',
        'contracts/openapi-provider-observations.json', 'contracts/gate6-authority-writers.json',
        'contracts/openapi-attachments.json', 'contracts/openapi-messages.json',
        'contracts/openapi-routing.json', 'contracts/openapi-operations.json',
        'scripts/prepare-dependencies.py', 'scripts/lib/prepared_archive.py',
        'docs/developer/peer-credentials.md', 'docs/developer/canonical-service.md',
        'docs/developer/events.md', 'docs/developer/provisioning.md',
        'docs/developer/messages.md', 'docs/developer/membership.md',
        'docs/developer/attachments-and-receipts.md', 'docs/developer/operations.md',
        'docs/decisions/014-gate-6-contract-version.md',
        'contracts/openapi-provisioning.json', 'contracts/openapi-events.json',
        'contracts/gate6-event-writers.json',
        'contracts/schemas/uctp-envelope-2.schema.json',
        'contracts/uctp-vectors/gate6-envelope.json')]
    for folder in ('crates', 'examples/connector-template', 'examples/connector-contribution'):
        paths += [p for p in (ROOT / folder).rglob('*')
                  if p.is_file() and 'target' not in p.relative_to(ROOT).parts and (p.suffix in ('.rs', '.sql') or p.name in ('Cargo.toml', 'Cargo.lock'))]
    paths += [ROOT / item['path'] for item in json.loads((ROOT / 'dependencies/source-lock.json').read_text())['patches']]
    paths += [p for p in (ROOT / 'contracts').rglob('*') if p.is_file() and p.suffix in ('.json', '.md')]
    paths += [ROOT / '.deps/rvoip' / name for name in ('Cargo.toml', 'Cargo.lock', 'docs/CONVERSATION_PROTOCOL.md')]
    for folder in ('identity/auth-core', 'foundation/rvoip-core-traits', 'foundation/rvoip-core',
                   'uctp/rvoip-uctp', 'uctp/rvoip-quic', 'uctp/rvoip-webtransport', 'uctp/rvoip-websocket'):
        directory = ROOT / '.deps/rvoip/crates' / folder
        paths += [p for p in directory.rglob('*') if p.is_file() and 'target' not in p.relative_to(directory).parts
                  and p.suffix in ('.rs', '.json', '.toml', '.sql', '.pem', '.der', '.md', '.txt', '.bin', '.crt', '.key')] + [directory / 'Cargo.toml']
    return {str(p.relative_to(ROOT)): digest(p) for p in sorted(set(paths))}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=pathlib.Path,
                        default=ROOT / 'docs/evidence/gate-6/store-peer-authority.json')
    output_path = parser.parse_args().output
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S%fZ')
    directory = ROOT / '.local/gate-6/store-peer-authority' / stamp
    samples = directory / 'http-samples'
    prism_state = directory / 'prism-state'
    directory.mkdir(parents=True)
    prism_state.mkdir()
    before = sources()
    with tempfile.TemporaryDirectory(prefix='connect-authority-case-') as probe:
        lower = pathlib.Path(probe) / 'state.sqlite3'
        lower.touch()
        fixture_case_alias_checked = lower.with_name('STATE.SQLITE3').exists()
    env = dict(os.environ, CONNECT_GATE6_HTTP_SAMPLES=str(samples), PRISM_TEST_STATE_DIR=str(prism_state))
    cargo = ['cargo', 'test', '--no-fail-fast', '--locked', '--offline', '-j1']
    upstream = cargo + ['--target-dir', 'target', '--manifest-path', '.deps/rvoip/Cargo.toml']
    commands = [
        ('dependencies', ['bash', 'scripts/prepare-dependencies.sh'], 0),
        ('format', ['bash', 'scripts/format.sh', '--check'], 0),
        ('upstream-format', ['rustfmt', '--edition', '2021', '--config', 'skip_children=true', '--check'] +
            [str(ROOT / '.deps/rvoip' / relative) for relative in (
                'crates/identity/auth-core/src/sig9421.rs',
                'crates/uctp/rvoip-uctp/src/v2/auth_dispatch.rs',
                'crates/uctp/rvoip-uctp/src/v2/auth_peer.rs',
                'crates/uctp/rvoip-uctp/src/v2/auth_peer/exchange.rs',
                'crates/uctp/rvoip-uctp/src/v2/auth_peer/retirement.rs',
                'crates/uctp/rvoip-uctp/tests/v2_auth_dispatch.rs')], 0),
        ('upstream-signing', upstream + ['-p', 'rvoip-auth-core', '--lib', 'sig9421'], 31),
        ('upstream-peer', upstream + ['-p', 'rvoip-uctp', '--lib', '--test', 'v2_auth_dispatch',
                                    '--test', 'v2_auth_peer', '--test', 'v2_canonical_transports',
                                    '--test', 'v2_step_up', '--test', 'v2_auth_recovery', '--test', 'v2_peer_admission'], 235),
        ('protocol', cargo + ['-p', 'connect-protocol'], 30),
        ('store', cargo + ['-p', 'connect-store'], 407),
        ('service', cargo + ['-p', 'connect-service'], 32),
        ('http', cargo + ['-p', 'connect-server', '--test', 'gate6_provisioning',
                         '--test', 'setup_vault_auth', '--test', 'first_run_schema_boundary',
                         '--test', 'maintenance', '--test', 'gate6_operations'], 16),
        ('native-regression', cargo + ['-p', 'connect-server',
            '--test', 'gate4_native_grants', '--test', 'gate4_native_grant_lifecycle',
            '--test', 'gate4_native_tasks', '--test', 'gate4_native_media',
            '--test', 'gate4_native_media_revocation', '--test', 'gate3_webrtc_media',
            '--test', 'gate6_attachment_links', '--test', 'gate6_receipts'], 14),
        ('provider-regression', cargo + ['-p', 'connect-server',
            '--test', 'provider_flow', '--test', 'provider_context_manifest',
            '--test', 'connector_template', '--test', 'gate4_provider_tasks',
            '--test', 'gate4_task_uncertainty', '--test', 'gate4_task_wire',
            '--test', 'gate3_sip_media', '--test', 'gate4_vapi_post_call'], 23),
        ('message-http', cargo + ['-p', 'connect-connector-http'], 6),
        ('connector-api', cargo + ['-p', 'connect-connector-api'], 11),
        ('connectors', cargo + ['-p', 'connect-connectors'], 42),
        ('message-dispatch', cargo + ['-p', 'connect-server', '--test', 'gate6_provider_dispatch',
            '--test', 'connector_contribution', '--test', 'entry_connector',
            '--test', 'gate4_vapi_pending_continuation'], 11),
        ('template-conformance', cargo + ['--manifest-path', 'examples/connector-template/Cargo.toml'], 4),
        ('contribution-conformance', cargo + ['--manifest-path', 'examples/connector-contribution/Cargo.toml'], 4),
        ('server-api-build', ['cargo', 'check', '--locked', '--offline', '-j1', '-p', 'connect-server', '--tests'], 0),
        ('contracts', [str(ROOT / '.local/gate-6/verification-venv/bin/python'),
                       'scripts/check-gate6-contracts.py', '--output', str(directory / 'contracts.json')], 0),
        ('progress', ['python3', 'scripts/check-gate6-progress.py'], 0),
        ('diff', ['git', 'diff', '--check'], 0),
    ]
    steps = []
    for name, command, minimum in commands:
        log = directory / (name + '.log')
        with log.open('w') as output:
            result = subprocess.run(command, cwd=ROOT, env=env, stdout=output, stderr=subprocess.STDOUT)
        raw = log.read_text(errors='replace')
        tests = re.findall(r'^test (\S+) \.\.\. ok$', raw, re.M)
        binaries = {}
        for name_in_log in re.findall(r'Running [^\n]+\((target/debug/deps/[^)]+)\)', raw):
            path = ROOT / name_in_log
            if path.is_file():
                binaries[name_in_log] = digest(path)
        step = dict(name=name, command=command, exit_code=result.returncode,
                    tests_passed=tests, test_binaries_sha256=binaries,
                    log=str(log.relative_to(ROOT)), log_sha256=digest(log),
                    passed=result.returncode == 0 and len(tests) >= minimum)
        steps.append(step)
        print(json.dumps(dict(step=name, passed=step['passed'], tests=len(tests))), flush=True)
    apis = {name: json.loads((ROOT / f'contracts/openapi-{file}.json').read_text())
            for name, file in [('provisioning', 'provisioning'), ('attachments', 'attachments'), ('provider-observations', 'provider-observations')]}
    inventory = json.loads((ROOT / 'contracts/gate6-authority-writers.json').read_text())
    inventory_errors = []
    for group in inventory['qualified_groups']:
        for relative in group['paths']:
            if not (ROOT / relative).is_file(): inventory_errors.append(f'Missing {relative}')
        for function in group.get('functions', []):
            if not any(re.search(r'fn\s+' + re.escape(function) + r'\b', (ROOT / p).read_text())
                       for p in group['paths'] if p.endswith('.rs')):
                inventory_errors.append(f'Missing function {function}')
    checked, contract_errors = [], []
    for path in sorted(samples.glob('*.json')):
        sample = json.loads(path.read_text())
        try:
            api = apis[sample.get('api', 'provisioning')]
            route = api['paths'][sample['route']]['post']
            exchanges = [(sample['request'], route['requestBody']['content']['application/json']['schema'])]
            if sample['status'] == 204:
                assert sample['response'] is None
                assert '204' in route['responses']
            else:
                exchanges.append((sample['response'], route['responses'][str(sample['status'])]['content']['application/json']['schema']))
            for value, schema in exchanges:
                jsonschema.Draft202012Validator(dict(schema, components=api['components'])).validate(value)
            checked.append(dict(api=sample.get('api', 'provisioning'), route=sample['route'], status=sample['status'],
                                path=str(path.relative_to(ROOT)), sha256=digest(path)))
        except (AssertionError, KeyError, jsonschema.ValidationError) as error:
            contract_errors.append(dict(sample=path.name, error=str(error)))
    required_samples = {'signing-key-enroll.json', 'signing-key-stale.json',
                        'signing-key-revoke.json', 'device-revoke.json', 'signing-key-busy.json',
                        'receipt-privacy-busy.json', 'receipt-privacy-retry.json',
                        'sms-control-busy.json', 'sms-control-retry.json',
                        'provider-dispatch-busy.json', 'provider-dispatch-retry.json',
                        'provider-status-busy.json', 'provider-status-retry.json'}
    missing_samples = sorted(required_samples - {path.name for path in samples.glob('*.json')})
    required_tests = {
        'actual_tcp_blocked_signed_response_holds_revocation_until_key_deadline_closes_socket_and_releases_proof',
        'peer_authority::tests::peer_proof_expiry_can_only_narrow_and_blocks_sql_after_its_validated_source_expires',
        'refresh_union_honors_shorter_old_source_expiry_but_successor_lifetime_uses_renewed_credential',
        'retired_peer_transport_lease_holds_real_root_until_cleanup_and_old_policy_cannot_adopt_restart',
        'signed_handler_rejects_facts_only_public_carrier_without_resource_mutation',
        'current_authority_is_retained_while_signature_replay_storage_is_pending',
        'v2::auth_peer::retirement::custody_tests::policy_registration_custody_survives_lifetime_clones_and_releases_without_cancelling_signal',
        'v2::auth_peer::retirement::custody_tests::installed_fact_deadline_retires_pending_wait_without_cancelling_policy_signal',
        'refreshed_store_lifetime_is_independent_of_retired_old_credential_incarnation',
        'signed_verification_retains_original_store_lease_through_async_lookup_and_queued_revocation',
        'peer_authority::tests::peer_admission_requires_exact_current_device_key_credential_algorithm_and_resource',
        'peer_authority::tests::peer_sql_rejects_foreign_root_wrong_credential_command_and_conversation_without_mutation',
        'peer_authority::tests::peer_held_sql_completes_while_actual_key_retirement_waits_without_read_reentry',
        'peer_authority::tests::peer_sql_wait_rechecks_real_key_expiry_before_any_durable_write',
        'store_peer_lifetime_capacity_follows_last_clone_and_reclaims_dropped_registration',
        'store_peer_rejects_forged_bearer_snapshot_device_and_foreign_policy_lifetime',
        'actual_tcp_idle_signed_peers_retire_for_store_key_device_credential_identity_root_and_key_expiry',
        'store_peer_key_deadline_invalidates_retained_transport_lease_without_new_store_poll',

        'authority_fences::tests::writer_intent_stops_new_readers_held_proof_reuses_without_reentry_and_other_identity_proceeds',
        'authority_fences::tests::cancellation_deadline_and_partial_acquisition_release_every_scope',
        'authority_fences::tests::namespace_privacy_symlink_alias_hardlink_refusal_and_foreign_guard_are_enforced',
        'authority_fences::tests::actual_child_process_revocation_waits_for_parent_delivery_guard_and_admin_open_preserves_runtime',
        'authority_fences::tests::actual_child_death_releases_kernel_reader_without_persistent_phantom_authority',
        'authority_fences::tests::actual_key_and_device_retirement_wait_for_current_identity_delivery',
        'authority_fences::tests::membership_removal_and_other_member_revocation_wait_for_snapshot_delivery',
        'transitions::tests::administration_child_get_retry_and_claim_preserve_live_transition_worker',
        'messages::authority_tests::cancelled_handler_retains_authority_until_uncancellable_store_work_settles',
        'actual_canonical_peer_refresh_keeps_identity_and_requires_a_new_explicit_credential_binding',
        'key_ownership_validity_revisions_and_retirement_are_not_overridden_by_retry',
        'fresh_schema_snapshot_digest_includes_keys_bindings_and_real_issuance',
        'key_and_credential_history_bounds_do_not_evict_old_bindings_or_grant_the_overflow',
        'persisted_issuance_and_ed25519_binding_survive_restart_and_refresh_without_borrowing_keys',
        'es256_keys_require_exact_owner_algorithm_current_credential_device_and_validity',
        'future_and_expired_keys_never_resolve_from_a_still_current_credential',
        'signing_key_and_device_routes_enforce_admin_target_replay_and_closed_contracts',
        'transitions::tests::process_crashes_cover_durable_boundaries_without_duplicate_callbacks_or_partial_local_commit',
        'transitions::tests::process_crashes_keep_cancellation_evidence_reconciliation_and_cleanup_atomic',
    }
    required_tests.update('tests::' + test for test in (
        'actual_socket_rechecks_authority_before_each_write_and_drops_socket_before_custody',
        'cancelling_http_future_closes_real_pending_connection_and_releases_transport_custody',
        'permit_expiry_closes_stalled_http_without_retry_and_a_clone_cannot_create_twice',
        'pinned_tls_pending_request_has_the_same_owned_cancellation_boundary',
        'configuration_rejects_plaintext_remote_and_credential_bearing_urls',
        'read_only_status_permit_refuses_creation_and_retains_socket_until_cancelled_get_closes'))
    required_tests.update(test for group in inventory['qualified_groups'] for test in group['required_tests'])
    observed = {test for step in steps for test in step['tests_passed']}
    missing_tests = sorted(required_tests - observed)
    after = sources()
    report = dict(
        format=1, task='G6-037', implementation_status='in-progress', schema=31,
        qualification='Selected local macOS arm64 Store-backed exact Device/key/credential admission and binding retirement for installed canonical message/history/snapshot commands. Retained verification and signed writer leases, real SQL wait expiry and queued key retirement, weak lifetime capacity, and idle actual TCP retirement are exercised alongside signing, peer, canonical transport and prior selected Store/service/provider regressions. Production listener/full dispatch and full source reader policy remain open.',
        executed_at=datetime.datetime.now(datetime.timezone.utc).isoformat(),
        platform=platform.platform(),
        fixture_case_alias_branch=dict(executed=fixture_case_alias_checked,
                                      disposition='Case-insensitive temporary fixture filesystem; required namespace case-alias branch executes.' if fixture_case_alias_checked else 'Case-sensitive temporary fixture filesystem; case-alias branch skipped, not qualified.'),
        source_revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
        source_inputs_sha256=before, sources_unchanged=before == after,
        python=sys.version, jsonschema=importlib.metadata.version('jsonschema'),
        steps=steps, required_tests=sorted(required_tests), missing_required_tests=missing_tests,
        http_openapi_samples=checked, missing_http_samples=missing_samples,
        contract_errors=contract_errors, writer_inventory_errors=inventory_errors,
        writer_groups=[group['id'] for group in inventory['qualified_groups']],
        remaining_writer_audit=inventory['remaining_audit'],
        ignored_test_disposition={
            'transitions::tests::crash_worker': 'Explicitly invoked at 27 crash boundaries by the two required parent crash tests (expected exit 86), plus one successful administration child invoked by its required parent. The standalone ignored target is not counted as a pass.',
            'installation_ownership::tests::process_worker': 'Explicitly invoked by two required passing parents: a child refusing live parent adoption and a real child owner killed while holding the installation kernel lock. The standalone ignored target is not counted as a pass.',
            'authority_fences::tests::process_worker': 'Explicitly invoked in two child processes by required passing parents: a real revocation writer and a reader killed while holding a kernel lock. The standalone ignored target is not counted as a pass.',
        },
        not_qualified=[
            'Selected Telnyx/Vapi message creation retains actual HTTP/TLS socket custody. Built-in provider status capability expansion, voice/SIP, bootstrap/provisioning SDK I/O, post-retirement attributed settlement, all natural-expiry and other SMS bulk profiles remain unqualified. Complete reader scopes, task/transition held composition, actual HTTP response-body admission and complete retirement policy remain open.',
            'Production canonical listener installation, full command/extension coverage and canonical QUIC/HTTP/SDK parity remain open.',
            'Per-Sink-poll checks do not qualify per-underlying-write admission across every transport driver. Device/key registry revocation does not qualify active native media cleanup or current endpoint/session projection; those integrations remain open.',
            'Remote/browser/live-provider checks, independent conformance, publication and all Gate 6 acceptance remain unqualified.',
        ], acceptance_qualified=False,
    )
    report['unqualified_candidates'] = []
    for relative, reason in (
        ('.local/gate-6/store-peer-qualification-candidate-1.json', 'Intermediate broad candidate includes an expiry fixture failure and source changes made for source-expiry narrowing, renewed lifetime bounds and actual blocked response qualification. Retained as unqualified; final source is rerun separately.'),
        ('.local/gate-6/store-peer-authority/20261017T054908880919Z/store.log', 'Broad Store run passed 405 cases; the two-second key expiry fixture expired during concurrent setup before admission. Expiry now derives at enrollment, with a ten-second validity window and the same real expiry-behind-SQLite assertion. Intermediate sources changed; this candidate is unqualified.'),
        ('.local/gate-6/store-peer-uctp-regression-1.log', '71 upstream unit and 19 dispatch cases passed; the twentieth old fixture waited for a revocation write while verification correctly retained its read guard. Interrupted the deadlocked fixture, then updated it to prove writer exclusion, admitted completion and denial on the next request. No pass inferred from the interrupted run.'),
        ('.local/gate-6/store-peer-service-tests-1.log', 'New real Device fixture reused one endpoint address for three owners, violating endpoint uniqueness. Corrected to distinct per-Device addresses before qualification.'),
        ('.local/gate-6/store-peer-auth-regression-1.log', 'Upstream package is not a member of the Connect workspace; rerun through the prepared upstream manifest with the shared target directory.'),
        ('.local/gate-6/reconciliation-server-compile-1.log', 'New real-status fixture used ProviderDelivery.id instead of the public job_id field. Corrected before behavioral execution.'),
        ('.local/gate-6/provider-dispatch/20261017T043101078462Z/store.log', 'All 261 unit cases and earlier integration cases passed; the old interrupted-context fixture had not admitted dispatch before expecting uncertainty. It now admits the possible effect explicitly. The backup fixture is also strengthened to refuse creation from a restored pending snapshot. The final runner uses no-fail-fast so later targets execute even if a fixture fails.'),
        ('.local/gate-6/provider-dispatch-candidate-1.json', 'Initial broad candidate is unqualified: task isolation and unadmitted recovery assumptions were corrected; the pending-provider MCP lane exposed unnecessary grant replay write locking, and the standalone Prism runner omitted its required ignored state directory. Source changed during diagnostics. None of this candidate is sealed as passing.'),
        ('.local/gate-6/provider-dispatch/20261017T041235850598Z/store.log', 'Initial broad Store candidate passed 258 unit cases and failed two task-provider cases: route FIFO was overbroad for independent task contexts, and the interrupted-send fixture had never admitted host I/O. The selector now isolates task turns; recovery fixture explicitly admits the possible effect. All six task-provider regressions pass before final source qualification.'),
        ('.local/gate-6/dispatch-http-tests-4.log', 'Four HTTP cases passed; the cloned-permit case caught missing mutation admission in the new transport. Fixed before the final source qualification.'),
        ('.local/gate-6/dispatch-server-compile-1.log', 'New integration fixture used a removed ProviderStatus.limit field; corrected to the current public operation before execution.'),
        ('.local/gate-6/dispatch-store-tests-1.log', 'New Store fixtures initially used obsolete membership/session fields; corrected before behavioral execution.'),
        ('.local/gate-6/installation-ownership-tests-4.log', 'Intermediate epoch-helper signature propagation compile failure, fixed before behavioral execution.'),
        ('.local/gate-6/installation-ownership-tests-5.log', '250 Store unit cases passed; backup test first correctly refused old physical observations before reservation conflict. Strengthened to prove both old-observation denial and conflict after explicit fresh fixture observation.'),
        ('.local/gate-6/installation-ownership-tests-6.log', 'Same restored-observation test expectation before strengthening; 30 other transition cases passed.'),
        ('.local/gate-6/operation-authority-tests-5.log', 'Six new behavioral tests passed; revoked-credential denial correctly returned unauthorized, so the seventh fixture expectation was corrected from forbidden.'),
        ('.local/gate-6/operation-authority-tests-1.log', 'Initial helper compile errors: &str conversion and MutexGuard dereference corrected before execution.'),
        ('.local/gate-6/operation-authority-tests-3.log', 'New fixture compile errors: retained original claim lease and supplied explicit revocation Actor.'),
        ('.local/gate-6/operation-authority-tests-4.log', 'New scoped fixture thread needed to own its revocation Actor.'),
        ('.local/gate-6/runtime-authority-tests-1.log', 'Copied-job negative test tried a second route in a schema-unique Identity/Conversation slot before reaching completion; corrected to a valid distinct Conversation route.'),
        ('.local/gate-6/runtime-authority-tests-2.log', 'Changing only the provider target still violated the same unique Identity/Conversation fixture constraint; corrected the Conversation as required by the existing contract.'),
        ('.local/gate-6/authority-writers-candidate-1.json', '421 Rust tests, 179 schema vectors and 13 HTTP/OpenAPI samples passed on unchanged source, but selected native regression compilation exposed a pre-existing fixture editing removed Send.body; updated to the current typed message.body. No native pass inferred from this candidate.'),
        ('.local/gate-6/authority-writers-compile-1.log', 'Intermediate compile failure while propagating the owned proof into every shared engine caller.'),
        ('.local/gate-6/authority-writers-store-2.log', 'Administration replay fixture regenerated an expiry-bearing request across a clock boundary; fixed to transmit the exact accepted request.'),
        ('.local/gate-6/authority-writers-mutations-1.log', 'New artifact fixture used a disallowed zero-byte artifact; replaced with an actual one-byte upload and integrity-checked publication.'),
        ('.local/gate-6/authority-fences-candidate-1.json', 'All 410 selected Rust tests, 179 schema vectors and 11 HTTP/OpenAPI samples passed; intermediate source changed for canonical/blob/case-alias fixes, and the runner incorrectly expected ten instead of nine selected HTTP/CLI-boundary tests. Not sealed.'),
        ('.local/gate-6/authority-fences-tests-1.log', 'Initial fence candidate failed fixture setup and hardlink validation before the fixes.'),
        ('.local/gate-6/authority-blocking-test-1.log', 'Initial cancellation fixture failed compilation because four existing Service test constructors needed the new test hook field.'),
        ('.local/gate-6/authority-fences-tests-3.log', 'Expanded fixture compile errors: lookup algorithm type and snapshot revision field corrected.'),
        ('.local/gate-6/authority-fences-tests-4.log', 'Expanded fixtures supplied revision 1 for newly created revision-0 resources; corrected to current metadata.'),
    ):
        candidate = ROOT / relative
        if candidate.is_file():
            report['unqualified_candidates'].append(dict(path=relative, sha256=digest(candidate), reason=reason))
    report['passed'] = before == after and all(step['passed'] for step in steps) and not missing_tests and not missing_samples and not contract_errors and not inventory_errors and len(checked) >= 19
    path = output_path
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(dict(passed=report['passed'], rust_tests=sum(len(s['tests_passed']) for s in steps), http_samples=len(checked), missing_tests=missing_tests, missing_samples=missing_samples)))
    raise SystemExit(0 if report['passed'] else 1)


if __name__ == '__main__':
    main()
