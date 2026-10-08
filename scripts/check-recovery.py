#!/usr/bin/env python3
"""Qualify local recovery using independent CLI/server processes and disposable state."""
import hashlib, json, os, pathlib, shutil, signal, socket, sqlite3, subprocess, tempfile, time, tomllib
from lib.local_process import ROOT, BINARY, run, port, Server, request, config_port
if not os.environ.get('CONNECT_BINARY'):
    subprocess.run(['cargo','build','--locked','-p','connect-server','--bin','connect'],cwd=ROOT,check=True)

with tempfile.TemporaryDirectory(prefix='connect-recovery-') as temporary:
    work=pathlib.Path(temporary);state=work/'state';address=f'127.0.0.1:{port()}'
    run('init','--directory',state,'--example','review-loop','--bind',address)
    agent=tomllib.loads((state/'agent.toml').read_text())['identity_id']
    with Server(state,work):
        second=run('serve','--config',state/'server.toml',success=False)
        assert 'already in use' in second.stderr and 'ready transport' not in second.stdout
        create=request(state,'reviews',{'op':'create','request_id':'create','participants':[agent]})
        conversation=create['data']['id']
        send={'op':'send','request_id':'send','conversation_id':conversation,'body':'backup fixture'}
        accepted=request(state,'reviews',send)
        page=request(state,'agent',{'op':'inbox','cursor':None,'limit':100,'wait_ms':0})
        run('backup','--config',state/'agent.toml','--server-config',state/'server.toml','--output',work/'denied',success=False)
        assert not (work/'denied').exists()
        app_id=tomllib.loads((state/'reviews.toml').read_text())['identity_id']
        issued=state/'issued.toml'
        run('issue','--config',state/'agent.toml','--server-config',state/'server.toml','--identity-id',app_id,'--recipients',agent,'--output',issued,success=False)
        assert not issued.exists()
        run('issue','--config',state/'operator.toml','--server-config',state/'server.toml','--identity-id',app_id,'--recipients',agent,'--output',issued)
        assert 'installation_id' in run('health','--config',issued).stdout
        credential_id=tomllib.loads(issued.read_text())['credential_id']
        run('revoke','--config',state/'operator.toml','--server-config',state/'server.toml',credential_id)
        run('health','--config',issued,success=False)
        backup=work/'backup'
        run('backup','--config',state/'operator.toml','--server-config',state/'server.toml','--output',backup)
        run('backup','--config',state/'operator.toml','--server-config',state/'server.toml','--output',backup,success=False)
        later=request(state,'reviews',{'op':'send','request_id':'later','conversation_id':conversation,'body':'after backup'})
        newer=request(state,'agent',{'op':'inbox','cursor':page['data']['cursor'],'limit':100,'wait_ms':0})['data']['cursor']
    restored=work/'restored'
    run('restore','--backup',backup,'--directory',restored,'--bind',f'127.0.0.1:{port()}')
    run('restore','--backup',backup,'--directory',restored,success=False)
    with Server(restored,work):
        assert request(restored,'reviews',send)==accepted
        assert request(restored,'agent',{'op':'inbox','cursor':None,'limit':100,'wait_ms':0})==page
        run('request','--config',restored/'agent.toml',value={'op':'inbox','cursor':newer,'limit':100,'wait_ms':0},success=False)
        health=json.loads(run('health','--config',restored/'operator.toml').stdout)
        assert health['result']['data']['schema']==6
    tampered=work/'tampered';shutil.copytree(backup,tampered)
    (tampered/'certificate.der').write_bytes(b'altered')
    run('restore','--backup',tampered,'--directory',work/'never-created',success=False)
    assert not (work/'never-created').exists()
    # Actual preserved prior milestone binary, not a mock reader.
    previous=pathlib.Path(os.environ.get('CONNECT_PREVIOUS_BINARY',ROOT/'.local/milestones/m1/connect'))
    assert previous.is_file(), 'Supply the archived schema-1 milestone binary (see docs/LOCAL_RELEASE.md)'
    prior_manifest=json.loads(previous.with_name('manifest.json').read_text())
    assert hashlib.sha256(previous.read_bytes()).hexdigest()==prior_manifest['binary_sha256']
    legacy=work/'legacy'
    run('init','--directory',legacy,'--example','review-loop','--bind',f'127.0.0.1:{port()}',binary=previous)
    old_backup=work/'old-backup'
    run('backup','--config',legacy/'operator.toml','--server-config',legacy/'server.toml','--output',old_backup)
    run('migrate','--config',legacy/'operator.toml','--server-config',legacy/'server.toml','--backup',old_backup)
    with Server(legacy,work):assert 'installation_id' in run('health','--config',legacy/'operator.toml').stdout
    rejected=run('serve','--config',legacy/'server.toml',success=False,binary=previous)
    assert 'unsupported-schema' in rejected.stderr and 'ready transport' not in rejected.stdout
    rollback=work/'rollback'
    run('restore','--backup',old_backup,'--directory',rollback,'--bind',f'127.0.0.1:{port()}')
    with Server(rollback,work,binary=previous):assert 'installation_id' in run('health','--config',rollback/'operator.toml',binary=previous).stdout
    failed=work/'failed-upgrade';shutil.copytree(rollback,failed);config_port(failed,port())
    with sqlite3.connect(failed/'state.sqlite3') as db:db.execute('CREATE TABLE conversations(broken TEXT)')
    run('migrate','--config',failed/'operator.toml','--server-config',failed/'server.toml','--backup',old_backup,success=False)
    with sqlite3.connect(failed/'state.sqlite3') as db:assert db.execute('PRAGMA user_version').fetchone()[0]==1
    recovered=work/'failed-upgrade-recovered'
    run('restore','--backup',old_backup,'--directory',recovered,'--bind',f'127.0.0.1:{port()}')
    with Server(recovered,work,binary=previous):assert 'installation_id' in run('health','--config',recovered/'operator.toml',binary=previous).stdout
    # Original source installation still has post-backup accepted work.
    with Server(state,work):assert request(state,'reviews',{'op':'request_status','request_id':'later'})['data']['state']=='accepted'
    print('PASS: single-instance lock; online consistent backup; authority; restore/cursor/replay; checksum tamper; actual schema-1 binary upgrade/refusal/rollback; failed-upgrade recovery; original state preserved')
