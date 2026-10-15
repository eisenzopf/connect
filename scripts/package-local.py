#!/usr/bin/env python3
"""Build an unpublished local runtime, SDK source, and recovery qualification bundle."""
import argparse
import hashlib
import json
import os
import pathlib
import platform
import re
import shutil
import subprocess
import tarfile
import tomllib
from lib.local_process import ROOT
from lib.package_inventory import source_inventory, copy_gate5_assets, verify_baseline, copy_prepared_crate_notices
from lib.package_environment import fixture_environment
from lib.registry_licenses import copy_registry_notices, selected_dependency_ids

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--profile', choices=('gate4-local', 'gate5-local'), default='gate4-local')
parser.add_argument('--previous-binary', type=pathlib.Path, help='Schema1 fixture; Gate4 retains its archived milestone default, Gate5 uses the public pinned fixture')
parser.add_argument('--gate4-binary', type=pathlib.Path, help='Gate5 public pinned schema16 historical binary')
parser.add_argument('--rehearse-source', type=pathlib.Path, metavar='REPORT', help='Offline source inventory only; allows dirty source, creates no release candidate')
args = parser.parse_args()
selected_environment = fixture_environment()
os.environ.clear()
os.environ.update(selected_environment)
os.umask(0o077)
if args.rehearse_source:
    if args.profile != 'gate5-local':
        parser.error('Source rehearsal requires explicit gate5-local profile')
    inputs = source_inventory(ROOT)
    report = {'profile': args.profile, 'result': 'source-inventory-verified', 'files': inputs,
              'source_revision': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
              'source_dirty': bool(subprocess.check_output(['git', 'status', '--porcelain'], cwd=ROOT)),
              'candidate_created': False, 'behavior_qualified': False, 'whole_gate_qualified': False,
              'not_run': ['release compilation', 'archive installation', 'historical upgrade', 'installed media', 'installed setup']}
    args.rehearse_source.parent.mkdir(parents=True, exist_ok=True)
    args.rehearse_source.write_text(json.dumps(report, indent=2)+'\n')
    print('Offline source inventory only:', args.rehearse_source)
    raise SystemExit(0)
if platform.system() != 'Darwin' or platform.machine() != 'arm64':
    parser.error('This candidate qualifies macOS arm64 only')
if subprocess.check_output(['git', 'status', '--porcelain'], cwd=ROOT, text=True).strip():
    parser.error('Commit the candidate source first; packaging requires a clean worktree')
catalog = json.loads((ROOT/'dependencies/recovery-baselines.json').read_text())['baselines'] if args.profile == 'gate5-local' else {}
previous_default = ROOT/'.local/milestones/m1/connect' if args.profile == 'gate4-local' else ROOT/'.local/recovery-baselines'/catalog['1']['revision']/'connect'
previous = (args.previous_binary or previous_default).resolve()
prior = json.loads(previous.with_name('manifest.json').read_text())
def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()
if digest(previous) != prior['binary_sha256']:
    parser.error('Archived milestone binary checksum mismatch')
if args.profile == 'gate5-local':
    source_inventory(ROOT)
    verify_baseline(previous, prior, ROOT, 1)
    gate4 = (args.gate4_binary or ROOT/'.local/recovery-baselines'/catalog['16']['revision']/'connect').resolve()
    gate4_manifest = json.loads(gate4.with_name('manifest.json').read_text())
    verify_baseline(gate4, gate4_manifest, ROOT, 16)
revision = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()


def ensure_source_unchanged():
    """Refuse a candidate if edits or a new commit raced the long build."""
    head = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
    dirty = subprocess.check_output(['git', 'status', '--porcelain'], cwd=ROOT, text=True)
    if head != revision or dirty.strip():
        raise RuntimeError('Candidate source changed during packaging; use a clean new revision')


version = tomllib.loads((ROOT/'Cargo.toml').read_text())['workspace']['package']['version']
candidate = f'connect-{version}-macos-arm64-{revision[:12]}'
if args.profile == 'gate5-local':
    candidate += '-gate5-local'
output = ROOT/'dist'/candidate
if output.exists():
    parser.error('Candidate directory exists; retain it and use a new source revision')
subprocess.run(['bash', 'scripts/prepare-dependencies.sh'], cwd=ROOT, check=True, timeout=600)
subprocess.run(['npm', 'ci', '--prefix', 'sdk/typescript'], cwd=ROOT, check=True, timeout=300)
subprocess.run(['npm', 'run', 'build', '--prefix', 'sdk/typescript'], cwd=ROOT, check=True, timeout=120)
build = ['cargo', 'build', '--locked', '--release', '-p', 'connect-server', '-p', 'connect-review-loop']
if args.profile == 'gate5-local':
    build += ['-p', 'connect-native-video']
subprocess.run(build + ['--bins'], cwd=ROOT, check=True, timeout=7200)
ensure_source_unchanged()
metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--locked', '--format-version=1'], cwd=ROOT, timeout=120))
# Use the build's Cargo configuration, including an explicit CARGO_TARGET_DIR.
target_directory = pathlib.Path(metadata['target_directory'])
output.mkdir(parents=True)
runtime = output/candidate
(runtime/'bin').mkdir(parents=True)
for name in ['connect', 'connect-example'] + (['connect-video'] if args.profile == 'gate5-local' else []):
    source = target_directory/'release'/name
    for control in [b'CONNECT_TEST_FAULT_STAGE', b'CONNECT_EXAMPLE_FAULT_STAGE', b'CONNECT_CONNECTED_APP_FAULT']:
        if control in source.read_bytes():
            raise RuntimeError('Release binary contains a test-only fault control')
    shutil.copy2(source, runtime/'bin'/name)
    (runtime/'bin'/name).chmod(0o755)
for source, destination in [
    ('LICENSE', 'LICENSE'), ('THIRD_PARTY_NOTICES.md', 'THIRD_PARTY_NOTICES.md'),
    ('docs/LOCAL_RELEASE.md', 'README.md'),
    ('examples/review-loop/README.md', 'docs/review-loop.md'),
    ('config/local.example.toml', 'config/local.example.toml'),
    ('dependencies/source-lock.json', 'dependencies/source-lock.json'),
    ('.deps/rvoip/LICENSE', 'licenses/rvoip-LICENSE'),
    ('.deps/parley/LICENSE', 'licenses/parley-LICENSE'),
]:
    target = runtime/destination
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(ROOT/source, target)
# Keep the public review report and linked plans with the runtime SDK guide.
# The root README is copied from docs/, so local documentation links need a prefix.
shutil.copytree(ROOT/'docs', runtime/'docs', dirs_exist_ok=True)
shutil.copytree(ROOT/'contracts', runtime/'contracts')
shutil.copytree(ROOT/'skills/connect', runtime/'skills/connect')
readme = runtime/'README.md'
readme.write_text(re.sub(
    r'\]\((?![a-zA-Z][a-zA-Z0-9+.-]*:|/|#)([^)]+)\)',
    r'](docs/\1)', readme.read_text()))
# Usable language clients accompany the native helper; no compiler, dependencies,
# caches or installation state are copied into the runtime archive.
shutil.copytree(ROOT/'sdk/python/connect_sdk', runtime/'sdk/python/connect_sdk',
                ignore=shutil.ignore_patterns('__pycache__', '*.pyc'))
shutil.copytree(ROOT/'sdk/typescript/dist', runtime/'sdk/typescript/dist')
shutil.copyfile(ROOT/'sdk/typescript/package.json', runtime/'sdk/typescript/package.json')
shutil.copytree(ROOT/'examples/agent-presence', runtime/'examples/agent-presence',
                ignore=shutil.ignore_patterns('__pycache__', '*.pyc', 'tests'))
shutil.copytree(ROOT/'examples/conversation-client', runtime/'examples/conversation-client',
                ignore=shutil.ignore_patterns('__pycache__', '*.pyc'))
if args.profile == 'gate5-local':
    copy_gate5_assets(ROOT, runtime)
    with readme.open('a') as guide:
        guide.write('\nGate 5 candidates use the explicit [Gate 5 package profile](docs/GATE5_PACKAGE.md); inventory and retained baseline checks do not establish Gate 5 closure.\n')
# Keep the conservative index, but enforce notice coverage for the exact
# default-feature normal/build graph of the selected macOS binary roots.
filtered_metadata = json.loads(subprocess.check_output(
    ['cargo', 'metadata', '--locked', '--offline', '--format-version=1',
     '--filter-platform', 'aarch64-apple-darwin'], cwd=ROOT, timeout=120))
license_roots = ['connect-server', 'connect-review-loop']
if args.profile == 'gate5-local':
    license_roots.append('connect-native-video')
copy_registry_notices(ROOT, runtime, metadata,
                      selected_dependency_ids(filtered_metadata, license_roots))
copy_prepared_crate_notices(ROOT, runtime, metadata)
files = {str(p.relative_to(runtime)): digest(p) for p in sorted(runtime.rglob('*')) if p.is_file()}
ensure_source_unchanged()
manifest = {'candidate': candidate, 'version': version, 'source_revision': revision,
            'schema': int(re.search(r'pub const SCHEMA_VERSION: u32 = (\d+);',
                                   (ROOT/'crates/connect-store/src/lib.rs').read_text()).group(1)),
            'target': 'aarch64-apple-darwin', 'os': platform.mac_ver()[0],
            'rustc': subprocess.check_output(['rustc', '--version'], text=True).strip(),
            'cargo_lock_sha256': digest(ROOT/'Cargo.lock'),
            'dependencies': json.loads((ROOT/'dependencies/source-lock.json').read_text()),
            'features': 'default; no test-support', 'scope': 'local candidate; unpublished',
            'qualification_profile': args.profile, 'files': files}
(runtime/'manifest.json').write_text(json.dumps(manifest, indent=2)+'\n')
with tarfile.open(output/(candidate+'.tar.gz'), 'w:gz') as archive:
    archive.add(runtime, arcname=candidate)
source_archive = output/(candidate+'-source.tar.gz')
with source_archive.open('wb') as stream:
    subprocess.run(['git', 'archive', '--format=tar.gz', '--prefix=connect-source/', revision], cwd=ROOT, stdout=stream, check=True)
qualification = output/'qualification'
qualification.mkdir()
shutil.copy2(previous, qualification/'connect-schema1')
(qualification/'manifest.json').write_text(json.dumps(prior, indent=2)+'\n')
if args.profile == 'gate5-local':
    shutil.copy2(gate4, qualification/'connect-schema16')
    (qualification/'schema16-manifest.json').write_text(json.dumps(gate4_manifest, indent=2)+'\n')
with tarfile.open(output/(candidate+'-qualification.tar.gz'), 'w:gz') as archive:
    archive.add(qualification, arcname='qualification')
archives = sorted(output.glob('*.tar.gz'))
(output/'SHA256SUMS').write_text(''.join(f'{digest(p)}  {p.name}\n' for p in archives))
(output/'QUALIFICATION.json').write_text(json.dumps({'candidate': candidate, 'result': 'not-run', 'profile': args.profile, 'whole_gate_qualified': False, 'scope': 'local macOS arm64 only'}, indent=2)+'\n')
print('Local candidate:', output)
print('Runtime, complete source and historical qualification archives created for', args.profile + '. Nothing published.')
