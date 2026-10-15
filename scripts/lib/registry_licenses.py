"""Collect exact registry notices, with pinned repository-root provenance.

The selected graph is normal/build dependencies of the package binaries on the
requested target. The unfiltered index deliberately also documents other nodes;
those nodes never acquire a runtime-coverage claim merely by appearing there.
"""
import hashlib
import json
import pathlib
import shutil
import tomllib
import tarfile

PREFIXES = ('LICENSE', 'LICENCE', 'COPYING', 'NOTICE', 'UNLICENSE')
REGISTRY = 'registry+https://github.com/rust-lang/crates.io-index'
LOCK_PATH = 'dependencies/registry-license-lock.json'


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def safe_file(base, relative):
    relative = pathlib.PurePosixPath(relative)
    if relative.is_absolute() or not relative.parts or any(p in ('.', '..') for p in relative.parts):
        raise ValueError('Unsafe license path')
    path = base
    for part in relative.parts:
        path = path / part
        if path.is_symlink():
            raise ValueError('Symlink license path')
    if not path.is_file() or not path.resolve().is_relative_to(base.resolve()):
        raise ValueError('License path missing or outside source')
    return path


def selected_dependency_ids(metadata, roots):
    """Input must be Cargo metadata filtered for the actual build target."""
    packages = metadata['packages']
    selected = []
    for root in roots:
        matches = [p['id'] for p in packages if p['name'] == root]
        if len(matches) != 1:
            raise ValueError('Missing or ambiguous runtime root')
        selected += matches
    nodes = {n['id']: n for n in metadata['resolve']['nodes']}
    result = set()
    while selected:
        identity = selected.pop()
        if identity in result:
            continue
        result.add(identity)
        if identity not in nodes:
            raise ValueError('Selected dependency node missing')
        for dependency in nodes[identity]['deps']:
            if any(k['kind'] in (None, 'build') for k in dependency['dep_kinds']):
                selected.append(dependency['pkg'])
    return result


def notice_paths(package):
    base = pathlib.Path(package['manifest_path']).parent
    found = set()
    for path in base.rglob('*'):
        relative = path.relative_to(base)
        # Preserve directory layouts (for example LICENSES/MIT.txt), not just
        # familiar top-level filenames. Reject symlinks instead of following.
        if ((len(relative.parts) == 1 and path.name.upper().startswith(PREFIXES)) or
                any(part.upper() in ('LICENSES', 'LICENCES') for part in relative.parts[:-1])):
            if path.is_symlink():
                raise ValueError('Symlink notice')
            if path.is_file():
                found.add(relative.as_posix())
    explicit = package.get('license_file')
    if explicit:
        path = pathlib.Path(explicit)
        if path.is_absolute():
            try:
                explicit = path.relative_to(base).as_posix()
            except ValueError as error:
                raise ValueError('Explicit license outside package') from error
        safe_file(base, explicit)
        found.add(explicit)
    return sorted(found)


def copy_registry_notices(root, runtime, metadata, selected_ids):
    registry = [p for p in metadata['packages'] if (p.get('source') or '').startswith('registry+')]
    known_ids = {p['id'] for p in metadata['packages']}
    if not selected_ids <= known_ids:
        raise ValueError('Selected graph does not match package metadata')
    lock = json.loads(safe_file(root, LOCK_PATH).read_text())
    if lock.get('version') != 1:
        raise ValueError('Unsupported registry license provenance version')
    entries = lock['packages']
    cargo = tomllib.loads((root / 'Cargo.lock').read_text())
    cargo_packages = {(p['name'], p['version'], p.get('source')): p for p in cargo['package']}
    notices, used, copies = [], set(), {}
    for package in registry:
        identity = package['name'] + '-' + package['version']
        source = package['source']
        base = pathlib.Path(package['manifest_path']).parent
        cargo_entry = cargo_packages.get((package['name'], package['version'], source))
        if not cargo_entry or not cargo_entry.get('checksum'):
            raise ValueError('Registry package missing exact Cargo.lock identity')
        # Cargo's extracted registry tree need not contain .cargo-checksum.json.
        # Bind notice bytes to the actual checksummed .crate archive instead.
        if base.parent.parent.name != 'src':
            raise ValueError('Unexpected registry cache layout')
        archive = base.parents[2] / 'cache' / base.parent.name / (identity + '.crate')
        if not archive.is_file() or digest(archive) != cargo_entry['checksum']:
            raise ValueError('Registry archive checksum mismatch')
        relative_notices = notice_paths(package)
        needed = set(relative_notices) | {'Cargo.toml'}
        if identity in entries:
            needed.add('.cargo_vcs_info.json')
        archived = {}
        with tarfile.open(archive) as tar:
            for member in tar:
                prefix = identity + '/'
                relative = member.name[len(prefix):] if member.name.startswith(prefix) else None
                if relative not in needed:
                    continue
                if not member.isfile() or relative in archived:
                    raise ValueError('Invalid or duplicate archive notice member')
                with tar.extractfile(member) as stream:
                    archived[relative] = hashlib.file_digest(stream, 'sha256').hexdigest()

        def verified_crate_file(relative):
            path = safe_file(base, relative)
            if archived.get(relative) != digest(path):
                raise ValueError('Registry file checksum mismatch')
            return path

        verified_crate_file('Cargo.toml')
        texts = []
        for relative in relative_notices:
            path = verified_crate_file(relative)
            target = 'licenses/registry/' + identity + '/' + relative
            copies[target] = path
            texts.append(target)
        provenance = None
        if identity in entries:
            entry = entries[identity]
            used.add(identity)
            for key, actual in (('name', package['name']), ('version', package['version']),
                                ('source', source), ('checksum', cargo_entry['checksum']),
                                ('license', package.get('license')), ('repository', package.get('repository'))):
                if entry.get(key) != actual:
                    raise ValueError('Registry license provenance identity mismatch: ' + key)
            vcs = json.loads(verified_crate_file('.cargo_vcs_info.json').read_text())
            if vcs['git']['sha1'] != entry['revision'] or vcs.get('path_in_vcs') != entry['path_in_vcs']:
                raise ValueError('Registry VCS provenance mismatch')
            if not entry['license_files']:
                raise ValueError('Empty explicit license provenance')
            for notice in entry['license_files']:
                path = safe_file(root, notice['path'])
                if digest(path) != notice['sha256']:
                    raise ValueError('Pinned repository license digest mismatch')
                expected = 'https://raw.githubusercontent.com/' + entry['github_repository'] + '/' + entry['revision'] + '/' + notice['upstream_path']
                if notice['url'] != expected:
                    raise ValueError('Pinned license URL does not bind exact revision')
                repository = entry['repository'].rstrip('/').removesuffix('.git')
                if repository != 'https://github.com/' + entry['github_repository']:
                    raise ValueError('Pinned license repository mismatch')
                target = 'licenses/provenance/' + notice['path']
                copies[target] = path
                texts.append(target)
            provenance = entry
        required = package['id'] in selected_ids
        if required and not texts:
            raise ValueError('Selected runtime/build dependency has no license text: ' + identity)
        notices.append({'package': identity, 'license': package.get('license'), 'source': source,
                        'texts': sorted(set(texts)), 'selected_normal_build_dependency': required,
                        'coverage': 'text-bundled' if texts else 'outside-selected-graph-declaration-only',
                        'repository_license_provenance': provenance})
    if used != set(entries):
        raise ValueError('Unused or stale registry license provenance entries')
    copies['licenses/provenance/' + LOCK_PATH] = safe_file(root, LOCK_PATH)
    for relative, source in sorted(copies.items()):
        target = runtime / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, target)
    target = runtime / 'licenses/registry-index.json'
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(json.dumps(notices, indent=2) + '\n')
    return notices
