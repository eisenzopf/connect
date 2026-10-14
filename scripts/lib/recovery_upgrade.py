"""Actual pre-Gate-4 binary upgrade and matching-snapshot rollback."""
import base64
import hashlib
import shutil
import sqlite3
import tomllib

from connect_sdk import NativeClient
from .local_process import Server, config_port, port, run
from .recovery_state import preserved, sms_fixture, snapshot

DRAFT = b'Connect recovery draft: retain context across channels.\n' * 512


def backup(binary, state, output):
    run('backup', '--config', state / 'operator.toml', '--server-config',
        state / 'server.toml', '--output', output, binary=binary)


def restore(binary, source, destination):
    run('restore', '--backup', source, '--directory', destination,
        '--bind', f'127.0.0.1:{port()}', binary=binary)


def upload(client, cid):
    artifact = client.artifact_begin('recovery-draft', cid, 'draft.txt', 'text/plain',
                                     len(DRAFT), hashlib.sha256(DRAFT).hexdigest())
    client.artifact_append('recovery-draft-chunk', cid, artifact['id'], 0,
                           base64.b64encode(DRAFT).decode())
    client.artifact_finish('recovery-draft-finish', cid, artifact['id'])
    return artifact['id']


def verify(client, saved):
    assert client.send('recovery-fact', saved['cid'], 'Keep the approved blue draft.') == saved['receipt']
    chunk = client.artifact_get(saved['cid'], saved['artifact'])
    assert base64.b64decode(chunk['data']) == DRAFT
    assert chunk['artifact']['sha256'] == hashlib.sha256(DRAFT).hexdigest()


def qualify(binary, previous, work, schema, historical_schema=10, populate=None):
    state = work / 'legacy'
    run('init', '--directory', state, '--example', 'review-loop',
        '--bind', f'127.0.0.1:{port()}', binary=previous)
    peer = tomllib.loads((state / 'agent.toml').read_text())['identity_id']
    owner = tomllib.loads((state / 'operator.toml').read_text())['identity_id']
    # This disposable participant must be able to answer the operator's task.
    run('issue', '--config', state / 'operator.toml', '--server-config', state / 'server.toml',
        '--identity-id', peer, '--recipients', owner, '--output', state / 'respondent.toml',
        binary=previous)
    (state / 'respondent.toml').replace(state / 'agent.toml')
    with Server(state, binary=previous, log_path=work / 'legacy.log'):
        with NativeClient(state / 'operator.toml', executable=str(previous)) as client:
            cid = client.create('recovery-conversation', [peer])['id']
            saved = {'cid': cid, 'peer': peer, 'artifact': upload(client, cid),
                     'receipt': client.send('recovery-fact', cid, 'Keep the approved blue draft.')}
    sms_fixture(state / 'state.sqlite3', cid)
    if populate:
        populate(previous, work, state, saved, historical_schema)
    original = snapshot(state / 'state.sqlite3')
    preserved(state / 'state.sqlite3', original, historical_schema)
    before = work / 'before-upgrade'
    backup(binary, state, before)
    run('migrate', '--config', state / 'operator.toml', '--server-config',
        state / 'server.toml', '--backup', before, binary=binary)
    preserved(state / 'state.sqlite3', original, schema)
    with Server(state, binary=binary, log_path=work / 'upgraded.log'):
        with NativeClient(state / 'operator.toml', executable=str(binary)) as client:
            verify(client, saved)
    preserved(state / 'state.sqlite3', original, schema)
    rejected = run('serve', '--config', state / 'server.toml', binary=previous, success=False)
    assert 'unsupported-schema' in rejected.stderr and 'ready transport' not in rejected.stdout
    rollback = work / 'rollback'
    restore(binary, before, rollback)
    with Server(rollback, binary=previous, log_path=work / 'rollback.log'):
        with NativeClient(rollback / 'operator.toml', executable=str(previous)) as client:
            verify(client, saved)
    preserved(rollback / 'state.sqlite3', original, historical_schema)
    failed = work / 'failed-upgrade'
    shutil.copytree(rollback, failed)
    config_port(failed, port())
    with sqlite3.connect(failed / 'state.sqlite3') as db:
        table = 'contact_pairings' if historical_schema == 16 else 'tasks'
        db.execute(f'CREATE TABLE {table}(broken TEXT)')
    failed_input = snapshot(failed / 'state.sqlite3')
    # Back up the actual failing input, so this reaches DDL rather than only
    # failing the stale-backup guard. The original good rollback stays intact.
    failing_backup = work / 'failed-input-backup'
    backup(binary, failed, failing_backup)
    result = run('migrate', '--config', failed / 'operator.toml', '--server-config',
                 failed / 'server.toml', '--backup', failing_backup, binary=binary, success=False)
    # The public CLI redacts SQLite details as storage-unavailable.
    assert 'storage-unavailable' in result.stderr, result.stderr
    preserved(failed / 'state.sqlite3', failed_input, historical_schema)
    assert snapshot(failed / 'state.sqlite3') == failed_input
    recovered = work / 'recovered'
    restore(binary, before, recovered)
    with Server(recovered, binary=previous, log_path=work / 'recovered.log'):
        with NativeClient(recovered / 'operator.toml', executable=str(previous)) as client:
            verify(client, saved)
    preserved(recovered / 'state.sqlite3', original, historical_schema)
    return state, saved, {'original_tables': len(original),
                          'original_rows': sum(len(t['rows']) for t in original.values()),
                          'artifact_bytes': len(DRAFT), 'migration_ddl_rollback': True,
                          'old_binary_refuses_new_schema': True,
                          'matching_snapshot_rollback': True, 'synthetic_sms_rows_preserved': True}
