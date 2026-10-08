#!/usr/bin/env python3
"""Install and qualify the three local candidate archives without shared private state."""
import argparse
import datetime
import hashlib
import json
import os
import pathlib
import re
import shutil
import socket
import subprocess
import tarfile
import tempfile
import tomllib

ROOT = pathlib.Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('candidate', type=pathlib.Path)
args = parser.parse_args()
candidate = args.candidate.resolve()
os.umask(0o077)
results = ROOT/'.local/package-checks'/datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ')
results.mkdir(parents=True, mode=0o700)
report = {'scope': 'local macOS arm64; no remote deployment', 'candidate': candidate.name, 'checks': {}, 'result': 'running'}
def save():
    (results/'result.json').write_text(json.dumps(report, indent=2)+'\n')
def passed(name, detail):
    report['checks'][name] = {'result': 'passed', 'detail': detail}
    save()
    print('PASS', name, flush=True)
def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

save()
try:
    checksums = {}
    for line in (candidate/'SHA256SUMS').read_text().splitlines():
        checksum, name = line.split('  ', 1)
        assert pathlib.Path(name).name == name and name not in checksums
        assert digest(candidate/name) == checksum, 'Archive checksum mismatch'
        checksums[name] = checksum
    expected = {candidate.name+suffix for suffix in ['.tar.gz', '-source.tar.gz', '-qualification.tar.gz']}
    assert set(checksums) == expected
    report['archives'] = checksums
    passed('archive-checksums', 'All three archives match SHA256SUMS')
    with tempfile.TemporaryDirectory(prefix='connect-installed-') as temporary:
        work = pathlib.Path(temporary)
        os.chdir(work)
        for name in checksums:
            with tarfile.open(candidate/name) as archive:
                for member in archive.getmembers():
                    path = pathlib.PurePosixPath(member.name)
                    assert not path.is_absolute() and '..' not in path.parts
                    assert member.isfile() or member.isdir(), 'Only ordinary files/directories allowed'
                    assert not any(x in path.parts for x in ['.deps', '.local', '.git', 'target', 'dist'])
                    assert not member.name.endswith(('.sqlite3', '.der', '.log'))
                archive.extractall(work, filter='data')
        release = work/candidate.name
        manifest = json.loads((release/'manifest.json').read_text())
        assert manifest['candidate'] == candidate.name and manifest['target'] == 'aarch64-apple-darwin'
        actual = {str(p.relative_to(release)) for p in release.rglob('*') if p.is_file()}
        assert actual == set(manifest['files']) | {'manifest.json'}
        for name, checksum in manifest['files'].items():
            assert digest(release/name) == checksum
        for name in ['connect', 'connect-example']:
            binary = release/'bin'/name
            assert binary.stat().st_mode & 0o111
            assert b'CONNECT_TEST_FAULT_STAGE' not in binary.read_bytes()
            assert b'CONNECT_EXAMPLE_FAULT_STAGE' not in binary.read_bytes()
        prior = work/'qualification/connect-schema1'
        assert digest(prior) == json.loads(prior.with_name('manifest.json').read_text())['binary_sha256']
        source = work/'connect-source'
        assert digest(source/'Cargo.lock') == manifest['cargo_lock_sha256']
        assert json.loads((source/'dependencies/source-lock.json').read_text()) == manifest['dependencies']
        # Fault-enabled builds must correspond to the installed candidate source.
        for directory in ['crates', 'examples', 'scripts', 'patches', 'dependencies']:
            for path in (source/directory).rglob('*'):
                if path.is_file():
                    assert digest(path) == digest(ROOT/path.relative_to(source)), 'Qualification source differs from candidate'
        passed('inventory', 'Runtime hashes; clean full SDK source; separate historical binary; no mutable state, keys or fault binaries')
        report['manifest'] = manifest
        source_environment = dict(os.environ, CARGO_TARGET_DIR=str(ROOT/'target/package-source'))
        with (results/'sdk-source-build.log').open('w') as log:
            subprocess.run(['bash', 'scripts/prepare-dependencies.sh', '--cache', str(ROOT/'.deps')], cwd=source, env=source_environment,
                           stdout=log, stderr=subprocess.STDOUT, check=True, timeout=600)
            subprocess.run(['cargo', 'check', '--locked', '-p', 'connect-client'], cwd=source, env=source_environment,
                           stdout=log, stderr=subprocess.STDOUT, check=True, timeout=2400)
        passed('sdk-source-build', 'Public native SDK compiles from the extracted source archive and freshly prepared pinned dependencies')
        os.environ.update(CONNECT_BINARY=str(release/'bin/connect'), CONNECT_EXAMPLE_BINARY=str(release/'bin/connect-example'), CONNECT_PREVIOUS_BINARY=str(prior))
        # Import only after choosing installed binaries, including default arguments.
        from lib.local_process import BINARY, Server, run, port, request
        from lib.examples import EXAMPLE, inspect, pair
        assert manifest['version'] in run('--version').stdout
        state = work/'state'
        init = run('init', '--directory', state, '--example', 'review-loop', '--bind', f'127.0.0.1:{port()}')
        before = (state/'agent.toml').read_bytes()
        run('init', '--directory', state, '--example', 'review-loop')
        assert (state/'agent.toml').read_bytes() == before
        agent = tomllib.loads((state/'agent.toml').read_text())['identity_id']
        canary = 'private-message-canary-DO-NOT-LOG'
        logical = 'private-request-canary'
        secrets = [tomllib.loads((state/(role+'.toml')).read_text())['credential'] for role in ['operator', 'reviews', 'agent']]
        logs = init.stdout+init.stderr
        with Server(state) as server:
            health = json.loads(run('health', '--config', state/'operator.toml').stdout)
            assert health['transport'] == 'quic' and health['result']['data']['schema'] == 2
            run('diagnostics', '--config', state/'operator.toml')
            pair(state)
            assert json.loads(run('demo', 'verify', '--config', state/'operator.toml').stdout)['result'] == 'passed'
            created = request(state, 'reviews', {'op': 'create', 'request_id': 'log-create', 'participants': [agent]})['data']['id']
            sent = run('request', '--config', state/'reviews.toml', value={'op': 'send', 'request_id': logical, 'conversation_id': created, 'body': canary})
            logs += sent.stderr  # stdout is an intentional API result, not a diagnostic log.
            server.log.seek(0)
            logs += server.log.read()
        assert canary not in logs and logical not in logs
        assert all(secret not in logs for secret in secrets)
        assert 'PRIVATE KEY' not in logs
        assert 'application operation completed' in logs and 'operation="send"' in logs
        assert hashlib.sha256(logical.encode()).hexdigest()[:16] in logs
        for path in state.rglob('*'):
            assert path.stat().st_mode & 0o077 == 0, 'Private state has group/world permissions'
        passed('installed-walkthrough', 'Fresh init preservation; version; ready QUIC health; separate app/agent; bounded demo; protected files; safe hashed operation logs')
        with Server(state):
            assert json.loads(run('demo', 'verify', '--config', state/'operator.toml').stdout)['result'] == 'passed'
            assert request(state, 'reviews', {'op': 'request_status', 'request_id': logical})['data']['state'] == 'accepted'
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
            endpoint = tomllib.loads((state/'server.toml').read_text())['bind']
            sock.bind(('127.0.0.1', int(endpoint.rsplit(':', 1)[1])))
        passed('restart-shutdown', 'Installed restart preserves receipts and example effects; SIGTERM drain releases loopback port')
        for script in ['check-examples.py', 'check-recovery.py', 'check-crash-replay.py', 'check-example-recovery.py']:
            with (results/(script+'.log')).open('w') as log:
                subprocess.run(['python3', str(ROOT/'scripts'/script)], cwd=work, stdout=log, stderr=subprocess.STDOUT, check=True, timeout=600)
            passed(script, 'Independent installed normal binaries; disposable local state; exact-source featured binaries only for abrupt faults')
    report['result'] = 'passed'
    save()
    print('PASS local package qualification:', results/'result.json', flush=True)
except BaseException as error:
    report.update(result='failed', error=type(error).__name__)
    save()
    raise
