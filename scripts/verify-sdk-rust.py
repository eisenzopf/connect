#!/usr/bin/env python3
"""Independently bind the raw Rust candidate ZIP and CI proof to original sources."""
import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import runpy
import subprocess
import tarfile
import tomllib
import zipfile

ROOT = Path(__file__).resolve().parents[1]


def sha(stream):
    return hashlib.file_digest(stream, 'sha256').hexdigest()


def checked_zip(path, expected):
    with path.open('rb') as stream:
        if sha(stream) != expected:
            raise ValueError('Raw GitHub artifact digest differs')
    return zipfile.ZipFile(path)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--evidence', type=Path, required=True)
    parser.add_argument('--evidence-sha256', required=True)
    parser.add_argument('--package', type=Path, required=True)
    parser.add_argument('--package-sha256', required=True)
    parser.add_argument('--revision', required=True)
    parser.add_argument('--prepared', type=Path, required=True)
    parser.add_argument('--archives', type=Path)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    verification = runpy.run_path(str(ROOT / 'scripts/verify-linux-installation.py'))
    with checked_zip(args.evidence, args.evidence_sha256) as proof:
        source_bytes = proof.read('source-inputs.json')
        sources = json.loads(source_bytes)
        manifest_bytes = proof.read('candidate-manifest.json')
        manifest = json.loads(manifest_bytes)
        external = json.loads(proof.read('external/summary.json'))
        archive_digest = proof.read('archive.sha256').decode().split()[0]
    if not (manifest['source_revision'] == external['source_revision'] == args.revision
            and external['candidate_manifest_sha256'] == hashlib.sha256(manifest_bytes).hexdigest()
            and external['passed'] and external['candidate_unchanged']
            and external['consumer_outside_repository'] and external['empty_cargo_home']
            and external['empty_target'] and external['cargo_offline']
            and [step['name'] for step in external['steps']] == ['resolve', 'metadata', 'check']
            and all(step['exit_code'] == 0 for step in external['steps'])):
        raise ValueError('External consumer proof is incomplete or mismatched')
    counts, git_hashes = verification['verify_sources'](sources, args.revision, args.prepared, args.archives)
    if manifest['inputs'] != git_hashes:
        raise ValueError('Package tracked source inventory differs from Git')
    lock = tomllib.loads(subprocess.check_output(['git', 'show', args.revision + ':Cargo.lock'], cwd=ROOT, text=True))
    registry = {(p['name'], p['version']): p['checksum'] for p in lock['package'] if p.get('source', '').startswith('registry+')}
    packages = manifest['prepared_packages'] + manifest['registry_packages']
    if len(packages) != external['extracted_archives']:
        raise ValueError('External archive count differs')
    for package in manifest['registry_packages']:
        if registry[(package['name'], package['version'])] != package['sha256']:
            raise ValueError('Registry archive is not the locked original')
    for package in manifest['prepared_packages']:
        base = PurePosixPath(package['manifest_source']).parent
        for name, expected in package['source_files'].items():
            path = str(base / name)
            if sources.get(path, {}).get('sha256') != expected:
                raise ValueError('Prepared package source is absent from verified inputs: ' + path)
    actual = {}; package_manifest = None
    with checked_zip(args.package, args.package_sha256) as package_zip:
        names = package_zip.namelist()
        if len(names) != 1 or not names[0].endswith('.tar.gz'):
            raise ValueError('Expected one complete offline candidate archive')
        with package_zip.open(names[0]) as stream:
            if sha(stream) != archive_digest:
                raise ValueError('Candidate tarball digest differs from CI proof')
        with package_zip.open(names[0]) as stream, tarfile.open(fileobj=stream, mode='r|gz') as archive:
            seen = set()
            for member in archive:
                path = PurePosixPath(member.name)
                if path.is_absolute() or '..' in path.parts or not path.parts or path.parts[0] != 'candidate' or member.name in seen:
                    raise ValueError('Invalid or duplicate candidate path')
                seen.add(member.name)
                if member.isdir():
                    continue
                if not member.isfile():
                    raise ValueError('Nonordinary candidate archive file')
                name = '/'.join(path.parts[1:])
                with archive.extractfile(member) as content:
                    if name == 'manifest.json':
                        package_manifest = content.read()
                    else:
                        actual[name] = sha(content)
    if package_manifest != manifest_bytes or actual != manifest['files']:
        raise ValueError('Actual candidate bytes differ from declared complete file inventory')
    for package in packages:
        if actual.get('archives/' + package['archive']) != package['sha256']:
            raise ValueError('Actual .crate archive differs')
    for package in manifest['prepared_packages']:
        prefix = 'vendor/' + package['name'] + '-' + package['version'] + '/'
        base = PurePosixPath(package['manifest_source']).parent
        if actual.get(prefix + 'Cargo.toml') != package['normalized_manifest_sha256']:
            raise ValueError('Prepared manifest differs')
        if actual.get(prefix + package['source_manifest_file']) != sources[str(base / 'Cargo.toml')]['sha256']:
            raise ValueError('Original Cargo manifest differs')
        for name, expected in package['source_files'].items():
            if name != 'Cargo.toml' and actual.get(prefix + name) != expected:
                raise ValueError('Actual prepared source payload differs: ' + prefix + name)
    report = {'format': 'connect/verified-rust-sdk-candidate/1', 'source_revision': args.revision,
              'sources_verified': counts, 'files_verified': len(actual),
              'prepared_archives_verified': len(manifest['prepared_packages']),
              'locked_registry_archives_verified': len(manifest['registry_packages']),
              'candidate_archive_sha256': archive_digest,
              'candidate_manifest_sha256': hashlib.sha256(manifest_bytes).hexdigest(),
              'artifact_zip_digests': {'evidence': args.evidence_sha256, 'package': args.package_sha256},
              'external_consumer': external, 'passed': True, 'task_complete': False,
              'whole_gate_qualified': False, 'publication_authorized': False}
    args.output.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({key: report[key] for key in ('passed', 'sources_verified', 'files_verified', 'prepared_archives_verified', 'locked_registry_archives_verified')}))


if __name__ == '__main__':
    main()
