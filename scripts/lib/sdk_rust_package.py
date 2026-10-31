"""Standalone Cargo candidate manifests and checksummed registry-format archives.

No source checkout is edited. Workspace inheritance and local dependency paths
are resolved into ordinary version requirements in the candidate only. Original
manifests and exact source bytes accompany every prepared dependency archive.
"""
import copy
import gzip
import hashlib
import json
from pathlib import Path, PurePosixPath
import shutil
import tarfile
import tomllib


def sha256(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def toml_value(value):
    if isinstance(value, bool):
        return 'true' if value else 'false'
    if isinstance(value, str):
        return json.dumps(value, ensure_ascii=False)
    if isinstance(value, int):
        return str(value)
    if isinstance(value, list):
        return '[' + ', '.join(toml_value(item) for item in value) + ']'
    if isinstance(value, dict):
        return '{' + ', '.join(toml_value(key) + ' = ' + toml_value(item) for key, item in value.items()) + '}'
    raise ValueError('Unsupported Cargo manifest value')


def write_toml(path, value):
    text = ''.join(toml_value(key) + ' = ' + toml_value(item) + '\n' for key, item in value.items())
    if tomllib.loads(text) != value:
        raise ValueError('Manifest did not round trip')
    Path(path).write_text(text)


def workspace(manifest):
    data = tomllib.loads(manifest.read_text())
    explicit = data.get('package', {}).get('workspace')
    parents = [manifest.parent / explicit] if explicit else manifest.parent.parents
    for parent in parents:
        candidate = parent / 'Cargo.toml'
        if candidate.is_file():
            value = tomllib.loads(candidate.read_text())
            if 'workspace' in value:
                return parent.resolve(), value['workspace']
    return manifest.parent, {}


def standalone_manifest(manifest):
    manifest = Path(manifest).resolve(strict=True)
    value = tomllib.loads(manifest.read_text())
    base, inherited = workspace(manifest)
    value = copy.deepcopy(value)
    package = value['package']
    for key, item in list(package.items()):
        if isinstance(item, dict) and item.get('workspace') is True:
            package[key] = copy.deepcopy(inherited['package'][key])
    package.pop('workspace', None)
    # All generated candidate manifests explicitly refuse accidental publication.
    package['publish'] = False
    paths = []

    def dependencies(table, kind):
        for alias, specification in list(table.items()):
            dependency = {'version': specification} if isinstance(specification, str) else copy.deepcopy(specification)
            dependency_base = manifest.parent
            if dependency.pop('workspace', False):
                source = copy.deepcopy(inherited['dependencies'][alias])
                source = {'version': source} if isinstance(source, str) else source
                dependency['features'] = list(dict.fromkeys(source.get('features', []) + dependency.get('features', [])))
                source.update(dependency)
                dependency = source
                dependency_base = base
            if 'path' in dependency:
                # Cargo itself omits path-only forward development dependencies
                # from published archives. They are not runtime dependencies.
                if kind == 'dev-dependencies' and 'version' not in dependency:
                    del table[alias]
                    continue
                target = (dependency_base / dependency.pop('path') / 'Cargo.toml').resolve(strict=True)
                target_data = tomllib.loads(target.read_text())['package']
                version = target_data['version']
                if isinstance(version, dict):
                    version = workspace(target)[1]['package']['version']
                name = target_data['name']
                if dependency.get('package', alias) != name:
                    raise ValueError('Path dependency package identity mismatch')
                dependency['version'] = '=' + version
                paths.append(target)
            if any(key in dependency for key in ('git', 'branch', 'rev', 'tag')):
                raise ValueError('Unpinned registry candidate dependency')
            table[alias] = dependency

    for kind in ('dependencies', 'build-dependencies', 'dev-dependencies'):
        dependencies(value.get(kind, {}), kind)
    for target in value.get('target', {}).values():
        for kind in ('dependencies', 'build-dependencies', 'dev-dependencies'):
            dependencies(target.get(kind, {}), kind)
    if value.get('lints', {}).get('workspace'):
        value['lints'] = copy.deepcopy(inherited['lints'])
    for key in ('workspace', 'patch', 'replace'):
        value.pop(key, None)
    return value, paths, base


def ordinary_files(directory):
    ignored = {'.git', '.local', '.deps', 'target', 'node_modules', '__pycache__'}
    for path in sorted(directory.rglob('*')):
        relative = path.relative_to(directory)
        if any(part in ignored for part in relative.parts):
            continue
        if path.is_symlink():
            raise ValueError('Symlink in candidate source')
        if path.is_file() and relative.as_posix() != '.cargo-checksum.json':
            yield path


def create_archive(directory, output, epoch):
    with output.open('xb') as stream, gzip.GzipFile(fileobj=stream, mode='wb', filename='', mtime=epoch) as compressed, tarfile.open(fileobj=compressed, mode='w') as archive:
        for source in ordinary_files(directory):
            info = archive.gettarinfo(str(source), directory.name + '/' + source.relative_to(directory).as_posix())
            info.uid = info.gid = 0
            info.uname = info.gname = ''
            info.mtime = epoch
            info.mode = 0o755 if source.stat().st_mode & 0o111 else 0o644
            with source.open('rb') as content:
                archive.addfile(info, content)


def prepare_package(manifest, vendor, archives, epoch):
    value, paths, workspace_root = standalone_manifest(manifest)
    package = value['package']
    identity = package['name'] + '-' + package['version']
    destination = vendor / identity
    destination.mkdir()
    before = {}
    for source in ordinary_files(manifest.parent):
        relative = source.relative_to(manifest.parent)
        before[relative.as_posix()] = sha256(source)
        target = destination / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, target)
        target.chmod(0o755 if source.stat().st_mode & 0o111 else 0o644)
    shutil.copyfile(manifest, destination / 'Cargo.toml.orig')
    for field in ('readme', 'license-file'):
        name = package.get(field)
        if isinstance(name, str):
            relative = PurePosixPath(name)
            if relative.is_absolute() or '..' in relative.parts:
                raise ValueError('External package documentation path')
    # MIT RVoIP/Connect packages inherit the grant but often have no local file.
    if package.get('license') == 'MIT' and not (destination / 'LICENSE').exists():
        license_path = workspace_root / 'LICENSE'
        if not license_path.is_file() or license_path.is_symlink():
            raise ValueError('Missing original inherited license')
        shutil.copyfile(license_path, destination / 'LICENSE')
    write_toml(destination / 'Cargo.toml', value)
    for table in (value.get('lib', {}), *value.get('bin', []), *value.get('example', []), *value.get('test', []), *value.get('bench', [])):
        name = table.get('path')
        if name and (PurePosixPath(name).is_absolute() or '..' in PurePosixPath(name).parts):
            raise ValueError('External Cargo target path')
    archive = archives / (identity + '.crate')
    create_archive(destination, archive, epoch)
    checksums = {p.relative_to(destination).as_posix(): sha256(p) for p in ordinary_files(destination)}
    (destination / '.cargo-checksum.json').write_text(json.dumps({'package': sha256(archive), 'files': checksums}, sort_keys=True) + '\n')
    if before != {p.relative_to(manifest.parent).as_posix(): sha256(p) for p in ordinary_files(manifest.parent)}:
        raise ValueError('Prepared package source changed during packaging')
    return {'name': package['name'], 'version': package['version'], 'archive': archive.name,
            'sha256': sha256(archive), 'source_files': before,
            'normalized_manifest_sha256': sha256(destination / 'Cargo.toml'),
            'publication_authorized': False}, paths


def install_archive(archive, vendor, name, version, expected):
    """Extract verified registry bytes, refusing escaping/link/duplicate entries."""
    if sha256(archive) != expected:
        raise ValueError('Registry archive checksum mismatch')
    identity = name + '-' + version
    destination = vendor / identity
    destination.mkdir()
    with tarfile.open(archive, 'r:gz') as content:
        members = content.getmembers()
        if len(members) > 30000 or sum(m.size for m in members) > 256 * 1024 * 1024:
            raise ValueError('Registry archive exceeds package budget')
        seen = set()
        for member in members:
            relative = PurePosixPath(member.name)
            if relative.is_absolute() or '..' in relative.parts or not relative.parts or relative.parts[0] != identity or member.name in seen:
                raise ValueError('Invalid registry archive path')
            seen.add(member.name)
            if not (member.isfile() or member.isdir()):
                raise ValueError('Nonordinary registry archive entry')
            path = vendor.joinpath(*relative.parts)
            if member.isdir():
                path.mkdir(parents=True, exist_ok=True)
            else:
                path.parent.mkdir(parents=True, exist_ok=True)
                with content.extractfile(member) as source, path.open('xb') as target:
                    shutil.copyfileobj(source, target)
                path.chmod(0o755 if member.mode & 0o111 else 0o644)
    value = tomllib.loads((destination / 'Cargo.toml').read_text())['package']
    if value['name'] != name or value['version'] != version:
        raise ValueError('Registry archive identity mismatch')
    checksums = {p.relative_to(destination).as_posix(): sha256(p) for p in ordinary_files(destination)}
    (destination / '.cargo-checksum.json').write_text(json.dumps({'package': expected, 'files': checksums}, sort_keys=True) + '\n')
