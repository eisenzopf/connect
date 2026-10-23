#!/usr/bin/env python3
"""Verify G6-038's current Store/physical peer/Core projection checkpoint.

This is a partial Session checkpoint, not four-party negotiated media acceptance.
The G6-028 and session-control reports remain immutable historical checkpoints.
"""
import argparse
import shutil
import datetime
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess

ROOT=Path(__file__).resolve().parents[1]
spec=importlib.util.spec_from_file_location('controls',ROOT/'scripts/check-gate6-session-control.py')
CONTROLS=importlib.util.module_from_spec(spec);spec.loader.exec_module(CONTROLS)
BASE=CONTROLS.BASE
REQUIRED=CONTROLS.REQUIRED|{
    'peer_authority::tests::session_control::session_projection_requires_completed_exact_intent_and_retains_current_epoch',
    'general_session_control::signed_session_projection_uses_actual_physical_peers_and_replay_cannot_resurrect_end',
    'relationships::tests::persisted_membership_preserves_large_epochs_and_current_activity',
    'relationships::tests::restored_readmission_cannot_reuse_prior_presence_or_connection',
}
def source_inputs():
    historical=json.loads((ROOT/'docs/evidence/gate-6/upstream.json').read_text())
    lock=json.loads((ROOT/'dependencies/source-lock.json').read_text())
    assert historical['passed'] and lock['sources']==historical['source_revisions']
    assert lock['patches'][:103]==historical['patches'] and len(lock['patches'])==104
    assert lock['patches'][103]['path']=='patches/rvoip/gate6-current-membership.patch'
    paths={ROOT/name for name in ('Cargo.toml','Cargo.lock','dependencies/source-lock.json','docs/evidence/gate-6/upstream.json','scripts/format.sh','scripts/prepare-dependencies.sh','scripts/prepare-dependencies.py','scripts/check-gate6-session-runtime.py')}
    allowed_delta={'.deps/rvoip/crates/foundation/rvoip-core/src/'+name for name in ('relationships.rs','relationships/tests.rs')}
    for name,want in historical['source_inputs_sha256'].items():
        if name.startswith(('.deps/','patches/')):
            assert name in allowed_delta or BASE.sha(ROOT/name)==want,name
            paths.add(ROOT/name)
    for patch in lock['patches']:
        p=ROOT/patch['path'];assert BASE.sha(p)==patch['sha256'],p;paths.add(p)
    # The preparation command independently reproduces/checks the ordered patch
    # tree before compilation. Pin its current recorded patched files as inputs.
    marker=ROOT/'.deps/rvoip/.git/connect-prepared.json'
    assert json.loads(marker.read_text())==lock
    manifest=json.loads((ROOT/'.deps/rvoip/.git/connect-files.json').read_text())
    for name,want in manifest.items():
        p=ROOT/'.deps/rvoip'/name;assert BASE.sha(p)==want,name;paths.add(p)
    for directory in ('crates','examples'):
        paths.update((ROOT/directory).rglob('*.rs'));paths.update((ROOT/directory).rglob('Cargo.toml'))
    for directory in ('scripts','docs/developer','contracts'):
        paths.update(p for p in (ROOT/directory).rglob('*') if p.is_file() and '__pycache__' not in p.parts)
    paths.update((ROOT/'crates/connect-store/migrations').glob('*.sql'))
    return {str(p.relative_to(ROOT)):BASE.sha(p) for p in sorted(paths)}
def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--reuse-core-store-candidate',type=Path)
    args=parser.parse_args();reuse={}
    if args.reuse_core_store_candidate:
        candidate=json.loads(args.reuse_core_store_candidate.read_text());current=source_inputs()
        assert candidate['sources_unchanged']
        changed={name for name in set(current)|set(candidate['source_inputs_sha256']) if current.get(name)!=candidate['source_inputs_sha256'].get(name)}
        assert changed<={'crates/connect-service/src/tasks/tests.rs','crates/connect-service/src/event_tests.rs','crates/connect-client/src/canonical.rs','scripts/check-gate6-session-runtime.py'},changed
        for name,count in [('core-memberships',9),('store',321)]:
            step=next(s for s in candidate['steps'] if s['name']==name)
            assert step['passed'] and step['exit_code']==0 and len(step['tests'])==count
            assert BASE.sha(ROOT/step['log'])==step['log_sha256']
            for binary,digest in step['test_binaries_sha256'].items():
                assert BASE.sha(ROOT/binary)==digest
                depinfo=(ROOT/(binary+'.d')).read_text()
                assert all(path not in depinfo for path in changed),changed
            reuse[name]=(step,dict(candidate=str(args.reuse_core_store_candidate),sha256=BASE.sha(args.reuse_core_store_candidate),changed_unconsumed_inputs=sorted(changed),same_command_and_compiled_inputs=True))
    directory=ROOT/'.local/gate-6/session-runtime'/datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S%fZ');directory.mkdir(parents=True)
    samples=directory/'samples';samples.mkdir();before=source_inputs();steps=[]
    env=dict(os.environ,CONNECT_G6_CANONICAL_HTTP_SAMPLES=str(samples),CONNECT_G6_SESSION_DECISION_SAMPLES=str(samples),CONNECT_GATE6_HTTP_SAMPLES=str(samples))
    commands=[
        ('dependencies',['bash','scripts/prepare-dependencies.sh'],0),
        ('format',['bash','scripts/format.sh','--check'],0),
        ('core-memberships',['cargo','test','--locked','--offline','-j2','--target-dir','target','--manifest-path','.deps/rvoip/Cargo.toml','-p','rvoip-core','--features','rvoip-core/opus,rvoip-core/vcon-signing','--lib','relationships::tests','--','--test-threads=1'],9),
        ('store',['cargo','test','--locked','--offline','-j2','-p','connect-store','--lib','--','--test-threads=1'],321),
        ('service',['cargo','test','--locked','--offline','-j2','-p','connect-service','--lib','--','--test-threads=1'],18),
        ('client',['cargo','test','--locked','--offline','-j2','-p','connect-client','--lib','canonical::tests','--','--test-threads=1'],9),
        ('runtime',['cargo','test','--locked','--offline','-j2','-p','connect-server','--test','gate6_canonical_runtime','--test','gate6_canonical_transitions','--test','gate6_application_sdk','--','--test-threads=1'],34),
        ('reference',['python3','scripts/generate-canonical-http-openapi.py','--check'],0),
        ('openapi',[str(ROOT/'.local/gate-6/verification-venv/bin/python'),'scripts/check-gate6-canonical-http.py','--validate-openapi',str(samples)],0),
        ('progress',['python3','scripts/check-gate6-progress.py'],0),
        ('diff',['git','diff','--check'],0),
    ]
    for name,command,minimum in commands:
        log=directory/(name+'.log')
        if name in reuse:
            assert command==reuse[name][0]['command']
            shutil.copyfile(ROOT/reuse[name][0]['log'],log)
            result=subprocess.CompletedProcess(command,0)
        else:
            with log.open('w') as output: result=subprocess.run(command,cwd=ROOT,env=env,stdout=output,stderr=subprocess.STDOUT)
        raw=log.read_text(errors='replace');tests=BASE.STATUSES.passed_tests(raw)
        helpers=re.findall(r'^test (\S+) \.\.\. ignored.*$',raw,re.M)
        passed=result.returncode==0 and len(tests)>=minimum
        if minimum:passed=passed and 'test result: ok.' in raw and 'test result: FAILED' not in raw
        if name=='store':passed=passed and set(helpers)=={'authority_fences::tests::process_worker','installation_ownership::tests::process_worker','transitions::tests::crash_worker'}
        binaries={str((ROOT/p).relative_to(ROOT)):BASE.sha(ROOT/p) for p in re.findall(r'^\s+Running .+ \(([^)]+)\)$',raw,re.M)}
        steps.append(dict(name=name,command=command,passed=passed,exit_code=result.returncode,tests=tests,ignored_subprocess_helpers=helpers,log=str(log.relative_to(ROOT)),log_sha256=BASE.sha(log),test_binaries_sha256=binaries))
        if name in reuse:steps[-1]['reused_successful_unchanged_run']=reuse[name][1]
        print(json.dumps(dict(step=name,passed=passed,tests=len(tests))),flush=True)
        if not passed:break
    observed={test for step in steps for test in step['tests']};missing=sorted(REQUIRED-observed);unchanged=before==source_inputs()
    report=dict(task='G6-038',implementation_status='in-progress',scope='Local macOS arm64 exact durable Session projection into Core, actual authenticated physical peer views and reader ownership, signed reconnect/replay/end. No negotiated media or six physical Connection claim.',passed=len(steps)==len(commands) and all(s['passed'] for s in steps) and unchanged and not missing,executed_at=datetime.datetime.now(datetime.timezone.utc).isoformat(),schema_version=35,source_lock_patches=104,source_inputs_sha256=before,sources_unchanged=unchanged,steps=steps,required_tests=sorted(REQUIRED),missing_required_tests=missing,runtime_tests=sum(len(s['tests']) for s in steps),actual_http_samples_sha256={str(p.relative_to(ROOT)):BASE.sha(p) for p in sorted(samples.glob('*.json'))},historical_checkpoint_sha256={name:BASE.sha(ROOT/'docs/evidence/gate-6'/name) for name in ('upstream.json','canonical-http.json','session-control.json')},not_qualified=['G6-038 completion: four heterogeneous negotiated media participants, six real Connections, live stream isolation and teardown.','HTTP/ordinary mutation runtime reconciliation, durable physical Connection binding and media policy/negotiation remain to compose.','Linux/browser/remote/provider, full upstream requalification, load/performance, independent acceptance and publication.'],known_unresolved=['Historical generic facade migration: gate6_canonical_operations previously had 14 unsupported-command failures; tracked by G6-059/G6-065.'])
    if any(s['name']=='openapi' and s['passed'] for s in steps):report['http_validation']=json.loads((directory/'openapi.log').read_text())
    output=ROOT/'docs/evidence/gate-6/session-runtime.json';output.write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps({k:report[k] for k in ('passed','runtime_tests','sources_unchanged','missing_required_tests')}),flush=True)
    raise SystemExit(0 if report['passed'] else 1)
if __name__=='__main__':main()
