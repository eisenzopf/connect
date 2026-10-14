#!/usr/bin/env python3
"""Build a pinned public recovery baseline; schema 1 is an explicit fixture patch."""
import argparse
import hashlib
import json
import os
import pathlib
import re
import subprocess
import shutil
import tempfile

from lib.local_process import ROOT
from lib.ci_cache import retire_baseline_cache

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--schema', type=int, choices=(1, 6, 10, 16), default=6)
parser.add_argument('--describe', action='store_true', help='Print the public source and patch without building')
parser.add_argument('--public-source', action='store_true',
                    help='Fetch the exact public commit into private scratch; works without local Git history')
parser.add_argument('--retire-build-cache', action='store_true',
                    help='After verifying the sealed executable, retire only this baseline target directory')
args = parser.parse_args()
catalog = json.loads((ROOT / 'dependencies/recovery-baselines.json').read_text())
specification = catalog['baselines'][str(args.schema)]
REVISION = specification['revision']
patch = ROOT / specification['patch'] if specification.get('patch') else None
if patch and hashlib.sha256(patch.read_bytes()).hexdigest() != specification['patch_sha256']:
    parser.error('Public baseline patch digest mismatch')
if args.describe:
    print(json.dumps(dict(specification, repository=catalog['repository'], schema=args.schema), indent=2))
    raise SystemExit(0)
os.umask(0o077)
output = ROOT / '.local' / 'recovery-baselines' / REVISION
output.mkdir(parents=True, exist_ok=True)
manifest = output / 'manifest.json'
binary = output / 'connect'
def retire_if_requested():
    if args.retire_build_cache:
        report = retire_baseline_cache(output, REVISION)
        (output / 'cache-retirement.json').write_text(json.dumps(report, indent=2) + '\n')
# Legacy choices retain their no-fetch default. Explicit public source works
# from an extracted current source archive without borrowing private Git objects.
repository = ROOT
if args.public_source:
    repository = output / 'public-source.git'
    if not repository.exists():
        subprocess.run(['git', 'init', '--bare', str(repository)], check=True, stdout=subprocess.DEVNULL)
    public_environment = os.environ.copy()
    for key in tuple(public_environment):
        if key.startswith('GIT_CONFIG_'):
            del public_environment[key]
    public_environment.update(GIT_TERMINAL_PROMPT='0', GIT_CONFIG_NOSYSTEM='1',
                              GIT_CONFIG_GLOBAL=os.devnull, GIT_ASKPASS=os.devnull,
                              SSH_ASKPASS=os.devnull)
    subprocess.run(['git', '-c', 'credential.helper=', '-c', 'core.askPass=',
                    '-c', 'http.extraHeader=', '-c', 'core.hooksPath=/dev/null',
                    '-C', str(repository), 'fetch', '--depth=1',
                    catalog['repository'], REVISION], env=public_environment, check=True)
    acquisition = {
        'schema': args.schema, 'source_repository': catalog['repository'],
        'source_revision': REVISION,
        'source_tree': subprocess.check_output(
            ['git', '-C', str(repository), 'rev-parse', REVISION + '^{tree}'], text=True).strip(),
        'authentication': 'credential helpers, prompts, global/system configuration and extra headers disabled',
    }
    (output / 'public-fetch.json').write_text(json.dumps(acquisition, indent=2) + '\n')
if args.public_source:
    subprocess.run(['git', 'cat-file', '-e', REVISION + '^{commit}'], cwd=repository, check=True)
if manifest.exists() and binary.exists():
    saved = json.loads(manifest.read_text())
    if (saved.get('source_revision') == REVISION and saved.get('schema') == args.schema
            and saved.get('patch_sha256') == specification.get('patch_sha256')
            and saved.get('binary_sha256') == hashlib.sha256(binary.read_bytes()).hexdigest()
            and (not args.public_source or saved.get('source_tree') == acquisition['source_tree'])):
        retire_if_requested()
        print(binary)
        raise SystemExit(0)
if not args.public_source:
    subprocess.run(['git', 'cat-file', '-e', REVISION + '^{commit}'], cwd=repository, check=True)
with tempfile.TemporaryDirectory(prefix='source-', dir=output) as directory:
    source = pathlib.Path(directory)
    archive = source / 'source.tar'
    subprocess.run(['git', 'archive', '--format=tar', '--output', str(archive), REVISION], cwd=repository, check=True)
    subprocess.run(['tar', '-xf', str(archive), '-C', str(source)], check=True)
    archive.unlink()
    if patch:
        subprocess.run(['git', 'apply', '--check', str(patch)], cwd=source, check=True)
        subprocess.run(['git', 'apply', str(patch)], cwd=source, check=True)
    schema_match = re.search(r'pub const SCHEMA_VERSION: u32 = (\d+);',
                             (source / 'crates/connect-store/src/lib.rs').read_text())
    if not schema_match or int(schema_match.group(1)) != args.schema:
        raise ValueError('Historical source/fixture schema does not match the selected catalog entry')
    environment = os.environ.copy()
    if args.public_source:
        # Historical dependency preparation also acquires only public sources.
        for key in tuple(environment):
            if key.startswith('GIT_CONFIG_'):
                del environment[key]
        for key in ('GIT_TERMINAL_PROMPT', 'GIT_CONFIG_NOSYSTEM', 'GIT_CONFIG_GLOBAL',
                    'GIT_ASKPASS', 'SSH_ASKPASS'):
            environment[key] = public_environment[key]
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
        'source_tree': subprocess.check_output(['git', 'rev-parse', REVISION + '^{tree}'], cwd=repository, text=True).strip(),
        'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
        'source_repository': catalog['repository'], 'baseline_kind': specification['kind'],
        'source_acquisition': 'anonymous-public-fetch' if args.public_source else 'local-pinned-git-object',
        'patch': specification.get('patch'), 'patch_sha256': specification.get('patch_sha256'),
        'qualification': specification.get('qualification', 'Unmodified pinned public historical source'),
        'cargo_lock_sha256': hashlib.sha256((source / 'Cargo.lock').read_bytes()).hexdigest(),
        'dependencies': json.loads((source / 'dependencies/source-lock.json').read_text()),
        'rustc': subprocess.check_output(['rustc', '--version'], env=environment, text=True).strip(),
        'build_environment': {key: environment[key] for key in
                              ['CARGO_PROFILE_DEV_DEBUG', 'CARGO_PROFILE_TEST_DEBUG', 'CARGO_INCREMENTAL']
                              if key in environment},
        'build': 'cargo build --locked -p connect-server --bin connect; pinned historical dependency preparation',
    }, indent=2) + '\n')
retire_if_requested()
print(binary)
