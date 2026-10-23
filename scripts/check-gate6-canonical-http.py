#!/usr/bin/env python3
"""Verify the current G6-037 shared native/HTTP dispatch and closed extensions."""
import argparse
import datetime
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import sys

ROOT=Path(__file__).resolve().parents[1]
SUPPORTED=['conversation.create','conversation.list','conversation.get','conversation.close','conversation.subscribe',
    'conversation.participant.add','conversation.participant.update','conversation.participant.remove',
    'message.send','message.history','message.read','operation.get','operation.cancel','operation.reconcile',
    'session.consult','session.handoff','connection.replace','session.reject','session.cancel',
    'session.invite','session.accept','session.update','session.end']
REQUIRED={
    'canonical_http_sdk_uses_shared_commands_receipts_membership_and_cursor_facts',
    'canonical_http_cases::canonical_http_refuses_malformed_unsupported_spoofed_and_expired_requests_without_effects',
    'canonical_http_cases::canonical_http_cancelled_wait_replays_the_original_mutation_once',
    'canonical_http_cases::canonical_http_bad_json_authentication_order_and_storage_recovery_preserve_original_ids',
    'canonical_http_cases::canonical_http_retired_generic_dispatch_refuses_canonical_commands_and_keeps_explicit_extensions',
    'canonical_http_transition_commands_share_native_admission_and_current_operation_decisions',
    'transitions::tests::peer_operations::http_transition_command_replay_preserves_expiry_and_checks_operation_resource_claims',
    'canonical::tests::canonical_cursor_facts_bind_outer_resources_to_the_authoritative_journal_event',
    'canonical_session_decisions::tests::legacy_application_profile_cannot_dispatch_new_invitation_decisions',
    'signed_invitation_decisions_share_http_receipts_without_changing_other_sessions',
    'application_task_discovery_uses_only_explicit_current_credential',
    'provisioning_sdk_executes_all_routes_and_projects_current_state_on_replay',
}
def sha(p):
    with Path(p).open('rb') as f:return hashlib.file_digest(f,'sha256').hexdigest()
def import_file(name,path):
    spec=importlib.util.spec_from_file_location(name,ROOT/path);module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module);return module
BASE=import_file('decision_inputs','scripts/check-gate6-session-decisions.py')
STATUSES=import_file('libtest_statuses','scripts/check-gate6-websocket-media.py')
def source_inputs():
    inputs=BASE.source_inputs()
    for name in ['scripts/check-gate6-canonical-http.py','scripts/generate-canonical-http-openapi.py',
        'contracts/openapi-canonical-http.json','docs/developer/canonical-http.md','docs/developer/application-sdk.md',
        'contracts/uctp-gate-6-requirements.json']:
        inputs[name]=sha(ROOT/name)
    return dict(sorted(inputs.items()))
def validate(directory):
    from jsonschema import Draft202012Validator
    doc=json.loads((ROOT/'contracts/openapi-canonical-http.json').read_text())
    def validator(name):
        schema={'$ref':'#/components/schemas/'+name,'components':doc['components']}
        Draft202012Validator.check_schema(schema);return Draft202012Validator(schema)
    request=validator('HttpEnvelope');batch=validator('ReplyBatch');rejection=validator('Rejection')
    samples=[]
    for path in sorted(directory.glob('*.json')):
        value=json.loads(path.read_text());samples.extend(value if isinstance(value,list) else [value])
    assert len(samples)>=20,len(samples)
    for sample in samples:
        if 'request' in sample:
            valid=not list(request.iter_errors(sample['request']))
            assert valid==sample.get('valid_request',True),sample.get('case',sample.get('route'))
        else:assert not sample.get('valid_request',True)
        response=sample['response']
        (rejection if 'problem' in response else batch).validate(response)
        if 'problem' in response:assert response['problem']['code']==sample['status']
        else:
            assert response['request_id']==sample['request']['id']
            assert all(e['in_reply_to']==response['request_id'] for e in response['envelopes'])
            terminal=response['envelopes'][-1]
            assert sample['status']==(terminal['payload']['code'] if terminal['type']=='error' else 200)
    cases={s.get('case') for s in samples}
    assert {'expired-credential','storage-unavailable','malformed-json','invalid-token-before-json','request-over-budget','wrong-sender','wrong-operation-resource','transport-signature-refused','history-operation-assertion'}<=cases
    print(json.dumps(dict(passed=True,actual_http_exchanges=len(samples),request_schema_valid=sum('request' in s and not list(request.iter_errors(s['request'])) for s in samples),bounded_statuses=sorted({s['status'] for s in samples}))))
def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--validate-openapi',type=Path);args=parser.parse_args()
    if args.validate_openapi:validate(args.validate_openapi);return
    directory=ROOT/'.local/gate-6/canonical-http'/datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S%fZ');directory.mkdir(parents=True)
    samples=directory/'samples';samples.mkdir();before=source_inputs();steps=[]
    env=dict(os.environ,CONNECT_G6_CANONICAL_HTTP_SAMPLES=str(samples),CONNECT_G6_SESSION_DECISION_SAMPLES=str(samples),CONNECT_GATE6_HTTP_SAMPLES=str(samples))
    commands=[
        ('dependencies',['bash','scripts/prepare-dependencies.sh'],0),
        ('format',['bash','scripts/format.sh','--check'],0),
        ('reference',['python3','scripts/generate-canonical-http-openapi.py','--check'],0),
        ('store-peer',['cargo','test','--locked','--offline','-j2','-p','connect-store','--lib','peer_authority::tests','--','--test-threads=1'],26),
        ('store-operations',['cargo','test','--locked','--offline','-j2','-p','connect-store','--lib','transitions::tests::peer_operations','--','--test-threads=1'],1),
        ('service',['cargo','test','--locked','--offline','-j2','-p','connect-service','--lib','--','--test-threads=1'],18),
        ('client',['cargo','test','--locked','--offline','-j2','-p','connect-client','--lib','canonical::tests','--','--test-threads=1'],1),
        ('runtime',['cargo','test','--locked','--offline','-j2','-p','connect-server','--test','gate6_canonical_runtime','--test','gate6_canonical_transitions','--test','gate6_application_sdk','--','--test-threads=1'],31),
        ('openapi',[str(ROOT/'.local/gate-6/verification-venv/bin/python'),'scripts/check-gate6-canonical-http.py','--validate-openapi',str(samples)],0),
        ('progress',['python3','scripts/check-gate6-progress.py'],0),
        ('diff',['git','diff','--check'],0),
    ]
    for name,command,minimum in commands:
        log=directory/(name+'.log')
        with log.open('w') as f:result=subprocess.run(command,cwd=ROOT,env=env,stdout=f,stderr=subprocess.STDOUT)
        raw=log.read_text(errors='replace');tests=STATUSES.passed_tests(raw)
        binaries={str(Path(p).relative_to(ROOT)):sha(p) for p in (ROOT/q for q in re.findall(r'^\s+Running .+ \(([^)]+)\)$',raw,re.M))}
        passed=result.returncode==0 and len(tests)>=minimum
        steps.append(dict(name=name,command=command,passed=passed,exit_code=result.returncode,tests_passed=tests,log=str(log.relative_to(ROOT)),log_sha256=sha(log),test_binaries_sha256=binaries))
        print(json.dumps(dict(step=name,passed=passed,tests=len(tests))),flush=True)
        if not passed:break
    observed={t for s in steps for t in s['tests_passed']};missing=sorted(REQUIRED-observed);unchanged=before==source_inputs()
    binaries={p:digest for s in steps for p,digest in s['test_binaries_sha256'].items()}
    if len(steps)==len(commands) and all(s['passed'] for s in steps):binaries['target/debug/connect']=sha(ROOT/'target/debug/connect')
    ledger=json.loads((ROOT/'contracts/uctp-gate-6-requirements.json').read_text())
    ownership={e['type']:e['implementation_items'] for e in ledger['envelopes'] if e['type'].startswith(('session.','connection.','stream.'))}
    report=dict(task='G6-037',implementation_status='shared-canonical-native-http-and-closed-application-dispatch',
        passed=len(steps)==len(commands) and all(s['passed'] for s in steps) and unchanged and not missing,
        executed_at=datetime.datetime.now(datetime.timezone.utc).isoformat(),
        qualification='Current-source isolated local macOS arm64 shared canonical control router, explicit HTTP SDK, retained signed native path, closed genuine application extensions, original operation receipts, failure/recovery and current reply correlation. No historical test totals are combined.',
        not_qualified=['G6-038 and subsequent general Session/connection/stream orchestration and later integration task outcomes.','Linux x86_64, live browser/remote/provider, HTTPS/public ingress and physical HTTP response-write custody, independent conformance/performance/acceptance and publication.'],
        supported_commands=SUPPORTED, later_command_ownership=ownership, source_inputs_sha256=before,sources_unchanged=unchanged,
        required_tests=sorted(REQUIRED),missing_required_tests=missing,runtime_tests=sum(len(s['tests_passed']) for s in steps),steps=steps,
        executable_sha256=binaries,actual_http_samples_sha256={str(p.relative_to(ROOT)):sha(p) for p in sorted(samples.glob('*.json'))},
        upstream_checkpoint_sha256=sha(ROOT/'docs/evidence/gate-6/upstream.json'),schema_version=35,source_lock_patches=103)
    if any(s['name']=='openapi' and s['passed'] for s in steps):report['openapi_verification']=json.loads((directory/'openapi.log').read_text())
    (ROOT/'docs/evidence/gate-6/canonical-http.json').write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps({k:report[k] for k in ['passed','runtime_tests','sources_unchanged','missing_required_tests']}));raise SystemExit(0 if report['passed'] else 1)
if __name__=='__main__':main()
