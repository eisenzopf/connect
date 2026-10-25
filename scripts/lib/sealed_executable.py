"""Seal executable bytes once; hard-link only independently copied sealed files."""
import hashlib
import os
from pathlib import Path
import shutil
import stat
import uuid

ROOT = Path(__file__).resolve().parents[2]


def digest(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def seal_executable(source, destination, expected_sha256, *, pool=None):
    source, destination = Path(source), Path(destination)
    pool = Path(pool) if pool is not None else ROOT / '.local/gate-6/sealed-executables'
    private = (ROOT / '.local').resolve()
    if not destination.resolve().is_relative_to(private) or not pool.resolve().is_relative_to(private):
        raise ValueError('Keep sealed executables under ignored .local/')
    if source.is_symlink() or not source.is_file() or digest(source) != expected_sha256:
        raise ValueError('Source executable changed before sealing')
    mode = stat.S_IMODE(source.stat().st_mode)
    if not mode & 0o111:
        raise ValueError('Source is not executable')
    pool.mkdir(parents=True, exist_ok=True)
    stored = pool / f'{expected_sha256}-{mode:04o}'
    if not stored.exists():
        temporary = pool / f'.candidate-{uuid.uuid4().hex}'
        try:
            # Never link a Cargo output: a later build may rewrite it in place.
            shutil.copy2(source, temporary)
            if digest(temporary) != expected_sha256 or stat.S_IMODE(temporary.stat().st_mode) != mode:
                raise ValueError('Executable changed during independent copy')
            try:
                os.link(temporary, stored)
            except FileExistsError:
                pass  # A concurrent sealer must have stored identical bytes.
        finally:
            temporary.unlink(missing_ok=True)
    if stored.is_symlink() or not stored.is_file() or digest(stored) != expected_sha256:
        raise ValueError('Sealed executable pool is corrupt')
    if stat.S_IMODE(stored.stat().st_mode) != mode or digest(source) != expected_sha256:
        raise ValueError('Executable mode or source changed before admission')
    if source.stat().st_dev == stored.stat().st_dev and source.stat().st_ino == stored.stat().st_ino:
        raise ValueError('A sealed executable must be independent of the build output')
    os.link(stored, destination)  # Exclusive creation; never replace evidence.
    if digest(destination) != expected_sha256:
        raise ValueError('Sealed executable changed before execution')
