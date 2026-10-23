#!/usr/bin/env python3
"""Source-bound explicit application SDK and default command qualification."""
import datetime,importlib.util,json,pathlib,platform,re,subprocess
ROOT=pathlib.Path(__file__).resolve().parents[1]
spec=importlib.util.spec_from_file_location('bootstrap',ROOT/'scripts/check-gate6-canonical-bootstrap.py')
base=importlib.util.module_from_spec(spec);spec.loader.exec_module(base);digest=base.digest

def sources():
    values=base.sources();name='scripts/check-gate6-application-sdk.py';values[name]=digest(ROOT/name);return values

def names(path,prefix=''):
    return {prefix+n for n in re.findall(r'#\[(?:tokio::)?test(?:\([^\]]*\))?\]\s*(?:async )?fn ([A-Za-z_][A-Za-z_0-9]*)\(',path.read_text())}

def main():
    directory=ROOT/'.local/gate-6/application-sdk'/datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S%fZ');directory.mkdir(parents=True);before=sources()
    cargo=['cargo','test','--locked','--offline','-j1']
    commands=[
        ('dependencies',['bash','scripts/prepare-dependencies.sh'],0),
        ('format',['bash','scripts/format.sh','--check'],0),
        ('client',cargo+['-p','connect-client','--lib','--','--test-threads=1'],36),
        ('docs',cargo+['-p','connect-client','--doc'],2),
        ('publish-race',cargo+['-p','connect-server','--lib','cli::canonical_init::tests','--','--test-threads=1'],1),
        ('application-sdk',cargo+['-p','connect-server','--test','gate6_application_sdk','--','--test-threads=1'],5),
        ('bootstrap',cargo+['-p','connect-server','--test','gate6_canonical_bootstrap','--','--test-threads=1'],5),
        ('production-native',cargo+['-p','connect-server','--test','gate6_canonical_runtime','--','--test-threads=1'],11),
        ('contracts',[str(ROOT/'.local/gate-6/verification-venv/bin/python'),'scripts/check-gate6-contracts.py','--output',str(directory/'contracts.json')],0),
        ('progress',['python3','scripts/check-gate6-progress.py'],0),
        ('diff',['git','diff','--check'],0)]
    steps=[]
    for name,command,minimum in commands:
        log=directory/(name+'.log')
        with log.open('w') as output:result=subprocess.run(command,cwd=ROOT,stdout=output,stderr=subprocess.STDOUT)
        raw=log.read_text(errors='replace');tests=re.findall(r'^test (.+?) \.\.\. ok$',raw,re.M)
        binaries={p:digest(ROOT/p) for p in re.findall(r'Running [^\n]+\((target/debug/deps/[^\)]+)\)',raw)}
        if name in ('application-sdk','bootstrap','production-native'):binaries['target/debug/connect']=digest(ROOT/'target/debug/connect')
        step=dict(name=name,command=command,exit_code=result.returncode,tests_passed=tests,private_log=str(log.relative_to(ROOT)),log_sha256=digest(log),test_binaries_sha256=binaries,passed=result.returncode==0 and len(tests)>=minimum)
        steps.append(step);print(json.dumps(dict(step=name,passed=step['passed'],tests=len(tests))),flush=True)
        if not step['passed']:break
    required={'cli::canonical_init::tests::destination_created_after_preflight_is_preserved_even_when_empty'}
    required.update(names(ROOT/'crates/connect-client/src/application/tests.rs','application::tests::'))
    required.update(names(ROOT/'crates/connect-client/src/canonical/tests.rs','canonical::tests::'))
    for name in ('gate6_application_sdk','gate6_canonical_bootstrap','gate6_canonical_runtime'):required.update(names(ROOT/f'crates/connect-server/tests/{name}.rs'))
    missing=sorted(required-{t for s in steps for t in s['tests_passed']});after=sources()
    report=dict(format=1,task='G6-037',implementation_status='in-progress',qualification='Explicit bounded local HTTP application SDK, closed canonical/application boundary, default canonical request and HTTP health/resolve commands, protected dual configuration bootstrap, current durable receipts and replay',executed_at=datetime.datetime.now(datetime.timezone.utc).isoformat(),platform=platform.platform(),source_revision=subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip(),source_inputs_sha256=before,sources_unchanged=before==after,steps=steps,required_tests=sorted(required),missing_required_tests=missing,
        not_qualified=['Earlier ConnectClient/ClientConfig and media facade, setup/presence, credential/recovery, connected-app and remaining task/example consumers; this is not complete high-level SDK migration.','All canonical/approved extension families, auth refresh/step-up, production media/G6-020, providers/voice/SIP/retirement and server physical HTTP-write custody.','The new grant tests qualify replacement/expiry, explicit discovery and unissued-token refusal without owner fallback, not every issued-grant/provider/host profile. Controlled HTTP peers qualify client behavior only.','All-workspace metadata was not repeated for these additive modules and the intentionally changed CLI contract; changed client/library/test/CLI binaries build and execute. Existing CLI compatibility/legacy workflow tests are not claimed as current qualification.','Linux, physical browser, remote/live-provider operations, release/performance, publication and Gate 6 acceptance.'],
        exploratory_attempts=[dict(private_log=str(p.relative_to(ROOT)),sha256=digest(p),disposition='Unqualified exploratory build/fixture attempt',failures_observed=bool(re.search(r'error\[|error:|FAILED',p.read_text(errors='replace')))) for p in sorted((ROOT/'.local/gate-6').glob('application-sdk-*-*.log')) if 'qualification' not in p.name],
        passed=before==after and len(steps)==len(commands) and len(required)>=40 and not missing and all(s['passed'] for s in steps))
    destination=ROOT/'docs/evidence/gate-6/application-sdk.json'
    if destination.exists():
        old=json.loads(destination.read_text());report['previous_attempts']=old.get('previous_attempts',[])+[dict(sha256=digest(destination),**{k:old[k] for k in ('passed','executed_at','source_inputs_sha256','steps')})]
    destination.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(dict(passed=report['passed'],missing=missing)),flush=True)
    if not report['passed']:raise SystemExit(1)
if __name__=='__main__':main()
