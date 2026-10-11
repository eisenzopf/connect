"""Owned reference-runner processes and read-only application-state observations."""
import json
import signal
import sqlite3
import subprocess
import time

from .local_process import run


class Worker:
    def __init__(self, example, state, directory, task):
        self.example, self.state, self.directory, self.task = example, state, directory, task
        self.database = directory / 'runner.sqlite3'
        self.process, self.log = None, None
        self.attempt = 0

    def start(self, environment):
        assert self.process is None
        self.attempt += 1
        self.log_path = self.directory / f'worker-{self.attempt}.log'
        self.log = self.log_path.open('w')
        try:
            self.process = subprocess.Popen([
                str(self.example), 'task', '--controller-config', str(self.state / 'operator.toml'),
                '--invocation-config', str(self.directory / 'invocation.toml'),
                '--task-id', self.task['id'], '--state', str(self.database),
            ], stdout=self.log, stderr=self.log, env=environment)
        except BaseException:
            self.log.close()
            raise

    def alive(self):
        assert self.process is not None and self.process.poll() is None, self.log_path

    def stop(self, expected=0):
        if self.process is None:
            return
        process = self.process
        try:
            if process.poll() is None:
                process.send_signal(signal.SIGINT)
            try:
                code = process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)
                raise
            assert code == expected, (self.log_path, code, expected)
        finally:
            self.log.close()
            self.process = None

    def inspect(self):
        return json.loads(run('task-inspect', '--state', self.database, binary=self.example).stdout)

    def claim_intent(self):
        # Application-owned state only; never open the Connect server database.
        with sqlite3.connect(self.database.as_uri() + '?mode=ro', uri=True) as database:
            row = database.execute("SELECT command FROM runner_commands WHERE key='question-claim'").fetchone()
            assert row is not None
            return row[0]


def settle(workers, effects):
    deadline = time.monotonic() + 20
    while time.monotonic() < deadline:
        for worker in workers:
            worker.alive()
        counts = [worker.inspect() for worker in workers]
        if all(c['business_effects'] == effects and c['pending_commands'] == 0
               and not c['page_pending'] for c in counts):
            return counts
        assert all(c['business_effects'] <= effects for c in counts), 'Duplicate business action'
        time.sleep(.05)
    raise TimeoutError('Reference runners did not settle within 20 seconds')


def snapshot(workers):
    for worker in workers:
        worker.alive()
    pids = {worker.process.pid for worker in workers}
    rows = subprocess.check_output(['ps', '-o', 'pid=,rss=,comm=', '-p', ','.join(map(str, pids))],
                                   text=True, timeout=3).splitlines()
    observed = []
    for row in rows:
        pid, rss, command = row.strip().split(maxsplit=2)
        assert command == str(workers[0].example), 'Unexpected runner executable'
        observed.append({'pid': int(pid), 'rss_bytes': int(rss) * 1024})
    assert {row['pid'] for row in observed} == pids
    return observed


def stop_all(workers):
    errors = []
    for worker in workers:
        try:
            worker.stop()
        except BaseException as error:
            errors.append(str(error))
    assert not errors, errors
