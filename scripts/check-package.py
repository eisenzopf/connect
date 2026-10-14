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
import sys
import tarfile
import tempfile
import tomllib
from lib.package_environment import fixture_environment

ROOT = pathlib.Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('candidate', type=pathlib.Path)
parser.add_argument('--codex-binary', type=pathlib.Path,
                    help='Explicit installed Codex for account-free Gate 4 host qualification')
parser.add_argument('--profile', choices=('gate4-local', 'gate5-local'), default='gate4-local',
                    help='Require this exact candidate profile; never infer a newer gate from legacy checks')
parser.add_argument('--inventory-only', action='store_true', help='Verify extracted archives and matching source; no builds or behavior qualification')
parser.add_argument('--independent-python', type=pathlib.Path, help='Prepared pinned aioquic environment for Gate5 installed control checks')
parser.add_argument('--node-binary', type=pathlib.Path, help='Explicit Node executable for Gate5 media')
parser.add_argument('--ffmpeg-binary', type=pathlib.Path, help='Explicit external FFmpeg codec for Gate5 media')
parser.add_argument('--chrome-binary', type=pathlib.Path, help='Explicit local Chrome executable for Gate5 media')
parser.add_argument('--playwright-module', type=pathlib.Path, help='Explicit prepared Playwright module for Gate5 media')
args = parser.parse_args()
if sys.flags.optimize:
    parser.error('Verification assertions must remain enabled')
candidate = args.candidate.resolve()
if args.independent_python is not None:
    args.independent_python = args.independent_python.resolve(strict=True)
    if not args.independent_python.is_file():
        parser.error('Selected independent Python executable is unavailable')
codex_binary = args.codex_binary.resolve() if args.codex_binary else None
media_tools = {'node': args.node_binary, 'ffmpeg': args.ffmpeg_binary,
               'chrome': args.chrome_binary, 'playwright': args.playwright_module}
if any(media_tools.values()):
    if not all(media_tools.values()):
        parser.error('Select all four media tool inputs together')
    media_tools = {name: path.resolve(strict=True) for name, path in media_tools.items()}
    if any(not path.is_file() for path in media_tools.values()):
        parser.error('Select ordinary media executable files and the Playwright module entry file (index.mjs)')
else:
    media_tools = None
if not args.inventory_only and (codex_binary is None or not codex_binary.is_file()):
    parser.error('Selected Codex executable is unavailable')
# Installed qualification uses local fixtures. Do not give inherited provider
# tokens or model settings to CLI, Cargo, SDK probes or the fixture server.
selected_environment = fixture_environment()
os.environ.clear()
os.environ.update(selected_environment)
os.umask(0o077)
results = ROOT/'.local/package-checks'/datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ')
results.mkdir(parents=True, mode=0o700)
report = {'expected_profile': args.profile, 'scope': 'local macOS arm64; no remote deployment', 'candidate': candidate.name, 'checks': {}, 'result': 'running'}
def save():
    (results/'result.json').write_text(json.dumps(report, indent=2)+'\n')
def passed(name, detail):
    report['checks'][name] = {'result': 'passed', 'detail': detail}
    save()
    print('PASS', name, flush=True)
def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def first_launch(name, binary, working_directory, flag='--version'):
    """Measure macOS loader assessment separately from application deadlines."""
    from lib.qualification_process import run_phase
    from lib.qualification_report import environment
    result = run_phase('first-launch-'+name, [binary, flag], 180,
                       results, working_directory, environment(codex_binary))
    report.setdefault('installation_startup', {})[name] = result
    save()
    if result['status'] != 'passed' or not (results/result['log']).read_text().strip():
        raise RuntimeError('Fresh executable startup failed: '+name)
    passed('first-launch-'+name, 'First launch within 180 seconds under unchanged macOS security policy, with owned-process cleanup; application deadlines remain unchanged')


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
                    assert not any(x in path.parts for x in ['.deps', '.local', '.git', 'target', 'node_modules', '__pycache__'])
                    if 'dist' in path.parts:
                        assert path.parts[:4] in [(candidate.name, 'sdk', 'typescript', 'dist'),
                                                  ('connect-source', 'sdk', 'typescript', 'dist')]
                    assert not member.name.endswith(('.sqlite3', '.der', '.log'))
                archive.extractall(work, filter='data')
        release = work/candidate.name
        manifest = json.loads((release/'manifest.json').read_text())
        assert manifest['candidate'] == candidate.name and manifest['target'] == 'aarch64-apple-darwin'
        assert manifest.get('qualification_profile') == args.profile, 'Candidate profile does not match explicit selection'
        # Helper imports cache default executable paths. Select the extracted
        # binaries before importing either the Gate 4 or legacy check helpers.
        os.environ.update(CONNECT_BINARY=str(release/'bin/connect'),
                          CONNECT_EXAMPLE_BINARY=str(release/'bin/connect-example'),
                          CONNECT_PREVIOUS_BINARY=str(work/'qualification/connect-schema1'))
        from lib.package_gate4 import inventory, qualify
        inventory(release)
        actual = {str(p.relative_to(release)) for p in release.rglob('*') if p.is_file()}
        assert actual == set(manifest['files']) | {'manifest.json'}
        for required in [
            'sdk/python/connect_sdk/native_audio.py',
            'sdk/typescript/dist/browser.js',
            'sdk/typescript/dist/native_audio.js',
            'examples/conversation-client/index.html',
            'examples/conversation-client/app.js',
            'examples/conversation-client/workflow.js',
            'docs/index.html',
            'docs/connect-pre-gate-4-review.html',
            'docs/connect-sms-eligibility.html',
            'docs/evidence/gate-3/acceptance.html',
            'docs/evidence/gate-3/result.json',
        ]:
            assert (release/required).is_file(), f'Missing Gate 3 runtime asset: {required}'
        assert '](docs/evidence/gate-3/acceptance.html)' in (release/'README.md').read_text()
        for link in re.findall(r'\]\(([^)]+)\)', (release/'README.md').read_text()):
            if not re.match(r'[a-zA-Z][a-zA-Z0-9+.-]*:|/|#', link):
                assert (release/link.split('#', 1)[0]).is_file(), f'Broken runtime README link: {link}'
        for name, checksum in manifest['files'].items():
            assert digest(release/name) == checksum
        for name in ['connect', 'connect-example'] + (['connect-video'] if args.profile == 'gate5-local' else []):
            binary = release/'bin'/name
            assert binary.stat().st_mode & 0o111
            assert b'CONNECT_TEST_FAULT_STAGE' not in binary.read_bytes()
            assert b'CONNECT_EXAMPLE_FAULT_STAGE' not in binary.read_bytes()
            assert b'CONNECT_CONNECTED_APP_FAULT' not in binary.read_bytes()
        prior = work/'qualification/connect-schema1'
        assert digest(prior) == json.loads(prior.with_name('manifest.json').read_text())['binary_sha256']
        source = work/'connect-source'
        assert digest(source/'Cargo.lock') == manifest['cargo_lock_sha256']
        assert json.loads((source/'dependencies/source-lock.json').read_text()) == manifest['dependencies']
        # Fault-enabled builds must correspond to the installed candidate source.
        for directory in ['crates', 'examples', 'scripts', 'patches', 'dependencies', 'sdk', 'contracts', 'skills', '.github']:
            for path in (source/directory).rglob('*'):
                if path.is_file():
                    assert digest(path) == digest(ROOT/path.relative_to(source)), 'Qualification source differs from candidate'
        if args.profile == 'gate5-local':
            from lib.package_inventory import inventory_gate5, verify_baseline
            report['gate5_inventory'] = inventory_gate5(release, source)
            report['historical_baselines'] = [verify_baseline(prior, json.loads((prior.with_name('manifest.json')).read_text()), source, 1),
                verify_baseline(work/'qualification/connect-schema16', json.loads((work/'qualification/schema16-manifest.json').read_text()), source, 16)]
            head = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
            assert manifest['source_revision'] == head, 'Gate5 source revision does not match qualification checkout'
            tracked = set(subprocess.check_output(['git', 'ls-tree', '-r', '--name-only', head], cwd=ROOT, text=True).splitlines())
            extracted = {str(path.relative_to(source)) for path in source.rglob('*') if path.is_file()}
            assert extracted == tracked, 'Gate5 complete source archive inventory mismatch'
            for name in tracked:
                assert digest(source/name) == digest(ROOT/name), 'Gate5 qualification input differs from complete candidate source: '+name
        passed('inventory', 'Runtime hashes; complete matching source; historical fixtures; no mutable state, keys or fault binaries')
        report['manifest'] = manifest
        if args.inventory_only:
            report.update(result='inventory-verified', whole_gate_qualified=False,
                          not_run=['compilation', 'installed behavior', 'historical upgrade', 'media', 'setup'])
            save()
            (candidate/'QUALIFICATION.json').write_text(json.dumps(report, indent=2)+'\n')
            print('PASS archive inventory only:', results/'result.json', flush=True)
            raise SystemExit(0)
        source_environment = dict(os.environ, CARGO_TARGET_DIR=str(ROOT/'target/package-source'))
        with (results/'sdk-source-build.log').open('w') as log:
            subprocess.run(['bash', 'scripts/prepare-dependencies.sh', '--cache', str(ROOT/'.deps')], cwd=source, env=source_environment,
                           stdout=log, stderr=subprocess.STDOUT, check=True, timeout=600)
            subprocess.run(['cargo', 'check', '--locked', '-p', 'connect-client'], cwd=source, env=source_environment,
                           stdout=log, stderr=subprocess.STDOUT, check=True, timeout=2400)
        passed('sdk-source-build', 'Public native SDK compiles from the extracted source archive and freshly prepared pinned dependencies')
        # Qualify the application's default diagnostics, independent of the
        # reviewer's shell-level logging overrides (this host uses warn).
        os.environ.pop('RUST_LOG', None)
        # Import only after choosing installed binaries, including default arguments.
        from lib.local_process import BINARY, Server, run, port, request
        from lib.examples import EXAMPLE, inspect, pair
        assert BINARY == (release/'bin/connect').resolve(), 'Server helper escaped the installed runtime'
        assert EXAMPLE == (release/'bin/connect-example').resolve(), 'Example helper escaped the installed runtime'
        # A freshly extracted Mach-O can wait in macOS security assessment
        # before main. Measure that installation step separately; subsequent
        # CLI, service and protocol deadlines retain their existing bounds.
        for name, binary in [('connect', BINARY), ('example', EXAMPLE), ('schema1', prior)]:
            first_launch(name, binary, work)
        if args.profile == 'gate5-local':
            first_launch('video', release/'bin/connect-video', work, '--help')
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
            assert health['transport'] == 'quic' and health['result']['data']['schema'] == manifest['schema']
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
        logs = re.sub(r'\x1b\[[0-9;]*m', '', logs)
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
        http_port = port()
        with Server(state, extra_args=['--http-bind', f'127.0.0.1:{http_port}']):
            sdk_env = dict(os.environ, PYTHONPATH=str(release/'sdk/python'),
                           CONNECT_TYPESCRIPT_SDK=str(release/'sdk/typescript/dist/index.js'))
            credential = tomllib.loads((state/'reviews.toml').read_text())['credential']
            with (results/'installed-language-sdks.log').open('w') as log:
                for executable, probe in [('python3', 'python_probe.py'), ('node', 'typescript_probe.mjs')]:
                    subprocess.run([executable, str(source/'sdk/tests'/probe), str(state/'reviews.toml'),
                                    f'http://127.0.0.1:{http_port}', str(BINARY)],
                                   input=json.dumps({'credential': credential}), text=True,
                                   env=sdk_env, stdout=log, stderr=subprocess.STDOUT, check=True, timeout=120)
                subprocess.run([
                    'node', '--input-type=module', '--eval',
                    "const sdk=await import(process.argv[1]); if(typeof sdk.BrowserVoiceSession!=='function')throw new Error('browser facade missing')",
                    (release/'sdk/typescript/dist/browser.js').as_uri(),
                ], stdout=log, stderr=subprocess.STDOUT, check=True, timeout=30)
        passed('installed-language-sdks', 'Packaged Python/TypeScript clients and installed native helper share authenticated QUIC/HTTP receipts, history and rejection behavior; browser facade imports independently')
        report['gate4_installed'] = qualify(release, source, codex_binary, results/'gate4-installed')
        passed('installed-gate4-agent', 'Packaged Python SDK and incoming app use the installed binary, protected MCP grants and actual Codex; source-bound reply, restart/replay and cancellation with no checkout runtime fallback')
        from lib.package_faults import prepare
        from lib.qualification_process import run_phase
        from lib.qualification_report import environment
        faults, report['qualification_fixtures'] = prepare(source, results/'qualification-fixtures', manifest)
        passed('qualification-fixture-build', 'Exact extracted source; owned compilation phase; test-only controls isolated from installed runtime binaries')
        for name in ['connect', 'connect-example']:
            first_launch('fault-'+name, faults/name, work)
        legacy_env = environment(codex_binary)
        legacy_env.update(CONNECT_BINARY=str(BINARY), CONNECT_EXAMPLE_BINARY=str(EXAMPLE),
                          CONNECT_TEST_BINARY=str(BINARY), PYTHONPATH=str(release/'sdk/python'),
                          CONNECT_PREVIOUS_BINARY=str(prior),
                          CONNECT_TEST_FAULT_BINARY=str(faults/'connect'),
                          CONNECT_EXAMPLE_FAULT_BINARY=str(faults/'connect-example'))
        for script in ['check-examples.py', 'check-recovery.py', 'check-crash-replay.py', 'check-example-recovery.py']:
            phase = run_phase(script, ['python3', str(source/'scripts'/script)], 600, results, work, legacy_env)
            report.setdefault('legacy_phases', []).append(phase)
            save()
            if phase['status'] != 'passed':
                raise RuntimeError('Installed behavior check failed: '+script)
            passed(script, 'Independent installed normal binaries; disposable local state; exact-source featured binaries only for abrupt faults')
        if args.profile == 'gate5-local':
            from lib.package_gate5 import qualify as qualify_gate5
            report['gate5_installed'] = qualify_gate5(release, source, manifest, results/'gate5-installed',
                args.independent_python, media_tools, {
                    'schema-one-binary': prior, 'schema-one-manifest': prior.with_name('manifest.json'),
                    'gate4-binary': work/'qualification/connect-schema16',
                    'gate4-manifest': work/'qualification/schema16-manifest.json'})
        # Behavior must not silently mutate the installed inputs it qualifies.
        final_files = {str(path.relative_to(release)) for path in release.rglob('*') if path.is_file()}
        assert final_files == set(manifest['files']) | {'manifest.json'}, 'Installed runtime inventory changed during behavior'
        for name, checksum in manifest['files'].items():
            path = release / name
            assert not path.is_symlink() and digest(path) == checksum, 'Installed runtime changed during behavior: ' + name
        report['runtime_inputs_stable'] = True
    report['result'] = report['gate5_installed']['status'] if args.profile == 'gate5-local' else 'passed'
    report['whole_gate_qualified'] = False
    report['coverage'] = 'retained installed baseline checks; Gate5 media/independent/setup additions require their own matching evidence'
    save()
    (candidate/'QUALIFICATION.json').write_text(json.dumps(report, indent=2)+'\n')
    print(report['result'].upper(), 'local package qualification:', results/'result.json', flush=True)
    if report['result'] == 'incomplete':
        raise SystemExit(2)
except SystemExit as error:
    if error.code in (0, 2):
        raise
    report.update(result='failed', error='SystemExit')
    save()
    raise
except BaseException as error:
    report.update(result='failed', error=type(error).__name__)
    save()
    (candidate/'QUALIFICATION.json').write_text(json.dumps(report, indent=2)+'\n')
    raise
