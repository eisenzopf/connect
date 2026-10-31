#!/usr/bin/env python3
"""Independently bind Linux installation evidence to Git/prepared/package bytes.

Inputs are retained raw workflow artifact ZIPs and their GitHub-reported digests.
No archive is extracted and no packaged code or installation credential is run.
"""
import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import subprocess
import tarfile
import zipfile

ROOT = Path(__file__).resolve().parents[1]


def sha(stream):
    return hashlib.file_digest(stream, 'sha256').hexdigest()


def artifact(path, expected):
    with path.open('rb') as stream:
        if sha(stream) != expected.removeprefix('sha256:'):
            raise ValueError('Artifact ZIP differs from GitHub digest')
    return zipfile.ZipFile(path)


def json_member(archive, basename):
    names = [name for name in archive.namelist() if PurePosixPath(name).name == basename]
    if len(names) != 1 or archive.getinfo(names[0]).file_size > 16 * 1024 * 1024:
        raise ValueError('Missing, ambiguous or oversized evidence member')
    return json.loads(archive.read(names[0]))


def git_blobs(revision):
    result = {}
    for entry in subprocess.check_output(['git', 'ls-tree', '-rz', revision], cwd=ROOT).split(b'\0'):
        if not entry:
            continue
        header, name = entry.split(b'\t', 1)
        mode, kind, oid = header.split()
        if kind == b'blob' and mode in (b'100644', b'100755'):
            result[name.decode()] = (int(mode, 8) & 0o777, oid)
    return result


def verify_sources(inputs, revision, prepared, archives):
    expected = git_blobs(revision)
    if not expected.keys() <= inputs.keys():
        raise ValueError('Source report omits tracked Git files')
    lock = json.loads(subprocess.check_output(['git', 'show', revision + ':dependencies/source-lock.json'], cwd=ROOT))
    pinned = {name + '-' + value['version'] + '.crate': value['sha256'] for name, value in lock['sources'].items() if value.get('kind') == 'crate'}
    counts = {'git': 0, 'prepared': 0, 'pinned_archives': 0}
    git_hashes = {}
    with subprocess.Popen(['git', 'cat-file', '--batch'], cwd=ROOT, stdin=subprocess.PIPE, stdout=subprocess.PIPE) as child:
        for name, value in inputs.items():
            path = PurePosixPath(name)
            if path.is_absolute() or '..' in path.parts:
                raise ValueError('Unsafe source path')
            if name in expected:
                mode, oid = expected[name]
                child.stdin.write(oid + b'\n'); child.stdin.flush()
                header = child.stdout.readline().split()
                body = child.stdout.read(int(header[2]))
                assert child.stdout.read(1) == b'\n'
                actual = hashlib.sha256(body).hexdigest()
                git_hashes[name] = actual
                counts['git'] += 1
            else:
                if not name.startswith('.deps/'):
                    raise ValueError('Untracked source outside prepared upstream')
                source = prepared / name.removeprefix('.deps/')
                if name.startswith('.deps/.archives/'):
                    if value['sha256'] != pinned.get(path.name):
                        raise ValueError('Unpinned source archive')
                    if not source.exists() and archives:
                        source = archives / path.name
                    counts['pinned_archives'] += 1
                else:
                    counts['prepared'] += 1
                if source.is_symlink() or not source.is_file():
                    raise ValueError('Missing/nonordinary prepared source')
                mode = source.stat().st_mode & 0o777
                with source.open('rb') as stream:
                    actual = sha(stream)
            if actual != value['sha256'] or mode != value['mode']:
                raise ValueError('Source bytes/mode mismatch: ' + name)
        child.stdin.close()
        assert child.wait() == 0
    return counts, git_hashes


def verify_package(archive, revision, git_hashes):
    names = [name for name in archive.namelist() if name.endswith('.tar.gz')]
    if len(names) != 1:
        raise ValueError('Expected one Linux package archive')
    with archive.open(names[0]) as stream:
        archive_sha256 = sha(stream)
    files, manifest, root, epoch = {}, None, None, None
    with archive.open(names[0]) as stream, tarfile.open(fileobj=stream, mode='r|gz') as tar:
        for member in tar:
            path = PurePosixPath(member.name)
            if path.is_absolute() or '..' in path.parts or not path.parts:
                raise ValueError('Unsafe package archive path')
            if root is None:
                root = path.parts[0]
                epoch = member.mtime
            if path.parts[0] != root or member.uid != 0 or member.gid != 0 or member.uname != 'root' or member.gname != 'root' or member.mtime != epoch:
                raise ValueError('Archive root/ownership/time differs')
            if member.isdir():
                if member.mode != 0o755:
                    raise ValueError('Nontraversable package directory')
                continue
            if not member.isfile() or len(path.parts) < 2:
                raise ValueError('Package contains unsupported file type')
            relative = '/'.join(path.parts[1:])
            if relative in files:
                raise ValueError('Duplicate package file')
            mode = 0o755 if relative == 'bin/connect' else 0o644
            if member.mode != mode:
                raise ValueError('Unexpected package mode')
            with tar.extractfile(member) as content:
                if relative == 'manifest.json':
                    if member.size > 8 * 1024 * 1024:
                        raise ValueError('Oversized package manifest')
                    body = content.read()
                    manifest = json.loads(body)
                    digest = hashlib.sha256(body).hexdigest()
                else:
                    digest = sha(content)
            files[relative] = {'sha256': digest, 'mode': mode}
    if not manifest or manifest['format'] != 'connect/linux-package/1' or manifest['target'] != 'ubuntu-24.04-x86_64':
        raise ValueError('Wrong package profile')
    if manifest['installation_id'] != root or manifest['source_epoch'] != epoch or manifest['source_revision'] != revision:
        raise ValueError('Package source/identity mismatch')
    manifest_sha256 = files.pop('manifest.json')['sha256']
    if manifest['files'] != files or manifest['inputs'] != git_hashes:
        raise ValueError('Package inventory/source map mismatch')
    if manifest['build_profile'] != 'release' or manifest['test_support'] is not False or manifest['publication_authorized'] is not False:
        raise ValueError('Package is not the unpublished production release profile')
    required = {'bin/connect', 'deploy/connect.service', 'deploy/install.py', 'README.md', 'LICENSE', 'dependencies/source-lock.json'}
    if not required <= files.keys():
        raise ValueError('Incomplete runtime package')
    return manifest, {'archive_sha256': archive_sha256, 'manifest_sha256': manifest_sha256, 'binary_sha256': files['bin/connect']['sha256'], 'files_verified': len(files)}


def candidate(evidence_path, evidence_digest, package_path, package_digest, revision, prepared, archives):
    with artifact(evidence_path, evidence_digest) as evidence:
        inputs = json_member(evidence, 'source-inputs.json')
        report = json_member(evidence, 'build.json')
        if report['source_revision'] != revision or not report['passed'] or not report['source_inputs_unchanged'] or report['source_count'] != len(inputs):
            raise ValueError('Build was not a completed source-stable pass')
        source_name = next(name for name in evidence.namelist() if PurePosixPath(name).name == 'source-inputs.json')
        if hashlib.sha256(evidence.read(source_name)).hexdigest() != report['source_inputs_sha256']:
            raise ValueError('Source manifest digest mismatch')
        if report['requested_source_revision'] != revision or not report['controller_source_revision'] or len(report['controller_source_revision']) != 40:
            raise ValueError('Controller/checkout source provenance is incomplete')
        counts, git_hashes = verify_sources(inputs, revision, prepared, archives)
        if [step['name'] for step in report['steps']] != ['recovery-tests', 'package'] or any(step['exit_code'] != 0 for step in report['steps']):
            raise ValueError('Missing required build/recovery steps')
        results = report['steps'][0]['test_results']
        if len(results) != 1 or not results[0].startswith('test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out;'):
            raise ValueError('Recovery tests missing, failed, ignored or filtered')
        with artifact(package_path, package_digest) as package:
            manifest, proof = verify_package(package, revision, git_hashes)
        if proof['archive_sha256'] != report['archive_sha256'] or proof['manifest_sha256'] != report['package_manifest_sha256']:
            raise ValueError('Build report/package bytes disagree')
        installation = None
        if any(PurePosixPath(name).name == 'installation.json' for name in evidence.namelist()):
            installation = json_member(evidence, 'installation.json')
    return {'source_revision': revision, 'controller_source_revision': report['controller_source_revision'], 'sources_verified': counts, 'package': proof}, manifest, installation


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for prefix in ('initial', 'current'):
        for name in ('evidence', 'package'):
            parser.add_argument(f'--{prefix}-{name}', type=Path, required=True)
            parser.add_argument(f'--{prefix}-{name}-digest', required=True)
        parser.add_argument(f'--{prefix}-revision', required=True)
        parser.add_argument(f'--{prefix}-run-id', type=int, required=True)
    parser.add_argument('--prepared', type=Path, required=True)
    parser.add_argument('--pinned-archives', type=Path)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    proofs, manifests, installation = {}, {}, None
    for prefix in ('initial', 'current'):
        revision = subprocess.check_output(['git', 'rev-parse', getattr(args, prefix + '_revision')], cwd=ROOT, text=True).strip()
        proofs[prefix], manifests[prefix], observed = candidate(getattr(args, prefix + '_evidence'), getattr(args, prefix + '_evidence_digest'), getattr(args, prefix + '_package'), getattr(args, prefix + '_package_digest'), revision, args.prepared, args.pinned_archives)
        proofs[prefix]['workflow_run_id'] = getattr(args, prefix + '_run_id')
        proofs[prefix]['artifact_digests'] = {name + '_zip_sha256': getattr(args, prefix + '_' + name + '_digest').removeprefix('sha256:') for name in ('evidence', 'package')}
        if prefix == 'current': installation = observed
    initial, current = proofs['initial'], proofs['current']
    if not installation or not installation['passed'] or installation['not_run']:
        raise ValueError('Installation/upgrade/recovery is incomplete')
    required = {'fresh_install', 'signed_remote_profile_mutation_and_read', 'systemd_secret_injection', 'installed_binary_and_filesystem_isolation', 'graceful_stop', 'restart_retains_state_and_enrolled_credential', 'restart_preserves_configuration_and_secrets', 'upgrade_preserves_configuration_and_secrets', 'distinct_source_upgrade', 'restored_state_and_signing_trust_without_original_files', 'secret_injection_file_preserved', 'no_writable_source_tree_required'}
    if not required <= installation['checks'].keys() or any(installation['checks'][key] is not True for key in required):
        raise ValueError('Required installation behavior not proved')
    if installation['source_revision'] != initial['source_revision'] or installation['upgrade_source_revision'] != current['source_revision'] or initial['source_revision'] == current['source_revision']:
        raise ValueError('Upgrade did not use two distinct verified source revisions')
    for label, prefix in [('installed', 'initial'), ('upgraded', 'current')]:
        if installation[label + '_binary_sha256'] != proofs[prefix]['package']['binary_sha256']:
            raise ValueError('Observed installed binary differs from package')
    if installation['installed_binary_sha256'] == installation['upgraded_binary_sha256']:
        raise ValueError('Upgrade did not change the installed binary')
    if installation['package_manifest_sha256'] != initial['package']['manifest_sha256'] or installation['upgrade_manifest_sha256'] != current['package']['manifest_sha256']:
        raise ValueError('Installed package manifest mismatch')
    if installation['installer_source_revision'] != current['source_revision'] or installation['installer_sha256'] != manifests['current']['files']['deploy/install.py']['sha256']:
        raise ValueError('Installer provenance mismatch')
    with Path(__file__).open('rb') as stream:
        proofs['verifier_sha256'] = sha(stream)
    proofs['verifier_source_revision'] = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
    proofs.update(format='connect/verified-linux-installation/1', task='G6-056', installation=installation, whole_gate_qualified=False, deferred_qualification_dependencies=['G6-054'], scope='Ubuntu24.04 native x86_64 systemd; signed remote-profile peer in a separate local network namespace. No public WAN/provider/model/browser/media claim.')
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(proofs, indent=2) + '\n')
    print(json.dumps({'verified': True, 'initial': initial['source_revision'], 'current': current['source_revision'], 'checks': len(required)}))


if __name__ == '__main__':
    main()
