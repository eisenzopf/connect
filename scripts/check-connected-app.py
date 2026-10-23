#!/usr/bin/env python3
"""Qualify selected normal connected-app binaries through delivered public CLIs.

The private app ledger may be inspected; Connect's database is never opened.
Crash injection remains a separate test-support lane, not a packaged capability.
"""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import signal
import sqlite3
import subprocess
import time
import tomllib
import uuid

from lib.local_process import ROOT, Server, port
from lib.qualification_report import environment


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def require(value, label):
    if not value:
        raise AssertionError(label)


class Journey:
    def __init__(self, args):
        self.env = environment()
        self.server = args.server_binary.resolve(strict=True)
        self.example = args.example_binary.resolve(strict=True)
        self.directory = args.work_root.resolve() / ('attempt-' + uuid.uuid4().hex)
        self.directory.mkdir(parents=True, mode=0o700)
        self.state = self.directory / 'state'
        self.ledger = self.state / 'business.sqlite3'
        self.processes = []
        self.handles = []
        self.serial = 0
        self.expected_exits = {}
        self.worker_logs = {}
        self.inputs = [Path(__file__).resolve(), ROOT / 'scripts/lib/local_process.py', ROOT / 'scripts/lib/qualification_report.py',
                       Path(os.sys.executable).resolve(), self.server, self.example]
        self.report = {'status': 'running', 'source_revision': args.source_revision, 'profile': 'connected-app-public-cli/1',
            'scope': 'selected normal binaries; local loopback; no provider or model operations',
            'selected_binaries': {'connect': str(self.server), 'connect-example': str(self.example)},
            'binary_sha256': {'connect': digest(self.server), 'connect-example': digest(self.example)},
            'driver_sha256': digest(__file__),
            'input_sha256': {str(path): digest(path) for path in self.inputs}, 'checks': [], 'cleanup': [],
            'private_attempt_directory': str(self.directory),
            'not_run': ['test-support lost-ack crash injection', 'actual-model inference', 'whole-gate qualification']}

    def check(self, label, value):
        require(value, label)
        self.report['checks'].append(label)

    def command(self, binary, *arguments, value=None, success=True):
        result = subprocess.run([str(binary), *map(str, arguments)],
            input=None if value is None else json.dumps(value), text=True,
            capture_output=True, timeout=15, env=self.env)
        self.serial += 1
        (self.directory / f'command-{self.serial}.json').write_text(json.dumps({
            'arguments': list(map(str, arguments)), 'returncode': result.returncode,
            'stdout': result.stdout, 'stderr': result.stderr}))
        require((result.returncode == 0) == success, 'public CLI exit status')
        return (json.loads(result.stdout) if result.stdout.lstrip().startswith('{') else result.stdout) if success else result

    def request(self, role, operation, success=True):
        extra = ['--task-address', 'connect:reviews@local'] if operation['op'] == 'task' else []
        if operation['op'] == 'task':
            operation = dict(operation, request={'task_profile': 'connect-task/1', 'command': operation['request']})
        return self.command(self.server, 'request', '--config', self.state / (role + '.toml'),
                            *extra, value=operation, success=success)

    def inspect(self):
        return self.command(self.example, 'app-inspect', '--state', self.ledger)

    def wait(self, callback, label):
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            result = callback()
            if result:
                return result
            time.sleep(.05)
        raise AssertionError(label)

    def spawn(self):
        log = (self.directory / ('worker-' + uuid.uuid4().hex + '.log')).open('x')
        self.handles.append(log)
        env = self.env
        process = subprocess.Popen([str(self.example), 'connected-app', '--config',
            str(self.state / 'reviews.toml'), '--policy', str(self.state / 'connected-app.json'),
            '--state', str(self.ledger)], stdout=log, stderr=log, env=env)
        self.processes.append(process)
        self.expected_exits[process.pid] = 0
        self.worker_logs[process.pid] = Path(log.name)
        return process

    def stop(self, process):
        if process.poll() is None:
            process.send_signal(signal.SIGINT)
            try:
                process.wait(timeout=8)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)
                raise AssertionError('worker graceful cleanup deadline')
        require(process.returncode == self.expected_exits[process.pid], 'owned worker expected exit')
        row = {'pid': process.pid, 'exit_code': process.returncode}
        if row not in self.report['cleanup']:
            self.report['cleanup'].append(row)

    def finish(self):
        # An early worker failure must never skip cleanup of later owned workers.
        for process in self.processes:
            try:
                self.stop(process)
            except BaseException as error:
                self.report.update(status='failed')
                self.report.setdefault('cleanup_errors', []).append({
                    'pid': process.pid, 'error_class': type(error).__name__})
        for handle in self.handles:
            handle.close()
        self.report['remaining_owned_processes'] = [p.pid for p in self.processes if p.poll() is None]
        self.report['input_sha256_after'] = {str(path): digest(path) for path in self.inputs if path.is_file()}
        self.report['tested_inputs_stable'] = self.report['input_sha256_after'] == self.report['input_sha256']
        if self.report['remaining_owned_processes'] or not self.report['tested_inputs_stable']:
            self.report.update(status='failed')

    def create(self, role, request_id):
        return self.request(role, {'op': 'create', 'request_id': request_id,
            'participants': [self.app_id]})['data']['id']

    def send(self, role, cid, request_id, body):
        return self.request(role, {'op': 'send', 'request_id': request_id,
            'conversation_id': cid, "message": {'to': 'all', 'content_type': 'text/plain', 'body': json.dumps(body), 'body_encoding': 'utf8', 'label': 'connect-messages', 'attachments': [], 'in_reply_to_msg': None, 'msg_id': None}})

    def history(self, role, cid):
        return self.request(role, {'op': 'history', 'conversation_id': cid,
            'after': None, 'limit': 100})['data']['messages']

    def snapshot(self):
        # Only the developer-owned application ledger, never server state.
        with sqlite3.connect(f'file:{self.ledger}?mode=ro', uri=True) as db:
            return {'commands': db.execute('SELECT key,operation FROM app_commands ORDER BY sequence').fetchall(),
                'business': db.execute('SELECT key,task_id,conversation_id,source_event,input_digest FROM app_business ORDER BY key').fetchall(),
                'cursor': db.execute("SELECT value FROM app_metadata WHERE key='cursor'").fetchone()}

    def ready(self, reviews, tickets):
        if not self.ledger.is_file():
            return None
        stats = self.inspect()
        return stats if (stats['reviews'], stats['tickets'], stats['pending_commands'], stats['queued_events']) == (reviews, tickets, 0, 0) else None

    def run(self):
        self.check('normal example excludes test-support crash controls',
                   b'CONNECT_CONNECTED_APP_FAULT' not in self.example.read_bytes())
        self.command(self.server, 'init', '--directory', self.state, '--example', 'connected-app',
                     '--bind', f'127.0.0.1:{port()}')
        self.app_id = tomllib.loads((self.state / 'reviews.toml').read_text())['identity_id']
        with Server(self.state, binary=self.server, env=self.env, log_path=self.directory / 'server-first.log'):
            worker = self.spawn()
            def question():
                page = self.request('agent', {'op': 'inbox', 'cursor': None, 'limit': 100, 'wait_ms': 0})
                return next((event['message'] for event in page['data']['events']
                    if event.get('message') and '"review_request"' in event['message']['body']), None)
            original = self.wait(question, 'outgoing question delivered')
            body = json.loads(original['body'])
            cid = original['conversation_id']
            artifact = body['artifact']
            data = bytearray()
            while True:
                chunk = self.request('agent', {'op': 'artifact_get', 'conversation_id': cid,
                    'artifact_id': artifact['id'], 'offset': len(data), 'limit': 1024})['data']
                require(chunk['artifact'] == artifact and chunk['offset'] == len(data), 'artifact attribution')
                data.extend(base64.b64decode(chunk['data'], validate=True))
                if chunk['eof']:
                    break
            self.check('outgoing artifact bytes, owner, conversation and draft digest',
                len(data) == artifact['size'] and hashlib.sha256(data).hexdigest() == artifact['sha256']
                and json.loads(data) == body['draft'] and artifact['owner_id'] == self.app_id
                and artifact['conversation_id'] == cid)
            self.report['artifact'] = {'id': artifact['id'], 'sha256': artifact['sha256'], 'size': len(data)}
            competitor = self.spawn()
            competitor.wait(timeout=8)
            self.expected_exits[competitor.pid] = competitor.returncode
            self.check('concurrent worker cannot own original ledger', competitor.returncode != 0 and worker.poll() is None
                and 'Another connected app owns this state' in self.worker_logs[competitor.pid].read_text())
            decision = {'action': 'review_decision', 'draft_id': body['draft']['id'],
                        'decision': 'approved', 'note': 'Reviewed actual delivered draft bytes.'}
            self.send('agent', cid, 'ambiguous-decision', decision)
            incoming = self.create('agent', 'independent-intake')
            self.send('agent', incoming, 'wrong-conversation-decision', decision)
            outside = self.create('outside', 'unauthorized-contact')
            self.send('outside', outside, 'unauthorized-decision', decision)
            def rejected():
                stats = self.inspect()
                return stats if stats['unassigned_events'] == 2 and stats['denied_events'] == 1 else None
            stats = self.wait(rejected, 'unbound and unauthorized inputs observed')
            self.check('unbound, wrong-conversation and unauthorized decisions cannot select a record', stats['reviews'] == stats['tickets'] == 0)
            submission = {'action': 'submit_draft', 'request_id': 'business-intake-1',
                'draft': {'id': 'incoming-1', 'title': 'Independent request', 'body': 'Retain this independent draft.'}}
            self.send('agent', incoming, 'incoming-wire-1', submission)
            reply = {'op': 'task', 'request': {'action': 'reply', 'request_id': 'review-approval-1',
                'conversation_id': cid, 'reply_to': original['id'], 'body': json.dumps(decision)}}
            accepted = self.request('agent', reply)
            settled = self.wait(lambda: self.ready(1, 1), 'two business results settled')
            self.check('both directions retain their original conversation mapping',
                {row['conversation_id'] for row in settled['results']} == {cid, incoming}
                and {row['status'] for row in settled['results']} == {'approved', 'received'})
            self.stop(worker)
            before = self.snapshot()
            self.check('app checkpoint and original business effect identities persisted', bool(before['cursor']) and len(before['business']) == 2)
        with Server(self.state, binary=self.server, env=self.env, log_path=self.directory / 'server-restart.log'):
            persisted = self.snapshot()
            worker = self.spawn()
            self.wait(lambda: self.ready(1, 1), 'restart settled')
            resumed = self.snapshot()
            self.check('original mutation and business IDs survive server and worker restart',
                before == persisted and before['commands'] == resumed['commands']
                and before['business'] == resumed['business'] and bool(resumed['cursor']))
            replay = self.request('agent', reply)
            self.check('same reviewer mutation replays exact accepted receipt', replay == accepted)
            self.send('agent', incoming, 'incoming-wire-duplicate', submission)
            changed = dict(decision, decision='rejected', note='Late reordered second decision')
            late = {'op': 'task', 'request': dict(reply['request'], request_id='late-second-decision', body=json.dumps(changed))}
            self.request('agent', late)
            conflict = dict(submission, draft=dict(submission['draft'], body='Changed under original business ID'))
            self.send('agent', incoming, 'incoming-wire-conflict', conflict)
            def reconciled():
                stats = self.ready(1, 1)
                return stats if stats and stats['conflict_events'] >= 1 else None
            stats = self.wait(reconciled, 'all replay inputs reconciled')
            self.check('late decision and semantic duplicates preserve original effects', before['business'] == self.snapshot()['business'])
            for conversation, action in [(cid, 'review_recorded'), (incoming, 'draft_received')]:
                messages = self.history('agent', conversation)
                self.check('one public response for ' + action,
                    sum(json.loads(message['body']).get('action') == action for message in messages) == 1)
            self.check('unauthorized contact receives no business response',
                all(message['sender'] != self.app_id for message in self.history('outside', outside)))
            self.report['results'] = stats['results']
            self.stop(worker)
        self.check('selected normal binary hashes unchanged', self.report['binary_sha256'] == {
            'connect': digest(self.server), 'connect-example': digest(self.example)})
        self.report['input_sha256_after'] = {str(path): digest(path) for path in self.inputs}
        self.report['tested_inputs_stable'] = self.report['input_sha256_after'] == self.report['input_sha256']
        self.check('selected driver and tools unchanged', self.report['tested_inputs_stable'])
        self.report['status'] = 'passed'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source-revision', required=True)
    parser.add_argument('--server-binary', type=Path, required=True)
    parser.add_argument('--example-binary', type=Path, required=True)
    parser.add_argument('--work-root', type=Path, default=ROOT / '.local/gate5/connected-app')
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    os.umask(0o077)
    require(not args.output.exists(), 'preserve earlier output; select a fresh report path')
    journey = Journey(args)
    try:
        journey.run()
    except BaseException as error:
        journey.report.update(status='failed', error_class=type(error).__name__,
                              failure=str(error) if isinstance(error, AssertionError) else 'private operation failed')
    finally:
        journey.finish()
        result = json.dumps(journey.report, indent=2) + '\n'
        (journey.directory / 'result.json').write_text(result)
        args.output.parent.mkdir(parents=True, exist_ok=True)
        with args.output.open('x') as output:
            output.write(result)
    print(json.dumps({'status': journey.report['status'], 'output': str(args.output)}))
    return 0 if journey.report['status'] == 'passed' else 1


if __name__ == '__main__':
    raise SystemExit(main())
