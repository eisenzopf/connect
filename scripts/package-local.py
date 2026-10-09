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

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--previous-binary', type=pathlib.Path, default=ROOT/'.local/milestones/m1/connect')
args = parser.parse_args()
os.umask(0o077)
if platform.system() != 'Darwin' or platform.machine() != 'arm64':
    parser.error('This candidate qualifies macOS arm64 only')
if subprocess.check_output(['git', 'status', '--porcelain'], cwd=ROOT, text=True).strip():
    parser.error('Commit the candidate source first; packaging requires a clean worktree')
previous = args.previous_binary.resolve()
prior = json.loads(previous.with_name('manifest.json').read_text())
def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()
if digest(previous) != prior['binary_sha256']:
    parser.error('Archived milestone binary checksum mismatch')
revision = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
version = tomllib.loads((ROOT/'Cargo.toml').read_text())['workspace']['package']['version']
candidate = f'connect-{version}-macos-arm64-{revision[:12]}'
output = ROOT/'dist'/candidate
if output.exists():
    parser.error('Candidate directory exists; retain it and use a new source revision')
subprocess.run(['bash', 'scripts/prepare-dependencies.sh'], cwd=ROOT, check=True)
subprocess.run(['npm', 'ci', '--prefix', 'sdk/typescript'], cwd=ROOT, check=True)
subprocess.run(['npm', 'run', 'build', '--prefix', 'sdk/typescript'], cwd=ROOT, check=True)
subprocess.run(['cargo', 'build', '--locked', '--release', '-p', 'connect-server',
                '-p', 'connect-review-loop', '--bins'], cwd=ROOT, check=True)
output.mkdir(parents=True)
runtime = output/candidate
(runtime/'bin').mkdir(parents=True)
for name in ['connect', 'connect-example']:
    source = ROOT/'target/release'/name
    for control in [b'CONNECT_TEST_FAULT_STAGE', b'CONNECT_EXAMPLE_FAULT_STAGE']:
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
# The root README is copied from docs/, so its report link needs one prefix.
shutil.copytree(ROOT/'docs', runtime/'docs', dirs_exist_ok=True)
readme = runtime/'README.md'
readme.write_text(readme.read_text().replace(
    '](evidence/gate-3/acceptance.html)', '](docs/evidence/gate-3/acceptance.html)'))
# Usable language clients accompany the native helper; no compiler, dependencies,
# caches or installation state are copied into the runtime archive.
shutil.copytree(ROOT/'sdk/python/connect_sdk', runtime/'sdk/python/connect_sdk',
                ignore=shutil.ignore_patterns('__pycache__', '*.pyc'))
shutil.copytree(ROOT/'sdk/typescript/dist', runtime/'sdk/typescript/dist')
shutil.copyfile(ROOT/'sdk/typescript/package.json', runtime/'sdk/typescript/package.json')
shutil.copytree(ROOT/'examples/agent-presence', runtime/'examples/agent-presence',
                ignore=shutil.ignore_patterns('__pycache__', '*.pyc'))
shutil.copytree(ROOT/'examples/conversation-client', runtime/'examples/conversation-client',
                ignore=shutil.ignore_patterns('__pycache__', '*.pyc'))
# Preserve dependency license/notice texts alongside a readable package index.
# The workspace graph is a conservative superset of the two runtime binaries.
metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--locked', '--format-version=1'], cwd=ROOT))
notices = []
for package in metadata['packages']:
    if not (package.get('source') or '').startswith('registry+'):
        continue
    identity = package['name']+'-'+package['version']
    texts = []
    for path in pathlib.Path(package['manifest_path']).parent.iterdir():
        if path.is_file() and path.name.upper().startswith(('LICENSE', 'LICENCE', 'COPYING', 'NOTICE', 'UNLICENSE')):
            destination = runtime/'licenses/registry'/identity/path.name
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(path, destination)
            texts.append(str(destination.relative_to(runtime)))
    notices.append({'package': identity, 'license': package.get('license'), 'source': package['source'], 'texts': sorted(texts)})
(runtime/'licenses/registry-index.json').write_text(json.dumps(notices, indent=2)+'\n')
files = {str(p.relative_to(runtime)): digest(p) for p in sorted(runtime.rglob('*')) if p.is_file()}
manifest = {'candidate': candidate, 'version': version, 'source_revision': revision,
            'schema': int(re.search(r'pub const SCHEMA_VERSION: u32 = (\d+);',
                                   (ROOT/'crates/connect-store/src/lib.rs').read_text()).group(1)),
            'target': 'aarch64-apple-darwin', 'os': platform.mac_ver()[0],
            'rustc': subprocess.check_output(['rustc', '--version'], text=True).strip(),
            'cargo_lock_sha256': digest(ROOT/'Cargo.lock'),
            'dependencies': json.loads((ROOT/'dependencies/source-lock.json').read_text()),
            'features': 'default; no test-support', 'scope': 'local candidate; unpublished', 'files': files}
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
with tarfile.open(output/(candidate+'-qualification.tar.gz'), 'w:gz') as archive:
    archive.add(qualification, arcname='qualification')
archives = sorted(output.glob('*.tar.gz'))
(output/'SHA256SUMS').write_text(''.join(f'{digest(p)}  {p.name}\n' for p in archives))
(output/'QUALIFICATION.json').write_text(json.dumps({'candidate': candidate, 'result': 'not-run', 'scope': 'local macOS arm64 only'}, indent=2)+'\n')
print('Local candidate:', output)
print('Runtime archive, complete SDK source, and archived schema-1 qualification binary created. Nothing published.')
