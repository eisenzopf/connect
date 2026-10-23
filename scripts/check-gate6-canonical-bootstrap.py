#!/usr/bin/env python3
"""Source-bound default signed init/serve qualification on local macOS arm64."""
import datetime, importlib.util, json, pathlib, platform, re, subprocess
ROOT=pathlib.Path(__file__).resolve().parents[1]
spec=importlib.util.spec_from_file_location('calls',ROOT/'scripts/check-gate6-canonical-calls.py')
base=importlib.util.module_from_spec(spec);spec.loader.exec_module(base)
digest=base.digest

def sources():
    values=base.sources()
    name='scripts/check-gate6-canonical-bootstrap.py';values[name]=digest(ROOT/name)
    return values

def main():
    directory=ROOT/'.local/gate-6/canonical-bootstrap'/datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S%fZ')
    directory.mkdir(parents=True);before=sources()
    cargo=['cargo','test','--locked','--offline','-j1']
    commands=[
        ('dependencies',['bash','scripts/prepare-dependencies.sh'],0),
        ('format',['bash','scripts/format.sh','--check'],0),
        ('publish-race',cargo+['-p','connect-server','--lib','cli::canonical_init::tests','--','--test-threads=1'],1),
        ('bootstrap',cargo+['-p','connect-server','--test','gate6_canonical_bootstrap','--','--test-threads=1'],5),
        ('production-native',cargo+['-p','connect-server','--test','gate6_canonical_runtime','--','--test-threads=1'],11),
        ('contracts',[str(ROOT/'.local/gate-6/verification-venv/bin/python'),'scripts/check-gate6-contracts.py','--output',str(directory/'contracts.json')],0),
        ('progress',['python3','scripts/check-gate6-progress.py'],0),
        ('diff',['git','diff','--check'],0)]
    steps=[]
    for name,command,minimum in commands:
        log=directory/(name+'.log')
        with log.open('w') as output:result=subprocess.run(command,cwd=ROOT,stdout=output,stderr=subprocess.STDOUT)
        raw=log.read_text(errors='replace');tests=re.findall(r'^test (\S+) \.\.\. ok$',raw,re.M)
        binaries={p:digest(ROOT/p) for p in re.findall(r'Running [^\n]+\((target/debug/deps/[^\)]+)\)',raw)}
        if name in ('bootstrap','production-native'):binaries['target/debug/connect']=digest(ROOT/'target/debug/connect')
        step=dict(name=name,command=command,exit_code=result.returncode,tests_passed=tests,private_log=str(log.relative_to(ROOT)),log_sha256=digest(log),test_binaries_sha256=binaries,passed=result.returncode==0 and len(tests)>=minimum)
        steps.append(step);print(json.dumps(dict(step=name,passed=step['passed'],tests=len(tests))),flush=True)
        if not step['passed']:break
    required={'cli::canonical_init::tests::destination_created_after_preflight_is_preserved_even_when_empty'}
    for name in ('gate6_canonical_bootstrap','gate6_canonical_runtime'):
        required.update(re.findall(r'#\[(?:tokio::)?test(?:\([^\]]*\))?\]\s*(?:async )?fn ([A-Za-z_][A-Za-z_0-9]*)\(', (ROOT/f'crates/connect-server/tests/{name}.rs').read_text()))
    missing=sorted(required-{t for step in steps for t in step['tests_passed']});after=sources()
    report=dict(format=1,task='G6-037',implementation_status='in-progress',qualification='Default signed canonical CLI init/serve, protected explicit key/pin enrollment, atomic no-replace publication and production runtime regression on local macOS arm64',executed_at=datetime.datetime.now(datetime.timezone.utc).isoformat(),platform=platform.platform(),source_revision=subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip(),source_inputs_sha256=before,sources_unchanged=before==after,steps=steps,required_tests=sorted(required),missing_required_tests=missing,
        not_qualified=['Earlier ConnectClient/ClientConfig, setup/presence, health/diagnostics/request, credential/recovery tooling, connected-app and example application migration. Review-loop preset generates enrolled configs only.','Full canonical/approved application families, auth refresh/step-up, production media, providers/voice/SIP/retirement and underlying HTTP write custody.','Power-loss/crash publication recovery; all-workspace metadata check was not repeated for this scoped CLI/module addition. The actual changed library/test/CLI binaries are built and exercised.','Linux, physical browser, remote/provider/live operations, publication and all Gate 6 acceptance.'],
        exploratory_attempts=[dict(private_log=str(p.relative_to(ROOT)),sha256=digest(p),disposition='Unqualified exploratory build/fixture attempt',failures_observed=bool(re.search(r'error\[|error:|FAILED',p.read_text(errors='replace')))) for p in sorted((ROOT/'.local/gate-6').glob('canonical-bootstrap-*-*.log')) if 'qualification' not in p.name],
        passed=before==after and len(steps)==len(commands) and len(required)>=17 and not missing and all(s['passed'] for s in steps))
    destination=ROOT/'docs/evidence/gate-6/canonical-bootstrap.json'
    if destination.exists():
        old=json.loads(destination.read_text());report['previous_attempts']=old.get('previous_attempts',[])+[dict(sha256=digest(destination),**{k:old[k] for k in ('passed','executed_at','source_inputs_sha256','steps')})]
    destination.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(dict(passed=report['passed'],missing=missing)),flush=True)
    if not report['passed']:raise SystemExit(1)
if __name__=='__main__':main()
