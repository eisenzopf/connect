#!/usr/bin/env python3
"""Source-bound Python helper lifecycle and existing isolated SDK regressions."""
import argparse
from collections import Counter
from contextlib import ExitStack
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


def bounded_cohort(commands, environment, logs, budget):
    """Independent process groups share one absolute execution/cleanup clock."""
    started=time.monotonic();deadline=started+budget;children=[];timed=False;forced=False;survivor=False
    def alive(child):
        # Reap exited siblings before probing their process-group liveness.
        child.poll()
        try:os.killpg(child.pid,0);return True
        except ProcessLookupError:return False
    with ExitStack() as stack:
        try:
            for command,log in zip(commands,logs,strict=True):
                if time.monotonic()>=deadline:
                    timed=True;break
                stream=stack.enter_context(log.open('wb'))
                children.append(subprocess.Popen(command,cwd=ROOT,env=environment,stdout=stream,
                    stderr=subprocess.STDOUT,start_new_session=True))
            while any(child.poll() is None for child in children) and time.monotonic()<deadline:
                time.sleep(min(.01,max(0,deadline-time.monotonic())))
            timed=timed or time.monotonic()>=deadline
            survivor=any(alive(child) for child in children)
        finally:
            remaining=[child for child in children if alive(child)]
            for child in remaining:
                try:os.killpg(child.pid,signal.SIGTERM)
                except ProcessLookupError:pass
            until=time.monotonic()+5
            while any(alive(child) for child in remaining) and time.monotonic()<until:
                for child in children:child.poll()
                time.sleep(.01)
            for child in remaining:
                if alive(child):
                    forced=True
                    try:os.killpg(child.pid,signal.SIGKILL)
                    except ProcessLookupError:pass
            until=time.monotonic()+5
            for child in children:child.wait(timeout=max(.001,until-time.monotonic()))
    codes=[child.returncode for child in children]
    complete=len(children)==len(commands)
    return dict(exit_code=next((code for code in codes if code),0) if complete and not survivor else -1,
        budget_seconds=budget,timed_out=timed,forced_cleanup=forced,seconds=time.monotonic()-started,
        workers=len(commands),worker_exit_codes=codes,original_processes_joined=all(child.poll() is not None for child in children),
        groups_drained=all(not alive(child) for child in children))


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
    # Discover the complete suite in each child; partition by module so no
    # nested/failed discovery test is silently omitted. Core keeps its original
    # 60s bound; HTTP and verified TLS each have a separate30s fixture lane.
    # Python3.13/3.14 exceeded the combined lane in CI after all core tests
    # passed. Splitting modules preserves every test and caller/worker deadline.
    # The two deliberately stalled
    # lifetime tests have their own10s bound and2s actual caller deadlines.
    run_suite='''import sys,unittest,json,asyncio,signal
def stalled_tasks(signum,frame):
 try:tasks=asyncio.all_tasks()
 except RuntimeError:tasks=[]
 stacks=[]
 for task in list(tasks)[:32]:
  current=task.get_coro();frames=[]
  for _ in range(24):
   frame=getattr(current,"cr_frame",None) or getattr(current,"gi_frame",None)
   if frame is not None:
    name=frame.f_code.co_filename.replace(chr(92),"/")
    if "/sdk/" in name:name="sdk/"+name.rsplit("/sdk/",1)[1]
    elif "/asyncio/" in name:name="asyncio/"+name.rsplit("/asyncio/",1)[1]
    else:name=None
    if name is not None:frames.append({"file":name,"line":frame.f_lineno})
   current=getattr(current,"cr_await",None) or getattr(current,"gi_yieldfrom",None)
   if current is None:break
  if frames:stacks.append(frames)
 print("connect-python-stalled-tasks "+json.dumps(stacks,separators=(",",":")),flush=True)
if sys.argv[1]=="http":
 signal.signal(signal.SIGALRM,stalled_tasks)
 signal.setitimer(signal.ITIMER_REAL,25)
suite=unittest.defaultTestLoader.discover("sdk/tests",pattern="test_*.py")
def leaves(suite):
 for item in suite:
  if isinstance(item,unittest.TestSuite):yield from leaves(item)
  else:yield item
def phase(case):
 if case.__class__.__module__=="test_python_https":return "https"
 if case.__class__.__module__ not in ("test_python_http","test_python_https"):return "core"
 if case._testMethodName in ("test_deadline_and_cancel_close_original_socket_and_keep_uncertain_mutation_key","test_deadline_terminates_and_joins_a_worker_blocked_inside_its_resolver"):return "http-lifetime"
 return "http"
worker,workers=int(sys.argv[2]),int(sys.argv[3])
assert workers==(2 if sys.argv[1]=="http" else 1) and 0<=worker<workers
selected=[case for case in leaves(suite) if phase(case)==sys.argv[1]][worker::workers]
class RecordingResult(unittest.TextTestResult):
 def __init__(self,*args,**kwargs):super().__init__(*args,**kwargs);self.passed=[]
 def startTest(self,test):
  super().startTest(test)
  print("connect-python-component-start "+json.dumps({"test":test.id()},separators=(",",":")),flush=True)
 def stopTest(self,test):
  print("connect-python-component-stop "+json.dumps({"test":test.id()},separators=(",",":")),flush=True)
  super().stopTest(test)
 def addSuccess(self,test):super().addSuccess(test);self.passed.append(test.id())
result=unittest.TextTestRunner(verbosity=2,resultclass=RecordingResult).run(unittest.TestSuite(selected))
print("connect-python-component-result "+json.dumps({"phase":sys.argv[1],"worker":worker,"workers":workers,"selected":[t.id() for t in selected],"passed":result.passed,"tests_run":result.testsRun,"failures":len(result.failures),"errors":len(result.errors),"skipped":len(result.skipped)},separators=(",",":")),flush=True)
raise SystemExit(not result.wasSuccessful())
'''
    for label,budget in [('core',60),('http',30),('https',30),('http-lifetime',10)]:
        workers=2 if label=='http' else 1
        logs=[private/f'{label}-{worker}-tests.log' for worker in range(workers)]
        commands=[[sys.executable,'-c',run_suite,label,str(worker),str(workers)] for worker in range(workers)]
        phases[label]=bounded_cohort(commands,environment,logs,budget)
        (private/(label+'-tests.log')).write_bytes(b'\n'.join(path.read_bytes() for path in logs if path.exists()))
        phases[label]['log_sha256']=digest(private/(label+'-tests.log'))
        timed=timed or phases[label]['timed_out'];forced=forced or phases[label]['forced_cleanup']
    (private/'tests.log').write_bytes(b'\n'.join((private/(label+'-tests.log')).read_bytes() for label in phases))
    log=(private/'tests.log').read_text(errors='replace')
    # A phase killed at its original bound has no final unittest record. Keep
    # only source-defined test IDs so its unfinished function remains visible
    # without exporting assertions, payloads or the private worker log.
    lifecycle={}
    for line in log.splitlines():
        match=re.search(r'connect-python-component-(start|stop) (\{"test":"[A-Za-z0-9_.]+"\})$',line)
        if match:
            lifecycle[json.loads(match[2])['test']]=match[1]
    unfinished_functions=sorted(name for name,state in lifecycle.items() if state=='start')
    stalled_task_stacks=[]
    for line in log.splitlines():
        if line.startswith('connect-python-stalled-tasks '):
            stacks=json.loads(line.split(' ',1)[1])
            assert isinstance(stacks,list) and len(stacks)<=32
            for stack in stacks:
                assert isinstance(stack,list) and len(stack)<=24
                for frame in stack:
                    assert set(frame)=={'file','line'}
                    assert re.fullmatch(r'(?:sdk/(?:tests|python/connect_sdk)|asyncio)/[A-Za-z0-9_/]+\.py',frame['file'])
                    assert type(frame['line']) is int and 1<=frame['line']<=100000
            stalled_task_stacks.extend(stacks)
    # Exact unittest result IDs remain independent of interleaved asyncio
    # debug text. Missing/mismatched inventories cannot become a passing scope.
    records=[json.loads(line.split(' ',1)[1]) for line in log.splitlines()
             if line.startswith('connect-python-component-result ')]
    expected={(label,worker) for label in phases for worker in range(2 if label=='http' else 1)}
    inventories=len(records)==len(expected) and {(r['phase'],r['worker']) for r in records}==expected
    for record in records:
        assert set(record)=={'phase','worker','workers','selected','passed','tests_run','failures','errors','skipped'}
        assert record['workers']==(2 if record['phase']=='http' else 1)
        assert len(set(record['selected']))==len(record['selected'])==record['tests_run']
        assert len(set(record['passed']))==len(record['passed'])
        assert set(record['passed']).issubset(record['selected'])
        inventories=inventories and (record['failures']==record['errors']==record['skipped']==0
            and set(record['passed'])==set(record['selected']))
    functions=[name for record in records for name in record['passed']]
    all_selected=[name for record in records for name in record['selected']]
    inventories=inventories and len(all_selected)==len(set(all_selected))
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
    counts_match=Counter(int(n) for n in re.findall(r'^Ran (\d+) tests in ',log,re.M))==Counter([198,10,9,4,2])
    result={'passed':bool(inventories and all(phase['exit_code']==0 for phase in phases.values()) and not timed and not forced and unchanged and not unsafe
                          and len(functions)==223 and len(owned)==17 and len(public)==16 and len(sessions)==8 and len(connections)==12 and len(movements)==12 and len(readers)==20 and len(raw)==19 and len(administrative)==17 and len(http)==21 and len(https)==4 and len(synchronous)==13 and len(media)==12 and len(data)==12
                          and counts_match
                          and (types is None or types['passed'])
                          and re.search(r'^OK$',log,re.M)),
            'scope':'Python owned helper and public canonical controls, consultation/handoff/replacement/return, durable event and invitation readers, finite raw and separate administrative controls through synthetic children, plus actual local HTTP/verified HTTPS servers and owned per-exchange Python workers, with persistent caller-thread synchronous ownership, bounded binary media and full64KiB typed-data channels through a synthetic owned child, and historical regressions. No decoded media, original Rust executable, real production server, installed package, complete Python SDK or G6-063 qualification.',
            'exit_code':next((phase['exit_code'] for phase in phases.values() if phase['exit_code']),0),'timed_out':timed,'forced_cleanup':forced,'phases':phases,
            'unfinished_functions':unfinished_functions,
            'inventories_valid':inventories,'unittest_counts_match':counts_match,
            'stalled_task_stacks':stalled_task_stacks,
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
