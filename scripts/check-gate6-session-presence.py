#!/usr/bin/env python3
"""Qualify G6-038 durable presence foundations; public orchestration remains open."""
import argparse
import datetime
import json
import os
import re
import shutil
from pathlib import Path
import subprocess
import importlib.util
ROOT=Path(__file__).resolve().parents[1]
def module(name,path):
    spec=importlib.util.spec_from_file_location(name,ROOT/path);m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m);return m
BASE=module('canonical_http_inputs','scripts/check-gate6-canonical-http.py')
REQUIRED={
    'sessions::presence_tests::invitation_presence_is_atomic_epoch_bound_and_terminal_presence_cannot_attach',
    'sessions::presence_tests::missing_invitation_presence_rolls_back_accept_and_retains_rejection',
    'sessions::presence_tests::four_party_presence_authorizes_current_members_and_removal_is_isolated',
    'sessions::lifetime_tests::current_authorized_end_is_cleanup_only_and_preserves_conversation_commands',
    'session_bindings::tests::native_resource_projection_checks_origin_binding_and_current_authority',
    'canonical_http_sdk_uses_shared_commands_receipts_membership_and_cursor_facts',
    'signed_invitation_decisions_share_http_receipts_without_changing_other_sessions',
    'canonical_http_transition_commands_share_native_admission_and_current_operation_decisions',
}
def source_inputs():
    result=BASE.source_inputs()
    for name in ['scripts/check-gate6-session-presence.py','docs/developer/session-decisions.md']:
        result[name]=BASE.sha(ROOT/name)
    return dict(sorted(result.items()))
def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--reuse-store-candidate',type=Path,help='Reuse successful Cargo output after an accounting-only harness correction, with exact unchanged compiled inputs.')
    args=parser.parse_args()
    reuse=None
    if args.reuse_store_candidate:
        candidate=json.loads(args.reuse_store_candidate.read_text())
        current=source_inputs()
        changed={name for name in set(current)|set(candidate['source_inputs_sha256']) if current.get(name)!=candidate['source_inputs_sha256'].get(name)}
        assert changed <= {'scripts/check-gate6-session-presence.py'},changed
        assert candidate['sources_unchanged']
        original=next(step for step in candidate['steps'] if step['name']=='store')
        assert original['exit_code']==0
        assert BASE.sha(ROOT/original['log'])==original['log_sha256']
        assert len(BASE.STATUSES.passed_tests((ROOT/original['log']).read_text()))==312
        reuse=(original,dict(candidate=str(args.reuse_store_candidate),sha256=BASE.sha(args.reuse_store_candidate),changed_harness_inputs=sorted(changed)))
    directory=ROOT/'.local/gate-6/session-presence'/datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S%fZ');directory.mkdir(parents=True)
    samples=directory/'samples';samples.mkdir();before=source_inputs();steps=[]
    env=dict(os.environ,CONNECT_G6_CANONICAL_HTTP_SAMPLES=str(samples),CONNECT_G6_SESSION_DECISION_SAMPLES=str(samples),CONNECT_GATE6_HTTP_SAMPLES=str(samples))
    commands=[
        ('dependencies',['bash','scripts/prepare-dependencies.sh'],0),
        ('format',['bash','scripts/format.sh','--check'],0),
        ('store',['cargo','test','--locked','--offline','-j2','-p','connect-store','--lib','--','--test-threads=1'],312),
        ('service',['cargo','test','--locked','--offline','-j2','-p','connect-service','--lib','--','--test-threads=1'],18),
        ('runtime',['cargo','test','--locked','--offline','-j2','-p','connect-server','--test','gate6_canonical_runtime','--test','gate6_canonical_transitions','--test','gate6_application_sdk','--','--test-threads=1'],31),
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
        passed=result.returncode==0 and len(tests)>=minimum
        if minimum:passed=passed and 'test result: FAILED' not in raw and 'test result: ok.' in raw
        helpers=re.findall(r'^test (\S+) \.\.\. ignored.*$',raw,re.M)
        if name=='store':
            passed=passed and set(helpers)=={'authority_fences::tests::process_worker','installation_ownership::tests::process_worker','transitions::tests::crash_worker'}
        binaries={str((ROOT/path).relative_to(ROOT)):BASE.sha(ROOT/path) for path in re.findall(r'^\s+Running .+ \(([^)]+)\)$',raw,re.M)}
        step=dict(name=name,command=command,exit_code=result.returncode,passed=passed,tests=tests,ignored_helpers=helpers,log=str(log.relative_to(ROOT)),log_sha256=BASE.sha(log),test_binaries_sha256=binaries)
        if name=='store' and reuse:step['reused_successful_cargo_run']=reuse[1]
        steps.append(step)
        print(json.dumps(dict(step=name,passed=passed,tests=len(tests))),flush=True)
        if not passed:break
    after=source_inputs();observed={test for step in steps for test in step['tests']};missing=sorted(REQUIRED-observed)
    report=dict(task='G6-038',scope='Local macOS arm64 durable Session presence foundation; four-party rows are trusted isolated fixtures, not public joins or real media.',passed=len(steps)==len(commands) and all(s['passed'] for s in steps) and before==after and not missing,
        executed_at=datetime.datetime.now(datetime.timezone.utc).isoformat(),runtime_tests=sum(len(s['tests']) for s in steps),sources_unchanged=before==after,source_inputs_sha256=before,steps=steps,required_tests=sorted(REQUIRED),missing_required_tests=missing,
        not_qualified=['Public general Session invite/join/accept/leave/role/upgrade/end installation','Removal of fixed-pair runtime projection and all task/provider assumptions','Four heterogeneous media participants and six physical connection slots','Main Session lifecycle and denied-camera/stream isolation','Linux, browser, remote/provider, independent conformance, performance, acceptance and publication'])
    for step in steps:
        if step['name']=='openapi' and step['passed']:report['http_validation']=json.loads((ROOT/step['log']).read_text())
    unresolved=ROOT/'.local/gate-6/session-presence-legacy-facades-red.json'
    if unresolved.exists():
        report['known_unresolved']=[dict(
            scope='Historical integration facade migration; not a passing or skipped runtime check.',
            report=str(unresolved.relative_to(ROOT)),sha256=BASE.sha(unresolved),
            actual_failure='gate6_canonical_operations: 14 failures using retired generic canonical commands; unsupported-command, no compatibility fallback restored.',
            not_run='gate6_events and gate6_operations targets were not reached after that failing target.',
            ownership='G6-059 SDK and G6-065 facade migration. Current shared-interface regressions remain selected above.')]
    report['actual_http_samples_sha256']={str(p.relative_to(ROOT)):BASE.sha(p) for p in sorted(samples.glob('*.json'))}
    output=ROOT/'docs/evidence/gate-6/session-presence.json';output.write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps({k:report[k] for k in ['passed','runtime_tests','sources_unchanged','missing_required_tests']}),flush=True)
    raise SystemExit(0 if report['passed'] else 1)
if __name__=='__main__':main()
