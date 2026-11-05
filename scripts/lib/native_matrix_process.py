"""One native workload group, anchored until every original child is joined.

The supervisor runs after any namespace privilege drop. Its own session, not a
sudo monitor's PID, identifies the observed ordinary-user process tree. EOF is
handled by that same owner and cannot leave native helpers behind.
"""
import json
import os
from pathlib import Path
import selectors
import signal
import subprocess
import sys
import time


def publish(path, value):
    temporary = path.with_suffix(".tmp")
    with temporary.open("x") as stream:
        json.dump(value, stream)
        stream.write("\n")
    temporary.replace(path)


def supervise(directory, command):
    # A namespace's sudo monitor may own the inherited group. Make the actual
    # unprivileged supervisor the anchor before spawning any fixture process.
    if os.getpgrp() != os.getpid():
        os.setsid()
    from native_process_owner import live_workers
    directory = Path(directory)
    publish(directory / "owner.json", dict(pid=os.getpid()))
    child = subprocess.Popen(command, stdin=subprocess.DEVNULL)
    selector = selectors.DefaultSelector()
    selector.register(sys.stdin.buffer, selectors.EVENT_READ)
    completed = False
    try:
        while True:
            code = child.poll()
            if code is not None and not completed:
                publish(directory / "completion.json", dict(owner_pid=os.getpid(), exit_code=code))
                completed = True
            if selector.select(.02):
                if os.read(sys.stdin.fileno(), 1):
                    raise ValueError("Only original owner EOF may retire this supervisor")
                if child.poll() is None or live_workers(os.getpid()):
                    os.killpg(os.getpid(), signal.SIGKILL)
                return
    finally:
        selector.close()


class NativeOwner:
    def __init__(self, command, directory, log, *, cwd, env, wrap=None):
        self.directory = Path(directory)
        self.directory.mkdir(mode=0o700)
        self.log = Path(log).open("xb")
        argv = [sys.executable, str(Path(__file__).resolve()), str(self.directory), *map(str, command)]
        self.launch = subprocess.Popen(wrap(argv) if wrap else argv, cwd=cwd, env=env,
            stdin=subprocess.PIPE, stdout=self.log, stderr=subprocess.STDOUT, start_new_session=True)
        self.pid = None
        self.joined = self.workers_joined = self.forced = False
        self.code = None

    def identity(self):
        path = self.directory / "owner.json"
        if not path.exists():
            return None
        if path.is_symlink() or path.stat().st_size > 128:
            raise ValueError("Original process identity bound")
        value = json.loads(path.read_text())
        if set(value) != {"pid"} or type(value["pid"]) is not int or value["pid"] <= 1:
            raise ValueError("Original process identity differs")
        if self.pid is not None and self.pid != value["pid"]:
            raise ValueError("Original process identity changed")
        self.pid = value["pid"]
        return self.pid

    def poll(self):
        path = self.directory / "completion.json"
        if not path.exists():
            return None
        if path.is_symlink() or path.stat().st_size > 128:
            raise ValueError("Original completion bound")
        value = json.loads(path.read_text())
        if (set(value) != {"owner_pid", "exit_code"} or value["owner_pid"] != self.identity()
                or type(value["exit_code"]) is not int):
            raise ValueError("Original completion differs")
        self.code = value["exit_code"]
        return self.code

    def alive(self):
        return self.launch.poll() is None

    def close(self):
        from .native_process_owner import live_workers
        try:
            if self.identity() is not None and self.poll() is not None:
                self.workers_joined = not live_workers(self.pid)
            self.forced = not self.workers_joined
        except (OSError, ValueError, RuntimeError, subprocess.SubprocessError):
            self.forced = True
        finally:
            # The still-live supervisor owns every group decision. No discovered
            # or already-reaped PID is signalled by the qualification driver.
            if not self.launch.stdin.closed:
                try:
                    self.launch.stdin.close()
                except BrokenPipeError:
                    self.forced = True
            try:
                self.launch.wait(timeout=5)
                self.joined = True
            except subprocess.TimeoutExpired:
                self.forced = True
                # The launched monitor remains unreaped, and EOF has already
                # requested the original supervisor's group cleanup.
                self.launch.kill()
                self.launch.wait(timeout=5)
                self.joined = True
            self.log.close()
        return dict(original_supervisor_joined=self.joined, original_workers_joined=self.workers_joined,
            forced=self.forced, exit_code=self.code)


if __name__ == "__main__":
    try:
        supervise(sys.argv[1], sys.argv[2:])
    except BaseException:
        if os.getpgrp() == os.getpid():
            os.killpg(os.getpid(), signal.SIGKILL)
        raise
