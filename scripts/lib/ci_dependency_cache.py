"""Fresh pinned CI dependencies; reuse only identical cached source timestamps."""
from pathlib import Path
import hashlib
import json
import os
import platform
import runpy
import uuid

ROOT = Path(__file__).resolve().parents[2]


def compatible(directory, lock):
    names = [name for name, source in lock['sources'].items()
        if source.get('kind') != 'crate' and any(p.get('target', 'rvoip') == name for p in lock['patches'])]
    if not names:
        return False
    for name in names:
        marker = directory / name / '.git/connect-prepared.json'
        if (marker.is_symlink() or not marker.is_file() or not marker.resolve().is_relative_to(directory.resolve())
                or marker.stat().st_size > 2 * 1024 * 1024):
            return False
        try:
            if json.loads(marker.read_text()) != lock:
                return False
        except (ValueError, OSError):
            return False
    return True


def restore_identical_times(current, previous):
    restored = different = 0
    for path in current.rglob('*'):
        relative = path.relative_to(current)
        if '.git' in relative.parts or path.is_symlink() or not path.is_file():
            continue
        info = path.stat()
        old = previous / relative
        if (info.st_nlink != 1 or old.is_symlink() or not old.is_file()
                or not old.resolve().is_relative_to(previous.resolve())):
            continue
        before = old.stat()
        if (info.st_size != before.st_size or info.st_mode & 0o777 != before.st_mode & 0o777
                or not 0 < before.st_mtime_ns < 2**63):
            different += 1
            continue
        with path.open('rb') as a, old.open('rb') as b:
            if hashlib.file_digest(a, 'sha256').digest() != hashlib.file_digest(b, 'sha256').digest():
                different += 1
                continue
        os.utime(path, ns=(info.st_atime_ns, before.st_mtime_ns), follow_symlinks=False)
        restored += 1
    return dict(restored_identical_files=restored, different_files=different)


def prepare(root, run):
    directory = root / '.deps'
    if directory.is_symlink():
        raise ValueError('CI dependency cache must not replace a selected dependency symlink')
    lock = json.loads((root / 'dependencies/source-lock.json').read_text())
    report = dict(scope='CI build-cache preparation only; no runtime qualification.',
        source_lock_sha256=hashlib.sha256((root / 'dependencies/source-lock.json').read_bytes()).hexdigest(),
        retained_incompatible_cache=False, restored_identical_files=0, different_files=0)
    command = ['bash', 'scripts/prepare-dependencies.sh']
    previous = None
    if directory.exists() and not compatible(directory, lock):
        parent = root / '.local/gate6-linux-ci/retained-deps'
        parent.mkdir(parents=True, exist_ok=True)
        previous = parent / uuid.uuid4().hex
        directory.rename(previous)
        command += ['--cache', str(previous)]
        report['retained_incompatible_cache'] = True
    # The ordinary preparer verifies pinned commits/archives and every patch.
    # Old bytes are never installed by the timestamp restoration below.
    run(command)
    if previous is not None:
        report.update(restore_identical_times(directory, previous))
    return report


def main():
    if (os.environ.get('GITHUB_ACTIONS') != 'true' or platform.system() != 'Linux'
            or Path(os.environ.get('GITHUB_WORKSPACE', '/missing')).resolve() != ROOT):
        raise ValueError('This cache preparation is restricted to its owned GitHub Linux workspace')
    output = ROOT / '.local/gate6-linux-ci/evidence'
    output.mkdir(parents=True, exist_ok=True)
    bounded = runpy.run_path(str(ROOT / 'scripts/rehearse-gate6-turn-relay.py'))['bounded_child']
    def run(command):
        step = bounded(command, dict(os.environ), output.parent / 'dependency-prepare-private.log', 600)
        if step['exit_code'] or step['timed_out'] or step['forced_kill']:
            raise RuntimeError('Pinned CI dependency preparation failed; inspect its private log')
    result = prepare(ROOT, run)
    (output / 'dependency-cache.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result))


if __name__ == '__main__':
    main()
