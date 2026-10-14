#!/usr/bin/env python3
"""Fresh public CLI setup against an explicitly selected local executable.

Only disposable state is touched. This proves the installed first-run surface;
native Keychain storage and actual-model inference remain separate checks.
"""
import argparse
import datetime
import hashlib
import json
import os
import pathlib
import socket
import sqlite3
import subprocess
import sys
import time
import tomllib
import traceback

from lib.local_process import ROOT, Server
from lib.qualification_report import environment


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


class PublicCli:
    def __init__(self, binary, env):
        self.binary, self.env = binary, env

    def run(self, *arguments, stdin=None, accepted=True, timeout=15, env=None):
        result = subprocess.run([str(self.binary), *map(str, arguments)],
            input=stdin, text=True, capture_output=True, env=env or self.env,
            timeout=timeout)
        if bool(result.returncode == 0) != accepted:
            # Do not copy bearer-bearing output or challenges into diagnostics.
            raise RuntimeError('Unexpected public CLI status: ' + str(arguments[0]))
        return result

    def data(self, *arguments, stdin=None):
        return json.loads(self.run(*arguments, stdin=stdin).stdout)


def first_run(cli, work):
    state = work / 'fresh'
    initial = cli.data('setup', 'init', '--directory', state, '--bind', '127.0.0.1:0')
    assert initial['profile'] == 'connect-first-run/1'
    schema = initial['schema']
    assert type(schema) is int and schema >= 17
    with sqlite3.connect(f'file:{state / "state.sqlite3"}?mode=ro', uri=True) as db:
        assert db.execute('PRAGMA user_version').fetchone()[0] == schema
    assert initial['provider_mode'] == 'none' and not initial['tasks_required']
    assert int(initial['bind'].rsplit(':', 1)[1]) != 0
    configs = {role: tomllib.loads((state / (role + '.toml')).read_text())
        for role in ('human', 'agent')}
    original = {role: (state / (role + '.toml')).read_bytes() for role in configs}
    assert cli.data('setup', 'init', '--directory', state, '--bind', '127.0.0.1:0') == initial
    assert all((state / (role + '.toml')).read_bytes() == data for role, data in original.items())
    assert cli.data('setup', 'inspect', '--directory', state) == initial
    demo = json.loads(
        cli.run('setup', 'demo', '--directory', state, '--bind', '127.0.0.1:0', timeout=45).stdout)
    assert demo['profile'] == 'connect-first-run-proof/1'
    assert all(demo[key] is True for key in ('messages_each_direction', 'files_each_direction',
        'restart_history', 'native_pairing'))
    assert demo['provider_requests'] == 0 and not demo['model_used'] and not demo['tasks_required']
    with Server(state, binary=cli.binary, env=cli.env, log_path=work / 'server.log'):
        # Independent observation after the executable's own demo/restart.
        for role in configs:
            history = cli.data('request', '--config', state / (role + '.toml'), stdin=json.dumps(
                {'op': 'history', 'conversation_id': demo['conversation_id'], 'after': None, 'limit': 100}))
            bodies = {message['body'] for message in history['data']['messages']}
            assert {'Hello from the local human', 'Reply from the local agent'} <= bodies
        pair = cli.data('setup', 'pair-start', '--config', state / 'human.toml',
            '--peer-identity-id', configs['agent']['identity_id'], '--request-id', 'installed-pair',
            '--expires-at', int(time.time()) + 300)['data']
        completed = cli.data('setup', 'pair-complete', '--config', state / 'agent.toml',
            '--pairing-id', pair['id'], '--request-id', 'installed-complete', stdin=pair['challenge'])
        assert completed['data']['state'] == 'verified_native'
        contact = cli.data('setup', 'contact', '--config', state / 'human.toml',
            '--peer-identity-id', configs['agent']['identity_id'])['data']
        assert contact['ownership'] == 'verified_native' and contact['sms_consent'] is False
        cli.run('setup', 'pair-revoke', '--config', state / 'human.toml',
            '--pairing-id', pair['id'], '--request-id', 'installed-revoke')
        cli.run('setup', 'pair-complete', '--config', state / 'agent.toml',
            '--pairing-id', pair['id'], '--request-id', 'installed-complete', stdin=pair['challenge'], accepted=False)
    for path in state.rglob('*'):
        assert path.stat().st_mode & 0o077 == 0
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as listener:
        listener.bind(('127.0.0.1', int(initial['bind'].rsplit(':', 1)[1])))
    return {'candidate_schema': schema, 'ordinary_identities': True, 'messages_and_files_each_direction': True,
        'history_independently_observed_after_restart': True, 'pairing_revocation_enforced_on_replay': True,
        'pairing_never_supplies_sms_consent': True, 'private_modes': True, 'port_released': True}


def failures(cli, work):
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as listener:
        listener.bind(('127.0.0.1', 0))
        target = work / 'occupied'
        cli.run('setup', 'init', '--directory', target,
            '--bind', '127.0.0.1:' + str(listener.getsockname()[1]), accepted=False)
        assert not target.exists()
    partial = work / 'partial'; partial.mkdir(mode=0o700)
    marker = partial / 'state.sqlite3'; marker.write_bytes(b'preserve-partial')
    cli.run('setup', 'init', '--directory', partial, '--bind', '127.0.0.1:0', accepted=False)
    assert marker.read_bytes() == b'preserve-partial'
    interrupted = work / '.interrupted.connect-initializing'; interrupted.mkdir(mode=0o700)
    cli.run('setup', 'init', '--directory', work / 'interrupted', '--bind', '127.0.0.1:0', accepted=False)
    assert interrupted.is_dir() and not (work / 'interrupted').exists()
    state = work / 'fresh'; protected = state / 'agent.toml'
    protected.chmod(0o644)
    try:
        cli.run('setup', 'inspect', '--directory', state, accepted=False)
    finally:
        protected.chmod(0o600)
    # An explicit empty reference map suppresses even ambient fixture credentials.
    # No network request can be made: the Telnyx credential is absent as well.
    env = dict(cli.env, CONNECT_SECRET_REFERENCES=str(state / 'secret-references.toml'),
        VAPI_PRIVATE_KEY='ambient-test-value-must-not-be-used')
    for command, flag in [('discover', '--output'), ('check', '--state')]:
        destination = work / ('must-not-exist-' + command + '.json')
        result = cli.run('providers', command, flag, destination, accepted=False, env=env)
        assert 'Required server secret is not configured' in result.stderr
        assert not destination.exists() and 'ambient-test-value-must-not-be-used' not in result.stderr
    destination = work / 'must-not-exist-setup-catalog.json'
    result = cli.run('setup', 'provider-discover', '--output', destination, accepted=False, env=env)
    assert 'Required server secret is not configured' in result.stderr
    assert not destination.exists()
    destination = work / 'must-not-exist-native-voice.json'
    result = cli.run('providers', 'voice-native', '--voice-state', work / 'no-input-needed.json',
        '--state', destination, accepted=False, env=env)
    assert 'Required server secret is not configured' in result.stderr
    assert not destination.exists()
    refs = state / 'secret-references.toml'
    refs.write_text('version = 1\n[references.VAPI_PRIVATE_KEY]\nstorage = "environment"\nvariable = "CONNECT_FIXTURE_SECRET"\n')
    result = cli.run('setup', 'secret-set', '--references', refs, '--name', 'VAPI_PRIVATE_KEY',
        stdin='test-value-never-written', accepted=False)
    assert 'Environment references cannot be rotated by Connect' in result.stderr
    assert 'test-value-never-written' not in result.stdout + result.stderr + refs.read_text()
    return {'occupied_port_preserved': True, 'partial_and_interrupted_state_preserved': True,
        'permissive_modes_denied': True, 'explicit_references_prevent_ambient_fallback': True,
        'environment_reference_rotation_denied': True}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=pathlib.Path)
    parser.add_argument('--source-revision',
        help='Required for installed proof; source default records this checkout revision')
    parser.add_argument('--work-root', required=True, type=pathlib.Path)
    parser.add_argument('--output', required=True, type=pathlib.Path)
    args = parser.parse_args()
    if sys.flags.optimize:
        parser.error('Verification assertions must remain enabled')
    if args.output.exists() or args.output.is_symlink():
        parser.error('Preserve the existing report and select a fresh output file')
    if args.source_revision is None:
        args.source_revision = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
    binary = args.binary.resolve(strict=True)
    os.umask(0o077)
    work = args.work_root / datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ')
    work.mkdir(parents=True, mode=0o700)
    inputs = [binary, pathlib.Path(__file__).resolve(), ROOT / 'scripts/lib/local_process.py',
        ROOT / 'scripts/lib/qualification_report.py']
    hashes = {str(path): digest(path) for path in inputs}
    report = {'status': 'running', 'source_revision': args.source_revision, 'binary_sha256': hashes[str(binary)],
        'input_sha256': hashes, 'scope': 'disposable local public CLI, no provider/device/model operations',
        'whole_gate_qualified': False, 'native_keychain_qualified': False}
    try:
        cli = PublicCli(binary, dict(environment(), CONNECT_BINARY=str(binary), CONNECT_TEST_BINARY=str(binary)))
        report['first_run'] = first_run(cli, work)
        report['candidate_schema'] = report['first_run']['candidate_schema']
        report['failures'] = failures(cli, work)
        report['tested_inputs_stable'] = all(digest(path) == hashes[str(path)] for path in inputs)
        assert report['tested_inputs_stable']
        report['status'] = 'passed'
    except BaseException as error:
        report.update(status='failed', error_class=type(error).__name__, error_frames=[
            {'file': pathlib.Path(frame.filename).name, 'line': frame.lineno, 'function': frame.name}
            for frame in traceback.extract_tb(error.__traceback__)])
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open('x') as stream:
        stream.write(json.dumps(report, indent=2) + '\n')
    print(report['status'].upper() + ' public setup: ' + str(args.output))
    return 0 if report['status'] == 'passed' else 1


if __name__ == '__main__':
    raise SystemExit(main())
