#!/usr/bin/env python3
"""Source-bound local credential revocation, replay, SDK, CLI and signed-peer checks."""
import datetime
import importlib.util
import json
import os
import pathlib
import platform
import re
import subprocess

ROOT = pathlib.Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('provisioning_sdk', ROOT / 'scripts/check-gate6-provisioning-sdk.py')
base = importlib.util.module_from_spec(spec)
spec.loader.exec_module(base)
digest = base.digest
names = base.base.names


def sources():
    values = base.sources()
    name = 'scripts/check-gate6-credential-revocation.py'
    values[name] = digest(ROOT / name)
    return values


def main():
    assert platform.system() == 'Darwin' and platform.machine() == 'arm64'
    directory = ROOT / '.local/gate-6/credential-revocation' / datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S%fZ')
    directory.mkdir(parents=True)
    before = sources()
    env = dict(os.environ, CONNECT_GATE6_HTTP_SAMPLES=str(directory / 'http-samples'))
    cargo = ['cargo', 'test', '--locked', '--offline', '-j1']
    commands = [
        ('dependencies', ['bash', 'scripts/prepare-dependencies.sh'], 0),
        ('format', ['bash', 'scripts/format.sh', '--check'], 0),
        ('client', cargo + ['-p', 'connect-client', '--lib', '--', '--test-threads=1'], 40),
        ('docs', cargo + ['-p', 'connect-client', '--doc'], 3),
        ('store-provisioning', cargo + ['-p', 'connect-store', '--test', 'gate6_provisioning', '--', '--test-threads=1'], 16),
        ('application-sdk', cargo + ['-p', 'connect-server', '--test', 'gate6_application_sdk', '--', '--test-threads=1'], 10),
        ('http-provisioning', cargo + ['-p', 'connect-server', '--test', 'gate6_provisioning', '--', '--test-threads=1'], 11),
        ('production-native', cargo + ['-p', 'connect-server', '--test', 'gate6_canonical_runtime', '--', '--test-threads=1'], 11),
        ('openapi', [str(ROOT / '.local/gate-6/verification-venv/bin/python'), 'scripts/check-gate6-provisioning-sdk.py', '--samples', str(directory / 'http-samples')], 0),
        ('contracts', [str(ROOT / '.local/gate-6/verification-venv/bin/python'), 'scripts/check-gate6-contracts.py', '--output', str(directory / 'contracts.json')], 0),
        ('progress', ['python3', 'scripts/check-gate6-progress.py'], 0),
        ('diff', ['git', 'diff', '--check'], 0),
    ]
    steps = []
    for name, command, minimum in commands:
        log = directory / (name + '.log')
        with log.open('w') as output:
            result = subprocess.run(command, cwd=ROOT, env=env, stdout=output, stderr=subprocess.STDOUT)
        raw = log.read_text(errors='replace')
        tests = re.findall(r'^test (.+?) \.\.\. ok$', raw, re.M)
        binaries = {p: digest(ROOT / p) for p in re.findall(r'Running [^\n]+\((target/debug/deps/[^\)]+)\)', raw)}
        if name in ('application-sdk', 'http-provisioning', 'production-native') and result.returncode == 0:
            binaries['target/debug/connect'] = digest(ROOT / 'target/debug/connect')
        step = dict(name=name, command=command, exit_code=result.returncode, tests_passed=tests,
                    private_log=str(log.relative_to(ROOT)), log_sha256=digest(log),
                    test_binaries_sha256=binaries, passed=result.returncode == 0 and len(tests) >= minimum)
        steps.append(step)
        print(json.dumps(dict(step=name, passed=step['passed'], tests=len(tests))), flush=True)
        if not step['passed']:
            break
    required = set()
    for file, prefix in (
        ('crates/connect-client/src/application/tests.rs', 'application::tests::'),
        ('crates/connect-client/src/canonical/tests.rs', 'canonical::tests::'),
        ('crates/connect-store/tests/gate6_provisioning.rs', ''),
        ('crates/connect-server/tests/gate6_application_sdk.rs', ''),
        ('crates/connect-server/tests/gate6_provisioning.rs', ''),
        ('crates/connect-server/tests/gate6_canonical_runtime.rs', ''),
    ):
        required.update(names(ROOT / file, prefix))
    missing = sorted(required - {t for step in steps for t in step['tests_passed']})
    after = sources()
    historical = ROOT / 'docs/evidence/gate-6/provisioning-sdk.json'
    report = dict(format=1, task='G6-037', implementation_status='in-progress',
        qualification='Typed durable credential revocation through current authority guards, shared provisioning HTTP/SDK and installed CLI; exact logical receipt replay/native recovery and signed peer retirement without borrowing another credential bound to the same key; local macOS arm64',
        executed_at=datetime.datetime.now(datetime.timezone.utc).isoformat(), platform=platform.platform(),
        source_revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
        source_inputs_sha256=before, sources_unchanged=before == after, steps=steps,
        required_tests=sorted(required), missing_required_tests=missing,
        historical_provisioning_sdk=dict(path=str(historical.relative_to(ROOT)), sha256=digest(historical), disposition='Unchanged historical source snapshot; not current-source qualification'),
        not_qualified=[
            'Credential secret issuance/recovery and rotation tooling remains on earlier configurations. This mutation returns metadata only and does not issue or recover secrets.',
            'Complete high-level ConnectClient/ClientConfig, setup/presence, connected-app and remaining task/example/media migration; G6-037 remains in progress.',
            'Complete canonical/extension installation, refresh/step-up, physical HTTP response-write custody, production media/G6-020, all provider/voice/SIP paths and Gate 6 acceptance.',
            'All-workspace metadata and broader bootstrap/provider/legacy workflow regressions were not repeated for this scoped protocol/Store/API/CLI change. Changed libraries and actual integration/CLI binaries compile and execute; selected production signed native tests are repeated.',
            'Linux, remote deployment/public ingress, live providers, physical browsers, release/performance and publication.'
        ],
        exploratory_attempts=[dict(private_log=str(p.relative_to(ROOT)), sha256=digest(p), disposition='Unqualified exploratory run') for p in sorted((ROOT / '.local/gate-6').glob('credential-revocation-*-exploratory.log'))],
        passed=before == after and len(steps) == len(commands) and len(required) >= 70 and not missing and all(step['passed'] for step in steps))
    destination = ROOT / 'docs/evidence/gate-6/credential-revocation.json'
    if destination.exists():
        previous = json.loads(destination.read_text())
        report['previous_attempts'] = previous.get('previous_attempts', []) + [dict(sha256=digest(destination), **{k: previous[k] for k in ('passed', 'executed_at', 'source_inputs_sha256', 'steps')})]
    destination.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(dict(passed=report['passed'], missing=missing)), flush=True)
    if not report['passed']:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
