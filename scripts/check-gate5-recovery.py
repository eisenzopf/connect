#!/usr/bin/env python3
"""Public pinned schema1 fixture/schema16 upgrade and current-schema recovery, disposable state only."""
import argparse
import datetime
import hashlib
import json
import os
import pathlib
import platform
import shutil
import sqlite3
import socket
import signal
import subprocess
import sys
import time
import traceback

from lib.local_process import ROOT, BINARY, Server, port, request, run
from lib.profile_metrics import ProfileReport
from lib.recovery_state import baseline, snapshot, preserved



def source_input_hashes():
    # __file__ may retain /var (or another source-root symlink), while ROOT is
    # canonical. Resolve both identity and containment before hashing; an input
    # symlink outside this source tree must still fail closed.
    inputs = [pathlib.Path(__file__), ROOT / 'scripts/lib/recovery_upgrade.py',
              ROOT / 'scripts/lib/recovery_tasks.py', ROOT / 'scripts/lib/recovery_interruption.py',
              ROOT / 'scripts/lib/recovery_restore_interruption.py',
              ROOT / 'scripts/check-recovery.py', ROOT / 'scripts/lib/recovery_state.py',
              ROOT / 'scripts/lib/local_process.py', ROOT / 'scripts/lib/profile_metrics.py']
    inputs = [p.resolve(strict=True) for p in inputs]
    return {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest() for p in inputs}


def setup_recovery(binary, work, schema):
    state = work / 'first-run'
    run('setup', 'init', '--directory', state, '--bind', f'127.0.0.1:{port()}', binary=binary)
    with Server(state, binary=binary):
        import tomllib
        agent = tomllib.loads((state / 'agent.toml').read_text())['identity_id']
        start = request(state, 'human', {'op': 'contact_pairing_start', 'request_id': 'recover-pair',
                        'peer_identity_id': agent, 'expires_at': int(time.time()) + 600}, binary=binary)['data']
        complete = {'op': 'contact_pairing_complete', 'request_id': 'recover-verify',
                    'pairing_id': start['id'], 'challenge': start['challenge']}
        assert request(state, 'agent', complete, binary=binary)['data']['state'] == 'verified_native'
        before = work / 'first-run-backup'
        backup(binary, state, before)
    saved = snapshot(before / 'state.sqlite3')
    recovered = work / 'first-run-restored'
    restore(binary, before, recovered)
    preserved(recovered / 'state.sqlite3', saved, schema)
    for name in ['human.toml', 'setup.json', 'secret-references.toml']:
        assert (recovered / name).is_file()
        assert (recovered / name).stat().st_mode & 0o077 == 0
    run('setup', 'inspect', '--directory', recovered, binary=binary)
    assert (recovered / 'secret-references.toml').read_bytes() == (state / 'secret-references.toml').read_bytes()
    with Server(recovered, binary=binary):
        assert request(recovered, 'agent', complete, binary=binary)['data']['state'] == 'verified_native'
        revoked = {'op': 'contact_pairing_revoke', 'request_id': 'recover-revoke', 'pairing_id': start['id']}
        request(recovered, 'human', revoked, binary=binary)
        result = run('request', '--config', recovered / 'agent.toml', value=complete, success=False, binary=binary)
        assert 'forbidden' in result.stderr or 'pairing' in result.stderr
    tampered = work / 'corrupt-first-run-backup'
    shutil.copytree(before, tampered)
    (tampered / 'setup.json').write_bytes(b'corrupt')
    refused = work / 'corrupt-must-not-restore'
    run('restore', '--backup', tampered, '--directory', refused, binary=binary, success=False)
    assert not refused.exists()
    # Simulate an interrupted restore's partial destination: never adopt or overwrite it.
    partial = work / 'interrupted-restore'
    partial.mkdir(mode=0o700)
    marker = partial / 'state.sqlite3'; marker.write_bytes(b'disposable-partial')
    run('restore', '--backup', before, '--directory', partial, binary=binary, success=False)
    assert marker.read_bytes() == b'disposable-partial'
    return {'setup_files_private': True, 'secret_refs_preserved_without_export': True,
            'pairing_receipt_restored_and_revocation_current': True,
            'corrupt_backup_refused_before_destination': True, 'partial_restore_preserved': True}


def populate_gate4(binary, work, state, saved, schema):
    tasks(binary, work, state, saved, schema)
    with sqlite3.connect(state / 'state.sqlite3') as db:
        task_id = db.execute('SELECT id FROM tasks WHERE conversation_id=?', (saved['cid'],)).fetchone()[0]
    with socket.socket() as listener:
        listener.bind(('127.0.0.1', 0)); address = f'127.0.0.1:{listener.getsockname()[1]}'
    intent = {'profile': 'connect-task-access/1', 'command': {'action': 'issue', 'request_id': 'recovery-native-grant',
              'definition': {'task_id': task_id, 'binding': {'kind': 'native', 'adapter': 'recovery-fixture', 'external_id': 'schema16'},
                             'operations': ['history'], 'expires_at': int(time.time()) + 300}}}
    intent_path = work / 'schema16-grant-intent.json'; intent_path.write_text(json.dumps(intent))
    with Server(state, binary=binary, extra_args=('--http-bind', address)):
        run('task-access', 'issue', '--config', state / 'operator.toml', '--http-address', address,
            '--input', intent_path, '--output', work / 'schema16-grant.json', binary=binary)
    with sqlite3.connect(state / 'state.sqlite3') as db:
        assert db.execute('SELECT COUNT(*) FROM task_grants').fetchone()[0] == 1


def task_continuation(binary, work, state, saved, schema):
    # Continue the waiting task populated by the actual schema16 runtime.
    with sqlite3.connect(state / 'state.sqlite3') as db:
        task_id = db.execute('SELECT id FROM tasks WHERE conversation_id=?', (saved['cid'],)).fetchone()[0]
    before = work / 'schema17-task-backup'
    backup(binary, state, before)
    restored = work / 'schema17-task-restored'
    restore(binary, before, restored)
    preserved(restored / 'state.sqlite3', snapshot(before / 'state.sqlite3'), schema)
    grant_config = work / 'restored-native-grant.toml'
    run('task-access', 'export-native', '--config', restored / 'operator.toml', '--grant-file', work / 'schema16-grant.json',
        '--output', grant_config, binary=binary)
    with Server(restored, binary=binary):
        with NativeClient(grant_config, executable=str(binary)) as grant:
            grant.negotiate_task(task_id)
            history = grant.task({'action': 'invoke', 'task_id': task_id, 'request_id': 'restored-grant-read',
                                  'step_id': 'restored-grant-read', 'reply_to': None,
                                  'operation': {'op': 'history', 'conversation_id': saved['cid'], 'after': None, 'limit': 100}})
            assert history['data']['status'] == 'completed'
            try:
                grant.task({'action': 'invoke', 'task_id': task_id, 'request_id': 'restored-no-live-media',
                            'step_id': 'restored-no-live-media', 'reply_to': None,
                            'operation': {'op': 'session_invite', 'request_id': 'restored-no-live-media',
                                          'conversation_id': saved['cid'], 'participant_id': saved['peer'], 'media': ['audio']}})
            except ConnectError as error:
                assert error.code in ('forbidden', 'unauthorized')
            else:
                raise AssertionError('Restored scoped grant admitted live media')
        with NativeClient(restored / 'operator.toml', executable=str(binary)) as client:
            assert client.negotiate_task(task_id)['state'] == 'waiting'
            question = invoke(task_id, saved['cid'], 'recover-question', 'Approve the blue draft?', True)
            receipt = client.task(question)
            assert receipt['kind'] == 'tool' and receipt['data']['status'] == 'pending'
            client.task({'action': 'cancel', 'task_id': task_id, 'request_id': 'schema17-cancel', 'reason': 'Current recovery authority'})
            try:
                client.task(question)
            except ConnectError as error:
                assert error.code == 'forbidden'
            else:
                raise AssertionError('Historical receipt bypassed current cancellation')
    with Server(restored, binary=binary):
        try:
            with NativeClient(grant_config, executable=str(binary)) as grant:
                grant.negotiate_task(task_id)
                grant.history(saved['cid'])
        except ConnectError as error:
            assert error.code in ('forbidden', 'unauthorized')
        else:
            raise AssertionError('Restored grant bypassed cancelled backing task')
    return {'schema16_waiting_task_restored': True, 'native_grant_rows_and_binding_preserved': True, 'restored_grant_denies_live_media': True, 'original_question_replayed': True, 'current_cancellation_denies_replay': True}


def main():
    global NativeClient, ConnectError, upgrade, backup, restore, tasks, invoke, interruption, restore_interruption
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=pathlib.Path, default=BINARY)
    parser.add_argument('--schema-one-binary', type=pathlib.Path)
    parser.add_argument('--gate4-binary', type=pathlib.Path)
    parser.add_argument('--schema-one-manifest', type=pathlib.Path)
    parser.add_argument('--gate4-manifest', type=pathlib.Path)
    parser.add_argument('--catalog', type=pathlib.Path, default=ROOT / 'dependencies/recovery-baselines.json')
    parser.add_argument('--python-sdk', type=pathlib.Path, default=pathlib.Path(os.environ.get('CONNECT_PYTHON_SDK', ROOT / 'sdk/python')), help='Installed SDK root containing connect_sdk; explicit selection precedes all SDK imports')
    parser.add_argument('--work-root', type=pathlib.Path, default=ROOT / '.local/gate5-recovery')
    parser.add_argument('--output', type=pathlib.Path, help='Write a metadata-only result copy to a fresh file')
    args = parser.parse_args()
    if sys.flags.optimize:
        parser.error('Verification assertions must remain enabled')
    os.umask(0o077)
    if args.output and args.output.exists():
        parser.error('Result output exists; preserve it and select a fresh file')
    catalog_path = args.catalog.resolve()
    catalog = json.loads(catalog_path.read_text())
    sdk_root = args.python_sdk.resolve()
    if not (sdk_root / 'connect_sdk/__init__.py').is_file():
        parser.error('Selected Python SDK root does not contain connect_sdk')
    sys.path.insert(0, str(sdk_root))
    from connect_sdk import NativeClient, ConnectError
    from lib.recovery_upgrade import qualify as upgrade, backup, restore
    from lib.recovery_tasks import qualify as tasks, invoke
    from lib.recovery_interruption import qualify as interruption
    from lib.recovery_restore_interruption import qualify as restore_interruption
    args.schema_one_binary = args.schema_one_binary or ROOT / '.local/recovery-baselines' / catalog['baselines']['1']['revision'] / 'connect'
    args.gate4_binary = args.gate4_binary or ROOT / '.local/recovery-baselines' / catalog['baselines']['16']['revision'] / 'connect'
    work = args.work_root.resolve() / datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ')
    work.mkdir(parents=True)
    binary, one, previous = [p.resolve() for p in [args.binary, args.schema_one_binary, args.gate4_binary]]
    hashes = source_input_hashes()
    sdk_hashes = {str(p.relative_to(sdk_root)): hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted((sdk_root / 'connect_sdk').rglob('*.py'))}
    catalog_hash = hashlib.sha256(catalog_path.read_bytes()).hexdigest()
    report = ProfileReport(work, {'python_sdk_sha256': sdk_hashes, 'catalog_sha256': catalog_hash, 'scope': 'local macOS arm64; isolated fixture state; no providers, personal database, model or deployment',
                                'source_inputs_sha256': hashes, 'budget_seconds': 600})
    def expired(_signal, _frame):
        raise TimeoutError('Recovery qualification exceeded its 600-second budget')
    before = signal.signal(signal.SIGALRM, expired)
    signal.setitimer(signal.ITIMER_REAL, 600)
    error = None
    try:
        assert (platform.system(), platform.machine()) == ('Darwin', 'arm64')
        assert binary.is_file()
        schema_probe = work / 'candidate-schema'
        initialized = json.loads(run('setup', 'init', '--directory', schema_probe,
            '--bind', f'127.0.0.1:{port()}', binary=binary).stdout)
        assert initialized['profile'] == 'connect-first-run/1'
        schema = initialized['schema']
        assert type(schema) is int and schema >= 17
        with sqlite3.connect(f'file:{schema_probe / "state.sqlite3"}?mode=ro', uri=True) as db:
            assert db.execute('PRAGMA user_version').fetchone()[0] == schema
        report.data['candidate_schema'] = schema
        report.data['binary_sha256'] = hashlib.sha256(binary.read_bytes()).hexdigest()
        report.data['baselines'] = [baseline(one, catalog['baselines']['1']['revision'], 1, args.schema_one_manifest), baseline(previous, catalog['baselines']['16']['revision'], 16, args.gate4_manifest)]
        one_manifest = json.loads((args.schema_one_manifest or one.with_name('manifest.json')).read_text())
        assert one_manifest['patch_sha256'] == catalog['baselines']['1']['patch_sha256']
        with report.phase('public-schema1-fixture-upgrade-refusal-rollback'):
            environment = os.environ.copy()
            environment['CONNECT_BINARY'] = str(binary); environment['CONNECT_PREVIOUS_BINARY'] = str(one)
            environment['TMPDIR'] = str(work)
            environment['CONNECT_PYTHON_SDK'] = str(sdk_root)
            if args.schema_one_manifest:
                environment['CONNECT_PREVIOUS_MANIFEST'] = str(args.schema_one_manifest.resolve())
            with (work / 'schema1.log').open('w') as log:
                subprocess.run([sys.executable, str(ROOT / 'scripts/check-recovery.py')], cwd=ROOT,
                               env=environment, stdout=log, stderr=subprocess.STDOUT, timeout=120, check=True)
        with report.phase('public-schema16-populated-upgrade-refusal-ddl-rollback'):
            state, saved, evidence = upgrade(binary, previous, work, schema, historical_schema=16, populate=populate_gate4)
            report.checkpoint(schema16=evidence)
        with report.phase('current-schema-task-continuation-current-authority'):
            report.checkpoint(tasks=task_continuation(binary, work, state, saved, schema))
        with report.phase('current-schema-interrupted-backup-completion-marker'):
            report.checkpoint(interruption=interruption(binary, work, state, schema))
        with report.phase('current-schema-actual-restore-interruption'):
            report.checkpoint(restore_interruption=restore_interruption(binary, work, state, schema))
        with report.phase('current-schema-first-run-pairing-private-reference-recovery'):
            report.checkpoint(setup=setup_recovery(binary, work, schema))
        assert source_input_hashes() == hashes
        assert hashlib.sha256(binary.read_bytes()).hexdigest() == report.data['binary_sha256']
        assert hashlib.sha256(catalog_path.read_bytes()).hexdigest() == catalog_hash
        assert all(hashlib.sha256((sdk_root / p).read_bytes()).hexdigest() == value for p, value in sdk_hashes.items())
    except BaseException as caught:
        error = caught
        (work / 'failure-traceback.log').write_text(traceback.format_exc())
    finally:
        signal.setitimer(signal.ITIMER_REAL, 0); signal.signal(signal.SIGALRM, before)
        report.finish('failed' if error else 'passed', work / 'legacy', error)
        if args.output:
            args.output.parent.mkdir(parents=True, exist_ok=True)
            with args.output.open('x') as stream:
                json.dump(report.data, stream, indent=2); stream.write('\n')
    print(('FAIL' if error else 'PASS') + ' Gate5 recovery: ' + str(work / 'result.json'))
    return 1 if error else 0

if __name__ == '__main__':
    sys.exit(main())
