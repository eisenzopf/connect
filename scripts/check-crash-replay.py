#!/usr/bin/env python3
"""Abrupt process faults across the SQLite commit / wire acknowledgment boundary."""
import json, os, pathlib, shutil, subprocess, tempfile, tomllib
from lib.local_process import ROOT, BINARY, run, port, Server, request
with tempfile.TemporaryDirectory(prefix='connect-crash-') as temporary:
    work=pathlib.Path(temporary)
    subprocess.run(['cargo','build','--locked','-p','connect-server','--features','test-support','--bin','connect'],cwd=ROOT,check=True)
    fault_binary=work/'connect-faults';shutil.copy2(ROOT/'target/debug/connect',fault_binary)
    if not os.environ.get('CONNECT_BINARY'):
        subprocess.run(['cargo','build','--locked','-p','connect-server','--bin','connect'],cwd=ROOT,check=True)
    normal=work/'connect';shutil.copy2(BINARY,normal)
    assert b'CONNECT_TEST_FAULT_STAGE' not in normal.read_bytes(), 'Normal binary contains test failpoint control'
    assert b'CONNECT_TEST_FAULT_STAGE' in fault_binary.read_bytes()
    for stage in ['before-commit','after-commit','before-ack']:
        state=work/stage
        run('init','--directory',state,'--example','review-loop','--bind',f'127.0.0.1:{port()}',binary=normal)
        for name in ['operator','reviews','agent']:
            path=state/(name+'.toml');path.write_text(path.read_text().replace('timeout_ms = 10000','timeout_ms = 1000'))
        agent=tomllib.loads((state/'agent.toml').read_text())['identity_id']
        with Server(state,binary=normal):
            conversation=request(state,'reviews',{'op':'create','request_id':'create','participants':[agent]},binary=normal)['data']['id']
        operation={'op':'send','request_id':'fault-send','conversation_id':conversation,'body':'one durable effect'}
        env=dict(os.environ,CONNECT_TEST_FAULT_STAGE=stage,CONNECT_TEST_FAULT_REQUEST='fault-send')
        with Server(state,binary=fault_binary,env=env,expected_exit=86) as server:
            uncertain=run('request','--config',state/'reviews.toml',value=operation,success=False,binary=normal)
            server.process.wait(timeout=5);assert server.process.returncode==86
            assert not uncertain.stdout.strip(), 'Faulted mutation was falsely acknowledged'
        with Server(state,binary=normal):
            status=request(state,'reviews',{'op':'request_status','request_id':'fault-send'},binary=normal)['data']
            assert status['state']==('not_found' if stage=='before-commit' else 'accepted')
            original=request(state,'reviews',operation,binary=normal)
            assert request(state,'reviews',operation,binary=normal)==original
            run('request','--config',state/'reviews.toml',value={**operation,'body':'changed'},success=False,binary=normal)
            history=request(state,'agent',{'op':'history','conversation_id':conversation,'after':None,'limit':100},binary=normal)['data']['messages']
            events=request(state,'agent',{'op':'inbox','cursor':None,'limit':100,'wait_ms':0},binary=normal)['data']['events']
            assert len(history)==1 and len([e for e in events if e['kind']=='message'])==1
        print('PASS abrupt '+stage+': uncertain response, receipt recovery, exact replay, input conflict, one message/event')
    print('PASS normal binary excludes failpoint controls; all processes and disposable state cleaned')
