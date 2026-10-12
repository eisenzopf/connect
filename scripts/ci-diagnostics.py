#!/usr/bin/env python3
"""Reserve diagnostic headroom or retain redacted source-only CI failure evidence."""
import argparse
import json
import os

from lib.ci_diagnostics import bundle, disk
from lib.local_process import ROOT

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("mode", choices=("prepare", "collect"))
args = parser.parse_args()
os.umask(0o077)
directory = ROOT / ".local/ci-diagnostics"
directory.mkdir(parents=True, exist_ok=True)
reserve = directory / "diagnostic-reserve"
if args.mode == "prepare":
    # Real allocation, not a sparse truncate. Only this owned file is released.
    with reserve.open("xb") as stream:
        for _ in range(16):
            stream.write(bytes(1024 * 1024))
    print(json.dumps(disk(ROOT), indent=2), flush=True)
else:
    if reserve.is_symlink():
        raise RuntimeError("Diagnostic reserve must be an owned ordinary file")
    reserve.unlink(missing_ok=True)
    bundle(ROOT, directory / "upload")
