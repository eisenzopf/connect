#!/usr/bin/env python3
"""Prove schema-6 upgrade/rollback with actual executables and disposable state.

Build the baseline with scripts/build-recovery-baseline.py. No ignored fixture
code, provider accounts or live installation is required.
"""
import argparse
import hashlib
import json
import os
import pathlib
import re
import shutil
import sqlite3
import subprocess
import tempfile
import tomllib

from lib.local_process import ROOT, BINARY, Server, config_port, port, request, run

BASELINE = '8bf5bd31a519a5fa84ef8f80e4da3b58374c6de5'
TABLES = ('conversations', 'participants', 'messages', 'requests', 'journal')


def snapshot(database):
    """Retain original columns so an additive migration cannot hide lost rows."""
    with sqlite3.connect(database) as connection:
        columns = {table: [row[1] for row in connection.execute(f'PRAGMA table_info({table})')]
                   for table in TABLES}
        rows = {table: connection.execute(f'SELECT * FROM {table}').fetchall() for table in TABLES}
    return columns, rows


def assert_preserved(database, columns, rows, schema):
    with sqlite3.connect(database) as connection:
        assert connection.execute('PRAGMA user_version').fetchone()[0] == schema
        for table in TABLES:
            selected = ','.join('"' + column.replace('"', '""') + '"' for column in columns[table])
            actual = connection.execute(f'SELECT {selected} FROM {table}').fetchall()
            assert actual == rows[table], f'Original rows changed in {table}'


def qualify(previous, *, binary=BINARY):
    manifest = json.loads(previous.with_name('manifest.json').read_text())
    assert manifest['source_revision'] == BASELINE, 'Unexpected baseline source'
    assert hashlib.sha256(previous.read_bytes()).hexdigest() == manifest['binary_sha256']
    match = re.search(r'pub const SCHEMA_VERSION: u32 = (\d+);',
                      (ROOT / 'crates/connect-store/src/lib.rs').read_text())
    assert match, 'Current schema constant missing'
    current_schema = int(match.group(1))
    with tempfile.TemporaryDirectory(prefix='connect-schema-six-') as directory:
        work = pathlib.Path(directory)
        legacy = work / 'legacy'
        run('init', '--directory', legacy, '--example', 'review-loop',
            '--bind', f'127.0.0.1:{port()}', binary=previous)
        with Server(legacy, work, binary=previous):
            agent = tomllib.loads((legacy / 'agent.toml').read_text())['identity_id']
            created = request(legacy, 'reviews', {'op': 'create', 'request_id': 'create',
                                                  'participants': [agent]}, binary=previous)
            command = {'op': 'send', 'request_id': 'stable-send',
                       'conversation_id': created['data']['id'], "message": {'to': 'all', 'content_type': 'text/plain', 'body': 'Schema-six retained fact', 'body_encoding': 'utf8', 'label': 'connect-messages', 'attachments': [], 'in_reply_to_msg': None, 'msg_id': None}}
            receipt = request(legacy, 'reviews', command, binary=previous)
        columns, rows = snapshot(legacy / 'state.sqlite3')
        assert_preserved(legacy / 'state.sqlite3', columns, rows, 6)
        backup = work / 'backup'
        run('backup', '--config', legacy / 'operator.toml', '--server-config',
            legacy / 'server.toml', '--output', backup, binary=binary)
        run('migrate', '--config', legacy / 'operator.toml', '--server-config',
            legacy / 'server.toml', '--backup', backup, binary=binary)
        with Server(legacy, work, binary=binary):
            assert request(legacy, 'reviews', command, binary=binary) == receipt
        assert_preserved(legacy / 'state.sqlite3', columns, rows, current_schema)
        rejected = run('serve', '--config', legacy / 'server.toml', success=False, binary=previous)
        assert 'unsupported-schema' in rejected.stderr and 'ready transport' not in rejected.stdout
        rollback = work / 'rollback'
        run('restore', '--backup', backup, '--directory', rollback, '--bind', f'127.0.0.1:{port()}', binary=binary)
        with Server(rollback, work, binary=previous):
            assert request(rollback, 'reviews', command, binary=previous) == receipt
        assert_preserved(rollback / 'state.sqlite3', columns, rows, 6)
        failed = work / 'failed-upgrade'
        shutil.copytree(rollback, failed)
        config_port(failed, port())
        with sqlite3.connect(failed / 'state.sqlite3') as connection:
            connection.execute('CREATE TABLE sessions(broken TEXT)')
        run('migrate', '--config', failed / 'operator.toml', '--server-config',
            failed / 'server.toml', '--backup', backup, success=False, binary=binary)
        assert_preserved(failed / 'state.sqlite3', columns, rows, 6)
        recovered = work / 'recovered'
        run('restore', '--backup', backup, '--directory', recovered, '--bind', f'127.0.0.1:{port()}', binary=binary)
        with Server(recovered, work, binary=previous):
            assert request(recovered, 'reviews', command, binary=previous) == receipt
        assert_preserved(recovered / 'state.sqlite3', columns, rows, 6)
    print('PASS schema-6 provenance, history/receipts, upgrade, refusal, rollback, migration failure and restore')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--previous-binary', required=True, type=pathlib.Path,
                        help='Schema-6 executable; adjacent manifest.json must pin its source and digest')
    args = parser.parse_args()
    if not os.environ.get('CONNECT_BINARY'):
        subprocess.run(['bash', 'scripts/prepare-dependencies.sh'], cwd=ROOT, check=True)
        subprocess.run(['cargo', 'build', '--locked', '-p', 'connect-server', '--bin', 'connect'],
                       cwd=ROOT, check=True)
    qualify(args.previous_binary.resolve())


if __name__ == '__main__':
    main()
