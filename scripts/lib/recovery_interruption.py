"""Interrupt a real snapshot before its completion marker, using disposable I/O."""
import os
import shutil
import signal
import subprocess
import time

from .local_process import run
from .recovery_state import preserved, snapshot
from .recovery_upgrade import backup, restore


def qualify(binary, work, original, schema):
    source_rows = snapshot(original / 'state.sqlite3')
    source = work / 'interrupted-source'
    shutil.copytree(original, source)
    # A FIFO holds this disposable backup between SQLite completion and key
    # copying. No server uses this copy, and no real installation is modified.
    key = source / 'private-key.der'
    assert key.is_file()
    key.unlink()
    os.mkfifo(key, 0o600)
    output = work / 'interrupted-backup'
    with (work / 'interrupted-backup.log').open('w') as log:
        process = subprocess.Popen([
            str(binary), 'backup', '--config', str(source / 'operator.toml'),
            '--server-config', str(source / 'server.toml'), '--output', str(output),
        ], stdout=log, stderr=log)
        try:
            deadline = time.monotonic() + 10
            while not (output / 'certificate.der').exists():
                assert process.poll() is None, 'Backup exited before the I/O barrier'
                assert time.monotonic() < deadline, 'Backup did not reach the I/O barrier'
                time.sleep(.02)
            assert (output / 'state.sqlite3').is_file()
            assert not (output / 'manifest.json').exists()
            assert process.poll() is None
            process.send_signal(signal.SIGKILL)
            assert process.wait(timeout=5) == -signal.SIGKILL
        finally:
            if process.poll() is None:
                process.kill()
                process.wait(timeout=5)
    rejected = work / 'incomplete-must-not-restore'
    result = run('restore', '--backup', output, '--directory', rejected,
                 binary=binary, success=False)
    assert not rejected.exists(), 'Incomplete snapshot became a restored installation'
    assert not (output / 'manifest.json').exists()
    # An interrupted destination is retained for inspection, never overwritten.
    run('backup', '--config', original / 'operator.toml', '--server-config',
        original / 'server.toml', '--output', output, binary=binary, success=False)
    retry = work / 'complete-after-interruption'
    backup(binary, original, retry)
    recovered = work / 'restored-after-interruption'
    restore(binary, retry, recovered)
    preserved(recovered / 'state.sqlite3', source_rows, schema)
    preserved(original / 'state.sqlite3', source_rows, schema)
    return {'killed_after_database_before_manifest': True,
            'incomplete_snapshot_refused': result.returncode != 0,
            'interrupted_destination_preserved': True,
            'fresh_backup_restores_original_rows': True}
