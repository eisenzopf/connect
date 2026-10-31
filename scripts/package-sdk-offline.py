#!/usr/bin/env python3
"""Ship pinned SDK source/build inputs and rebuild them using cold offline caches."""
import argparse
import base64
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import runpy
import shutil
import subprocess
import tarfile
import tempfile
import urllib.request
import venv
import zipfile

from lib.package_environment import fixture_environment
from lib.sdk_candidate import digest

ROOT = Path(__file__).resolve().parents[1]
TARGETS = {('darwin', 'arm64'), ('linux', 'x64')}


def npm_inputs(destination):
    lock = json.loads((ROOT / 'sdk/typescript/package-lock.json').read_text())
    archives = destination / 'npm-archives'; archives.mkdir()
    report = []
    for key, value in lock['packages'].items():
        if not key:
            continue
        if not any(os_name in value.get('os', [os_name]) and cpu in value.get('cpu', [cpu]) for os_name, cpu in TARGETS):
            continue
        name = key.split('node_modules/')[-1]
        url = value['resolved']
        if not url.startswith('https://registry.npmjs.org/') or '?' in url:
            raise ValueError('Noncanonical locked npm source URL')
        with urllib.request.urlopen(url, timeout=30) as response:
            body = response.read(64 * 1024 * 1024 + 1)
        algorithm, expected = value['integrity'].split('-', 1)
        if len(body) > 64 * 1024 * 1024 or algorithm not in ('sha512', 'sha256') or hashlib.new(algorithm, body).digest() != base64.b64decode(expected, validate=True):
            raise ValueError('Locked npm source integrity differs')
        notices = []; metadata = None
        with tarfile.open(fileobj=io.BytesIO(body), mode='r:gz') as archive:
            members = archive.getmembers()
            if len(members) > 10000 or sum(m.size for m in members) > 256 * 1024 * 1024:
                raise ValueError('npm source archive exceeds budget')
            for member in members:
                path = PurePosixPath(member.name)
                if path.is_absolute() or '..' in path.parts or not (member.isdir() or member.isfile()):
                    raise ValueError('Nonordinary npm source member')
                if len(path.parts) == 2 and member.isfile():
                    if path.name == 'package.json':
                        metadata = json.loads(archive.extractfile(member).read())
                    if path.name.upper().startswith(('LICENSE', 'LICENCE', 'NOTICE', 'COPYING')):
                        notices.append(path.name)
        if not metadata or metadata['name'] != name or metadata['version'] != value['version']:
            raise ValueError('Locked npm package identity differs')
        filename = name.removeprefix('@').replace('/', '-') + '-' + value['version'] + '.tgz'
        path = archives / filename
        path.write_bytes(body)
        report.append({'name': name, 'version': value['version'], 'file_name': filename,
                       'sha256': digest(path), 'integrity': value['integrity'], 'source_url': url,
                       'original_notice_members': notices, 'license': metadata.get('license')})
    return report


def tree_hashes(directory):
    return {p.relative_to(directory).as_posix(): digest(p) for p in directory.rglob('*') if p.is_file() and not p.is_symlink()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--evidence', type=Path, required=True)
    parser.add_argument('--wheel-proof', type=Path, required=True)
    args = parser.parse_args()
    output = args.output.resolve(); evidence = args.evidence.resolve()
    if not output.is_relative_to((ROOT / 'dist').resolve()) or not evidence.is_relative_to((ROOT / '.local').resolve()):
        parser.error('Use dist/ inputs and ignored .local/ evidence')
    if subprocess.check_output(['git', 'status', '--porcelain'], cwd=ROOT, text=True).strip():
        parser.error('Commit source before packaging')
    output.mkdir(parents=True, exist_ok=False); evidence.mkdir(parents=True, exist_ok=False)
    private = ROOT / '.local/gate6-sdk-offline/private' / evidence.name
    private.mkdir(parents=True, exist_ok=False)
    environment = fixture_environment()
    environment['SOURCE_DATE_EPOCH'] = subprocess.check_output(['git', 'show', '-s', '--format=%ct', 'HEAD'], cwd=ROOT, text=True).strip()
    bounded = runpy.run_path(str(ROOT / 'scripts/check-linux-package-build.py'))['run_bounded']
    report = {'format': 'connect/sdk-offline-build-inputs/1', 'passed': False, 'steps': [],
              'source_revision': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
              'supported_source_targets': ['darwin-arm64', 'linux-x64'], 'task_complete': False, 'whole_gate_qualified': False}
    sources = runpy.run_path(str(ROOT / 'scripts/check-gate6-linux-hosting.py'))['sources']
    before = sources()
    (evidence / 'source-inputs.json').write_text(json.dumps(before, sort_keys=True) + '\n')
    report['source_inputs_sha256'] = digest(evidence / 'source-inputs.json')
    def run(name, command, cwd=ROOT, budget=120):
        path = private / (name + '.log')
        status = bounded([str(c) for c in command], path, environment, budget, cwd=cwd)
        report['steps'].append({'name': name, 'exit_code': status, 'log_sha256': digest(path)})
        if status:
            raise ValueError('Offline source step failed: ' + name)
    try:
        report['npm_archives'] = npm_inputs(output)
        node_source = output / 'typescript-source'; node_source.mkdir()
        for name in ('package.json', 'package-lock.json', 'tsconfig.json', 'LICENSE', 'README.md'):
            shutil.copyfile(ROOT / 'sdk/typescript' / name, node_source / name)
        shutil.copytree(ROOT / 'sdk/typescript/src', node_source / 'src')
        python_source = output / 'python-source'; python_source.mkdir()
        for name in ('pyproject.toml', 'build-requirements.txt', 'LICENSE', 'README.md'):
            shutil.copyfile(ROOT / 'sdk/python' / name, python_source / name)
        shutil.copytree(ROOT / 'sdk/python/connect_sdk', python_source / 'connect_sdk', ignore=shutil.ignore_patterns('__pycache__', '*.pyc'))
        tools = output / 'python-build-tools'; tools.mkdir()
        run('download-locked-python-tools', ['python3', '-m', 'pip', 'download', '--disable-pip-version-check', '--no-cache-dir', '--no-deps',
            '--require-hashes', '--only-binary=:all:', '-r', python_source / 'build-requirements.txt', '--dest', tools])
        report['python_build_tools'] = []
        for path in sorted(tools.glob('*.whl')):
            with zipfile.ZipFile(path) as archive:
                notices = [name for name in archive.namelist() if PurePosixPath(name).name.upper().startswith(('LICENSE', 'LICENCE', 'COPYING', 'NOTICE'))]
                if not notices:
                    raise ValueError('Original Python build-tool license absent')
            report['python_build_tools'].append({'file_name': path.name, 'sha256': digest(path), 'original_notice_members': notices})
        if len(report['python_build_tools']) != 2:
            raise ValueError('Unexpected Python build-tool closure')
        with tempfile.TemporaryDirectory(prefix='connect-offline-sdk-source-') as directory:
            project = Path(directory).resolve()
            if project.is_relative_to(ROOT.resolve()):
                raise ValueError('Source rebuild must be outside repository')
            node = project / 'typescript'; shutil.copytree(node_source, node)
            cache = project / 'empty-npm-cache'
            if cache.exists():
                raise ValueError('Source cache was not empty')
            for index, package in enumerate(report['npm_archives']):
                run('npm-cache-' + str(index), ['npm', 'cache', 'add', '--offline', '--ignore-scripts', '--cache', cache, output / 'npm-archives' / package['file_name']], cwd=node)
            run('npm-offline-ci', ['npm', 'ci', '--offline', '--ignore-scripts', '--no-audit', '--no-fund', '--cache', cache], cwd=node)
            run('typescript-offline-build', ['npm', 'run', 'build', '--offline', '--cache', cache], cwd=node)
            if tree_hashes(node / 'dist') != tree_hashes(ROOT / 'sdk/typescript/dist'):
                raise ValueError('Offline TypeScript rebuild differs from candidate distribution')
            report['typescript_distribution_byte_identical'] = True
            # The build processes have joined. Release only this owned temporary
            # compiler/cache before the independent Python rebuild, keeping the
            # shipped archives and all private reports intact.
            shutil.rmtree(node)
            shutil.rmtree(cache)
            python = project / 'python'; shutil.copytree(python_source, python)
            target = project / 'venv'; venv.EnvBuilder(with_pip=False, symlinks=True).create(target)
            executable = target / 'bin/python'
            run('python-offline-builder-install', ['python3', '-m', 'pip', '--python', executable, 'install', '--disable-pip-version-check', '--no-cache-dir',
                '--no-index', '--no-deps', '--require-hashes', '--find-links', tools, '-r', python / 'build-requirements.txt'])
            wheels = project / 'wheels'; wheels.mkdir()
            run('python-offline-build', [executable, '-c', 'from setuptools.build_meta import build_wheel; build_wheel(' + repr(str(wheels)) + ')'], cwd=python)
            wheel, = wheels.glob('*.whl')
            proof = json.loads(args.wheel_proof.read_text())
            if not proof['passed'] or digest(wheel) != proof['wheel_sha256']:
                raise ValueError('Offline wheel rebuild differs from installed candidate')
            report['python_wheel_byte_identical'] = True
            report.update(passed=True, consumer_outside_repository=True, empty_caches=True,
                          typescript_distribution_byte_identical=True, python_wheel_byte_identical=True)
        files = tree_hashes(output)
        (output / 'manifest.json').write_text(json.dumps({'format': 'connect/sdk-offline-input-inventory/1',
            'source_revision': report['source_revision'], 'files': files}, sort_keys=True, indent=2) + '\n')
        report['manifest_sha256'] = digest(output / 'manifest.json')
    finally:
        report['source_inputs_unchanged'] = sources() == before
        report['passed'] = report['passed'] and report['source_inputs_unchanged']
        (evidence / 'summary.json').write_text(json.dumps(report, indent=2) + '\n')
        print(json.dumps({'passed': report['passed']}))
    if not report['passed']:
        raise SystemExit('Offline SDK source rebuild remains unqualified')


if __name__ == '__main__':
    main()
