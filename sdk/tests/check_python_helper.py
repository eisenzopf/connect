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
        'sdk/tests/fixtures/sdk_helper.mjs', 'sdk/tests/fixtures/sdk_media_helper.py', 'examples/conversation-client/pcmu.py',
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
    started=time.monotonic();timed=False;forced=False;phases={}
    # Discover the complete suite in both children; partition by module so no
    # nested/failed discovery test is silently omitted. Core keeps its original
    # 60s bound; HTTP/TLS protocol keeps30s. The two deliberately stalled
    # lifetime tests have their own10s bound and2s actual caller deadlines.
    run_suite='''import sys,unittest,json
suite=unittest.defaultTestLoader.discover("sdk/tests",pattern="test_*.py")
def leaves(suite):
 for item in suite:
  if isinstance(item,unittest.TestSuite):yield from leaves(item)
  else:yield item
def phase(case):
 if case.__class__.__module__ not in ("test_python_http","test_python_https"):return "core"
 if case._testMethodName in ("test_deadline_and_cancel_close_original_socket_and_keep_uncertain_mutation_key","test_deadline_terminates_and_joins_a_worker_blocked_inside_its_resolver"):return "http-lifetime"
 return "http"
selected=[case for case in leaves(suite) if phase(case)==sys.argv[1]]
class RecordingResult(unittest.TextTestResult):
 def __init__(self,*args,**kwargs):super().__init__(*args,**kwargs);self.passed=[]
 def addSuccess(self,test):super().addSuccess(test);self.passed.append(test.id())
result=unittest.TextTestRunner(verbosity=2,resultclass=RecordingResult).run(unittest.TestSuite(selected))
print("connect-python-component-result "+json.dumps({"phase":sys.argv[1],"selected":[t.id() for t in selected],"passed":result.passed,"tests_run":result.testsRun,"failures":len(result.failures),"errors":len(result.errors),"skipped":len(result.skipped)},separators=(",",":")),flush=True)
raise SystemExit(not result.wasSuccessful())
'''
    for label,budget in [('core',60),('http',30),('http-lifetime',10)]:
        phase_started=time.monotonic();phase_timed=False;phase_forced=False
        with (private/(label+'-tests.log')).open('wb') as stream:
            child=subprocess.Popen([sys.executable,'-c',run_suite,label],cwd=ROOT,env=environment,
                                   stdout=stream,stderr=subprocess.STDOUT,start_new_session=True)
            try:child.wait(timeout=budget)
            except subprocess.TimeoutExpired:
                phase_timed=True;os.killpg(child.pid,signal.SIGTERM)
                try:child.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    phase_forced=True;os.killpg(child.pid,signal.SIGKILL);child.wait(timeout=5)
        phases[label]={'exit_code':child.returncode,'budget_seconds':budget,'timed_out':phase_timed,
                       'forced_cleanup':phase_forced,'seconds':time.monotonic()-phase_started,
                       'log_sha256':digest(private/(label+'-tests.log'))}
        timed=timed or phase_timed;forced=forced or phase_forced
    (private/'tests.log').write_bytes(b'\n'.join((private/(label+'-tests.log')).read_bytes() for label in phases))
    log=(private/'tests.log').read_text(errors='replace')
    # Exact unittest result IDs remain independent of interleaved asyncio
    # debug text. Missing/mismatched inventories cannot become a passing scope.
    records=[json.loads(line.split(' ',1)[1]) for line in log.splitlines()
             if line.startswith('connect-python-component-result ')]
    inventories=len(records)==3 and {r['phase'] for r in records}==set(phases)
    for record in records:
        assert set(record)=={'phase','selected','passed','tests_run','failures','errors','skipped'}
        assert len(set(record['selected']))==len(record['selected'])==record['tests_run']
        assert len(set(record['passed']))==len(record['passed'])
        assert set(record['passed']).issubset(record['selected'])
        inventories=inventories and (record['failures']==record['errors']==record['skipped']==0
            and set(record['passed'])==set(record['selected']))
    functions=[name for record in records for name in record['passed']]
    assert all(re.fullmatch(r'[A-Za-z0-9_.]+',name) for name in functions)
    owned=[name for name in functions if '.OwnedSdkHelper.' in name]
    public=[name for name in functions if '.TypedPythonSdk.' in name]
    sessions=[name for name in functions if '.TypedPythonSessions.' in name]
    connections=[name for name in functions if '.TypedPythonConnections.' in name]
    movements=[name for name in functions if '.TypedPythonMovements.' in name]
    readers=[name for name in functions if '.TypedPythonReaders.' in name]
    raw=[name for name in functions if '.TypedPythonRaw.' in name]
    administrative=[name for name in functions if '.TypedPythonAdministration.' in name]
    http=[name for name in functions if '.TypedPythonHttp.' in name]
    https=[name for name in functions if '.TypedPythonHttps.' in name]
    synchronous=[name for name in functions if '.TypedPythonSync.' in name]
    media=[name for name in functions if '.PythonMedia.' in name]
    data=[name for name in functions if '.PythonData.' in name]
    unsafe=bool(re.search(r'exception in shielded future|Task was destroyed|ResourceWarning|Exception in callback|unclosed transport|exception was never retrieved',log,re.I))
    failed_functions=re.findall(r'^\w+ \(([A-Za-z0-9_.]+)\) \.\.\. (FAIL|ERROR)$',log,re.M)
    failed_functions=sorted(set(failed_functions) | {
        (name,kind) for kind,name in re.findall(r'^(FAIL|ERROR): \w+ \(([A-Za-z0-9_.]+)\)$',log,re.M)
    })
    # Publish only repository source locations, never assertion text, private
    # state, payloads, absolute paths or raw tracebacks from a failed fixture.
    failure_sites=[{'file':name,'line':int(line)} for name,line in sorted(set(
        re.findall(r'File "[^"\n]*/(sdk/(?:tests|python/connect_sdk)/[A-Za-z0-9_/]+\.py)", line (\d+)',log))) ]
    unchanged=sources()==before
    types=None
    if args.types_python:
        type_environment=dict(environment,MYPYPATH=str(ROOT/'sdk/python'))
        commands=[('positive', ['sdk/python/connect_sdk/client.py','sdk/python/connect_sdk/sdk_types.py',
                                'sdk/python/connect_sdk/_sdk_helper.py','sdk/python/connect_sdk/sdk_sessions.py',
                                'sdk/python/connect_sdk/sdk_connections.py','sdk/python/connect_sdk/sdk_movements.py',
                                'sdk/python/connect_sdk/_sdk_readers.py','sdk/python/connect_sdk/sdk_events.py',
                                'sdk/python/connect_sdk/sdk_incoming.py','sdk/python/connect_sdk/_sdk_validation.py',
                                'sdk/python/connect_sdk/sdk_raw.py','sdk/python/connect_sdk/sdk_administration.py',
                                'sdk/python/connect_sdk/_sdk_http_protocol.py','sdk/python/connect_sdk/_sdk_http_worker.py',
                                'sdk/python/connect_sdk/sdk_http.py','sdk/python/connect_sdk/sdk_sync.py',
                                'sdk/python/connect_sdk/_sdk_media_channel.py','sdk/python/connect_sdk/sdk_media_types.py',
                                'sdk/python/connect_sdk/sdk_media.py','sdk/python/connect_sdk/sdk_data_types.py',
                                'sdk/tests/python_sdk_positive.py']),
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
                         and sorted(checks['negative']['negative_error_codes'])==['arg-type']*28+['attr-defined']*9+['misc']*6,
               'checks':checks}
    unchanged=unchanged and sources()==before
    result={'passed':bool(inventories and all(phase['exit_code']==0 for phase in phases.values()) and not timed and not forced and unchanged and not unsafe
                          and len(functions)==217 and len(owned)==17 and len(public)==16 and len(sessions)==8 and len(connections)==11 and len(movements)==12 and len(readers)==20 and len(raw)==19 and len(administrative)==17 and len(http)==21 and len(https)==4 and len(synchronous)==13 and len(media)==12 and len(data)==11
                          and re.search(r'^Ran 192 tests in ',log,re.M) and re.search(r'^Ran 23 tests in ',log,re.M) and re.search(r'^Ran 2 tests in ',log,re.M)
                          and (types is None or types['passed'])
                          and re.search(r'^OK$',log,re.M)),
            'scope':'Python owned helper and public canonical controls, consultation/handoff/replacement/return, durable event and invitation readers, finite raw and separate administrative controls through synthetic children, plus actual local HTTP/verified HTTPS servers and owned per-exchange Python workers, with persistent caller-thread synchronous ownership, bounded binary media and full64KiB typed-data channels through a synthetic owned child, and historical regressions. No decoded media, original Rust executable, real production server, installed package, complete Python SDK or G6-063 qualification.',
            'exit_code':next((phase['exit_code'] for phase in phases.values() if phase['exit_code']),0),'timed_out':timed,'forced_cleanup':forced,'phases':phases,
            'source_inputs_unchanged':unchanged,'source_count':len(before),
            'source_inputs_sha256':digest(directory/'source-inputs.json'),
            'log_sha256':digest(private/'tests.log'),'unexpected_async_diagnostics':unsafe,
            'failed_functions':failed_functions,'failure_sites':failure_sites,
            'raw_logs_and_temporary_state_excluded_from_artifact':True,
            'functions_passed':len(functions),'owned_sdk_helper_functions_passed':len(owned),'typed_public_sdk_functions_passed':len(public),'functions':functions,
            'typed_session_functions_passed':len(sessions),'typed_connection_functions_passed':len(connections),
            'typed_movement_functions_passed':len(movements),
            'typed_reader_functions_passed':len(readers),
            'typed_raw_functions_passed':len(raw),
            'typed_administrative_functions_passed':len(administrative),
            'synchronous_functions_passed':len(synchronous),'typed_programmatic_media_functions_passed':len(media),'typed_reliable_data_functions_passed':len(data),
            'typed_http_functions_passed':len(http),'typed_https_functions_passed':len(https),
            'public_type_checks':types,
            'python_version':sys.version.split()[0],
            'node_version':subprocess.check_output(['node','--version'],text=True).strip(),
            'seconds':time.monotonic()-started,
            'finished_at':datetime.datetime.now(datetime.timezone.utc).isoformat()}
    (directory/'summary.json').write_text(json.dumps(result,indent=2)+'\n')
    print(json.dumps({k:v for k,v in result.items() if k!='functions'}))
    raise SystemExit(0 if result['passed'] else 1)


if __name__=='__main__':main()
