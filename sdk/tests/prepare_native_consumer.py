#!/usr/bin/env python3
"""Pack and install the current Node consumer locally; never publish or connect."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[2]


def digest(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def prepare(output, package_output):
    output, package_output = Path(output).absolute(), Path(package_output).absolute()
    assert output.resolve().is_relative_to((ROOT / '.local').resolve())
    assert package_output.resolve().is_relative_to((ROOT / 'dist').resolve())
    output.mkdir(mode=0o700, parents=True, exist_ok=False)
    package_output.mkdir(parents=True, exist_ok=False)
    environment = dict(os.environ, NPM_CONFIG_LOGS_DIR=str(output / 'npm-logs'))
    sdk = ROOT / 'sdk/typescript'
    # Caller builds with locked dependencies before the source snapshot. This
    # operation preserves those exact JS/declarations and runs no lifecycle code.
    files = {str(p.relative_to(sdk / 'dist')): digest(p)
             for p in (sdk / 'dist').rglob('*') if p.is_file()}
    assert files and 'index.js' in files
    with (output / 'pack.json').open('w') as stream:
        subprocess.run(['npm', 'pack', '--ignore-scripts', '--json', '--pack-destination', str(package_output)],
                       cwd=sdk, env=environment, stdout=stream, check=True, timeout=60)
    archive = package_output / 'rudeless-connect-0.1.0.tgz'
    project = output / 'project'
    project.mkdir(mode=0o700)
    (project / 'package.json').write_text('{"private":true,"type":"module"}\n')
    with (output / 'install.log').open('w') as stream:
        subprocess.run(['npm', 'install', '--offline', '--ignore-scripts', '--omit=dev', str(archive)],
                       cwd=project, env=environment, stdout=stream, stderr=subprocess.STDOUT,
                       check=True, timeout=60)
    installed = project / 'node_modules/@rudeless/connect/dist'
    observed = {str(p.relative_to(installed)): digest(p)
                for p in installed.rglob('*') if p.is_file()}
    assert observed == files, 'Installed distribution differs from current build'
    consumer = project / 'server-consume.mjs'
    shutil.copyfile(ROOT / 'sdk/tests/node_sdk_server_consumer.mjs', consumer)
    consultation = project / 'consultation-consume.mjs'
    shutil.copyfile(ROOT / 'sdk/tests/node_consultation_server_consumer.mjs', consultation)
    movement = project / 'movement-consume.mjs'
    shutil.copyfile(ROOT / 'sdk/tests/node_movement_server_consumer.mjs', movement)
    result = {'movement_consumer': str(movement.relative_to(ROOT)),
              'movement_consumer_sha256': digest(movement), 'consultation_consumer': str(consultation.relative_to(ROOT)),
              'consultation_consumer_sha256': digest(consultation), 'archive': str(archive.relative_to(ROOT)), 'archive_sha256': digest(archive),
              'installed_dist_matches': True, 'installed_dist_sha256': observed,
              'consumer': str(consumer.relative_to(ROOT)), 'consumer_sha256': digest(consumer),
              'node_version': subprocess.check_output(['node', '--version'], text=True).strip(),
              'publication': False}
    (output / 'package-result.json').write_text(json.dumps(result, indent=2) + '\n')
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--package-output', type=Path, required=True)
    args = parser.parse_args()
    print(json.dumps(prepare(args.output, args.package_output)))
