#!/usr/bin/env python3
"""Build an unpublished production Linux binary/systemd archive from clean source."""
import argparse
import gzip
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import tarfile
import tomllib
from lib.package_environment import fixture_environment
from lib.package_inventory import copy_prepared_crate_notices
from lib.registry_licenses import copy_registry_notices, selected_dependency_ids

ROOT = Path(__file__).resolve().parents[1]
TARGET = 'x86_64-unknown-linux-gnu'


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def git(*args):
    return subprocess.check_output(['git', *args], cwd=ROOT, text=True).strip()


def archive(runtime, output, epoch):
    # Stable order, ownership, modes, mtimes and gzip header for identical inputs.
    with output.open('xb') as stream, gzip.GzipFile(fileobj=stream, mode='wb', filename='', mtime=epoch) as compressed, tarfile.open(fileobj=compressed, mode='w') as tar:
        for path in [runtime, *sorted(runtime.rglob('*'))]:
            if path.is_symlink() or not (path.is_file() or path.is_dir()):
                raise ValueError('Unexpected package file type')
            info = tar.gettarinfo(str(path), str(path.relative_to(runtime.parent)))
            info.uid = info.gid = 0
            info.uname = info.gname = 'root'
            info.mtime = epoch
            info.mode = 0o755 if path.is_dir() or str(path.relative_to(runtime)) == 'bin/connect' else 0o644
            if path.is_file():
                with path.open('rb') as content:
                    tar.addfile(info, content)
            else:
                tar.addfile(info)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, default=ROOT / 'dist')
    args = parser.parse_args()
    release = platform.freedesktop_os_release()
    if platform.system() != 'Linux' or platform.machine() != 'x86_64' or release.get('ID') != 'ubuntu' or release.get('VERSION_ID') != '24.04':
        parser.error('Build natively on Ubuntu 24.04 x86_64')
    if git('status', '--porcelain'):
        parser.error('Commit source before packaging; clean checkout required')
    revision = git('rev-parse', 'HEAD')
    epoch = int(git('show', '-s', '--format=%ct', revision))
    env = fixture_environment()
    env['SOURCE_DATE_EPOCH'] = str(epoch)
    os.environ.clear()
    os.environ.update(env)
    subprocess.run(['bash', 'scripts/prepare-dependencies.sh'], cwd=ROOT, check=True, timeout=900)
    subprocess.run(['cargo', 'build', '--locked', '--release', '-p', 'connect-server', '--bin', 'connect'], cwd=ROOT, check=True, timeout=5400)
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--locked', '--filter-platform', TARGET, '--format-version=1'], cwd=ROOT, timeout=120))
    if git('rev-parse', 'HEAD') != revision or git('status', '--porcelain'):
        raise ValueError('Source changed during build')
    binary = Path(metadata['target_directory']) / 'release/connect'
    version = tomllib.loads((ROOT / 'Cargo.toml').read_text())['workspace']['package']['version']
    identity = f'connect-{version}-ubuntu-24.04-x86_64-{revision[:12]}'
    output = args.output.resolve()
    if not output.is_relative_to((ROOT / 'dist').resolve()):
        parser.error('Packages belong under dist/')
    output.mkdir(parents=True, exist_ok=True)
    runtime = output / identity
    runtime.mkdir()
    def include(source, target, mode=0o644):
        destination = runtime / target
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(ROOT / source, destination)
        destination.chmod(mode)
    include(binary, 'bin/connect', 0o755)
    for source, target in [('deploy/linux/install.py', 'deploy/install.py'), ('deploy/linux/connect.service', 'deploy/connect.service'), ('docs/LINUX_INSTALL.md', 'README.md'), ('LICENSE', 'LICENSE'), ('THIRD_PARTY_NOTICES.md', 'THIRD_PARTY_NOTICES.md'), ('dependencies/source-lock.json', 'dependencies/source-lock.json')]:
        include(source, target)
    for name in ('rvoip', 'parley'):
        include(f'.deps/{name}/LICENSE', f'licenses/{name}-LICENSE')
    copy_prepared_crate_notices(ROOT, runtime, metadata)
    copy_registry_notices(ROOT, runtime, metadata, selected_dependency_ids(metadata, ['connect-server']))
    linkage = subprocess.check_output(['ldd', str(binary)], text=True)
    if 'not found' in linkage:
        raise ValueError('Unresolved build-host runtime library')
    (runtime / 'dependencies/ldd.txt').write_text(linkage)
    files = {str(p.relative_to(runtime)): {'sha256': digest(p), 'mode': 0o755 if p.relative_to(runtime).as_posix() == 'bin/connect' else 0o644} for p in sorted(runtime.rglob('*')) if p.is_file()}
    inputs = {name: digest(ROOT / name) for name in git('ls-files').splitlines() if (ROOT / name).is_file() and not (ROOT / name).is_symlink()}
    manifest = {'format': 'connect/linux-package/1', 'target': 'ubuntu-24.04-x86_64', 'installation_id': identity, 'source_revision': revision, 'source_epoch': epoch, 'build_profile': 'release', 'test_support': False, 'rustc': subprocess.check_output(['rustc', '-Vv'], text=True), 'inputs': inputs, 'files': files, 'publication_authorized': False}
    (runtime / 'manifest.json').write_text(json.dumps(manifest, indent=2, sort_keys=True) + '\n')
    if git('rev-parse', 'HEAD') != revision or git('status', '--porcelain') or any(digest(ROOT / name) != sha for name, sha in inputs.items()):
        raise ValueError('Source changed during packaging')
    target = output / (identity + '.tar.gz')
    archive(runtime, target, epoch)
    target.with_suffix(target.suffix + '.sha256').write_text(digest(target) + '  ' + target.name + '\n')
    print(target)


if __name__ == '__main__':
    main()
