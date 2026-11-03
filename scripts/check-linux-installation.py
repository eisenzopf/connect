#!/usr/bin/env python3
"""Disposable native Ubuntu/systemd package rehearsal; synthetic state stays private.

Requires root and an exact package directory. Optional distinct-source upgrade
package qualifies an actual upgrade; otherwise reports that case as unrun.
Never use on a host containing a real Connect installation.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import time
import tomllib

ROOT = Path(__file__).resolve().parents[1]
PRIVATE = ROOT / '.local/gate6-installation/private'
EVIDENCE = ROOT / '.local/gate6-installation/evidence'
NS = 'connect-install-fixture'
LINK = 'g6-install-host'
BINARY = Path('/opt/connect/current/bin/connect')
STATE = Path('/var/lib/connect/installation')
CONFIG = Path('/etc/connect/server.toml')


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def run(args, *, input=None, timeout=60, ok=True):
    result = subprocess.run([str(a) for a in args], input=input, capture_output=True, text=True, timeout=timeout)
    with (PRIVATE / 'commands.log').open('a') as stream:
        stream.write(json.dumps({'command': [str(a) for a in args], 'exit_code': result.returncode, 'stdout': result.stdout, 'stderr': result.stderr}) + '\n')
    if ok and result.returncode:
        raise RuntimeError('Fixture command failed; see protected local log')
    return result


def set_fields(path, values):
    raw = path.read_text()
    for key, value in values.items():
        raw, count = re.subn(r'^' + key + r' = .*$', lambda _: key + ' = ' + json.dumps(value), raw, flags=re.M)
        if count != 1:
            raise ValueError('Unexpected canonical configuration shape')
    path.write_text(raw)


def service(action):
    return run(['systemctl', action, 'connect.service'])


def start():
    before = run(['journalctl', '-u', 'connect.service', '-n', '0', '--show-cursor', '--no-pager']).stdout
    cursor = re.search(r'-- cursor: (.+)', before)
    service('start')
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        args = ['journalctl', '-u', 'connect.service', '--no-pager', '-o', 'cat']
        if cursor:
            args += ['--after-cursor', cursor[1]]
        logs = run(args).stdout
        if 'ready transport=quic' in logs and 'profile=connect/2' in logs:
            return
        if run(['systemctl', 'is-active', 'connect.service'], ok=False).stdout.strip() == 'failed':
            break
        time.sleep(.1)
    raise RuntimeError('Actual readiness not observed within 15 seconds')


def stop():
    pid = run(['systemctl', 'show', '--value', '-p', 'MainPID', 'connect.service']).stdout.strip()
    service('stop')
    result = run(['systemctl', 'show', 'connect.service', '-p', 'Result', '-p', 'ExecMainStatus', '-p', 'ActiveState']).stdout
    if 'Result=success' not in result or 'ExecMainStatus=0' not in result or 'ActiveState=inactive' not in result:
        raise RuntimeError('Service did not stop gracefully')
    if pid != '0' and Path('/proc', pid).exists():
        raise RuntimeError('Service PID survived stop')


def client_config(source, destination):
    shutil.copyfile(source, destination)
    destination.chmod(0o600)
    config = tomllib.loads(source.read_text())
    fields = {key: str(source.parent / config[key]) for key in ('trust_certificate', 'signing_private_key', 'server_public_key')}
    fields.update(network_profile='remote', endpoint='10.203.0.1:4433')
    set_fields(destination, fields)
    run(['chown', 'connect:connect', destination])


def request(config, kind, payload, operation=None):
    command = {'command': {'type': kind, 'payload': payload}, 'scope': {}}
    if operation:
        command['scope']['operation_id'] = operation
    result = run(['ip', 'netns', 'exec', NS, 'runuser', '-u', 'connect', '--', BINARY, 'request', '--config', config], input=json.dumps(command), timeout=20)
    value = json.loads(result.stdout)
    if value.get('kind') != 'canonical_reply':
        raise RuntimeError('Missing verified canonical reply')
    return value['data']['frames']


def list_rooms(config):
    frames = request(config, 'conversation.list', {'filter': {}, 'limit': 20})
    acknowledgements = [v for v in frames if v.get('type') == 'ack']
    if len(acknowledgements) != 1:
        raise RuntimeError('Missing canonical list acknowledgement')
    return acknowledgements[0]['payload']['details']['page']['conversations']



def fresh_current(package, report):
    """Independently initialize the current binary after removing fixture state."""
    manifest = json.loads((package / 'manifest.json').read_text())
    assert run(['systemctl', 'is-active', 'connect.service'], ok=False).stdout.strip() == 'inactive'
    unit = Path('/etc/systemd/system/connect.service')
    assert not unit.is_symlink() and digest(unit) == manifest['files']['deploy/connect.service']['sha256']
    # main() refuses all four pre-existing paths. Only this disposable fixture
    # created them, and the original installed service has already been joined.
    for path in (Path('/opt/connect'), Path('/etc/connect'), Path('/var/lib/connect')):
        assert path.is_dir() and not path.is_symlink()
        shutil.rmtree(path)
    unit.unlink()
    run(['systemctl', 'daemon-reload'])
    run(['python3', package / 'deploy/install.py', '--package', package, '--initialize'])
    run(['runuser', '-u', 'connect', '--', 'test', '-r', CONFIG])
    assert CONFIG.stat().st_mode & 0o777 == 0o640
    set_fields(CONFIG, {'network_profile': 'remote', 'bind': '10.203.0.1:4433'})
    client = Path('/var/lib/connect/current-fresh-client.toml')
    client_config(STATE / 'operator.toml', client)
    start()
    pid = run(['systemctl', 'show', '-p', 'MainPID', '--value', 'connect.service']).stdout.strip()
    executable = Path('/proc', pid, 'exe')
    assert run(['readlink', executable]).stdout.strip() == str(Path('/opt/connect/current').resolve() / 'bin/connect')
    assert str(ROOT) not in run(['readlink', Path('/proc', pid, 'cwd')]).stdout
    assert run(['nsenter', '-t', pid, '-m', '--', 'runuser', '-u', 'connect', '--', 'sh', '-c', 'test ! -w /opt/connect/current/bin/connect && test ! -w /etc/connect/server.toml && test -w /var/lib/connect && test ! -r "$1/Cargo.toml"', 'sh', str(ROOT)], ok=False).returncode == 0
    report['fresh_current_binary_sha256'] = digest(executable)
    assert report['fresh_current_binary_sha256'] == manifest['files']['bin/connect']['sha256']
    assert list_rooms(client) == []
    request(client, 'conversation.create', {'tenant_id': 'tnt_default', 'policy': 'persistent', 'initial_participants': [], 'metadata': {'fixture': 'current-fresh-installation'}}, 'installation-current-fresh-create')
    assert len(list_rooms(client)) == 1
    stop()
    report['fresh_current_source_revision'] = manifest['source_revision']
    report['fresh_current_manifest_sha256'] = digest(package / 'manifest.json')
    report['checks']['fresh_current_install'] = True


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--package', required=True, type=Path)
    parser.add_argument('--upgrade-package', type=Path)
    args = parser.parse_args()
    if os.geteuid() != 0:
        parser.error('Disposable root/systemd host required')
    if any(p.exists() or p.is_symlink() for p in (Path('/opt/connect'), Path('/etc/connect'), Path('/var/lib/connect'), Path('/etc/systemd/system/connect.service'))):
        parser.error('Refusing a host with existing Connect paths')
    os.umask(0o077)
    PRIVATE.mkdir(parents=True)
    EVIDENCE.mkdir(parents=True, exist_ok=True)
    package = args.package.resolve()
    manifest = json.loads((package / 'manifest.json').read_text())
    report = {'source_revision': manifest['source_revision'], 'package_manifest_sha256': digest(package / 'manifest.json'), 'scope': 'Disposable native Ubuntu24.04 ARM64 systemd host; remote-profile signed client in a separate local network namespace. No public/WAN/provider/model/browser/media claim.', 'checks': {}, 'not_run': [], 'passed': False}
    try:
        installer_package = (args.upgrade_package or package).resolve()
        installer_manifest = json.loads((installer_package / 'manifest.json').read_text())
        report['installer_source_revision'] = installer_manifest['source_revision']
        report['installer_sha256'] = digest(installer_package / 'deploy/install.py')
        run(['python3', installer_package / 'deploy/install.py', '--package', package, '--initialize'])
        report['installed_binary_sha256'] = digest(BINARY)
        assert report['installed_binary_sha256'] == manifest['files']['bin/connect']['sha256']
        run(['runuser', '-u', 'connect', '--', 'test', '-r', CONFIG])
        assert CONFIG.stat().st_mode & 0o777 == 0o640
        report['checks']['fresh_install'] = True
        set_fields(CONFIG, {'network_profile': 'remote', 'bind': '10.203.0.1:4433'})
        run(['ip', 'netns', 'add', NS])
        run(['ip', 'link', 'add', LINK, 'type', 'veth', 'peer', 'name', 'g6-install-peer'])
        run(['ip', 'link', 'set', 'g6-install-peer', 'netns', NS])
        run(['ip', 'addr', 'add', '10.203.0.1/30', 'dev', LINK])
        run(['ip', 'link', 'set', LINK, 'up'])
        run(['ip', '-n', NS, 'addr', 'add', '10.203.0.2/30', 'dev', 'g6-install-peer'])
        run(['ip', '-n', NS, 'link', 'set', 'g6-install-peer', 'up'])
        run(['ip', '-n', NS, 'link', 'set', 'lo', 'up'])
        environment = Path('/etc/connect/environment')
        environment.write_text('CONNECT_INSTALLATION_SYNTHETIC_SECRET=fixture-only-no-account\n')
        environment.chmod(0o600)
        config_hash, secret_hash = digest(CONFIG), digest(environment)
        client = Path('/var/lib/connect/remote-client.toml')
        client_config(STATE / 'operator.toml', client)
        start()
        assert list_rooms(client) == []
        request(client, 'conversation.create', {'tenant_id': 'tnt_default', 'policy': 'persistent', 'initial_participants': [], 'metadata': {'fixture': 'linux-installation'}}, 'installation-original-create')
        original_rooms = list_rooms(client)
        assert len(original_rooms) == 1
        report['checks']['signed_remote_profile_mutation_and_read'] = True
        pid = run(['systemctl', 'show', '-p', 'MainPID', '--value', 'connect.service']).stdout.strip()
        assert run(['readlink', Path('/proc', pid, 'exe')]).stdout.strip() == str(Path('/opt/connect/current').resolve() / 'bin/connect')
        assert str(ROOT) not in run(['readlink', Path('/proc', pid, 'cwd')]).stdout
        assert b'CONNECT_INSTALLATION_SYNTHETIC_SECRET=fixture-only-no-account' in Path('/proc', pid, 'environ').read_bytes().split(b'\0')
        report['checks']['systemd_secret_injection'] = True
        # The installed service sees the private namespace's filesystem policy.
        access = run(['nsenter', '-t', pid, '-m', '--', 'runuser', '-u', 'connect', '--', 'sh', '-c', 'test ! -w /opt/connect/current/bin/connect && test ! -w /etc/connect/server.toml && test -w /var/lib/connect && test ! -r "$1/Cargo.toml"', 'sh', str(ROOT)], ok=False)
        assert access.returncode == 0
        report['checks']['installed_binary_and_filesystem_isolation'] = True
        stop()
        report['checks']['graceful_stop'] = True
        start()
        assert list_rooms(client) == original_rooms
        stop()
        report['checks']['restart_retains_state_and_enrolled_credential'] = True
        assert digest(CONFIG) == config_hash and digest(environment) == secret_hash
        report['checks']['restart_preserves_configuration_and_secrets'] = True
        if args.upgrade_package:
            upgrade = args.upgrade_package.resolve()
            next_manifest = json.loads((upgrade / 'manifest.json').read_text())
            assert next_manifest['source_revision'] != manifest['source_revision']
            run(['python3', upgrade / 'deploy/install.py', '--package', upgrade])
            assert Path('/opt/connect/current').resolve().name == next_manifest['installation_id']
            assert digest(CONFIG) == config_hash and digest(environment) == secret_hash
            start(); assert list_rooms(client) == original_rooms; stop()
            report['checks']['upgrade_preserves_configuration_and_secrets'] = True
            report['upgraded_binary_sha256'] = digest(BINARY)
            assert report['upgraded_binary_sha256'] == next_manifest['files']['bin/connect']['sha256']
            assert report['upgraded_binary_sha256'] != report['installed_binary_sha256']
            report['checks']['distinct_source_upgrade'] = True
            report['upgrade_manifest_sha256'] = digest(upgrade / 'manifest.json')
            report['upgrade_source_revision'] = next_manifest['source_revision']
        else:
            report['not_run'].append('distinct-source upgrade: supply --upgrade-package from a second recorded source revision')
        backup = Path('/var/lib/connect/backup')
        run(['runuser', '-u', 'connect', '--', BINARY, 'backup', '--server-config', CONFIG, '--config', STATE / 'operator.toml', '--output', backup])
        assert (backup / 'manifest.json').is_file()
        restore = Path('/var/lib/connect/restored')
        run(['runuser', '-u', 'connect', '--', BINARY, 'restore', '--backup', backup, '--directory', restore])
        restored_config = tomllib.loads((restore / 'server.toml').read_text())
        shutil.copyfile(restore / 'server.toml', CONFIG)
        set_fields(CONFIG, {key: str(restore / restored_config[key]) for key in ('database', 'certificate', 'private_key', 'signing_private_key')})
        run(['chown', 'root:connect', CONFIG]); CONFIG.chmod(0o640)
        restored_client = Path('/var/lib/connect/restored-client.toml')
        client_config(restore / 'operator.toml', restored_client)
        # Removing the original and backup makes accidental key/config fallback fail.
        shutil.rmtree(STATE); shutil.rmtree(backup)
        start(); assert list_rooms(restored_client) == original_rooms; stop()
        report['checks']['restored_state_and_signing_trust_without_original_files'] = True
        assert secret_hash == digest(environment)
        report['checks']['secret_injection_file_preserved'] = True
        # Check preserved configuration before the deliberately different restore config.
        report['configuration_before_restore_sha256'] = config_hash
        report['checks']['no_writable_source_tree_required'] = True
        fresh_current((args.upgrade_package or package).resolve(), report)
        report['passed'] = not report['not_run']
    finally:
        run(['systemctl', 'stop', 'connect.service'], ok=False)
        run(['ip', 'netns', 'delete', NS], ok=False)
        run(['ip', 'link', 'delete', LINK], ok=False)
        report['whole_gate_qualified'] = False
        (EVIDENCE / 'installation.json').write_text(json.dumps(report, indent=2) + '\n')
    if not report['passed']:
        raise SystemExit('Partial installation checks completed; upgrade qualification remains unrun')


if __name__ == '__main__':
    main()
