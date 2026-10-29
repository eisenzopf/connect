#!/usr/bin/env python3
"""Source-bound Python helper lifecycle and existing isolated SDK regressions."""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[2]


def digest(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def sources():
    paths = list((ROOT/'sdk/python/connect_sdk').glob('*.py'))
    paths += list((ROOT/'sdk/tests').glob('*.py'))
    paths += [ROOT/name for name in [
        'sdk/python/pyproject.toml', 'sdk/python/connect_sdk/py.typed',
        'sdk/tests/fixtures/sdk_helper.mjs', 'examples/conversation-client/pcmu.py',
        'crates/connect-client/src/sdk/helper.rs',
    ]]
    return {str(path.relative_to(ROOT)): {'sha256':digest(path),'mode':path.stat().st_mode & 0o777}
            for path in sorted(set(paths))}


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args()
    directory=args.output.absolute()
    assert directory.parent.resolve().is_relative_to((ROOT/'.local').resolve())
    directory.mkdir(mode=0o700,parents=True,exist_ok=False)
    private=directory.with_name(directory.name+'-private')
    private.mkdir(mode=0o700,exist_ok=False)
    for name in ['tmp','pycache']:
        (private/name).mkdir(mode=0o700)
    environment=dict(os.environ,PYTHONPATH=str(ROOT/'sdk/python'),PYTHONNOUSERSITE='1',
                     PYTHONPYCACHEPREFIX=str(private/'pycache'),TMPDIR=str(private/'tmp'))
    before=sources()
    (directory/'source-inputs.json').write_text(json.dumps(before,indent=2)+'\n')
    started=time.monotonic();timed=False;forced=False
    with (private/'tests.log').open('wb') as stream:
        child=subprocess.Popen([sys.executable,'-m','unittest','discover','-s','sdk/tests','-p','test_*.py','-v'],
                               cwd=ROOT,env=environment,stdout=stream,stderr=subprocess.STDOUT,start_new_session=True)
        try:
            child.wait(timeout=60)
        except subprocess.TimeoutExpired:
            timed=True;os.killpg(child.pid,signal.SIGTERM)
            try:child.wait(timeout=5)
            except subprocess.TimeoutExpired:
                forced=True;os.killpg(child.pid,signal.SIGKILL);child.wait(timeout=5)
    log=(private/'tests.log').read_text(errors='replace')
    functions=re.findall(r'^\w+ \(([^)]+)\) \.\.\. ok$',log,re.M)
    assert all(re.fullmatch(r'[A-Za-z0-9_.]+',name) for name in functions)
    owned=[name for name in functions if '.OwnedSdkHelper.' in name]
    unsafe=bool(re.search(r'exception in shielded future|Task was destroyed|ResourceWarning|Exception in callback|unclosed transport|exception was never retrieved',log,re.I))
    failed_functions=re.findall(r'^\w+ \(([A-Za-z0-9_.]+)\) \.\.\. (FAIL|ERROR)$',log,re.M)
    unchanged=sources()==before
    result={'passed':bool(child.returncode==0 and not timed and not forced and unchanged and not unsafe
                          and len(functions)==53 and len(owned)==17 and re.search(r'^Ran 53 tests in ',log,re.M)
                          and re.search(r'^OK$',log,re.M)),
            'scope':'Python standard-library owned SDK process/FIFO/deadline/cancellation/strict JSON tests through a synthetic helper, plus existing fake local SDK/HTTP/codec regression tests. No original Rust executable, real production server, installed package, full typed Python SDK or G6-063 qualification.',
            'exit_code':child.returncode,'timed_out':timed,'forced_cleanup':forced,
            'source_inputs_unchanged':unchanged,'source_count':len(before),
            'source_inputs_sha256':digest(directory/'source-inputs.json'),
            'log_sha256':digest(private/'tests.log'),'unexpected_async_diagnostics':unsafe,
            'failed_functions':failed_functions,'raw_logs_and_temporary_state_excluded_from_artifact':True,
            'functions_passed':len(functions),'owned_sdk_helper_functions_passed':len(owned),'functions':functions,
            'python_version':sys.version.split()[0],
            'node_version':subprocess.check_output(['node','--version'],text=True).strip(),
            'seconds':time.monotonic()-started,
            'finished_at':datetime.datetime.now(datetime.timezone.utc).isoformat()}
    (directory/'summary.json').write_text(json.dumps(result,indent=2)+'\n')
    print(json.dumps({k:v for k,v in result.items() if k!='functions'}))
    raise SystemExit(0 if result['passed'] else 1)


if __name__=='__main__':main()
