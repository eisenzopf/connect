"""Full declared task ceilings through the public SDK; never preseed store rows."""
import time

from connect_sdk import ConnectError

ACTIVE = 1024
RETAINED = 10000
STEPS = 16


def record(outcome):
    assert outcome['kind'] == 'record', 'Expected a task record'
    return outcome['data']


def denied(call, code='capacity-exceeded'):
    try:
        call()
    except ConnectError as error:
        assert error.code == code, error.code
    else:
        raise AssertionError('Expected explicit capacity denial')


class TaskCapacity:
    """Keep logical inputs stable, including a known-rejected admission retry."""
    def __init__(self, client, definition, report):
        self.client = client
        self.definition = definition
        self.report = report
        self.tasks = []

    def command(self, command):
        self.report.pending = command
        started = time.monotonic()
        try:
            outcome = self.client.task(command)
        except ConnectError as error:
            if error.code not in ('transport-uncertain', 'protocol'):
                self.report.pending = None
            raise
        else:
            self.report.pending = None
            return outcome
        finally:
            self.report.latency(time.monotonic() - started)

    def create(self, index):
        command = {'action': 'create', 'request_id': f'profile-create-{index}',
                   'definition': self.definition}
        return record(self.command(command))

    def cancel(self, task):
        return record(self.command({'action': 'cancel', 'task_id': task['id'],
                                    'request_id': 'profile-cancel-' + task['id'],
                                    'reason': 'operating-profile'}))

    def active(self):
        for index in range(ACTIVE):
            self.tasks.append(self.create(index))
            if (index + 1) % 128 == 0:
                self.report.checkpoint(accepted_tasks=len(self.tasks))
        denied(lambda: self.create(ACTIVE))
        assert self.create(0) == self.tasks[0], 'Matching replay changed at capacity'
        self.report.checkpoint(active_ceiling=ACTIVE, excess_active_denied=True,
                               matching_create_replay=True)

    def claims(self):
        def claim(index):
            task = self.tasks[index]
            return {'action': 'claim', 'task_id': task['id'], 'step_id': 'profile-step',
                    'worker_id': f'profile-worker-{index}',
                    'request_id': f'profile-claim-{index}', 'expected_revision': None}
        leases = []
        for index in range(STEPS):
            outcome = self.command(claim(index))
            assert outcome['kind'] == 'claim' and outcome['data'] is not None
            leases.append(outcome['data'])
        denied(lambda: self.command(claim(STEPS)))
        # Release one lease, then retry exactly the previously rejected input.
        self.command({'action': 'release_claim', 'task_id': self.tasks[0]['id'],
                      'step_id': 'profile-step', 'worker_id': 'profile-worker-0',
                      'request_id': 'profile-release-0',
                      'expected_revision': leases[0]['revision']})
        assert self.command(claim(STEPS))['kind'] == 'claim'
        now = int(time.time())
        assert all(now < lease['expires_at'] <= now + 30 for lease in leases)
        self.report.checkpoint(concurrent_step_ceiling=STEPS,
                               excess_step_denied=True, released_slot_reused=True)

    def retained(self):
        for task in self.tasks[1:]:
            assert self.cancel(task)['state'] == 'cancelled'
        for index in range(ACTIVE, RETAINED):
            task = self.create(index)
            self.tasks.append(task)
            assert self.cancel(task)['state'] == 'cancelled'
            if (index + 1) % 256 == 0:
                self.report.checkpoint(accepted_tasks=len(self.tasks))
        denied(lambda: self.create(RETAINED))
        assert self.create(0) == self.tasks[0], 'Original receipt changed at retained limit'
        # Reserved cancellation capacity must remain usable at the declared bound.
        assert self.cancel(self.tasks[0])['state'] == 'cancelled'
        denied(lambda: self.create(RETAINED))
        self.report.checkpoint(accepted_tasks=RETAINED, retained_ceiling=RETAINED,
                               excess_retained_denied=True,
                               cancellation_at_capacity=True)

    def after_restart(self, client):
        self.client = client
        client.negotiate_task(self.tasks[0]['id'])
        for index in (0, ACTIVE - 1, RETAINED - 1):
            task = record(self.command({'action': 'get', 'task_id': self.tasks[index]['id']}))
            assert task['state'] == 'cancelled'
            assert task['conversation_id'] == self.definition['conversation_id']
        denied(lambda: self.create(RETAINED))
        self.report.checkpoint(restart_preserved_samples=True,
                               retained_limit_survives_restart=True)
