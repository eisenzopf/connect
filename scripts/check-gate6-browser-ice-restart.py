#!/usr/bin/env python3
"""Run owned loopback Chrome ICE recovery and the unchanged clean AV profile.

Requires explicit installed tools. No named remote, provider, model or device
capture: both sources are synthetic canvas/audio, normal private CA trust.
"""
import argparse
import datetime
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('ice_inputs', ROOT/'scripts/check-gate6-ice-restart.py')
BASE = importlib.util.module_from_spec(spec)
spec.loader.exec_module(BASE)


def sha(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('output', 'target-dir', 'chrome', 'node', 'playwright-module'):
        parser.add_argument('--'+name, type=Path, required=True)
    args = parser.parse_args()
    output=args.output.resolve()
    assert output.is_relative_to((ROOT/'.local').resolve()), 'Use new ignored .local output'
    output.mkdir(parents=True, exist_ok=False)
    for name in ('chrome','node','playwright_module'):
        path=getattr(args,name)
        assert path.is_absolute() and path.is_file(), name
    target=args.target_dir.resolve()
    assert '.local' in target.parts, 'Keep compiler state under ignored .local'

    def inputs():
        value=BASE.inputs()
        for path in (ROOT/'examples/conversation-client').rglob('*'):
            if path.is_file():value[str(path.relative_to(ROOT))]=sha(path)
        tools=[args.chrome,args.node,args.playwright_module,args.playwright_module.parent/'package.json']
        # Bind the installed browser engine as well as its small launcher.
        tools+=list((args.chrome.parents[1]/'Frameworks').glob('Google Chrome Framework.framework/Versions/*/Google Chrome Framework'))
        for path in tools:
            if path.is_file():value['tool:'+str(path)]=sha(path)
        return dict(sorted(value.items()))

    before=inputs()
    (output/'inputs-before.json').write_text(json.dumps(before,indent=2)+'\n')
    env=dict(os.environ,CARGO_TARGET_DIR=str(target),CARGO_PROFILE_DEV_DEBUG='0',CARGO_PROFILE_TEST_DEBUG='0',
             CARGO_PROFILE_DEV_OPT_LEVEL='0',CARGO_PROFILE_TEST_OPT_LEVEL='0',CARGO_INCREMENTAL='0',CARGO_BUILD_JOBS='1',
             CONNECT_GATE6_CHROME=str(args.chrome),CONNECT_GATE6_NODE=str(args.node),CONNECT_PLAYWRIGHT_MODULE=str(args.playwright_module))
    steps=[]

    def run(name,command,extra=None):
        log=output/(name+'.log')
        started=datetime.datetime.now(datetime.timezone.utc)
        with log.open('w') as stream:
            result=subprocess.run(command,cwd=ROOT,env=dict(env,**(extra or {})),stdout=stream,stderr=subprocess.STDOUT)
        row=dict(name=name,command=command,exit_code=result.returncode,passed=result.returncode==0,
                 seconds=(datetime.datetime.now(datetime.timezone.utc)-started).total_seconds(),
                 log=str(log.relative_to(ROOT)),log_sha256=sha(log))
        steps.append(row)
        print(json.dumps(row),flush=True)
        return row['passed']

    ok=run('dependencies',['bash','scripts/prepare-dependencies.sh'])
    if ok:ok=run('format',['bash','scripts/format.sh','--check'])
    if ok:ok=run('sdk-build',['sdk/typescript/node_modules/.bin/tsc','-p','sdk/typescript/tsconfig.json'])
    if ok:ok=run('build',['cargo','test','--offline','--locked','-j1','-p','connect-server','--features','test-support',
                             '--test','gate6_canonical_webtransport','--no-run','--message-format=json'])
    binary=None
    binary_sha=None
    if ok:
        rows=[]
        for line in (output/'build.log').read_text(errors='replace').splitlines():
            try:row=json.loads(line)
            except ValueError:continue
            if row.get('reason')=='compiler-artifact' and row['target']['name']=='gate6_canonical_webtransport' and row.get('executable'):
                rows.append(row['executable'])
        assert len(rows)==1
        binary=rows[0]
        binary_sha=sha(Path(binary))
    results=[]
    for label in ('eddsa','es256'):
        if not ok:break
        directory=output/label
        directory.mkdir()
        case='canonical_webrtc_browser::canonical_chrome_original_peer_ice_restart_clean_'+label
        passed=run(label,[binary,case,'--exact','--ignored','--test-threads=1'],
                   {'CONNECT_GATE6_CANONICAL_BROWSER_ARTIFACTS':str(directory)})
        raw=(output/(label+'.log')).read_text(errors='replace')
        passed=passed and re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;',raw)==[('1','0','0')]
        steps[-1]['passed']=passed
        artifacts={str(path.relative_to(output)):sha(path) for path in directory.iterdir() if path.is_file()}
        results.append(dict(algorithm=label,passed=passed,artifacts_sha256=artifacts))
    after=inputs()
    changed=sorted(name for name in before.keys()|after.keys() if before.get(name)!=after.get(name))
    report=dict(task='G6-054',full_task_qualified=False,acceptance_qualified=False,
                scope='Actual installed Chrome, signed original WSS peers and public BrowserVideoSession same-peer two-generation ICE/recovery; independent decoded clean60second audio/video follows restart. Loopback only.',
                source_inputs_sha256=before,sources_unchanged=not changed,changed_inputs=changed,
                binary=binary,binary_sha256=binary_sha,steps=steps,results=results,
                not_run_algorithms=[name for name in ('eddsa','es256') if name not in {r['algorithm'] for r in results}],
                not_qualified=['Distinct-network/relay/expiry/loss ICE matrix','Firefox/native/Linux or full Gate6 acceptance'],
                passed=ok and len(results)==2 and all(r['passed'] for r in results) and not changed)
    (output/'summary.json').write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(dict(passed=report['passed'],sources=len(before),changed_inputs=changed)),flush=True)
    raise SystemExit(0 if report['passed'] else 1)


if __name__=='__main__':main()
