#!/usr/bin/env python3
"""Prepare a pinned independent QUIC environment under ignored .local only."""
import pathlib
import subprocess
import sys
import venv

ROOT = pathlib.Path(__file__).resolve().parents[1]
path = ROOT / '.local/independent-uctp/venv'
if not (path / 'bin/python').is_file():
    venv.EnvBuilder(with_pip=True).create(path)
subprocess.run([str(path / 'bin/python'), '-m', 'pip', 'install', '--require-hashes',
                '-r', str(ROOT / 'examples/independent-uctp-client/requirements.txt')], check=True, timeout=240)
subprocess.run([str(path / 'bin/python'), '-m', 'pip', 'check'], check=True, timeout=30)
