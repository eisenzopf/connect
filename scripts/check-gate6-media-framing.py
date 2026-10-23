#!/usr/bin/env python3
"""Qualify shared UCTP/RTP vectors in Rust and the browser fixture on Node."""
import datetime
import hashlib
import json
import pathlib
import platform
import re
import subprocess

ROOT = pathlib.Path(__file__).resolve().parents[1]
UPSTREAM = ROOT / '.deps/rvoip'
FIXTURE = 'crates/uctp/rvoip-uctp/tests/fixtures/cross_runtime_media.json'
BROWSER = 'tests/browser-smoke/fixtures/uctp_wt_acceptance.mjs'
NODE_TEST = 'tests/browser-smoke/tests/media_framing.test.mjs'
RUST_TEST = 'crates/uctp/rvoip-uctp/tests/datagram_conformance.rs'


def digest(path):
    with path.open('rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()


def sources():
    paths = [ROOT / 'scripts/check-gate6-media-framing.py', ROOT / 'scripts/prepare-dependencies.sh',
             ROOT / 'scripts/prepare-dependencies.py', ROOT / 'scripts/format.sh',
             ROOT / 'dependencies/source-lock.json', ROOT / 'contracts/native-media-profile.md', UPSTREAM / 'docs/CONVERSATION_PROTOCOL.md']
    paths += [UPSTREAM / name for name in (FIXTURE, BROWSER, NODE_TEST)]
    paths += [p for p in (ROOT / '.deps').rglob('*')
              if p.is_file() and (p.suffix == '.rs' or p.name in ('Cargo.toml', 'Cargo.lock'))]
    paths += list((ROOT / 'patches/rvoip').glob('*.patch'))
    return {str(path.relative_to(ROOT)): digest(path) for path in sorted(set(paths))}


def main():
    directory = ROOT / '.local/gate-6/media-framing' / datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S%fZ')
    directory.mkdir(parents=True)
    before = sources()
    vectors = json.loads((UPSTREAM / FIXTURE).read_text())
    required_node = {v['name'] for group in ('valid', 'invalid') for v in vectors[group]}
    cargo = ['cargo', 'test', '--locked', '--offline', '-j1', '--target-dir', 'target',
             '--manifest-path', '.deps/rvoip/Cargo.toml', '-p', 'rvoip-uctp']
    commands = [
        ('dependencies', ['bash', 'scripts/prepare-dependencies.sh'], 0),
        ('format', ['bash', 'scripts/format.sh', '--check'], 0),
        ('upstream-test-format', ['rustfmt', '--edition', '2021', '--config', 'skip_children=true', '--check', str((UPSTREAM / RUST_TEST).relative_to(ROOT))], 0),
        ('rust-conformance', cargo + ['--test', 'datagram_conformance', '--', '--test-threads=1'], 6),
        ('rust-library', cargo + ['--lib', 'substrate::datagram::tests', '--', '--test-threads=1'], 9),
        ('browser-fixture-node', ['node', '--test', '--test-reporter=tap', str((UPSTREAM / NODE_TEST).relative_to(ROOT))], len(required_node)),
        ('progress', ['python3', 'scripts/check-gate6-progress.py'], 0),
        ('diff', ['git', 'diff', '--check'], 0),
    ]
    steps = []
    for name, command, minimum in commands:
        log = directory / (name + '.log')
        with log.open('w') as output:
            result = subprocess.run(command, cwd=ROOT, stdout=output, stderr=subprocess.STDOUT)
        raw = log.read_text(errors='replace')
        tests = re.findall(r'^test (.+?) \.\.\. ok$', raw, re.M)
        if name == 'browser-fixture-node':
            tests = re.findall(r'^ok \d+ - (.+)$', raw, re.M)
        binaries = {path: digest(ROOT / path) for path in re.findall(r'Running [^\n]+\((target/debug/deps/[^\)]+)\)', raw)}
        step = dict(name=name, command=command, exit_code=result.returncode,
                    tests_passed=tests, log=str(log.relative_to(ROOT)), log_sha256=digest(log),
                    test_binaries_sha256=binaries, passed=result.returncode == 0 and len(tests) >= minimum)
        steps.append(step)
        print(json.dumps(dict(step=name, passed=step['passed'], tests=len(tests))), flush=True)
        if not step['passed']:
            break
    rust_cases = {t for s in steps if s['name'].startswith('rust-') for t in s['tests_passed']}
    node_cases = {t for s in steps if s['name'] == 'browser-fixture-node' for t in s['tests_passed']}
    missing = sorted(required_node - node_cases)
    required_rust = 'cross_runtime_media_vectors_match_checked_rust_and_browser_boundaries'
    if required_rust not in rust_cases:
        missing.append(required_rust)
    report = dict(task='G6-024', implementation_status='in-progress',
                  qualification='Shared checked UCTP datagram/RTP framing in native Rust APIs and the actual browser acceptance fixture executed by Node. Eight valid and 31 invalid vectors; byte order, full-range distinct sequence fields, nonzero physical handles, CSRC/extension/padding boundaries and payload normalization. Node executes the browser module; this does not claim a live browser/WebTransport network.',
                  not_qualified=['Full G6-024: current-authorized general Core/gateway DTMF routing and method/provenance, remaining media/feedback/profile coverage.',
                                 'Datagram MTU, fragmentation, decoded media and network replay/liveness are separate behavior checks; parsing is not authenticated route admission.',
                                 'Linux x86_64, independent browser network, remote/provider profiles, G6-093 rehearsals and Gate 6 acceptance.'],
                  executed_at=datetime.datetime.now(datetime.timezone.utc).isoformat(), platform=platform.platform(),
                  rustc=subprocess.check_output(['rustc', '--version'], text=True).strip(),
                  node=subprocess.check_output(['node', '--version'], text=True).strip(),
                  source_inputs_sha256=before, sources_unchanged=before == sources(), steps=steps,
                  valid_vectors=len(vectors['valid']), invalid_vectors=len(vectors['invalid']),
                  runtime_checks=sum(len(s['tests_passed']) for s in steps), missing_required_cases=missing,
                  passed=len(steps) == len(commands) and before == sources() and not missing and all(s['passed'] for s in steps))
    path = ROOT / 'docs/evidence/gate-6/media-framing.json'
    path.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(dict(passed=report['passed'], checks=report['runtime_checks'], report=str(path.relative_to(ROOT)))), flush=True)
    if not report['passed']:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
