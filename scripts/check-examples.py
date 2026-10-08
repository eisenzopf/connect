#!/usr/bin/env python3
"""Two independent native example processes, persistent jobs, offline recovery and restart."""
import json, os, pathlib, sqlite3, subprocess, tempfile, tomllib
from lib.local_process import ROOT,BINARY,Server,run,port,request
if not os.environ.get('CONNECT_BINARY'):
    subprocess.run(['cargo','build','--locked','-p','connect-server','-p','connect-review-loop','--bins'],cwd=ROOT,check=True)
from lib.examples import EXAMPLE,inspect,worker,finish,pair
with tempfile.TemporaryDirectory(prefix='connect-examples-') as temporary:
    state=pathlib.Path(temporary)/'state'
    run('init','--directory',state,'--example','review-loop','--bind',f'127.0.0.1:{port()}')
    with Server(state):
        pair(state)
        assert json.loads(run('demo','verify','--config',state/'operator.toml').stdout)['result']=='passed'
        for role in ['reviews','agent']:
            stats=inspect(state/(role+'.work.sqlite3'))
            assert stats['business_effects']==1 and stats['pending_commands']==0 and stats['accepted_commands']==3 and stats['cursor_saved']
        # Agent is offline; app creates a new request through the public SDK CLI.
        agent_id=tomllib.loads((state/'agent.toml').read_text())['identity_id']
        conversation=request(state,'reviews',{'op':'create','request_id':'offline-create','participants':[agent_id]})['data']['id']
        request(state,'reviews',{'op':'send','request_id':'offline-send','conversation_id':conversation,'body':'review_draft:draft-003'})
        finish(worker(state,'agent'))
        stats=inspect(state/'agent.work.sqlite3');assert stats['business_effects']==2 and stats['pending_commands']==0
        # Rewind only this disposable app checkpoint to force replay of events
        # whose effects were already committed. No server DB is inspected.
        with sqlite3.connect(state/'agent.work.sqlite3') as db:db.execute("DELETE FROM metadata WHERE key='cursor'")
        finish(worker(state,'agent'))
        assert inspect(state/'agent.work.sqlite3')['business_effects']==2
        # Each identity independently initiated its own conversation.
        conversations=[]
        for role in ['reviews','agent']:
            with sqlite3.connect(state/(role+'.work.sqlite3')) as db:
                operation,outcome=db.execute("SELECT operation,outcome FROM commands WHERE key='startup-create'").fetchone()
                assert json.loads(operation)['op']=='create'
                conversations.append(json.loads(outcome)['data']['id'])
        assert conversations[0]!=conversations[1]
    with Server(state):
        pair(state)
        assert inspect(state/'reviews.work.sqlite3')['business_effects']==1
        assert inspect(state/'agent.work.sqlite3')['business_effects']==2
        for role,conversation in zip(['reviews','agent'],conversations):
            history=request(state,role,{'op':'history','conversation_id':conversation,'after':None,'limit':100})['data']['messages']
            assert len(history)==2
    print('PASS independent SDK app/agent processes: both initiate/reply; durable outbox IDs; offline request catch-up; business-effect dedup after cursor replay; server/example restart; no server database reads')
