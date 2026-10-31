"""Install source-bound native candidate archives without trusting archive paths."""
import hashlib
import json
from pathlib import PurePosixPath
import re
import shutil
import tarfile


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def runtime_inventory(runtime):
    """Describe the portable archive modes, independent of private build umask."""
    files = {}
    for path in runtime.rglob('*'):
        if path.is_symlink():
            raise ValueError('Runtime asset contains a symlink')
        if path.is_file():
            name = path.relative_to(runtime).as_posix()
            files[name] = {'sha256': digest(path), 'mode': 0o755 if name == 'bin/connect' else 0o644}
    return files


def install_runtime(archive, proof, destination):
    if not (proof['passed'] and proof['source_inputs_unchanged'] and proof['production_release']
            and not proof['test_support'] and re.fullmatch('[0-9a-f]{40}', proof['source_revision'])
            and digest(archive) == proof['archive_sha256']):
        raise ValueError('Runtime producer/archive proof differs')
    identity = 'connect-runtime-' + proof['version'] + '-' + proof['target'] + '-' + proof['source_revision'][:12]
    destination.mkdir()
    actual = {}; manifest = None; total = 0; seen = set()
    with tarfile.open(archive, 'r:gz') as source:
        for member in source:
            path = PurePosixPath(member.name)
            canonical = path.as_posix()
            if (path.is_absolute() or '..' in path.parts or not path.parts or path.parts[0] != identity
                    or canonical != member.name.rstrip('/') or canonical in seen or len(seen) >= 10000):
                raise ValueError('Invalid or duplicate runtime archive path')
            seen.add(canonical)
            if member.isdir():
                continue
            total += member.size
            if not member.isfile() or total > 1024 * 1024 * 1024:
                raise ValueError('Runtime archive type/size budget exceeded')
            name = '/'.join(path.parts[1:])
            expected_mode = 0o755 if name == 'bin/connect' else 0o644
            if member.mode != expected_mode:
                raise ValueError('Unexpected runtime file permissions')
            target = destination.joinpath(*path.parts[1:])
            target.parent.mkdir(parents=True, exist_ok=True)
            with source.extractfile(member) as content, target.open('xb') as out:
                shutil.copyfileobj(content, out)
            target.chmod(expected_mode)
            if name == 'manifest.json':
                if digest(target) != proof['manifest_sha256']:
                    raise ValueError('Runtime manifest checksum differs')
                manifest = json.loads(target.read_text())
            else:
                actual[name] = {'sha256': digest(target), 'mode': expected_mode}
    if (manifest is None or actual != manifest['files'] or actual != proof['files']
            or manifest['source_revision'] != proof['source_revision'] or manifest['target'] != proof['target']
            or actual['bin/connect']['sha256'] != proof['binary_sha256']):
        raise ValueError('Actual runtime payload differs from source-bound manifest')
    return manifest
