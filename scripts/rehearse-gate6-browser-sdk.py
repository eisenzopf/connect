#!/usr/bin/env python3
"""Explicit local production-SDK/installed-Chrome control checkpoint, not media qualification."""
import argparse
import datetime
import json
import os
from pathlib import Path
import platform
import re
import runpy
import shutil
import signal
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
BASE = runpy.run_path(str(ROOT / 'scripts/rehearse-gate6-webtransport.py'))
digest = BASE['digest']
CASES = ['canonical_webtransport_browser::installed_chrome_production_sdk_signed_control_receipts_and_refusals_' + alg for alg in ('eddsa', 'es256')] + BASE['BROWSER']


def sources():
    result = BASE['sources']()
    paths = [ROOT / 'scripts/rehearse-gate6-browser-sdk.py']
    for folder, extensions in [('sdk/typescript/src', ('.ts',)), ('sdk/typescript/dist', ('.js', '.ts')), ('sdk/tests', ('.mjs', '.py'))]:
        paths.extend(p for p in (ROOT / folder).rglob('*') if p.is_file() and p.suffix in extensions)
    paths.extend(ROOT / 'sdk/typescript' / p for p in ('package.json', 'package-lock.json', 'tsconfig.json'))
    for path in paths:
        result[str(path.relative_to(ROOT))] = digest(path)
    return dict(sorted(result.items()))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--target-dir', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--installed-browser', action='store_true', required=True)
    args = parser.parse_args()
    directory = args.output.resolve()
    if not directory.is_relative_to((ROOT / '.local').resolve()):
        parser.error('Keep private evidence under ignored .local/')
    directory.mkdir(parents=True, exist_ok=False)
    (directory / 'tmp').mkdir()
    env = dict(os.environ, CARGO_INCREMENTAL='0', TMPDIR=str(directory / 'tmp'), CONNECT_GATE6_BROWSER_ARTIFACTS=str(directory / 'browser-artifacts'))
    report = dict(passed=False, started_at=datetime.datetime.now(datetime.timezone.utc).isoformat(), host=platform.platform(),
                  source_revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
                  scope='Actual production TypeScript canonical control in owned installed Chrome: both signing algorithms, separately enrolled trust, bad credential refusal, same-owner replay and native durable receipt parity. Two independent preexisting decoded-PCMU browser methods are regression checks only. No production SDK capture/Opus/VP8/data/media qualification, remote/provider, complete G6-050, acceptance or publication claim.', steps=[], cases=[])

    def step(name, command, json_output=False):
        start = time.monotonic()
        with (directory / (name + '.log')).open('w') as log:
            if json_output:
                with (directory / 'build.jsonl').open('w') as output:
                    code = subprocess.run(command, cwd=ROOT, env=env, stdout=output, stderr=log).returncode
            else:
                code = subprocess.run(command, cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT).returncode
        report['steps'].append(dict(name=name, command=command, exit_code=code, elapsed_seconds=time.monotonic() - start, log_sha256=digest(directory / (name + '.log'))))
        print(json.dumps(dict(step=name, exit_code=code)), flush=True)
        assert code == 0, name

    before = sources()
    report['source_inputs_sha256'] = before
    try:
        step('dependencies', ['bash', 'scripts/prepare-dependencies.sh'])
        step('rust-format', ['bash', 'scripts/format.sh', '--check'])
        step('sdk-suite', ['npm', 'test', '--prefix', 'sdk/typescript'])
        step('node', ['node', '--version'])
        step('rustc', ['rustc', '--version', '--verbose'])
        step('build', ['cargo', 'test', '-p', 'connect-server', '--test', 'gate6_canonical_webtransport', '--no-run', '--release', '--locked', '--offline', '-j2', '--target-dir', str(args.target_dir.resolve()), '--message-format=json'], True)
        rows = [json.loads(line) for line in (directory / 'build.jsonl').read_text().splitlines()]
        artifacts = [r for r in rows if r.get('reason') == 'compiler-artifact' and r.get('executable') and r['target']['name'] == 'gate6_canonical_webtransport' and r['profile']['test']]
        assert len(artifacts) == 1
        binary = Path(artifacts[0]['executable'])
        sealed = directory / 'qualified-browser-sdk'
        sha = digest(binary)
        shutil.copy2(binary, sealed)
        report['executable'] = dict(path=str(binary), sealed_path=str(sealed.relative_to(ROOT)), sha256=sha)
        env['CARGO_MANIFEST_DIR'] = artifacts[0]['manifest_path'].rsplit('/', 1)[0]
        available = subprocess.check_output([str(sealed), '--list'], env=env, text=True)
        assert all(case + ': test' in available for case in CASES)
        for index, case in enumerate(CASES):
            env['CONNECT_GATE6_BROWSER_ARTIFACTS'] = str(directory / 'browser-artifacts' / str(index))
            logpath = directory / f'case-{index}.log'
            start = time.monotonic()
            with logpath.open('w') as log:
                process = subprocess.Popen([str(sealed), case, '--exact', '--ignored', '--test-threads=1', '--nocapture'], cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
                try:
                    code = process.wait(timeout=120)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait()
                    code = 124
            passed = code == 0 and bool(re.search(r'test result: ok\. 1 passed; 0 failed; 0 ignored;', logpath.read_text()))
            row = dict(case=case, passed=passed, exit_code=code, elapsed_seconds=time.monotonic() - start, log=str(logpath.relative_to(ROOT)), log_sha256=digest(logpath))
            report['cases'].append(row)
            print(json.dumps(row), flush=True)
        report['executable_unchanged'] = digest(binary) == digest(sealed) == sha
    except Exception as error:
        report['failure'] = str(error)
    finally:
        report['source_inputs_unchanged'] = sources() == before
        report['passed'] = len(report['cases']) == len(CASES) and all(row['passed'] for row in report['cases']) and report['source_inputs_unchanged'] and report.get('executable_unchanged') is True and 'failure' not in report
        report['finished_at'] = datetime.datetime.now(datetime.timezone.utc).isoformat()
        (directory / 'summary.json').write_text(json.dumps(report, indent=2) + '\n')
        print(json.dumps(dict(passed=report['passed'], summary=str(directory / 'summary.json'))), flush=True)
    return 0 if report['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
