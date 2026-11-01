#!/usr/bin/env python3
"""Independently inspect the sealed SDK ZIP without extracting its payload."""
import argparse
import ast
import hashlib
import io
import json
from pathlib import Path, PurePosixPath
import runpy
import subprocess
import sys
import tarfile
import tomllib
import zipfile

ROOT = Path(__file__).resolve().parents[1]
DOCUMENTS = {'manifest.json', 'publication.json', 'sdk/rust/manifest.json', 'sources/manifest.json',
             *('verification/' + name + '.json' for name in ('runtime', 'node', 'python', 'installed', 'offline'))}


def digest(body):
    return hashlib.sha256(body).hexdigest()


def original_path(name):
    for prefix, replacement in [('sources/typescript-source/', 'sdk/typescript/'),
                                ('sources/python-source/', 'sdk/python/')]:
        if name.startswith(prefix):
            return replacement + name.removeprefix(prefix)
    if name in ('LICENSE', 'THIRD_PARTY_NOTICES.md') or name.startswith(('docs/', 'contracts/', 'examples/sdk-installation/')):
        return name
    return None


def generated_readme(source):
    values = [node.args[0].value for node in ast.walk(ast.parse(source))
              if isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute) and node.func.attr == 'write_text'
              and node.args and isinstance(node.args[0], ast.Constant) and isinstance(node.args[0].value, str)
              and node.args[0].value.startswith('# Connect SDK candidate\n')]
    value, = values
    return value.encode()


def inspect_archive(stream, identity, manifest_sha256):
    """Read ordinary bytes and modes; never resolve a path on the filesystem."""
    files, documents, seen = {}, {}, set()
    with tarfile.open(fileobj=stream, mode='r|gz') as archive:
        total = 0
        for member in archive:
            path = PurePosixPath(member.name)
            if (path.is_absolute() or '..' in path.parts or path.as_posix() != member.name
                    or len(path.parts) < 2 or path.parts[0] != identity or member.name in seen):
                raise ValueError('Unsafe or duplicate sealed candidate path')
            seen.add(member.name)
            total += member.size
            if len(seen) > 100000 or total > 4 * 1024**3 or member.size > 512 * 1024**2:
                raise ValueError('Sealed candidate exceeds inspection budget')
            if not member.isfile() or member.mode not in (0o644, 0o755):
                raise ValueError('Nonordinary sealed candidate file or mode')
            name = '/'.join(path.parts[1:])
            with archive.extractfile(member) as content:
                if name in DOCUMENTS:
                    if member.size > 24 * 1024**2:
                        raise ValueError('Sealed manifest exceeds inspection budget')
                    body = content.read()
                    documents[name] = json.loads(body)
                    checksum = digest(body)
                else:
                    checksum = hashlib.file_digest(content, 'sha256').hexdigest()
            files[name] = {'sha256': checksum, 'mode': member.mode}
    if not DOCUMENTS <= documents.keys():
        raise ValueError('Required sealed package manifests are missing')
    if files.pop('manifest.json') != {'sha256': manifest_sha256, 'mode': 0o644}:
        raise ValueError('Sealed manifest differs from independent evidence')
    if files != documents['manifest.json']['files']:
        raise ValueError('Actual sealed payload bytes or modes differ')
    return files, documents


def verify_components(files, documents, proofs, revision, target, runtime_verified):
    """Require exact original installed/offline/native identities, never just green flags."""
    manifest = documents['manifest.json']; publication = documents['publication.json']
    for name in ('runtime', 'installed', 'offline'):
        value = proofs[name]
        if value != documents['verification/' + name + '.json']:
            raise ValueError('Packaged component proof differs from CI evidence')
        if not (value['passed'] and value['source_inputs_unchanged'] and value['source_revision'] == revision):
            raise ValueError('Component source or successful result differs')
    runtime, installed, offline = (proofs[name] for name in ('runtime', 'installed', 'offline'))
    node, python = (documents['verification/' + name + '.json'] for name in ('node', 'python'))
    if not (manifest['source_revision'] == publication['source_revision'] == revision
            and manifest['target'] == publication['target'] == runtime['target'] == installed['target'] == target
            and manifest['version'] == publication['version'] == runtime['version'] == node['version']
            and manifest['candidate_installation_verified'] and not manifest['publication_authorized']
            and not manifest['release_ready'] and not manifest['task_complete'] and not manifest['whole_gate_qualified']
            and not publication['publication_authorized'] and not publication['release_ready']
            and not publication['registry_account_ownership_verified']):
        raise ValueError('Candidate identity or disposition differs')
    if not (runtime_verified['passed'] and runtime_verified['production_build_verified']
            and runtime_verified['source_revision'] == revision and runtime_verified['target'] == target
            and runtime_verified['binary_sha256'] == runtime['binary_sha256']
            and runtime_verified['archive_sha256'] == runtime['archive_sha256']
            and runtime_verified['manifest_sha256'] == runtime['manifest_sha256']):
        raise ValueError('Independent original native asset verification differs')
    if not (node['passed'] and python['passed'] and node['source_inputs_unchanged'] and python['source_inputs_unchanged']
            and node['consumer_outside_repository'] and node['empty_cache'] and node['external_declarations_checked']
            and python['consumer_outside_repository'] and python['consumer']['owned_workers_joined'] == 6
            and installed['consumer_outside_repository'] and installed['original_server_joined']
            and installed['native_helpers_joined'] == 2 and offline['consumer_outside_repository'] and offline['empty_caches']
            and offline['typescript_distribution_byte_identical'] and offline['python_wheel_byte_identical']):
        raise ValueError('Installed owners or cold offline build evidence is missing')
    for role in ('node', 'python'):
        if not (installed[role]['peer_capacity_reclaimed'] and installed[role]['helper'] ==
                {'native_helpers_joined': 1, 'actual_default_executable': True}):
            raise ValueError('Original installed helper or capacity reclaim differs')
    if not (installed['rust']['cold_offline_crate_installation'] and installed['rust']['peer_capacity_reclaimed']
            and installed['steps'] and offline['steps']
            and all(step['exit_code'] == 0 for step in installed['steps'] + offline['steps'])):
        raise ValueError('Native or offline execution is incomplete')
    for name, expected in [
        ('runtime/' + runtime['archive_name'], runtime['archive_sha256']),
        ('sdk/node/' + node['archive_name'], node['archive_sha256']),
        ('sdk/python/' + python['wheel_name'], python['wheel_sha256']),
        ('sdk/rust/manifest.json', installed['rust_source_manifest_sha256']),
        ('sources/manifest.json', offline['manifest_sha256']),
    ]:
        if files.get(name, {}).get('sha256') != expected:
            raise ValueError('Actual component archive or source manifest differs')
    if not (installed['runtime_archive_sha256'] == runtime['archive_sha256']
            and installed['node_archive_sha256'] == node['archive_sha256']
            and installed['python_wheel_sha256'] == python['wheel_sha256']
            and publication['runtime']['sha256'] == runtime['archive_sha256']
            and publication['runtime']['server_and_helper_sha256'] == runtime['binary_sha256']
            and publication['npm']['sha256'] == node['archive_sha256']
            and publication['pypi']['sha256'] == python['wheel_sha256']):
        raise ValueError('Installed or publication component bindings differ')
    for prefix in ('sdk/rust/', 'sources/'):
        source_manifest = documents[prefix + 'manifest.json']
        actual = {name.removeprefix(prefix): record['sha256'] for name, record in files.items()
                  if name.startswith(prefix) and name != prefix + 'manifest.json'}
        if source_manifest['source_revision'] != revision or actual != source_manifest['files']:
            raise ValueError('Complete nested source payload differs')
    for dependency in node['dependency_archives']:
        archive = 'sdk/node/dependencies/' + dependency['file_name']
        if (files.get(archive, {}).get('sha256') != dependency['sha256']
                or files.get(archive.removesuffix('.tgz') + '.LICENSE', {}).get('sha256') != dependency['license_sha256']):
            raise ValueError('Actual runtime dependency or original notice differs')


def verify_rust_sources(files, manifest, sources, registry):
    """Bind repackaged crate contents to the independently read source/lock."""
    for package in manifest['registry_packages']:
        if registry.get((package['name'], package['version'])) != package['sha256']:
            raise ValueError('Rust registry archive differs from original Cargo.lock')
    for package in manifest['prepared_packages'] + manifest['registry_packages']:
        if files.get('sdk/rust/archives/' + package['archive'], {}).get('sha256') != package['sha256']:
            raise ValueError('Actual Rust crate archive differs')
    for package in manifest['prepared_packages']:
        base = PurePosixPath(package['manifest_source']).parent
        prefix = 'sdk/rust/vendor/' + package['name'] + '-' + package['version'] + '/'
        if (files.get(prefix + 'Cargo.toml', {}).get('sha256') != package['normalized_manifest_sha256']
                or files.get(prefix + package['source_manifest_file'], {}).get('sha256') !=
                sources.get(str(base / 'Cargo.toml'), {}).get('sha256')):
            raise ValueError('Original or normalized Rust manifest differs')
        for name, expected in package['source_files'].items():
            if sources.get(str(base / name), {}).get('sha256') != expected:
                raise ValueError('Declared Rust source differs from original inputs')
            if name != 'Cargo.toml' and files.get(prefix + name, {}).get('sha256') != expected:
                raise ValueError('Actual repackaged Rust source differs: ' + prefix + name)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('evidence', 'prepared', 'runtime-verification', 'output'):
        parser.add_argument('--' + name, type=Path, required=True)
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument('--package', type=Path)
    source.add_argument('--package-stdin', action='store_true')
    for name in ('evidence-sha256', 'package-sha256', 'revision', 'target'):
        parser.add_argument('--' + name, required=True)
    parser.add_argument('--archives', type=Path)
    args = parser.parse_args()
    verifier = runpy.run_path(str(ROOT / 'scripts/verify-linux-installation.py'))
    proofs, sources = {}, None
    with verifier['artifact'](args.evidence, args.evidence_sha256) as evidence:
        prefix = args.target + '/'
        for name, suffix in [('runtime', 'runtime.json'), ('installed', 'installed/summary.json'),
                             ('offline', 'offline/summary.json'), ('assembled', 'assembled/summary.json')]:
            proofs[name] = json.loads(evidence.read(prefix + suffix))
        for name, suffix in [('runtime', 'source-inputs.json'), ('installed', 'installed/source-inputs.json'),
                             ('offline', 'offline/source-inputs.json')]:
            body = evidence.read(prefix + suffix)
            if digest(body) != proofs[name]['source_inputs_sha256']:
                raise ValueError('Original component source inventory digest differs')
            current = json.loads(body)
            if sources is not None and current != sources:
                raise ValueError('Components were not verified from the same complete sources')
            sources = current
    counts, git_hashes = verifier['verify_sources'](sources, args.revision, args.prepared, args.archives)
    sealed = proofs['assembled']
    if not (sealed['source_revision'] == args.revision and sealed['target'] == args.target
            and sealed['candidate_installation_verified'] and not sealed['publication_authorized']
            and not sealed['release_ready'] and not sealed['task_complete'] and not sealed['whole_gate_qualified']):
        raise ValueError('Sealed candidate evidence differs')
    if args.package_stdin:
        body = sys.stdin.buffer.read(512 * 1024**2 + 1)
        if len(body) > 512 * 1024**2 or digest(body) != args.package_sha256:
            raise ValueError('Bounded raw ZIP digest differs')
        package = zipfile.ZipFile(io.BytesIO(body))
    else:
        package = verifier['artifact'](args.package, args.package_sha256)
    with package as archive:
        name, = archive.namelist()
        if PurePosixPath(name).name != sealed['archive_name']:
            raise ValueError('Sealed artifact identity differs')
        with archive.open(name) as stream:
            if hashlib.file_digest(stream, 'sha256').hexdigest() != sealed['archive_sha256']:
                raise ValueError('Original sealed tarball digest differs')
        with archive.open(name) as stream:
            files, documents = inspect_archive(stream, sealed['archive_name'].removesuffix('.tar.gz'), sealed['manifest_sha256'])
    verify_components(files, documents, proofs, args.revision, args.target, json.loads(args.runtime_verification.read_text()))
    if len(files) != sealed['files'] or documents['sdk/rust/manifest.json']['inputs'] != git_hashes:
        raise ValueError('Sealed file count or Rust original Git inventory differs')
    lock = tomllib.loads(subprocess.check_output(['git', 'show', args.revision + ':Cargo.lock'], cwd=ROOT, text=True))
    registry = {(p['name'], p['version']): p['checksum'] for p in lock['package'] if p.get('source', '').startswith('registry+')}
    verify_rust_sources(files, documents['sdk/rust/manifest.json'], sources, registry)
    # Copied original public examples/docs and source build inputs must still be
    # original Git bytes, independently of the producer's declared inventory.
    original_count = 0
    for name, record in files.items():
        original = original_path(name)
        if original is not None:
            if record['sha256'] != git_hashes.get(original):
                raise ValueError('Shipped original source differs: ' + name)
            original_count += 1
    producer_source = subprocess.check_output(['git', 'show', args.revision + ':scripts/assemble-sdk-candidate.py'], cwd=ROOT)
    if files.get('README.md', {}).get('sha256') != digest(generated_readme(producer_source)):
        raise ValueError('Generated candidate instructions differ from original producer source')
    report = {'format': 'connect/verified-sealed-sdk-candidate/1', 'source_revision': args.revision,
              'target': args.target, 'sources_verified': counts, 'files_verified': len(files),
              'original_git_payload_files_verified': original_count, 'archive_sha256': sealed['archive_sha256'],
              'manifest_sha256': sealed['manifest_sha256'], 'passed': True,
              'artifact_zip_digests': {'evidence': args.evidence_sha256, 'package': args.package_sha256},
              'scope': 'Matching platform SDK candidate, original component identities and cold installed/offline evidence; not full SDK parity, final release, publication or Gate 6 acceptance.',
              'task_complete': False, 'whole_gate_qualified': False, 'publication_authorized': False}
    args.output.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({key: report[key] for key in ('passed', 'target', 'sources_verified', 'files_verified')}))


if __name__ == '__main__':
    main()
