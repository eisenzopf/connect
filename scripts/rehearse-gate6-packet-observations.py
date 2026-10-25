#!/usr/bin/env python3
"""Verify current packet metadata, native authority and process fences in isolation."""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
REGRESSION = 'peer_authority::connection_media::snapshot::tests::current_packet_metadata_survives_unrelated_sql_custody_but_observes_commits_and_authority'
GROUPS = [
    ('native-authority', 'connection_control::tests::native_stream_authority::'),
    ('snapshot', 'peer_authority::connection_media::snapshot::tests::'),
    ('file-fences', 'authority_fences::tests::'),
]


def digest(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def sources():
    paths = [ROOT / name for name in ('Cargo.toml', 'Cargo.lock', 'dependencies/source-lock.json', 'scripts/prepare-dependencies.sh', 'scripts/format.sh', 'scripts/rehearse-gate6-packet-observations.py')]
    paths = [p for p in paths if p.is_file()]
    for crate in ('connect-store', 'connect-protocol', 'connect-connector-api'):
        paths.extend(p for p in (ROOT / 'crates' / crate).rglob('*') if p.is_file() and (p.suffix in ('.rs', '.sql') or p.name == 'Cargo.toml'))
    return {str(p.relative_to(ROOT)): digest(p) for p in sorted(paths)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--target-dir', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--expect-regression', action='store_true', help='Historical negative phase only: verify the exact unrelated SQL custody assertion fails, never report a pass.')
    args = parser.parse_args()
    output = args.output.resolve()
    if not output.is_relative_to((ROOT / '.local').resolve()):
        parser.error('Keep private raw evidence under ignored .local/')
    output.mkdir(parents=True, exist_ok=False)
    (output / 'tmp').mkdir()
    env = dict(os.environ, CARGO_INCREMENTAL='0', TMPDIR=str(output / 'tmp'))
    report = dict(passed=False, expected_negative=args.expect_regression, verified_expected_failure=False,
                  started_at=datetime.datetime.now(datetime.timezone.utc).isoformat(),
                  source_revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
                  scope='Controlled current SQL metadata, native authority and process file fences only. No connected media, browser/native interoperability, performance, full G6-050 or acceptance qualification.',
                  source_inputs_sha256=sources(), steps=[], groups=[])

    def step(name, command, artifact=False):
        with (output / (name + '.log')).open('w') as log:
            if artifact:
                with (output / 'build.jsonl').open('w') as stdout:
                    code = subprocess.run(command, cwd=ROOT, env=env, stdout=stdout, stderr=log).returncode
            else:
                code = subprocess.run(command, cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT).returncode
        report['steps'].append(dict(name=name, command=command, exit_code=code, log_sha256=digest(output / (name + '.log'))))
        print(json.dumps(dict(step=name, exit_code=code)), flush=True)
        assert code == 0, name

    try:
        step('dependencies', ['bash', 'scripts/prepare-dependencies.sh'])
        step('format', ['bash', 'scripts/format.sh', '--check'])
        step('build', ['cargo', 'test', '-p', 'connect-store', '--lib', '--no-run', '--locked', '--offline', '-j2', '--target-dir', str(args.target_dir.resolve()), '--message-format=json'], True)
        rows = [json.loads(line) for line in (output / 'build.jsonl').read_text().splitlines()]
        artifacts = [r for r in rows if r.get('reason') == 'compiler-artifact' and r.get('executable') and r['target']['name'] == 'connect_store' and r['profile']['test']]
        assert len(artifacts) == 1
        binary = Path(artifacts[0]['executable'])
        sha = digest(binary)
        report['executable'] = dict(path=str(binary), sha256=sha)
        env['CARGO_MANIFEST_DIR'] = str(ROOT / 'crates/connect-store')
        groups = [('negative', REGRESSION)] if args.expect_regression else GROUPS
        for name, selection in groups:
            command = [str(binary), selection, '--test-threads=1', '--nocapture']
            if args.expect_regression:
                command.append('--exact')
            started = time.monotonic()
            path = output / (name + '.log')
            with path.open('w') as log:
                code = subprocess.run(command, cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT, timeout=300).returncode
            text = path.read_text()
            counts = re.search(r'test result: (?:ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored;', text)
            assert counts, name
            passed, failed, ignored = map(int, counts.groups())
            row = dict(name=name, selection=selection, command=command, exit_code=code, passed_methods=passed, failed_methods=failed, ignored_helpers=ignored,
                       log=str(path.relative_to(ROOT)), log_sha256=digest(path), elapsed_seconds=time.monotonic() - started)
            report['groups'].append(row)
            print(json.dumps(row), flush=True)
            if args.expect_regression:
                report['verified_expected_failure'] = code != 0 and (passed, failed, ignored) == (0, 1, 0) and 'unrelated primary SQL custody must not mute current packet reads' in text and 'authority-busy' in text
            else:
                assert code == 0 and passed > 0 and failed == 0 and (ignored == 0 or name == 'file-fences' and ignored == 1), name
        report['executable_unchanged'] = digest(binary) == sha
    except Exception as error:
        report['failure'] = str(error)
    finally:
        report['source_inputs_unchanged'] = sources() == report['source_inputs_sha256']
        valid = report.get('executable_unchanged') is True and report['source_inputs_unchanged'] and 'failure' not in report
        report['verified_expected_failure'] = report['verified_expected_failure'] and valid
        report['passed'] = not args.expect_regression and valid and len(report['groups']) == len(GROUPS)
        report['finished_at'] = datetime.datetime.now(datetime.timezone.utc).isoformat()
        (output / 'summary.json').write_text(json.dumps(report, indent=2) + '\n')
        print(json.dumps(dict(passed=report['passed'], verified_expected_failure=report['verified_expected_failure'], report=str(output / 'summary.json'))), flush=True)
    return 0 if report['passed'] or report['verified_expected_failure'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
