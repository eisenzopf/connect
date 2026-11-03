#!/usr/bin/env python3
"""Assemble an unpublished first-run kit from verified public component archives."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'scripts'))
from lib.sdk_candidate import digest
from lib.source_build_cache import snapshot


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('runtime', 'runtime-proof', 'node', 'node-proof', 'app-proof', 'output', 'evidence'):
        parser.add_argument('--' + name, type=Path, required=True)
    args = parser.parse_args()
    os.umask(0o077)
    if subprocess.check_output(['git', 'status', '--porcelain'], cwd=ROOT, text=True).strip():
        parser.error('Commit the sources before assembling a kit')
    output, evidence = args.output.resolve(), args.evidence.resolve()
    if not output.is_relative_to(ROOT / 'dist') or not evidence.is_relative_to(ROOT / '.local'):
        parser.error('Use dist/ kits and a fresh ignored .local/ evidence directory')
    evidence.mkdir(parents=True, exist_ok=False)
    before = snapshot(ROOT)
    revision = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
    app = json.loads(args.app_proof.read_text())
    native = json.loads(args.runtime_proof.read_text())
    node = json.loads(args.node_proof.read_text())
    assert app['passed'] and len(app['checks']) == 37
    assert all(app[key] for key in ['sources_unchanged', 'runtime_unchanged', 'browser_launcher_unchanged', 'application_runtime_matches'])
    assert app['binary_sha256'] == native['binary_sha256'] and app['node_archive_sha256'] == node['archive_sha256']
    assert app['runtime_source_revision'] == native['source_revision']
    spec = importlib.util.spec_from_file_location('first_run_components', ROOT / 'scripts/install-reference-app-components.py')
    components = importlib.util.module_from_spec(spec); spec.loader.exec_module(components)
    installation = components.install(args.runtime.resolve(), args.runtime_proof.resolve(), args.node.resolve(), args.node_proof.resolve(), evidence / 'verified-components')
    target = {'aarch64-apple-darwin': 'darwin-arm64', 'aarch64-unknown-linux-gnu': 'linux-arm64'}[native['target']]
    identity = 'connect-first-conversation-0.1.0-' + target + '-' + revision[:12]
    output.mkdir(parents=True, exist_ok=True)
    kit = output / identity; kit.mkdir()
    shutil.copytree(evidence / 'verified-components/runtime', kit / 'runtime')
    (kit / 'app').mkdir()
    for path in (ROOT / 'examples/parley-on-connect/app').rglob('*'):
        if not path.is_file():
            continue
        assert not path.is_symlink()
        name = path.relative_to(ROOT / 'examples/parley-on-connect/app').as_posix()
        assert digest(path) == app['source_inputs']['app'][name]
        if name.endswith('.test.mjs'):
            continue
        dest = kit / 'app' / name; dest.parent.mkdir(parents=True, exist_ok=True); shutil.copyfile(path, dest)
    for name in ('package.json', 'verify.mjs'):
        shutil.copyfile(ROOT / 'scripts/fixtures/first-conversation' / name, kit / name)
    for name in ('LICENSE', 'THIRD_PARTY_NOTICES.md'):
        shutil.copyfile(ROOT / name, kit / name)
    (kit / '.gitignore').write_text('.local/\nnode_modules/\n')
    readme = (ROOT / 'docs/start/quickstart.md').read_text().replace('../reference/index.html#voice',
        'https://github.com/eisenzopf/connect/blob/' + revision + '/docs/reference/index.html#voice')
    (kit / 'README.md').write_text(readme)
    (kit / 'sdk/dependencies').mkdir(parents=True)
    shutil.copyfile(args.node, kit / 'sdk' / node['archive_name'])
    for item in node['dependency_archives']:
        source = args.node.parent / 'dependencies' / item['file_name']
        assert digest(source) == item['sha256']
        shutil.copyfile(source, kit / 'sdk/dependencies' / item['file_name'])
        license_file = source.with_suffix('.LICENSE')
        assert digest(license_file) == item['license_sha256']
        shutil.copyfile(license_file, kit / 'sdk/dependencies' / license_file.name)
    for path in kit.rglob('*'):
        path.chmod(0o755 if path.is_dir() or path == kit / 'runtime/bin/connect' else 0o644)
    manifest = dict(format='connect/first-conversation-kit/1', source_revision=revision, platform=target,
        runtime_source_revision=native['source_revision'], runtime_archive_sha256=native['archive_sha256'],
        node_archive_sha256=node['archive_sha256'], app_proof_sha256=digest(args.app_proof),
        files={p.relative_to(kit).as_posix(): dict(sha256=digest(p), mode=0o755 if p == kit / 'runtime/bin/connect' else 0o644) for p in kit.rglob('*') if p.is_file()}, publication_authorized=False)
    (kit / 'quickstart-manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    epoch = int(subprocess.check_output(['git', 'show', '-s', '--format=%ct', revision], cwd=ROOT, text=True))
    archive = output / (identity + '.tar.gz')
    with tarfile.open(archive, 'w:gz') as tar:
        for path in sorted(kit.rglob('*')):
            if not path.is_file():
                continue
            info = tar.gettarinfo(str(path), identity + '/' + path.relative_to(kit).as_posix())
            info.uid = info.gid = 0; info.uname = info.gname = ''; info.mtime = epoch
            with path.open('rb') as stream: tar.addfile(info, stream)
    assert snapshot(ROOT) == before
    proof = dict(format='connect/first-conversation-kit-proof/1', passed=True, source_revision=revision,
        archive_name=archive.name, archive_sha256=digest(archive), manifest_sha256=digest(kit / 'quickstart-manifest.json'),
        platform=target, source_inputs_unchanged=True, verified_component_install_sha256=digest(evidence / 'verified-components/installation.json'),
        consumer_execution_verified=False, task_complete=False, publication_authorized=False)
    (evidence / 'kit.json').write_text(json.dumps(proof, indent=2) + '\n')
    print(json.dumps(proof))


if __name__ == '__main__':
    main()
