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
    parser.add_argument('--types-python',type=Path,help='Python with the pinned mypy test dependency')
    args=parser.parse_args()
    directory=args.output.absolute()
    assert directory.parent.resolve().is_relative_to((ROOT/'.local').resolve())
    directory.mkdir(mode=0o700,parents=True,exist_ok=False)
    private=directory.with_name(directory.name+'-private')
    private.mkdir(mode=0o700,exist_ok=False)
    for name in ['tmp','pycache']:
        (private/name).mkdir(mode=0o700)
    environment=dict(os.environ,PYTHONPATH=str(ROOT/'sdk/python'),PYTHONNOUSERSITE='1',PYTHONDONTWRITEBYTECODE='1',
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
    # Synchronous bounded local validation can trigger asyncio debug timing
    # lines between unittest's test header and its final result. Keep those
    # diagnostics in the private log and require an actual terminal ok status.
    outcomes=re.findall(r'^\w+ \(([A-Za-z0-9_.]+)\) \.\.\. (.*?)(?=^\w+ \([A-Za-z0-9_.]+\) \.\.\. |\n-+\n)',log,re.M|re.S)
    functions=[name for name,body in outcomes if body.strip()=='ok' or re.search(r'\nok\s*$',body)]
    assert all(re.fullmatch(r'[A-Za-z0-9_.]+',name) for name in functions)
    owned=[name for name in functions if '.OwnedSdkHelper.' in name]
    public=[name for name in functions if '.TypedPythonSdk.' in name]
    sessions=[name for name in functions if '.TypedPythonSessions.' in name]
    connections=[name for name in functions if '.TypedPythonConnections.' in name]
    movements=[name for name in functions if '.TypedPythonMovements.' in name]
    readers=[name for name in functions if '.TypedPythonReaders.' in name]
    raw=[name for name in functions if '.TypedPythonRaw.' in name]
    unsafe=bool(re.search(r'exception in shielded future|Task was destroyed|ResourceWarning|Exception in callback|unclosed transport|exception was never retrieved',log,re.I))
    failed_functions=re.findall(r'^\w+ \(([A-Za-z0-9_.]+)\) \.\.\. (FAIL|ERROR)$',log,re.M)
    unchanged=sources()==before
    types=None
    if args.types_python:
        type_environment=dict(environment,MYPYPATH=str(ROOT/'sdk/python'))
        commands=[('positive', ['sdk/python/connect_sdk/client.py','sdk/python/connect_sdk/sdk_types.py',
                                'sdk/python/connect_sdk/_sdk_helper.py','sdk/python/connect_sdk/sdk_sessions.py',
                                'sdk/python/connect_sdk/sdk_connections.py','sdk/python/connect_sdk/sdk_movements.py',
                                'sdk/python/connect_sdk/_sdk_readers.py','sdk/python/connect_sdk/sdk_events.py',
                                'sdk/python/connect_sdk/sdk_incoming.py','sdk/python/connect_sdk/_sdk_validation.py',
                                'sdk/python/connect_sdk/sdk_raw.py','sdk/tests/python_sdk_positive.py']),
                  ('negative', ['sdk/tests/python_sdk_negative.py'])]
        checks={}
        for label,paths in commands:
            with (private/(label+'-types.log')).open('wb') as stream:
                try:
                    checked=subprocess.run([str(args.types_python.absolute()),'-m','mypy','--follow-imports=silent',
                                            '--cache-dir',os.devnull,*paths],
                                           cwd=ROOT,env=type_environment,stdout=stream,stderr=subprocess.STDOUT,timeout=30)
                    code=checked.returncode
                except subprocess.TimeoutExpired:code=None
            type_log=(private/(label+'-types.log')).read_text(errors='replace')
            errors=re.findall(r'^sdk/tests/python_sdk_negative.py:\d+: error: .*? \[([a-z-]+)\]$',type_log,re.M)
            checks[label]={'exit_code':code,'log_sha256':digest(private/(label+'-types.log')),'negative_error_codes':errors}
        types={'passed':checks['positive']['exit_code']==0 and checks['negative']['exit_code']==1
                         and sorted(checks['negative']['negative_error_codes'])==['arg-type']*17+['attr-defined']*5+['misc']*3,
               'checks':checks}
    unchanged=unchanged and sources()==before
    result={'passed':bool(child.returncode==0 and not timed and not forced and unchanged and not unsafe
                          and len(functions)==139 and len(owned)==17 and len(public)==16 and len(sessions)==8 and len(connections)==11 and len(movements)==12 and len(readers)==20 and len(raw)==19
                          and re.search(r'^Ran 139 tests in ',log,re.M)
                          and (types is None or types['passed'])
                          and re.search(r'^OK$',log,re.M)),
            'scope':'Python owned helper and public canonical controls, consultation/handoff/replacement/return, durable event and invitation readers, frozen-parser-classified finite raw canonical exchanges, typed Operation/recovery/capability/receipt control through a synthetic child, plus existing fake SDK/HTTP/codec regressions. No decoded media, original Rust executable, real production server, installed package, complete Python SDK or G6-063 qualification.',
            'exit_code':child.returncode,'timed_out':timed,'forced_cleanup':forced,
            'source_inputs_unchanged':unchanged,'source_count':len(before),
            'source_inputs_sha256':digest(directory/'source-inputs.json'),
            'log_sha256':digest(private/'tests.log'),'unexpected_async_diagnostics':unsafe,
            'failed_functions':failed_functions,'raw_logs_and_temporary_state_excluded_from_artifact':True,
            'functions_passed':len(functions),'owned_sdk_helper_functions_passed':len(owned),'typed_public_sdk_functions_passed':len(public),'functions':functions,
            'typed_session_functions_passed':len(sessions),'typed_connection_functions_passed':len(connections),
            'typed_movement_functions_passed':len(movements),
            'typed_reader_functions_passed':len(readers),
            'typed_raw_functions_passed':len(raw),
            'public_type_checks':types,
            'python_version':sys.version.split()[0],
            'node_version':subprocess.check_output(['node','--version'],text=True).strip(),
            'seconds':time.monotonic()-started,
            'finished_at':datetime.datetime.now(datetime.timezone.utc).isoformat()}
    (directory/'summary.json').write_text(json.dumps(result,indent=2)+'\n')
    print(json.dumps({k:v for k,v in result.items() if k!='functions'}))
    raise SystemExit(0 if result['passed'] else 1)


if __name__=='__main__':main()
