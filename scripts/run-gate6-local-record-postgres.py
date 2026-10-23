#!/usr/bin/env python3
"""Run one compiled record fixture in a fresh bounded local Docker database.

Never uses DATABASE_URL from the parent or connects to an existing database.
Never pulls an image or changes an existing container/network.
"""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import time
import uuid

ROOT = Path(__file__).resolve().parents[1]

def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('executable', type=Path)
    parser.add_argument('--evidence-dir', type=Path, required=True)
    args = parser.parse_args()
    executable = args.executable.resolve()
    directory = args.evidence_dir.resolve()
    assert executable.parent == (ROOT / 'target/debug/deps').resolve() and executable.is_file()
    assert directory.is_relative_to(ROOT / '.local/gate-6') and not directory.exists()
    directory.mkdir(parents=True)
    def call(command, timeout=15):
        result = subprocess.run(command, capture_output=True, text=True, timeout=timeout)
        if result.returncode:
            raise RuntimeError(f'fixture command failed: {command[0]} {command[1]}')
        return result.stdout.strip()
    context = call(['docker', 'context', 'show'])
    endpoint = json.loads(call(['docker', 'context', 'inspect', '--format', '{{json .Endpoints.docker.Host}}', context]))
    assert endpoint.startswith('unix://'), 'A local Docker context is required'
    image = json.loads(call(['docker', 'image', 'inspect', 'postgres:16-alpine']))[0]
    image_id = image['Id']
    case = 'tests::live_put_get_delete_list_and_hash'
    inventory = call([str(executable), '--list', '--format', 'terse'])
    assert case + ': test' in inventory, 'live fixture was not compiled'
    original_binary = digest(executable)
    name = 'connect-g6-records-' + uuid.uuid4().hex[:12]
    network_id = None
    container_id = None
    configuration = dict(name=name, image_id=image_id, image_os=image['Os'], image_architecture=image['Architecture'],
                         bind='127.0.0.1', memory_bytes=268435456, cpus=1, pids_limit=128,
                         tmpfs_bytes=167772160, isolated_network=True, internal_network=False, no_image_pull=True,
                         test_timeout_seconds=90, startup_timeout_seconds=30,
                         source_inputs_sha256={relative: digest(ROOT / relative) for relative in (
                             'scripts/run-gate6-local-record-postgres.py', 'dependencies/source-lock.json',
                             'patches/rvoip/gate6-conversation-records.patch',
                             '.deps/rvoip/crates/extensions/rvoip-vcon-postgres/src/lib.rs',
                             '.deps/rvoip/crates/extensions/rvoip-vcon-postgres/migrations/0001_vcon_store.sql')})
    (directory / 'configuration.json').write_text(json.dumps(configuration, indent=2) + '\n')
    passed = False
    result = None
    cleanup = {}
    try:
        network_id = call(['docker', 'network', 'create', '--driver', 'bridge', '--label', 'connect.gate6.fixture=record-postgres', name])
        container_id = call(['docker', 'run', '--detach', '--pull=never', '--name', name,
                             '--network', name, '--label', 'connect.gate6.fixture=record-postgres',
                             '--memory=256m', '--cpus=1', '--pids-limit=128',
                             '--tmpfs', '/var/lib/postgresql/data:rw,noexec,nosuid,size=160m',
                             '-e', 'POSTGRES_USER=connect_fixture', '-e', 'POSTGRES_DB=connect_fixture',
                             '-e', 'POSTGRES_HOST_AUTH_METHOD=trust', '-p', '127.0.0.1::5432',
                             image_id, '-c', 'shared_buffers=16MB', '-c', 'max_connections=12',
                             '-c', 'max_wal_size=64MB'], timeout=30)
        deadline = time.monotonic() + 30
        while True:
            ready = subprocess.run(['docker', 'exec', container_id, 'pg_isready', '-h', '127.0.0.1', '-U', 'connect_fixture', '-d', 'connect_fixture', '-t', '1'], capture_output=True, timeout=5)
            if ready.returncode == 0:
                break
            if time.monotonic() >= deadline:
                raise RuntimeError('isolated fixture startup timed out')
            time.sleep(0.2)
        address = call(['docker', 'port', container_id, '5432/tcp'])
        assert re.fullmatch(r'127\.0\.0\.1:\d+', address)
        env = dict(os.environ)
        env['DATABASE_URL'] = 'postgresql://connect_fixture@' + address + '/connect_fixture'
        env['CARGO_MANIFEST_DIR'] = str(ROOT / '.deps/rvoip/crates/extensions/rvoip-vcon-postgres')
        command = [str(executable), '--ignored', '--exact', case, '--test-threads=1']
        print(json.dumps(dict(step='owned-local-postgres-ready', image_id=image_id)), flush=True)
        with (directory / 'test.log').open('w') as output:
            result = subprocess.run(command, cwd=ROOT, env=env, stdout=output, stderr=subprocess.STDOUT, timeout=90)
        names = re.findall(r'^test (.+?) \.\.\. ok$', (directory / 'test.log').read_text(), re.M)
        passed = result.returncode == 0 and names == [case] and digest(executable) == original_binary
        configuration['server_version'] = call(['docker', 'exec', container_id, 'postgres', '--version'])
    finally:
        # Reconcile a create command whose response was lost using its original
        # unique name and fixture label, never by creating a replacement.
        if not container_id:
            found = subprocess.run(['docker', 'container', 'inspect', name], capture_output=True, text=True, timeout=10)
            if found.returncode == 0:
                candidate = json.loads(found.stdout)[0]
                if candidate['Config'].get('Labels', {}).get('connect.gate6.fixture') == 'record-postgres':
                    container_id = candidate['Id']
        if not network_id:
            found = subprocess.run(['docker', 'network', 'inspect', name], capture_output=True, text=True, timeout=10)
            if found.returncode == 0:
                candidate = json.loads(found.stdout)[0]
                if candidate.get('Labels', {}).get('connect.gate6.fixture') == 'record-postgres':
                    network_id = candidate['Id']
        if container_id:
            logs = subprocess.run(['docker', 'logs', container_id], capture_output=True, text=True, timeout=10)
            (directory / 'database.log').write_text(logs.stdout + logs.stderr)
            remove = subprocess.run(['docker', 'rm', '--force', container_id], capture_output=True, text=True, timeout=15)
            cleanup['container_removed'] = remove.returncode == 0
        if network_id:
            remove = subprocess.run(['docker', 'network', 'rm', network_id], capture_output=True, text=True, timeout=15)
            cleanup['network_removed'] = remove.returncode == 0
        unchanged = all(digest(ROOT / relative) == sha for relative, sha in configuration['source_inputs_sha256'].items())
        report = dict(task='G6-027', lane='owned isolated loopback Docker Postgres',
                      passed=passed and unchanged and cleanup == {'container_removed': True, 'network_removed': True},
                      full_task_qualification=False, configuration=configuration, cleanup=cleanup,
                      executable=str(executable), executable_sha256=original_binary,
                      tests_passed=[case] if passed else [], exit_code=result.returncode if result else None,
                      sources_unchanged=unchanged, executed_at=datetime.datetime.now(datetime.timezone.utc).isoformat(),
                      not_qualified=['Existing/remote database, Linux x86_64 hosting, provider deletion, production Connect policy, full G6-027 and Gate 6 acceptance.'])
        (directory / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
        print(json.dumps(dict(passed=report['passed'], report=str(directory / 'report.json'), cleanup=cleanup)), flush=True)
    if not passed or not report['passed']:
        raise SystemExit(1)

if __name__ == '__main__':
    main()
