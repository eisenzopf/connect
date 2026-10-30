#!/usr/bin/env python3
"""Run one command with a new owned PulseAudio null sink in the isolated container."""
from pathlib import Path
import argparse
import hashlib
import json
import os
import platform
import shutil
import signal
import subprocess
import time

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--output', type=Path, required=True)
parser.add_argument('command', nargs=argparse.REMAINDER)
args = parser.parse_args()
root = Path(__file__).resolve().parents[2]
output = args.output.resolve()
assert platform.system() == 'Linux' and os.environ.get('CONNECT_GATE6_OWNED_BROWSER_CONTAINER') == '1'
assert output.is_relative_to(root / '.local') and args.command[:1] == ['--'] and len(args.command) > 1
output.mkdir(parents=True, mode=0o700, exist_ok=False)
binary = Path(shutil.which('pulseaudio')).resolve(strict=True)
socket = output / 'native'
cookie = output / 'cookie'
config = output / 'fixture.pa'
# Native protocol auth-cookie and client PULSE_COOKIE select only this private
# file: https://wiki.freedesktop.org/www/Software/PulseAudio/Documentation/User/Modules/
config.write_text(f'load-module module-native-protocol-unix socket={socket} auth-cookie={cookie}\n'
    'load-module module-null-sink sink_name=connect_fixture rate=48000 channels=2\n'
    'set-default-sink connect_fixture\n')
config.chmod(0o600)
environment = dict(os.environ, PULSE_SERVER='unix:' + str(socket), PULSE_COOKIE=str(cookie), PULSE_RUNTIME_PATH=str(output))
report = dict(scope='Owned anonymous container software output; no physical audio or personal daemon.', passed=False,
    binary_sha256=hashlib.file_digest(binary.open('rb'), 'sha256').hexdigest(),
    version=subprocess.check_output([str(binary), '--version'], text=True).strip())
with (output / 'pulse.log').open('xb') as log:
    pulse = subprocess.Popen([str(binary), '-n', '--daemonize=no', '--exit-idle-time=-1',
        '--disable-shm=yes', '--use-pid-file=no', '--log-target=stderr', '--log-level=warning',
        '--file=' + str(config)], env=environment, stdout=log, stderr=log, start_new_session=True)
    try:
        deadline = time.monotonic() + 5
        while not socket.exists():
            assert pulse.poll() is None and time.monotonic() < deadline, 'Owned audio startup failed'
            time.sleep(.025)
        result = subprocess.run(args.command[1:], env=environment, timeout=1200)
        report['command_exit_code'] = result.returncode
        report['passed'] = result.returncode == 0
    finally:
        if pulse.poll() is None:
            os.killpg(pulse.pid, signal.SIGTERM)
            try:
                pulse.wait(timeout=5)
            except subprocess.TimeoutExpired:
                report['forced_kill'] = True
                os.killpg(pulse.pid, signal.SIGKILL)
                pulse.wait()
        report['original_process_joined'] = pulse.poll() is not None
        report['exit_code'] = pulse.returncode
        report['startup_socket_created'] = socket.exists()
        report['private_cookie_created'] = cookie.is_file()
        text = (output / 'pulse.log').read_text(errors='replace')
        report['error_categories'] = [name for name, pattern in (
            ('module-load', 'Failed to load module'), ('cookie-load', 'Failed to load authentication key'),
            ('permission', 'Permission denied'), ('bind', 'bind() failed')) if pattern in text]
        report['passed'] = report['passed'] and pulse.returncode == 0 and not report.get('forced_kill') and report['private_cookie_created']
        (output / 'summary.json').write_text(json.dumps(report, indent=2) + '\n')
assert report['passed']
