"""Explicit local Gate 4 qualification coverage and immutable phase budgets."""
import sys

HOST_TESTS = ('gate4_codex_runtime', 'gate4_codex_call_intent', 'gate4_codex_reply_workflow',
              'gate4_codex_packaged_runtime')


def phases(source_only=False):
    cargo = ['cargo', 'test', '--locked']
    server = [*cargo, '-p', 'connect-server', '--all-features']
    steps = [
        ('dependencies', ['bash', 'scripts/prepare-dependencies.sh'], 600),
        ('format', ['bash', 'scripts/format.sh', '--check'], 120),
        ('typescript-install', ['npm', 'ci', '--prefix', 'sdk/typescript'], 300),
        ('typescript-tests', ['npm', 'test', '--prefix', 'sdk/typescript'], 300),
        ('typescript-format', ['npm', 'run', 'format:check', '--prefix', 'sdk/typescript'], 120),
        ('typescript-distribution', ['git', 'diff', '--exit-code', '--', 'sdk/typescript/dist'], 60),
        ('python-sdk', [sys.executable, '-m', 'unittest', 'discover', '-s', 'sdk/tests', '-p', 'test_*.py'], 300),
        ('runner-failure-tests', [sys.executable, '-m', 'unittest', 'discover', '-s', 'scripts/tests', '-p', 'test_qualification*.py'], 120),
        ('lint', ['cargo', 'clippy', '--locked', '--workspace', '--all-targets', '--all-features', '--', '-D', 'warnings'], 2400),
        ('rustdoc', ['cargo', 'doc', '--locked', '--workspace', '--all-features', '--no-deps'], 2400),
        # All workspace gates, SDK parity, MCP, task/host fixtures, app/runner,
        # recovery and provider simulators run once under the same feature set.
        ('workspace', [*cargo, '--workspace', '--all-features', '--', '--test-threads=1'], 7200),
        ('media-pressure', [*server, '--test', 'gate3_media_pressure', '--', '--ignored', '--test-threads=1'], 1200),
    ]
    if not source_only:
        steps.append(('installed-host', [*server, *[arg for name in HOST_TESTS for arg in ('--test', name)],
                                        '--', '--ignored', '--test-threads=1'], 1800))
    steps += [
        ('upstream-regression', ['bash', 'scripts/check-baseline.sh'], 3600),
        ('upstream-vapi', ['env', 'CARGO_TARGET_DIR=target/upstream', *cargo,
                           '--manifest-path', '.deps/rvoip/Cargo.toml', '-p', 'rvoip-vapi',
                           '--test', 'mock_transport'], 1200),
        ('upstream-queues', ['env', 'CARGO_TARGET_DIR=target/upstream', *cargo,
                             '--manifest-path', '.deps/rvoip/Cargo.toml', '-p', 'rvoip-sip',
                             '-p', 'rvoip-webrtc', '--lib', 'queue_budget_tests'], 1200),
        ('upstream-codec', ['env', 'CARGO_TARGET_DIR=target/upstream', *cargo,
                            '--manifest-path', '.deps/rvoip/Cargo.toml', '-p', 'rvoip-sip',
                            '--test', 'adapter_renegotiate'], 1200),
        ('current-binaries', ['cargo', 'build', '--locked', '-p', 'connect-server', '--bin', 'connect'], 1200),
        ('native-task-capacity', [sys.executable, 'scripts/check-task-capacity.py'], 960),
        ('schema-six-build', [sys.executable, 'scripts/build-recovery-baseline.py', '--schema', '6'], 7500),
        ('schema-ten-build', [sys.executable, 'scripts/build-recovery-baseline.py', '--schema', '10'], 7500),
        ('historical-task-recovery', [sys.executable, 'scripts/check-gate4-recovery.py', '--previous-binary',
                                    '.local/recovery-baselines/99243f3962cf25e780bbd176e56a4f27c752e121/connect'], 660),
    ]
    for name, script in [('dispatch', 'check-dispatch.py'), ('identity', 'check-identity.py'),
                         ('typescript-probe', 'check-typescript.py'),
                         ('crash-replay', 'check-crash-replay.py'), ('examples', 'check-examples.py'),
                         ('example-recovery', 'check-example-recovery.py')]:
        steps.append((name, [sys.executable, 'scripts/' + script], 600))
    steps.append(('current-recovery', [sys.executable, 'scripts/check-recovery.py', '--current-only'], 660))
    steps.append(('datagram-example', ['cargo', 'run', '--locked', '-p', 'connect-server',
                                      '--example', 'quic_audio'], 600))
    return steps
