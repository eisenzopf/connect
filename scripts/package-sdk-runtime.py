#!/usr/bin/env python3
"""Build matching unpublished server/helper assets on each supported native host.

This produces platform assets without Gate 4/5 historical binary prerequisites.
Outside-repository SDK installation and behavior remain separate qualification.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import runpy
import shutil
import subprocess
import tomllib

from lib.package_environment import fixture_environment
from lib.package_inventory import copy_prepared_crate_notices
from lib.registry_licenses import copy_registry_notices, selected_dependency_ids
from lib.sdk_candidate import runtime_inventory
from lib.sdk_runtime_build import bundled_opus

ROOT = Path(__file__).resolve().parents[1]


def digest(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def git(*arguments):
    return subprocess.check_output(['git', *arguments], cwd=ROOT, text=True).strip()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--target', choices=('aarch64-apple-darwin', 'x86_64-unknown-linux-gnu'), required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--evidence', type=Path, required=True)
    args = parser.parse_args()
    if args.target == 'aarch64-apple-darwin':
        if platform.system() != 'Darwin' or platform.machine() != 'arm64':
            parser.error('Native macOS arm64 host required')
    else:
        release = platform.freedesktop_os_release()
        if platform.system() != 'Linux' or platform.machine() != 'x86_64' or release.get('ID') != 'ubuntu' or release.get('VERSION_ID') != '24.04':
            parser.error('Native Ubuntu24.04 x86_64 host required')
    output = args.output.resolve(); evidence = args.evidence.resolve()
    if not output.is_relative_to((ROOT / 'dist').resolve()) or not evidence.is_relative_to((ROOT / '.local').resolve()):
        parser.error('Use dist/ assets and ignored .local/ evidence')
    if git('status', '--porcelain'):
        parser.error('Commit source first; clean checkout required')
    os.umask(0o077)
    revision = git('rev-parse', 'HEAD')
    version = tomllib.loads((ROOT / 'Cargo.toml').read_text())['workspace']['package']['version']
    evidence.mkdir(parents=True, exist_ok=False)
    private = evidence / 'private'
    private.mkdir(parents=True, exist_ok=False)
    source = runpy.run_path(str(ROOT / 'scripts/check-gate6-linux-hosting.py'))['sources']
    before = source()
    (evidence / 'source-inputs.json').write_text(json.dumps(before, sort_keys=True) + '\n')
    environment = fixture_environment()
    environment['CARGO_PROFILE_RELEASE_DEBUG'] = '0'
    build_settings = {}
    if args.target == 'aarch64-apple-darwin':
        # Use the exact bundled/locked Opus source; do not acquire an unrecorded
        # Homebrew codec or ship a dependency on the build host's cellar paths.
        build_settings = {'MACOSX_DEPLOYMENT_TARGET': '15.0', 'OPUS_STATIC': '1',
                          'LIBOPUS_NO_PKG': '1', 'CMAKE_POLICY_VERSION_MINIMUM': '3.5'}
        environment.update(build_settings)
    bounded = runpy.run_path(str(ROOT / 'scripts/check-linux-package-build.py'))['run_bounded']
    log = private / 'build.log'
    status = bounded(['cargo', 'build', '--release', '--locked', '--message-format=json',
                      '-p', 'connect-server', '--bin', 'connect'], log, environment, 5400)
    record = {'format': 'connect/sdk-native-runtime-candidate/1', 'source_revision': revision,
              'target': args.target, 'version': version, 'build_exit_code': status,
              'build_settings': build_settings, 'platform': platform.platform(), 'passed': False,
              'sdk_installation_qualified': False, 'task_complete': False,
              'whole_gate_qualified': False, 'publication_authorized': False}
    record.update(source_inputs_sha256=digest(evidence / 'source-inputs.json'), source_count=len(before))
    try:
        if status:
            raise ValueError('Native runtime compilation failed; private compiler log retained')
        if args.target == 'aarch64-apple-darwin':
            # Requested flags do not prove that a restored native build obeyed
            # them. Cargo can reuse a prior pkg-config result from Homebrew.
            record['opus_native_build'] = bundled_opus(log)
        matches = []
        for line in log.read_text().splitlines():
            try:
                item = json.loads(line)
            except ValueError:
                continue
            if item.get('reason') == 'compiler-artifact' and item['target']['name'] == 'connect' and item.get('executable'):
                if item['profile']['test'] or item['profile']['opt_level'] != '3' or 'test-support' in item['features']:
                    raise ValueError('Native asset is not the production release profile')
                matches.append(Path(item['executable']))
        binary, = matches
        record['completed_build_artifact_sha256'] = digest(binary)
        metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--locked', '--offline',
            '--filter-platform', args.target, '--format-version', '1'], cwd=ROOT, env=environment, timeout=120))
        identity = 'connect-runtime-' + version + '-' + args.target + '-' + revision[:12]
        output.mkdir(parents=True, exist_ok=True)
        runtime = output / identity; runtime.mkdir()
        def include(source, name, mode=0o644):
            path = runtime / name; path.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source, path); path.chmod(mode)
        include(binary, 'bin/connect', 0o755)
        copied = runtime / 'bin/connect'
        if digest(copied) != digest(binary):
            raise ValueError('Running asset changed during copy')
        with copied.open('rb') as stream:
            header = stream.read(64)
        if args.target == 'aarch64-apple-darwin':
            if header[:4] != b'\xcf\xfa\xed\xfe' or int.from_bytes(header[4:8], 'little') != 0x0100000C:
                raise ValueError('Native asset is not an arm64 Mach-O executable')
            linkage = subprocess.check_output(['otool', '-L', str(copied)], text=True)
            libraries = [line.strip().split()[0] for line in linkage.splitlines()[1:]]
            if any(not name.startswith(('/System/Library/', '/usr/lib/')) for name in libraries):
                raise ValueError('Native asset retains a build-host library path')
        else:
            if header[:6] != b'\x7fELF\x02\x01' or int.from_bytes(header[18:20], 'little') != 62:
                raise ValueError('Native asset is not an x86_64 ELF executable')
            linkage = subprocess.check_output(['ldd', str(copied)], text=True)
            if 'not found' in linkage:
                raise ValueError('Native runtime library missing')
            include(Path('/usr/share/doc/libopus0/copyright'), 'licenses/system/libopus0-copyright')
        linkage = re.sub(r'\s+\(0x[0-9a-fA-F]+\)', '', linkage)
        linkage = linkage.replace(str(copied), 'bin/connect')
        (runtime / 'linkage.txt').write_text(linkage)
        for name in ('LICENSE', 'THIRD_PARTY_NOTICES.md', 'dependencies/source-lock.json', 'docs/LINUX_INSTALL.md'):
            include(ROOT / name, name)
        for name in ('rvoip', 'parley'):
            include(ROOT / '.deps' / name / 'LICENSE', 'licenses/' + name + '-LICENSE')
        copy_prepared_crate_notices(ROOT, runtime, metadata)
        copy_registry_notices(ROOT, runtime, metadata, selected_dependency_ids(metadata, ['connect-server']))
        profiles = {}
        for name in ('helper.rs', 'helper/administrative.rs', 'helper/media.rs'):
            path = ROOT / 'crates/connect-client/src/sdk' / name
            for key, value in re.findall(r'pub const (\w+): &str = "([^"]+)";', path.read_text()):
                profiles[name + ':' + key] = value
        binary_sha = digest(copied)
        record.update(binary_sha256=binary_sha, helper_profiles=profiles,
                      roles={'server': {'path': 'bin/connect', 'sha256': binary_sha},
                             'sdk_helper': {'path': 'bin/connect', 'sha256': binary_sha}},
                      production_release=True, test_support=False,
                      rustc=subprocess.check_output(['rustc', '-Vv'], env=environment, text=True))
        record['files'] = runtime_inventory(runtime)
        if source() != before:
            raise ValueError('Original sources changed before asset publication')
        asset_manifest = {key: value for key, value in record.items() if key != 'passed'}
        asset_manifest['production_build_verified'] = True
        (runtime / 'manifest.json').write_text(json.dumps(asset_manifest, indent=2, sort_keys=True) + '\n')
        archive = output / (identity + '.tar.gz')
        runpy.run_path(str(ROOT / 'scripts/package-linux.py'))['archive'](runtime, archive, int(git('show', '-s', '--format=%ct', revision)))
        record.update(archive_name=archive.name, archive_sha256=digest(archive), manifest_sha256=digest(runtime / 'manifest.json'))
        record['passed'] = True
    finally:
        record['source_inputs_unchanged'] = before == source() and git('rev-parse', 'HEAD') == revision and not git('status', '--porcelain')
        record['passed'] = record['passed'] and record['source_inputs_unchanged']
        if os.environ.get('GITHUB_OUTPUT') and record['source_inputs_unchanged'] and record.get('completed_build_artifact_sha256'):
            with Path(os.environ['GITHUB_OUTPUT']).open('a') as stream:
                stream.write('cacheable_build=true\n')
        (evidence / 'runtime.json').write_text(json.dumps(record, indent=2) + '\n')
        print(json.dumps({key: record[key] for key in ('source_revision', 'target', 'passed', 'build_exit_code')}))
    if not record['passed']:
        raise SystemExit('Native runtime asset remains unqualified')


if __name__ == '__main__':
    main()
