#!/usr/bin/env python3
"""Build a pinned schema-6 or pre-Gate-4 baseline from local Git history."""
import argparse
import hashlib
import json
import os
import pathlib
import subprocess
import shutil
import tempfile

from lib.local_process import ROOT
from lib.ci_cache import retire_baseline_cache

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--schema', type=int, choices=(6, 10), default=6)
parser.add_argument('--retire-build-cache', action='store_true',
                    help='After verifying the sealed executable, retire only this baseline target directory')
args = parser.parse_args()
REVISION = {
    6: '8bf5bd31a519a5fa84ef8f80e4da3b58374c6de5',
    10: '99243f3962cf25e780bbd176e56a4f27c752e121',
}[args.schema]
os.umask(0o077)
output = ROOT / '.local' / 'recovery-baselines' / REVISION
output.mkdir(parents=True, exist_ok=True)
manifest = output / 'manifest.json'
binary = output / 'connect'
def retire_if_requested():
    if args.retire_build_cache:
        report = retire_baseline_cache(output, REVISION)
        (output / 'cache-retirement.json').write_text(json.dumps(report, indent=2) + '\n')
if manifest.exists() and binary.exists():
    saved = json.loads(manifest.read_text())
    if (saved.get('source_revision') == REVISION
            and saved.get('binary_sha256') == hashlib.sha256(binary.read_bytes()).hexdigest()):
        retire_if_requested()
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
        'schema': args.schema, 'source_revision': REVISION,
        'source_tree': subprocess.check_output(['git', 'rev-parse', REVISION + '^{tree}'], cwd=ROOT, text=True).strip(),
        'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
        'build_environment': {key: environment[key] for key in
                              ['CARGO_PROFILE_DEV_DEBUG', 'CARGO_PROFILE_TEST_DEBUG', 'CARGO_INCREMENTAL']
                              if key in environment},
        'build': 'cargo build --locked -p connect-server --bin connect; pinned historical dependency preparation',
    }, indent=2) + '\n')
retire_if_requested()
print(binary)
