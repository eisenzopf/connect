#!/usr/bin/env python3
"""Qualify public G6-038 Session control; physical media coordination stays open."""
import argparse
import datetime
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
ROOT=Path(__file__).resolve().parents[1]
def module(name,path):
    spec=importlib.util.spec_from_file_location(name,ROOT/path);result=importlib.util.module_from_spec(spec);spec.loader.exec_module(result);return result
BASE=module('session_control_inputs','scripts/check-gate6-canonical-http.py')
REQUIRED=set(BASE.REQUIRED)|{
    'canonical_http_cases::canonical_http_duplicate_json_keys_are_refused_before_effects_with_authentication_first',
    'session_control::tests::public_session_invite_accept_leave_role_and_end_keep_general_presence_and_main_id',
    'session_control::tests::existing_session_invitation_adds_only_explicit_participants_and_reject_does_not_end_others',
    'session_control::tests::camera_mismatch_stale_revision_spoof_and_changed_intent_preserve_existing_session',
    'session_control::tests::original_session_receipt_and_source_facts_survive_backup_and_capability_storage_failure_recovers',
    'session_control::tests::current_recipient_ceiling_cannot_end_other_participants_and_unknown_codecs_do_not_admit',
    'session_control::tests::parallel_sessions_keep_main_identity_and_roles_while_unconnected_capabilities_change',
    'peer_authority::tests::session_control::session_control_requires_exact_current_owned_peer_proof_and_replays_one_receipt',
    'peer_authority::tests::session_control::session_control_source_expiring_behind_sql_cannot_create_presence_audience_or_receipt',
    'general_session_control::signed_and_http_general_session_lifecycle_share_four_party_receipts_and_current_views',
}
def source_inputs():
    inputs=BASE.source_inputs()
    for name in ['scripts/check-gate6-session-control.py','docs/developer/general-sessions.md','docs/developer/session-decisions.md']:
        inputs[name]=BASE.sha(ROOT/name)
    return dict(sorted(inputs.items()))
def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--reuse-store-candidate',type=Path,help='Reuse the successful Store run only after the bounded HTTP parser/server-test/verifier changes named below.')
    args=parser.parse_args();reuse=None
    if args.reuse_store_candidate:
        candidate=json.loads(args.reuse_store_candidate.read_text())
        current=source_inputs()
        changed={name for name in set(current)|set(candidate['source_inputs_sha256']) if current.get(name)!=candidate['source_inputs_sha256'].get(name)}
        allowed={'crates/connect-server/src/transports/canonical_http.rs','crates/connect-server/tests/support/canonical_http_cases.rs','crates/connect-server/tests/gate6_application_sdk.rs','scripts/check-gate6-session-control.py'}
        assert changed<=allowed,changed
        assert candidate['sources_unchanged']
        original=next(step for step in candidate['steps'] if step['name']=='store')
        assert original['passed'] and original['exit_code']==0 and len(original['tests'])==320
        assert BASE.sha(ROOT/original['log'])==original['log_sha256']
        for name,digest in original['test_binaries_sha256'].items():
            assert BASE.sha(ROOT/name)==digest,name
            dependency_info=ROOT/(name+'.d')
            assert dependency_info.exists(),dependency_info
            dependencies=dependency_info.read_text()
            assert all(name not in dependencies for name in changed),changed
        reuse=(original,dict(candidate=str(args.reuse_store_candidate),sha256=BASE.sha(args.reuse_store_candidate),unchanged_store_binary=True,changed_unconsumed_server_or_verifier_inputs=sorted(changed)))
    directory=ROOT/'.local/gate-6/session-control'/datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S%fZ');directory.mkdir(parents=True)
    samples=directory/'samples';samples.mkdir();before=source_inputs();steps=[]
    env=dict(os.environ,CONNECT_G6_CANONICAL_HTTP_SAMPLES=str(samples),CONNECT_G6_SESSION_DECISION_SAMPLES=str(samples),CONNECT_GATE6_HTTP_SAMPLES=str(samples))
    commands=[
        ('dependencies',['bash','scripts/prepare-dependencies.sh'],0),
        ('format',['bash','scripts/format.sh','--check'],0),
        ('store',['cargo','test','--locked','--offline','-j2','-p','connect-store','--lib','--','--test-threads=1'],320),
        ('service',['cargo','test','--locked','--offline','-j2','-p','connect-service','--lib','--','--test-threads=1'],18),
        ('client',['cargo','test','--locked','--offline','-j2','-p','connect-client','--lib','canonical::tests','--','--test-threads=1'],9),
        ('runtime',['cargo','test','--locked','--offline','-j2','-p','connect-server','--test','gate6_canonical_runtime','--test','gate6_canonical_transitions','--test','gate6_application_sdk','--','--test-threads=1'],33),
        ('reference',['python3','scripts/generate-canonical-http-openapi.py','--check'],0),
        ('openapi',[str(ROOT/'.local/gate-6/verification-venv/bin/python'),'scripts/check-gate6-canonical-http.py','--validate-openapi',str(samples)],0),
        ('progress',['python3','scripts/check-gate6-progress.py'],0),
        ('diff',['git','diff','--check'],0),
    ]
    for name,command,minimum in commands:
        log=directory/(name+'.log')
        if name=='store' and reuse:
            assert command==reuse[0]['command']
            shutil.copyfile(ROOT/reuse[0]['log'],log)
            result=subprocess.CompletedProcess(command,0)
        else:
            with log.open('w') as f:result=subprocess.run(command,cwd=ROOT,env=env,stdout=f,stderr=subprocess.STDOUT)
        raw=log.read_text(errors='replace');tests=BASE.STATUSES.passed_tests(raw)
        helpers=re.findall(r'^test (\S+) \.\.\. ignored.*$',raw,re.M)
        passed=result.returncode==0 and len(tests)>=minimum
        if minimum:passed=passed and 'test result: ok.' in raw and 'test result: FAILED' not in raw
        if name=='store':passed=passed and set(helpers)=={'authority_fences::tests::process_worker','installation_ownership::tests::process_worker','transitions::tests::crash_worker'}
        binaries={str((ROOT/path).relative_to(ROOT)):BASE.sha(ROOT/path) for path in re.findall(r'^\s+Running .+ \(([^)]+)\)$',raw,re.M)}
        step=dict(name=name,command=command,passed=passed,exit_code=result.returncode,tests=tests,ignored_subprocess_helpers=helpers,log=str(log.relative_to(ROOT)),log_sha256=BASE.sha(log),test_binaries_sha256=binaries)
        if name=='store' and reuse:step['reused_successful_store_run']=reuse[1]
        steps.append(step)
        print(json.dumps(dict(step=name,passed=passed,tests=len(tests))),flush=True)
        if not passed:break
    observed={test for step in steps for test in step['tests']};missing=sorted(REQUIRED-observed);unchanged=before==source_inputs()
    report=dict(task='G6-038',scope='Local macOS arm64 public general Session control over signed native QUIC and explicit HTTP. Four actual authorized participants; declared media capabilities, not physical audio/video.',
        passed=len(steps)==len(commands) and all(step['passed'] for step in steps) and unchanged and not missing,
        executed_at=datetime.datetime.now(datetime.timezone.utc).isoformat(),schema_version=35,source_lock_patches=103,
        source_inputs_sha256=before,sources_unchanged=unchanged,steps=steps,required_tests=sorted(REQUIRED),missing_required_tests=missing,runtime_tests=sum(len(s['tests']) for s in steps),
        actual_http_samples_sha256={str(p.relative_to(ROOT)):BASE.sha(p) for p in sorted(samples.glob('*.json'))},
        not_qualified=['G6-038 full completion: dynamic Core projection, invitation/media coordination, six physical connection slots, real heterogeneous media, stream isolation and physical teardown.','Live capability changes remain explicitly unsupported pending the coordinator; no unverified route switch is claimed.','Fixed-pair legacy task/provider media composition still requires replacement; canonical control has no compatibility fallback.','Linux/browser/remote/provider, independent conformance, load/performance, acceptance and publication.'],
        known_unresolved=[dict(task_ownership=['G6-059','G6-065'],scope='Previously observed historical generic facade migration, not a pass of this candidate.',tests='gate6_canonical_operations: 14 unsupported-command failures through retired canonical generic commands; gate6_events/gate6_operations not reached in that broader run.',checkpoint_sha256='2413c3b6bebeb99a80850ba171c9100f73a641501cbb1b3ff4c092a2d925c5bf')])
    if any(s['name']=='openapi' and s['passed'] for s in steps):report['http_validation']=json.loads((directory/'openapi.log').read_text())
    output=ROOT/'docs/evidence/gate-6/session-control.json';output.write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps({key:report[key] for key in ['passed','runtime_tests','sources_unchanged','missing_required_tests']}),flush=True)
    raise SystemExit(0 if report['passed'] else 1)
if __name__=='__main__':main()
