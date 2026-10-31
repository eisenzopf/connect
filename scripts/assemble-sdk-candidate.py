#!/usr/bin/env python3
"""Seal an unpublished platform SDK candidate from matching qualified components."""
import argparse
import gzip
import json
import os
from pathlib import Path
import runpy
import shutil
import subprocess
import tarfile
import tomllib

from lib.sdk_candidate import digest

ROOT = Path(__file__).resolve().parents[1]


def inventory(root, normalized=False):
    values = {}
    for path in sorted(root.rglob('*')):
        if path.is_symlink():
            raise ValueError('Candidate contains a symlink')
        if path.is_file():
            mode = path.stat().st_mode & 0o777
            values[path.relative_to(root).as_posix()] = {'sha256': digest(path), 'mode': (0o755 if mode & 0o111 else 0o644) if normalized else mode}
    return values


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('runtime-proof', 'node-proof', 'python-proof', 'rust-source', 'installed-proof', 'offline-proof', 'offline-inputs', 'output', 'evidence'):
        parser.add_argument('--' + name, type=Path, required=True)
    args = parser.parse_args()
    def read(path): return json.loads(path.read_text())
    runtime = read(args.runtime_proof); node = read(args.node_proof); python = read(args.python_proof)
    rust = read(args.rust_source / 'manifest.json'); installed = read(args.installed_proof); offline = read(args.offline_proof)
    head = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
    if subprocess.check_output(['git', 'status', '--porcelain'], cwd=ROOT, text=True).strip():
        raise ValueError('Candidate source is not committed and clean')
    if not all(value['passed'] and value['source_inputs_unchanged'] for value in (runtime, installed, offline)):
        raise ValueError('Required native/offline qualification did not pass')
    if not (node['passed'] and python['passed'] and installed['native_helpers_joined'] == 2
            and installed['original_server_joined'] and offline['typescript_distribution_byte_identical'] and offline['python_wheel_byte_identical']):
        raise ValueError('Required package/owner checks are missing')
    if {value['source_revision'] for value in (runtime, rust, installed, offline)} != {head}:
        raise ValueError('Qualified component source revisions differ')
    if not (installed['runtime_archive_sha256'] == runtime['archive_sha256']
            and installed['node_archive_sha256'] == node['archive_sha256']
            and installed['python_wheel_sha256'] == python['wheel_sha256']
            and installed['rust_source_manifest_sha256'] == digest(args.rust_source / 'manifest.json')):
        raise ValueError('Installed component identity differs')
    source_inputs = read(args.installed_proof.parent / 'source-inputs.json')
    if digest(args.installed_proof.parent / 'source-inputs.json') != installed['source_inputs_sha256']:
        raise ValueError('Installed source inventory changed')
    if source_inputs != runpy.run_path(str(ROOT / 'scripts/check-gate6-linux-hosting.py'))['sources']():
        raise ValueError('Qualified full source inventory differs')
    for name, record in source_inputs.items():
        path = ROOT / name
        if not path.is_file() or path.is_symlink() or digest(path) != record['sha256'] or path.stat().st_mode & 0o777 != record['mode']:
            raise ValueError('Current source differs from qualified input')
    output = args.output.resolve(); evidence = args.evidence.resolve()
    if not output.is_relative_to((ROOT / 'dist').resolve()) or not evidence.is_relative_to((ROOT / '.local').resolve()):
        parser.error('Use dist/ candidates and ignored .local/ evidence')
    identity = 'connect-sdk-' + runtime['version'] + '-' + runtime['target'] + '-' + head[:12]
    candidate = output / identity; candidate.mkdir(parents=True, exist_ok=False)
    def include(source, name):
        target = candidate / name; target.parent.mkdir(parents=True, exist_ok=True); shutil.copyfile(source, target); target.chmod(0o644)
        return target
    def tree(source, name):
        # These are independent immutable producer copies, never Cargo outputs
        # or the source checkout. Hard links bound assembly disk use; the final
        # tarball contains regular bytes and no external filesystem dependency.
        shutil.copytree(source, candidate / name, copy_function=os.link)
    runtime_archive = ROOT / 'dist/sdk/runtime' / runtime['archive_name']
    if digest(runtime_archive) != runtime['archive_sha256']:
        raise ValueError('Native archive changed before sealing')
    include(runtime_archive, 'runtime/' + runtime_archive.name)
    node_archive = ROOT / 'dist/sdk/typescript' / args.node_proof.parent.name / node['archive_name']
    wheel = ROOT / 'dist/sdk/python' / args.python_proof.parent.name / python['wheel_name']
    if digest(node_archive) != node['archive_sha256'] or digest(wheel) != python['wheel_sha256']:
        raise ValueError('SDK archive changed before sealing')
    include(node_archive, 'sdk/node/' + node_archive.name); include(wheel, 'sdk/python/' + wheel.name)
    for dependency in node['dependency_archives']:
        source = node_archive.parent / 'dependencies' / dependency['file_name']
        if digest(source) != dependency['sha256']:
            raise ValueError('Pinned SDK runtime dependency changed')
        include(source, 'sdk/node/dependencies/' + source.name)
        license_path = source.with_suffix('.LICENSE')
        if digest(license_path) != dependency['license_sha256']:
            raise ValueError('SDK dependency notice changed')
        include(license_path, 'sdk/node/dependencies/' + license_path.name)
    expected = rust['files']
    if {name: value['sha256'] for name, value in inventory(args.rust_source).items() if name != 'manifest.json'} != expected:
        raise ValueError('Qualified Rust source bundle changed')
    offline_manifest = read(args.offline_inputs / 'manifest.json')
    if (digest(args.offline_inputs / 'manifest.json') != offline['manifest_sha256']
            or {name: value['sha256'] for name, value in inventory(args.offline_inputs).items() if name != 'manifest.json'} != offline_manifest['files']):
        raise ValueError('Qualified offline source inputs changed')
    tree(args.rust_source, 'sdk/rust'); tree(args.offline_inputs, 'sources')
    shutil.copytree(ROOT / 'examples/sdk-installation', candidate / 'examples/sdk-installation')
    for name in ('LICENSE', 'THIRD_PARTY_NOTICES.md', 'docs/developer/python-sdk.md', 'docs/developer/node-typed-sdk.md',
                 'docs/developer/rust-sdk.md', 'docs/developer/browser-sdk.md',
                 'contracts/openapi-application-http.json', 'contracts/application-http-dispatch.json'):
        if not (ROOT / name).is_file():
            raise ValueError('Required public package reference is absent: ' + name)
        include(ROOT / name, name)
    for name, path in [('runtime', args.runtime_proof), ('node', args.node_proof), ('python', args.python_proof),
                       ('installed', args.installed_proof), ('offline', args.offline_proof)]:
        include(path, 'verification/' + name + '.json')
    publication = {'format': 'connect/sdk-publication-manifest/1', 'source_revision': head,
        'publication_authorized': False, 'target': runtime['target'], 'version': runtime['version'],
        'npm': {'name': node['package_name'], 'archive': 'sdk/node/' + node_archive.name, 'sha256': node['archive_sha256']},
        'pypi': {'distribution': python['distribution_name'], 'module': 'connect_sdk', 'archive': 'sdk/python/' + wheel.name, 'sha256': python['wheel_sha256']},
        'cargo': {'name': 'connect-client', 'version': rust['version'], 'archive': 'sdk/rust/archives/connect-client-' + rust['version'] + '.crate',
                  'distribution': 'complete-offline-directory-source', 'prepared_upstream_archives': rust['prepared_packages'],
                  'public_registry_resolution_ready': False,
                  'review_required': 'Prepared dependency forks use exact source replacements. A public crates.io upload requires an approved fork/name/version strategy; these guarded manifests must not be published as-is.'},
        'runtime': {'archive': 'runtime/' + runtime_archive.name, 'sha256': runtime['archive_sha256'],
                    'server_and_helper_sha256': runtime['binary_sha256'], 'helper_profiles': runtime['helper_profiles']},
        'registry_account_ownership_verified': False, 'release_ready': False,
        'remaining_release_tasks': ['G6-070 full installed matrix', 'G6-101 required review findings', 'G6-102 release-ready package seal', 'G6-104 publication handoff', 'G6-105 actual owner acceptance']}
    (candidate / 'publication.json').write_text(json.dumps(publication, indent=2) + '\n')
    (candidate / 'README.md').write_text('''# Connect SDK candidate

This unpublished candidate contains versioned SDK archives, the exact matching
server/helper runtime, original licenses, standalone examples and offline build
inputs. See publication.json for every component identity and remaining release
work. Registry account rights and public Cargo fork/version resolution are not
approved by a successful local archive installation.

Extract runtime/*.tar.gz and put that runtime's bin directory first on PATH.
Use examples/sdk-installation/README.md from a separate working directory.
Node imports @rudeless/connect; Python installs rudeless-connect and imports
connect_sdk. Rust uses sdk/rust/.cargo/config.toml's checksummed offline source.

sources/ includes TypeScript/Python source, locked npm archives for both supported
platforms, and hash-locked Python build wheels. These reproduce the supplied SDK
distributions with cold offline caches. sdk/rust/ supplies original crate archives,
prepared upstream source/patch provenance and its standalone Cargo examples.

Installed read-only native package observations are recorded in verification/.
They do not qualify the complete version/facade/media matrix or Gate 6 acceptance.
No publication or release acceptance is authorized by this candidate.
''')
    files = inventory(candidate, normalized=True)
    manifest = {'format': 'connect/platform-sdk-candidate/1', 'source_revision': head, 'target': runtime['target'],
                'version': runtime['version'], 'files': files, 'publication_authorized': False,
                'candidate_installation_verified': True, 'release_ready': False, 'task_complete': False, 'whole_gate_qualified': False}
    (candidate / 'manifest.json').write_text(json.dumps(manifest, sort_keys=True, indent=2) + '\n')
    manifest_sha = digest(candidate / 'manifest.json')
    epoch = int(subprocess.check_output(['git', 'show', '-s', '--format=%ct', 'HEAD'], cwd=ROOT, text=True))
    archive = output / (identity + '.tar.gz')
    with archive.open('xb') as stream, gzip.GzipFile(fileobj=stream, mode='wb', filename='', mtime=epoch) as compressed, tarfile.open(fileobj=compressed, mode='w') as tar:
        # Emit each path as ordinary bytes even when immutable staging files
        # share an inode with a producer copy.
        for path in sorted(candidate.rglob('*')):
            if not path.is_file(): continue
            info = tarfile.TarInfo(identity + '/' + path.relative_to(candidate).as_posix())
            info.size = path.stat().st_size; info.mode = 0o755 if path.stat().st_mode & 0o111 else 0o644; info.mtime = epoch
            with path.open('rb') as content: tar.addfile(info, content)
    if inventory(candidate, normalized=True) != {**files, 'manifest.json': {'sha256': manifest_sha, 'mode': 0o644}}:
        raise ValueError('Candidate changed during archive sealing')
    if (source_inputs != runpy.run_path(str(ROOT / 'scripts/check-gate6-linux-hosting.py'))['sources']()
            or subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip() != head
            or subprocess.check_output(['git', 'status', '--porcelain'], cwd=ROOT, text=True).strip()):
        raise ValueError('Original source changed during candidate sealing')
    evidence.mkdir(parents=True, exist_ok=False)
    record = {'format': 'connect/sealed-platform-sdk-candidate/1', 'source_revision': head, 'target': runtime['target'],
              'archive_name': archive.name, 'archive_sha256': digest(archive), 'manifest_sha256': manifest_sha,
              'files': len(files), 'candidate_installation_verified': True, 'publication_authorized': False,
              'release_ready': False, 'task_complete': False, 'whole_gate_qualified': False}
    (evidence / 'summary.json').write_text(json.dumps(record, indent=2) + '\n')
    print(json.dumps(record))


if __name__ == '__main__':
    main()
