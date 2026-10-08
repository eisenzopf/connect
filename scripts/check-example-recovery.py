#!/usr/bin/env python3
"""Abrupt app-side effect/outbox failures with real native communication."""
import os,pathlib,shutil,subprocess,tempfile
from lib.local_process import ROOT,BINARY,Server,run,port,request
from lib.examples import EXAMPLE,inspect,worker,finish,pair
with tempfile.TemporaryDirectory(prefix='connect-example-recovery-') as temporary:
    work=pathlib.Path(temporary)
    subprocess.run(['cargo','build','--locked','-p','connect-review-loop','--features','test-support','--bin','connect-example'],cwd=ROOT,check=True)
    fault_binary=work/'connect-example-faults';shutil.copy2(ROOT/'target/debug/connect-example',fault_binary)
    if not os.environ.get('CONNECT_BINARY'):
        subprocess.run(['cargo','build','--locked','-p','connect-review-loop','-p','connect-server','--bins'],cwd=ROOT,check=True)
    assert b'CONNECT_EXAMPLE_FAULT_STAGE' not in EXAMPLE.read_bytes()
    for stage in ['before-effect','after-effect','after-server-ack']:
        state=work/stage
        run('init','--directory',state,'--example','review-loop','--bind',f'127.0.0.1:{port()}')
        with Server(state):
            app=worker(state,'reviews',1200,env=dict(os.environ,CONNECT_EXAMPLE_FAULT_STAGE=stage),binary=fault_binary)
            agent=worker(state,'agent',1200)
            try:finish(app,86);finish(agent)
            finally:
                for process in [app,agent]:
                    if process.poll() is None:process.kill();process.communicate()
            before=inspect(state/'reviews.work.sqlite3')
            assert before['business_effects']==(1 if stage=='after-effect' else 0)
            pair(state)
            for role in ['reviews','agent']:
                stats=inspect(state/(role+'.work.sqlite3'))
                assert stats['business_effects']==1 and stats['pending_commands']==0 and stats['accepted_commands']==3
            pair(state)
            assert inspect(state/'reviews.work.sqlite3')['business_effects']==1
        print('PASS example abrupt '+stage+': saved logical IDs replay; local action once; reply outbox resumes; repeat restart stable')
    print('PASS normal example binary excludes fault controls; task-owned processes cleaned')
