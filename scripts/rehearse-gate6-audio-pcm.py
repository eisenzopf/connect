#!/usr/bin/env python3
"""Controlled original-PCM mixer regression; no host/media operating qualification."""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import re
import runpy
import shutil
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
BASE = runpy.run_path(str(ROOT / 'scripts/rehearse-gate6-catalog-retry.py'))
REGRESSION = 'staged_streams::tests::guarded_mixer::guarded_mixer_contiguous_original_packets_remain_ordered_across_one_tick'
ASSERTION = 'contiguous original audio frame was discarded by newer input'
GROUPS = ('staged_streams::tests::guarded_mixer::', 'staged_streams::tests::media_scene::')


def digest(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def sources():
    result = BASE['sources']()
    result['scripts/rehearse-gate6-audio-pcm.py'] = digest(Path(__file__))
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--target-dir', required=True, type=Path)
    parser.add_argument('--expect-regression', action='store_true')
    args = parser.parse_args()
    directory = args.output.resolve()
    if not directory.is_relative_to((ROOT / '.local').resolve()):
        parser.error('Keep raw evidence under ignored .local/')
    directory.mkdir(parents=True, exist_ok=False)
    (directory / 'tmp').mkdir()
    env = dict(os.environ, TMPDIR=str(directory / 'tmp'), CARGO_INCREMENTAL='0',
               CARGO_PROFILE_RELEASE_LTO='false', CARGO_PROFILE_RELEASE_CODEGEN_UNITS='16',
               CARGO_PROFILE_RELEASE_DEBUG='0', CARGO_PROFILE_RELEASE_PANIC='unwind')
    report = dict(passed=False, verified_expected_failure=False, expect_regression=args.expect_regression,
                  scope='Actual Core guarded mixer/scene tests only. No Connect host throughput, browser/native interoperability, full operating/task/acceptance/publication qualification.',
                  source_revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
                  started_at=datetime.datetime.now(datetime.timezone.utc).isoformat(), steps=[], cases=[])
    before = sources()
    report['source_inputs_sha256'] = before
    executable = None

    def step(name, command, compiler=False):
        start = time.monotonic()
        with (directory / (name + '.log')).open('w') as log:
            if compiler:
                with (directory / 'build.jsonl').open('w') as output:
                    code = subprocess.run(command, cwd=ROOT, env=env, stdout=output, stderr=log).returncode
            else:
                code = subprocess.run(command, cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT).returncode
        report['steps'].append(dict(name=name, command=command, exit_code=code,
                                    elapsed_seconds=time.monotonic() - start, log_sha256=digest(directory / (name + '.log'))))
        print(json.dumps(dict(step=name, exit_code=code)), flush=True)
        assert code == 0, name

    try:
        step('dependencies', ['bash', 'scripts/prepare-dependencies.sh'])
        step('format', ['bash', 'scripts/format.sh', '--check'])
        step('build', ['cargo', 'test', '--manifest-path', '.deps/rvoip/Cargo.toml', '-p', 'rvoip-core',
                       '--features', 'opus', '--lib', '--no-run', '--release', '--locked', '--offline', '-j2',
                       '--target-dir', str(args.target_dir.resolve()), '--message-format=json'], True)
        artifacts = [json.loads(line) for line in (directory / 'build.jsonl').read_text().splitlines()]
        found = [r for r in artifacts if r.get('reason') == 'compiler-artifact' and r.get('executable')
                 and r['target']['name'] == 'rvoip_core' and r['profile']['test']]
        assert len(found) == 1
        env['CARGO_MANIFEST_DIR'] = str(Path(found[0]['manifest_path']).parent)
        binary = Path(found[0]['executable'])
        sealed = directory / 'qualified-core-audio'
        sha = digest(binary)
        shutil.copy2(binary, sealed)
        executable = dict(path=str(binary), sealed_path=str(sealed.relative_to(ROOT)), sha256=sha)
        report['executable'] = executable
        available = subprocess.check_output([str(sealed), '--list'], env=env, text=True)
        names = [line.removesuffix(': test') for line in available.splitlines() if line.endswith(': test')]
        assert REGRESSION in names
        selections = [(REGRESSION, True)] if args.expect_regression else [(group, False) for group in GROUPS]
        for index, (selection, exact) in enumerate(selections):
            selected = [name for name in names if name == selection] if exact else [name for name in names if name.startswith(selection)]
            assert selected
            command = [str(sealed), selection, '--test-threads=1', '--nocapture'] + (['--exact'] if exact else [])
            log = directory / f'case-{index}.log'
            start = time.monotonic()
            with log.open('w') as output:
                code = subprocess.run(command, cwd=ROOT, env=env, stdout=output, stderr=subprocess.STDOUT, timeout=120).returncode
            text = log.read_text()
            match = re.search(r'test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored', text)
            counts = tuple(map(int, match.group(2, 3, 4))) if match else None
            passed = code == 0 and counts == (len(selected), 0, 0)
            expected = args.expect_regression and code == 101 and counts == (0, 1, 0) and ASSERTION in text
            row = dict(selection=selection, selected=selected, exit_code=code, passed=passed,
                       expected_failure=expected, counts=counts, log=str(log.relative_to(ROOT)),
                       log_sha256=digest(log), elapsed_seconds=time.monotonic() - start)
            report['cases'].append(row)
            print(json.dumps({k:v for k,v in row.items() if k != 'selected'}), flush=True)
        step('dependencies-after', ['bash', 'scripts/prepare-dependencies.sh'])
    except Exception as error:
        report['failure'] = str(error)
    finally:
        report['source_inputs_unchanged'] = sources() == before
        report['executable_unchanged'] = bool(executable) and digest(executable['path']) == digest(ROOT / executable['sealed_path']) == executable['sha256']
        guarded = report['source_inputs_unchanged'] and report['executable_unchanged'] and 'failure' not in report
        report['passed'] = bool(not args.expect_regression and guarded and len(report['cases']) == len(GROUPS) and all(c['passed'] for c in report['cases']))
        report['verified_expected_failure'] = bool(args.expect_regression and guarded and len(report['cases']) == 1 and report['cases'][0]['expected_failure'])
        report['finished_at'] = datetime.datetime.now(datetime.timezone.utc).isoformat()
        (directory / 'summary.json').write_text(json.dumps(report, indent=2) + '\n')
        print(json.dumps({k:report[k] for k in ('passed', 'verified_expected_failure')}), flush=True)
    return 0 if report['passed'] or report['verified_expected_failure'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
