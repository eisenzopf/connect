#!/usr/bin/env python3
"""Resolve and build actual candidate .crate archives in a cold external project."""
import argparse
import json
import os
from pathlib import Path
import runpy
import shutil
import subprocess
import tempfile
import time

from lib.package_environment import fixture_environment
from lib.sdk_rust_package import install_archive, sha256

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--candidate', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    candidate = args.candidate.resolve(strict=True)
    output = args.output.resolve()
    if not candidate.is_relative_to((ROOT / 'dist').resolve()) or not output.is_relative_to((ROOT / '.local').resolve()):
        parser.error('Use dist/ candidates and private .local/ evidence')
    os.umask(0o077)
    output.mkdir(parents=True, exist_ok=False)
    private = ROOT / '.local/gate6-sdk-candidate/private' / output.name
    private.mkdir(parents=True, mode=0o700, exist_ok=False)
    manifest = json.loads((candidate / 'manifest.json').read_text())
    expected = manifest['files']
    actual = {p.relative_to(candidate).as_posix(): sha256(p) for p in candidate.rglob('*') if p.is_file() and p != candidate / 'manifest.json'}
    if expected != actual or any(p.is_symlink() for p in candidate.rglob('*')):
        raise ValueError('Candidate file inventory differs')
    report = {'format': 'connect/installed-rust-sdk-candidate/1', 'source_revision': manifest['source_revision'],
              'candidate_manifest_sha256': sha256(candidate / 'manifest.json'), 'passed': False,
              'scope': 'Actual registry-format archive extraction, cold external Cargo resolution and public SDK example check. No server/helper/runtime/platform-pair/full SDK qualification.',
              'steps': [], 'task_complete': False, 'whole_gate_qualified': False}
    bounded = runpy.run_path(str(ROOT / 'scripts/check-linux-package-build.py'))['run_bounded']
    try:
        with tempfile.TemporaryDirectory(prefix='connect-rust-installed-') as directory:
            project = Path(directory).resolve()
            if project.is_relative_to(ROOT.resolve()):
                raise ValueError('Consumer is inside the repository')
            vendor = project / 'vendor'; vendor.mkdir()
            packages = manifest['prepared_packages'] + manifest['registry_packages']
            for package in packages:
                install_archive(candidate / 'archives' / package['archive'], vendor,
                                package['name'], package['version'], package['sha256'])
            for name in ('.cargo', 'examples'):
                shutil.copytree(candidate / name, project / name)
            shutil.copyfile(candidate / 'rust-toolchain.toml', project / 'rust-toolchain.toml')
            home = project / 'cold-cargo-home'; target = project / 'cold-target'
            if home.exists() or target.exists():
                raise ValueError('Consumer cache is not empty')
            environment = fixture_environment()
            environment.update(CARGO_HOME=str(home), CARGO_TARGET_DIR=str(target))
            manifest_path = 'examples/rust-sdk-journey/Cargo.toml'
            for name, command, budget in [
                ('resolve', ['cargo', 'generate-lockfile', '--offline', '--manifest-path', manifest_path], 120),
                ('metadata', ['cargo', 'metadata', '--offline', '--locked', '--manifest-path', manifest_path, '--format-version', '1'], 120),
                ('check', ['cargo', 'check', '--frozen', '--manifest-path', manifest_path, '--message-format=json'], 2400),
            ]:
                start = time.monotonic()
                # The bounded runner deliberately owns the entire compiler group.
                log = private / (name + '.log')
                status = bounded(command, log, environment, budget, cwd=project)
                report['steps'].append({'name': name, 'exit_code': status, 'elapsed_seconds': round(time.monotonic() - start, 3), 'log_sha256': sha256(log)})
                if status:
                    raise ValueError('External Rust candidate step failed: ' + name)
                if name == 'metadata':
                    values = [json.loads(line) for line in log.read_text().splitlines() if line.startswith('{')]
                    metadata, = [value for value in values if isinstance(value.get('packages'), list)]
                    selected = next(p for p in metadata['packages'] if p['name'] == 'connect-client')
                    if selected['version'] != manifest['version'] or not selected['source'].startswith('registry+'):
                        raise ValueError('SDK did not resolve as an ordinary versioned package')
                    if any(d.get('path') for p in metadata['packages'] for d in p['dependencies']):
                        raise ValueError('Published candidate retains path dependencies')
                    report['resolved_packages'] = len(metadata['packages'])
                    report['resolved_sdk_source'] = selected['source']
            report.update(passed=True, consumer_outside_repository=True, empty_cargo_home=True,
                          empty_target=True, cargo_offline=True, extracted_archives=len(packages))
    finally:
        after = {p.relative_to(candidate).as_posix(): sha256(p) for p in candidate.rglob('*') if p.is_file() and p != candidate / 'manifest.json'}
        report['candidate_unchanged'] = after == expected
        if not report['candidate_unchanged']:
            report['passed'] = False
        (output / 'summary.json').write_text(json.dumps(report, indent=2) + '\n')
        print(json.dumps(report))
    if not report['passed']:
        raise SystemExit('Rust SDK candidate remains unqualified')


if __name__ == '__main__':
    main()
