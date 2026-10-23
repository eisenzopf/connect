#!/usr/bin/env python3
"""Qualify G6-031 current membership/audience policy with isolated local fixtures."""
import argparse
import datetime
import hashlib
import json
import os
import pathlib
import platform
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def source_inputs():
    paths = [ROOT / name for name in (
        'Cargo.toml', 'Cargo.lock', 'dependencies/source-lock.json',
        'scripts/check-gate6-membership.py', 'scripts/check-gate6-contracts.py',
        'scripts/prepare-dependencies.sh', 'scripts/format.sh',
        'docs/developer/membership.md', 'docs/decisions/014-gate-6-contract-version.md',
    )]
    for folder in ('crates', 'contracts'):
        paths.extend(path for path in (ROOT / folder).rglob('*')
                     if path.is_file() and (path.suffix in ('.rs', '.sql', '.json', '.md')
                                            or path.name == 'Cargo.toml'))
    return {str(path.relative_to(ROOT)): digest(path) for path in sorted(set(paths))}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=pathlib.Path)
    args = parser.parse_args()
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S%fZ')
    directory = ROOT / '.local/gate-6/membership' / stamp
    directory.mkdir(parents=True)
    before = source_inputs()
    steps = []
    commands = [
        ('dependencies', ['bash', 'scripts/prepare-dependencies.sh'], 0),
        ('format', ['bash', 'scripts/format.sh', '--check'], 0),
        ('policy-regression', ['cargo', 'test', '--locked', '--no-fail-fast',
                               '-p', 'connect-store', '-p', 'connect-service',
                               '-p', 'connect-protocol', '-p', 'connect-connector-api',
                               '--lib', '--tests'], 250),
        ('server-regression', ['cargo', 'test', '--locked', '--no-fail-fast', '-p', 'connect-server',
                               '--lib', '--test', 'gate6_provisioning',
                               '--test', 'first_run_schema_boundary', '--test', 'artifact_recovery'], 50),
        ('contracts', [sys.executable, 'scripts/check-gate6-contracts.py',
                       '--output', str(directory / 'contracts.json')], 0),
        ('progress', [sys.executable, 'scripts/check-gate6-progress.py'], 0),
        ('diff', ['git', 'diff', '--check'], 0),
    ]
    for name, command, minimum in commands:
        log = directory / (name + '.log')
        with log.open('w') as output:
            result = subprocess.run(command, cwd=ROOT, env=dict(os.environ),
                                    stdout=output, stderr=subprocess.STDOUT)
        raw = log.read_text()
        names = re.findall(r'^test (\S+) \.\.\. ok$', raw, re.M)
        binaries = {}
        for path in re.findall(r'Running [^\n]+\((target/debug/deps/[^\)]+)\)', raw):
            binary = ROOT / path
            if binary.is_file():
                binaries[path] = digest(binary)
        steps.append(dict(name=name, command=command, exit_code=result.returncode,
                          tests_passed=names, test_binaries_sha256=binaries,
                          private_log=str(log.relative_to(ROOT)), log_sha256=digest(log),
                          passed=result.returncode == 0 and len(names) >= minimum))
        print(json.dumps(dict(step=name, passed=steps[-1]['passed'], tests=len(names))), flush=True)
    dispositions = {
        'membership-integration-attempt1.log': 'Earlier candidate exposed physical-deletion fixtures incompatible with retained membership epochs; fixtures now remove memberships logically. Current recipient ceilings remain enforced.',
        'membership-integration-attempt2.log': 'Same unqualified epoch-fixture failures before their repair.',
        'membership-integration-attempt4.log': 'Task context export withheld private counts; connector DTO/readers now represent undisclosed counts explicitly rather than parsing them as owner recall totals.',
        'membership-focused-attempt1.log': 'Temporary mapped-row borrow in revocation inventory was fixed by collecting rows before the statement drops.',
        'membership-focused-attempt2.log': 'Fault-injection test expected an incorrect storage error code; it now requires storage-unavailable and verifies rollback/retry.',
        'membership-focused-attempt3.log': 'Tenant fixture omitted its column list; the corrected fixture explicitly inserts the tenant ID.',
        'membership-focused-attempt4.log': 'Deliberately corrupted-schema fixture now disables its own foreign-key checks before dropping a referenced table. Production constraints remain enabled.',
        'membership-regression-attempt1.log': 'An additional native-context assertion still parsed provider export as owner recall; it now verifies the explicit bounded projection without a cursor/count.',
        'membership-regression-attempt2.log': 'Backup test used an incorrect Snapshot field name; corrected to digest.',
        'membership-regression-attempt3.log': 'Inbox expectation assumed conversation access required every member in the recipient ceiling. Own conversation metadata remains visible; the strengthened test separately denies an unpermitted peer source.',
        'membership-regression-attempt4.log': 'Fixed unique endpoint addresses, released the lock at credential expiry before the configured SQLite busy timeout, corrected own-metadata inbox expectations and removed owner delivery-log expectations from model exports. These were unsealed candidates; no failures are counted as passes.',
    }
    earlier = [dict(private_log=str(path.relative_to(ROOT)), log_sha256=digest(path),
                    disposition=reason, scope='Earlier unsealed candidate; not current qualification.')
               for name, reason in dispositions.items()
               if (path := ROOT / '.local/gate-6' / name).exists()]
    after = source_inputs()
    passed = before == after and all(step['passed'] for step in steps)
    report = dict(format=1, task='G6-031',
                  executed_at=datetime.datetime.now(datetime.timezone.utc).isoformat(),
                  scope='Durable membership/invitations/sharing; current source audiences, replay, files, task/context projections, queued chat/voice/SIP admission, service principal composition and local server regressions. Generic record-kind fixtures qualify the policy hook only. No canonical dispatch, multiparty physical teardown, record production, Linux/remote, live provider or full Gate 6 acceptance qualification.',
                  platform=platform.platform(), machine=platform.machine(), schema=int(re.search(r"SCHEMA_VERSION: u32 = (\d+)", (ROOT / "crates/connect-store/src/lib.rs").read_text()).group(1)),
                  source_revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
                  source_inputs_sha256=before, sources_unchanged=before == after,
                  source_lock_sha256=digest(ROOT / 'dependencies/source-lock.json'),
                  steps=steps, earlier_failed_attempts=earlier, passed=passed)
    output = args.output or ROOT / 'docs/evidence/gate-6/membership.json'
    output.parent.mkdir(parents=True, exist_ok=True)
    if output.exists():
        previous = json.loads(output.read_text())
        report['previous_attempts'] = previous.get('previous_attempts', []) + [dict(
            executed_at=previous['executed_at'], passed=previous['passed'],
            report_sha256=digest(output), steps=previous['steps'],
            source_inputs_sha256=previous['source_inputs_sha256'])]
    output.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(dict(task=report['task'], passed=passed, sources_unchanged=before == after)), flush=True)
    if not passed:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
