#!/usr/bin/env python3
"""Install a verified Connect package on Ubuntu 24.04 x86_64; never start it.

Run as root from a trusted, digest-checked extracted package. Upgrade requires a
stopped service and preserves configuration/state. Publication is a separate act.
"""
import argparse
import hashlib
import grp
import json
import os
from pathlib import Path, PurePosixPath
import platform
import pwd
import re
import shutil
import stat
import subprocess
import tempfile
import tomllib


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def verified_files(package):
    manifest_path = package / 'manifest.json'
    if manifest_path.is_symlink() or not manifest_path.is_file() or manifest_path.stat().st_size > 8 * 1024 * 1024:
        raise ValueError('Invalid package manifest')
    manifest = json.loads(manifest_path.read_text())
    if manifest.get('format') != 'connect/linux-package/1' or manifest.get('target') != 'ubuntu-24.04-x86_64':
        raise ValueError('Unsupported package format/target')
    if not re.fullmatch(r'connect-[a-zA-Z0-9._-]+', manifest['installation_id']):
        raise ValueError('Unsafe installation ID')
    files = manifest['files']
    if not {'bin/connect', 'deploy/connect.service', 'deploy/install.py', 'README.md', 'LICENSE'} <= files.keys():
        raise ValueError('Incomplete package')
    for name, entry in files.items():
        relative = PurePosixPath(name)
        if relative.is_absolute() or not relative.parts or any(p in ('.', '..') for p in relative.parts) or str(relative) != name:
            raise ValueError('Unsafe package path')
        path = package
        for part in relative.parts:
            path = path / part
            if path.is_symlink():
                raise ValueError('Symlink package material')
        if not path.is_file() or digest(path) != entry['sha256'] or entry['mode'] not in (0o644, 0o755):
            raise ValueError('Package file integrity failure')
    actual = {str(p.relative_to(package)) for p in package.rglob('*') if p.is_file() and p != package / 'manifest.json'}
    if actual != set(files) or any(p.is_symlink() for p in package.rglob('*')):
        raise ValueError('Unlisted package material')
    return manifest


def directory(path, mode=0o755, uid=0, gid=0):
    if path.is_symlink():
        raise ValueError('Installation directories must not be symlinks')
    path.mkdir(mode=mode, parents=True, exist_ok=True)
    if not path.is_dir() or path.stat().st_uid not in (0, uid):
        raise ValueError('Unexpected installation directory owner')
    os.chown(path, uid, gid)
    path.chmod(mode)


def check_platform():
    release = platform.freedesktop_os_release()
    if os.geteuid() != 0 or platform.machine() != 'x86_64' or release.get('ID') != 'ubuntu' or release.get('VERSION_ID') != '24.04':
        raise ValueError('Root on Ubuntu 24.04 x86_64 required')
    if not Path('/run/systemd/system').is_dir():
        raise ValueError('A running systemd host is required')


def stopped():
    result = subprocess.run(['systemctl', 'is-active', 'connect.service'], capture_output=True, text=True)
    if result.stdout.strip() not in ('inactive', 'failed', 'unknown'):
        raise ValueError('Stop connect.service and confirm it is inactive before installing')


def account():
    try:
        user = pwd.getpwnam('connect')
    except KeyError:
        subprocess.run(['useradd', '--system', '--user-group', '--home-dir', '/var/lib/connect', '--shell', '/usr/sbin/nologin', 'connect'], check=True)
        user = pwd.getpwnam('connect')
    if user.pw_uid >= 1000 or user.pw_uid == 0 or user.pw_gid == 0 or user.pw_gid != grp.getgrnam('connect').gr_gid or user.pw_shell not in ('/usr/sbin/nologin', '/sbin/nologin'):
        raise ValueError('connect must be a dedicated unprivileged system account')
    return user


def initialize(binary, bind):
    state = Path('/var/lib/connect/installation')
    config = Path('/etc/connect/server.toml')
    if state.exists() or state.is_symlink() or config.exists() or config.is_symlink():
        raise ValueError('Initialization requires a fresh installation and configuration')
    subprocess.run(['runuser', '-u', 'connect', '--', str(binary), 'init', '--directory', str(state), '--bind', bind], check=True)
    raw = (state / 'server.toml').read_text()
    values = tomllib.loads(raw)
    for key in ('database', 'certificate', 'private_key', 'signing_private_key'):
        value = str(state / values[key])
        raw, count = re.subn(r'^' + key + r' = .*$', lambda _: key + ' = ' + json.dumps(value), raw, flags=re.M)
        if count != 1:
            raise ValueError('Unexpected canonical initialization output; preserve state for review')
    # Exclusive publication; never overwrite operator configuration on upgrade.
    fd = os.open(config, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o640)
    with os.fdopen(fd, 'w') as stream:
        stream.write(raw)
        stream.flush()
        os.fsync(stream.fileno())
    os.chown(config, 0, pwd.getpwnam('connect').pw_gid)


def seal_directories(stage):
    for path in sorted((p for p in stage.rglob('*') if p.is_dir()), reverse=True) + [stage]:
        if path.is_symlink():
            raise ValueError('Unexpected symlink in verified stage')
        path.chmod(0o755)
        fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(fd)
        finally:
            os.close(fd)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--package', type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument('--initialize', action='store_true', help='Create new loopback state and canonical configuration; refuses existing state')
    parser.add_argument('--bind', default='127.0.0.1:4433', help='Fresh loopback initialization only')
    args = parser.parse_args()
    os.umask(0o077)
    check_platform()
    package = args.package.resolve()
    manifest = verified_files(package)
    stopped()
    user = account()
    for path in (Path('/opt/connect'), Path('/opt/connect/releases')):
        directory(path)
    directory(Path('/etc/connect'), 0o750, 0, user.pw_gid)
    directory(Path('/var/lib/connect'), 0o700, user.pw_uid, user.pw_gid)
    releases = Path('/opt/connect/releases')
    destination = releases / manifest['installation_id']
    if destination.exists() or destination.is_symlink():
        raise ValueError('This release already exists; preserve it and select a new source revision')
    stage = Path(tempfile.mkdtemp(prefix='.stage-', dir=releases))
    try:
        for name in manifest['files']:
            target = stage / name
            target.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
            shutil.copyfile(package / name, target, follow_symlinks=False)
        raw_manifest = (package / 'manifest.json').read_bytes()
        if json.loads(raw_manifest) != manifest:
            raise ValueError('Package manifest changed during installation')
        (stage / 'manifest.json').write_bytes(raw_manifest)
        verified_files(stage)
        # Only verified assets become world-readable/executable. Keep staging
        # private while copying, including with an operator's restrictive umask.
        for name, entry in manifest['files'].items():
            target = stage / name
            target.chmod(entry['mode'])
            with target.open('rb') as stream:
                os.fsync(stream.fileno())
        (stage / 'manifest.json').chmod(0o644)
        with (stage / 'manifest.json').open('rb') as stream:
            os.fsync(stream.fileno())
        seal_directories(stage)
        links = subprocess.run(['ldd', str(stage / 'bin/connect')], capture_output=True, text=True, check=True)
        if 'not found' in links.stdout:
            raise ValueError('Missing runtime libraries; install the documented prerequisites')
        os.rename(stage, destination)
        fd = os.open(releases, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(fd)
        finally:
            os.close(fd)
    finally:
        if stage.exists():
            shutil.rmtree(stage)
    if args.initialize:
        initialize(destination / 'bin/connect', args.bind)
    current = Path('/opt/connect/current')
    if current.exists() and not current.is_symlink():
        raise ValueError('Refusing to replace a non-symlink current installation')
    temporary = Path('/opt/connect/.current-new')
    if temporary.exists() or temporary.is_symlink():
        raise ValueError('Unexpected pending activation; inspect before continuing')
    temporary.symlink_to(destination)
    os.replace(temporary, current)
    fd = os.open(current.parent, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)
    unit = Path('/etc/systemd/system/connect.service')
    if unit.is_symlink():
        raise ValueError('Service unit must not be a symlink')
    shutil.copyfile(destination / 'deploy/connect.service', unit)
    unit.chmod(0o644)
    subprocess.run(['systemctl', 'daemon-reload'], check=True)
    print('Installed', manifest['installation_id'], '(stopped). Review /etc/connect/server.toml, then systemctl enable --now connect.service and verify authenticated readiness.')


if __name__ == '__main__':
    main()
