#!/usr/bin/env python3
"""Source-bound local qualification of the production signed native runtime."""
import datetime
import hashlib
import importlib.util
import json
import pathlib
import platform
import re
import subprocess

ROOT=pathlib.Path(__file__).resolve().parents[1]
spec=importlib.util.spec_from_file_location('media',ROOT/'scripts/check-gate6-media-control.py')
base=importlib.util.module_from_spec(spec)
spec.loader.exec_module(base)
def digest(path):
    h=hashlib.sha256()
    with path.open('rb') as source:
        for block in iter(lambda:source.read(1024*1024),b''):h.update(block)
    return h.hexdigest()
def sources():
    values=base.sources()
    paths=[ROOT/name for name in ('Cargo.toml','Cargo.lock','scripts/format.sh','scripts/check-gate6-canonical-runtime.py','docs/developer/canonical-runtime.md')]
    for directory in (ROOT/'crates',):
        paths+=list(directory.rglob('*.rs'))+list(directory.rglob('Cargo.toml'))
    paths+=[ROOT/name for name in subprocess.check_output(['rg','--files','-g','*.rs','-g','Cargo.toml','examples'],cwd=ROOT,text=True).splitlines()]
    paths+=list((ROOT/'docs/developer').glob('*.md'))
    paths+=list((ROOT/'contracts').rglob('*.json'))+list((ROOT/'contracts').glob('*.md'))
    for path in sorted(set(paths)):
        if 'target' not in path.parts:values[str(path.relative_to(ROOT))]=digest(path)
    return values

def main():
    directory=ROOT/'.local/gate-6/canonical-runtime'/datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S%fZ')
    directory.mkdir(parents=True);before=sources()
    cargo=['cargo','test','--locked','--offline','-j1']
    commands=[
        ('dependencies',['bash','scripts/prepare-dependencies.sh'],0),
        ('format',['bash','scripts/format.sh','--check'],0),
        ('client',cargo+['-p','connect-client','--lib','--tests'],1),
        ('server-library',cargo+['-p','connect-server','--lib'],1),
        ('production-native',cargo+['-p','connect-server','--test','gate6_canonical_runtime','--','--test-threads=1'],7),
        ('canonical-service',cargo+['-p','connect-server','--test','gate6_canonical_operations','--','--test-threads=1'],14),
        ('review-loop',cargo+['-p','connect-review-loop','--lib'],17),
        ('workspace-targets',['cargo','check','--locked','--offline','-j1','--workspace','--all-targets'],0),
        ('contracts',[str(ROOT/'.local/gate-6/verification-venv/bin/python'),'scripts/check-gate6-contracts.py','--output',str(directory/'contracts.json')],0),
        ('progress',['python3','scripts/check-gate6-progress.py'],0),
        ('diff',['git','diff','--check'],0)]
    steps=[]
    for name,command,minimum in commands:
        log=directory/(name+'.log')
        with log.open('w') as output: result=subprocess.run(command,cwd=ROOT,stdout=output,stderr=subprocess.STDOUT)
        raw=log.read_text(errors='replace');tests=re.findall(r'^test (\S+) \.\.\. ok$',raw,re.M)
        binaries={path:digest(ROOT/path) for path in re.findall(r'Running [^\n]+\((target/debug/deps/[^\)]+)\)',raw)}
        if name=='canonical-service' and (ROOT/'target/debug/connect').is_file():binaries['target/debug/connect']=digest(ROOT/'target/debug/connect')
        step=dict(name=name,command=command,exit_code=result.returncode,tests_passed=tests,private_log=str(log.relative_to(ROOT)),log_sha256=digest(log),test_binaries_sha256=binaries,passed=result.returncode==0 and len(tests)>=minimum)
        steps.append(step);print(json.dumps(dict(step=name,passed=step['passed'],tests=len(tests))),flush=True)
        if not step['passed']:break
    required=set(re.findall(r'#\[(?:tokio::)?test(?:\([^\]]*\))?\]\s*(?:async )?fn ([A-Za-z_][A-Za-z_0-9]*)\(',(ROOT/'crates/connect-server/tests/gate6_canonical_runtime.rs').read_text()))
    missing=sorted(required-{t for step in steps for t in step['tests_passed']});after=sources()
    report=dict(format=1,task='G6-037',implementation_status='in-progress',qualification='Production opt-in signed local QUIC listener, public canonical Rust client, actual CLI/HTTP shared Store composition; existing default listener/high-level SDK migration remains open',executed_at=datetime.datetime.now(datetime.timezone.utc).isoformat(),platform=platform.platform(),source_revision=subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip(),source_inputs_sha256=before,sources_unchanged=before==after,steps=steps,required_tests=sorted(required),missing_required_tests=missing,
        not_qualified=['Default serve/ConnectClient migration, full high-level SDK facade parity and auth refresh/step-up composition.','Canonical media/remaining command and extension families, provider/voice/SIP/retirement and underlying HTTP write custody.','Linux, physical browser, remote/provider/live operations, publication and every Gate 6 acceptance criterion.'],
        exploratory_attempts=[dict(private_log=str(p.relative_to(ROOT)),sha256=digest(p),disposition='Unqualified exploratory compile/fixture attempt',failures_observed=bool(re.search(r'error\[|error:|FAILED',p.read_text(errors='replace')))) for p in sorted((ROOT/'.local/gate-6').glob('canonical-runtime-*-*.log')) if 'qualification' not in p.name],
        passed=before==after and len(steps)==len(commands) and len(required)>=7 and not missing and all(s['passed'] for s in steps))
    destination=ROOT/'docs/evidence/gate-6/canonical-runtime.json'
    if destination.exists():
        old=json.loads(destination.read_text());report['previous_attempts']=old.get('previous_attempts',[])+[dict(sha256=digest(destination),**{k:old[k] for k in ('passed','executed_at','source_inputs_sha256','steps')})]
    destination.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(dict(passed=report['passed'],missing=missing)),flush=True)
    if not report['passed']:raise SystemExit(1)
if __name__=='__main__':main()
