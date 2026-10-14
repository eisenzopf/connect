"""Interrupt an actual restore into an owned fresh destination, retaining partial files."""
import hashlib
import shutil
import signal
import sqlite3
import subprocess
import time

from .local_process import run
from .recovery_state import snapshot, preserved
from .recovery_upgrade import backup, restore


def qualify(binary, work, original, schema):
    source = work / 'restore-interruption-source'
    shutil.copytree(original, source)
    # Non-secret padding in this disposable database makes the actual online
    # SQLite copy observable. No production hook, owner state or deadline changes.
    with sqlite3.connect(source / 'state.sqlite3') as db:
        db.execute('CREATE TABLE restore_interruption_fixture(payload BLOB NOT NULL)')
        db.execute('INSERT INTO restore_interruption_fixture VALUES(zeroblob(?))', (32 * 1024 * 1024,))
    before = snapshot(source / 'state.sqlite3')
    saved = work / 'restore-interruption-backup'
    backup(binary, source, saved)
    partial = work / 'restore-interruption-partial'
    with (work / 'restore-interruption.log').open('w') as log:
        process = subprocess.Popen([str(binary), 'restore', '--backup', str(saved), '--directory', str(partial)], stdout=log, stderr=log)
        try:
            deadline = time.monotonic() + 10
            while not (partial / 'state.sqlite3').exists():
                assert process.poll() is None, 'Restore exited before its database copy'
                assert time.monotonic() < deadline, 'Restore did not reach its database copy'
                time.sleep(.001)
            assert process.poll() is None
            assert not (partial / 'server.toml').exists()
            process.send_signal(signal.SIGKILL)
            assert process.wait(timeout=5) == -signal.SIGKILL
        finally:
            if process.poll() is None:
                process.kill(); process.wait(timeout=5)
    assert partial.exists() and not (partial / 'server.toml').exists()
    partial_files = {p.name: hashlib.sha256(p.read_bytes()).hexdigest()
                     for p in partial.iterdir() if p.is_file()}
    run('restore', '--backup', saved, '--directory', partial, binary=binary, success=False)
    assert {p.name: hashlib.sha256(p.read_bytes()).hexdigest()
            for p in partial.iterdir() if p.is_file()} == partial_files
    complete = work / 'restore-interruption-complete'
    restore(binary, saved, complete)
    preserved(complete / 'state.sqlite3', before, schema)
    preserved(source / 'state.sqlite3', before, schema)
    return {'actual_restore_killed_during_database_copy': True,
            'partial_destination_not_adopted_or_overwritten': True,
            'matching_backup_restores_to_new_destination': True,
            'disposable_padding_bytes': 32 * 1024 * 1024}
