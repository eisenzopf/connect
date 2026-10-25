#!/usr/bin/env python3
"""Isolated native profile and owned Ubuntu guest rehearsal; never a live-provider lane."""
import argparse
import datetime
import json
import os
from pathlib import Path
import re
import runpy
import signal
import subprocess
import time
import tomllib

ROOT = Path(__file__).resolve().parents[1]
HELPERS = runpy.run_path(str(ROOT / 'scripts/rehearse-gate6-fifty-participants.py'))
PREFIX = 'canonical_native_connections::remote::'
SIP = 'canonical_native_connections::consultation::groups::replacement::telephone::initial::crash::'
CASES = [
    ('migration-eddsa', PREFIX + 'production_remote_profile_ca_chain_and_native_duplex_survive_real_nat_rebinding_eddsa'),
    ('migration-es256', PREFIX + 'production_remote_profile_ca_chain_and_native_duplex_survive_real_nat_rebinding_es256'),
    ('identity-root', PREFIX + 'production_remote_profile_refuses_wrong_host_and_unconfigured_roots_before_commands'),
    ('intermediate', PREFIX + 'production_remote_profile_requires_its_complete_intermediate_chain'),
    ('trust-assets', PREFIX + 'production_remote_profile_missing_invalid_and_oversized_trust_never_opens_commands'),
    ('credentials', PREFIX + 'production_remote_profile_refuses_expired_and_revoked_credentials_without_media'),
    ('open-deadline', PREFIX + 'production_remote_profile_one_open_deadline_spans_delayed_challenge_and_stalled_auth'),
    ('expired-tls', PREFIX + 'production_remote_profile_expired_tls_is_refused_by_server_and_actual_client_verifier'),
    ('sip-operator-eddsa', SIP + 'production_sigkill_sip_submission_keeps_external_uncertainty_without_redial_eddsa'),
    ('sip-operator-es256', SIP + 'production_sigkill_sip_submission_keeps_external_uncertainty_without_redial_es256'),
]
DRAINS = [
    ('http-drain', 'canonical_record_http::actual_http_shutdown_cancels_stalled_record_io_before_runtime_retirement'),
    ('https-drain', 'canonical_record_http::actual_https_reference_expiry_and_shutdown_abort_stalled_tls_and_release_guards'),
]


def digest(path):
    return HELPERS['digest'](path)


def run(command, log, env, timeout=None, stdout=None):
    with log.open('w') as output:
        process = subprocess.Popen(command, cwd=ROOT, env=env, stdout=stdout or output,
                                   stderr=output, start_new_session=True)
        try:
            return process.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait()
            return 124


def build(target, target_dir, out, env):
    with (out / f'{target}.jsonl').open('w') as artifacts:
        command = ['cargo', 'test', '-p', 'connect-server', '--test', target, '--no-run',
                   '--release', '--locked', '--offline', '-j4', '--target-dir', str(target_dir),
                   '--message-format=json']
        assert run(command, out / f'{target}.build.log', env, stdout=artifacts) == 0
    rows = [json.loads(line) for line in (out / f'{target}.jsonl').read_text().splitlines()]
    found = [r for r in rows if r.get('reason') == 'compiler-artifact'
             and r.get('target', {}).get('name') == target and r.get('executable')]
    assert len(found) == 1 and found[0]['profile']['opt_level'] == '3'
    return Path(found[0]['executable'])


def cases(binary, selected, out, env):
    listed = subprocess.check_output([str(binary), '--list'], text=True, cwd=ROOT)
    names = {line.removesuffix(': test') for line in listed.splitlines() if line.endswith(': test')}
    assert all(case in names for _, case in selected), 'Selected method is absent; zero tests is not a pass'
    results = []
    for name, case in selected:
        log = out / f'{name}.log'
        code = run([str(binary), case, '--exact', '--test-threads=1', '--nocapture'], log, env, 180)
        raw = log.read_text()
        passed = code == 0 and bool(re.search(r'test result: ok\. 1 passed; 0 failed; 0 ignored;', raw))
        row = dict(name=name, case=case, exit_code=code, passed=passed, log_sha256=digest(log))
        if name.startswith('migration') and passed:
            witnesses = [json.loads(line.split('native-remote-witness ', 1)[1])
                         for line in raw.splitlines() if 'native-remote-witness ' in line]
            assert len(witnesses) == 1
            witness = witnesses[0]
            assert witness['nat_paths'] == 4 and witness['logical_bindings_preserved']
            assert witness['decoded_directions_before_and_after'] == 4
            row['witness'] = witness
        results.append(row)
        print(json.dumps(row), flush=True)
    return results


def guest_build(image, target, out, env):
    metadata = json.loads(subprocess.check_output(['docker', 'image', 'inspect', image]))[0]
    assert metadata['Os'] == 'linux' and metadata['Architecture'] == 'amd64'
    script = out / 'guest-build.sh'
    script.write_text('''#!/usr/bin/env bash
set -euo pipefail
rustc -Vv
python3 --version
git --version
uname -srm
cat /etc/os-release
dpkg-query -W > /target/packages.tsv
bash scripts/prepare-dependencies.sh
cargo build -p connect-client --example native_remote_probe --locked --offline --target-dir /target -j2 --message-format=json > /target/build.jsonl
''')
    version = tomllib.loads((ROOT / 'rust-toolchain.toml').read_text())['toolchain']['channel']
    command = ['docker', 'run', '--rm', '--read-only', '--network', 'none', '--platform', 'linux/amd64',
               '--cpus', '2', '--memory', '4g', '--tmpfs', '/tmp:rw,size=1g',
               '--tmpfs', '/task-cargo:rw,size=64m', '--env', 'CARGO_HOME=/task-cargo',
               '--env', f'RUSTUP_TOOLCHAIN={version}-x86_64-unknown-linux-gnu',
               '--env', 'CC=clang', '--env', 'CXX=clang++', '--env', 'CARGO_INCREMENTAL=0',
               '--env', 'CARGO_PROFILE_DEV_DEBUG=0', '--workdir', str(ROOT)]
    cargo_home = Path(os.environ.get('CARGO_HOME', Path.home() / '.cargo'))
    for source, destination, readonly in [(ROOT, ROOT, True), ((ROOT / '.deps').resolve(), (ROOT / '.deps').resolve(), True),
                                          (cargo_home / 'registry', Path('/task-cargo/registry'), True),
                                          (target, Path('/target'), False)]:
        command += ['--mount', f'type=bind,source={source},target={destination}' + (',readonly' if readonly else '')]
    command += [image, 'bash', str(script)]
    assert run(command, out / 'guest-build.log', env) == 0
    binary = target / 'debug/examples/native_remote_probe'
    assert binary.is_file()
    return binary, dict(image_id=metadata['Id'], image_os=metadata['Os'], image_architecture=metadata['Architecture'],
                        packages_sha256=digest(target / 'packages.tsv'), log_sha256=digest(out / 'guest-build.log'))


def distinct(binary, guest, image, out, env):
    results = []
    for algorithm in ['EdDSA', 'ES256']:
        fixture = out / algorithm
        fixture.mkdir(mode=0o700)
        child_env = dict(env, CONNECT_G6_DISTINCT_NATIVE_DIR=str(fixture),
                         CONNECT_G6_DISTINCT_NATIVE_ALGORITHM=algorithm, TMPDIR=str(fixture))
        log = fixture / 'server.log'
        with log.open('w') as output:
            process = subprocess.Popen([str(binary), PREFIX + 'distinct_host_server_child', '--exact',
                                        '--ignored', '--test-threads=1', '--nocapture'], cwd=ROOT, env=child_env,
                                       stdout=output, stderr=subprocess.STDOUT, start_new_session=True)
            try:
                started = time.monotonic()
                while not (fixture / 'ready.json').exists():
                    assert process.poll() is None, 'Owned server fixture ended before readiness'
                    assert time.monotonic() - started < 60, 'Bounded server fixture setup expired'
                    time.sleep(.02)
                command = ['docker', 'run', '--rm', '--cidfile', str(fixture / 'container.id'), '--read-only',
                           '--platform', 'linux/amd64', '--cpus', '2', '--memory', '1g',
                           '--mount', f'type=bind,source={guest.parent},target=/probe,readonly',
                           '--mount', f'type=bind,source={fixture},target=/fixture,readonly', image,
                           '/probe/native_remote_probe', '/fixture/client.toml', 'host.lima.internal']
                try:
                    guest_code = run(command, fixture / 'guest.log', env, 120)
                finally:
                    if (fixture / 'container.id').exists():
                        cid = (fixture / 'container.id').read_text().strip()
                        assert re.fullmatch('[0-9a-f]{64}', cid)
                        subprocess.run(['docker', 'rm', '--force', cid], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                (fixture / 'done').write_text('complete\n')
                server_code = process.wait(timeout=30)
            finally:
                if process.poll() is None:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait()
        guest_raw = (fixture / 'guest.log').read_text()
        server_raw = log.read_text()
        client = [json.loads(line) for line in guest_raw.splitlines() if line.startswith('{')]
        server = [json.loads(line.split('distinct-native-server-witness ', 1)[1])
                  for line in server_raw.splitlines() if 'distinct-native-server-witness ' in line]
        passed = guest_code == 0 and server_code == 0 and len(client) == 1 and len(server) == 1
        if passed:
            passed = (client[0]['algorithm'] == algorithm and client[0]['architecture'] == 'x86_64'
                      and client[0]['os'] == 'linux' and client[0]['reconnected_receipt_preserved']
                      and client[0]['verified_native_heartbeat'] and server[0]['durable_create_operations'] == 1
                      and server[0]['old_physical_peers_drained']
                      and bool(re.search(r'test result: ok\. 1 passed; 0 failed; 0 ignored;', server_raw)))
        row = dict(algorithm=algorithm, passed=passed, guest_exit_code=guest_code, server_exit_code=server_code,
                   guest_witness=client, server_witness=server, guest_log_sha256=digest(fixture / 'guest.log'),
                   server_log_sha256=digest(log))
        results.append(row)
        print(json.dumps(row), flush=True)
    return results


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True, help='New directory under .local/gate-6/native-remote')
    parser.add_argument('--target-dir', type=Path, required=True, help='Native release build cache')
    parser.add_argument('--guest-target-dir', type=Path, required=True, help='Owned Linux build cache')
    parser.add_argument('--guest-image', default='connect-gate6-native-client:clang1971-ubuntu2404')
    args = parser.parse_args()
    os.chdir(ROOT)
    allowed = ROOT / '.local/gate-6/native-remote'
    allowed.mkdir(parents=True, exist_ok=True)
    out = args.output.resolve()
    assert out.is_relative_to(allowed.resolve()), 'Raw evidence and credentials must stay in the ignored fixture directory'
    out.mkdir(exist_ok=False)
    tmp = out / 'tmp'
    tmp.mkdir()
    env = dict(os.environ, CARGO_INCREMENTAL='0', TMPDIR=str(tmp), RUST_LOG='connect_service=debug',
               CONNECT_G6_SIP_CRASH_CAPTURE=str(out / 'sip-submitted'))
    before = HELPERS['sources']()
    report = dict(passed=False, scope='Eight isolated native Remote profile methods, two actual SIP SIGKILL public operator methods, two record drain methods and two macOS-server/Ubuntu-guest native control/reconnect profiles. Linux/amd64 guest is emulated on local aarch64 Lima; no distinct-host decoded media or native x86_64 performance claim.',
                  source_revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip(),
                  source_inputs_sha256=before, source_lock_sha256=digest(ROOT / 'dependencies/source-lock.json'),
                  started_at=datetime.datetime.now(datetime.timezone.utc).isoformat(), cases=[], distinct_profiles=[])
    binaries = {}
    try:
        assert run(['bash', 'scripts/prepare-dependencies.sh'], out / 'dependencies.log', env) == 0
        native = build('gate6_canonical_runtime', args.target_dir.resolve(), out, env)
        auxiliary = build('gate6_auxiliary', args.target_dir.resolve(), out, env)
        args.guest_target_dir.mkdir(parents=True, exist_ok=True)
        guest, platform = guest_build(args.guest_image, args.guest_target_dir.resolve(), out, env)
        report['guest_platform'] = platform
        binaries = {str(path): digest(path) for path in [native, auxiliary, guest]}
        report['executables_sha256'] = binaries
        report['cases'] = cases(native, CASES, out, env) + cases(auxiliary, DRAINS, out, env)
        report['distinct_profiles'] = distinct(native, guest, args.guest_image, out, env)
    finally:
        report['source_inputs_unchanged'] = HELPERS['sources']() == before
        report['executables_unchanged'] = bool(binaries) and all(digest(Path(path)) == value for path, value in binaries.items())
        report['passed'] = (len(report['cases']) == 12 and all(row['passed'] for row in report['cases'])
                            and len(report['distinct_profiles']) == 2 and all(row['passed'] for row in report['distinct_profiles'])
                            and report['source_inputs_unchanged'] and report['executables_unchanged'])
        report['finished_at'] = datetime.datetime.now(datetime.timezone.utc).isoformat()
        (out / 'summary.json').write_text(json.dumps(report, indent=2) + '\n')
        print(json.dumps(dict(passed=report['passed'], summary=str(out / 'summary.json'))), flush=True)
    raise SystemExit(0 if report['passed'] else 1)


if __name__ == '__main__':
    main()
