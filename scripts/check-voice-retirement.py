#!/usr/bin/env python3
"""Qualify all 10,000 retained voice setups with real expiry and listener reuse."""
import datetime
import json
import os
import platform
import sys
import traceback

from lib.local_process import ROOT
from lib.qualification_process import run_phase
from lib.qualification_report import environment, source_inputs, save
from lib.voice_retirement_report import TEST, validate


def main():
    assert (platform.system(), platform.machine()) == ('Darwin', 'arm64')
    os.umask(0o077)
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ')
    directory = ROOT / '.local/voice-retirement' / stamp
    directory.mkdir(parents=True, mode=0o700)
    temporary = directory / 'tmp'
    temporary.mkdir(mode=0o700)
    before = source_inputs()
    report = {'status': 'running', 'scope': 'Local macOS arm64; real-clock retirement; loopback fixtures only',
              'source_inputs': before, 'phases': [], 'test_budget_seconds': 2700,
              'phase_budget_seconds': 3300, 'account_free': True}
    save(directory, report)
    env = environment()
    env.update(CONNECT_VOICE_RETIREMENT_DIR=str(directory), TMPDIR=str(temporary))
    command = ['cargo', 'test', '--locked', '-p', 'connect-server', '--all-features',
               '--test', 'gate4_vapi_retirement_load', '--', '--ignored', '--exact', TEST,
               '--nocapture', '--test-threads=1']
    error = None
    try:
        phase = run_phase('retirement', command, 3300, directory, ROOT, env)
        report['phases'].append(phase)
        assert phase['status'] == 'passed', 'Retirement executable did not pass'
        report['measurements'] = validate(json.loads((directory / 'observations.json').read_text()),
                                          (directory / phase['log']).read_text())
        assert (directory / 'retained.sqlite3').is_file(), 'Missing retained Store snapshot'
        assert before == source_inputs(), 'Sources changed during qualification'
    except BaseException as caught:
        error = caught
        report['error'] = str(caught)
        traceback.print_exc()
    finally:
        report['source_stable'] = before == source_inputs()
        report['status'] = 'failed' if error else 'passed'
        save(directory, report)
    print(report['status'].upper(), directory / 'result.json', flush=True)
    return int(error is not None)


if __name__ == '__main__':
    sys.exit(main())
