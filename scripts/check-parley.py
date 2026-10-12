#!/usr/bin/env python3
"""Run pinned Parley conversation regressions without touching the pinned source."""
import pathlib, shutil, subprocess, tempfile, os
root=pathlib.Path(__file__).resolve().parents[1]
with tempfile.TemporaryDirectory(prefix='connect-parley-') as work:
    dest=pathlib.Path(work)/'parley'
    shutil.copytree(root/'.deps/parley',dest,ignore=shutil.ignore_patterns('.git','target'))
    (pathlib.Path(work)/'rvoip-conference').symlink_to(root/'.deps/rvoip',target_is_directory=True)
    environment=dict(os.environ,CARGO_TARGET_DIR=str(root/'target/upstream'))
    subprocess.run(['cargo','test','--locked','--no-default-features','--features','uctp,sms-fake','--test','api_conversations','--test','uctp_conference','--test','uctp_create'],cwd=dest,env=environment,check=True)
