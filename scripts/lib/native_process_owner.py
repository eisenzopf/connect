"""Portable owned process groups: retain the supervisor anchor through cleanup."""
import json
import os
import pathlib
import selectors
import signal
import subprocess
import sys
import tempfile


def publish(path, value):
    temporary = path.with_suffix('.tmp')
    descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(descriptor, 'w') as stream:
        stream.write(json.dumps(value) + '\n')
    temporary.replace(path)


def live_workers(group):
    """Inspect only our anchored group; the observation subprocess is excluded."""
    with subprocess.Popen(['/bin/ps', '-axo', 'pid=,pgid=,stat='],
            stdout=subprocess.PIPE, text=True) as probe:
        output, _ = probe.communicate(timeout=2)
        if probe.returncode:
            raise RuntimeError('Owned group observation failed')
        for line in output.splitlines():
            fields = line.split()
            if len(fields) == 3 and int(fields[1]) == group:
                if int(fields[0]) not in (group, probe.pid) and not fields[2].startswith('Z'):
                    return True
    return False


def supervise(completion, command):
    """One loop alone reaps/signals the child; EOF retires any remaining group."""
    group = os.getpid()
    if os.getpgrp() != group:
        raise RuntimeError('Supervisor must own its newly created process group')
    child = subprocess.Popen(command, stdin=subprocess.DEVNULL)
    result = {'owner_pid': group, 'native_pid': child.pid, 'exit_code': None}
    publish(completion, result)
    selector = selectors.DefaultSelector()
    selector.register(sys.stdin.buffer, selectors.EVENT_READ)
    pending = b''
    try:
        while True:
            code = child.poll()
            if code is not None and result['exit_code'] is None:
                result['exit_code'] = code
                publish(completion, result)
            for key, _ in selector.select(.01):
                chunk = os.read(key.fileobj.fileno(), 4096)
                if not chunk:
                    # The parent's normal wait closes this pipe only after all
                    # group signals. A crash/early EOF must not orphan codecs.
                    try:
                        remaining = child.poll() is None or live_workers(group)
                    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError):
                        remaining = True
                    if remaining:
                        os.killpg(group, signal.SIGKILL)
                    return
                pending += chunk
                if len(pending) > 4096:
                    os.killpg(group, signal.SIGKILL)
                while b'\n' in pending:
                    line, pending = pending.split(b'\n', 1)
                    requested = json.loads(line)['signal']
                    if requested not in (signal.SIGINT, signal.SIGTERM):
                        raise ValueError('Unsupported owned child signal')
                    # No other thread or handler reaps this child. Even if it
                    # exits after this poll, its PID stays reserved until the
                    # next iteration; signal delivery cannot target a reused PID.
                    if child.poll() is None:
                        child.send_signal(requested)
    finally:
        selector.close()


class OwnedChild:
    """Keep an unreaped group leader while exposing the real native exit code.

    The portable supervisor holds the PGID until the owner finishes every group
    signal. Native exit receipts are separate from supervisor exit status.
    Neither Python waitid availability nor an unanchored discovered PID is used.
    """
    def __init__(self, command, **kwargs):
        root = pathlib.Path(__file__).resolve().parents[2] / '.local/gate5/native-process-owners'
        root.mkdir(parents=True, mode=0o700, exist_ok=True)
        self.directory = pathlib.Path(tempfile.mkdtemp(dir=root))
        self.completion = self.directory / 'completion.json'
        self.child = subprocess.Popen([sys.executable, str(pathlib.Path(__file__).resolve()),
            str(self.completion), *command], start_new_session=True,
            stdin=subprocess.PIPE, **kwargs)
        self.pid = self.child.pid  # Anchored process-group ID, not the native PID.
        self.reaped = False
        self.code = None
        self.supervisor_exit_code = None

    def receipt(self):
        if not self.completion.exists():
            return None
        value = json.loads(self.completion.read_text())
        if value['owner_pid'] != self.pid or type(value['native_pid']) is not int:
            raise RuntimeError('Owned completion receipt does not match its supervisor')
        code = value['exit_code']
        if code is not None and type(code) is not int:
            raise RuntimeError('Owned native exit code is malformed')
        return value

    def poll(self):
        if not self.reaped:
            value = self.receipt()
            if value is not None:
                self.code = value['exit_code']
        return self.code

    @property
    def native_pid(self):
        value = self.receipt()
        return value['native_pid'] if value is not None else None

    def send_signal(self, selected):
        if self.reaped or self.poll() is not None:
            raise ProcessLookupError('Owned native child is already terminal')
        if selected not in (signal.SIGINT, signal.SIGTERM):
            raise ValueError('Unsupported owned native signal')
        self.child.stdin.write((json.dumps({'signal': int(selected)}) + '\n').encode())
        self.child.stdin.flush()

    def kill_group(self):
        if self.reaped:
            raise RuntimeError('Cannot signal an owned group after its leader was reaped')
        os.killpg(self.pid, signal.SIGKILL)

    def wait(self, timeout):
        # Cleanup has already finished all group decisions before any wait.
        self.poll()
        if not self.child.stdin.closed:
            self.child.stdin.close()
        self.supervisor_exit_code = self.child.wait(timeout=timeout)
        self.poll()
        self.reaped = True
        return self.code


if __name__ == '__main__':
    try:
        supervise(pathlib.Path(sys.argv[1]), sys.argv[2:])
    except BaseException:
        # A malformed control or observation bug cannot leave owned descendants.
        if os.getpgrp() == os.getpid():
            os.killpg(os.getpid(), signal.SIGKILL)
        raise
