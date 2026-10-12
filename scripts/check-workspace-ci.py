#!/usr/bin/env python3
"""Bound peak hosted-CI disk usage while executing every Cargo workspace target."""
import argparse
import datetime
import os

from lib.ci_workspace import run
from lib.local_process import ROOT

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--all-features", action="store_true")
args = parser.parse_args()
os.umask(0o077)
directory = ROOT / ".local/ci-workspace" / datetime.datetime.now(
    datetime.timezone.utc).strftime("%Y%m%dT%H%M%S.%fZ")
print("CI workspace results:", directory, flush=True)
run(ROOT, directory, args.all_features)
