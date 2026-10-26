#!/usr/bin/env python3
"""Export account-free connector examples and their pinned public build inputs.

Only public connector/conformance packages, standalone fixtures and selected
prepared engine dependencies enter the kit. No Store, service, server, private
state, Git history or dependency symlinks are copied. Consumer manifest rewrites
are explicit patches plus original/generated hashes; Rust sources are unchanged.
"""
import argparse
import copy
import difflib
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tomllib

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ('connector-template', 'connector-contribution', 'media-connector-template')
PUBLIC = {'connect-connector-api', 'connect-media-conformance', 'connect-media-connector-template',
          'connect-connector-template', 'prism-fixture'}
ASSETS = ('LICENSE', 'rust-toolchain.toml', 'docs/CONNECTORS.md', 'contracts/connectors/profile-v2.md',
          'contracts/connectors/host-context-v1.md', 'contracts/connectors/live-fixture-v1.md',
          'scripts/check-connector-contract.py', 'scripts/check-media-connector-contract.py',
          'scripts/export-connector-kit.py', 'contracts/connectors/contribution-kit-v2.md',
          'dependencies/source-lock.json')
EXCLUDED = {'target', '.git', '.local', '.deps', '__pycache__', 'node_modules'}


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def toml_value(value):
    if isinstance(value, dict):
        return '{ ' + ', '.join(json.dumps(k) + ' = ' + toml_value(v) for k, v in value.items()) + ' }'
    if isinstance(value, list):
        return '[' + ', '.join(toml_value(v) for v in value) + ']'
    if isinstance(value, (str, bool, int, float)):
        return json.dumps(value, ensure_ascii=False)
    raise ValueError('Unsupported manifest value')


def workspace(directory):
    for parent in (directory, *directory.parents):
        manifest = parent / 'Cargo.toml'
        if manifest.is_file():
            data = tomllib.loads(manifest.read_text())
            if 'workspace' in data:
                return parent, data['workspace']
    raise ValueError('Manifest has no declared workspace')


def manifest_for(package, destination, locations):
    original = Path(package['manifest_path'])
    directory = original.parent
    data = tomllib.loads(original.read_text())
    inherited_root, inherited = workspace(directory)
    engine = package['name'] not in PUBLIC
    for key, value in list(data['package'].items()):
        if isinstance(value, dict) and value.get('workspace') is True:
            data['package'][key] = copy.deepcopy(inherited['package'][key])
    # These upstream crates are consumer dependencies here. Their development
    # suites require other transport products and are not kit qualification.
    if engine:
        data.pop('dev-dependencies', None)
        for key in ('test', 'example', 'bench', 'bin'):
            data.pop(key, None)
        for key in ('autotests', 'autoexamples', 'autobenches', 'autobins'):
            data['package'][key] = False
        data.setdefault('lib', {}).update(test=False, doctest=False)
    sections = [(data, kind) for kind in ('dependencies', 'build-dependencies', 'dev-dependencies')]
    for target in data.get('target', {}).values():
        if engine:
            target.pop('dev-dependencies', None)
        sections.extend((target, kind) for kind in ('dependencies', 'build-dependencies', 'dev-dependencies'))
    for table, section in sections:
        for name, declared in list(table.get(section, {}).items()):
            value = copy.deepcopy(declared)
            path_root = directory
            if isinstance(value, dict) and value.pop('workspace', False):
                base = copy.deepcopy(inherited['dependencies'][name])
                base = {'version': base} if isinstance(base, str) else base
                features = list(dict.fromkeys(base.get('features', []) + value.get('features', [])))
                base.update(value)
                if features:
                    base['features'] = features
                value = base
                path_root = inherited_root
            if isinstance(value, dict) and 'path' in value:
                target = (path_root / value['path']).resolve()
                if target not in locations:
                    raise ValueError(f'Unexported {section} path in {package["name"]}: {name}')
                value['path'] = os.path.relpath(destination / locations[target], destination / locations[directory.resolve()])
            table[section][name] = value
    if data.get('lints', {}).get('workspace'):
        data['lints'] = copy.deepcopy(inherited['lints'])
    data.pop('workspace', None)
    data['workspace'] = {'resolver': '2'}
    data['package'].pop('workspace', None)
    # Explicit inherited values preserve library dependency/feature semantics.
    # File-valued package metadata is materialized at its original relative path.
    for key in ('readme', 'license-file'):
        value = data['package'].get(key)
        if isinstance(value, str):
            old = tomllib.loads(original.read_text())['package'].get(key)
            root = inherited_root if isinstance(old, dict) else directory
            source = (root / value).resolve()
            if source.is_file():
                target = destination / 'provenance/workspace-files' / package['name'] / source.name
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(source, target)
                data['package'][key] = os.path.relpath(target, destination / locations[directory.resolve()])
    text = '# Generated consumer manifest. See connector-kit-source.json and provenance/.\n'
    text += '\n'.join(json.dumps(key) + ' = ' + toml_value(value) for key, value in data.items()) + '\n'
    assert tomllib.loads(text) == data
    return text, engine


def metadata(manifest, locked=True):
    args = ['cargo', 'metadata', '--offline', '--format-version=1', '--manifest-path', str(manifest)]
    if locked:
        args.append('--locked')
    if manifest.parent.name == 'media-connector-template':
        args += ['--features', 'conformance']
    return json.loads(subprocess.check_output(args, cwd=manifest.parent, stderr=subprocess.PIPE, text=True, timeout=60))


def copy_tree(source, destination, source_hashes, label):
    for path in sorted(source.rglob('*')):
        relative = path.relative_to(source)
        if any(part in EXCLUDED for part in relative.parts):
            continue
        if path.is_symlink():
            raise ValueError('Package contains an unsupported source symlink')
        if path.is_file():
            target = destination / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(path, target)
            source_hashes[label + '/' + str(relative)] = digest(path)
            if digest(target) != digest(path):
                raise ValueError('Source copy hash mismatch')


def verify(destination):
    saved = json.loads((destination / 'connector-kit-source.json').read_text())
    actual = {str(p.relative_to(destination)): digest(p) for p in destination.rglob('*')
              if p.is_file() and p.name != 'connector-kit-source.json'
              and not any(part in EXCLUDED for part in p.relative_to(destination).parts)}
    if any(p.is_symlink() for p in destination.rglob('*')) or actual != saved['files']:
        raise ValueError('Kit file inventory changed')
    roots = [Path('examples') / name for name in FIXTURES] + [Path('crates/connect-connector-api'), Path('crates/connect-media-conformance')]
    for root in roots:
        data = metadata(destination / root / 'Cargo.toml')
        if any(not Path(p['manifest_path']).resolve().is_relative_to(destination)
               for p in data['packages'] if p['source'] is None):
            raise ValueError('Kit resolves a local dependency outside itself')
    return {'files': len(actual), 'fixtures': len(FIXTURES), 'independent_libraries': 2, 'all_local_dependencies_inside_kit': True}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--verify', action='store_true', help='Verify an existing kit inventory and local Cargo graph')
    args = parser.parse_args()
    destination = args.output.absolute()
    if args.verify:
        print(json.dumps(verify(destination.resolve(strict=True))))
        return
    if destination.exists() or destination.is_symlink():
        parser.error('Destination exists; choose a fresh directory')
    if not (ROOT / 'scripts/prepare-dependencies.sh').is_file():
        parser.error('Create exports from a Connect checkout; existing kits support --verify')
    # Verify original locked source/patch state before any package snapshot.
    subprocess.run(['bash', str(ROOT / 'scripts/prepare-dependencies.sh')], cwd=ROOT, check=True, timeout=600)
    packages = {}
    originals = {}
    for name in FIXTURES:
        data = metadata(ROOT / 'examples' / name / 'Cargo.toml')
        originals[name] = data
        for package in data['packages']:
            if package['source'] is None:
                packages[Path(package['manifest_path']).parent.resolve()] = package
    engine_root = (ROOT / '.deps/rvoip').resolve()
    locations = {}
    for directory, package in packages.items():
        if package['name'] in PUBLIC and directory.is_relative_to(ROOT):
            locations[directory] = directory.relative_to(ROOT)
        elif directory.is_relative_to(engine_root):
            locations[directory] = Path('engine/rvoip') / directory.relative_to(engine_root)
        else:
            raise ValueError('Kit requires an unapproved local package')
    destination.mkdir(parents=True)
    source_hashes, transforms = {}, []
    for directory, package in packages.items():
        label = str(locations[directory])
        target = destination / locations[directory]
        copy_tree(directory, target, source_hashes, label)
        original = directory / 'Cargo.toml'
        rendered, engine = manifest_for(package, destination, locations)
        before = original.read_text()
        (target / 'Cargo.toml').write_text(rendered)
        provenance = destination / 'provenance/manifests' / package['name']
        provenance.mkdir(parents=True)
        (provenance / 'Cargo.toml').write_text(before)
        patch = provenance / 'consumer.patch'
        patch.write_text(''.join(difflib.unified_diff(before.splitlines(True), rendered.splitlines(True),
                         fromfile=label + '/Cargo.toml', tofile=label + '/Cargo.toml')))
        transforms.append({'package': package['name'], 'manifest': label + '/Cargo.toml',
                           'original_sha256': digest(original), 'generated_sha256': digest(target / 'Cargo.toml'),
                           'patch': str(patch.relative_to(destination)), 'patch_sha256': digest(patch),
                           'changes': 'Flatten workspace inheritance and relocate local dependency paths inside the kit.'
                                      + (' Exclude engine development dependencies/targets; no upstream unit-suite claim.' if engine else '')})
    for name in ASSETS:
        target = destination / name
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(ROOT / name, target)
        source_hashes[name] = digest(ROOT / name)
    lock = json.loads((ROOT / 'dependencies/source-lock.json').read_text())
    for item in lock['patches']:
        name = item['path']
        assert digest(ROOT / name) == item['sha256']
        target = destination / name
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(ROOT / name, target)
        source_hashes[name] = item['sha256']
    for name in ('LICENSE', 'README.md'):
        source = engine_root / name
        if source.is_file():
            target = destination / 'engine/rvoip' / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, target)
            source_hashes['engine/rvoip/' + name] = digest(source)
    (destination / 'Cargo.toml').write_text('[workspace]\nresolver = "2"\nmembers = []\n')
    (destination / 'README.md').write_text('''# Independent connector contribution kit

This snapshot includes standalone message/PCM and actual outbound-PCMU fixtures,
the public connector API, reusable conformance harness and selected pinned engine
consumer sources. It contains no Connect Store/service/server or application DB.

From this directory, with the pinned Rust toolchain and locked registry dependencies available:

```sh
cargo test --offline --locked --manifest-path examples/connector-template/Cargo.toml --target-dir /absolute/private/owned-target
cargo test --offline --locked --manifest-path examples/connector-contribution/Cargo.toml --target-dir /absolute/private/owned-target
cargo test --release --offline --locked --manifest-path examples/media-connector-template/Cargo.toml --target-dir /absolute/private/owned-target
python3 scripts/check-media-connector-contract.py --target-dir /absolute/private/owned-target --output /absolute/private/media-positive.json
python3 scripts/check-media-connector-contract.py --target-dir /absolute/private/owned-target --negative-control --output /absolute/private/media-negative.json
```

Verify the unchanged kit and all local dependency boundaries before reuse:

```sh
python3 scripts/export-connector-kit.py --output "$PWD" --verify
```

Use fresh report paths. Registry caches are external build inputs; this kit does
not vendor them or claim clean-cache/package/platform qualification. See
`connector-kit-source.json` for exact source/patch digests and generated consumer
manifest patches. No Rust source is modified. Upstream engine development targets
are excluded from generated dependency manifests; their original manifests and
patches are retained under provenance. Do not infer upstream unit-suite coverage.

Read docs/CONNECTORS.md and contracts/connectors/live-fixture-v1.md. Component
reports do not qualify stock application workflows, real accounts/models,
network/browser/Linux/remote profiles, process crash persistence or publication.
''')
    # The public libraries inherit the repository lock in their source workspace.
    # Give their standalone consumer workspaces their own locked test graphs too.
    locked_versions = {(p['name'], p['version'], p.get('source'))
                       for p in tomllib.loads((ROOT / 'Cargo.lock').read_text())['package']
                       if p.get('source') is not None}
    for folder in ('crates/connect-connector-api', 'crates/connect-media-conformance'):
        shutil.copy2(ROOT / 'Cargo.lock', destination / folder / 'Cargo.lock')
        resolved = metadata(destination / folder / 'Cargo.toml', locked=False)
        versions = {(p['name'], p['version'], p['source']) for p in resolved['packages'] if p['source'] is not None}
        if not versions <= locked_versions:
            raise ValueError('Standalone public library introduced an unlocked registry version')
    for name in FIXTURES:
        # Keep source lock versions. Cargo may normalize the path-only package
        # inventory after manifest relocation; registry versions must not change.
        moved = metadata(destination / 'examples' / name / 'Cargo.toml', locked=False)
        original_versions = {(p['name'], p['version'], p['source']) for p in originals[name]['packages'] if p['source'] is not None}
        moved_versions = {(p['name'], p['version'], p['source']) for p in moved['packages'] if p['source'] is not None}
        if moved_versions != original_versions:
            raise ValueError('Export changed registry dependency resolution')
    record = {'format': 1, 'profile': 'connect-connector-contribution-kit-v2',
              'source_revision': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
              'source_lock_sha256': digest(ROOT / 'dependencies/source-lock.json'),
              'sources': lock['sources'], 'source_files': source_hashes, 'consumer_manifests': transforms,
              'scope': 'Public standalone connector component sources; no stock host, installed runtime, clean-cache, provider or publication claim.'}
    record['files'] = {str(p.relative_to(destination)): digest(p) for p in sorted(destination.rglob('*')) if p.is_file()}
    (destination / 'connector-kit-source.json').write_text(json.dumps(record, indent=2) + '\n')
    print(json.dumps({'exported': True, **verify(destination.resolve())}))


if __name__ == '__main__':
    main()
