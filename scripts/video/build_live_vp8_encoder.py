#!/usr/bin/env python3
"""Build only the bounded local VP8 fixture helper; never execute a workload."""
import argparse
import hashlib
import json
from pathlib import Path
import shlex
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
from lib.package_environment import fixture_environment


def build(output):
    env = fixture_environment()
    source = Path(__file__).with_name("live_vp8_encoder.c")
    original = source.read_bytes()
    flags = shlex.split(subprocess.check_output(
        ["pkg-config", "--cflags", "--libs", "vpx"], env=env, text=True, timeout=10))
    version = subprocess.check_output(
        ["pkg-config", "--modversion", "vpx"], env=env, text=True, timeout=10).strip()
    output = Path(output).resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    if output.exists():
        raise ValueError("Choose a fresh fixture executable destination")
    subprocess.run(["cc", "-std=c11", "-O2", "-Wall", "-Wextra", "-Werror",
                    str(source), "-o", str(output), *flags], env=env, check=True, timeout=60)
    if source.read_bytes() != original:
        output.unlink()
        raise ValueError("Fixture encoder source changed during compilation")
    return dict(source_sha256=hashlib.sha256(original).hexdigest(),
                executable_sha256=hashlib.sha256(output.read_bytes()).hexdigest(),
                libvpx=version, output=str(output),
                scope="Build only; not encoder behavior, transport or media qualification")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    print(json.dumps(build(args.output)))
