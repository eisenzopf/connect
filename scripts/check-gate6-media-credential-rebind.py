#!/usr/bin/env python3
"""Source-bound retained media credential rebinding qualification."""
import datetime, importlib.util, json, pathlib, platform, re, subprocess
ROOT=pathlib.Path(__file__).resolve().parents[1]
spec=importlib.util.spec_from_file_location('previous',ROOT/'scripts/check-gate6-participant-control.py')
base=importlib.util.module_from_spec(spec);spec.loader.exec_module(base);digest=base.digest
UPSTREAM=ROOT/'.deps/rvoip'
FILES=["crates/foundation/rvoip-core/src/relationships.rs", "crates/foundation/rvoip-core/src/staged_streams.rs", "crates/foundation/rvoip-core/src/staged_streams/credential_rebind.rs", "crates/foundation/rvoip-core/src/staged_streams/credential_rebind_tests.rs", "crates/foundation/rvoip-core/src/staged_streams/tests.rs", "crates/foundation/rvoip-core/src/stream_lifecycle.rs", "crates/uctp/rvoip-uctp/src/v2/native.rs", "crates/uctp/rvoip-uctp/tests/support/credential_rebind_wire_tests.rs", "crates/uctp/rvoip-uctp/tests/support/media_control.rs"]
def sources():
    values=base.sources()
    for name in ['scripts/check-gate6-media-credential-rebind.py','docs/developer/media-credential-rebind.md','docs/developer/media-control.md','patches/rvoip/gate6-media-credential-rebind.patch','dependencies/source-lock.json']:
        values[name]=digest(ROOT/name)
    for name in FILES:values['.deps/rvoip/'+name]=digest(UPSTREAM/name)
    return values

def main():
    assert platform.system()=='Darwin' and platform.machine()=='arm64'
    directory=ROOT/'.local/gate-6/media-credential-rebind'/datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S%fZ');directory.mkdir(parents=True);before=sources()
    cargo=['cargo','test','--locked','--offline','-j1','--target-dir','target','--manifest-path','.deps/rvoip/Cargo.toml']
    suites=['participant_control','participant_roles','session_lifecycle','peer_admission','peer_sessions','stream_readiness','per_connection_control','conversation_lifecycle']
    commands=[
        ('dependencies',['bash','scripts/prepare-dependencies.sh'],0),
        ('format',['bash','scripts/format.sh','--check'],0),
        ('upstream-format',['rustfmt','--edition','2021','--config','skip_children=true','--check']+['.deps/rvoip/'+name for name in FILES],0),
        ('core-library',cargo+['-p','rvoip-core','--features','opus','--lib','--','--test-threads=1'],263),
        ('participant-session-peer',cargo+['-p','rvoip-core','--features','opus']+[arg for suite in suites for arg in ('--test',suite)]+['--','--test-threads=1'],64),
        ('canonical-peer',cargo+['-p','rvoip-uctp','--test','v2_peer_admission','--','--test-threads=1'],47),
        ('contracts',[str(ROOT/'.local/gate-6/verification-venv/bin/python'),'scripts/check-gate6-contracts.py','--output',str(directory/'contracts.json')],0),
        ('progress',['python3','scripts/check-gate6-progress.py'],0),
        ('diff',['git','diff','--check'],0)]
    steps=[]
    for name,command,minimum in commands:
        log=directory/(name+'.log')
        with log.open('w') as output:result=subprocess.run(command,cwd=ROOT,stdout=output,stderr=subprocess.STDOUT)
        raw=log.read_text(errors='replace');tests=re.findall(r'^test (.+?) \.\.\. ok$',raw,re.M)
        binaries={path:digest(ROOT/path) for path in re.findall(r'Running [^\n]+\((target/debug/deps/[^\)]+)\)',raw)}
        step=dict(name=name,command=command,exit_code=result.returncode,tests_passed=tests,private_log=str(log.relative_to(ROOT)),log_sha256=digest(log),test_binaries_sha256=binaries,passed=result.returncode==0 and len(tests)>=minimum)
        steps.append(step);print(json.dumps(dict(step=name,passed=step['passed'],tests=len(tests))),flush=True)
        if not step['passed']:break
    required={'staged_streams::tests::credential_rebind_tests::'+n for n in base.base.names(UPSTREAM/'crates/foundation/rvoip-core/src/staged_streams/credential_rebind_tests.rs')}
    required.update('peer_native::canonical_media::media_control::credential_rebind_wire_tests::'+n for n in base.base.names(UPSTREAM/'crates/uctp/rvoip-uctp/tests/support/credential_rebind_wire_tests.rs'))
    missing=sorted(required-{name for step in steps for name in step['tests_passed']});after=sources();historical=ROOT/'docs/evidence/gate-6/participant-control.json'
    report=dict(format=1,task='G6-020',implementation_status='in-progress',qualification='Checked same-origin retained media credential rebinding with pinned control snapshots, drained sends, invalidated old queues and stable model/handles/history; actual signed local TLS QUIC/WT refresh plus trusted fixture SPI and decoded RTP after original expiry; local macOS arm64',executed_at=datetime.datetime.now(datetime.timezone.utc).isoformat(),platform=platform.platform(),source_revision=subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip(),source_inputs_sha256=before,sources_unchanged=before==after,source_lock_patches=len(json.loads((ROOT/'dependencies/source-lock.json').read_text())['patches']),steps=steps,required_tests=sorted(required),missing_required_tests=missing,historical_participant_control=dict(path=str(historical.relative_to(ROOT)),sha256=digest(historical),disposition='Unchanged historical source snapshot; not current-source qualification'),
        not_qualified=['Connect production Store media/participant policy and automatic refresh/step-up rebinding installation. Wire handlers/readiness/current relationship mapping are controlled fixtures; the rebind SPI is invoked under a later signed control command after authenticated refresh, not a newly installed public wire operation.', 'Canonical offer/answer/ready and actual endpoint confirmation/Participant presence, managed guarded mixer/subscriptions, general durable Session projection and complete public Session/Connection/Stream integration.', 'Full Connect workspace/package/browser/provider/remote profiles, cold startup/release/performance, all Gate 6 acceptance and publication.'],
        exploratory_attempts=[dict(private_log=str(path.relative_to(ROOT)),sha256=digest(path),disposition='Unqualified exploratory run') for path in sorted((ROOT/'.local/gate-6').glob('credential-rebind-*-exploratory.log'))],passed=before==after and len(steps)==len(commands) and len(required)==15 and not missing and all(s['passed'] for s in steps))
    destination=ROOT/'docs/evidence/gate-6/media-credential-rebind.json'
    if destination.exists():
        old=json.loads(destination.read_text());report['previous_attempts']=old.get('previous_attempts',[])+[dict(sha256=digest(destination),**{k:old[k] for k in ('passed','executed_at','source_inputs_sha256','steps')})]
    destination.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(dict(passed=report['passed'],missing=missing)),flush=True)
    if not report['passed']:raise SystemExit(1)
if __name__=='__main__':main()
