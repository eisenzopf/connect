#!/usr/bin/env python3
"""Selected binary/SDK AV Session controls through CLI and four SDK transports.

Rust-client, direct-HTTP and MCP parity remain separately identified source tests.
This lane launches the selected server, never an embedded source-built fixture.
"""
import argparse
import datetime
import hashlib
import json
import os
import pathlib
import subprocess
import sys
import tomllib
import traceback
from lib.local_process import ROOT, Server, port
from lib.qualification_report import environment
from lib.qualification_process import run_phase


def digest(path):
    with pathlib.Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def inputs(binary, assets, node):
    paths = [binary, node, pathlib.Path(sys.executable).resolve(), pathlib.Path(__file__).resolve(),
        ROOT / 'scripts/lib/local_process.py', ROOT / 'scripts/lib/qualification_report.py',
        ROOT / 'scripts/lib/qualification_process.py', ROOT / 'scripts/lib/phase_supervisor.py',
        ROOT / 'sdk/tests/session_probe.py', ROOT / 'sdk/tests/session_probe.mjs']
    for folder in ('sdk/python/connect_sdk', 'sdk/typescript/dist'):
        paths += [path for path in (assets / folder).rglob('*') if path.is_file()
            and path.suffix not in ('.pyc', '.pyo') and '__pycache__' not in path.parts]
    return {str(path.resolve(strict=True)): digest(path) for path in paths}


def qualify(args, work, report):
    env = dict(environment(), PYTHONPATH=str(args.assets / 'sdk/python'),
        CONNECT_TYPESCRIPT_SDK=str(args.assets / 'sdk/typescript/dist/index.js'),
        CONNECT_BINARY=str(args.binary), CONNECT_TEST_BINARY=str(args.binary))
    state = work / 'state'
    secrets = []
    diagnostics = []

    def command(arguments, value=None, accepted=True):
        result = subprocess.run([str(args.binary), *map(str, arguments)],
            input=None if value is None else json.dumps(value), capture_output=True, text=True,
            timeout=20, env=env)
        assert (result.returncode == 0) is accepted, 'Selected CLI returned unexpected status'
        diagnostics.append(result.stdout + result.stderr)
        assert all(secret not in result.stdout + result.stderr for secret in secrets), 'Credential in public diagnostics'
        return result

    def request(actor, operation, error=None):
        result = command(['request', '--config', configs[actor], '--envelope'], operation, error is None)
        envelope = json.loads(result.stdout)['result']
        if error is not None:
            assert envelope['Err']['code'] == error
            return None
        return envelope['Ok']

    command(['init', '--directory', state, '--example', 'review-loop', '--bind', '127.0.0.1:' + str(port())])
    configs = {'owner': state / 'reviews.toml', 'peer': state / 'agent.toml'}
    values = {actor: tomllib.loads(path.read_text()) for actor, path in configs.items()}
    secrets.extend(row['credential'] for row in values.values())
    secrets.append(tomllib.loads((state / 'operator.toml').read_text())['credential'])
    for actor, peer in (('owner', 'peer'), ('peer', 'owner')):
        selected = state / (actor + '-communications.toml')
        command(['issue', '--config', state / 'operator.toml', '--server-config', state / 'server.toml',
            '--identity-id', values[actor]['identity_id'], '--recipients', values[peer]['identity_id'],
            '--voice', '--output', selected])
        configs[actor] = selected
        values[actor] = tomllib.loads(selected.read_text())
        secrets.append(values[actor]['credential'])
    url = 'http://127.0.0.1:' + str(port())
    records = []
    probe_serial = 0

    def record(actor, operation, error=None, state=None):
        result = request(actor, operation, error)
        row = {'actor': actor, 'operation': operation}
        if error:
            row['error'] = error
        else:
            assert result['kind'] == 'session' and result['data']['media'] == ['audio', 'video']
            assert result['data']['state'] == state
            row['outcome'] = result
        records.append(row)
        return result

    def probe(language, rows, replay_only):
        nonlocal probe_serial
        probe_serial += 1
        executable, filename = (sys.executable, 'session_probe.py') if language == 'python' else (args.node, 'session_probe.mjs')
        payload = {'owner': values['owner']['credential'], 'peer': values['peer']['credential'],
            'records': rows, 'conversation_id': cid, 'participant_id': values['peer']['identity_id'],
            'replay_only': replay_only}
        payload_path = work / ('probe-' + str(probe_serial) + '-input.json')
        payload_path.write_text(json.dumps(payload))
        # The owned supervisor contains the SDK and every native helper even on
        # timeout. A small exec wrapper supplies private stdin without argv secrets.
        wrapper = 'import os,sys; f=open(sys.argv[1],"rb"); os.dup2(f.fileno(),0); os.execv(sys.argv[2],sys.argv[2:])'
        phase = run_phase('session-' + language + '-' + str(probe_serial),
            [sys.executable, '-c', wrapper, str(payload_path), str(executable),
                str(ROOT / 'sdk/tests' / filename), str(configs['owner']), str(configs['peer']), url, str(args.binary)],
            60, work, args.assets, env)
        report.setdefault('sdk_phases', []).append(phase)
        output = (work / phase['log']).read_text()
        diagnostics.append(output)
        assert all(secret not in output for secret in secrets), 'SDK leaked credential'
        assert phase['status'] == 'passed', 'Selected SDK Session probe failed: ' + language
        generated = json.loads(output)
        assert isinstance(generated, list) and len(generated) == (0 if replay_only else 8)
        for row in generated:
            assert request(row['actor'], row['operation']) == row['outcome']
        return generated

    with Server(state, binary=args.binary, env=env, extra_args=['--http-bind', url.removeprefix('http://')], log_path=work / 'server.log'):
        cid = request('owner', {'op': 'create', 'request_id': 'session-cid', 'participants': [values['peer']['identity_id']]})['data']['id']
        other = request('owner', {'op': 'create', 'request_id': 'session-other', 'participants': [values['peer']['identity_id']]})['data']['id']
        for ending, terminal in [('end', 'ended'), ('reject', 'rejected'), ('cancel', 'cancelled')]:
            invite = {'op': 'session_invite', 'request_id': 'session-' + ending + '-invite',
                'conversation_id': cid, 'participant_id': values['peer']['identity_id'], 'media': ['video', 'audio']}
            invited = record('owner', invite, state='invited')['data']
            base = {'conversation_id': cid, 'session_id': invited['id']}
            record('owner', dict(base, op='session_accept', request_id='wrong-' + ending), error='forbidden')
            record('peer', dict(base, op='session_get', conversation_id=other), error='forbidden')
            record('owner', dict(invite, media=['audio']), error='conflict')
            if ending == 'end':
                record('peer', dict(base, op='session_accept', request_id='accept'), state='accepted')
            operation = dict(base, op='session_' + ending, request_id=ending)
            if ending != 'cancel':
                operation['reason'] = 'selected SDK parity'
            record('peer' if ending == 'reject' else 'owner', operation, state=terminal)
            record('owner', dict(base, op='session_get'), state=terminal)
        assert len(records) == 19
        report['cli_seed_records'] = len(records)
        for language in ('python', 'typescript'):
            generated = probe(language, records, False)
            records.extend(generated)
        assert len(records) == 35
        # Recheck all SDK-origin writes through both SDKs, after both have authored.
        for language in ('python', 'typescript'):
            probe(language, records, True)
        report['original_scope_records'] = len(records)
        outside = tomllib.loads((state / 'operator.toml').read_text())['identity_id']
        for actor in ('owner', 'peer'):
            command(['revoke', '--config', state / 'operator.toml', '--server-config', state / 'server.toml', values[actor]['credential_id']])
            narrowed = state / (actor + '-narrowed.toml')
            command(['issue', '--config', state / 'operator.toml', '--server-config', state / 'server.toml',
                '--identity-id', values[actor]['identity_id'], '--recipients', outside, '--voice', '--output', narrowed])
            configs[actor] = narrowed
            values[actor] = tomllib.loads(narrowed.read_text())
            secrets.append(values[actor]['credential'])
        denied = [{'actor': row['actor'], 'operation': row['operation'], 'error': 'forbidden'}
            for row in records if 'outcome' in row]
        assert len(denied) == 26
        for row in denied:
            request(row['actor'], row['operation'], 'forbidden')
        for language in ('python', 'typescript'):
            probe(language, denied, True)
        report['narrowed_scope_denials'] = len(denied)
        report['sdk_origin_records'] = 16
        report['interfaces'] = ['CLI', 'Python native helper', 'Python HTTP', 'TypeScript native helper', 'TypeScript HTTP']
        report['current_authorization_enforced_on_replay'] = True
    diagnostics.append((work / 'server.log').read_text())
    assert all(secret not in output for secret in secrets for output in diagnostics), 'Credential in retained setup/server/SDK diagnostics'
    report['credential_diagnostics_redacted'] = True


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=pathlib.Path)
    parser.add_argument('--assets', required=True, type=pathlib.Path)
    parser.add_argument('--node', required=True, type=pathlib.Path)
    parser.add_argument('--source-revision', required=True)
    parser.add_argument('--work-root', required=True, type=pathlib.Path)
    parser.add_argument('--output', required=True, type=pathlib.Path)
    args = parser.parse_args()
    if sys.flags.optimize:
        parser.error('Verification assertions must remain enabled')
    if args.output.exists():
        parser.error('Preserve existing report; select fresh output')
    for field in ('binary', 'assets', 'node'):
        setattr(args, field, getattr(args, field).resolve(strict=True))
    os.umask(0o077)
    work = args.work_root / datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ')
    work.mkdir(parents=True, mode=0o700)
    bound = inputs(args.binary, args.assets, args.node)
    report = {'status': 'running', 'profile': 'installed-session-controls/1', 'source_revision': args.source_revision,
        'selected_binary': str(args.binary), 'selected_assets': str(args.assets), 'input_sha256': bound,
        'whole_gate_qualified': False, 'scope': 'Selected server/helper and SDKs; CLI plus four SDK transports. Rust, direct-HTTP and MCP parity use the separately scoped source fixture; no AV media acquisition or provider/model calls.'}
    try:
        qualify(args, work, report)
        report['status'] = 'passed'
    except BaseException as error:
        report.update(status='failed', error_class=type(error).__name__, error_frames=[
            {'file': pathlib.Path(frame.filename).name, 'line': frame.lineno, 'function': frame.name}
            for frame in traceback.extract_tb(error.__traceback__)])
    report['input_sha256_after'] = inputs(args.binary, args.assets, args.node)
    report['tested_inputs_stable'] = report['input_sha256_after'] == bound
    if not report['tested_inputs_stable']:
        report['status'] = 'failed'
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open('x') as stream:
        stream.write(json.dumps(report, indent=2) + '\n')
    (work / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'status': report['status'], 'output': str(args.output)}))
    return 0 if report['status'] == 'passed' else 1


if __name__ == '__main__':
    raise SystemExit(main())
