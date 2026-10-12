"""Bounded private qualification phases with owned inherited-group cleanup."""
import json
import os
import pathlib
import signal
import subprocess
import sys
import time

SUPERVISOR = pathlib.Path(__file__).with_name('phase_supervisor.py')


def group_members(group):
    """Bounded observation; never signal a PID discovered through this listing."""
    text = subprocess.check_output(['ps', '-axo', 'pid=,pgid=,stat='], text=True, timeout=3)
    return [int(pid) for pid, pgid, state in (line.split() for line in text.splitlines())
            if int(pgid) == group and int(pid) != group and not state.startswith('Z')]


def run_phase(name, command, timeout, directory, cwd, environment):
    """Return failed evidence on command, deadline, interruption or cleanup error.

    This owns the supervisor group, not deliberately detached sessions. The
    leader is never polled/reaped until all signalling of that group finishes.
    """
    marker = directory / (name + '.exit.json')
    result = {'name': name, 'command': list(map(str, command)), 'timeout_seconds': timeout,
              'log': name + '.log', 'status': 'running'}
    started = time.monotonic()
    process = None
    with (directory / result['log']).open('w') as log:
        try:
            process = subprocess.Popen(
                [sys.executable, str(SUPERVISOR), str(marker), *map(str, command)],
                cwd=cwd, env=environment, stdin=subprocess.PIPE,
                stdout=log, stderr=log, start_new_session=True,
            )
            deadline = started + timeout
            while not marker.exists():
                if time.monotonic() >= deadline:
                    raise TimeoutError(f'Phase exceeded {timeout} seconds')
                time.sleep(.05)
            if marker.stat().st_size > 4096:
                raise ValueError('Oversized supervisor completion marker')
            result.update(json.loads(marker.read_text()))
            # An exited command leaving work behind is a failed qualification,
            # even though the runner stops those processes before returning.
            result['leftover_group_members'] = group_members(process.pid)
            if result['leftover_group_members']:
                result['error'] = 'LeakedProcessGroup'
        except BaseException as error:
            result.update(error=type(error).__name__, detail=str(error))
        finally:
            if process is not None:
                try:
                    # The live (or unreaped) supervisor reserves the group ID.
                    os.killpg(process.pid, signal.SIGKILL)
                    result['group_cleanup'] = 'signalled'
                except ProcessLookupError:
                    result['group_cleanup'] = 'already-exited'
                except OSError as error:
                    result['cleanup_error'] = str(error)
                    process.kill()
                finally:
                    process.stdin.close()
                    try:
                        process.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        result['cleanup_error'] = 'Supervisor could not be reaped'
            result['seconds'] = round(time.monotonic() - started, 3)
    result['status'] = ('passed' if result.get('exit_code') == 0
                        and not result.get('error') and not result.get('cleanup_error') else 'failed')
    return result
