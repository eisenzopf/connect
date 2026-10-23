"""Reproduce one pinned crate archive plus recorded patches without registry edits."""
import hashlib
import json
import os
import pathlib
import shutil
import subprocess
import tarfile
import tempfile
import urllib.request

MAX_ARCHIVE_BYTES = 16 * 1024 * 1024
MAX_EXTRACTED_BYTES = 64 * 1024 * 1024
MAX_FILES = 4096
MARKER = '.connect-prepared.json'
PATCH_APPLICATION = 'isolated-git-apply-v1'


def digest(path):
    value = hashlib.sha256()
    with path.open('rb') as stream:
        for chunk in iter(lambda: stream.read(65536), b''): value.update(chunk)
    return value.hexdigest()


def file_manifest(directory):
    result = {}
    for path in sorted(directory.rglob('*')):
        if path.is_symlink(): raise ValueError('Prepared crate contains a symbolic link')
        relative = path.relative_to(directory).as_posix()
        if path.is_file() and relative != MARKER:
            result[relative] = {'sha256': digest(path), 'mode': path.stat().st_mode & 0o777}
        elif not path.is_dir() and relative != MARKER:
            raise ValueError('Prepared crate contains a special file')
    return result


def extract_archive(archive, destination, prefix):
    # Stream headers so even a compressed archive containing huge numbers of
    # empty entries cannot allocate an unbounded getmembers() list first.
    with tarfile.open(archive, 'r|gz') as bundle:
        seen = set()
        count = size = 0
        for member in bundle:
            if member.size < 0:
                raise ValueError('Crate archive contains a negative member size')
            count += 1
            size += member.size
            if count > MAX_FILES or size > MAX_EXTRACTED_BYTES:
                raise ValueError('Crate archive exceeds extraction bounds')
            path = pathlib.PurePosixPath(member.name)
            if path.is_absolute() or '..' in path.parts or not path.parts or path.parts[0] != prefix:
                raise ValueError('Crate archive path escapes its declared root')
            relative = pathlib.Path(*path.parts[1:])
            if relative.as_posix() in seen: raise ValueError('Duplicate crate archive member')
            seen.add(relative.as_posix())
            if not member.isfile() and not member.isdir():
                raise ValueError('Crate archive links and special files are forbidden')
            target = destination / relative
            if member.isdir(): target.mkdir(parents=True, exist_ok=True); continue
            if relative.as_posix() in ('.', MARKER): raise ValueError('Reserved crate archive member')
            target.parent.mkdir(parents=True, exist_ok=True)
            with bundle.extractfile(member) as source, target.open('xb') as output:
                shutil.copyfileobj(source, output, 65536)
            target.chmod(0o755 if member.mode & 0o111 else 0o644)


def verified_archive(directory, name, source, cache):
    filename = f'{name}-{source["version"]}.crate'
    archive_dir = directory / '.archives'
    archive_dir.mkdir(exist_ok=True)
    archive = (cache / filename) if cache and (cache / filename).is_file() else archive_dir / filename
    if not archive.exists():
        if not source['url'].startswith('https://static.crates.io/crates/'):
            raise ValueError('Crate archive requires the pinned HTTPS registry source')
        handle, temporary = tempfile.mkstemp(prefix=filename + '.', dir=archive_dir)
        temporary = pathlib.Path(temporary)
        try:
            size = 0
            with os.fdopen(handle, 'wb') as output, urllib.request.urlopen(source['url'], timeout=30) as response:
                while chunk := response.read(65536):
                    size += len(chunk)
                    if size > MAX_ARCHIVE_BYTES: raise ValueError('Crate archive exceeds download bound')
                    output.write(chunk)
            if digest(temporary) != source['sha256']: raise ValueError('Crate archive digest mismatch')
            temporary.replace(archive)
        finally:
            temporary.unlink(missing_ok=True)
    if archive.is_symlink() or archive.stat().st_size > MAX_ARCHIVE_BYTES or digest(archive) != source['sha256']:
        raise ValueError('Crate archive identity mismatch')
    return archive


def prepare_archive(root, directory, name, source, patches, cache=None):
    if source.get('kind') != 'crate': raise ValueError('Unsupported archive source kind')
    for notice in source.get('license_files', []):
        if digest(root / notice['path']) != notice['sha256']:
            raise ValueError('Prepared crate license digest mismatch')
    selected = [patch for patch in patches if patch.get('target', 'rvoip') == name]
    for patch in selected:
        if digest(root / patch['path']) != patch['sha256']: raise ValueError('Crate patch digest mismatch')
    identity = {'source': source, 'patches': selected, 'patch_application': PATCH_APPLICATION}
    destination = directory / name
    if destination.exists():
        marker = destination / MARKER
        if destination.is_symlink() or not marker.is_file() or marker.is_symlink():
            raise ValueError('Existing crate preparation is unrecorded; preserved')
        saved = json.loads(marker.read_text())
        if saved.get('identity') != identity or saved.get('files') != file_manifest(destination):
            raise ValueError('Prepared crate identity or complete file manifest changed; preserved')
        return
    archive = verified_archive(directory, name, source, cache)
    staging = pathlib.Path(tempfile.mkdtemp(prefix=f'.{name}-preparing-', dir=directory))
    try:
        extract_archive(archive, staging, f'{name}-{source["version"]}')
        # An archive has no repository of its own. Parent-repository discovery
        # makes Git-format patches appear outside the current prefix: git can
        # then report success while silently skipping their files. Apply as a
        # plain directory, with discovery stopped at the staging parent.
        patch_environment = dict(os.environ)
        for key in ('GIT_DIR', 'GIT_WORK_TREE', 'GIT_COMMON_DIR', 'GIT_INDEX_FILE', 'GIT_PREFIX'):
            patch_environment.pop(key, None)
        patch_environment['GIT_CEILING_DIRECTORIES'] = str(staging.parent.resolve())
        for patch in selected:
            path = str((root / patch['path']).resolve())
            subprocess.run(['git', '-C', str(staging), 'apply', '--no-index', '--check', path], check=True, env=patch_environment)
            subprocess.run(['git', '-C', str(staging), 'apply', '--no-index', path], check=True, env=patch_environment)
        files = file_manifest(staging)
        (staging / MARKER).write_text(json.dumps({'identity': identity, 'files': files}, sort_keys=True) + '\n')
        staging.replace(destination)
    finally:
        if staging.exists(): shutil.rmtree(staging)
