#!/usr/bin/env python3
"""Verify native asset ZIPs against original Git/prepared bytes and binary ABI."""
import argparse
import hashlib
import io
import json
from pathlib import Path, PurePosixPath
import re
import runpy
import subprocess
import sys
import tarfile
import zipfile

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('evidence', 'prepared', 'output'):
        parser.add_argument('--' + name, type=Path, required=True)
    package_input = parser.add_mutually_exclusive_group(required=True)
    package_input.add_argument('--package', type=Path)
    package_input.add_argument('--package-stdin', action='store_true', help='Verify a bounded ZIP in memory instead of allocating an extraction/download tree')
    parser.add_argument('--archives', type=Path)
    for name in ('evidence-sha256', 'package-sha256', 'revision'):
        parser.add_argument('--' + name, required=True)
    args = parser.parse_args()
    verifier = runpy.run_path(str(ROOT / 'scripts/verify-linux-installation.py'))
    with verifier['artifact'](args.evidence, args.evidence_sha256) as evidence:
        proof = verifier['json_member'](evidence, 'runtime.json')
        inputs_name, = [name for name in evidence.namelist() if PurePosixPath(name).name == 'source-inputs.json']
        source_bytes = evidence.read(inputs_name)
    if not (proof['passed'] and proof['source_inputs_unchanged'] and proof['build_exit_code'] == 0
            and proof['production_release'] and not proof['test_support'] and proof['source_revision'] == args.revision
            and hashlib.sha256(source_bytes).hexdigest() == proof['source_inputs_sha256']):
        raise ValueError('Native build proof is incomplete or mismatched')
    counts, _ = verifier['verify_sources'](json.loads(source_bytes), args.revision, args.prepared, args.archives)
    profiles = {}
    for name in ('helper.rs', 'helper/administrative.rs', 'helper/media.rs'):
        body = subprocess.check_output(['git', 'show', args.revision + ':crates/connect-client/src/sdk/' + name], cwd=ROOT, text=True)
        for key, value in re.findall(r'pub const (\w+): &str = "([^"]+)";', body):
            profiles[name + ':' + key] = value
    if proof['helper_profiles'] != profiles:
        raise ValueError('Helper profiles differ from original compiled source')
    identity = 'connect-runtime-' + proof['version'] + '-' + proof['target'] + '-' + args.revision[:12]
    files = {}; manifest = None; binary_header = None
    if args.package_stdin:
        body = sys.stdin.buffer.read(128 * 1024 * 1024 + 1)
        if len(body) > 128 * 1024 * 1024 or hashlib.sha256(body).hexdigest() != args.package_sha256:
            raise ValueError('Bounded native ZIP stdin digest differs')
        package_archive = zipfile.ZipFile(io.BytesIO(body))
    else:
        package_archive = verifier['artifact'](args.package, args.package_sha256)
    with package_archive as package:
        name, = package.namelist()
        if PurePosixPath(name).name != proof['archive_name']:
            raise ValueError('Native archive identity differs')
        with package.open(name) as stream:
            if hashlib.file_digest(stream, 'sha256').hexdigest() != proof['archive_sha256']:
                raise ValueError('Native tarball differs from proof')
        seen = set()
        with package.open(name) as stream, tarfile.open(fileobj=stream, mode='r|gz') as archive:
            for member in archive:
                path = PurePosixPath(member.name)
                if (path.is_absolute() or '..' in path.parts or path.as_posix() != member.name.rstrip('/')
                        or not path.parts or path.parts[0] != identity or path.as_posix() in seen):
                    raise ValueError('Unsafe or duplicate native archive member')
                seen.add(path.as_posix())
                if member.isdir(): continue
                if not member.isfile(): raise ValueError('Nonordinary native asset')
                relative = '/'.join(path.parts[1:])
                with archive.extractfile(member) as content:
                    if relative == 'manifest.json':
                        body = content.read()
                        if hashlib.sha256(body).hexdigest() != proof['manifest_sha256']:
                            raise ValueError('Native manifest differs from proof')
                        manifest = json.loads(body)
                    elif relative == 'bin/connect':
                        binary_header = content.read(64)
                        digest = hashlib.sha256(binary_header)
                        while block := content.read(1024 * 1024): digest.update(block)
                        files[relative] = {'sha256': digest.hexdigest(), 'mode': member.mode}
                    else:
                        files[relative] = {'sha256': hashlib.file_digest(content, 'sha256').hexdigest(), 'mode': member.mode}
    if manifest is not None and files != proof['files']:
        keys = set(files) | set(proof['files'])
        content = [name for name in sorted(keys) if files.get(name, {}).get('sha256') != proof['files'].get(name, {}).get('sha256')]
        modes = [name for name in sorted(keys) if files.get(name, {}).get('mode') != proof['files'].get(name, {}).get('mode')]
        args.output.write_text(json.dumps({'format': 'connect/native-sdk-runtime-payload-negative/1',
            'source_revision': args.revision, 'sources_verified': counts, 'files_read': len(files),
            'content_mismatch_count': len(content), 'content_mismatches': content,
            'mode_mismatch_count': len(modes), 'mode_mismatch_samples': modes[:10],
            'archive_sha256': proof['archive_sha256'], 'manifest_sha256': proof['manifest_sha256'],
            'binary_sha256': files.get('bin/connect', {}).get('sha256'),
            'artifact_zip_digests': {'evidence': args.evidence_sha256, 'package': args.package_sha256},
            'package_input': 'bounded-stdin' if args.package_stdin else 'retained-file',
            'passed': False, 'task_complete': False, 'whole_gate_qualified': False}, indent=2) + '\n')
        raise ValueError('Native payload manifest differs: ' + str(len(content)) + ' content, ' + str(len(modes)) + ' mode mismatches')
    if (manifest is None or files != proof['files'] or files != manifest['files']
            or manifest['source_revision'] != args.revision or manifest['target'] != proof['target']
            or manifest['helper_profiles'] != profiles or not manifest['production_build_verified']
            or len({files['bin/connect']['sha256'], proof['binary_sha256'], proof['completed_build_artifact_sha256'], manifest['binary_sha256']}) != 1):
        raise ValueError('Native file payload or identity differs')
    if proof['target'] == 'x86_64-unknown-linux-gnu':
        abi = binary_header[:6] == b'\x7fELF\x02\x01' and int.from_bytes(binary_header[18:20], 'little') == 62
    elif proof['target'] == 'aarch64-apple-darwin':
        abi = binary_header[:4] == b'\xcf\xfa\xed\xfe' and int.from_bytes(binary_header[4:8], 'little') == 0x0100000C
    else:
        abi = False
    expected_role = {'path': 'bin/connect', 'sha256': proof['binary_sha256']}
    if not abi or proof['roles'] != {'server': expected_role, 'sdk_helper': expected_role} or manifest['roles'] != proof['roles']:
        raise ValueError('Native ABI or server/helper role binding differs')
    report = {'format': 'connect/verified-native-sdk-runtime/1', 'source_revision': args.revision,
              'target': proof['target'], 'sources_verified': counts, 'files_verified': len(files),
              'binary_sha256': proof['binary_sha256'], 'helper_profiles': profiles,
              'archive_sha256': proof['archive_sha256'], 'manifest_sha256': proof['manifest_sha256'],
              'artifact_zip_digests': {'evidence': args.evidence_sha256, 'package': args.package_sha256},
              'passed': True, 'production_build_verified': True, 'sdk_installation_qualified': False,
              'package_input': 'bounded-stdin' if args.package_stdin else 'retained-file',
              'task_complete': False, 'whole_gate_qualified': False, 'publication_authorized': False}
    args.output.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({key: report[key] for key in ('passed', 'target', 'sources_verified', 'files_verified', 'binary_sha256')}))


if __name__ == '__main__':
    main()
