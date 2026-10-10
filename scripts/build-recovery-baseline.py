#!/usr/bin/env python3
"""Build the pinned schema-6 baseline from Git history, without private fixtures."""
import hashlib
import json
import os
import pathlib
import subprocess
import shutil
import tempfile

from lib.local_process import ROOT

REVISION = '8bf5bd31a519a5fa84ef8f80e4da3b58374c6de5'
os.umask(0o077)
output = ROOT / '.local' / 'recovery-baselines' / REVISION
output.mkdir(parents=True, exist_ok=True)
manifest = output / 'manifest.json'
binary = output / 'connect'
if manifest.exists() and binary.exists():
    saved = json.loads(manifest.read_text())
    if (saved.get('source_revision') == REVISION
            and saved.get('binary_sha256') == hashlib.sha256(binary.read_bytes()).hexdigest()):
        print(binary)
        raise SystemExit(0)
# Checkout must include history. CI uses fetch-depth: 0; no implicit remote fetch.
subprocess.run(['git', 'cat-file', '-e', REVISION + '^{commit}'], cwd=ROOT, check=True)
with tempfile.TemporaryDirectory(prefix='source-', dir=output) as directory:
    source = pathlib.Path(directory)
    archive = source / 'source.tar'
    subprocess.run(['git', 'archive', '--format=tar', '--output', str(archive), REVISION], cwd=ROOT, check=True)
    subprocess.run(['tar', '-xf', str(archive), '-C', str(source)], check=True)
    archive.unlink()
    environment = os.environ.copy()
    environment['CARGO_TARGET_DIR'] = str(output / 'target')
    # Builds and offline checks never require provider credentials.
    for key in ['TELNYX_API_KEY', 'VAPI_PRIVATE_KEY', 'CONNECT_SMS_AUTHORITY_TOKEN']:
        environment.pop(key, None)
    with (output / 'build.log').open('w') as log:
        for command in [['bash', 'scripts/prepare-dependencies.sh'], ['cargo', 'build', '--locked', '-p', 'connect-server', '--bin', 'connect']]:
            subprocess.run(command, cwd=source, env=environment, stdout=log, stderr=subprocess.STDOUT, timeout=3600, check=True)
    shutil.copy2(output / 'target/debug/connect', binary)
    manifest.write_text(json.dumps({
        'schema': 6, 'source_revision': REVISION,
        'source_tree': subprocess.check_output(['git', 'rev-parse', REVISION + '^{tree}'], cwd=ROOT, text=True).strip(),
        'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
        'build': 'cargo build --locked -p connect-server --bin connect; pinned historical dependency preparation',
    }, indent=2) + '\n')
print(binary)
